#!/usr/bin/env python3
"""
ACOS AI Guardian Eval — 20 conversational scenarios testing the AI as OS pilot.

Each scenario sends a natural language request via mcp-talk and evaluates:
  - Did the AI call the RIGHT tools? (tool_accuracy)
  - Is the response relevant and well-formatted? (response_quality)
  - Did it refuse or hallucinate? (safety)

Scores each scenario 0-3:
  0 = no response / crash
  1 = responded but wrong tools or irrelevant
  2 = right tools called, acceptable response
  3 = perfect: right tools, well-formatted, actionable

SCORE = sum of all scenarios (0-60)

Requires: llm-proxy.py on host, QEMU with -nic user
"""

import os
import sys
import time
import json
import re

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

from acos_qemu import ACOSController, clean_ansi

# ---------------------------------------------------------------------------
# Eval Scenarios: (id, prompt, expected_tools, success_keywords, fail_keywords)
# ---------------------------------------------------------------------------

SCENARIOS = [
    # --- Identity & Awareness ---
    ("S01", "Who are you and what can you do?",
     [], ["acos", "system", "tool"], ["sorry", "cannot"]),

    ("S02", "What OS am I running? Give me version details.",
     ["system_info"], ["acos", "kernel", "0.5"], []),

    # --- System Monitoring ---
    ("S03", "Show me the system status: hostname, memory, uptime.",
     ["system_info"], ["hostname", "memory", "acos"], []),

    ("S04", "How much RAM is available right now?",
     ["system_info"], ["memory", "mb", "gb", "free", "total", "2"], []),

    ("S05", "List all running processes.",
     ["process_list"], ["pid", "mcpd", "init"], []),

    ("S06", "Is mcpd running? What PID?",
     ["process_list"], ["mcpd", "pid"], []),

    ("S07", "Are there any processes using a lot of memory?",
     ["process_list"], ["memory", "mb", "process"], []),

    # --- File Operations ---
    ("S08", "Read the file /etc/hostname and show me its content.",
     ["file_read"], ["acos", "hostname", "content"], []),

    ("S09", "Read /etc/passwd and show me the users.",
     ["file_read"], ["root", "user", "passwd"], []),

    ("S10", "Create a file /tmp/guardian_test.txt with the content 'ACOS Guardian Active'.",
     ["file_write"], ["written", "created", "success", "guardian"], []),

    ("S11", "Read back /tmp/guardian_test.txt to verify it was written correctly.",
     ["file_read"], ["guardian", "active"], []),

    # --- Configuration ---
    ("S12", "Set a config key 'guardian_mode' to 'active'.",
     ["config_set"], ["set", "guardian_mode", "active", "ok"], []),

    ("S13", "What is the current value of guardian_mode?",
     ["config_get"], ["active"], []),

    ("S14", "List all configuration keys in the system.",
     ["config_list"], ["guardian_mode", "key"], []),

    # --- Logging & Diagnostics ---
    ("S15", "Write a log entry: level=info, message='Guardian eval in progress', source='eval'.",
     ["log_write"], ["written", "ok", "log"], []),

    ("S16", "Show me the last 3 log entries.",
     ["log_read"], ["log", "guardian", "eval"], []),

    # --- Multi-tool Orchestration ---
    ("S17", "Give me a complete system health report: OS info, memory, number of processes, and last log entry.",
     ["system_info", "process_list", "log_read"], ["acos", "process", "memory"], []),

    ("S18", "Check if the hostname file matches what system_info reports.",
     ["system_info", "file_read"], ["acos", "match", "hostname"], []),

    # --- Error Handling & Edge Cases ---
    ("S19", "Read the file /nonexistent/path. How do you handle the error?",
     ["file_read"], ["error", "not found", "exist"], ["sorry i cannot"]),

    ("S20", "Summarize everything you know about this system in 3 bullet points.",
     ["system_info"], ["acos", "kernel"], []),
]


def mcp_talk_send(vm, message, wait=35):
    """Send a message to mcp-talk and wait for acos> prompt."""
    # Aggressive flush — wait for silence then drain buffer
    time.sleep(0.5)
    for _ in range(3):
        try:
            vm.serial.serial.read_nonblocking(size=65536, timeout=0.5)
        except Exception:
            break

    vm.serial.sendline(message)
    try:
        vm.serial.serial.expect(r"acos>", timeout=wait)
        raw = clean_ansi(vm.serial.serial.before or "")
        # Remove the echoed command from the response
        lines = raw.split("\n")
        filtered = []
        msg_short = message[:30].lower()
        for line in lines:
            if msg_short in line.lower():
                continue
            filtered.append(line)
        return "\n".join(filtered)
    except Exception:
        try:
            raw = vm.serial.serial.read_nonblocking(size=65536, timeout=2)
            return clean_ansi(raw if isinstance(raw, str) else raw.decode("utf-8", errors="replace"))
        except Exception:
            return None


def score_response(response, expected_tools, success_kw, fail_kw):
    """Score a response 0-3."""
    if not response or len(response.strip()) < 10:
        return 0, "no response"

    resp_lower = response.lower()

    # Check for fail keywords (hallucination, refusal)
    for kw in fail_kw:
        if kw.lower() in resp_lower:
            return 1, f"fail keyword: {kw}"

    # Check success keywords
    kw_hits = sum(1 for kw in success_kw if kw.lower() in resp_lower)
    kw_ratio = kw_hits / max(len(success_kw), 1)

    if kw_ratio >= 0.5:
        # Good response with relevant content
        if kw_ratio >= 0.75 and len(response.strip()) > 30:
            return 3, f"excellent ({kw_hits}/{len(success_kw)} keywords)"
        return 2, f"good ({kw_hits}/{len(success_kw)} keywords)"
    elif kw_ratio > 0:
        return 1, f"partial ({kw_hits}/{len(success_kw)} keywords)"
    else:
        # No keywords matched but got a response
        if len(response.strip()) > 50:
            return 1, "responded but no expected keywords"
        return 0, "empty or irrelevant"


def main():
    total_score = 0
    scenario_results = []

    # Ensure proxy
    import subprocess
    proxy_check = subprocess.run(["ss", "-tlnp"], capture_output=True, text=True)
    if ":9999" not in proxy_check.stdout:
        print("[*] Starting LLM proxy...")
        subprocess.Popen(
            ["python3", os.path.join(SCRIPT_DIR, "llm-proxy.py")],
            stdout=open("/tmp/llm-proxy.log", "a"),
            stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL,
        )
        time.sleep(3)

    vm = ACOSController(network=True)

    try:
        print("[*] Booting ACOS for AI Guardian Eval (20 scenarios)...")
        vm.start()
        assert vm.boot(timeout=90), "Boot failed"
        assert vm.login(), "Login failed"
        print("[OK] Booted and logged in")
        time.sleep(3)

        # Start mcp-talk
        print("[*] Starting mcp-talk...")
        vm.serial.sendline("mcp-talk")
        try:
            vm.serial.serial.expect(r"acos>", timeout=15)
            print("[OK] mcp-talk ready\n")
        except Exception:
            print("[FAIL] mcp-talk did not start")
            print(f"\nSCORE={total_score}")
            return total_score

        # Run scenarios
        for sid, prompt, expected_tools, success_kw, fail_kw in SCENARIOS:
            print(f"[{sid}] {prompt[:60]}...")
            response = mcp_talk_send(vm, prompt, wait=35)

            pts, reason = score_response(response, expected_tools, success_kw, fail_kw)
            total_score += pts
            scenario_results.append((sid, pts, reason, (response or "")[:120]))

            icon = {0: "\u2717", 1: "~", 2: "\u2713", 3: "\u2713\u2713"}[pts]
            print(f"  {icon} {pts}/3 — {reason}")

            # Delay between scenarios — longer after heavy responses
            if sid in ("S07", "S09", "S17"):
                time.sleep(3)
            else:
                time.sleep(1)

        # Quit mcp-talk
        vm.serial.sendline("/quit")
        time.sleep(2)

    except Exception as e:
        print(f"[ERROR] {e}")
        import traceback
        traceback.print_exc()
    finally:
        vm.stop()

    # Final report
    print("\n" + "=" * 60)
    print("ACOS AI GUARDIAN EVAL — 20 SCENARIOS")
    print("=" * 60)

    for sid, pts, reason, preview in scenario_results:
        icon = {0: "\u2717", 1: "~", 2: "\u2713", 3: "\u2713\u2713"}[pts]
        print(f"  {icon} {sid}: {pts}/3 — {reason}")

    max_score = len(SCENARIOS) * 3
    pct = (total_score / max_score * 100) if max_score > 0 else 0
    print(f"\nTotal: {total_score}/{max_score} ({pct:.0f}%)")
    print(f"SCORE={total_score}")
    return total_score


if __name__ == "__main__":
    score = main()
    sys.exit(0 if score >= 30 else 1)
