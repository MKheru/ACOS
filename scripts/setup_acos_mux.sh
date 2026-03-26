#!/bin/bash
# Phase 0: Fork emux → acos-mux
# Copies emux_base into components/acos-mux and renames all crates

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$PROJECT_ROOT/emux_base"
DST="$PROJECT_ROOT/components/acos-mux"

if [ -d "$DST" ]; then
    echo "ERROR: $DST already exists. Remove it first if you want to re-fork."
    exit 1
fi

echo "=== Phase 0: Fork emux → acos-mux ==="

# 1. Copy source (without .git)
echo "[1/5] Copying emux_base → components/acos-mux..."
cp -r "$SRC" "$DST"
rm -rf "$DST/.git"

# 2. Rename crate directories
echo "[2/5] Renaming crate directories..."
for crate_dir in "$DST/crates/emux-"*; do
    new_name="${crate_dir/emux-/acos-mux-}"
    mv "$crate_dir" "$new_name"
done

# Rename bin directory
if [ -d "$DST/bins/emux" ]; then
    mv "$DST/bins/emux" "$DST/bins/acos-mux"
fi

# 3. Rename in all Cargo.toml and Rust files
echo "[3/5] Renaming emux → acos-mux in all files..."

# Cargo.toml: crate names and paths
find "$DST" -name "Cargo.toml" -exec sed -i \
    -e 's/name = "emux/name = "acos-mux/g' \
    -e 's/emux-vt/acos-mux-vt/g' \
    -e 's/emux-term/acos-mux-term/g' \
    -e 's/emux-pty/acos-mux-pty/g' \
    -e 's/emux-mux/acos-mux-mux/g' \
    -e 's/emux-config/acos-mux-config/g' \
    -e 's/emux-render/acos-mux-render/g' \
    -e 's/emux-ipc/acos-mux-ipc/g' \
    -e 's/emux-daemon/acos-mux-daemon/g' \
    -e 's|crates/emux-|crates/acos-mux-|g' \
    -e 's|bins/emux|bins/acos-mux|g' \
    {} +

# Rust files: use/extern crate references
find "$DST" -name "*.rs" -exec sed -i \
    -e 's/emux_vt/acos_mux_vt/g' \
    -e 's/emux_term/acos_mux_term/g' \
    -e 's/emux_pty/acos_mux_pty/g' \
    -e 's/emux_mux/acos_mux_mux/g' \
    -e 's/emux_config/acos_mux_config/g' \
    -e 's/emux_render/acos_mux_render/g' \
    -e 's/emux_ipc/acos_mux_ipc/g' \
    -e 's/emux_daemon/acos_mux_daemon/g' \
    {} +

# String literals and display names
find "$DST" -name "*.rs" -exec sed -i \
    -e 's/"emux"/"acos-mux"/g' \
    -e 's/emux-sockets/acos-mux-sockets/g' \
    -e 's/\.emux/\.acos-mux/g' \
    {} +

# Config paths
find "$DST" -name "*.rs" -exec sed -i \
    -e 's|/emux/|/acos-mux/|g' \
    {} +

# 4. Update doc files
echo "[4/5] Updating documentation..."
find "$DST" -name "*.md" -exec sed -i \
    -e 's/emux/acos-mux/g' \
    -e 's/Emux/ACOS-MUX/g' \
    -e 's/EMUX/ACOS-MUX/g' \
    {} +

# 5. Verify build
echo "[5/5] Verifying cargo check..."
cd "$DST"
if cargo check 2>&1; then
    echo ""
    echo "=== SUCCESS: acos-mux fork ready ==="
    echo "Location: $DST"
    echo "Next: run autoresearch labs on each crate"
else
    echo ""
    echo "=== WARNING: cargo check failed ==="
    echo "Review rename errors before proceeding"
fi
