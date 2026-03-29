#!/bin/bash
# setup-redox.sh — Clone Redox OS and apply ACOS patches
#
# This script sets up the Redox OS build environment with ACOS modifications:
# 1. Clones Redox OS at the known-good commit
# 2. Applies ACOS branding and config patches
# 3. Copies ACOS config (acos-bare.toml)
# 4. Clones Ion shell and applies MCP builtin patches
# 5. Links mcpd source into the recipes tree

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ACOS_DIR="$(dirname "$SCRIPT_DIR")"
REDOX_DIR="$ACOS_DIR/redox_base"

# Known-good upstream commits
REDOX_COMMIT="91e0399"  # Redox OS upstream before ACOS patches
ION_COMMIT="1440704f"   # Ion upstream before ACOS patches

echo "=== ACOS Setup ==="
echo "ACOS root: $ACOS_DIR"

# --- Step 1: Clone Redox ---
if [ -d "$REDOX_DIR/.git" ]; then
    echo "Redox already cloned at $REDOX_DIR"
else
    echo "Cloning Redox OS..."
    git clone https://gitlab.redox-os.org/redox-os/redox.git "$REDOX_DIR"
    cd "$REDOX_DIR"
    git checkout "$REDOX_COMMIT"
fi

# --- Step 2: Apply Redox patches ---
echo "Applying ACOS patches to Redox..."
cd "$REDOX_DIR"
for patch in "$ACOS_DIR"/patches/redox/*.patch; do
    if git apply --check "$patch" 2>/dev/null; then
        git am "$patch"
        echo "  Applied: $(basename "$patch")"
    else
        echo "  Skipped (already applied): $(basename "$patch")"
    fi
done

# --- Step 3: Copy ACOS config ---
cp "$ACOS_DIR/config/acos-bare.toml" "$REDOX_DIR/config/"
echo "Copied acos-bare.toml"

# --- Step 4: Setup Ion with MCP builtins ---
ION_DIR="$REDOX_DIR/recipes/core/ion/source"
if [ -d "$ION_DIR/.git" ]; then
    echo "Ion already present"
else
    echo "Cloning Ion shell..."
    git clone https://gitlab.redox-os.org/redox-os/ion.git "$ION_DIR"
    cd "$ION_DIR"
    git checkout "$ION_COMMIT"
fi

echo "Applying Ion MCP patches..."
cd "$ION_DIR"
for patch in "$ACOS_DIR"/patches/ion/*.patch; do
    if git apply --check "$patch" 2>/dev/null; then
        git am "$patch"
        echo "  Applied: $(basename "$patch")"
    else
        echo "  Skipped (already applied): $(basename "$patch")"
    fi
done

# --- Step 5: Link mcpd source ---
MCPD_RECIPE="$REDOX_DIR/recipes/other/mcpd"
mkdir -p "$MCPD_RECIPE"
if [ ! -L "$MCPD_RECIPE/source" ] && [ ! -d "$MCPD_RECIPE/source" ]; then
    ln -s "$ACOS_DIR/mcpd" "$MCPD_RECIPE/source"
    echo "Linked mcpd source"
fi
cp "$ACOS_DIR/config/recipe.toml" "$MCPD_RECIPE/"
echo "Copied mcpd recipe.toml"

echo ""
echo "=== Setup complete ==="
echo "Next steps:"
echo "  cd $REDOX_DIR"
echo "  make all CONFIG_NAME=acos-bare"
echo "  ../scripts/build-inject-all.sh --rebuild"
