#!/usr/bin/env bash
#
# Download SmolLM-135M Q4 GGUF model for ACOS LLM runtime.
#
# Downloads the quantized model from HuggingFace and stores it in models/.
# The model is ~80MB and fits within the 512MB ACOS disk image.
#
# Integrity:
#   - huggingface-cli path validates etag/sha256 natively
#   - wget/curl fallback REQUIRES EXPECTED_SHA256 env var (fail-closed)
#   - on success the computed SHA256 is printed so the maintainer can pin it
#
# Usage:
#   ./scripts/download_model.sh
#   EXPECTED_SHA256=<hex> ./scripts/download_model.sh   # for wget/curl path

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
MODELS_DIR="$PROJECT_DIR/models"
MODEL_FILENAME="smollm-135M.Q4_K_M.gguf"
MODEL_URL="https://huggingface.co/HuggingFaceTB/SmolLM-135M-GGUF/resolve/main/$MODEL_FILENAME"
EXPECTED_SHA256="${EXPECTED_SHA256:-}"

mkdir -p "$MODELS_DIR"

echo "=== Downloading SmolLM-135M Q4 GGUF model ==="

if [ -f "$MODELS_DIR/$MODEL_FILENAME" ]; then
    echo "Model already exists: $MODELS_DIR/$MODEL_FILENAME"
    echo "Delete it first to re-download."
    exit 0
fi

verify_sha256() {
    local file="$1"
    local expected="$2"
    local actual
    actual="$(sha256sum "$file" | awk '{print $1}')"
    if [ "$actual" != "$expected" ]; then
        echo "ERROR: SHA256 mismatch for $file"
        echo "  expected: $expected"
        echo "  actual:   $actual"
        rm -f "$file"
        exit 1
    fi
    echo "  SHA256 verified: $actual"
}

# Prefer huggingface-cli (validates etag/sha256 natively, handles auth tokens)
if command -v huggingface-cli &>/dev/null; then
    echo "Using huggingface-cli (native integrity check) ..."
    huggingface-cli download HuggingFaceTB/SmolLM-135M-GGUF "$MODEL_FILENAME" \
        --local-dir "$MODELS_DIR"
    # Optional cross-check if maintainer pinned a SHA256
    if [ -n "$EXPECTED_SHA256" ]; then
        verify_sha256 "$MODELS_DIR/$MODEL_FILENAME" "$EXPECTED_SHA256"
    fi
elif command -v wget &>/dev/null || command -v curl &>/dev/null; then
    if [ -z "$EXPECTED_SHA256" ]; then
        echo "ERROR: wget/curl fallback requires EXPECTED_SHA256 env var (supply chain safety)."
        echo "  Install huggingface-cli (preferred), or run:"
        echo "    EXPECTED_SHA256=<hex> $0"
        exit 1
    fi
    if command -v wget &>/dev/null; then
        echo "Using wget ..."
        wget -O "$MODELS_DIR/$MODEL_FILENAME" "$MODEL_URL"
    else
        echo "Using curl ..."
        curl -fL --proto '=https' -o "$MODELS_DIR/$MODEL_FILENAME" "$MODEL_URL"
    fi
    verify_sha256 "$MODELS_DIR/$MODEL_FILENAME" "$EXPECTED_SHA256"
else
    echo "ERROR: No download tool found. Install huggingface-cli, wget, or curl."
    exit 1
fi

COMPUTED_SHA256="$(sha256sum "$MODELS_DIR/$MODEL_FILENAME" | awk '{print $1}')"
echo "Model saved to: $MODELS_DIR/$MODEL_FILENAME"
echo "  SHA256: $COMPUTED_SHA256"
if [ -z "$EXPECTED_SHA256" ]; then
    echo "  (pin this value in EXPECTED_SHA256 for reproducible builds)"
fi
echo ""
echo "To inject into the ACOS image:"
echo "  ./scripts/inject_model.sh $MODELS_DIR/$MODEL_FILENAME"
echo "=== Done ==="
