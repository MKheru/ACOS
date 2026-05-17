#!/usr/bin/env python3
"""WS0.5 — baseline calibration for ACOS QEMU boot and MCP latency.

Runs repeated clean boots and in-guest MCP calls, records image SHA256 and git
revision for every run, and writes deterministic mean+2σ thresholds to
``baselines/thresholds.json``.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_DIR = SCRIPT_DIR.parent
BASELINES_DIR = PROJECT_DIR / "baselines"
THRESHOLDS_PATH = BASELINES_DIR / "thresholds.json"
DEFAULT_IMAGE = PROJECT_DIR / "redox_base" / "build" / "x86_64" / "acos-bare" / "harddrive.img"
DEFAULT_COMMAND = "mcp-query system info"

sys.path.insert(0, str(SCRIPT_DIR))


def percentile(values: List[int], pct: float) -> int:
    if not values:
        raise ValueError("percentile requires at least one value")
    ordered = sorted(values)
    rank = max(1, math.ceil((pct / 100.0) * len(ordered)))
    return ordered[min(rank, len(ordered)) - 1]


def mean_plus_2sigma(values: Iterable[int | float]) -> float:
    vals = [float(v) for v in values]
    if not vals:
        raise ValueError("mean_plus_2sigma requires at least one value")
    if len(vals) == 1:
        return round(vals[0], 2)
    return round(statistics.fmean(vals) + (2.0 * statistics.pstdev(vals)), 2)


def sha256_file(path: str | Path, chunk_size: int = 1024 * 1024) -> str:
    import hashlib

    digest = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(chunk_size), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_revision(repo: str | Path = PROJECT_DIR) -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short=12", "HEAD"],
            cwd=str(repo),
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except Exception:
        return "unknown"


@dataclass(frozen=True)
class CalibrationRun:
    run: int
    image_sha256: str
    git_rev: str
    boot_ms: int
    mcp_latency_us: tuple[int, ...]
    mcp_failures: int

    def to_json(self) -> dict:
        latencies = list(self.mcp_latency_us)
        return {
            "run": self.run,
            "image_sha256": self.image_sha256,
            "git_rev": self.git_rev,
            "boot_ms": self.boot_ms,
            "mcp_calls": len(latencies),
            "mcp_failures": self.mcp_failures,
            "mcp_latency_us": summarize_values(latencies),
        }


def summarize_values(values: List[int]) -> dict:
    if not values:
        return {}
    return {
        "min": min(values),
        "p50": percentile(values, 50),
        "p95": percentile(values, 95),
        "p99": percentile(values, 99),
        "max": max(values),
        "mean": round(statistics.fmean(values), 2),
    }


def build_thresholds(runs: List[CalibrationRun], command: str) -> dict:
    if not runs:
        raise ValueError("at least one calibration run is required")
    boot_values = [r.boot_ms for r in runs]
    all_latencies = [lat for run in runs for lat in run.mcp_latency_us]
    if not all_latencies:
        raise ValueError("at least one MCP latency sample is required")
    image_shas = sorted({r.image_sha256 for r in runs})
    git_revs = sorted({r.git_rev for r in runs})
    return {
        "schema": "acos.ws0.5.thresholds.v1",
        "generated_at_unix": int(time.time()),
        "calibration": {
            "boot_runs": len(runs),
            "mcp_calls_total": len(all_latencies),
            "command": command,
        },
        "image_sha256": image_shas[-1] if len(image_shas) == 1 else image_shas,
        "git_rev": git_revs[-1] if len(git_revs) == 1 else git_revs,
        "thresholds": {
            "boot_ms_max": mean_plus_2sigma(boot_values),
            "mcp_latency_us_p50_max": mean_plus_2sigma([summarize_values(list(r.mcp_latency_us))["p50"] for r in runs]),
            "mcp_latency_us_p95_max": mean_plus_2sigma([summarize_values(list(r.mcp_latency_us))["p95"] for r in runs]),
            "mcp_latency_us_p99_max": mean_plus_2sigma([summarize_values(list(r.mcp_latency_us))["p99"] for r in runs]),
        },
        "observed": {
            "boot_ms": summarize_values(boot_values),
            "mcp_latency_us": summarize_values(all_latencies),
        },
        "runs": [r.to_json() for r in runs],
    }


def quote_for_ion_single_arg(value: str) -> str:
    if "'" in value:
        raise ValueError("probe command cannot contain single quotes")
    return f"'{value}'"


def run_mcp_sample(vm, command: str, timeout: int) -> tuple[int | None, bool]:
    wrapped = (
        "t0=$(date +%s%N); "
        f"sh -c {quote_for_ion_single_arg(command)} >/tmp/acos_ws05_mcp.out 2>&1; "
        "rc=$?; t1=$(date +%s%N); "
        "echo ACOS_WS05_MCP:$rc:$t0:$t1"
    )
    output = vm.run(wrapped, timeout=timeout)
    if not output:
        return None, True
    import re

    match = re.search(r"ACOS_WS05_MCP:(\d+):(\d+):(\d+)", output)
    if not match:
        return None, True
    rc, t0, t1 = int(match.group(1)), int(match.group(2)), int(match.group(3))
    if t1 < t0:
        return None, True
    return (t1 - t0) // 1000, rc != 0


def run_calibration(args: argparse.Namespace) -> int:
    from acos_qemu import ACOSController, ArtifactManager

    image = Path(args.image)
    image_sha = sha256_file(image)
    git_rev = git_revision()
    runs: List[CalibrationRun] = []
    for run_no in range(1, args.boots + 1):
        latencies: list[int] = []
        failures = 0
        with ACOSController(image=str(image), network=not args.no_network) as vm:
            start = time.monotonic_ns()
            vm.start()
            if not vm.boot(timeout=args.boot_timeout):
                raise RuntimeError(f"boot {run_no} timed out")
            boot_ms = (time.monotonic_ns() - start) // 1_000_000
            if not vm.login():
                raise RuntimeError(f"boot {run_no} login failed")
            for _ in range(args.mcp_calls):
                latency, failed = run_mcp_sample(vm, args.command, args.command_timeout)
                if latency is None or failed:
                    failures += 1
                    continue
                latencies.append(latency)
        runs.append(
            CalibrationRun(
                run=run_no,
                image_sha256=image_sha,
                git_rev=git_rev,
                boot_ms=int(boot_ms),
                mcp_latency_us=tuple(latencies),
                mcp_failures=failures,
            )
        )
        print(f"[*] run {run_no}/{args.boots}: boot_ms={boot_ms} mcp_ok={len(latencies)}/{args.mcp_calls}", file=sys.stderr)

    payload = build_thresholds(runs, args.command)
    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    manager = ArtifactManager("ws0.5")
    artifact = manager.write_json(payload)
    manager.prune()
    print(json.dumps({"thresholds": str(output_path), "artifact": artifact}, indent=2, sort_keys=True))
    return 0


def self_test() -> int:
    runs = [
        CalibrationRun(1, "a" * 64, "rev1", 1000, (10, 20, 30), 0),
        CalibrationRun(2, "a" * 64, "rev1", 1200, (20, 40, 60), 1),
    ]
    payload = build_thresholds(runs, "mcp-query system info")
    assert payload["schema"] == "acos.ws0.5.thresholds.v1"
    assert payload["calibration"]["boot_runs"] == 2
    assert payload["calibration"]["mcp_calls_total"] == 6
    assert payload["image_sha256"] == "a" * 64
    assert payload["thresholds"]["boot_ms_max"] >= 1200
    assert summarize_values([1, 2, 3, 4])["p99"] == 4
    print("self-test ok")
    return 0


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Calibrate ACOS QEMU baseline thresholds")
    parser.add_argument("--boots", type=int, default=10)
    parser.add_argument("--mcp-calls", type=int, default=100)
    parser.add_argument("--command", default=DEFAULT_COMMAND)
    parser.add_argument("--image", default=str(DEFAULT_IMAGE))
    parser.add_argument("--output", default=str(THRESHOLDS_PATH))
    parser.add_argument("--boot-timeout", type=int, default=90)
    parser.add_argument("--command-timeout", type=int, default=10)
    parser.add_argument("--no-network", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.boots <= 0:
        parser.error("--boots must be > 0")
    if args.mcp_calls <= 0:
        parser.error("--mcp-calls must be > 0")
    return args


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.self_test:
        return self_test()
    return run_calibration(args)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
