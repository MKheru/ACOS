#!/usr/bin/env bash
#
# ACOS QEMU Image Injector
#
# Injects mcpd binary + autotest init script into ACOS disk image.
# Called by autoresearch.sh for QEMU-type labs.
#
# Usage: ./harness/qemu_inject.sh <lab_id> <round>
#
# Exit codes:
#   0 = injection successful
#   1 = mount failed
#   2 = autotest generation failed
#   3 = mcpd binary not found

set -euo pipefail

LAB_ID="${1:?Usage: $0 <lab_id> <round>}"
ROUND="${2:?Usage: $0 <lab_id> <round>}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

IMAGE_PATH="$PROJECT_DIR/redox_base/build/x86_64/acos-bare/harddrive.img"
MCPD_BINARY="$PROJECT_DIR/redox_base/recipes/other/mcpd/source/target/x86_64-unknown-redox/release/mcpd"
MOUNT_DIR="/tmp/acos_mount"
SERIAL_LOG="/tmp/acos_serial_${LAB_ID}.log"

MOUNTED=false

cleanup() {
    if [ "$MOUNTED" = true ]; then
        fusermount3 -u "$MOUNT_DIR" 2>/dev/null || true
        MOUNTED=false
    fi
}
trap cleanup EXIT

# --- Serial log rotation (keep last 5) ---
for i in 4 3 2 1; do
    [ -f "${SERIAL_LOG}.${i}" ] && mv "${SERIAL_LOG}.${i}" "${SERIAL_LOG}.$((i+1))"
done
[ -f "${SERIAL_LOG}" ] && mv "${SERIAL_LOG}" "${SERIAL_LOG}.1"

# --- Pre-flight checks ---
if [ ! -f "$IMAGE_PATH" ]; then
    echo "ERROR: Image not found: $IMAGE_PATH" >&2
    exit 1
fi

if [ ! -f "$MCPD_BINARY" ]; then
    echo "ERROR: mcpd binary not found: $MCPD_BINARY" >&2
    exit 3
fi

# --- Mount image ---
mkdir -p "$MOUNT_DIR"
"$PROJECT_DIR/redox_base/build/fstools/bin/redoxfs" "$IMAGE_PATH" "$MOUNT_DIR" &
REDOXFS_PID=$!
# Poll for mount (up to 10 attempts, 500ms each)
for i in $(seq 1 10); do
    if [ -d "$MOUNT_DIR/usr" ]; then
        MOUNTED=true
        break
    fi
    sleep 0.5
done
if [ "$MOUNTED" != "true" ]; then
    echo "ERROR: Failed to mount image after 5s" >&2
    kill "$REDOXFS_PID" 2>/dev/null
    exit 1
fi

# --- Inject mcpd binary ---
cp "$MCPD_BINARY" "$MOUNT_DIR/usr/bin/mcpd"

# --- Generate and inject autotest init script ---
AUTOTEST_CONTENT=$(python3 "$PROJECT_DIR/harness/parse_lab.py" "$LAB_ID" autotest)
if [ $? -ne 0 ]; then
    echo "ERROR: Failed to generate autotest script for lab '$LAB_ID'" >&2
    fusermount3 -u "$MOUNT_DIR"
    MOUNTED=false
    exit 2
fi

mkdir -p "$MOUNT_DIR/usr/lib/init.d"
printf '%s\n' "$AUTOTEST_CONTENT" > "$MOUNT_DIR/usr/lib/init.d/98_autotest"
chmod +x "$MOUNT_DIR/usr/lib/init.d/98_autotest"

# --- Unmount cleanly ---
fusermount3 -u "$MOUNT_DIR"
MOUNTED=false

echo "INJECT_OK:lab_id=${LAB_ID},round=${ROUND}"
