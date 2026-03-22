#!/usr/bin/env bash
#
# ACOS QEMU Headless Runner
#
# Boots a Redox OS image in QEMU headless mode, captures serial output,
# and checks for success/failure markers.
#
# Usage:
#   ./qemu_runner.sh <image_path> [timeout_seconds]
#
# Exit codes:
#   0 = boot successful (success marker found in serial output)
#   1 = boot failed (panic or timeout)

set -euo pipefail

IMAGE="${1:?Usage: $0 <image_path> [timeout_seconds]}"
TIMEOUT="${2:-120}"
SERIAL_LOG="/tmp/acos_qemu_serial_$$.log"

cleanup() {
    rm -f "$SERIAL_LOG"
    # Kill QEMU if still running
    if [ -n "${QEMU_PID:-}" ] && kill -0 "$QEMU_PID" 2>/dev/null; then
        kill "$QEMU_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

echo "=== QEMU Headless Boot ==="
echo "Image: $IMAGE"
echo "Timeout: ${TIMEOUT}s"

# Check OVMF firmware
FIRMWARE=""
for fw in /usr/share/edk2/ovmf/OVMF_CODE.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/qemu/edk2-x86_64-code.fd; do
    if [ -f "$fw" ]; then
        FIRMWARE="$fw"
        break
    fi
done

QEMU_FLAGS=(
    -nographic
    -vga none
    -machine q35
    -cpu host
    -enable-kvm
    -smp 4
    -m 2048
    -serial file:"$SERIAL_LOG"
    -drive "file=$IMAGE,format=raw,if=none,id=drv0"
    -device "nvme,drive=drv0,serial=ACOS"
    -net none
    -no-reboot
)

if [ -n "$FIRMWARE" ]; then
    QEMU_FLAGS+=(-bios "$FIRMWARE")
fi

# Start QEMU in background
qemu-system-x86_64 "${QEMU_FLAGS[@]}" &
QEMU_PID=$!

echo "QEMU PID: $QEMU_PID"

# Wait for boot, checking serial output
ELAPSED=0
BOOT_SUCCESS=false
while [ $ELAPSED -lt "$TIMEOUT" ]; do
    sleep 2
    ELAPSED=$((ELAPSED + 2))

    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "QEMU exited after ${ELAPSED}s"
        break
    fi

    if [ -f "$SERIAL_LOG" ]; then
        # Check for kernel panic
        if grep -qi "panic\|KERNEL PANIC\|triple fault" "$SERIAL_LOG" 2>/dev/null; then
            echo "KERNEL PANIC detected at ${ELAPSED}s"
            echo "--- Serial output (last 50 lines) ---"
            tail -50 "$SERIAL_LOG"
            kill "$QEMU_PID" 2>/dev/null || true
            exit 1
        fi

        # Check for success marker (ACOS/Redox login prompt or custom marker)
        if grep -qi "login:\|ACOS_BOOT_OK\|ion:" "$SERIAL_LOG" 2>/dev/null; then
            BOOT_SUCCESS=true
            echo "Boot SUCCESS at ${ELAPSED}s"
            break
        fi
    fi
done

# Kill QEMU
kill "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true

if [ "$BOOT_SUCCESS" = true ]; then
    echo "=== BOOT OK ==="
    exit 0
else
    echo "=== BOOT FAILED (timeout or no success marker) ==="
    if [ -f "$SERIAL_LOG" ]; then
        echo "--- Serial output (last 30 lines) ---"
        tail -30 "$SERIAL_LOG"
    fi
    exit 1
fi
