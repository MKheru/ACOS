#!/usr/bin/env python3
"""
ACOS QEMU Controller — reliable boot, login, and command execution.

Architecture:
- QEMU runs with -vga std (VGA window) + -serial pty (serial on host PTY)
- QMP socket for sending keyboard input (needed for resolution selector)
- pexpect on the serial PTY for reading output / interacting with shell

The ACOS bootloader shows a resolution selector on VGA that needs Enter.
After that, serial console becomes active with a login prompt.

Usage:
    python3 qemu-test.py                    # Boot + login + echo HELLO test
    python3 qemu-test.py boot-and-test-mux  # Boot + login + test acos-mux
    python3 qemu-test.py --keep             # Don't kill QEMU after test
"""

import sys
import os
import re
import json
import time
import signal
import socket
import subprocess
import pexpect
import pexpect.fdpexpect

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REDOX_DIR = os.path.join(SCRIPT_DIR, "..", "redox_base")
IMAGE = os.path.join(REDOX_DIR, "build", "x86_64", "acos-bare", "harddrive.img")
QMP_SOCK = "/tmp/acos-qmp.sock"
BOOT_TIMEOUT = 60
CMD_TIMEOUT = 10
LOGIN_TIMEOUT = 15


class QEMUController:
    def __init__(self, image=IMAGE):
        self.image = os.path.abspath(image)
        self.qemu_pid = None
        self.serial = None   # pexpect.fdpexpect.fdspawn on serial PTY
        self.pty_path = None
        self.qmp_sock = None  # socket for QMP

    def _qmp_connect(self):
        """Connect to QMP and do capabilities handshake."""
        self.qmp_sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.qmp_sock.settimeout(5)
        self.qmp_sock.connect(QMP_SOCK)
        # Read greeting
        self.qmp_sock.recv(4096)
        # Send capabilities
        self.qmp_sock.sendall(b'{"execute":"qmp_capabilities"}\n')
        self.qmp_sock.recv(4096)

    def _qmp_send_key(self, key):
        """Send a key via QMP."""
        cmd = json.dumps({
            "execute": "send-key",
            "arguments": {"keys": [{"type": "qcode", "data": key}]}
        })
        self.qmp_sock.sendall((cmd + "\n").encode())
        try:
            self.qmp_sock.recv(4096)
        except socket.timeout:
            pass

    def _qmp_send_string(self, text):
        """Send a string character by character via QMP."""
        keymap = {}
        for c in "abcdefghijklmnopqrstuvwxyz":
            keymap[c] = c
        for c in "0123456789":
            keymap[c] = c
        keymap[" "] = "spc"
        keymap["/"] = "slash"
        keymap["-"] = "minus"
        keymap["_"] = "shift-minus"
        keymap["="] = "equal"
        keymap["."] = "dot"
        keymap[","] = "comma"

        for ch in text:
            if ch in keymap:
                qcode = keymap[ch]
            elif ch.isupper():
                qcode = f"shift-{ch.lower()}"
            else:
                continue
            # Handle shift combinations
            if qcode.startswith("shift-"):
                actual_key = qcode[6:]
                cmd = json.dumps({
                    "execute": "send-key",
                    "arguments": {"keys": [
                        {"type": "qcode", "data": "shift"},
                        {"type": "qcode", "data": actual_key}
                    ]}
                })
            else:
                cmd = json.dumps({
                    "execute": "send-key",
                    "arguments": {"keys": [{"type": "qcode", "data": qcode}]}
                })
            self.qmp_sock.sendall((cmd + "\n").encode())
            try:
                self.qmp_sock.recv(4096)
            except socket.timeout:
                pass
            time.sleep(0.03)

    def start(self):
        """Boot QEMU with VGA window + serial PTY + QMP."""
        if not os.path.exists(self.image):
            raise FileNotFoundError(f"Image not found: {self.image}")

        # Clean up old sockets
        for f in [QMP_SOCK]:
            try:
                os.unlink(f)
            except OSError:
                pass

        firmware = ""
        for fw in [
            "/usr/share/edk2/ovmf/OVMF_CODE.fd",
            "/usr/share/OVMF/OVMF_CODE.fd",
        ]:
            if os.path.exists(fw):
                firmware = fw
                break

        stderr_log = "/tmp/acos-qemu-stderr.log"

        qemu_cmd = (
            f"qemu-system-x86_64"
            f" -machine q35 -cpu host -enable-kvm -smp 4 -m 2048"
            f" -vga std"
            f" -serial pty"
            f" -qmp unix:{QMP_SOCK},server,nowait"
            f" -drive file={self.image},format=raw,if=none,id=drv0"
            f" -device nvme,drive=drv0,serial=ACOS"
            f" -net none -no-reboot"
        )
        if firmware:
            qemu_cmd += f" -bios {firmware}"

        # Use Popen to avoid blocking on QEMU's background process
        # Redirect QEMU's stdout/stderr to files so shell returns immediately
        serial_log = "/tmp/acos-serial-detect.log"
        shell_cmd = f"{qemu_cmd} >{serial_log} 2>{stderr_log} & echo $!"
        print("[QEMU] Starting...")
        proc = subprocess.Popen(
            ["bash", "-c", shell_cmd],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            stdin=subprocess.DEVNULL
        )
        pid_line = proc.stdout.readline().decode().strip()
        proc.stdout.close()
        proc.wait()

        if not pid_line.isdigit():
            print(f"[QEMU] Failed to get PID: '{pid_line}'")
            raise RuntimeError("QEMU did not start")
        self.qemu_pid = int(pid_line)
        print(f"[QEMU] PID: {self.qemu_pid}")

        # Wait for PTY path in stdout or stderr log
        deadline = time.time() + 5
        while not self.pty_path and time.time() < deadline:
            time.sleep(0.3)
            try:
                os.kill(self.qemu_pid, 0)
            except OSError:
                errmsg = ""
                for logf in [stderr_log, serial_log]:
                    if os.path.exists(logf):
                        with open(logf) as f:
                            errmsg += f.read()
                print(f"[QEMU] Exited early. logs: {errmsg}")
                raise RuntimeError("QEMU exited")
            # Check both logs for the PTY path
            for logf in [serial_log, stderr_log]:
                if os.path.exists(logf):
                    with open(logf) as f:
                        content = f.read()
                    match = re.search(r"redirected to (/dev/pts/\d+)", content)
                    if match:
                        self.pty_path = match.group(1)
                        break

        if not self.pty_path:
            self.stop()
            raise RuntimeError("Failed to get PTY path")

        print(f"[QEMU] Serial PTY: {self.pty_path}")

        # Wait a moment for PTY to be ready
        time.sleep(0.5)

        # Connect to serial PTY
        fd = os.open(self.pty_path, os.O_RDWR)
        self.serial = pexpect.fdpexpect.fdspawn(fd, timeout=CMD_TIMEOUT, encoding="utf-8",
                                       codec_errors="replace", maxread=65536)
        print("[QEMU] Serial connected")

        # Connect QMP
        time.sleep(0.5)
        self._qmp_connect()
        print("[QEMU] QMP connected")

        return self

    def wait_boot(self, timeout=BOOT_TIMEOUT):
        """Wait for boot, pressing Enter to pass the resolution selector."""
        print(f"[QEMU] Waiting for boot (timeout={timeout}s)...")

        # The bootloader shows a resolution selector on VGA.
        # We need to press Enter to accept the default and continue.
        # Send Enter a few times with delays to catch the right moment.
        def send_enter_periodically():
            """Send Enter via QMP every 3s for the first 20s of boot."""
            for i in range(7):
                time.sleep(3)
                try:
                    self._qmp_send_key("ret")
                    if i == 0:
                        print("[QEMU] Sent Enter (resolution selector)")
                except Exception:
                    break

        import threading
        enter_thread = threading.Thread(target=send_enter_periodically, daemon=True)
        enter_thread.start()

        try:
            idx = self.serial.expect(
                [r"login:", r"ACOS_BOOT_OK"],
                timeout=timeout,
            )
            marker = ["login:", "ACOS_BOOT_OK"][idx]
            print(f"[QEMU] Boot complete — '{marker}'")
            return True
        except pexpect.TIMEOUT:
            print("[QEMU] Boot TIMEOUT")
            if self.serial.before:
                last = self.serial.before[-500:]
                print(f"[QEMU] Last serial: ...{last}")
            return False
        except pexpect.EOF:
            print("[QEMU] Serial EOF")
            return False

    def login(self, user="root", password="password"):
        """Login via serial console."""
        print(f"[QEMU] Logging in as {user}...")
        # Wait for the login prompt to actually appear
        try:
            self.serial.expect(r"login:", timeout=LOGIN_TIMEOUT)
        except pexpect.TIMEOUT:
            print("[QEMU] No login prompt found, sending Enter")
            self.serial.sendline("")
            try:
                self.serial.expect(r"login:", timeout=LOGIN_TIMEOUT)
            except pexpect.TIMEOUT:
                print("[QEMU] Still no login prompt")
                return False
        self.serial.sendline(user)
        try:
            idx = self.serial.expect(
                [r"[Pp]assword:", r"#", r"\$", r"ion:"],
                timeout=LOGIN_TIMEOUT,
            )
            if idx == 0:
                self.serial.sendline(password)
                self.serial.expect([r"#", r"\$", r"ion:"], timeout=LOGIN_TIMEOUT)
            # Wait for shell to be fully ready
            time.sleep(1)
            # Drain any remaining output by sending a no-op
            self.serial.sendline("")
            time.sleep(0.5)
            # Try to get a clean prompt
            self.serial.sendline("echo READY")
            try:
                self.serial.expect("READY", timeout=5)
            except pexpect.TIMEOUT:
                pass
            print("[QEMU] Login OK")
            return True
        except pexpect.TIMEOUT:
            print("[QEMU] Login timeout")
            if self.serial.before:
                print(f"[QEMU] Last: {self.serial.before[-300:]}")
            return False

    def run_command(self, cmd, timeout=CMD_TIMEOUT):
        """Run a command via serial, return output."""
        marker_start = f"__START_{int(time.time())}__"
        marker_end = f"__END_{int(time.time())}__"
        # Send start marker, command, end marker as a compound command
        compound = f"echo {marker_start}; {cmd}; echo {marker_end}"
        self.serial.sendline(compound)
        try:
            # Wait for start marker (may be command echo or output)
            self.serial.expect(marker_start, timeout=timeout)
            # Now wait for end marker — everything between includes our output
            self.serial.expect(marker_end, timeout=timeout)
            output = self.serial.before
            # Strip ANSI escape sequences
            clean = re.sub(r'\x1b\[[0-9;]*[a-zA-Z]', '', output)
            clean = re.sub(r'\x1b\[[?][0-9;]*[a-zA-Z]', '', clean)
            # Remove shell prompt noise and clean up
            lines = clean.strip().split("\n")
            result_lines = []
            for line in lines:
                stripped = line.strip()
                if not stripped:
                    continue
                # Skip lines that contain our markers or the compound command echo
                if marker_start in stripped or marker_end in stripped:
                    continue
                # Skip the shell echo of the full compound command
                if "echo " + marker_start in stripped:
                    continue
                result_lines.append(stripped)
            return "\n".join(result_lines)
        except pexpect.TIMEOUT:
            print(f"[QEMU] Command timeout: {cmd}")
            if self.serial.before:
                print(f"[QEMU] Last: {self.serial.before[-300:]}")
            return None

    def stop(self):
        """Kill QEMU."""
        if self.qmp_sock:
            try:
                self.qmp_sock.close()
            except Exception:
                pass
        if self.serial:
            try:
                self.serial.close()
            except Exception:
                pass
        if self.qemu_pid:
            try:
                os.kill(self.qemu_pid, signal.SIGTERM)
                for _ in range(10):
                    time.sleep(0.5)
                    try:
                        os.kill(self.qemu_pid, 0)
                    except OSError:
                        break
                else:
                    os.kill(self.qemu_pid, signal.SIGKILL)
            except OSError:
                pass
            print("[QEMU] Stopped")


def test_echo(keep=False):
    """Phase A: boot → login → echo HELLO → verify."""
    q = QEMUController()
    try:
        q.start()
        if not q.wait_boot():
            print("FAIL: Boot failed")
            return False

        if not q.login():
            print("FAIL: Login failed")
            return False

        output = q.run_command("echo HELLO")
        print(f"[TEST] Output: '{output}'")

        if output and "HELLO" in output:
            print("PASS: echo HELLO → HELLO found")
            return True
        else:
            print("FAIL: HELLO not found")
            return False
    finally:
        if not keep:
            q.stop()


def serial_cmd(q, cmd, timeout=5):
    """Run a command on serial, return all output. Ion-compatible."""
    # IMPORTANT: ion doesn't support 2>&1 or 2> syntax
    q.serial.sendline(cmd)
    time.sleep(1)
    marker = f"__DONE_{int(time.time() * 1000) % 100000}__"
    q.serial.sendline(f"echo {marker}")
    try:
        q.serial.expect(marker, timeout=timeout)
        raw = q.serial.before or ""
        # Clean ANSI (CSI + OSC)
        raw = re.sub(r'\x1b\][^\x07]*\x07', '', raw)
        raw = re.sub(r'\x1b\[[0-9;]*[a-zA-Z]', '', raw)
        raw = re.sub(r'\x1b\[[?][0-9;]*[a-zA-Z]', '', raw)
        return raw.strip()
    except pexpect.TIMEOUT:
        return None


def test_mux(keep=False):
    """Phase B: boot → login → run acos-mux → check it stays alive."""
    q = QEMUController()
    score = 0
    try:
        q.start()
        if not q.wait_boot():
            print(f"SCORE={score}")
            return score

        if not q.login():
            print(f"SCORE={score}")
            return score

        # Launch mux in background. Only redirect stdout (ion doesn't support 2> syntax).
        # Stderr stays on serial for debug visibility.
        q.serial.sendline("EMUX_LOG=/tmp/m.log acos-mux >/dev/null &")

        # Wait for mux to start and emit stderr
        time.sleep(5)
        # Capture everything on serial since launch
        q.serial.sendline("echo CAPTURE_MUX_DONE")
        try:
            q.serial.expect("CAPTURE_MUX_DONE", timeout=10)
        except pexpect.TIMEOUT:
            pass
        startup_text = q.serial.before or ""
        # Clean ANSI
        startup_text = re.sub(r'\x1b\][^\x07]*\x07', '', startup_text)
        startup_text = re.sub(r'\x1b\[[0-9;]*[a-zA-Z]', '', startup_text)
        startup_text = re.sub(r'\x1b\[[?][0-9;]*[a-zA-Z]', '', startup_text)
        # Print all acos-mux related lines
        for line in startup_text.split('\n'):
            line = line.strip()
            if line and ('acos-mux' in line or 'panic' in line.lower() or 'FAILED' in line) and 'login' not in line and 'echo' not in line and 'EMUX_LOG' not in line:
                print(f"[MUX] {line}")

        # Check if alive with a simple ps
        time.sleep(1)
        q.serial.sendline("ps")
        time.sleep(2)
        q.serial.sendline("echo PS_DONE")
        try:
            q.serial.expect("PS_DONE", timeout=5)
        except pexpect.TIMEOUT:
            pass
        ps_text = q.serial.before or ""
        ps_text = re.sub(r'\x1b\][^\x07]*\x07', '', ps_text)
        ps_text = re.sub(r'\x1b\[[0-9;]*[a-zA-Z]', '', ps_text)
        ps_text = re.sub(r'\x1b\[[?][0-9;]*[a-zA-Z]', '', ps_text)
        ps_lines = [l.strip() for l in ps_text.split('\n') if l.strip()]
        # Find lines that look like ps output with acos-mux
        mux_procs = [l for l in ps_lines if 'acos-mux' in l and 'grep' not in l and 'echo' not in l and '>' not in l and 'EMUX' not in l]
        print(f"[DEBUG] ps mux procs: {mux_procs}")
        has_mux = len(mux_procs) > 0
        if has_mux:
            score = 1
            print("[TEST] acos-mux running (score=1)")

            # Debug: list files (ion-compatible, no 2>&1)
            dbg = serial_cmd(q, "ls /tmp/acos-mux*")
            print(f"[DEBUG] debug files: {dbg}")

            # Debug: check EMUX log
            dbg = serial_cmd(q, "cat /tmp/m.log")
            print(f"[DEBUG] emux log: {dbg}")
        else:
            print("[TEST] acos-mux not running")
            print(f"SCORE={score}")
            return score

        time.sleep(3)
        # Check ps with proper approach
        q.serial.sendline("ps")
        time.sleep(2)
        q.serial.sendline("echo PS2_DONE")
        try:
            q.serial.expect("PS2_DONE", timeout=5)
        except pexpect.TIMEOUT:
            pass
        ps_text2 = q.serial.before or ""
        ps_text2 = re.sub(r'\x1b\][^\x07]*\x07', '', ps_text2)
        ps_text2 = re.sub(r'\x1b\[[0-9;]*[a-zA-Z]', '', ps_text2)
        ps_text2 = re.sub(r'\x1b\[[?][0-9;]*[a-zA-Z]', '', ps_text2)
        mux_procs2 = [l.strip() for l in ps_text2.split('\n') if 'acos-mux' in l and 'grep' not in l and 'echo' not in l and '>' not in l and 'EMUX' not in l]
        print(f"[DEBUG] ps2 mux procs: {mux_procs2}")
        if mux_procs2:
            score = 2
            print("[TEST] acos-mux alive after 3s (score=2)")
        else:
            print("[TEST] acos-mux died")
            # Try to get debug after death
            time.sleep(1)
            dbg = serial_cmd(q, "cat /tmp/acos-mux.crash", timeout=3)
            if dbg:
                print(f"[DEBUG] post-crash: {dbg}")
            print(f"SCORE={score}")
            return score

        # Read EMUX log using proper capture
        q.serial.sendline("cat /tmp/m.log | head -20")
        time.sleep(2)
        q.serial.sendline("echo LOG_READ_DONE")
        try:
            q.serial.expect("LOG_READ_DONE", timeout=5)
        except pexpect.TIMEOUT:
            pass
        log_text = q.serial.before or ""
        log_text = re.sub(r'\x1b\][^\x07]*\x07', '', log_text)
        log_text = re.sub(r'\x1b\[[0-9;]*[a-zA-Z]', '', log_text)
        log_text = re.sub(r'\x1b\[[?][0-9;]*[a-zA-Z]', '', log_text)
        # Print log contents
        log_lines = [l.strip() for l in log_text.split('\n') if l.strip() and 'root:~#' not in l and 'echo' not in l and 'LOG_READ' not in l]
        print(f"[DEBUG] mux log lines: {log_lines}")
        if any("ion:" in l or "shell" in l.lower() or "pty read" in l for l in log_lines):
            score = 3
            print("[TEST] Shell prompt in PTY (score=3)")
        else:
            print(f"[TEST] No shell prompt in log")

        print(f"SCORE={score}")
        return score
    finally:
        if not keep:
            q.stop()


if __name__ == "__main__":
    keep = "--keep" in sys.argv
    args = [a for a in sys.argv[1:] if not a.startswith("--")]

    if not args or args[0] == "test-echo":
        ok = test_echo(keep=keep)
        sys.exit(0 if ok else 1)
    elif args[0] == "boot-and-test-mux":
        score = test_mux(keep=keep)
        sys.exit(0 if score >= 3 else 1)
    else:
        print(f"Unknown command: {args[0]}")
        print("Usage: qemu-test.py [test-echo|boot-and-test-mux] [--keep]")
        sys.exit(1)
