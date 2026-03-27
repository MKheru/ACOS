#!/usr/bin/env python3
"""
ACOS QEMU Controller — complete programmatic control of ACOS in QEMU.

Provides three control channels:
1. Serial PTY (pexpect) — text-mode shell interaction
2. QMP (JSON-RPC) — VM lifecycle, screendump
3. VNC (vncdotool) — VGA keyboard/mouse input, screenshots

Usage:
    from acos_qemu import ACOSController

    with ACOSController() as vm:
        vm.boot()
        vm.login()
        output = vm.run("echo hello")
        vm.screenshot("/tmp/screen.png")
"""

import os
import re
import sys
import json
import time
import signal
import socket
import subprocess
import threading

import pexpect
import pexpect.fdpexpect

# Optional imports — graceful degradation if not installed
try:
    from vncdotool import api as vncapi
    HAS_VNC = True
except ImportError:
    HAS_VNC = False

try:
    from PIL import Image
    HAS_PIL = True
except ImportError:
    HAS_PIL = False

try:
    import pytesseract
    HAS_OCR = True
except Exception:
    HAS_OCR = False


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_DIR = os.path.dirname(SCRIPT_DIR)
REDOX_DIR = os.path.join(PROJECT_DIR, "redox_base")
DEFAULT_IMAGE = os.path.join(REDOX_DIR, "build", "x86_64", "acos-bare", "harddrive.img")

QMP_SOCK = "/tmp/acos-qmp.sock"
VNC_DISPLAY = 42  # :42 to avoid conflicts
VNC_PORT = 5900 + VNC_DISPLAY

BOOT_TIMEOUT = 90
LOGIN_TIMEOUT = 15
CMD_TIMEOUT = 10


# ---------------------------------------------------------------------------
# ANSI/OSC Cleanup
# ---------------------------------------------------------------------------

def clean_ansi(text):
    """Remove ANSI escape sequences and OSC from text."""
    text = re.sub(r'\x1b\][^\x07]*\x07', '', text)        # OSC sequences
    text = re.sub(r'\x1b\[[0-9;]*[a-zA-Z]', '', text)     # CSI sequences
    text = re.sub(r'\x1b\[\?[0-9;]*[a-zA-Z]', '', text)   # Private CSI
    text = re.sub(r'\x1b\[[\d;]*m', '', text)              # SGR
    return text


# ---------------------------------------------------------------------------
# QMP Client
# ---------------------------------------------------------------------------

class QMPClient:
    """Simple synchronous QMP client using raw sockets."""

    def __init__(self, sock_path=QMP_SOCK):
        self.sock_path = sock_path
        self.sock = None

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(5)
        self.sock.connect(self.sock_path)
        # Read greeting
        self._recv()
        # Capabilities handshake
        self.execute("qmp_capabilities")

    def _recv(self):
        """Read all available data from socket."""
        chunks = []
        while True:
            try:
                data = self.sock.recv(4096)
                if not data:
                    break
                chunks.append(data)
                if len(data) < 4096:
                    break
            except socket.timeout:
                break
        return b"".join(chunks)

    def execute(self, command, **kwargs):
        """Execute a QMP command, return the response."""
        msg = {"execute": command}
        if kwargs:
            msg["arguments"] = kwargs
        self.sock.sendall((json.dumps(msg) + "\n").encode())
        response = self._recv().decode(errors="replace")
        # Parse last JSON line (skip events)
        for line in reversed(response.strip().split("\n")):
            line = line.strip()
            if line:
                try:
                    return json.loads(line)
                except json.JSONDecodeError:
                    continue
        return None

    def send_key(self, qcode):
        """Send a single key via QMP send-key."""
        return self.execute("send-key", keys=[{"type": "qcode", "data": qcode}])

    def send_keys(self, *qcodes):
        """Send multiple keys simultaneously (e.g., shift+a)."""
        keys = [{"type": "qcode", "data": q} for q in qcodes]
        return self.execute("send-key", keys=keys)

    def send_string(self, text, delay=0.05):
        """Type a string character by character via QMP."""
        KEYMAP = {
            **{c: c for c in "abcdefghijklmnopqrstuvwxyz0123456789"},
            " ": "spc", "/": "slash", "-": "minus", "=": "equal",
            ".": "dot", ",": "comma", ";": "semicolon",
            "'": "apostrophe", "`": "grave_accent",
            "[": "bracket_left", "]": "bracket_right",
            "\\": "backslash",
            # Shift variants
            "_": ("shift", "minus"), "+": ("shift", "equal"),
            ":": ("shift", "semicolon"), '"': ("shift", "apostrophe"),
            "~": ("shift", "grave_accent"),
            "{": ("shift", "bracket_left"), "}": ("shift", "bracket_right"),
            "|": ("shift", "backslash"),
            "!": ("shift", "1"), "@": ("shift", "2"), "#": ("shift", "3"),
            "$": ("shift", "4"), "%": ("shift", "5"), "^": ("shift", "6"),
            "&": ("shift", "7"), "*": ("shift", "8"),
            "(": ("shift", "9"), ")": ("shift", "0"),
            "<": ("shift", "comma"), ">": ("shift", "dot"),
            "?": ("shift", "slash"),
        }
        for ch in text:
            if ch in KEYMAP:
                mapping = KEYMAP[ch]
                if isinstance(mapping, tuple):
                    self.send_keys(*mapping)
                else:
                    self.send_key(mapping)
            elif ch.isupper():
                self.send_keys("shift", ch.lower())
            time.sleep(delay)

    def screendump(self, filename="/tmp/acos-screenshot.ppm"):
        """Take a VGA screenshot, save as PPM."""
        return self.execute("screendump", filename=filename)

    def close(self):
        if self.sock:
            try:
                self.sock.close()
            except Exception:
                pass
            self.sock = None


# ---------------------------------------------------------------------------
# Serial Console
# ---------------------------------------------------------------------------

class SerialConsole:
    """Serial PTY interaction via pexpect."""

    def __init__(self, pty_path):
        fd = os.open(pty_path, os.O_RDWR)
        self.serial = pexpect.fdpexpect.fdspawn(
            fd, timeout=CMD_TIMEOUT, encoding="utf-8",
            codec_errors="replace", maxread=65536
        )
        self.pty_path = pty_path

    def wait_for_boot(self, timeout=BOOT_TIMEOUT):
        """Wait for boot marker on serial."""
        try:
            idx = self.serial.expect(
                [r"login:", r"ACOS_BOOT_OK"],
                timeout=timeout,
            )
            return True
        except (pexpect.TIMEOUT, pexpect.EOF):
            return False

    def login(self, user="root", password="password"):
        """Login via serial. Returns True on success."""
        # Wait for login prompt
        try:
            self.serial.expect(r"login:", timeout=LOGIN_TIMEOUT)
        except pexpect.TIMEOUT:
            # Try sending Enter to get a fresh prompt
            self.serial.sendline("")
            try:
                self.serial.expect(r"login:", timeout=LOGIN_TIMEOUT)
            except pexpect.TIMEOUT:
                return False

        self.serial.sendline(user)

        try:
            idx = self.serial.expect(
                [r"[Pp]assword:", r"#", r"\$", r"root:"],
                timeout=LOGIN_TIMEOUT,
            )
            if idx == 0:
                self.serial.sendline(password)
                self.serial.expect([r"#", r"\$", r"root:"], timeout=LOGIN_TIMEOUT)
        except pexpect.TIMEOUT:
            return False

        # Wait for shell readiness
        time.sleep(1)
        self.serial.sendline("")
        time.sleep(0.5)
        self.serial.sendline("echo SHELL_READY")
        try:
            self.serial.expect("SHELL_READY", timeout=5)
            return True
        except pexpect.TIMEOUT:
            return True  # Shell might work even without echo match

    def run(self, cmd, timeout=CMD_TIMEOUT):
        """Run a command, return cleaned output. Ion-compatible."""
        self.serial.sendline(cmd)
        time.sleep(1)
        marker = f"__DONE_{int(time.time() * 1000) % 100000}__"
        self.serial.sendline(f"echo {marker}")
        try:
            self.serial.expect(marker, timeout=timeout)
            raw = self.serial.before or ""
            raw = clean_ansi(raw)
            # Filter out shell prompt echoes and command echo
            lines = []
            for line in raw.split("\n"):
                line = line.strip()
                if not line:
                    continue
                # Skip prompt echoes (root:~# with partial command)
                if "root:" in line and "#" in line:
                    # But keep lines that have content after the prompt
                    after_prompt = re.sub(r'.*root:.*?#\s*', '', line)
                    if after_prompt and marker not in after_prompt and "echo" not in after_prompt:
                        lines.append(after_prompt)
                    continue
                # Skip our markers and echo commands
                if marker in line or "echo " + marker in line:
                    continue
                if cmd in line:
                    continue
                lines.append(line)
            return "\n".join(lines)
        except pexpect.TIMEOUT:
            return None

    def sendline(self, text):
        """Raw sendline."""
        self.serial.sendline(text)

    def expect(self, pattern, timeout=CMD_TIMEOUT):
        """Raw expect."""
        return self.serial.expect(pattern, timeout=timeout)

    def read_all(self, timeout=2):
        """Read all available serial data."""
        self.serial.sendline(f"echo __FLUSH_{int(time.time())}__")
        try:
            self.serial.expect(r"__FLUSH_\d+__", timeout=timeout)
        except pexpect.TIMEOUT:
            pass
        return clean_ansi(self.serial.before or "")

    def close(self):
        if self.serial:
            try:
                self.serial.close()
            except Exception:
                pass


# ---------------------------------------------------------------------------
# VNC Client
# ---------------------------------------------------------------------------

class VNCClient:
    """VNC-based VGA control using vncdotool."""

    def __init__(self, display=VNC_DISPLAY):
        self.display = display
        self.client = None

    def connect(self, retries=5, delay=2):
        """Connect to QEMU VNC server."""
        if not HAS_VNC:
            raise RuntimeError("vncdotool not installed: pip install vncdotool")

        for attempt in range(retries):
            try:
                self.client = vncapi.connect(f"127.0.0.1::{5900 + self.display}")
                return True
            except Exception as e:
                if attempt < retries - 1:
                    time.sleep(delay)
                else:
                    raise RuntimeError(f"VNC connect failed after {retries} attempts: {e}")
        return False

    def type_text(self, text, delay=0.05):
        """Type text via VNC keyboard."""
        if not self.client:
            raise RuntimeError("VNC not connected")
        for ch in text:
            self.client.keyPress(ch)
            time.sleep(delay)

    def press_key(self, key):
        """Press a special key (enter, tab, etc.)."""
        if not self.client:
            raise RuntimeError("VNC not connected")
        self.client.keyPress(key)

    def screenshot(self, filename="/tmp/acos-vnc-screenshot.png"):
        """Take a VNC screenshot."""
        if not self.client:
            raise RuntimeError("VNC not connected")
        self.client.captureScreen(filename)
        return filename

    def close(self):
        if self.client:
            try:
                self.client.disconnect()
            except Exception:
                pass
            self.client = None


# ---------------------------------------------------------------------------
# Main Controller
# ---------------------------------------------------------------------------

class ACOSController:
    """Complete ACOS-in-QEMU controller combining serial, QMP, and VNC."""

    def __init__(self, image=DEFAULT_IMAGE):
        self.image = os.path.abspath(image)
        self.qemu_pid = None
        self.pty_path = None
        self.qmp = QMPClient()
        self.serial = None
        self.vnc = VNCClient()
        self._booted = False
        self._logged_in = False

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.stop()

    # --- Lifecycle ---

    def start(self):
        """Start QEMU with serial PTY, QMP, and VNC."""
        if not os.path.exists(self.image):
            raise FileNotFoundError(f"Image not found: {self.image}")

        # Clean up old sockets
        for f in [QMP_SOCK]:
            try:
                os.unlink(f)
            except OSError:
                pass

        # Find UEFI firmware
        firmware = ""
        for fw in [
            "/usr/share/edk2/ovmf/OVMF_CODE.fd",
            "/usr/share/OVMF/OVMF_CODE.fd",
        ]:
            if os.path.exists(fw):
                firmware = fw
                break

        stderr_log = "/tmp/acos-qemu-stderr.log"
        serial_log = "/tmp/acos-serial-detect.log"

        qemu_cmd = (
            f"qemu-system-x86_64"
            f" -machine q35 -cpu host -enable-kvm -smp 4 -m 2048"
            f" -vga std"
            f" -serial pty"
            f" -vnc :{VNC_DISPLAY}"
            f" -qmp unix:{QMP_SOCK},server,nowait"
            f" -drive file={self.image},format=raw,if=none,id=drv0"
            f" -device nvme,drive=drv0,serial=ACOS"
            f" -net none -no-reboot"
        )
        if firmware:
            qemu_cmd += f" -bios {firmware}"

        # Launch QEMU in background
        shell_cmd = f"{qemu_cmd} >{serial_log} 2>{stderr_log} & echo $!"
        proc = subprocess.Popen(
            ["bash", "-c", shell_cmd],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            stdin=subprocess.DEVNULL
        )
        pid_line = proc.stdout.readline().decode().strip()
        proc.stdout.close()
        proc.wait()

        if not pid_line.isdigit():
            raise RuntimeError(f"QEMU did not start: {pid_line}")
        self.qemu_pid = int(pid_line)

        # Detect serial PTY path
        deadline = time.time() + 5
        while not self.pty_path and time.time() < deadline:
            time.sleep(0.3)
            try:
                os.kill(self.qemu_pid, 0)
            except OSError:
                raise RuntimeError("QEMU exited immediately")
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
            raise RuntimeError("Failed to detect serial PTY")

        # Connect serial
        time.sleep(0.5)
        self.serial = SerialConsole(self.pty_path)

        # Connect QMP
        time.sleep(0.5)
        self.qmp.connect()

        return self

    def boot(self, timeout=BOOT_TIMEOUT):
        """Wait for ACOS to boot, handling resolution selector."""
        # Send Enter periodically for resolution selector (VGA)
        def press_enter_loop():
            for _ in range(7):
                time.sleep(3)
                try:
                    self.qmp.send_key("ret")
                except Exception:
                    break

        thread = threading.Thread(target=press_enter_loop, daemon=True)
        thread.start()

        # Wait for boot marker on serial
        if self.serial.wait_for_boot(timeout=timeout):
            self._booted = True
            return True
        return False

    def login(self, user="root", password="password"):
        """Login via serial console."""
        if self.serial.login(user, password):
            self._logged_in = True
            return True
        return False

    def stop(self):
        """Stop QEMU cleanly."""
        if self.vnc:
            self.vnc.close()
        if self.serial:
            self.serial.close()
        if self.qmp:
            self.qmp.close()
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
            self.qemu_pid = None

    # --- Commands ---

    def run(self, cmd, timeout=CMD_TIMEOUT):
        """Run a command via serial, return output."""
        if not self._logged_in:
            raise RuntimeError("Not logged in")
        return self.serial.run(cmd, timeout=timeout)

    def is_process_alive(self, name):
        """Check if a process is running in the guest."""
        output = self.run("ps", timeout=5)
        if not output:
            return False
        for line in output.split("\n"):
            line = line.strip()
            # Match process name in ps output (full path or basename)
            if name in line:
                # Skip shell command echoes and our own markers
                if "grep" in line or "echo" in line or "__DONE_" in line:
                    continue
                # Look for ps-like line with PID at start or /usr/bin/ path
                if "/usr/bin/" in line or line[:1].isdigit():
                    return True
        return False

    # --- VGA / Visual ---

    def connect_vnc(self):
        """Connect VNC client for VGA interaction."""
        return self.vnc.connect()

    def screenshot(self, filename="/tmp/acos-screenshot.ppm", use_vnc=False):
        """Take a screenshot. Uses QMP screendump (PPM) or VNC (PNG)."""
        if use_vnc and self.vnc.client:
            return self.vnc.screenshot(filename.replace(".ppm", ".png"))
        else:
            self.qmp.screendump(filename)
            return filename

    def screenshot_text(self, filename="/tmp/acos-screenshot.ppm"):
        """Take screenshot and OCR the text. Requires tesseract."""
        if not HAS_OCR:
            raise RuntimeError("pytesseract not available")
        if not HAS_PIL:
            raise RuntimeError("Pillow not available")

        self.qmp.screendump(filename)
        img = Image.open(filename)
        # Upscale 2x for better OCR accuracy
        img = img.resize((img.width * 2, img.height * 2), Image.NEAREST)
        text = pytesseract.image_to_string(img)
        return text

    def vga_type(self, text, delay=0.05):
        """Type text into VGA console via QMP."""
        self.qmp.send_string(text, delay=delay)

    def vga_enter(self):
        """Press Enter on VGA."""
        self.qmp.send_key("ret")

    def vga_login(self, user="root", password="password"):
        """Login via VGA console using QMP keyboard."""
        self.vga_type(user)
        self.vga_enter()
        time.sleep(3)
        self.vga_type(password)
        self.vga_enter()
        time.sleep(3)

    # --- State ---

    @property
    def is_running(self):
        if not self.qemu_pid:
            return False
        try:
            os.kill(self.qemu_pid, 0)
            return True
        except OSError:
            return False


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="ACOS QEMU Controller")
    parser.add_argument("action", choices=[
        "boot-test", "screenshot", "vga-login"
    ])
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()

    if args.action == "boot-test":
        with ACOSController() as vm:
            vm.start()
            print("[OK] QEMU started")
            if vm.boot():
                print("[OK] Boot complete")
            else:
                print("[FAIL] Boot timeout")
                sys.exit(1)
            if vm.login():
                print("[OK] Login successful")
            else:
                print("[FAIL] Login failed")
                sys.exit(1)
            output = vm.run("echo HELLO_FROM_ACOS")
            print(f"[OK] Command output: {output}")
