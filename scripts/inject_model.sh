#!/usr/bin/env bash
#
# Inject LLM model weights into the ACOS disk image.
#
# Mounts the ACOS disk image via redoxfs, copies the model file to
# /usr/share/llm/ inside the image, then unmounts.
#
# Usage: ./scripts/inject_model.sh <model_file.gguf>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
REDOX_DIR="$PROJECT_DIR/redox_base"
IMAGE="$REDOX_DIR/build/x86_64/acos-bare/harddrive.img"
REDOXFS="$REDOX_DIR/build/fstools/bin/redoxfs"
MODEL_FILE="${1:?Usage: inject_model.sh <model_file.gguf>}"
MOUNT_DIR="/tmp/acos_mount_$$"

echo "=== Injecting LLM model into ACOS disk image ==="

if [ ! -f "$IMAGE" ]; then
    echo "ERROR: Image not found: $IMAGE"
    echo "Build the ACOS image first."
    exit 1
fi

if [ ! -f "$REDOXFS" ]; then
    echo "ERROR: redoxfs not found: $REDOXFS"
    echo "Build the fstools first."
    exit 1
fi

if [ ! -f "$MODEL_FILE" ]; then
    echo "ERROR: Model file not found: $MODEL_FILE"
    exit 1
fi

mkdir -p "$MOUNT_DIR"

echo "Mounting $IMAGE at $MOUNT_DIR ..."
"$REDOXFS" "$IMAGE" "$MOUNT_DIR" &
REDOXFS_PID=$!
sleep 2

# Verify mount succeeded
if [ ! -d "$MOUNT_DIR/usr" ]; then
    echo "ERROR: Mount failed — /usr not found in mount dir"
    kill "$REDOXFS_PID" 2>/dev/null || true
    rmdir "$MOUNT_DIR"
    exit 1
fi

mkdir -p "$MOUNT_DIR/usr/share/llm"
cp "$MODEL_FILE" "$MOUNT_DIR/usr/share/llm/"
echo "Injected $(basename "$MODEL_FILE") into /usr/share/llm/"

echo "Unmounting ..."
fusermount3 -u "$MOUNT_DIR"
rmdir "$MOUNT_DIR"

echo "=== Done ==="
