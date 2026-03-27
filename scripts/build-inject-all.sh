#!/bin/bash
# Inject ALL ACOS binaries + branding into QEMU disk image.
#
# Assumes binaries are already cross-compiled (see recipe.toml build).
# Use this after `make image` to restore all ACOS components.
#
# Usage: ./scripts/build-inject-all.sh [--rebuild] [--test]
#   --rebuild  Cross-compile all binaries in podman first
#   --test     Run audit tests after injection

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
REDOX_DIR="$PROJECT_DIR/redox_base"
IMAGE="$REDOX_DIR/build/x86_64/acos-bare/harddrive.img"
REDOXFS="$REDOX_DIR/build/fstools/bin/redoxfs"
MCPD_SRC="$REDOX_DIR/recipes/other/mcpd/source"
MOUNT_POINT="/tmp/acos_inject_$$"

# Binary locations (pre-compiled)
declare -A BINARIES=(
    [mcpd]="$MCPD_SRC/target/x86_64-unknown-redox/release/mcpd"
    [mcp-query]="$MCPD_SRC/mcp_query/target/x86_64-unknown-redox/release/mcp-query"
    [mcp-talk]="$MCPD_SRC/mcp_talk/target/x86_64-unknown-redox/release/mcp-talk"
    [acos-guardian]="$MCPD_SRC/acos_guardian/target/x86_64-unknown-redox/release/acos-guardian"
    [acos-mux]="$MCPD_SRC/acos_mux/target/x86_64-unknown-redox/release/acos-mux"
)

DO_REBUILD=false
DO_TEST=false

for arg in "$@"; do
    case "$arg" in
        --rebuild) DO_REBUILD=true ;;
        --test) DO_TEST=true ;;
    esac
done

# --- Phase 0: Optional rebuild ---
if [ "$DO_REBUILD" = true ]; then
    echo "=== Phase 0: Cross-compile all binaries ==="
    cd "$REDOX_DIR"
    podman run --rm --cap-add SYS_ADMIN --device /dev/fuse --network=host \
      --volume "$(pwd):/mnt/redox:Z" --volume "$(pwd)/build/podman:/root:Z" \
      --workdir /mnt/redox/recipes/other/mcpd/source redox-base bash -c '
        export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
        export RUSTUP_TOOLCHAIN=redox
        export CARGO_TARGET_DIR="${PWD}/target"

        echo "Building mcpd..."
        cargo build --release --target x86_64-unknown-redox --no-default-features --features redox

        echo "Building mcp-query..."
        cargo build --manifest-path mcp_query/Cargo.toml --release --target x86_64-unknown-redox

        echo "Building mcp-talk..."
        cargo build --manifest-path mcp_talk/Cargo.toml --release --target x86_64-unknown-redox

        echo "Building acos-guardian..."
        cargo build --manifest-path acos_guardian/Cargo.toml --release --target x86_64-unknown-redox

        echo "Building acos-mux..."
        cargo build --manifest-path acos_mux/Cargo.toml --release --target x86_64-unknown-redox -p acos-mux --features acos
      '
    cd "$PROJECT_DIR"
fi

# --- Phase 1: Verify binaries ---
echo "=== Phase 1: Verify pre-compiled binaries ==="
MISSING=0
for name in "${!BINARIES[@]}"; do
    path="${BINARIES[$name]}"
    if [ -f "$path" ]; then
        size=$(stat -c%s "$path")
        echo "  OK $name ($((size/1024))K)"
    else
        echo "  MISSING $name (expected: $path)"
        MISSING=$((MISSING + 1))
    fi
done

if [ "$MISSING" -gt 0 ]; then
    echo "ERROR: $MISSING binaries missing. Run with --rebuild to compile."
    exit 1
fi

# --- Phase 2: Verify image ---
echo "=== Phase 2: Verify disk image ==="
if [ ! -f "$IMAGE" ]; then
    echo "ERROR: Disk image not found at $IMAGE"
    echo "Run: cd redox_base && make image"
    exit 1
fi

if [ ! -f "$REDOXFS" ]; then
    echo "ERROR: RedoxFS tool not found at $REDOXFS"
    exit 1
fi

# --- Phase 3: Clean up stale processes ---
echo "=== Phase 3: Cleanup ==="
pkill -f "qemu-system.*acos-bare" 2>/dev/null || true
pkill -f "redoxfs.*harddrive" 2>/dev/null || true
fusermount3 -uz /tmp/acos_mount 2>/dev/null || true
fusermount3 -uz "$MOUNT_POINT" 2>/dev/null || true
sleep 1

# --- Phase 4: Mount and inject ---
echo "=== Phase 4: Mount image and inject ==="
mkdir -p "$MOUNT_POINT"
"$REDOXFS" "$IMAGE" "$MOUNT_POINT" &
MOUNT_PID=$!
sleep 3

if [ ! -d "$MOUNT_POINT/usr/bin" ]; then
    echo "ERROR: Mount failed (no /usr/bin found)"
    kill "$MOUNT_PID" 2>/dev/null || true
    exit 1
fi

# Inject binaries
INJECTED=0
for name in "${!BINARIES[@]}"; do
    path="${BINARIES[$name]}"
    cp "$path" "$MOUNT_POINT/usr/bin/$name"
    echo "  Injected $name"
    INJECTED=$((INJECTED + 1))
done

# Fix branding — both /etc/issue (pre-login banner) and /etc/motd (post-login)
printf '########## ACOS ##########\n# Agent-Centric OS        #\n# Login: user or root     #\n# root password: password  #\n############################\n' > "$MOUNT_POINT/etc/issue"
echo "  Fixed /etc/issue branding"

printf 'Welcome to ACOS — Agent-Centric Operating System\nMCP Bus: mcp://\nType mcp-talk for AI, mcp-query for services, acos-mux for terminal multiplexer.\n' > "$MOUNT_POINT/etc/motd"
echo "  Fixed /etc/motd (Welcome message)"

# Verify init scripts
if [ ! -f "$MOUNT_POINT/usr/lib/init.d/15_mcp" ]; then
    printf 'requires_weak 10_net\nscheme mcp mcpd\n' > "$MOUNT_POINT/usr/lib/init.d/15_mcp"
    echo "  Created 15_mcp init script"
fi

if [ ! -f "$MOUNT_POINT/usr/lib/init.d/99_acos_ready" ]; then
    printf 'echo ACOS_BOOT_OK\n' > "$MOUNT_POINT/usr/lib/init.d/99_acos_ready"
    echo "  Created 99_acos_ready init script"
fi

# --- Phase 5: Unmount ---
echo "=== Phase 5: Unmount ==="
fusermount3 -u "$MOUNT_POINT"
wait "$MOUNT_PID" 2>/dev/null || true
rmdir "$MOUNT_POINT" 2>/dev/null || true
echo "  Unmounted successfully"

echo ""
echo "=== INJECTION COMPLETE ==="
echo "$INJECTED binaries injected + branding fixed"
echo ""

# --- Phase 6: Optional test ---
if [ "$DO_TEST" = true ]; then
    echo "=== Phase 6: Running ACOS audit ==="
    cd "$PROJECT_DIR"
    timeout 300 python3 -u scripts/test_acos_audit.py 2>&1
fi
