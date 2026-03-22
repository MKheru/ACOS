#!/usr/bin/env bash
#
# build_offline.sh — Complete offline build pipeline for ACOS
#
# Usage:
#   ./scripts/build_offline.sh cache   # Download everything needed for offline builds
#   ./scripts/build_offline.sh build   # Build the complete ACOS image offline
#   ./scripts/build_offline.sh test    # Run QEMU boot test on existing image
#
# The script must be run from the project root:
#   /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os/

set -euo pipefail

# --- Colors ---
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

ok()   { echo -e "${GREEN}[OK]${NC} $*"; }
fail() { echo -e "${RED}[FAIL]${NC} $*" >&2; exit 1; }
info() { echo -e "${YELLOW}[INFO]${NC} $*"; }

# --- Paths ---
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
REDOX_DIR="$PROJECT_DIR/redox_base"
HARNESS_DIR="$PROJECT_DIR/harness"
CONFIG_NAME="acos-bare"
IMAGE_PATH="$REDOX_DIR/build/x86_64/$CONFIG_NAME/harddrive.img"
MCPD_BIN="$REDOX_DIR/recipes/other/mcpd/source/target/x86_64-unknown-redox/release/mcpd"
CONTAINER_TAG="$REDOX_DIR/build/container.tag"
MOUNT_DIR="/tmp/acos_mount_$$"

# Boot test timeout in seconds
BOOT_TIMEOUT=60

# ---------------------------------------------------------------------------
# Subcommand: cache
# Downloads everything needed so subsequent builds can run offline.
# ---------------------------------------------------------------------------
cmd_cache() {
    info "=== ACOS Cache: Populating offline build caches ==="

    if [ ! -d "$REDOX_DIR" ]; then
        fail "Redox base not found at $REDOX_DIR. Clone it first."
    fi

    cd "$REDOX_DIR"

    info "Running full Redox build to populate caches (this may take a while)..."
    CI=1 PODMAN_BUILD=1 make all CONFIG_NAME="$CONFIG_NAME"

    ok "Podman container image cached (tag: $CONTAINER_TAG)"

    PREFIX_SIZE=$(du -sh "$REDOX_DIR/prefix" 2>/dev/null | cut -f1 || echo "N/A")
    BUILD_SIZE=$(du -sh "$REDOX_DIR/build" 2>/dev/null | cut -f1 || echo "N/A")

    ok "=== Cache summary ==="
    ok "  prefix/ (toolchain tarballs): $PREFIX_SIZE"
    ok "  build/  (REPO_BINARY packages + image): $BUILD_SIZE"
    ok "Cache complete. Subsequent builds can run offline."
}

# ---------------------------------------------------------------------------
# Subcommand: build
# Builds the complete ACOS image offline.
# ---------------------------------------------------------------------------
cmd_build() {
    info "=== ACOS Offline Build ==="

    if [ ! -d "$REDOX_DIR" ]; then
        fail "Redox base not found at $REDOX_DIR. Run 'cache' first."
    fi

    # 1. Verify Podman container exists
    info "Step 1/5: Verifying Podman container..."
    if [ ! -f "$CONTAINER_TAG" ]; then
        fail "Podman container not found ($CONTAINER_TAG). Run '$0 cache' first."
    fi
    ok "Podman container found."

    # 2. Build base Redox image
    info "Step 2/5: Building base Redox/ACOS image..."
    cd "$REDOX_DIR"
    CI=1 PODMAN_BUILD=1 make all CONFIG_NAME="$CONFIG_NAME"
    ok "Base image built: $IMAGE_PATH"

    # 3. Cross-compile mcpd in the container
    info "Step 3/5: Cross-compiling mcpd for x86_64-unknown-redox..."
    MCPD_SRC="$REDOX_DIR/recipes/other/mcpd/source"
    if [ ! -d "$MCPD_SRC" ]; then
        fail "mcpd source not found at $MCPD_SRC"
    fi

    podman run --rm \
        --cap-add SYS_ADMIN --device /dev/fuse --network=host \
        --volume "$REDOX_DIR:/mnt/redox:Z" \
        --volume "$REDOX_DIR/build/podman:/root:Z" \
        --workdir /mnt/redox/recipes/other/mcpd/source \
        redox-base bash -c '
            export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
            export RUSTUP_TOOLCHAIN=redox
            cargo build --release --target x86_64-unknown-redox
        '

    if [ ! -f "$MCPD_BIN" ]; then
        fail "mcpd binary not found after build: $MCPD_BIN"
    fi
    ok "mcpd built: $MCPD_BIN"

    # 4. Inject mcpd into the image
    info "Step 4/5: Injecting mcpd into ACOS image..."
    _inject_mcpd
    ok "mcpd injected into image."

    # 5. Run boot test
    info "Step 5/5: Running QEMU boot test..."
    cmd_test
}

# ---------------------------------------------------------------------------
# Helper: mount image, inject mcpd + init scripts, unmount
# ---------------------------------------------------------------------------
_inject_mcpd() {
    local REDOXFS="$REDOX_DIR/build/fstools/bin/redoxfs"

    if [ ! -x "$REDOXFS" ]; then
        fail "redoxfs not found at $REDOXFS. Ensure the base build completed."
    fi

    if [ ! -f "$IMAGE_PATH" ]; then
        fail "Image not found: $IMAGE_PATH"
    fi

    mkdir -p "$MOUNT_DIR"

    # Cleanup on exit
    trap '_cleanup_mount' EXIT INT TERM

    "$REDOXFS" "$IMAGE_PATH" "$MOUNT_DIR" &
    sleep 2

    if ! mountpoint -q "$MOUNT_DIR"; then
        fail "redoxfs did not mount at $MOUNT_DIR"
    fi

    cp "$MCPD_BIN" "$MOUNT_DIR/usr/bin/mcpd"

    # MCP daemon init entry
    printf 'requires_weak 00_base\nnowait mcpd\n' > "$MOUNT_DIR/usr/lib/init.d/15_mcp"

    # Boot success marker
    printf 'echo ACOS_BOOT_OK\n' > "$MOUNT_DIR/usr/lib/init.d/99_acos_ready"

    fusermount3 -u "$MOUNT_DIR"
    trap - EXIT INT TERM
    rmdir "$MOUNT_DIR" 2>/dev/null || true
}

_cleanup_mount() {
    if mountpoint -q "$MOUNT_DIR" 2>/dev/null; then
        fusermount3 -u "$MOUNT_DIR" 2>/dev/null || true
    fi
    rmdir "$MOUNT_DIR" 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# Subcommand: test
# Runs the QEMU boot test on the existing image.
# ---------------------------------------------------------------------------
cmd_test() {
    info "=== ACOS Boot Test (timeout: ${BOOT_TIMEOUT}s) ==="

    if [ ! -f "$IMAGE_PATH" ]; then
        fail "Image not found: $IMAGE_PATH. Run '$0 build' first."
    fi

    # Use harness boot script if available
    local HARNESS_BOOT="$HARNESS_DIR/boot_test.sh"
    if [ -x "$HARNESS_BOOT" ]; then
        info "Using harness: $HARNESS_BOOT"
        "$HARNESS_BOOT" "$IMAGE_PATH"
        return
    fi

    # Fallback: direct QEMU headless boot, look for ACOS_BOOT_OK
    info "Harness not found, running QEMU directly..."
    local SERIAL_LOG
    SERIAL_LOG="$(mktemp /tmp/acos_serial_XXXXXX.log)"

    qemu-system-x86_64 \
        -drive file="$IMAGE_PATH",format=raw \
        -serial file:"$SERIAL_LOG" \
        -display none \
        -m 512M \
        -enable-kvm \
        -nographic \
        &
    local QEMU_PID=$!

    local ELAPSED=0
    local BOOT_OK=0
    while [ $ELAPSED -lt $BOOT_TIMEOUT ]; do
        if grep -q "ACOS_BOOT_OK" "$SERIAL_LOG" 2>/dev/null; then
            BOOT_OK=1
            break
        fi
        sleep 2
        ELAPSED=$((ELAPSED + 2))
    done

    kill "$QEMU_PID" 2>/dev/null || true
    wait "$QEMU_PID" 2>/dev/null || true

    if [ $BOOT_OK -eq 1 ]; then
        ok "Boot test PASSED (ACOS_BOOT_OK found in serial output)."
    else
        fail "Boot test FAILED: ACOS_BOOT_OK not seen within ${BOOT_TIMEOUT}s. Serial log: $SERIAL_LOG"
    fi

    rm -f "$SERIAL_LOG"
}

# ---------------------------------------------------------------------------
# Entrypoint
# ---------------------------------------------------------------------------
SUBCOMMAND="${1:-}"

case "$SUBCOMMAND" in
    cache) cmd_cache ;;
    build) cmd_build ;;
    test)  cmd_test  ;;
    *)
        echo "Usage: $0 {cache|build|test}"
        echo ""
        echo "  cache  Download and cache all artifacts needed for offline builds"
        echo "  build  Build the complete ACOS image offline"
        echo "  test   Run QEMU boot test on existing image"
        exit 1
        ;;
esac
