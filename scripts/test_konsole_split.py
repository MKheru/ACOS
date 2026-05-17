#!/usr/bin/env python3
"""
WS0.4 — ACOS Konsole split-screen visual smoke test.

Boots ACOS in QEMU, captures a VGA screenshot, compares it to the checked-in
idle split baseline, and records JSON evidence that the screen still contains a
split midline plus at least one textual keyword and one active quadrant.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import List, Sequence

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_DIR = SCRIPT_DIR.parent
sys.path.insert(0, str(SCRIPT_DIR))

from visual_baseline import evaluate_konsole_split  # noqa: E402

DEFAULT_BASELINE = PROJECT_DIR / "baselines" / "konsole_split_idle.ppm"
DEFAULT_KEYWORDS = ["acos", "root", "ion", "mcp", "guardian", "login"]


def collect_evidence_text(vm) -> str:
    """Collect cheap textual evidence from serial; OCR is optional/out of scope."""
    chunks = []
    try:
        chunks.append(vm.run("echo ACOS_KONSOLE_SPLIT_EVIDENCE", timeout=5) or "")
    except Exception as exc:  # pragma: no cover - live QEMU only
        chunks.append(f"serial_echo_error={exc}")
    try:
        chunks.append(vm.run("ps", timeout=5) or "")
    except Exception as exc:  # pragma: no cover - live QEMU only
        chunks.append(f"serial_ps_error={exc}")
    return "\n".join(chunks)


def build_report(screenshot: Path, baseline: Path, evidence_text: str, keywords: Sequence[str]) -> dict:
    report = evaluate_konsole_split(screenshot, baseline, evidence_text, keywords)
    report.update(
        {
            "schema": "acos.ws0.4.konsole_split.v1",
            "screenshot": str(screenshot),
            "baseline": str(baseline),
            "keywords_checked": list(keywords),
        }
    )
    return report


def run_live(args: argparse.Namespace) -> int:
    from acos_qemu import ACOSController, ArtifactManager

    baseline = Path(args.baseline)
    if not baseline.exists():
        print(f"[FAIL] baseline missing: {baseline}", file=sys.stderr)
        return 2

    manager = ArtifactManager("ws0_4_konsole_split")
    with ACOSController(network=not args.no_network) as vm:
        print("[*] Starting QEMU for Konsole split visual test...", file=sys.stderr)
        vm.start()
        if not vm.boot(timeout=args.boot_timeout):
            print("[FAIL] QEMU boot timeout", file=sys.stderr)
            return 2
        if not vm.login():
            print("[FAIL] QEMU login failed", file=sys.stderr)
            return 2

        screenshot_paths = manager.capture_screenshot(vm, "konsole_split_idle")
        screenshot = Path(screenshot_paths["ppm"])
        evidence_text = collect_evidence_text(vm)

    report = build_report(screenshot, baseline, evidence_text, args.keyword)
    report["artifacts"] = screenshot_paths
    json_path = args.json_out or manager.json_path
    Path(json_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 1


def self_test() -> int:
    report = build_report(DEFAULT_BASELINE, DEFAULT_BASELINE, "ACOS root ion", DEFAULT_KEYWORDS)
    assert report["ok"] is True
    assert report["split_midline_detected"] is True
    assert report["pixel_diff_ok"] is True
    assert report["keyword_hits"]
    assert report["active_quadrants"]
    print("self-test ok")
    return 0


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="ACOS Konsole split-screen visual smoke test")
    parser.add_argument("--baseline", default=str(DEFAULT_BASELINE))
    parser.add_argument("--json-out")
    parser.add_argument("--boot-timeout", type=int, default=90)
    parser.add_argument("--no-network", action="store_true")
    parser.add_argument("--keyword", action="append", default=list(DEFAULT_KEYWORDS))
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.boot_timeout <= 0:
        parser.error("--boot-timeout must be > 0")
    if not args.keyword:
        parser.error("at least one --keyword is required")
    return args


def main(argv: List[str]) -> int:
    args = parse_args(argv)
    if args.self_test:
        return self_test()
    return run_live(args)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
