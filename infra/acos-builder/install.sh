#!/bin/bash
# infra/acos-builder/install.sh
#
# Install / update acos-builder.service on a VPS. Requires sudo.
# Idempotent: re-running upgrades the server.py and reloads systemd.
#
# Usage:
#   sudo ./install.sh                      # default user `hermes`
#   sudo ACOS_BUILDER_USER=foo ./install.sh # override user
#
# See docs/BUILDING_ACOS_ON_VPS.md for what this enables.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
USER_NAME="${ACOS_BUILDER_USER:-hermes}"

if [ "$(id -u)" -ne 0 ]; then
    echo "fatal: install.sh must be run as root (use sudo)" >&2
    exit 2
fi

if ! id -u "$USER_NAME" >/dev/null 2>&1; then
    echo "fatal: user '$USER_NAME' does not exist (set ACOS_BUILDER_USER)" >&2
    exit 3
fi

GROUP_NAME="$(id -gn "$USER_NAME")"

echo "Installing acos-builder.service for user=$USER_NAME group=$GROUP_NAME"

# 1. Install server.py
install -d -o root -g root -m 755 /opt/acos-builder
install -m 755 -o root -g root "$SCRIPT_DIR/server.py" /opt/acos-builder/server.py
echo "  /opt/acos-builder/server.py installed"

# 2. State directories (writable by the service user)
install -d -o "$USER_NAME" -g "$GROUP_NAME" -m 755 /var/lib/acos-builder
install -d -o "$USER_NAME" -g "$GROUP_NAME" -m 755 /var/lib/acos-builder/logs
install -d -o "$USER_NAME" -g "$GROUP_NAME" -m 755 /var/lib/acos-builder/jobs
echo "  /var/lib/acos-builder/{logs,jobs} owned by $USER_NAME"

# 3. systemd unit — substitute the user if different from default.
UNIT_TMP="$(mktemp)"
trap 'rm -f "$UNIT_TMP"' EXIT
if [ "$USER_NAME" = "hermes" ]; then
    cp "$SCRIPT_DIR/acos-builder.service" "$UNIT_TMP"
else
    sed -e "s/^User=hermes/User=$USER_NAME/" \
        -e "s/^Group=hermes/Group=$GROUP_NAME/" \
        -e "s|WorkingDirectory=/home/hermes|WorkingDirectory=/home/$USER_NAME|" \
        -e "s|ReadWritePaths=/home/hermes|ReadWritePaths=/home/$USER_NAME|" \
        "$SCRIPT_DIR/acos-builder.service" > "$UNIT_TMP"
fi
install -m 644 -o root -g root "$UNIT_TMP" /etc/systemd/system/acos-builder.service
echo "  /etc/systemd/system/acos-builder.service installed"

# 4. Reload + enable + (re)start
systemctl daemon-reload
systemctl enable acos-builder.service >/dev/null
systemctl restart acos-builder.service
echo "  service enabled + restarted"

# 5. Smoke check
sleep 2
if ! systemctl is-active acos-builder.service >/dev/null; then
    echo "fatal: acos-builder.service failed to start" >&2
    journalctl -u acos-builder.service --no-pager -n 30
    exit 4
fi

# 6. Probe HTTP
if command -v curl >/dev/null 2>&1; then
    if curl -sS --max-time 5 http://127.0.0.1:8772/health >/dev/null; then
        echo "  HTTP probe: /health 200"
    else
        echo "warning: /health probe failed — service is up but not reachable" >&2
    fi
fi

echo "done."
