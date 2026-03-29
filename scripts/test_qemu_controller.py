#!/usr/bin/env python3
"""
Metric script for the acos-qemu-controller AutoResearch lab.

Runs N test scenarios against a live QEMU instance, counts passes.
Output: SCORE=N (0-10)

Scenarios:
 1. QEMU starts and PTY detected
 2. QMP connects and capabilities OK
 3. Boot completes (ACOS_BOOT_OK on serial)
 4. Login via serial succeeds
 5. Run echo command, get correct output
 6. Check process listing works
 7. QMP screendump produces valid file
 8. VNC connects successfully
 9. VNC screenshot produces valid image
10. Clean shutdown works
"""

import os
import sys
import time
import traceback

# Add scripts dir to path
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

from acos_qemu import ACOSController, HAS_VNC, HAS_PIL


def run_tests():
    score = 0
    vm = ACOSController()
    results = {}

    try:
        # --- Test 1: QEMU starts ---
        try:
            vm.start()
            assert vm.is_running, "QEMU not running"
            assert vm.pty_path, "No PTY path"
            results["1_qemu_start"] = "PASS"
            score += 1
        except Exception as e:
            results["1_qemu_start"] = f"FAIL: {e}"
            print(f"SCORE={score}")
            return score

        # --- Test 2: QMP connected ---
        try:
            resp = vm.qmp.execute("query-status")
            assert resp is not None, "QMP returned None"
            results["2_qmp_connect"] = "PASS"
            score += 1
        except Exception as e:
            results["2_qmp_connect"] = f"FAIL: {e}"

        # --- Test 3: Boot completes ---
        try:
            ok = vm.boot(timeout=90)
            assert ok, "Boot did not complete"
            results["3_boot"] = "PASS"
            score += 1
        except Exception as e:
            results["3_boot"] = f"FAIL: {e}"
            print(f"SCORE={score}")
            return score

        # --- Test 4: Login ---
        try:
            ok = vm.login()
            assert ok, "Login failed"
            results["4_login"] = "PASS"
            score += 1
        except Exception as e:
            results["4_login"] = f"FAIL: {e}"
            print(f"SCORE={score}")
            return score

        # --- Test 5: Run echo command ---
        try:
            output = vm.run("echo HELLO_ACOS_TEST")
            assert output is not None, "Command returned None (timeout)"
            assert "HELLO_ACOS_TEST" in output, f"Expected HELLO_ACOS_TEST in output, got: {repr(output)}"
            results["5_echo_cmd"] = "PASS"
            score += 1
        except Exception as e:
            results["5_echo_cmd"] = f"FAIL: {e}"

        # --- Test 6: Process listing ---
        try:
            output = vm.run("ps | head -5")
            assert output is not None, "ps returned None"
            # ps output should contain at least "PID" or process info
            assert len(output.strip()) > 0, "ps output is empty"
            results["6_ps_listing"] = "PASS"
            score += 1
        except Exception as e:
            results["6_ps_listing"] = f"FAIL: {e}"

        # --- Test 7: QMP screendump ---
        try:
            screenshot_path = "/tmp/acos-test-screenshot.ppm"
            vm.qmp.screendump(screenshot_path)
            time.sleep(1)
            assert os.path.exists(screenshot_path), "Screenshot file not created"
            size = os.path.getsize(screenshot_path)
            assert size > 1000, f"Screenshot too small: {size} bytes"
            results["7_screendump"] = "PASS"
            score += 1
        except Exception as e:
            results["7_screendump"] = f"FAIL: {e}"

        # --- Test 8: VNC connect ---
        try:
            if not HAS_VNC:
                results["8_vnc_connect"] = "SKIP: vncdotool not installed"
            else:
                vm.connect_vnc()
                assert vm.vnc.client is not None, "VNC client is None"
                results["8_vnc_connect"] = "PASS"
                score += 1
        except Exception as e:
            results["8_vnc_connect"] = f"FAIL: {e}"

        # --- Test 9: VNC screenshot ---
        try:
            if not HAS_VNC or not vm.vnc.client:
                results["9_vnc_screenshot"] = "SKIP: VNC not connected"
            else:
                vnc_path = "/tmp/acos-test-vnc.png"
                vm.vnc.screenshot(vnc_path)
                assert os.path.exists(vnc_path), "VNC screenshot not created"
                size = os.path.getsize(vnc_path)
                assert size > 1000, f"VNC screenshot too small: {size} bytes"
                results["9_vnc_screenshot"] = "PASS"
                score += 1
        except Exception as e:
            results["9_vnc_screenshot"] = f"FAIL: {e}"

        # --- Test 10: Clean shutdown ---
        try:
            assert vm.is_running, "QEMU not running before stop"
            pid = vm.qemu_pid
            vm.stop()
            time.sleep(2)
            # Check process is gone
            try:
                os.kill(pid, 0)
                results["10_shutdown"] = "FAIL: QEMU still running"
            except OSError:
                results["10_shutdown"] = "PASS"
                score += 1
        except Exception as e:
            results["10_shutdown"] = f"FAIL: {e}"

    except Exception as e:
        traceback.print_exc()
    finally:
        # Ensure cleanup
        try:
            vm.stop()
        except Exception:
            pass

    # Print results
    print("\n=== QEMU Controller Test Results ===")
    for name, result in sorted(results.items()):
        status = "✓" if result == "PASS" else ("○" if result.startswith("SKIP") else "✗")
        print(f"  {status} {name}: {result}")
    print(f"\nSCORE={score}")
    return score


if __name__ == "__main__":
    score = run_tests()
    sys.exit(0 if score >= 10 else 1)
