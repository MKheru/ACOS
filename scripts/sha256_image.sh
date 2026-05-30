#!/usr/bin/env bash
# WS0.5 — print SHA256 image version for baseline artifacts.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${1:-$ROOT_DIR/redox_base/build/x86_64/acos-bare/harddrive.img}"

if [[ ! -f "$IMAGE" ]]; then
  echo "image not found: $IMAGE" >&2
  exit 2
fi

sha256sum "$IMAGE"
