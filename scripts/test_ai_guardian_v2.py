#!/usr/bin/env python3
"""
ACOS AI Guardian Eval v2 — Real evaluation with proxy log verification.

Unlike v1 (keyword matching), this eval:
1. Captures REAL tool calls from the LLM proxy logs
2. Verifies the AI called the RIGHT tools for each scenario
3. Tests reasoning scenarios (not just data retrieval)
4. Scores on 3 axes: tool_accuracy, response_quality, reasoning

15 scenarios, each scored 0-5:
  tool_accuracy (0-2): did the AI call the expected tools?
  response_quality (0-2): is the response relevant and well-formed?
  reasoning (0-1): did the AI reason correctly (not just dump data)?

SCORE = 0-75
"""

import os
import sys
import time
import json
import re
import subprocess

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

from acos_qemu import ACOSController, clean_ansi

PROXY_LOG = "/tmp/llm-proxy-eval.log"


# ---------------------------------------------------------------------------
# Scenarios: (id, prompt, expected_tools, quality_keywords, reasoning_check)
#
# expected_tools: list of tool names that MUST appear in proxy logs
# quality_keywords: words that should be in the response (loose check)
# reasoning_check: callable(response) -> bool for reasoning evaluation
# ---------------------------------------------------------------------------

def check_formatted_numbers(resp):
    """Response should have human-readable numbers (MB/GB), not raw bytes."""
    return bool(re.search(r'\d+\s*(MB|GB|mb|gb|Mo|Go)', resp))

def check_table_format(resp):
    """Response should contain a table-like structure (| or aligned columns)."""
    return '|' in resp or resp.count('  ') > 3

def check_error_handling(resp):
    """Response should acknowledge the error, not pretend success."""
    r = resp.lower()
    return any(w in r for w in ["error", "not found", "does not exist", "n'existe", "introuvable", "failed"])

def check_comparison(resp):
    """Response should explicitly compare/match two values."""
    r = resp.lower()
    return any(w in r for w in ["match", "same", "identical", "correspond", "both", "confirms", "consistent"])

def check_diagnosis(resp):
    """Response should identify the highest-memory process."""
    r = resp.lower()
    return any(w in r for w in ["redoxfs", "highest", "most", "largest", "plus", "consomme", "top"])

def check_multi_step(resp):
    """Response should show results from multiple operations."""
    r = resp.lower()
    count = sum(1 for w in ["hostname", "kernel", "process", "memory", "log", "pid", "uptime"] if w in r)
    return count >= 3

def check_written_and_verified(resp):
    """Response should confirm both write and read-back."""
    r = resp.lower()
    return ("guardian" in r or "written" in r or "created" in r) and ("read" in r or "content" in r or "verified" in r or "confirm" in r)

def check_anomaly_reasoning(resp):
    """Response should reason about what's normal vs abnormal."""
    r = resp.lower()
    return any(w in r for w in ["normal", "expected", "healthy", "anomaly", "unusual", "nothing wrong", "all good", "stable", "no issue", "aucun", "sain"])


SCENARIOS = [
    # --- A. Tool Accuracy Tests ---
    ("A1", "What is the hostname of this machine?",
     ["system_info"], ["acos"], None),

    ("A2", "Read the file /etc/motd.",
     ["file_read"], ["acos", "welcome"], None),

    ("A3", "How many processes are currently running?",
     ["process_list"], ["process"], None),

    ("A4", "Set config key 'eval_run' to 'v2' and then read it back to confirm.",
     ["config_set", "config_get"], ["eval_run", "v2"], None),

    ("A5", "Write 'AI Guardian Online' to /tmp/guardian_v2.txt then verify by reading it.",
     ["file_write", "file_read"], ["guardian", "online"], check_written_and_verified),

    # --- B. Response Quality Tests ---
    ("B1", "Give me the system RAM in a human-readable format.",
     ["system_info"], ["memory"], check_formatted_numbers),

    ("B2", "Show me the top 5 processes by memory usage in a table.",
     ["process_list"], ["pid", "memory"], check_table_format),

    ("B3", "Read /nonexistent/file.txt and explain what happened.",
     ["file_read"], ["error", "not found"], check_error_handling),

    ("B4", "What kernel version are we running? Include the full version string.",
     ["system_info"], ["acos", "0.5"], None),

    ("B5", "List all config keys and explain what each one is for.",
     ["config_list"], ["key"], None),

    # --- C. Reasoning Tests ---
    ("C1", "Check if /etc/hostname content matches what system_info reports. Are they consistent?",
     ["system_info", "file_read"], ["acos"], check_comparison),

    ("C2", "Which process is using the most memory? Should I be concerned?",
     ["process_list"], ["memory", "process"], check_diagnosis),

    ("C3", "Generate a complete system health report: OS, memory, processes count, and last log.",
     ["system_info", "process_list", "log_read"], ["acos"], check_multi_step),

    ("C4", "Look at the running processes. Is there anything abnormal or unexpected?",
     ["process_list"], ["process"], check_anomaly_reasoning),

    ("C5", "Write a diagnostic log entry saying the system check passed, then show me the last 3 logs to verify.",
     ["log_write", "log_read"], ["log", "passed"], None),
]


def get_proxy_tool_calls_since(log_path, marker_time):
    """Parse proxy log and return tool calls since marker_time."""
    calls = []
    try:
        with open(log_path) as f:
            for line in f:
                # Format: [HH:MM:SS] Gemini function call: tool_name({args})
                m = re.match(r'\[(\d+:\d+:\d+)\] Gemini function call: (\w+)\(', line)
                if m:
                    ts, tool = m.group(1), m.group(2)
                    if ts >= marker_time:
                        calls.append(tool)
    except FileNotFoundError:
        pass
    return calls


def mcp_talk_send(vm, message, wait=40):
    """Send a message to mcp-talk and wait for acos> prompt."""
    time.sleep(0.5)
    for _ in range(3):
        try:
            vm.serial.serial.read_nonblocking(size=65536, timeout=0.3)
        except Exception:
            break

    vm.serial.sendline(message)
    try:
        vm.serial.serial.expect(r"acos>", timeout=wait)
        raw = clean_ansi(vm.serial.serial.before or "")
        return raw
    except Exception:
        try:
            raw = vm.serial.serial.read_nonblocking(size=65536, timeout=2)
            return clean_ansi(raw if isinstance(raw, str) else raw.decode("utf-8", errors="replace"))
        except Exception:
            return None


def score_scenario(sid, response, expected_tools, quality_kw, reasoning_fn, actual_tools):
    """Score a scenario on 3 axes (0-5 total)."""
    tool_score = 0
    quality_score = 0
    reasoning_score = 0

    # --- Tool Accuracy (0-2) ---
    if expected_tools:
        matched = sum(1 for t in expected_tools if t in actual_tools)
        ratio = matched / len(expected_tools)
        if ratio >= 1.0:
            tool_score = 2
        elif ratio >= 0.5:
            tool_score = 1
        # else 0
    else:
        tool_score = 2  # No tools expected = auto pass

    # --- Response Quality (0-2) ---
    if response and len(response.strip()) > 15:
        resp_lower = response.lower()
        kw_hits = sum(1 for kw in quality_kw if kw.lower() in resp_lower)
        kw_ratio = kw_hits / max(len(quality_kw), 1)
        if kw_ratio >= 0.5:
            quality_score = 2
        elif kw_ratio > 0 or len(response.strip()) > 50:
            quality_score = 1

    # --- Reasoning (0-1) ---
    if reasoning_fn is None:
        reasoning_score = 1  # No reasoning check = auto pass
    elif response and reasoning_fn(response):
        reasoning_score = 1

    return tool_score, quality_score, reasoning_score


def main():
    total = 0
    results = []

    # Start fresh proxy log
    try:
        os.unlink(PROXY_LOG)
    except FileNotFoundError:
        pass

    # Kill old proxy, start new one with dedicated log
    subprocess.run(["pkill", "-f", "llm-proxy.py"], capture_output=True)
    time.sleep(1)
    subprocess.Popen(
        ["python3", os.path.join(SCRIPT_DIR, "llm-proxy.py")],
        stdout=subprocess.DEVNULL,
        stderr=open(PROXY_LOG, "w"),
        stdin=subprocess.DEVNULL,
    )
    time.sleep(3)

    # Verify proxy
    check = subprocess.run(["ss", "-tlnp"], capture_output=True, text=True)
    if ":9999" not in check.stdout:
        print("[FATAL] LLM proxy failed to start")
        print("SCORE=0")
        return 0

    vm = ACOSController(network=True)

    try:
        print("[*] Booting ACOS for AI Guardian Eval v2 (15 scenarios)...")
        vm.start()
        assert vm.boot(timeout=90), "Boot failed"
        assert vm.login(), "Login failed"
        print("[OK] Booted and logged in")
        time.sleep(3)

        # Start mcp-talk
        vm.serial.sendline("mcp-talk")
        try:
            vm.serial.serial.expect(r"acos>", timeout=15)
            print("[OK] mcp-talk ready\n")
        except Exception:
            print("[FAIL] mcp-talk did not start")
            print("SCORE=0")
            return 0

        # Run scenarios
        for sid, prompt, expected_tools, quality_kw, reasoning_fn in SCENARIOS:
            print(f"[{sid}] {prompt[:65]}...")

            # Mark time before sending
            marker = time.strftime("%H:%M:%S")

            response = mcp_talk_send(vm, prompt, wait=40)
            time.sleep(2)  # Let proxy log flush

            # Read tool calls from proxy log
            actual_tools = get_proxy_tool_calls_since(PROXY_LOG, marker)

            # Score
            t, q, r = score_scenario(sid, response, expected_tools, quality_kw, reasoning_fn, actual_tools)
            pts = t + q + r
            total += pts
            results.append((sid, t, q, r, pts, actual_tools, (response or "")[:100]))

            # Display
            tools_str = ",".join(actual_tools) if actual_tools else "(none)"
            print(f"  Tools: [{tools_str}] — accuracy={t}/2, quality={q}/2, reasoning={r}/1 → {pts}/5")

            # Pacing
            time.sleep(1)

        vm.serial.sendline("/quit")
        time.sleep(2)

    except Exception as e:
        print(f"[ERROR] {e}")
        import traceback
        traceback.print_exc()
    finally:
        vm.stop()

    # Final report
    max_score = len(SCENARIOS) * 5
    pct = (total / max_score * 100) if max_score else 0

    print("\n" + "=" * 70)
    print("ACOS AI GUARDIAN EVAL v2 — RESULTS")
    print("=" * 70)
    print(f"{'ID':<5} {'Tools':>8} {'Quality':>8} {'Reason':>8} {'Total':>8}  Actual tools called")
    print("-" * 70)

    sum_t, sum_q, sum_r = 0, 0, 0
    for sid, t, q, r, pts, tools, preview in results:
        tools_str = ",".join(tools) if tools else "-"
        print(f"{sid:<5} {t:>5}/2  {q:>5}/2  {r:>5}/1  {pts:>5}/5   {tools_str}")
        sum_t += t
        sum_q += q
        sum_r += r

    print("-" * 70)
    print(f"{'SUM':<5} {sum_t:>5}/{len(SCENARIOS)*2}  {sum_q:>5}/{len(SCENARIOS)*2}  {sum_r:>5}/{len(SCENARIOS)}  {total:>5}/{max_score}")
    print(f"\nTool Accuracy:    {sum_t}/{len(SCENARIOS)*2} ({sum_t/len(SCENARIOS)/2*100:.0f}%)")
    print(f"Response Quality: {sum_q}/{len(SCENARIOS)*2} ({sum_q/len(SCENARIOS)/2*100:.0f}%)")
    print(f"Reasoning:        {sum_r}/{len(SCENARIOS)} ({sum_r/len(SCENARIOS)*100:.0f}%)")
    print(f"\nTotal: {total}/{max_score} ({pct:.0f}%)")
    print(f"SCORE={total}")
    return total


if __name__ == "__main__":
    score = main()
    sys.exit(0 if score >= 45 else 1)
