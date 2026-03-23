#!/usr/bin/env bash
# Run ACOS in QEMU with LLM proxy support.
#
# Usage:
#   ./scripts/run-acos.sh          # Start proxy + QEMU
#   ./scripts/run-acos.sh --no-proxy  # QEMU only (local inference)
#
# The LLM proxy bridges ACOS → Gemini API via virtio-serial.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REDOX_DIR="$PROJECT_DIR/redox_base"
SOCKET="/tmp/acos-llm.sock"

NO_PROXY=false
for arg in "$@"; do
    case "$arg" in
        --no-proxy) NO_PROXY=true ;;
    esac
done

# Start LLM proxy in background
if [ "$NO_PROXY" = false ]; then
    echo "Starting LLM proxy (Gemini 2.5 Flash)..."
    python3 "$SCRIPT_DIR/llm-proxy.py" --socket "$SOCKET" &
    PROXY_PID=$!
    sleep 1
    echo "Proxy PID: $PROXY_PID (socket: $SOCKET)"

    # Cleanup on exit
    trap "kill $PROXY_PID 2>/dev/null; rm -f $SOCKET; echo 'Proxy stopped'" EXIT
fi

# Build QEMU command with virtio-serial for LLM proxy
echo "Starting ACOS in QEMU..."
cd "$REDOX_DIR"

EXTRA_FLAGS=""
if [ "$NO_PROXY" = false ]; then
    EXTRA_FLAGS="-chardev socket,id=llm,path=$SOCKET,server=off -device virtio-serial-pci -device virtserialport,chardev=llm,name=llm"
fi

# Use make qemu but with extra flags
export QEMUFLAGS_EXTRA="$EXTRA_FLAGS"
make qemu CONFIG_NAME=acos-bare gpu=no kvm=yes QEMU_EXTRA="$EXTRA_FLAGS"
