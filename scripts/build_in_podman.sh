#!/usr/bin/env bash
#
# Build Redox OS + ACOS components using Podman
#
# This script uses the Redox OS Podman build system to compile
# everything in an isolated container.
#
# Usage:
#   ./scripts/build_in_podman.sh [minimal|desktop]
#
# The config name determines which packages are included.
# For ACOS development, we use 'minimal' to keep build times short.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
REDOX_DIR="$PROJECT_DIR/redox_base"
CONFIG="${1:-minimal}"

echo "=== ACOS Podman Build ==="
echo "Redox dir: $REDOX_DIR"
echo "Config: $CONFIG"

if [ ! -d "$REDOX_DIR" ]; then
    echo "ERROR: Redox base not found at $REDOX_DIR"
    echo "Clone it with: git clone https://gitlab.redox-os.org/redox-os/redox.git redox_base"
    exit 1
fi

cd "$REDOX_DIR"

# Ensure Podman build is enabled
export PODMAN_BUILD=1
export CONFIG_NAME="$CONFIG"

# Build the container first if needed
echo "--- Building Podman container (if needed) ---"
make build/container.tag

# Build the minimal Redox image
echo "--- Building Redox OS ($CONFIG) ---"
make all CONFIG_NAME="$CONFIG"

echo "=== Build complete ==="
echo "Image: $REDOX_DIR/build/x86_64/$CONFIG/harddrive.img"
