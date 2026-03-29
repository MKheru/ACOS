# ACOS QEMU Programmatic Control Guide

Complete reference for automated, headless control of ACOS inside QEMU.
All interaction is programmatic — no manual window interaction required.

**Status:** Fully operational (10/10 test scenarios pass).
**Last validated:** 2026-03-26.

---

## 1. Architecture

```
Host (Fedora / Bazzite)
┌──────────────────────────────────────────────┐
│                                              │
│  Python (acos_qemu.py)                       │
│  ┌────────────┐ ┌──────────┐ ┌────────────┐ │
│  │SerialConsole│ │ QMPClient│ │ VNCClient  │ │
│  │ (pexpect)  │ │ (socket) │ │(vncdotool) │ │
│  └─────┬──────┘ └────┬─────┘ └─────┬──────┘ │
│        │              │              │        │
│   /dev/pts/X    /tmp/acos-qmp    :5942       │
│        │              │              │        │
│  ┌─────┴──────────────┴──────────────┴──────┐│
│  │         qemu-system-x86_64               ││
│  │         (background process)             ││
│  └──────────────────────────────────────────┘│
│                     │                         │
│              harddrive.img                    │
│                     │                         │
│  ┌──────────────────┴───────────────────────┐│
│  │            ACOS (Redox guest)            ││
│  │  ┌──────────┐  ┌──────────┐             ││
│  │  │  Serial  │  │   VGA    │             ││
│  │  │  Console │  │  Console │             ││
│  │  │ (ttyS0)  │  │ (tty1)   │             ││
│  │  └──────────┘  └──────────┘             ││
│  └──────────────────────────────────────────┘│
└──────────────────────────────────────────────┘
```

**Three independent control channels:**

| Channel | Interface | Primary use | Read output? |
|---------|-----------|-------------|-------------|
| **Serial PTY** | pexpect on `/dev/pts/X` | Shell login, command execution, text I/O | Yes (text) |
| **QMP** | Unix socket JSON-RPC | VM lifecycle, screendump, keyboard to VGA | No (control only) |
| **VNC** | vncdotool on port 5942 | Screenshots, keyboard/mouse to VGA | Yes (pixels) |

The Serial and VGA are **separate TTYs** inside the guest. Logging into
one does not affect the other.

---

## 2. Technology Stack

### Validated packages (all working)

| Package | Version | Install | Purpose |
|---------|---------|---------|---------|
| `pexpect` | 4.9+ | pre-installed | Serial PTY interaction |
| `Pillow` | 12.1+ | pre-installed | Image processing |
| `vncdotool` | 1.2.0 | `pip install vncdotool` | VNC client (screenshots + keyboard) |
| `pytesseract` | 0.3.13 | `pip install pytesseract` | OCR wrapper (optional) |

### System requirements

| Component | Package | Notes |
|-----------|---------|-------|
| QEMU | `qemu-system-x86_64` | KVM support required (`-enable-kvm`) |
| UEFI firmware | `edk2-ovmf` | `OVMF_CODE.fd` for UEFI boot |
| FUSE 3 | `fuse3` | For RedoxFS image mounting |
| Podman | `podman` | Cross-compilation container |
| Tesseract | `tesseract-ocr` | OCR binary (optional, not on Bazzite) |

### Alternatives evaluated but not used

| Solution | Why not |
|----------|---------|
| `qemu.qmp` (PyPI) | Async-only API, overkill for sync use. Raw socket is simpler |
| `pyvnc` / `asyncvnc` | Newer but less tested than vncdotool |
| `avocado-vt` | Enterprise framework, massive overkill for ACOS |
| `virtio-console` | Redox doesn't have the driver |

---

## 3. Quick Start

### Minimal usage

```python
from acos_qemu import ACOSController

with ACOSController() as vm:
    vm.start()                          # Launch QEMU in background
    vm.boot()                           # Wait for boot (handles resolution selector)
    vm.login()                          # Login via serial (root/password)
    output = vm.run("echo hello")       # Run command, get output
    vm.screenshot("/tmp/screen.ppm")    # Take VGA screenshot
    # vm.stop() is automatic via __exit__
```

### From the command line

```bash
# Full boot test
python3 scripts/acos_qemu.py boot-test

# Run the test suite (10 scenarios)
timeout 180 python3 -u scripts/test_qemu_controller.py

# Build, inject binary, and test acos-mux
bash scripts/build-inject-test-mux.sh
```

---

## 4. QEMU Launch Configuration

### Command line (used by `ACOSController.start()`)

```bash
qemu-system-x86_64 \
  -machine q35                           \  # Modern chipset
  -cpu host -enable-kvm                  \  # KVM passthrough (required)
  -smp 4 -m 2048                         \  # 4 vCPUs, 2GB RAM
  -vga std                               \  # Standard VGA
  -serial pty                            \  # Serial → host PTY (auto /dev/pts/X)
  -vnc :42                               \  # VNC on port 5942 (display :42)
  -qmp unix:/tmp/acos-qmp.sock,server,nowait \  # QMP control socket
  -drive file=harddrive.img,format=raw,if=none,id=drv0 \
  -device nvme,drive=drv0,serial=ACOS    \  # NVMe storage
  -net none -no-reboot                   \  # No network, no auto-reboot
  -bios /usr/share/edk2/ovmf/OVMF_CODE.fd  # UEFI firmware
```

### Key configuration choices

| Flag | Why |
|------|-----|
| `-serial pty` | Auto-assigns `/dev/pts/X`, detected from QEMU stderr |
| `-vnc :42` | Display :42 = port 5942, avoids conflict with desktop VNC |
| `-qmp unix:...,server,nowait` | Persistent socket, no client needed to start |
| `-enable-kvm` | Required — ACOS is too slow without hardware virtualization |
| `-no-reboot` | Prevents reboot loops on crash |

### For fully headless operation (no VGA window)

Add `-display none` to suppress the QEMU window entirely. VNC and
serial still work. Useful for CI/CD.

---

## 5. Boot Sequence & Timing

```
 Time   Serial output              VGA screen               Action needed
──────┬──────────────────────────┬──────────────────────────┬─────────────────────
 0s   │ (nothing)               │ UEFI splash              │ Wait
 ~1s  │ "redirected to pts/X"   │ UEFI splash              │ Detect PTY path
 ~2s  │ (nothing)               │ UEFI splash              │ Connect serial + QMP
 ~5s  │ (nothing)               │ Resolution selector      │ Send Enter via QMP ①
 ~8s  │ (nothing)               │ "Booting from disk..."   │ —
~15s  │ Boot messages            │ Kernel messages          │ —
~25s  │ "ACOS_BOOT_OK"          │ Login prompt             │ Boot marker detected
~27s  │ "acos login:"           │ "acos login:"            │ Wait for login: ②
~30s  │ Shell prompt             │ Shell prompt             │ Login complete
```

### ① Resolution Selector (critical)

The ACOS bootloader shows a resolution selector on VGA that **blocks boot**.
The serial console receives nothing until Enter is pressed on VGA.

**Solution:** background thread sends Enter via QMP every 3 seconds:

```python
def press_enter_loop(qmp):
    for _ in range(7):
        time.sleep(3)
        try:
            qmp.send_key("ret")
        except Exception:
            break

thread = threading.Thread(target=press_enter_loop, args=(qmp,), daemon=True)
thread.start()
```

### ② Login prompt race

After `ACOS_BOOT_OK` appears on serial, the `login:` prompt may take
1-2 more seconds. **Always `expect("login:")` before sending username.**

---

## 6. Channel Reference

### 6.1 Serial Console — `SerialConsole`

Primary channel for text-mode shell interaction.

#### Connection

```python
from acos_qemu import SerialConsole

serial = SerialConsole("/dev/pts/8")  # PTY path from QEMU startup
```

#### Login

```python
serial.login(user="root", password="password")
# Internally: wait for "login:" → send user → wait for "Password:" →
# send password → wait for shell prompt → verify with "echo SHELL_READY"
```

#### Running commands

```python
output = serial.run("echo hello")
# Returns cleaned text output (ANSI/OSC stripped, prompt echoes filtered)
```

**How it works internally:**
1. `sendline(cmd)` — sends the command
2. `sendline("echo __DONE_XXXXX__")` — sends a unique end marker
3. `expect(marker)` — waits for the marker in output
4. Cleans `serial.before` (remove ANSI, OSC, prompt echoes)
5. Returns filtered lines

#### Raw access

```python
serial.sendline("ps")        # Send raw text
serial.expect("pattern")     # Wait for pattern
text = serial.read_all()     # Flush and read all buffered text
```

#### Ion shell compatibility rules

| DO | DON'T |
|----|-------|
| `cmd >/dev/null` | `cmd 2>/dev/null` |
| `cmd &` (background) | `cmd 2>&1` |
| `cmd1; cmd2` (chain) | Heredocs (`<<EOF`) |
| `cmd \| cmd2` (pipe) | Process substitution (`<()`) |
| `echo $VAR` | Arrays (`${arr[@]}`) |

Ion outputs `ion: bg [N] PID` for backgrounded jobs and
`ion: syntax error: ...` for unsupported bash-isms.

#### Serial echo behavior

Ion echoes **every character** with an OSC title update prefix:

```
]0;root: /home/root\x07root:~# e
]0;root: /home/root\x07root:~# ec
]0;root: /home/root\x07root:~# ech
]0;root: /home/root\x07root:~# echo
```

The `clean_ansi()` function strips these. After cleaning, filter lines
containing `root:` + `#` to remove prompt echoes.

### 6.2 QMP — `QMPClient`

Machine-level control via JSON-RPC over Unix socket.

#### Connection

```python
from acos_qemu import QMPClient

qmp = QMPClient("/tmp/acos-qmp.sock")
qmp.connect()  # Handshake: greeting → qmp_capabilities
```

#### Key operations

```python
# Query VM status
status = qmp.execute("query-status")
# → {"return": {"running": true, "status": "running"}}

# Send a single key to VGA
qmp.send_key("ret")            # Enter
qmp.send_key("a")              # Letter 'a'

# Send key combination (e.g., Shift+A)
qmp.send_keys("shift", "a")

# Type a full string (character by character, 50ms delay)
qmp.send_string("root", delay=0.05)

# Take VGA screenshot (PPM format)
qmp.screendump("/tmp/screenshot.ppm")

# Clean shutdown
qmp.execute("system_powerdown")

# Force quit
qmp.execute("quit")
```

#### QMP keyboard mapping

```python
# Full keymap in QMPClient.send_string():
KEYMAP = {
    "a"-"z": "a"-"z",     "0"-"9": "0"-"9",
    " ": "spc",           "/": "slash",
    "-": "minus",         "=": "equal",
    ".": "dot",           ",": "comma",
    ";": "semicolon",     "'": "apostrophe",
    # Shift combos: "_", "+", ":", '"', "~", "{", "}", "|",
    # "!", "@", "#", "$", "%", "^", "&", "*", "(", ")",
    # "<", ">", "?"
    # Uppercase: send_keys("shift", letter)
}

# Special keys (use send_key directly):
# "ret" (Enter), "tab", "esc", "backspace", "delete",
# "up", "down", "left", "right",
# "f1"-"f12", "home", "end", "pgup", "pgdn",
# "ctrl", "alt", "shift", "caps_lock"
```

#### QMP screendump format

```python
# PPM output (default)
qmp.screendump("/tmp/screen.ppm")

# Convert to PNG with Pillow:
from PIL import Image
img = Image.open("/tmp/screen.ppm")
img.save("/tmp/screen.png")
```

The PPM file is uncompressed (~2-5 MB for 1024x768). For automated
comparison, convert to PNG first.

### 6.3 VNC — `VNCClient`

Pixel-level VGA access via the RFB protocol.

#### Connection

```python
from acos_qemu import VNCClient

vnc = VNCClient(display=42)       # QEMU started with -vnc :42
vnc.connect(retries=5, delay=2)   # Retry if QEMU isn't ready yet
```

#### Operations

```python
# Type text (character by character)
vnc.type_text("root", delay=0.05)

# Press special key
vnc.press_key("enter")
vnc.press_key("tab")

# Take screenshot (PNG)
vnc.screenshot("/tmp/vnc-screen.png")
```

#### VNC vs QMP for keyboard input

| | QMP `send-key` | VNC `keyPress` |
|--|----------------|----------------|
| Speed | ~50ms per key | ~50ms per key |
| Reliability | High (JSON-RPC) | Good (RFB protocol) |
| Key combos | Yes (`send_keys`) | Yes (`keyDown`/`keyUp`) |
| Feedback | None | Can screenshot after |
| Best for | Boot phase (before VNC is ready) | After boot (with visual verification) |

**Recommendation:** use QMP for keyboard during boot (simpler, no VNC
connection needed). Use VNC when you need screenshots to verify results.

---

## 7. VGA Login (typing into the QEMU window)

When you need to interact with the VGA console (not serial):

```python
with ACOSController() as vm:
    vm.start()
    vm.boot()
    # Login via VGA (types into the QEMU window)
    vm.vga_login(user="root", password="password")
    # Take screenshot to verify
    vm.screenshot("/tmp/after-login.ppm")
```

**vga_login internals:**
1. `qmp.send_string("root")` — types "root" character by character (50ms/char)
2. `qmp.send_key("ret")` — presses Enter
3. `time.sleep(3)` — waits for password prompt
4. `qmp.send_string("password")` — types password
5. `qmp.send_key("ret")` — presses Enter
6. `time.sleep(3)` — waits for shell

**Note:** VGA login has no output feedback. Take a screenshot after to
verify success. The serial login is more reliable for automated tests.

---

## 8. Screenshot & Visual Verification

### Take a screenshot

```python
# Via QMP (always available, PPM format)
vm.screenshot("/tmp/screen.ppm")

# Via VNC (requires VNC connection, PNG format)
vm.connect_vnc()
vm.screenshot("/tmp/screen.png", use_vnc=True)
```

### OCR text from screenshot (when tesseract is available)

```python
text = vm.screenshot_text()  # Returns OCR'd text from VGA
if "acos login:" in text:
    print("Login prompt visible on VGA")
```

**OCR tips:**
- Upscale 2x before OCR for better accuracy
- Terminal fonts are monospaced and high-contrast — Tesseract handles them well
- OCR is ~100-500ms per screenshot

### Pixel comparison (without OCR)

```python
from PIL import Image

img = Image.open("/tmp/screen.ppm")
# Check if screen is not entirely black (boot in progress)
pixels = list(img.getdata())
non_black = sum(1 for r, g, b in pixels if r + g + b > 30)
is_booted = non_black > len(pixels) * 0.01  # >1% non-black pixels
```

---

## 9. Binary Injection Pipeline

For modifying and testing ACOS binaries:

```
┌─────────────────┐    ┌─────────────────┐    ┌───────────────┐
│ 1. Cross-compile│───>│ 2. Inject into  │───>│ 3. Boot QEMU  │
│ (podman+redox)  │    │    image (FUSE) │    │    & test      │
└─────────────────┘    └─────────────────┘    └───────────────┘
     ~20s                    ~5s                   ~90s
```

### Step 1: Cross-compile

```bash
cd redox_base
podman run --rm --cap-add SYS_ADMIN --device /dev/fuse --network=host \
  --volume "$(pwd):/mnt/redox:Z" --volume "$(pwd)/build/podman:/root:Z" \
  --workdir /mnt/redox/recipes/other/mcpd/source/acos_mux \
  redox-base bash -c '
    export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
    export RUSTUP_TOOLCHAIN=redox
    cargo build --release --target x86_64-unknown-redox -p acos-mux --features acos
  '
```

### Step 2: Inject

```bash
IMAGE="redox_base/build/x86_64/acos-bare/harddrive.img"
MOUNT="/tmp/acos_mount"

mkdir -p "$MOUNT"
redox_base/build/fstools/bin/redoxfs "$IMAGE" "$MOUNT" &
sleep 3
cp target/x86_64-unknown-redox/release/acos-mux "$MOUNT/usr/bin/acos-mux"
fusermount3 -u "$MOUNT"
wait
```

### Step 3: Test

```bash
python3 scripts/acos_qemu.py boot-test
# or
bash scripts/build-inject-test-mux.sh  # does all 3 steps
```

### Image integrity

The RedoxFS disk image uses a **Merkle hash tree**. Unclean QEMU
shutdowns corrupt it:

```
ERROR redoxfs: READ_BLOCK: INCORRECT HASH 0xBFAC... != 0x16CC...
```

When this happens, binary injection **silently fails** — the old
binary stays. QEMU still boots fine (read-only integrity is OK).

**Recovery:** `cd redox_base && make image` (~30s rebuild).

**Prevention:** always stop QEMU with SIGTERM (the controller does this).

---

## 10. Filesystem Notes (Guest)

| Guest path | Scheme | Persistent across reboot? | Writable from Rust? | Notes |
|------------|--------|--------------------------|--------------------|----|
| `/usr/bin/` | RedoxFS | Yes | Via FUSE mount from host | Binary location |
| `/tmp/` | `tmp:` | No (RAM-backed) | Unreliable | `std::fs::write` may silently fail |
| `/home/root/` | RedoxFS | On disk, not at runtime | No | Exists on image but not writable in guest |
| `/scheme/pty` | `pty:` | N/A | N/A | Open for PTY master allocation |
| `/dev/null` | `null:` | N/A | N/A | Works for stdout redirect |

**Key lesson:** for in-guest debugging, use `eprintln!()` (stderr to
serial) instead of file writes. File writes to `/tmp/` and `/home/`
are unreliable on Redox.

---

## 11. Debugging Patterns

### Capture stderr from a background process

```bash
# ion-compatible (NEVER use 2> or 2>&1):
EMUX_LOG=/tmp/m.log acos-mux >/dev/null &
# Stderr goes to serial → visible via pexpect
# Stdout goes to /dev/null → doesn't pollute serial
```

### Read stderr via serial

```python
vm.serial.sendline("EMUX_LOG=/tmp/m.log acos-mux >/dev/null &")
time.sleep(5)
# Capture everything since launch
vm.serial.sendline("echo CAPTURE_DONE")
vm.serial.expect("CAPTURE_DONE", timeout=10)
stderr_text = clean_ansi(vm.serial.serial.before)
for line in stderr_text.split('\n'):
    if 'acos-mux' in line or 'error' in line.lower():
        print(f"[MUX] {line.strip()}")
```

### Check process status

```python
alive = vm.is_process_alive("acos-mux")
# Internally: sends "ps", reads output, filters for process name
# Filters OUT: grep, echo, >, EMUX (avoids matching command echoes)
```

### Visual verification via screenshot

```python
vm.screenshot("/tmp/before.ppm")
vm.run("acos-mux >/dev/null &")
time.sleep(5)
vm.screenshot("/tmp/after.ppm")
# Compare images to verify mux rendered something
```

---

## 12. Checklists

### Before running a QEMU test

- [ ] No stale FUSE mounts: `mount | grep acos` → empty
- [ ] No orphaned QEMU: `pgrep -f "qemu-system.*acos-bare"` → empty
- [ ] No orphaned redoxfs: `pgrep -f "redoxfs.*harddrive"` → empty
- [ ] Image healthy: mount succeeds without `INCORRECT HASH`
- [ ] Binary recompiled: check `Compiling` in build output (not just `Finished`)
- [ ] Ion syntax: no `2>` or `2>&1` in any guest command

### After an unclean QEMU shutdown

1. Kill orphans: `pkill -f qemu-system; pkill -f redoxfs`
2. Unmount: `fusermount3 -uz /tmp/acos_mount`
3. Wait: `sleep 2`
4. Rebuild image: `cd redox_base && make image`
5. Verify mount works without hash errors
6. Re-inject binary and re-test

### Cleanup one-liner

```bash
pkill -f qemu-system; pkill -f redoxfs; fusermount3 -uz /tmp/acos_mount 2>/dev/null; sleep 2
```

---

## 13. Test Suite

The test suite validates all 10 controller capabilities:

```bash
timeout 180 python3 -u scripts/test_qemu_controller.py
```

| # | Scenario | What it validates |
|---|----------|-------------------|
| 1 | QEMU start | Process launches, PTY detected |
| 2 | QMP connect | Socket connection, capabilities handshake |
| 3 | Boot | ACOS_BOOT_OK on serial within 90s |
| 4 | Login | Serial login (root/password), shell readiness |
| 5 | Echo command | `run("echo HELLO")` returns "HELLO" |
| 6 | Process listing | `run("ps \| head -5")` returns non-empty output |
| 7 | QMP screendump | PPM file created, >1KB size |
| 8 | VNC connect | vncdotool connects to :42 |
| 9 | VNC screenshot | PNG file created, >1KB size |
| 10 | Shutdown | SIGTERM stops QEMU, process exits |

**Expected output:**
```
=== QEMU Controller Test Results ===
  ✓ 1_qemu_start: PASS
  ✓ 2_qmp_connect: PASS
  ✓ 3_boot: PASS
  ✓ 4_login: PASS
  ✓ 5_echo_cmd: PASS
  ✓ 6_ps_listing: PASS
  ✓ 7_screendump: PASS
  ✓ 8_vnc_connect: PASS
  ✓ 9_vnc_screenshot: PASS
  ✓ 10_shutdown: PASS
SCORE=10
```

---

## 14. File Locations

```
projects/agent_centric_os/
├── scripts/
│   ├── acos_qemu.py                 # Main controller module ← USE THIS
│   ├── test_qemu_controller.py      # 10-scenario test suite
│   ├── build-inject-test-mux.sh     # Full build→inject→test pipeline
│   ├── qemu-test.py                 # Legacy serial-only test (kept for mux lab)
│   └── qemu-control.sh              # Legacy bash control (superseded)
├── redox_base/
│   ├── .config                      # CONFIG_NAME=acos-bare
│   ├── Makefile                     # `make image` to rebuild disk image
│   ├── config/acos-bare.toml        # Image package list
│   ├── build/x86_64/acos-bare/
│   │   └── harddrive.img            # 512MB disk image (NOT in git)
│   ├── build/fstools/bin/
│   │   └── redoxfs                  # FUSE mount/mkfs tool
│   └── recipes/other/mcpd/source/acos_mux/
│       ├── bins/acos-mux/src/       # Binary entry point
│       └── crates/acos-mux-pty/src/
│           └── acos_redox.rs        # Redox PTY backend
└── architecture/
    └── QEMU_TESTING_GUIDE.md        # This file
```

---

## 15. API Quick Reference

```python
from acos_qemu import ACOSController

# Lifecycle
vm = ACOSController(image="path/to/harddrive.img")
vm.start()                    # Launch QEMU, connect serial + QMP
vm.boot(timeout=90)           # Wait for boot (handles resolution selector)
vm.login(user, password)      # Login via serial
vm.stop()                     # SIGTERM → SIGKILL if needed

# Serial commands (ion-compatible)
output = vm.run("echo hello")              # Run command, get cleaned output
alive = vm.is_process_alive("acos-mux")    # Check ps for process

# VGA keyboard (via QMP)
vm.vga_type("text")                        # Type text into VGA
vm.vga_enter()                             # Press Enter on VGA
vm.vga_login(user, password)               # Full VGA login sequence

# Screenshots
vm.screenshot("/tmp/s.ppm")                # QMP screendump (PPM)
vm.screenshot("/tmp/s.png", use_vnc=True)  # VNC screenshot (PNG)
text = vm.screenshot_text()                # Screenshot + OCR (needs tesseract)

# VNC
vm.connect_vnc()                           # Connect vncdotool
vm.vnc.type_text("text")                   # Type via VNC
vm.vnc.press_key("enter")                  # Key via VNC
vm.vnc.screenshot("/tmp/s.png")            # VNC screenshot

# QMP direct
vm.qmp.execute("query-status")             # Any QMP command
vm.qmp.send_key("ret")                     # Single key
vm.qmp.send_keys("ctrl", "c")             # Key combo
vm.qmp.screendump("/tmp/s.ppm")           # Screendump

# State
vm.is_running                              # bool: QEMU process alive?
vm._booted                                 # bool: boot completed?
vm._logged_in                              # bool: serial login done?

# Context manager
with ACOSController() as vm:
    vm.start()
    vm.boot()
    vm.login()
    # ... vm.stop() called automatically
```
