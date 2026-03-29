#!/usr/bin/env python3
"""
QEMU Verification Script for ion-acos-v2 lab.
Cross-compiles ion, injects into image, boots QEMU, runs real tests.

Exit code 0 = all critical tests pass
Exit code 1 = one or more critical tests failed

Output format: VERIFY_RESULT:<test_name>=<PASS|FAIL>:<detail>
"""

import sys
import os
import subprocess
import time
import signal
import json

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_DIR = os.path.dirname(SCRIPT_DIR)
REDOX_DIR = os.path.join(PROJECT_DIR, "redox_base")
ION_SOURCE = os.path.join(REDOX_DIR, "recipes", "core", "ion", "source")
ION_BINARY = os.path.join(ION_SOURCE, "target", "x86_64-unknown-redox", "release", "ion")
IMAGE = os.path.join(REDOX_DIR, "build", "x86_64", "acos-bare", "harddrive.img")
IMAGE_BACKUP = IMAGE + ".bak-verify"
REDOXFS = os.path.join(REDOX_DIR, "build", "fstools", "bin", "redoxfs")
MOUNT_DIR = "/tmp/acos_verify_ion"

sys.path.insert(0, SCRIPT_DIR)
from acos_qemu import ACOSController

results = {}


def result(name, passed, detail=""):
    status = "PASS" if passed else "FAIL"
    results[name] = (passed, detail)
    print(f"VERIFY_RESULT:{name}={status}:{detail}")


def cross_compile():
    """Cross-compile ion for x86_64-unknown-redox."""
    print("=== Phase 1: Cross-compile ion ===")

    cmd = [
        "podman", "run", "--rm",
        "--cap-add", "SYS_ADMIN", "--device", "/dev/fuse",
        "--network=host",
        "--volume", f"{REDOX_DIR}:/mnt/redox:Z",
        "--volume", f"{os.path.join(REDOX_DIR, 'build', 'podman')}:/root:Z",
        "--workdir", "/mnt/redox/recipes/core/ion/source",
        "redox-base", "bash", "-c",
        'export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH" && '
        'export RUSTUP_TOOLCHAIN=redox && '
        'export CARGO_TARGET_DIR="${PWD}/target" && '
        'cargo build --release --target x86_64-unknown-redox 2>&1'
    ]

    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
    compiled = proc.returncode == 0 and os.path.exists(ION_BINARY)
    result("cross_compile", compiled,
           f"exit={proc.returncode}" + (f" err={proc.stderr[-200:]}" if not compiled else ""))

    if not compiled:
        print(proc.stdout[-500:] if proc.stdout else "")
        print(proc.stderr[-500:] if proc.stderr else "")
    return compiled


def inject_binary():
    """Inject cross-compiled ion into QEMU image."""
    print("\n=== Phase 2: Inject ion into image ===")

    # Backup image
    if not os.path.exists(IMAGE_BACKUP):
        subprocess.run(["cp", IMAGE, IMAGE_BACKUP])

    os.makedirs(MOUNT_DIR, exist_ok=True)

    # Mount
    proc = subprocess.Popen([REDOXFS, IMAGE, MOUNT_DIR])
    time.sleep(3)

    mounted = os.path.isdir(os.path.join(MOUNT_DIR, "usr"))
    if not mounted:
        proc.terminate()
        result("inject", False, "mount failed")
        return False

    # Inject
    try:
        subprocess.run(["cp", ION_BINARY, os.path.join(MOUNT_DIR, "usr", "bin", "ion")])
        subprocess.run(["sync"])
        subprocess.run(["fusermount3", "-u", MOUNT_DIR])
        time.sleep(1)
        result("inject", True, "binary injected")
        return True
    except Exception as e:
        subprocess.run(["fusermount3", "-u", MOUNT_DIR], check=False)
        result("inject", False, str(e))
        return False


def qemu_tests():
    """Boot QEMU with network and run real tests."""
    print("\n=== Phase 3: QEMU real tests ===")

    vm = ACOSController(network=True)
    try:
        vm.start()
        booted = vm.boot(timeout=90)
        result("qemu_boot", booted, "")
        if not booted:
            return

        logged = vm.login()
        result("qemu_login", logged, "")
        if not logged:
            return

        # Test 1: ion version — confirm our binary
        out = vm.run("ion --version") or ""
        # Our binary should have a different rev than the original
        has_ion = "ion" in out and "1.0.0" in out
        result("ion_version", has_ion, out.strip()[-80:])

        # Test 2: mcp list — must return REAL services from mcpd
        out = vm.run('ion -c "mcp list"') or ""
        # Should contain real service names, not just "system\nguardian\nnetwork"
        has_real = any(svc in out for svc in ["echo", "process", "memory", "config"])
        result("mcp_list_real", has_real, out.strip()[-200:])

        # Test 3: mcp call echo ping — real mcpd echo service
        out = vm.run('ion -c "mcp call echo ping"') or ""
        has_pong = "pong" in out.lower() or "result" in out
        result("mcp_call_echo", has_pong, out.strip()[-200:])

        # Test 4: mcp call system info — real system info
        out = vm.run('ion -c "mcp call system info"') or ""
        has_hostname = "acos" in out.lower() or "hostname" in out
        result("mcp_call_system", has_hostname, out.strip()[-200:])

        # Test 5: guardian status — real guardian via mcpd
        out = vm.run('ion -c "guardian status"') or ""
        has_response = "jsonrpc" in out or "result" in out or "guardian" in out
        no_error = "command not found" not in out
        result("guardian_status", has_response and no_error, out.strip()[-200:])

        # Test 6: mcp call with JSON params
        out = vm.run('ion -c "mcp call echo echo"') or ""
        has_result = "result" in out or "jsonrpc" in out
        result("mcp_call_params", has_result, out.strip()[-200:])

        # Test 7: mcp help still works
        out = vm.run('ion -c "mcp"') or ""
        has_help = "list" in out and "call" in out
        result("mcp_help", has_help, out.strip()[-100:])

        # Test 8: --agent mode
        out = vm.run('echo \'{"id": "1", "command": "echo hello"}\' | ion --agent') or ""
        has_agent = "status" in out and "hello" in out
        result("agent_mode", has_agent, out.strip()[-200:])

        # Test 9: MCP network transport (if LLM proxy is running on host)
        # Try to connect to host:9999 via curl first to see if proxy is up
        curl_out = vm.run("curl -s http://10.0.2.2:9999/ 2>/dev/null", timeout=5) or ""
        proxy_up = len(curl_out.strip()) > 0 or "connection" not in curl_out.lower()

        if proxy_up:
            out = vm.run('ion -c "mcp net 10.0.2.2:9999 ping"', timeout=10) or ""
            has_net = "error" not in out.lower() or "result" in out
            result("mcp_net_llm", has_net, out.strip()[-200:])
        else:
            result("mcp_net_llm", False, "LLM proxy not reachable at 10.0.2.2:9999")

        # Test 10: No mock in Redox transport — mcp call to nonexistent service
        out = vm.run('ion -c "mcp call nonexistent test"') or ""
        has_real_error = "not found" in out.lower() or "error" in out
        no_mock_ok = "status" not in out or "ok" not in out  # mock would return {"status":"ok"}
        result("no_mock_transport", has_real_error and no_mock_ok, out.strip()[-200:])

        # Reference: what mcp-query returns (ground truth)
        ref_out = vm.run("mcp-query echo ping") or ""
        print(f"\n  REFERENCE mcp-query echo ping: {ref_out.strip()[-200:]}")

        ref_out = vm.run("mcp-query system info") or ""
        print(f"  REFERENCE mcp-query system info: {ref_out.strip()[-200:]}")

    except Exception as e:
        import traceback
        traceback.print_exc()
        result("qemu_error", False, str(e))
    finally:
        vm.stop()


def restore_image():
    """Restore original image from backup."""
    if os.path.exists(IMAGE_BACKUP):
        subprocess.run(["cp", IMAGE_BACKUP, IMAGE])


def main():
    try:
        # Phase 1: Cross-compile
        if not cross_compile():
            print("\nFATAL: Cross-compilation failed. Cannot verify.")
            sys.exit(1)

        # Phase 2: Inject
        if not inject_binary():
            print("\nFATAL: Binary injection failed. Cannot verify.")
            sys.exit(1)

        # Phase 3: QEMU tests
        qemu_tests()

    finally:
        # Restore original image so other labs aren't affected
        restore_image()

    # Summary
    print("\n" + "=" * 60)
    print("  QEMU VERIFICATION SUMMARY")
    print("=" * 60)
    passed = 0
    failed = 0
    for name, (ok, detail) in results.items():
        icon = "PASS" if ok else "FAIL"
        passed += ok
        failed += not ok
        print(f"  [{icon}] {name}: {detail[:60]}")
    print(f"\n  Total: {passed} passed, {failed} failed")
    print("=" * 60)

    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
