#!/usr/bin/env python3
"""
Test acos-mux CPU usage in QEMU.

Launches acos-mux, waits 10s, then checks CPU time.
A busy-spin would show >8s of CPU in 10s wall time.
After fix, should be <1s of CPU in 10s.

SCORE=0-3:
  1 point: acos-mux starts and stays alive
  1 point: CPU time < 5s after 10s wall time (not busy-spinning)
  1 point: CPU time < 1s after 10s wall time (well-behaved)
"""

import os
import sys
import re
import time

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

from acos_qemu import ACOSController


def main():
    score = 0
    vm = ACOSController()

    try:
        print("[*] Starting QEMU...")
        vm.start()
        vm.boot(timeout=90)
        vm.login()
        print("[OK] Booted and logged in")
        time.sleep(2)

        # Launch acos-mux
        print("[*] Launching acos-mux...")
        vm.serial.sendline("EMUX_LOG=/tmp/m.log acos-mux >/dev/null &")
        time.sleep(3)

        # Check it's alive
        alive = vm.is_process_alive("acos-mux")
        if alive:
            print("[OK] acos-mux is alive")
            score += 1
        else:
            print("[FAIL] acos-mux not alive")
            print(f"\nSCORE={score}")
            return score

        # Get initial CPU time
        ps_before = vm.run("ps", timeout=5) or ""
        mux_line_before = None
        for line in ps_before.split("\n"):
            if "acos-mux" in line and "/usr/bin/" in line:
                mux_line_before = line.strip()
                break

        print(f"[*] Waiting 10s to measure CPU usage...")
        time.sleep(10)

        # Get CPU time after wait
        ps_after = vm.run("ps", timeout=5) or ""
        mux_line_after = None
        for line in ps_after.split("\n"):
            if "acos-mux" in line and "/usr/bin/" in line:
                mux_line_after = line.strip()
                break

        if mux_line_after:
            # Parse CPU time from ps output (format: HH:MM:SS.cc)
            time_match = re.search(r'(\d+):(\d+):(\d+)\.(\d+)', mux_line_after)
            if time_match:
                h, m, s, cs = int(time_match.group(1)), int(time_match.group(2)), int(time_match.group(3)), int(time_match.group(4))
                cpu_seconds = h * 3600 + m * 60 + s + cs / 100.0
                print(f"[*] acos-mux CPU time: {cpu_seconds:.2f}s after ~13s wall time")
                print(f"[*] PS line: {mux_line_after}")

                if cpu_seconds < 5.0:
                    print("[OK] CPU < 5s — not busy-spinning")
                    score += 1
                else:
                    print(f"[FAIL] CPU >= 5s — likely busy-spinning ({cpu_seconds:.1f}s)")

                if cpu_seconds < 1.0:
                    print("[OK] CPU < 1s — well-behaved event loop")
                    score += 1
                else:
                    print(f"[INFO] CPU >= 1s ({cpu_seconds:.1f}s)")
            else:
                print(f"[WARN] Could not parse CPU time from: {mux_line_after}")
        else:
            print("[FAIL] acos-mux disappeared from ps")

    except Exception as e:
        print(f"[ERROR] {e}")
    finally:
        vm.stop()

    print(f"\nSCORE={score}")
    return score


if __name__ == "__main__":
    score = main()
    sys.exit(0 if score >= 2 else 1)
