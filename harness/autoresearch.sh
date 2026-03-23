#!/usr/bin/env bash
#
# ACOS AutoResearch Universal Test Runner
#
# Main script the AI agent calls each iteration to compile, test,
# extract metrics, and track progress for a given lab.
#
# Usage: ./harness/autoresearch.sh <lab_id> <round_number>
#
# Exit codes:
#   0 = iteration complete (check AUTORESEARCH_RESULT for metric)
#   1 = compile failed
#   2 = host tests failed
#   3 = cross-compile failed
#   4 = QEMU boot failed
#   5 = metric extraction failed

set -euo pipefail

# ── Args & paths ──────────────────────────────────────────────────

LAB_ID="${1:?Usage: $0 <lab_id> <round_number>}"
ROUND="${2:?Usage: $0 <lab_id> <round_number>}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

METRIC_VALUE=""

# ── Helper: read a YAML field (top-level or one-level nested) ────

_yaml_field() {
    # Usage: _yaml_field "key" or _yaml_field "parent.key"
    local lab_file="evolution/labs/${LAB_ID}.yaml"
    if [[ "$1" == *.* ]]; then
        local parent="${1%%.*}"
        local child="${1#*.}"
        python3 -c "
import sys, os
sys.path.insert(0, os.path.join('harness'))
from parse_lab import load_lab
lab = load_lab(sys.argv[1])
v = lab.get(sys.argv[2], {}).get(sys.argv[3], '')
print(v if v is not None else '')
" "${LAB_ID}" "$parent" "$child"
    else
        python3 -c "
import sys, os
sys.path.insert(0, os.path.join('harness'))
from parse_lab import load_lab
lab = load_lab(sys.argv[1])
v = lab.get(sys.argv[2], '')
print(v if v is not None else '')
" "${LAB_ID}" "$1"
    fi
}

# ── Validate lab config ──────────────────────────────────────────

echo "AUTORESEARCH_STEP:validate"
if ! python3 harness/parse_lab.py "$LAB_ID" validate > /dev/null; then
    echo "ERROR: Lab config validation failed for '$LAB_ID'" >&2
    exit 1
fi

TYPE=$(_yaml_field "type")
TYPE="${TYPE:-host}"
COMPONENT=$(_yaml_field "component")
COMPONENT_DIR="components/${COMPONENT}"

# ── Backup allowed files ─────────────────────────────────────────

BACKUP_DIR="/tmp/acos_backup_${LAB_ID}_${ROUND}"
mkdir -p "$BACKUP_DIR"
ALLOWED_FILES=$(python3 harness/parse_lab.py "$LAB_ID" allowed_files)

while IFS= read -r file; do
    [ -z "$file" ] && continue
    if [ -f "$COMPONENT_DIR/$file" ]; then
        mkdir -p "$BACKUP_DIR/$(dirname "$file")"
        cp "$COMPONENT_DIR/$file" "$BACKUP_DIR/$file"
    fi
done <<< "$ALLOWED_FILES"

# ── Rollback function ────────────────────────────────────────────

_rollback() {
    echo "AUTORESEARCH_STEP:rollback"
    while IFS= read -r file; do
        [ -z "$file" ] && continue
        if [ -f "$BACKUP_DIR/$file" ]; then
            cp "$BACKUP_DIR/$file" "$COMPONENT_DIR/$file"
        fi
    done <<< "$ALLOWED_FILES"
    while IFS= read -r file; do
        [ -z "$file" ] && continue
        if [ ! -f "$BACKUP_DIR/$file" ] && [ -f "$COMPONENT_DIR/$file" ]; then
            rm "$COMPONENT_DIR/$file"
        fi
    done <<< "$ALLOWED_FILES"
    echo "AUTORESEARCH_ROLLBACK:restored from backup"
}

# ── Cleanup on exit ──────────────────────────────────────────────

_cleanup() {
    [ -d "$BACKUP_DIR" ] && rm -rf "$BACKUP_DIR"
}
trap _cleanup EXIT

# ── Host compile (fast-fail) ─────────────────────────────────────

COMPILE_CMD=$(_yaml_field "host_test.compile")

if [ -n "$COMPILE_CMD" ]; then
    echo "AUTORESEARCH_STEP:compile"
    pushd "$COMPONENT_DIR" > /dev/null
    if ! bash -c "$COMPILE_CMD" 2>&1; then
        popd > /dev/null
        echo "AUTORESEARCH_RESULT:metric=0,status=compile_fail,round=$ROUND"
        exit 1
    fi
    popd > /dev/null
fi

# ── Host tests (fast-fail) ───────────────────────────────────────

TEST_CMD=$(_yaml_field "host_test.test")

if [ -n "$TEST_CMD" ]; then
    echo "AUTORESEARCH_STEP:test"
    pushd "$COMPONENT_DIR" > /dev/null
    if ! bash -c "$TEST_CMD" 2>&1; then
        popd > /dev/null
        echo "AUTORESEARCH_RESULT:metric=0,status=test_fail,round=$ROUND"
        _rollback
        exit 2
    fi
    popd > /dev/null
fi

# ── Extract metric (host mode) ───────────────────────────────────

if [ "$TYPE" = "host" ]; then
    echo "AUTORESEARCH_STEP:extract_metric"
    METRIC_CMD=$(_yaml_field "host_test.metric_command")
    METRIC_REGEX=$(_yaml_field "host_test.metric_regex")

    if [ -n "$METRIC_CMD" ]; then
        pushd "$COMPONENT_DIR" > /dev/null
        METRIC_OUTPUT=$(bash -c "$METRIC_CMD" 2>&1) || true
        popd > /dev/null

        if [ -n "$METRIC_REGEX" ]; then
            METRIC_VALUE=$(echo "$METRIC_OUTPUT" | grep -oP "$METRIC_REGEX" | head -1 | grep -oP '\d+\.?\d*' || true)
        fi
    fi
fi

# ── QEMU flow ────────────────────────────────────────────────────

if [ "$TYPE" = "qemu" ]; then
    # Inject source into recipe
    echo "AUTORESEARCH_STEP:inject_source"
    if [ -f scripts/inject_mcpd.sh ]; then
        bash scripts/inject_mcpd.sh
    fi

    # Cross-compile in Podman
    echo "AUTORESEARCH_STEP:cross_compile"
    pushd redox_base > /dev/null
    if ! podman run --rm \
        --cap-add SYS_ADMIN --device /dev/fuse --network=host \
        --volume "$(pwd):/mnt/redox:Z" \
        --volume "$(pwd)/build/podman:/root:Z" \
        --workdir /mnt/redox/recipes/other/mcpd/source \
        redox-base bash -c '
            export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
            export RUSTUP_TOOLCHAIN=redox
            cargo build --release --target x86_64-unknown-redox --no-default-features --features redox
        ' 2>&1; then
        popd > /dev/null
        echo "AUTORESEARCH_RESULT:metric=0,status=cross_compile_fail,round=$ROUND"
        _rollback
        exit 3
    fi
    popd > /dev/null

    # Inject binary + autotest into image
    echo "AUTORESEARCH_STEP:inject_image"
    if ! bash harness/qemu_inject.sh "$LAB_ID" "$ROUND"; then
        echo "AUTORESEARCH_RESULT:metric=0,status=inject_fail,round=$ROUND"
        _rollback
        exit 4
    fi

    # Boot QEMU headless, capture serial
    echo "AUTORESEARCH_STEP:qemu_boot"
    SERIAL_LOG="/tmp/acos_serial_${LAB_ID}.log"
    TIMEOUT=$(_yaml_field "qemu_test.timeout_seconds")
    TIMEOUT="${TIMEOUT:-60}"
    SUCCESS_MARKER=$(_yaml_field "qemu_test.success_marker")
    SUCCESS_MARKER="${SUCCESS_MARKER:-AUTORESEARCH_DONE}"

    IMAGE_PATH="redox_base/build/x86_64/acos-bare/harddrive.img"
    OVMF=$(find /usr/share -name "OVMF_CODE.fd" 2>/dev/null | head -1 || true)

    timeout "$TIMEOUT" qemu-system-x86_64 -nographic -machine q35 -cpu host -enable-kvm -smp 4 -m 2048 \
        -serial file:"$SERIAL_LOG" \
        -drive file="$IMAGE_PATH",format=raw,if=none,id=drv0 \
        -device nvme,drive=drv0,serial=ACOS \
        ${OVMF:+-bios "$OVMF"} \
        -netdev user,id=net0,hostfwd=tcp::10022-:22 -device e1000,netdev=net0 &
    QEMU_PID=$!

    # Wait for success marker or timeout
    ELAPSED=0
    while [ "$ELAPSED" -lt "$TIMEOUT" ]; do
        if [ -f "$SERIAL_LOG" ] && grep -q "$SUCCESS_MARKER" "$SERIAL_LOG" 2>/dev/null; then
            kill "$QEMU_PID" 2>/dev/null || true
            wait "$QEMU_PID" 2>/dev/null || true
            break
        fi
        sleep 2
        ELAPSED=$((ELAPSED + 2))
    done

    # Kill QEMU if still running
    kill "$QEMU_PID" 2>/dev/null || true
    wait "$QEMU_PID" 2>/dev/null || true

    if ! grep -q "$SUCCESS_MARKER" "$SERIAL_LOG" 2>/dev/null; then
        echo "AUTORESEARCH_RESULT:metric=0,status=qemu_timeout,round=$ROUND"
        _rollback
        exit 4
    fi

    # Extract metric from serial log
    echo "AUTORESEARCH_STEP:extract_metric"
    METRIC_NAME=$(_yaml_field "metric.name")
    METRIC_VALUE=$(grep "AUTORESEARCH_METRIC:${METRIC_NAME}=" "$SERIAL_LOG" | tail -1 | sed "s/.*${METRIC_NAME}=//" || true)
fi

# ── Check metric extraction ──────────────────────────────────────

if [ -z "$METRIC_VALUE" ]; then
    echo "AUTORESEARCH_RESULT:metric=0,status=metric_extraction_fail,round=$ROUND"
    _rollback
    exit 5
fi

# ── Check target ─────────────────────────────────────────────────

python3 harness/parse_lab.py "$LAB_ID" check "$METRIC_VALUE"
TARGET_MET=$?

if [ $TARGET_MET -eq 0 ]; then
    STATUS="target_met"
else
    STATUS="pass"
fi

# ── Check for regression & rollback ──────────────────────────────

RESULTS_FILE="evolution/results/${LAB_ID}.tsv"
ROLLBACK_ON_REGRESSION=$(_yaml_field "rollback_on_regression")

if [ -f "$RESULTS_FILE" ] && [ "$ROLLBACK_ON_REGRESSION" = "true" ]; then
    PREV_METRIC=$(tail -1 "$RESULTS_FILE" | cut -f3)
    if [ -n "$PREV_METRIC" ] && [ "$PREV_METRIC" != "metric" ]; then
        REGRESSED=$(python3 -c "
import sys, os
sys.path.insert(0, 'harness')
from parse_lab import load_lab
lab = load_lab(sys.argv[1])
target = lab.get('metric', {}).get('target', '')
prev, curr = float(sys.argv[2]), float(sys.argv[3])
if target.startswith('>'):
    print('true' if curr < prev else 'false')
else:
    print('true' if curr > prev else 'false')
" "${LAB_ID}" "$PREV_METRIC" "$METRIC_VALUE" 2>/dev/null) || true

        if [ "$REGRESSED" = "true" ]; then
            STATUS="regression"
            _rollback
        fi
    fi
fi

# ── Output result & append TSV ───────────────────────────────────

echo "AUTORESEARCH_RESULT:metric=$METRIC_VALUE,status=$STATUS,round=$ROUND"

mkdir -p "evolution/results"
TIMESTAMP=$(date -Iseconds)
if [ ! -f "$RESULTS_FILE" ]; then
    printf "timestamp\tround\tmetric\tstatus\tnotes\n" > "$RESULTS_FILE"
fi
printf "%s\t%s\t%s\t%s\t\n" "$TIMESTAMP" "$ROUND" "$METRIC_VALUE" "$STATUS" >> "$RESULTS_FILE"

# ── Write round memory template ──────────────────────────────────

MEMORY_FILE="evolution/memory/${LAB_ID}_round_${ROUND}.md"
mkdir -p "evolution/memory"
cat > "$MEMORY_FILE" << MEMEOF
# Round $ROUND — $LAB_ID

**Metric:** $METRIC_VALUE
**Status:** $STATUS
**Timestamp:** $TIMESTAMP

## What Changed
<!-- Agent fills this in -->

## Result Analysis
<!-- Agent fills this in -->

## Next Steps
<!-- Agent fills this in -->
MEMEOF
