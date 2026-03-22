#!/usr/bin/env bash
#
# Download SmolLM-135M Q4 GGUF model for ACOS LLM runtime.
#
# Downloads the quantized model from HuggingFace and stores it in models/.
# The model is ~80MB and fits within the 512MB ACOS disk image.
#
# Usage: ./scripts/download_model.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
MODELS_DIR="$PROJECT_DIR/models"
MODEL_FILENAME="smollm-135M.Q4_K_M.gguf"
MODEL_URL="https://huggingface.co/HuggingFaceTB/SmolLM-135M-GGUF/resolve/main/$MODEL_FILENAME"

mkdir -p "$MODELS_DIR"

echo "=== Downloading SmolLM-135M Q4 GGUF model ==="

if [ -f "$MODELS_DIR/$MODEL_FILENAME" ]; then
    echo "Model already exists: $MODELS_DIR/$MODEL_FILENAME"
    echo "Delete it first to re-download."
    exit 0
fi

# Try huggingface-cli first (preferred — handles auth tokens)
if command -v huggingface-cli &>/dev/null; then
    echo "Using huggingface-cli ..."
    huggingface-cli download HuggingFaceTB/SmolLM-135M-GGUF "$MODEL_FILENAME" \
        --local-dir "$MODELS_DIR"
elif command -v wget &>/dev/null; then
    echo "Using wget ..."
    wget -O "$MODELS_DIR/$MODEL_FILENAME" "$MODEL_URL"
elif command -v curl &>/dev/null; then
    echo "Using curl ..."
    curl -L -o "$MODELS_DIR/$MODEL_FILENAME" "$MODEL_URL"
else
    echo "ERROR: No download tool found. Install huggingface-cli, wget, or curl."
    exit 1
fi

echo "Model saved to: $MODELS_DIR/$MODEL_FILENAME"
echo ""
echo "To inject into the ACOS image:"
echo "  ./scripts/inject_model.sh $MODELS_DIR/$MODEL_FILENAME"
echo "=== Done ==="
