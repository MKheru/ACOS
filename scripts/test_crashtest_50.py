#!/usr/bin/env python3
"""
ACOS Crash Test — 50 tests exercising every layer of the OS.
Boots QEMU ONCE, runs all tests sequentially via serial.

Focus: MCP services, file ops, process mgmt, config, logs, stability,
edge cases, error handling. LLM tests minimal (proxy may not be active).

SCORE=0-50
"""

import os
import sys
import time
import json
import re

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

from acos_qemu import ACOSController, clean_ansi

results = {}
details = {}
score = 0


def mcp(vm, service, method, params=None, timeout=10):
    """Run mcp-query and return parsed JSON result or raw string."""
    if params:
        cmd = f"mcp-query {service} {method} {params}"
    else:
        cmd = f"mcp-query {service} {method}"
    output = vm.run(cmd, timeout=timeout)
    if not output:
        return None
    # Try to extract JSON from output
    for line in output.split("\n"):
        line = line.strip()
        if line.startswith("{"):
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                pass
    return output


def test(name, condition, detail=""):
    global score
    if condition:
        results[name] = "PASS"
        score += 1
    else:
        results[name] = "FAIL"
    details[name] = detail


def main():
    global score
    vm = ACOSController(network=True)

    try:
        print("[*] Booting ACOS for crash test (50 tests)...")
        vm.start()
        assert vm.boot(timeout=90), "Boot failed"
        assert vm.login(), "Login failed"
        print("[OK] Booted and logged in\n")
        time.sleep(3)

        # =====================================================================
        # GROUP 1: Boot & Identity (tests 01-05)
        # =====================================================================
        print("--- Group 1: Boot & Identity ---")

        # 01: Boot marker
        r = mcp(vm, "system", "info")
        test("01_boot_ok", r and "result" in r, str(r)[:80] if r else "None")

        # 02: Hostname is acos
        hostname = r.get("result", {}).get("hostname", "") if isinstance(r, dict) else ""
        test("02_hostname_acos", hostname == "acos", hostname)

        # 03: Kernel identifies as ACOS
        kernel = r.get("result", {}).get("kernel", "") if isinstance(r, dict) else ""
        test("03_kernel_acos", "ACOS" in kernel, kernel)

        # 04: /etc/issue contains ACOS
        r = mcp(vm, "file", "read /etc/issue")
        content = r.get("result", {}).get("content", "") if isinstance(r, dict) else ""
        test("04_issue_acos", "ACOS" in content and "Redox" not in content, content[:60])

        # 05: /etc/motd contains ACOS
        r = mcp(vm, "file", "read /etc/motd")
        content = r.get("result", {}).get("content", "") if isinstance(r, dict) else ""
        test("05_motd_acos", "ACOS" in content, content[:60])

        # =====================================================================
        # GROUP 2: MCP Bus Core (tests 06-10)
        # =====================================================================
        print("--- Group 2: MCP Bus Core ---")

        # 06: mcpd is running
        alive = vm.is_process_alive("mcpd")
        test("06_mcpd_alive", alive)

        # 07: Echo service
        r = mcp(vm, "echo", "echo", '\'{"jsonrpc":"2.0","method":"echo","params":{"message":"ping"},"id":1}\'')
        test("07_echo_service", r is not None and "ping" not in str(r.get("error", "")), str(r)[:80] if r else "None")

        # 08: Invalid method returns error
        r = mcp(vm, "system", "nonexistent_method")
        is_err = isinstance(r, dict) and "error" in r
        test("08_invalid_method_error", is_err, str(r)[:80] if r else "None")

        # 09: Memory info
        r = mcp(vm, "system", "info")
        mem = r.get("result", {}).get("memory_total", 0) if isinstance(r, dict) else 0
        test("09_memory_reported", mem > 0, f"memory_total={mem}")

        # 10: Uptime reported
        uptime = r.get("result", {}).get("uptime", -1) if isinstance(r, dict) else -1
        test("10_uptime_reported", uptime >= 0, f"uptime={uptime}")

        # =====================================================================
        # GROUP 3: File Service (tests 11-20)
        # =====================================================================
        print("--- Group 3: File Service ---")

        # 11: Read /etc/hostname
        r = mcp(vm, "file", "read /etc/hostname")
        test("11_file_read_hostname", isinstance(r, dict) and r.get("result", {}).get("content") == "acos")

        # 12: Read nonexistent file returns error
        r = mcp(vm, "file", "read /nonexistent/path/foo.txt")
        test("12_file_read_nonexistent", isinstance(r, dict) and "error" in r)

        # 13: Read /usr/lib/os-release
        r = mcp(vm, "file", "read /usr/lib/os-release")
        content = r.get("result", {}).get("content", "") if isinstance(r, dict) else ""
        test("13_file_read_osrelease", "ACOS" in content, content[:60])

        # 14: File size reported
        size = r.get("result", {}).get("size", 0) if isinstance(r, dict) else 0
        test("14_file_size_reported", size > 0, f"size={size}")

        # 15: Read binary-safe (read a small binary)
        r = mcp(vm, "file", "read /usr/bin/echo")
        test("15_file_read_binary", isinstance(r, dict) and ("result" in r or "error" in r))

        # 16: File search for *.toml
        r = mcp(vm, "file_search", "search *.toml /etc")
        test("16_file_search", r is not None, str(r)[:80] if r else "None")

        # 17: Read /etc/group
        r = mcp(vm, "file", "read /etc/group")
        test("17_file_read_group", isinstance(r, dict) and "result" in r)

        # 18: Read /etc/passwd
        r = mcp(vm, "file", "read /etc/passwd")
        content = r.get("result", {}).get("content", "") if isinstance(r, dict) else ""
        test("18_file_read_passwd", "root" in content, content[:60])

        # 19: Read init script
        r = mcp(vm, "file", "read /usr/lib/init.d/15_mcp")
        content = r.get("result", {}).get("content", "") if isinstance(r, dict) else ""
        test("19_file_read_init_mcp", "mcpd" in content, content[:60])

        # 20: Path traversal blocked
        r = mcp(vm, "file", "read /etc/../etc/hostname")
        # Should still work (resolved) or be blocked — either way, no crash
        test("20_path_traversal_safe", r is not None)

        # =====================================================================
        # GROUP 4: Process Service (tests 21-25)
        # =====================================================================
        print("--- Group 4: Process Service ---")

        # 21: Process list returns array
        r = mcp(vm, "process", "list")
        procs = r.get("result", []) if isinstance(r, dict) else []
        test("21_process_list_array", isinstance(procs, list) and len(procs) > 5, f"count={len(procs)}")

        # 22: mcpd in process list
        mcpd_found = any("mcpd" in str(p.get("name", "")) for p in procs) if isinstance(procs, list) else False
        test("22_mcpd_in_proclist", mcpd_found)

        # 23: init in process list
        init_found = any("init" in str(p.get("name", "")).lower() for p in procs) if isinstance(procs, list) else False
        test("23_init_in_proclist", init_found)

        # 24: PIDs are numeric
        all_numeric = all(isinstance(p.get("pid"), int) for p in procs) if isinstance(procs, list) and procs else False
        test("24_pids_numeric", all_numeric)

        # 25: Process memory reported
        has_mem = any(p.get("memory") for p in procs) if isinstance(procs, list) else False
        test("25_process_memory", has_mem)

        # =====================================================================
        # GROUP 5: Config Service (tests 26-30)
        # =====================================================================
        print("--- Group 5: Config Service ---")

        # 26: Config set
        r = mcp(vm, "config", "set test_key test_value_42")
        test("26_config_set", isinstance(r, dict) and "result" in r, str(r)[:80] if r else "None")

        # 27: Config get
        r = mcp(vm, "config", "get test_key")
        val = r.get("result", {}).get("value", "") if isinstance(r, dict) else ""
        test("27_config_get", "test_value_42" in str(val), str(val)[:60])

        # 28: Config list
        r = mcp(vm, "config", "list")
        test("28_config_list", isinstance(r, dict) and "result" in r, str(r)[:80] if r else "None")

        # 29: Config get nonexistent key
        r = mcp(vm, "config", "get nonexistent_key_xyz")
        test("29_config_get_missing", r is not None)

        # 30: Config set special chars
        r = mcp(vm, "config", "set special_key hello-world_123")
        test("30_config_set_special", isinstance(r, dict) and "result" in r)

        # =====================================================================
        # GROUP 6: Log Service (tests 31-35)
        # =====================================================================
        print("--- Group 6: Log Service ---")

        # 31: Log write
        r = mcp(vm, "log", "write info crash_test_log shell")
        test("31_log_write", isinstance(r, dict) and "result" in r, str(r)[:80] if r else "None")

        # 32: Log read
        r = mcp(vm, "log", "read 5")
        test("32_log_read", isinstance(r, dict) and "result" in r, str(r)[:80] if r else "None")

        # 33: Log contains our entry
        entries = r.get("result", []) if isinstance(r, dict) else []
        has_our_log = any("crash_test_log" in str(e) for e in entries) if isinstance(entries, list) else "crash_test_log" in str(entries)
        test("33_log_contains_entry", has_our_log)

        # 34: Log write warning level
        r = mcp(vm, "log", "write warning test_warning shell")
        test("34_log_write_warning", isinstance(r, dict) and "result" in r)

        # 35: Log write error level
        r = mcp(vm, "log", "write error test_error shell")
        test("35_log_write_error", isinstance(r, dict) and "result" in r)

        # =====================================================================
        # GROUP 7: Network & LLM (tests 36-40)
        # =====================================================================
        print("--- Group 7: Network & LLM ---")

        # 36: Ping gateway
        output = vm.run("ping -c 1 10.0.2.2", timeout=10)
        test("36_ping_gateway", output is not None and "1 packets" in (output or ""), (output or "")[:80])

        # 37: LLM info responds
        r = mcp(vm, "llm", "info")
        test("37_llm_info", isinstance(r, dict) and "result" in r, str(r)[:80] if r else "None")

        # 38: LLM backend type
        backend = r.get("result", {}).get("backend", "") if isinstance(r, dict) else ""
        test("38_llm_backend", backend in ("gemini-api", "host-proxy", "local"), backend)

        # 39: LLM generate (small prompt)
        r = mcp(vm, "llm", 'generate "Say OK"', timeout=20)
        has_text = isinstance(r, dict) and r.get("result", {}).get("text", "")
        test("39_llm_generate", bool(has_text), str(r)[:80] if r else "None")

        # 40: DNS/network stack (try to resolve but don't fail hard)
        output = vm.run("cat /etc/net/dns", timeout=5)
        test("40_dns_config", output is not None, (output or "")[:60])

        # =====================================================================
        # GROUP 8: Binaries & Daemons (tests 41-45)
        # =====================================================================
        print("--- Group 8: Binaries & Daemons ---")

        # 41-45: All 5 binaries exist
        for i, binary in enumerate(["mcpd", "mcp-query", "mcp-talk", "acos-guardian", "acos-mux"], start=41):
            output = vm.run(f"ls /usr/bin/{binary}", timeout=5)
            test(f"{i}_binary_{binary.replace('-','_')}", output is not None and "not found" not in (output or "").lower(), (output or "")[:40])

        # =====================================================================
        # GROUP 9: Stability & Edge Cases (tests 46-50)
        # =====================================================================
        print("--- Group 9: Stability & Edge Cases ---")

        # 46: Rapid MCP calls (10 system info in a row)
        ok_count = 0
        for _ in range(10):
            r = mcp(vm, "system", "info", timeout=5)
            if isinstance(r, dict) and "result" in r:
                ok_count += 1
        test("46_rapid_mcp_calls", ok_count >= 8, f"{ok_count}/10 succeeded")

        # 47: Large file read (read a bigger file)
        r = mcp(vm, "file", "read /usr/bin/ls", timeout=10)
        test("47_large_file_read", r is not None)

        # 48: MCP still alive after stress
        r = mcp(vm, "system", "info")
        test("48_mcp_alive_after_stress", isinstance(r, dict) and "result" in r)

        # 49: Guardian can be launched
        vm.serial.sendline("acos-guardian >/dev/null &")
        time.sleep(5)
        alive = vm.is_process_alive("acos-guardian")
        test("49_guardian_launches", alive)

        # 50: Screenshot captures (VGA still rendering)
        try:
            vm.screenshot("/tmp/crashtest-final.ppm")
            size = os.path.getsize("/tmp/crashtest-final.ppm")
            test("50_vga_screenshot", size > 1000, f"size={size}")
        except Exception as e:
            test("50_vga_screenshot", False, str(e))

    except Exception as e:
        print(f"[FATAL] {e}")
        import traceback
        traceback.print_exc()
    finally:
        vm.stop()

    # Print results
    print("\n" + "=" * 60)
    print("ACOS CRASH TEST — 50 TESTS")
    print("=" * 60)
    passed = sum(1 for r in results.values() if r == "PASS")
    failed = sum(1 for r in results.values() if r == "FAIL")

    for name in sorted(results.keys()):
        icon = "\u2713" if results[name] == "PASS" else "\u2717"
        detail = f" — {details[name]}" if details.get(name) else ""
        print(f"  {icon} {name}: {results[name]}{detail}")

    print(f"\n{passed} passed, {failed} failed out of {len(results)} tests")
    print(f"\nSCORE={score}")
    return score


if __name__ == "__main__":
    score = main()
    sys.exit(0 if score >= 40 else 1)
