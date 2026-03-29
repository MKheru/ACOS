#!/bin/bash
# setup-acos.sh — Set up the ACOS build environment
#
# Downloads the base micro-kernel, applies ACOS patches, and links
# all components into the build tree:
# 1. Clones the upstream micro-kernel at a known-good commit
# 2. Applies ACOS branding and config patches
# 3. Copies ACOS image config (acos-bare.toml)
# 4. Clones Ion shell and applies MCP builtin patches
# 5. Links mcpd source into the recipes tree

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ACOS_DIR="$(dirname "$SCRIPT_DIR")"
BASE_DIR="$ACOS_DIR/base"

# Known-good upstream commits
BASE_COMMIT="91e0399"   # Base micro-kernel commit before ACOS patches
ION_COMMIT="1440704f"   # Ion shell commit before ACOS patches

echo "=== ACOS Setup ==="
echo "ACOS root: $ACOS_DIR"

# --- Step 1: Clone Redox ---
if [ -d "$BASE_DIR/.git" ]; then
    echo "Base kernel already cloned at $BASE_DIR"
else
    echo "Cloning base micro-kernel..."
    git clone https://gitlab.redox-os.org/redox-os/redox.git "$BASE_DIR"
    cd "$BASE_DIR"
    git checkout "$BASE_COMMIT"
fi

# --- Step 2: Apply ACOS patches ---
echo "Applying ACOS patches..."
cd "$BASE_DIR"
for patch in "$ACOS_DIR"/patches/redox/*.patch; do
    if git apply --check "$patch" 2>/dev/null; then
        git am "$patch"
        echo "  Applied: $(basename "$patch")"
    else
        echo "  Skipped (already applied): $(basename "$patch")"
    fi
done

# --- Step 3: Copy ACOS config ---
cp "$ACOS_DIR/config/acos-bare.toml" "$BASE_DIR/config/"
echo "Copied acos-bare.toml"

# --- Step 4: Setup Ion with MCP builtins ---
ION_DIR="$BASE_DIR/recipes/core/ion/source"
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
MCPD_RECIPE="$BASE_DIR/recipes/other/mcpd"
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
echo "  cd $BASE_DIR"
echo "  make all CONFIG_NAME=acos-bare"
echo "  ../scripts/build-inject-all.sh --rebuild"
