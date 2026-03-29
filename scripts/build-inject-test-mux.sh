#!/bin/bash
# Build acos-mux in podman, inject into QEMU image, then run qemu-test.py
# Used as the metric command for the autoresearch lab.
#
# Usage: ./scripts/build-inject-test-mux.sh
# Output: SCORE=N (0-3)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
REDOX_DIR="$PROJECT_DIR/redox_base"
IMAGE="$REDOX_DIR/build/x86_64/acos-bare/harddrive.img"
MUX_SOURCE="$REDOX_DIR/recipes/other/mcpd/source/acos_mux"
MOUNT_POINT="/tmp/acos_mount"
BINARY="$MUX_SOURCE/target/x86_64-unknown-redox/release/acos-mux"

echo "=== Phase 1: Cross-compile acos-mux ==="

cd "$REDOX_DIR"
podman run --rm --cap-add SYS_ADMIN --device /dev/fuse --network=host \
  --volume "$(pwd):/mnt/redox:Z" --volume "$(pwd)/build/podman:/root:Z" \
  --workdir /mnt/redox/recipes/other/mcpd/source/acos_mux redox-base bash -c '
    export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
    export RUSTUP_TOOLCHAIN=redox
    cargo build --release --target x86_64-unknown-redox -p acos-mux --features acos 2>&1
  '

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Build failed — binary not found at $BINARY"
    echo "SCORE=0"
    exit 0
fi

echo "=== Phase 2: Inject into QEMU image ==="

# Kill any existing QEMU using this image
pkill -f "qemu-system.*acos-bare" 2>/dev/null || true
sleep 1

# Mount the RedoxFS image
mkdir -p "$MOUNT_POINT"
"$REDOX_DIR/build/fstools/bin/redoxfs" "$IMAGE" "$MOUNT_POINT" &
MOUNT_PID=$!
sleep 3

if [ ! -d "$MOUNT_POINT/usr/bin" ]; then
    echo "ERROR: Failed to mount RedoxFS image"
    kill "$MOUNT_PID" 2>/dev/null || true
    echo "SCORE=0"
    exit 0
fi

cp "$BINARY" "$MOUNT_POINT/usr/bin/acos-mux"
fusermount3 -u "$MOUNT_POINT"
wait "$MOUNT_PID" 2>/dev/null || true

echo "=== Phase 3: Test in QEMU ==="

cd "$PROJECT_DIR"
python3 -u scripts/qemu-test.py boot-and-test-mux 2>&1

# qemu-test.py already prints SCORE=N
