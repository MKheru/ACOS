#!/usr/bin/env bash
#
# Inject the mcpd source code into the Redox build tree.
#
# The Redox cookbook expects sources in a specific location.
# This script packages our mcpd + mcp_scheme crates and places them
# where the recipe expects to find them.
#
# Usage: ./scripts/inject_mcpd.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
REDOX_DIR="$PROJECT_DIR/redox_base"
RECIPE_DIR="$REDOX_DIR/recipes/other/mcpd"
SOURCE_DIR="$RECIPE_DIR/source"

echo "=== Injecting mcpd into Redox build tree ==="

# Clean previous source
rm -rf "$SOURCE_DIR"
mkdir -p "$SOURCE_DIR"

# Copy mcpd crate (the binary)
cp -r "$PROJECT_DIR/components/mcpd/src" "$SOURCE_DIR/src"
cp "$PROJECT_DIR/components/mcpd/Cargo.toml" "$SOURCE_DIR/Cargo.toml"

# Copy mcp_scheme crate (the library dependency)
mkdir -p "$SOURCE_DIR/mcp_scheme"
cp -r "$PROJECT_DIR/components/mcp_scheme/src" "$SOURCE_DIR/mcp_scheme/src"
cp "$PROJECT_DIR/components/mcp_scheme/Cargo.toml" "$SOURCE_DIR/mcp_scheme/Cargo.toml"

# Fix the path dependency in mcpd's Cargo.toml to point to local mcp_scheme
sed -i 's|path = "../mcp_scheme"|path = "mcp_scheme"|' "$SOURCE_DIR/Cargo.toml"

# Remove dev-dependencies (benchmarks) from mcp_scheme to avoid build issues
# in the cross-compilation environment
sed -i '/\[dev-dependencies\]/,/^$/d' "$SOURCE_DIR/mcp_scheme/Cargo.toml"
sed -i '/\[\[bench\]\]/,/^$/d' "$SOURCE_DIR/mcp_scheme/Cargo.toml"

# Remove bench directory if it was copied
rm -rf "$SOURCE_DIR/mcp_scheme/benches"

echo "Source injected at: $SOURCE_DIR"
echo "Files:"
find "$SOURCE_DIR" -type f | sort | sed 's|^|  |'

# Re-init git repo and update recipe rev
cd "$SOURCE_DIR"
git init -q 2>/dev/null
git add -A
REV=$(git commit -m "mcpd inject $(date +%s)" --allow-empty 2>/dev/null | grep -oP '[0-9a-f]{7}' | head -1)
if [ -z "$REV" ]; then
    REV=$(git rev-parse --short HEAD)
fi

# Update rev in recipe (keep the rest intact)
sed -i "s/^rev = .*/rev = \"$REV\"/" "$RECIPE_DIR/recipe.toml"

echo "Git rev: $REV"
echo "Recipe rev updated in: $RECIPE_DIR/recipe.toml"
echo "=== Done ==="
