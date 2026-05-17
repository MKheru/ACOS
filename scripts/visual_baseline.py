#!/usr/bin/env python3
"""
Visual baseline helpers for ACOS QEMU VGA screenshots.

The module is intentionally stdlib-only so it can run inside the VPS harness
without Pillow. It reads QMP PPM screenshots, detects the konsole split midline,
compares screenshots against a checked-in baseline, and produces quadrant-level
visual evidence for JSON reports.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List, Sequence, Tuple

RGB = Tuple[int, int, int]


def _ppm_tokens(data: bytes) -> Iterable[bytes]:
    token = bytearray()
    in_comment = False
    for byte in data:
        if in_comment:
            if byte in b"\r\n":
                in_comment = False
            continue
        if byte == ord("#"):
            in_comment = True
            if token:
                yield bytes(token)
                token.clear()
            continue
        if byte in b" \t\r\n":
            if token:
                yield bytes(token)
                token.clear()
            continue
        token.append(byte)
    if token:
        yield bytes(token)


def read_ppm(path: str | Path) -> tuple[int, int, bytes]:
    """Read an 8-bit P3/P6 PPM file and return (width, height, rgb_bytes)."""
    raw = Path(path).read_bytes()
    tokens = list(_ppm_tokens(raw))
    if len(tokens) < 4:
        raise ValueError("invalid PPM: missing header")
    magic = tokens[0]
    if magic not in (b"P3", b"P6"):
        raise ValueError(f"unsupported PPM format: {magic!r}")
    width = int(tokens[1])
    height = int(tokens[2])
    maxval = int(tokens[3])
    if width <= 0 or height <= 0:
        raise ValueError("invalid PPM: width/height must be positive")
    if maxval <= 0 or maxval > 255:
        raise ValueError("only 8-bit PPM files are supported")
    expected = width * height * 3
    if magic == b"P3":
        values = [int(tok) for tok in tokens[4:]]
        if len(values) < expected:
            raise ValueError("invalid P3 PPM: truncated raster")
        return width, height, bytes((value * 255) // maxval for value in values[:expected])

    # For P6, find the byte offset after the fourth token and trailing whitespace.
    pos = 0
    seen = 0
    in_comment = False
    while pos < len(raw) and seen < 4:
        byte = raw[pos]
        if in_comment:
            if byte in b"\r\n":
                in_comment = False
            pos += 1
            continue
        if byte == ord("#"):
            in_comment = True
            pos += 1
            continue
        if byte in b" \t\r\n":
            pos += 1
            continue
        while pos < len(raw) and raw[pos] not in b" \t\r\n":
            pos += 1
        seen += 1
    while pos < len(raw) and raw[pos] in b" \t\r\n":
        pos += 1
    raster = raw[pos:pos + expected]
    if len(raster) < expected:
        raise ValueError("invalid P6 PPM: truncated raster")
    if maxval == 255:
        return width, height, raster
    return width, height, bytes((value * 255) // maxval for value in raster)


def _pixel(rgb: bytes, width: int, x: int, y: int) -> RGB:
    offset = (y * width + x) * 3
    return rgb[offset], rgb[offset + 1], rgb[offset + 2]


def _luma(px: RGB) -> int:
    return (px[0] * 299 + px[1] * 587 + px[2] * 114) // 1000


@dataclass(frozen=True)
class QuadrantEvidence:
    name: str
    non_black_ratio: float
    mean_luma: float
    active: bool


@dataclass(frozen=True)
class VisualAnalysis:
    width: int
    height: int
    split_midline_score: float
    split_midline_detected: bool
    quadrants: List[QuadrantEvidence]

    def active_quadrants(self) -> List[str]:
        return [q.name for q in self.quadrants if q.active]

    def to_dict(self) -> dict:
        return {
            "width": self.width,
            "height": self.height,
            "split_midline_score": round(self.split_midline_score, 4),
            "split_midline_detected": self.split_midline_detected,
            "active_quadrants": self.active_quadrants(),
            "quadrants": [q.__dict__ for q in self.quadrants],
        }


@dataclass(frozen=True)
class PixelDiff:
    comparable: bool
    mean_abs_diff: float
    changed_pixel_ratio: float
    reason: str = ""

    def to_dict(self) -> dict:
        return {
            "comparable": self.comparable,
            "mean_abs_diff": round(self.mean_abs_diff, 4),
            "changed_pixel_ratio": round(self.changed_pixel_ratio, 4),
            "reason": self.reason,
        }


def analyze_rgb(width: int, height: int, rgb: bytes, *, min_active_luma: int = 8) -> VisualAnalysis:
    if len(rgb) != width * height * 3:
        raise ValueError("RGB raster size does not match dimensions")

    mid_x = width // 2
    sample_columns = [x for x in (mid_x - 1, mid_x, mid_x + 1) if 0 <= x < width]
    if not sample_columns:
        raise ValueError("image too narrow for midline analysis")

    # A split line is usually a dark or high-contrast vertical seam near center.
    seam_votes = 0
    for y in range(height):
        lumas = [_luma(_pixel(rgb, width, x, y)) for x in sample_columns]
        center_dark = min(lumas) <= 48
        left_x = max(0, mid_x - 2)
        right_x = min(width - 1, mid_x + 2)
        contrast = abs(_luma(_pixel(rgb, width, left_x, y)) - _luma(_pixel(rgb, width, right_x, y))) >= 24
        if center_dark or contrast:
            seam_votes += 1
    split_score = seam_votes / height

    quadrants = []
    boxes = {
        "top_left": (0, 0, mid_x, height // 2),
        "top_right": (mid_x, 0, width, height // 2),
        "bottom_left": (0, height // 2, mid_x, height),
        "bottom_right": (mid_x, height // 2, width, height),
    }
    for name, (x0, y0, x1, y1) in boxes.items():
        total = max(1, (x1 - x0) * (y1 - y0))
        non_black = 0
        luma_sum = 0
        for y in range(y0, y1):
            for x in range(x0, x1):
                luma = _luma(_pixel(rgb, width, x, y))
                luma_sum += luma
                if luma > min_active_luma:
                    non_black += 1
        ratio = non_black / total
        mean = luma_sum / total
        quadrants.append(QuadrantEvidence(name, ratio, mean, ratio >= 0.01 or mean >= min_active_luma))

    return VisualAnalysis(
        width=width,
        height=height,
        split_midline_score=split_score,
        split_midline_detected=split_score >= 0.50,
        quadrants=quadrants,
    )


def analyze_ppm(path: str | Path) -> VisualAnalysis:
    width, height, rgb = read_ppm(path)
    return analyze_rgb(width, height, rgb)


def compare_ppm(current: str | Path, baseline: str | Path, *, changed_threshold: int = 12) -> PixelDiff:
    cw, ch, crgb = read_ppm(current)
    bw, bh, brgb = read_ppm(baseline)
    if (cw, ch) != (bw, bh):
        return PixelDiff(False, 0.0, 1.0, f"dimension mismatch current={cw}x{ch} baseline={bw}x{bh}")
    total_abs = 0
    changed = 0
    pixels = cw * ch
    for idx in range(0, len(crgb), 3):
        diff = abs(crgb[idx] - brgb[idx]) + abs(crgb[idx + 1] - brgb[idx + 1]) + abs(crgb[idx + 2] - brgb[idx + 2])
        total_abs += diff / 3
        if diff / 3 > changed_threshold:
            changed += 1
    return PixelDiff(True, total_abs / pixels, changed / pixels)


def keyword_hits(text: str, keywords: Sequence[str]) -> List[str]:
    lowered = text.lower()
    return [word for word in keywords if word.lower() in lowered]


def evaluate_konsole_split(
    screenshot: str | Path,
    baseline: str | Path,
    evidence_text: str,
    keywords: Sequence[str],
    *,
    max_changed_pixel_ratio: float = 0.35,
) -> dict:
    visual = analyze_ppm(screenshot)
    diff = compare_ppm(screenshot, baseline)
    hits = keyword_hits(evidence_text, keywords)
    active_quadrants = visual.active_quadrants()
    pixel_ok = diff.comparable and diff.changed_pixel_ratio <= max_changed_pixel_ratio
    result_ok = bool(
        visual.split_midline_detected
        and pixel_ok
        and hits
        and active_quadrants
    )
    return {
        "schema": "acos.ws0.4.konsole_split.visual.v1",
        "ok": result_ok,
        "split_midline_detected": visual.split_midline_detected,
        "pixel_diff_ok": pixel_ok,
        "keyword_hits": hits,
        "active_quadrants": active_quadrants,
        "visual": visual.to_dict(),
        "pixel_diff": diff.to_dict(),
    }


def self_test() -> int:
    fixture = Path(__file__).resolve().parent.parent / "baselines" / "konsole_split_idle.ppm"
    analysis = analyze_ppm(fixture)
    assert analysis.width == 4
    assert analysis.height == 4
    assert analysis.split_midline_detected
    assert "top_left" in analysis.active_quadrants()
    diff = compare_ppm(fixture, fixture)
    assert diff.comparable
    assert diff.changed_pixel_ratio == 0.0
    report = evaluate_konsole_split(fixture, fixture, "root ACOS ion", ["acos", "missing"])
    assert report["ok"] is True
    assert report["keyword_hits"] == ["acos"]
    print("self-test ok")
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Analyze ACOS VGA PPM visual baselines")
    parser.add_argument("screenshot", nargs="?")
    parser.add_argument("--baseline")
    parser.add_argument("--evidence-text", default="")
    parser.add_argument("--keyword", action="append", default=[])
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    if not args.screenshot or not args.baseline:
        parser.error("screenshot and --baseline are required unless --self-test is used")
    report = evaluate_konsole_split(args.screenshot, args.baseline, args.evidence_text, args.keyword)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
