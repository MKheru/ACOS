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

# Copy llm_engine crate (LLM inference dependency of mcp_scheme)
mkdir -p "$SOURCE_DIR/llm_engine"
cp -r "$PROJECT_DIR/components/llm_engine/src" "$SOURCE_DIR/llm_engine/src"
cp "$PROJECT_DIR/components/llm_engine/Cargo.toml" "$SOURCE_DIR/llm_engine/Cargo.toml"

# Copy mcp_query crate (CLI debug tool)
mkdir -p "$SOURCE_DIR/mcp_query"
cp -r "$PROJECT_DIR/components/mcp_query/src" "$SOURCE_DIR/mcp_query/src"
cp "$PROJECT_DIR/components/mcp_query/Cargo.toml" "$SOURCE_DIR/mcp_query/Cargo.toml"

# Copy mcp_talk crate (AI terminal interface)
mkdir -p "$SOURCE_DIR/mcp_talk"
cp -r "$PROJECT_DIR/components/mcp_talk/src" "$SOURCE_DIR/mcp_talk/src"
cp "$PROJECT_DIR/components/mcp_talk/Cargo.toml" "$SOURCE_DIR/mcp_talk/Cargo.toml"

# Copy acos_guardian crate (autonomous system monitor)
echo ">>> Copying acos_guardian..."
rm -rf "$SOURCE_DIR/acos_guardian"
cp -r "$PROJECT_DIR/components/acos_guardian" "$SOURCE_DIR/acos_guardian"

# Copy acos_mux crate (terminal multiplexer)
echo ">>> Copying acos_mux..."
rm -rf "$SOURCE_DIR/acos_mux"
cp -r "$PROJECT_DIR/components/acos_mux" "$SOURCE_DIR/acos_mux"

# Fix the path dependency in mcpd's Cargo.toml to point to local mcp_scheme
sed -i 's|path = "../mcp_scheme"|path = "mcp_scheme"|' "$SOURCE_DIR/Cargo.toml"

# Fix llm_engine path in mcp_scheme's Cargo.toml
sed -i 's|path = "../llm_engine"|path = "../llm_engine"|' "$SOURCE_DIR/mcp_scheme/Cargo.toml"

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

# Update recipe.toml build script to include mcp-talk build and binary copy
# Insert mcp-talk build and copy before the closing """ (only if not already present)
if ! grep -q "mcp-talk" "$RECIPE_DIR/recipe.toml"; then
    sed -i '/^"""/i\
\
"${COOKBOOK_CARGO}" build \\\
    --manifest-path "${COOKBOOK_SOURCE}/Cargo.toml" \\\
    --package mcp-talk \\\
    ${build_flags}\
\
cp "target/${TARGET}/${build_type}/mcp-talk" "${COOKBOOK_STAGE}/usr/bin/mcp-talk"' "$RECIPE_DIR/recipe.toml"
fi

if ! grep -q "acos-guardian" "$RECIPE_DIR/recipe.toml"; then
    sed -i '/^"""/i\
\
"${COOKBOOK_CARGO}" build \\\
    --manifest-path "${COOKBOOK_SOURCE}/acos_guardian/Cargo.toml" \\\
    --target "${TARGET}" \\\
    ${build_flags}\
\
cp "target/${TARGET}/${build_type}/acos-guardian" "${COOKBOOK_STAGE}/usr/bin/acos-guardian"' "$RECIPE_DIR/recipe.toml"
fi

echo "Git rev: $REV"
echo "Recipe rev updated in: $RECIPE_DIR/recipe.toml"
echo "=== Done ==="
