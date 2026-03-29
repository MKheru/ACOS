#!/bin/bash
# QEMU Control Script — launch ACOS with QMP API for programmatic control
# Usage:
#   ./qemu-control.sh start       — Boot ACOS with QMP + serial capture
#   ./qemu-control.sh send "text" — Send text to the serial console
#   ./qemu-control.sh key "ret"   — Send a special key (ret, tab, esc, etc.)
#   ./qemu-control.sh read        — Read serial console output
#   ./qemu-control.sh screenshot  — Take a VGA screenshot (PPM format)
#   ./qemu-control.sh stop        — Shutdown QEMU
#   ./qemu-control.sh login       — Auto-login as root
#   ./qemu-control.sh run "cmd"   — Login + run command + read output

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REDOX_DIR="$SCRIPT_DIR/../redox_base"
IMAGE="$REDOX_DIR/build/x86_64/acos-bare/harddrive.img"
QMP_SOCK="/tmp/acos-qmp.sock"
SERIAL_LOG="/tmp/acos-serial.log"
SERIAL_SOCK="/tmp/acos-serial.sock"
PID_FILE="/tmp/acos-qemu.pid"

send_qmp() {
    echo "$1" | socat - UNIX-CONNECT:$QMP_SOCK 2>/dev/null
}

send_qmp_cmd() {
    # Send QMP command and get response
    local cmd="$1"
    echo "$cmd" | socat -t1 - UNIX-CONNECT:$QMP_SOCK 2>/dev/null | tail -1
}

send_key() {
    send_qmp_cmd '{"execute":"send-key","arguments":{"keys":[{"type":"qcode","data":"'"$1"'"}]}}'
}

send_string() {
    local text="$1"
    for (( i=0; i<${#text}; i++ )); do
        local c="${text:$i:1}"
        local qcode=""
        case "$c" in
            [a-z]) qcode="$c" ;;
            [A-Z]) qcode="shift-$(echo "$c" | tr A-Z a-z)" ;;
            [0-9]) qcode="$c" ;;
            ' ') qcode="spc" ;;
            '/') qcode="slash" ;;
            '-') qcode="minus" ;;
            '_') qcode="shift-minus" ;;
            '=') qcode="equal" ;;
            '.') qcode="dot" ;;
            ',') qcode="comma" ;;
            ':') qcode="shift-semicolon" ;;
            ';') qcode="semicolon" ;;
            '"') qcode="shift-apostrophe" ;;
            "'") qcode="apostrophe" ;;
            '(') qcode="shift-9" ;;
            ')') qcode="shift-0" ;;
            '[') qcode="bracket_left" ;;
            ']') qcode="bracket_right" ;;
            '{') qcode="shift-bracket_left" ;;
            '}') qcode="shift-bracket_right" ;;
            '!') qcode="shift-1" ;;
            '@') qcode="shift-2" ;;
            '#') qcode="shift-3" ;;
            '$') qcode="shift-4" ;;
            '%') qcode="shift-5" ;;
            '&') qcode="shift-7" ;;
            '*') qcode="shift-8" ;;
            '+') qcode="shift-equal" ;;
            '~') qcode="shift-grave_accent" ;;
            '`') qcode="grave_accent" ;;
            '\') qcode="backslash" ;;
            '|') qcode="shift-backslash" ;;
            '<') qcode="shift-comma" ;;
            '>') qcode="shift-dot" ;;
            '?') qcode="shift-slash" ;;
            *) continue ;;
        esac
        send_key "$qcode" > /dev/null
        sleep 0.03
    done
}

case "$1" in
    start)
        if [ -f "$PID_FILE" ] && kill -0 "$(cat $PID_FILE)" 2>/dev/null; then
            echo "QEMU already running (PID $(cat $PID_FILE))"
            exit 0
        fi

        > "$SERIAL_LOG"

        qemu-system-x86_64 \
            -machine q35 -cpu host -enable-kvm -smp 4 -m 2048 \
            -vga std \
            -qmp unix:$QMP_SOCK,server,nowait \
            -serial file:$SERIAL_LOG \
            -drive file=$IMAGE,format=raw,if=none,id=drv0 \
            -device nvme,drive=drv0,serial=ACOS \
            -net none -no-reboot \
            -daemonize \
            -pidfile $PID_FILE

        echo "QEMU started (PID $(cat $PID_FILE))"
        echo "QMP socket: $QMP_SOCK"
        echo "Serial log: $SERIAL_LOG"

        # Init QMP
        sleep 1
        send_qmp '{"execute":"qmp_capabilities"}' > /dev/null

        # Wait for boot
        echo "Waiting for boot..."
        for i in $(seq 1 30); do
            sleep 2
            if grep -q "acos login:" "$SERIAL_LOG" 2>/dev/null; then
                echo "Boot complete (${i}x2s)"
                break
            fi
        done
        ;;

    send)
        send_string "$2"
        ;;

    key)
        send_key "$2"
        ;;

    enter)
        send_string "$2"
        send_key "ret"
        ;;

    read)
        # Read last N lines of serial output (default 20)
        local lines="${2:-20}"
        strings "$SERIAL_LOG" | tail -${lines}
        ;;

    screenshot)
        local outfile="${2:-/tmp/acos-screenshot.ppm}"
        send_qmp_cmd '{"execute":"screendump","arguments":{"filename":"'"$outfile"'"}}' > /dev/null
        echo "Screenshot saved to $outfile"
        ;;

    login)
        echo "Logging in as root..."
        send_string "root"
        send_key "ret"
        sleep 2
        send_string "password"
        send_key "ret"
        sleep 2
        echo "Logged in."
        ;;

    run)
        # Send a command and wait for output
        local cmd="$2"
        local marker="__DONE_$(date +%s)__"

        # Type the command
        send_string "$cmd"
        send_key "ret"
        sleep 1

        # Type echo marker so we know when output is complete
        send_string "echo $marker"
        send_key "ret"

        # Wait for marker in serial log
        for i in $(seq 1 20); do
            sleep 1
            if grep -q "$marker" "$SERIAL_LOG" 2>/dev/null; then
                break
            fi
        done

        # Extract output between command and marker
        strings "$SERIAL_LOG" | tail -30
        ;;

    stop)
        if [ -f "$PID_FILE" ]; then
            kill "$(cat $PID_FILE)" 2>/dev/null
            rm -f "$PID_FILE"
            echo "QEMU stopped"
        else
            echo "No QEMU running"
        fi
        ;;

    status)
        if [ -f "$PID_FILE" ] && kill -0 "$(cat $PID_FILE)" 2>/dev/null; then
            echo "QEMU running (PID $(cat $PID_FILE))"
        else
            echo "QEMU not running"
        fi
        ;;

    *)
        echo "Usage: $0 {start|send|key|enter|read|screenshot|login|run|stop|status}"
        echo ""
        echo "  start           Boot ACOS with QMP API (daemonized, opens VGA window)"
        echo "  send 'text'     Type text into the console"
        echo "  key 'keyname'   Send special key (ret, tab, esc, up, down, left, right)"
        echo "  enter 'text'    Type text + press Enter"
        echo "  read [N]        Read last N lines of serial output"
        echo "  screenshot [f]  Take VGA screenshot"
        echo "  login           Auto-login as root"
        echo "  run 'command'   Send command + read output"
        echo "  stop            Kill QEMU"
        echo "  status          Check if QEMU is running"
        ;;
esac
