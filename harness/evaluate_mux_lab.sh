#!/bin/bash
# Harness d'évaluation pour les labs ACOS-MUX
# Usage: ./evaluate_mux_lab.sh <lab_id> [working_dir]
#
# Mesure: compile_ok (0/1) + tests_pass (0/1) + tests_count (int) → score composite
# Score = compile_ok * 10 + tests_pass * 10 + tests_count

set -uo pipefail

LAB_ID="${1:?Usage: evaluate_mux_lab.sh <lab_id> [working_dir]}"
WORKING_DIR="${2:-projects/agent_centric_os/components/acos-mux}"
PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK_PATH="$PROJECT_ROOT/$WORKING_DIR"

echo "=== ACOS-MUX Lab Evaluation: $LAB_ID ==="
echo "Working dir: $WORK_PATH"
echo ""

cd "$WORK_PATH" || { echo "EVAL_ERROR: working dir not found"; exit 1; }

# --- Step 1: Cargo check (compile) ---
echo "[1/3] cargo check..."
COMPILE_OUTPUT=$(cargo check 2>&1)
COMPILE_EXIT=$?

if [ $COMPILE_EXIT -eq 0 ]; then
    COMPILE_OK=1
    echo "  compile: PASS"
else
    COMPILE_OK=0
    echo "  compile: FAIL"
    echo "$COMPILE_OUTPUT" | tail -20
fi

# --- Step 2: Cargo test (specific crate if possible) ---
echo "[2/3] cargo test..."

# Map lab_id to crate
case "$LAB_ID" in
    acos-mux-pty)    CRATE_FLAG="-p acos-mux-pty" ;;
    acos-mux-render) CRATE_FLAG="-p acos-mux-render" ;;
    acos-mux-ipc)    CRATE_FLAG="-p acos-mux-ipc" ;;
    acos-mux-daemon) CRATE_FLAG="-p acos-mux-daemon" ;;
    *)               CRATE_FLAG="" ;;
esac

TEST_OUTPUT=$(cargo test $CRATE_FLAG 2>&1)
TEST_EXIT=$?

if [ $TEST_EXIT -eq 0 ]; then
    TESTS_PASS=1
    echo "  tests: PASS"
else
    TESTS_PASS=0
    echo "  tests: FAIL"
fi

# Extract test count
TESTS_COUNT=$(echo "$TEST_OUTPUT" | grep -oP 'test result: ok\. \K\d+' | tail -1)
TESTS_COUNT=${TESTS_COUNT:-0}
echo "  tests count: $TESTS_COUNT"

# --- Step 3: Score ---
SCORE=$((COMPILE_OK * 10 + TESTS_PASS * 10 + TESTS_COUNT))

echo ""
echo "[3/3] Results:"
echo "  compile_ok=$COMPILE_OK"
echo "  tests_pass=$TESTS_PASS"
echo "  tests_count=$TESTS_COUNT"
echo ""
echo "EVAL_SCORE=$SCORE"
echo "EVAL_COMPILE=$COMPILE_OK"
echo "EVAL_TESTS_PASS=$TESTS_PASS"
echo "EVAL_TESTS_COUNT=$TESTS_COUNT"
