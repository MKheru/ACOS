#!/bin/bash
# patch_branding.sh — Remove Redox branding from mounted ACOS image
# Usage: bash patch_branding.sh /path/to/mount

MOUNT="$1"
if [ -z "$MOUNT" ] || [ ! -d "$MOUNT/usr" ]; then
    echo "Usage: $0 <mount_dir>"
    exit 1
fi

echo "=== Patching ACOS branding ==="

# /etc/issue — login banner
cat > "$MOUNT/etc/issue" << 'EOF'
########## ACOS ##########
# Agent-Centric OS        #
# Login: user or root     #
# root password: password  #
############################
EOF
echo "✓ /etc/issue"

# /etc/motd — message of the day
cat > "$MOUNT/etc/motd" << 'EOF'
Welcome to ACOS
EOF
echo "✓ /etc/motd"

# /etc/hostname
echo "acos" > "$MOUNT/etc/hostname"
echo "✓ /etc/hostname"

# /usr/lib/os-release
cat > "$MOUNT/usr/lib/os-release" << 'EOF'
PRETTY_NAME="ACOS 0.9.0 (WS9B)"
NAME="ACOS"
VERSION_ID="0.9.0"
ID="acos"
EOF
echo "✓ /usr/lib/os-release"

echo "=== Branding patched ==="
