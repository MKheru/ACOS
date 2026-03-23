#!/usr/bin/env python3
"""
ACOS AutoResearch Evaluation Harness

This script evaluates a modification to the MCP scheme component by:
1. Compiling the Rust component (cargo build + cargo test)
2. Optionally: injecting into Redox image and booting in QEMU headless
3. Measuring metrics (compilation time, test pass rate, latency benchmarks)
4. Outputting a composite score

Usage:
    python evaluate.py                    # Quick mode: compile + unit tests only
    python evaluate.py --full             # Full mode: + QEMU integration test
    python evaluate.py --bench            # Benchmark mode: + latency measurements

Output format (last line, parseable):
    SCORE: <float>

Exit codes:
    0 = success (score printed)
    1 = compilation failure
    2 = test failure
    3 = QEMU boot failure
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

# Paths
PROJECT_ROOT = Path(__file__).parent.parent
COMPONENT_DIR = PROJECT_ROOT / "components" / "mcp_scheme"
REDOX_DIR = PROJECT_ROOT / "redox_base"
RESULTS_DIR = PROJECT_ROOT / "evolution" / "results"

HARNESS_DIR = Path(__file__).parent

# Default commands — overridden when --lab config specifies host_test commands
_COMPILE_CMD = ["cargo", "build", "--features", "host-test"]
_TEST_CMD = ["cargo", "test", "--features", "host-test"]


def run_cmd(cmd, cwd=None, timeout=300):
    """Run a command and return (success, stdout, stderr, duration)."""
    t0 = time.time()
    try:
        result = subprocess.run(
            cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout
        )
        dt = time.time() - t0
        return result.returncode == 0, result.stdout, result.stderr, dt
    except subprocess.TimeoutExpired:
        return False, "", f"TIMEOUT after {timeout}s", time.time() - t0


def evaluate_compile():
    """Compile the component. Returns (success, duration_s)."""
    print("=== COMPILE ===")
    success, stdout, stderr, dt = run_cmd(
        _COMPILE_CMD,
        cwd=COMPONENT_DIR
    )
    if not success:
        print(f"COMPILE FAILED ({dt:.1f}s):")
        print(stderr[-2000:] if len(stderr) > 2000 else stderr)
        return False, dt
    print(f"COMPILE OK ({dt:.1f}s)")
    return True, dt


def evaluate_tests():
    """Run unit tests. Returns (success, pass_count, fail_count, duration_s)."""
    print("=== TESTS ===")
    success, stdout, stderr, dt = run_cmd(
        _TEST_CMD,
        cwd=COMPONENT_DIR
    )

    # Parse test results from stderr (stable cargo test output)
    pass_count = 0
    fail_count = 0
    output = stdout + "\n" + stderr
    for line in output.splitlines():
        if "test result:" in line:
            parts = line.split()
            for i, p in enumerate(parts):
                if p == "passed;":
                    pass_count += int(parts[i - 1])
                elif p == "failed;":
                    fail_count += int(parts[i - 1])

    total = pass_count + fail_count
    if total == 0:
        print(f"NO TESTS FOUND ({dt:.1f}s)")
        print(stderr[-1000:])
        return False, 0, 0, dt

    rate = pass_count / total if total > 0 else 0
    print(f"TESTS: {pass_count}/{total} passed ({rate:.0%}) in {dt:.1f}s")

    if not success:
        print("TEST OUTPUT:")
        print(stderr[-2000:] if len(stderr) > 2000 else stderr)

    return fail_count == 0, pass_count, fail_count, dt


def evaluate_bench():
    """Run benchmarks if available. Returns (latency_us, throughput_ops)."""
    print("=== BENCH ===")
    success, stdout, stderr, dt = run_cmd(
        ["cargo", "bench", "--features", "host-test"],
        cwd=COMPONENT_DIR,
        timeout=120
    )

    if not success:
        print(f"BENCH skipped or failed ({dt:.1f}s)")
        return None, None

    # Parse criterion output for latency
    latency_us = None
    for line in stdout.splitlines():
        if "time:" in line.lower() and "ns" in line:
            # Try to extract timing
            parts = line.split()
            for i, p in enumerate(parts):
                if p == "ns" and i > 0:
                    try:
                        latency_us = float(parts[i - 1]) / 1000.0
                    except ValueError:
                        pass

    print(f"BENCH: latency={latency_us}us" if latency_us else "BENCH: no latency data")
    return latency_us, None


def compute_score(compile_ok, compile_time, test_pass, test_total, latency_us=None):
    """
    Compute a composite score (higher is better).

    Score = compile_bonus + test_score + speed_bonus

    - compile_bonus: 100 if compiles, 0 otherwise
    - test_score: (pass_rate * 200) — up to 200 points
    - speed_bonus: 100 / (1 + compile_time/10) — faster compile = higher
    - latency_bonus: 100 / (1 + latency_us) if available
    """
    if not compile_ok:
        return 0.0

    score = 100.0  # compile bonus

    # Test score
    if test_total > 0:
        pass_rate = test_pass / test_total
        score += pass_rate * 200.0
    else:
        score += 50.0  # partial credit for no tests crashing

    # Compile speed bonus
    score += 100.0 / (1.0 + compile_time / 10.0)

    # Latency bonus
    if latency_us is not None and latency_us > 0:
        score += 100.0 / (1.0 + latency_us)

    return round(score, 4)


def save_result(score, details):
    """Append result to results TSV."""
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    results_file = RESULTS_DIR / "mcp_scheme_results.tsv"

    timestamp = time.strftime("%Y-%m-%d %H:%M:%S")
    header = "timestamp\tscore\tcompile\ttests_pass\ttests_total\tcompile_time\tnotes\n"

    if not results_file.exists():
        results_file.write_text(header)

    with open(results_file, "a") as f:
        f.write(f"{timestamp}\t{score}\t{details.get('compile', False)}\t"
                f"{details.get('test_pass', 0)}\t{details.get('test_total', 0)}\t"
                f"{details.get('compile_time', 0):.1f}\t{details.get('notes', '')}\n")


def load_lab_config(lab_id):
    """Load lab config via parse_lab.load_lab(). Returns config dict."""
    sys.path.insert(0, str(HARNESS_DIR))
    try:
        from parse_lab import load_lab
        cfg = load_lab(lab_id)
        if cfg is None:
            print(f"Failed to load lab config for '{lab_id}'")
            sys.exit(1)
        return cfg
    except Exception as e:
        print(f"Failed to load lab config for '{lab_id}': {e}")
        sys.exit(1)


def main():
    parser = argparse.ArgumentParser(description="ACOS AutoResearch Evaluation Harness")
    parser.add_argument("--full", action="store_true", help="Include QEMU integration test")
    parser.add_argument("--bench", action="store_true", help="Include benchmarks")
    parser.add_argument("--lab", metavar="LAB_ID", help="Use lab config for paths and commands")
    parser.add_argument("--round", type=int, default=1, help="Round number (used with --lab)")
    args = parser.parse_args()

    # --lab + --full: delegate entirely to autoresearch.sh
    if args.lab and args.full:
        result = subprocess.run(
            ["bash", str(HARNESS_DIR / "autoresearch.sh"), args.lab, str(args.round)]
        )
        sys.exit(result.returncode)

    # When --lab provided, override paths and commands from config
    global COMPONENT_DIR, _COMPILE_CMD, _TEST_CMD

    if args.lab:
        cfg = load_lab_config(args.lab)
        component = cfg.get("component", "mcp_scheme")
        source_dir = cfg.get("source_dir")
        if source_dir:
            COMPONENT_DIR = Path(source_dir)
        else:
            COMPONENT_DIR = PROJECT_ROOT / "components" / component

        host_test = cfg.get("host_test", {})
        if host_test.get("compile"):
            _COMPILE_CMD = host_test["compile"]
        if host_test.get("test"):
            _TEST_CMD = host_test["test"]

    details = {}

    # Step 1: Compile
    compile_ok, compile_time = evaluate_compile()
    details["compile"] = compile_ok
    details["compile_time"] = compile_time
    if not compile_ok:
        score = 0.0
        details["notes"] = "compilation_failed"
        save_result(score, details)
        print(f"\nSCORE: {score}")
        sys.exit(1)

    # Step 2: Tests
    test_ok, test_pass, test_fail, test_time = evaluate_tests()
    details["test_pass"] = test_pass
    details["test_total"] = test_pass + test_fail
    if not test_ok:
        details["notes"] = "tests_failed"

    # Step 3: Benchmarks (optional)
    latency_us = None
    if args.bench:
        latency_us, _ = evaluate_bench()

    # Step 4: QEMU integration (optional)
    if args.full:
        print("=== QEMU ===")
        if args.lab:
            qemu_result = subprocess.run(
                ["bash", str(HARNESS_DIR / "qemu_inject.sh"), args.lab, str(args.round)],
                capture_output=True, text=True
            )
            if qemu_result.returncode != 0:
                print(f"QEMU injection failed: {qemu_result.stderr.strip()}")
                details["notes"] = details.get("notes", "") + " qemu_failed"
            else:
                print("QEMU injection OK")
                details["notes"] = details.get("notes", "") + " qemu_ok"
        else:
            print("QEMU integration test not yet implemented")
            details["notes"] = details.get("notes", "") + " qemu_skipped"

    # Compute and report score
    score = compute_score(
        compile_ok, compile_time,
        test_pass, details["test_total"],
        latency_us
    )

    save_result(score, details)

    print(f"\n{'='*40}")
    print(f"SCORE: {score}")

    sys.exit(0 if test_ok else 2)


if __name__ == "__main__":
    main()
