#!/usr/bin/env python3
"""WS0.5 — boot determinism harness.

This pytest-compatible module verifies that baseline threshold artifacts are
well-formed and provides an opt-in real QEMU determinism run. Real QEMU is not
executed unless ``ACOS_RUN_QEMU_DETERMINISM=1`` is set.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_DIR = SCRIPT_DIR.parent
sys.path.insert(0, str(SCRIPT_DIR))

from baseline_calibrate import (  # noqa: E402
    CalibrationRun,
    build_thresholds,
    mean_plus_2sigma,
    sha256_file,
    summarize_values,
)

REQUIRED_THRESHOLD_KEYS = {
    "boot_ms_max",
    "mcp_latency_us_p50_max",
    "mcp_latency_us_p95_max",
    "mcp_latency_us_p99_max",
}


def validate_threshold_payload(payload: dict) -> None:
    assert payload["schema"] == "acos.ws0.5.thresholds.v1"
    assert payload["calibration"]["boot_runs"] >= 1
    assert payload["calibration"]["mcp_calls_total"] >= 1
    assert REQUIRED_THRESHOLD_KEYS <= set(payload["thresholds"])
    assert payload["image_sha256"]
    assert payload["git_rev"]
    for run in payload["runs"]:
        assert run["image_sha256"]
        assert run["git_rev"]
        assert run["boot_ms"] >= 0
        assert run["mcp_calls"] >= 0


def test_threshold_payload_records_image_sha_and_git_rev_per_run():
    runs = [
        CalibrationRun(1, "f" * 64, "abc123", 1000, (10, 20, 30), 0),
        CalibrationRun(2, "f" * 64, "abc123", 1100, (20, 30, 40), 0),
    ]

    payload = build_thresholds(runs, "mcp-query system info")

    validate_threshold_payload(payload)
    assert payload["image_sha256"] == "f" * 64
    assert payload["git_rev"] == "abc123"
    assert payload["runs"][0]["image_sha256"] == "f" * 64
    assert payload["runs"][0]["git_rev"] == "abc123"


def test_thresholds_are_mean_plus_two_sigma():
    runs = [
        CalibrationRun(1, "a" * 64, "rev", 100, (10, 20, 30), 0),
        CalibrationRun(2, "a" * 64, "rev", 200, (20, 40, 60), 0),
        CalibrationRun(3, "a" * 64, "rev", 300, (30, 60, 90), 0),
    ]

    payload = build_thresholds(runs, "mcp-query system info")

    assert payload["thresholds"]["boot_ms_max"] == mean_plus_2sigma([100, 200, 300])
    assert payload["thresholds"]["mcp_latency_us_p99_max"] == mean_plus_2sigma([30, 60, 90])


def test_sha256_file(tmp_path):
    path = tmp_path / "image.img"
    path.write_bytes(b"acos-image")

    assert sha256_file(path) == "c7ade82f5908d1579413019955c33f6b7d9fa7e5fb3fd1c4c582bc85172b109f"


def test_summarize_values_percentiles():
    summary = summarize_values([10, 20, 30, 40])

    assert summary["p50"] == 20
    assert summary["p95"] == 40
    assert summary["p99"] == 40


def test_optional_real_qemu_baseline_calibration(tmp_path):
    if os.environ.get("ACOS_RUN_QEMU_DETERMINISM") != "1":
        return

    from baseline_calibrate import main

    output = tmp_path / "thresholds.json"
    rc = main([
        "--boots", "10",
        "--mcp-calls", "100",
        "--output", str(output),
    ])
    assert rc == 0
    payload = json.loads(output.read_text(encoding="utf-8"))
    validate_threshold_payload(payload)
    assert payload["calibration"]["boot_runs"] == 10
    assert payload["calibration"]["mcp_calls_total"] >= 1


if __name__ == "__main__":
    # Lightweight smoke mode without pytest.
    test_threshold_payload_records_image_sha_and_git_rev_per_run()
    test_thresholds_are_mean_plus_two_sigma()
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        test_sha256_file(Path(td))
    test_summarize_values_percentiles()
    print("self-test ok")
