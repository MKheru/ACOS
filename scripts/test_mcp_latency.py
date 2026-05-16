#!/usr/bin/env python3
"""
WS0.3 — WARN-only MCP latency probe for ACOS QEMU.

Runs an in-guest MCP command repeatedly and records latency percentiles as JSON.
Timing-source decision:
- Prefer `date +%s%N` because it measures only in-guest command duration and
  excludes serial/QEMU host round-trip overhead.
- Fall back to host-side monotonic timing only when the guest does not expose a
  nanosecond-capable `date`; fallback samples are explicitly marked because
  they include serial transport overhead and are not comparable to in-guest p99.

Default probe command: `mcp-query system info`.
This script is WARN-only: latency thresholds and per-iteration command failures
are reported in JSON warnings, not as hard test failures. Harness failures
(QEMU boot/login/no samples) still return non-zero.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import sys
import time
from dataclasses import dataclass
from typing import Iterable, List, Optional

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

LATENCY_RE = re.compile(r"ACOS_LATENCY_RESULT:(?P<rc>\d+):(?P<t0>\d+):(?P<t1>\d+)")


@dataclass
class Sample:
    iteration: int
    latency_us: int
    rc: int
    timing_source: str


def percentile(values: List[int], pct: float) -> int:
    """Nearest-rank percentile for small deterministic harness samples."""
    if not values:
        raise ValueError("percentile requires at least one value")
    ordered = sorted(values)
    rank = max(1, int((pct / 100.0) * len(ordered) + 0.999999999))
    return ordered[min(rank, len(ordered)) - 1]


def parse_latency_line(output: Optional[str]) -> Optional[tuple[int, int, int]]:
    if not output:
        return None
    match = LATENCY_RE.search(output)
    if not match:
        return None
    rc = int(match.group("rc"))
    t0 = int(match.group("t0"))
    t1 = int(match.group("t1"))
    if t1 < t0:
        return None
    return rc, t0, t1


def quote_for_ion_single_arg(value: str) -> str:
    """Quote a string for the simple commands used here; reject single quotes."""
    if "'" in value:
        raise ValueError("probe command cannot contain single quotes")
    return f"'{value}'"


def detect_timing_source(vm: ACOSController) -> str:
    probe = vm.run("date +%s%N", timeout=5)
    if probe:
        compact = "".join(line.strip() for line in probe.splitlines())
        if compact.isdigit() and len(compact) >= 13:
            return "guest_date_ns"
    return "host_monotonic_warn_fallback"


def run_guest_date_sample(vm: ACOSController, command: str, iteration: int, timeout: int) -> Optional[Sample]:
    quoted_command = quote_for_ion_single_arg(command)
    wrapped = (
        "t0=$(date +%s%N); "
        f"sh -c {quoted_command} >/tmp/acos_mcp_latency.out 2>&1; "
        "rc=$?; t1=$(date +%s%N); "
        "echo ACOS_LATENCY_RESULT:$rc:$t0:$t1"
    )
    parsed = parse_latency_line(vm.run(wrapped, timeout=timeout))
    if not parsed:
        return None
    rc, t0, t1 = parsed
    return Sample(iteration=iteration, latency_us=(t1 - t0) // 1000, rc=rc, timing_source="guest_date_ns")


def run_host_fallback_sample(vm: ACOSController, command: str, iteration: int, timeout: int) -> Optional[Sample]:
    start = time.monotonic_ns()
    output = vm.run(f"{command} >/tmp/acos_mcp_latency.out 2>&1; echo ACOS_LATENCY_RC:$?", timeout=timeout)
    end = time.monotonic_ns()
    if output is None:
        return None
    match = re.search(r"ACOS_LATENCY_RC:(\d+)", output)
    rc = int(match.group(1)) if match else 1
    return Sample(
        iteration=iteration,
        latency_us=(end - start) // 1000,
        rc=rc,
        timing_source="host_monotonic_warn_fallback",
    )


def summarize(samples: Iterable[Sample], iterations_requested: int, command: str) -> dict:
    samples = list(samples)
    latencies = [s.latency_us for s in samples]
    failed_rc = [s.iteration for s in samples if s.rc != 0]
    warnings = []
    if len(samples) != iterations_requested:
        warnings.append(f"only {len(samples)}/{iterations_requested} samples collected")
    if failed_rc:
        warnings.append(f"probe command returned non-zero in {len(failed_rc)} iterations")
    timing_sources = sorted({s.timing_source for s in samples})
    if "host_monotonic_warn_fallback" in timing_sources:
        warnings.append("guest date +%s%N unavailable; host fallback includes serial/QEMU overhead")

    result = {
        "schema": "acos.ws0.3.mcp_latency.v1",
        "status": "warn" if warnings else "ok",
        "warn_only": True,
        "command": command,
        "iterations_requested": iterations_requested,
        "iterations_collected": len(samples),
        "timing_sources": timing_sources,
        "latency_us": {},
        "failures": {"nonzero_rc_iterations": failed_rc},
        "warnings": warnings,
    }
    if latencies:
        result["latency_us"] = {
            "min": min(latencies),
            "p50": percentile(latencies, 50),
            "p95": percentile(latencies, 95),
            "p99": percentile(latencies, 99),
            "max": max(latencies),
            "mean": round(statistics.fmean(latencies), 2),
        }
    return result


def run_probe(args: argparse.Namespace) -> int:
    from acos_qemu import ACOSController

    samples: List[Sample] = []
    with ACOSController(network=not args.no_network) as vm:
        print("[*] Starting QEMU for MCP latency probe...", file=sys.stderr)
        vm.start()
        if not vm.boot(timeout=args.boot_timeout):
            print("[FAIL] QEMU boot timeout", file=sys.stderr)
            return 2
        if not vm.login():
            print("[FAIL] QEMU login failed", file=sys.stderr)
            return 2

        timing_source = detect_timing_source(vm)
        print(f"[*] timing_source={timing_source}", file=sys.stderr)
        for i in range(1, args.iterations + 1):
            if timing_source == "guest_date_ns":
                sample = run_guest_date_sample(vm, args.command, i, args.command_timeout)
            else:
                sample = run_host_fallback_sample(vm, args.command, i, args.command_timeout)
            if sample is None:
                print(f"[WARN] iteration {i}: unable to parse latency sample", file=sys.stderr)
                continue
            samples.append(sample)
            if args.progress and (i == 1 or i % args.progress == 0 or i == args.iterations):
                print(f"[*] collected {len(samples)}/{i} samples", file=sys.stderr)

    report = summarize(samples, args.iterations, args.command)
    output = json.dumps(report, indent=2, sort_keys=True)
    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as f:
            f.write(output + "\n")
    print(output)
    return 0 if samples else 1


def self_test() -> int:
    assert percentile([10, 20, 30, 40], 50) == 20
    assert percentile([10, 20, 30, 40], 95) == 40
    assert parse_latency_line("x ACOS_LATENCY_RESULT:0:1000:2500 y") == (0, 1000, 2500)
    assert parse_latency_line("ACOS_LATENCY_RESULT:0:2500:1000") is None
    report = summarize(
        [Sample(1, 10, 0, "guest_date_ns"), Sample(2, 20, 1, "guest_date_ns")],
        2,
        "mcp-query system info",
    )
    assert report["latency_us"]["p50"] == 10
    assert report["latency_us"]["p99"] == 20
    assert report["status"] == "warn"
    print("self-test ok")
    return 0


def parse_args(argv: List[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="WARN-only ACOS MCP latency probe")
    parser.add_argument("--iterations", type=int, default=100)
    parser.add_argument("--command", default="mcp-query system info")
    parser.add_argument("--json-out")
    parser.add_argument("--boot-timeout", type=int, default=90)
    parser.add_argument("--command-timeout", type=int, default=10)
    parser.add_argument("--progress", type=int, default=10)
    parser.add_argument("--no-network", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.iterations <= 0:
        parser.error("--iterations must be > 0")
    if args.progress < 0:
        parser.error("--progress must be >= 0")
    return args


def main(argv: List[str]) -> int:
    args = parse_args(argv)
    if args.self_test:
        return self_test()
    return run_probe(args)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
