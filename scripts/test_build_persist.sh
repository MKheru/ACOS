#!/bin/bash
# Test: after `make image`, do all 5 ACOS binaries exist in the image?
# Metric: SCORE=N (0-5) — number of binaries found
#
# This test mounts the freshly-built image and checks for each binary.
# It does NOT rebuild — run `make image` first.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
REDOX_DIR="$PROJECT_DIR/redox_base"
IMAGE="$REDOX_DIR/build/x86_64/acos-bare/harddrive.img"
REDOXFS="$REDOX_DIR/build/fstools/bin/redoxfs"
MOUNT="/tmp/acos_persist_test_$$"

BINARIES=(mcpd mcp-query mcp-talk acos-guardian acos-mux)

if [ ! -f "$IMAGE" ]; then
    echo "ERROR: No image at $IMAGE"
    echo "SCORE=0"
    exit 0
fi

# Cleanup stale
pkill -f "qemu-system.*acos-bare" 2>/dev/null || true
pkill -f "redoxfs.*harddrive" 2>/dev/null || true
sleep 1

# Mount
mkdir -p "$MOUNT"
"$REDOXFS" "$IMAGE" "$MOUNT" &
MOUNT_PID=$!
sleep 3

if [ ! -d "$MOUNT/usr/bin" ]; then
    echo "ERROR: Mount failed"
    kill "$MOUNT_PID" 2>/dev/null || true
    echo "SCORE=0"
    exit 0
fi

# Check binaries
SCORE=0
for bin in "${BINARIES[@]}"; do
    if [ -f "$MOUNT/usr/bin/$bin" ]; then
        SIZE=$(stat -c%s "$MOUNT/usr/bin/$bin")
        echo "  OK $bin ($((SIZE/1024))K)"
        SCORE=$((SCORE + 1))
    else
        echo "  MISSING $bin"
    fi
done

# Also check branding
if [ -f "$MOUNT/etc/issue" ]; then
    ISSUE=$(cat "$MOUNT/etc/issue")
    if echo "$ISSUE" | grep -q "ACOS"; then
        echo "  Branding: ACOS (correct)"
    elif echo "$ISSUE" | grep -q "Redox"; then
        echo "  Branding: Redox (WRONG)"
    fi
fi

# Unmount
fusermount3 -u "$MOUNT"
wait "$MOUNT_PID" 2>/dev/null || true
rmdir "$MOUNT" 2>/dev/null || true

echo ""
echo "SCORE=$SCORE"
