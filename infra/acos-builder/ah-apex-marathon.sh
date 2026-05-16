#!/bin/bash
# ah-apex-marathon.sh — drive AH through the [available] APEX backlog autonomously.
#
# Spawns a fresh AH session per task (each does ONE task via the LOOP_PROMPT).
# Loops until MAX_TASKS, MAX_HOURS, backlog drained, or STOP sentinel.
#
# Parses AH's `RESULT: ...` final line to know what happened:
#   RESULT: PR_OPENED #N for <task-id>     → success
#   RESULT: TASK_FAILED:<reason>           → counted as failure, continue
#   RESULT: BACKLOG_DRAINED                → exit
#   RESULT: BUILDER_BUSY                   → wait + continue
#   RESULT: DUPLICATE_SKIPPED              → continue, no failure
#   RESULT: TOKEN_EXPIRED / BUILDER_ERROR  → exit
#
# Run as root (uses sudo systemd-run to give AH the hermes env).

set -u

MAX_TASKS="${MAX_TASKS:-12}"
MAX_HOURS="${MAX_HOURS:-8}"
COOLDOWN_S="${COOLDOWN_S:-30}"
STOP_SENTINEL="/tmp/apex-marathon.stop"
START_TS=$(date +%s)
DEADLINE=$((START_TS + MAX_HOURS * 3600))

RUN_TS=$(date +%Y%m%dT%H%M%S)
LOG_ROOT="/var/log/acos-builder/apex-marathon-${RUN_TS}"
mkdir -p "$LOG_ROOT"

MARATHON_LOG="$LOG_ROOT/marathon.log"
exec > >(tee -a "$MARATHON_LOG") 2>&1

echo "============================================================"
echo "AH APEX marathon — start $RUN_TS"
echo "  MAX_TASKS=$MAX_TASKS  MAX_HOURS=$MAX_HOURS  COOLDOWN_S=$COOLDOWN_S"
echo "  Log root: $LOG_ROOT"
echo "  Stop sentinel: $STOP_SENTINEL (touch this file to halt gracefully)"
echo "============================================================"

success_count=0
failure_count=0
consecutive_failures=0
i=0

for i in $(seq 1 "$MAX_TASKS"); do
    NOW=$(date +%s)
    if [ "$NOW" -ge "$DEADLINE" ]; then
        echo
        echo "DEADLINE_REACHED after $((NOW - START_TS))s — stopping"
        break
    fi

    if [ -f "$STOP_SENTINEL" ]; then
        echo
        echo "STOP_SENTINEL detected at $STOP_SENTINEL — graceful halt"
        rm -f "$STOP_SENTINEL"
        break
    fi

    if [ "$consecutive_failures" -ge 3 ]; then
        echo
        echo "ABORT — 3 consecutive failures, stopping marathon"
        break
    fi

    TS=$(date +%Y%m%dT%H%M%S)
    TASK_LOG="$LOG_ROOT/task-${i}-${TS}.log"
    echo
    echo "----- iteration $i / $MAX_TASKS at $TS -----"
    echo "  task log: $TASK_LOG"

    # Invoke AH for one task via the loop prompt
    sudo systemd-run --uid=hermes --gid=hermes \
        -p EnvironmentFile=/etc/hermes/env.list \
        -p WorkingDirectory=/home/hermes \
        -p Environment=HERMES_HOME=/home/hermes/.hermes \
        --pipe --wait --collect --quiet \
        /home/hermes/hermes-agent/venv/bin/hermes -z \
        "$(cat /home/hermes/ACOS_APEX_LOOP_PROMPT.md)" \
        > "$TASK_LOG" 2>&1
    EXIT=$?

    # Detect Codex 5h-quota exhaustion BEFORE parsing RESULT line, because
    # the 429 hits AH's own inference call so no RESULT line is ever emitted
    # by AH. Pattern observed 2026-05-16 marathons v2/v8:
    #   - "API call failed after 3 retries: HTTP 429: The usage limit has been reached"
    #   - cascading "AuthError: No Codex credentials stored. Run hermes auth..."
    if grep -qE "HTTP 429: The usage limit has been reached|AuthError: No Codex credentials" "$TASK_LOG"; then
        echo "  signal: Codex 5h-plan quota exhausted — pausing marathon and probing every 10 min"
        consecutive_failures=0  # don't count this as a failure; we know it's a hard external limit
        probe_count=0
        while true; do
            # Check elapsed against deadline before sleeping
            NOW=$(date +%s)
            if [ "$NOW" -ge "$DEADLINE" ]; then
                echo "  signal: DEADLINE_REACHED during 429 backoff, stopping"
                break 2
            fi
            if [ -f "$STOP_SENTINEL" ]; then
                echo "  signal: STOP_SENTINEL during 429 backoff, stopping"
                rm -f "$STOP_SENTINEL"
                break 2
            fi
            sleep 600  # 10 min between probes
            probe_count=$((probe_count + 1))
            # Probe with a minimal hermes call
            probe_out=$(timeout 60 sudo systemd-run --uid=hermes --gid=hermes \
                -p EnvironmentFile=/etc/hermes/env.list \
                -p WorkingDirectory=/home/hermes \
                -p Environment=HERMES_HOME=/home/hermes/.hermes \
                --pipe --wait --collect --quiet \
                /home/hermes/hermes-agent/venv/bin/hermes -z "Reponds OK." 2>&1)
            echo "  probe $probe_count after 10 min: $(echo "$probe_out" | tr -d "\n" | head -c 80)"
            if echo "$probe_out" | grep -qE "^OK\b|^\s*OK\s*$"; then
                echo "  signal: Codex quota restored, resuming marathon"
                break
            fi
            # Safety cap: don't probe forever (max 36 probes = 6h, plenty for any 5h reset)
            if [ "$probe_count" -ge 36 ]; then
                echo "  signal: 6h probing without quota return — stopping for human inspection"
                break 2
            fi
        done
        # Resume the loop — re-run this iteration since the 429 wasted it
        continue
    fi

    # Extract the RESULT: line (last occurrence)
    RESULT_LINE=$(grep -E "^RESULT:" "$TASK_LOG" | tail -1)
    if [ -z "$RESULT_LINE" ]; then
        RESULT_LINE="RESULT: UNKNOWN (no RESULT: line in AH output; exit=$EXIT)"
    fi

    echo "  $RESULT_LINE"

    case "$RESULT_LINE" in
        *"PR_OPENED"*)
            success_count=$((success_count + 1))
            consecutive_failures=0
            echo "  → success (total=$success_count)"
            ;;
        *"DUPLICATE_SKIPPED"*)
            echo "  → duplicate skipped, no failure"
            ;;
        *"BUILDER_BUSY"*)
            echo "  → builder busy, extra 120s sleep"
            sleep 120
            continue
            ;;
        *"BACKLOG_DRAINED"*)
            echo "  → backlog drained, exiting marathon"
            break
            ;;
        *"TOKEN_EXPIRED"*)
            echo "  → token expired, EXITING — manual rotation needed"
            break
            ;;
        *"BUILDER_ERROR"*)
            # Could be a real builder crash OR AH misusing the tool as a shell
            # command (happened at iteration 2 of marathon 02:36). Don't
            # auto-exit — count as failure, the 3-consecutive-failures cap
            # catches real persistent issues.
            failure_count=$((failure_count + 1))
            consecutive_failures=$((consecutive_failures + 1))
            echo "  → builder error (consecutive=$consecutive_failures, total=$failure_count); continuing"
            ;;
        *"TASK_FAILED"*)
            failure_count=$((failure_count + 1))
            consecutive_failures=$((consecutive_failures + 1))
            echo "  → task failed (consecutive=$consecutive_failures, total=$failure_count)"
            ;;
        *)
            failure_count=$((failure_count + 1))
            consecutive_failures=$((consecutive_failures + 1))
            echo "  → unknown result (consecutive=$consecutive_failures, total=$failure_count)"
            ;;
    esac

    if [ "$i" -lt "$MAX_TASKS" ]; then
        sleep "$COOLDOWN_S"
    fi
done

ELAPSED=$(($(date +%s) - START_TS))
echo
echo "============================================================"
echo "AH APEX marathon — done"
echo "  Iterations: $i"
echo "  Successes:  $success_count"
echo "  Failures:   $failure_count"
echo "  Elapsed:    ${ELAPSED}s ($((ELAPSED/60)) min)"
echo "  Log root:   $LOG_ROOT"
echo "============================================================"
