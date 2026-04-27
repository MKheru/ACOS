#!/usr/bin/env python3
"""SMCP lab harness — score any candidate MCP output sanitiser.

Usage:
    python run_lab.py <module_path> [--corpus PATH] [--out PATH]

The candidate must expose a ``sanitize_mcp_output(text, server_name="x") -> str``
function. Detection is judged by output != input (the sanitiser modified
the text — wrapping, redacting, etc.). Benign passthroughs are judged by
output == input.

Outputs a JSON metrics block to stdout (and optionally to --out path):

    {
      "candidate": "agent.mcp_sanitizer",
      "corpus_size": 70,
      "attacks": 50, "benign": 20,
      "tp": 38, "fn": 12, "tn": 18, "fp": 2,
      "detection_rate": 0.76,    # tp / (tp + fn)
      "false_positive_rate": 0.10,
      "f1": 0.84,
      "latency_p50_us": 18.3,
      "latency_p99_us": 95.1,
      "missed_categories": {"multilingual_fr": 1, "encoded_b64": 1, ...},
      "false_positive_ids": ["ben-015", "ben-016"]
    }

ARC consumes this JSON to compare candidate variants. The optimisation
target is to maximise ``detection_rate`` while keeping
``false_positive_rate`` low (suggested target: detection >= 0.95,
fpr <= 0.05).
"""

from __future__ import annotations

import argparse
import importlib
import importlib.util
import json
import statistics
import sys
import time
from collections import Counter
from pathlib import Path
from typing import Any, Callable

DEFAULT_CORPUS = Path(__file__).parent / "attack_corpus.jsonl"


def load_corpus(path: Path) -> list[dict]:
    items = []
    with path.open(encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                items.append(json.loads(line))
            except json.JSONDecodeError as exc:
                raise ValueError(f"corpus line {n} malformed: {exc}") from exc
    return items


def load_candidate(module_spec: str) -> Callable[[str, str], str]:
    """Load a candidate sanitiser by dotted module path or file path.

    Accepts either:
      - 'agent.mcp_sanitizer' (importable module on sys.path)
      - '/abs/path/to/candidate.py' (loaded ad hoc)
    The module must expose ``sanitize_mcp_output``.
    """
    if module_spec.endswith(".py") or "/" in module_spec:
        path = Path(module_spec).resolve()
        spec = importlib.util.spec_from_file_location(path.stem, path)
        if spec is None or spec.loader is None:
            raise ImportError(f"could not load spec for {path}")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
    else:
        mod = importlib.import_module(module_spec)

    fn = getattr(mod, "sanitize_mcp_output", None)
    if fn is None:
        raise AttributeError(
            f"{module_spec!r} does not expose sanitize_mcp_output"
        )
    return fn


def score(candidate: Callable[[str, str], str], items: list[dict]) -> dict:
    tp = fn_ = tn = fp = 0
    latencies_us: list[float] = []
    missed_cats: Counter[str] = Counter()
    fp_ids: list[str] = []

    for item in items:
        text = item["text"]
        label = item["label"]
        category = item.get("category", "unknown")

        t0 = time.perf_counter()
        try:
            out = candidate(text, server_name="lab-harness")
        except Exception as exc:
            print(
                f"WARN candidate raised on {item['id']}: {type(exc).__name__}: {exc}",
                file=sys.stderr,
            )
            out = text  # treat as passthrough on crash
        elapsed_us = (time.perf_counter() - t0) * 1e6
        latencies_us.append(elapsed_us)

        detected = (out != text)
        if label == "attack":
            if detected:
                tp += 1
            else:
                fn_ += 1
                missed_cats[category] += 1
        else:  # benign
            if detected:
                fp += 1
                fp_ids.append(item["id"])
            else:
                tn += 1

    n_attacks = tp + fn_
    n_benign = tn + fp
    detection_rate = tp / n_attacks if n_attacks else 0.0
    fpr = fp / n_benign if n_benign else 0.0
    precision = tp / (tp + fp) if (tp + fp) else 0.0
    recall = detection_rate
    f1 = (2 * precision * recall / (precision + recall)) if (precision + recall) else 0.0

    return {
        "corpus_size": len(items),
        "attacks": n_attacks,
        "benign": n_benign,
        "tp": tp,
        "fn": fn_,
        "tn": tn,
        "fp": fp,
        "detection_rate": round(detection_rate, 4),
        "false_positive_rate": round(fpr, 4),
        "precision": round(precision, 4),
        "recall": round(recall, 4),
        "f1": round(f1, 4),
        "latency_p50_us": round(statistics.median(latencies_us), 2) if latencies_us else 0.0,
        "latency_p99_us": round(
            statistics.quantiles(latencies_us, n=100)[98], 2
        ) if len(latencies_us) >= 100 else round(max(latencies_us), 2),
        "missed_categories": dict(missed_cats),
        "false_positive_ids": fp_ids,
    }


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("candidate", help="dotted module path or .py file")
    p.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    p.add_argument("--out", type=Path, default=None,
                   help="if set, also write the JSON metrics here")
    p.add_argument("--repo", type=Path, default=None,
                   help="repo root to add to sys.path (defaults to ACOS-HERMES sibling)")
    args = p.parse_args()

    # Ensure ACOS-HERMES is on sys.path so 'agent.mcp_sanitizer' resolves.
    repo = args.repo or (Path(__file__).resolve().parents[3] / "ACOS-HERMES")
    if repo.exists() and str(repo) not in sys.path:
        sys.path.insert(0, str(repo))

    if not args.corpus.exists():
        print(f"ERROR corpus not found: {args.corpus}", file=sys.stderr)
        return 2

    items = load_corpus(args.corpus)
    candidate_fn = load_candidate(args.candidate)
    metrics = score(candidate_fn, items)
    metrics["candidate"] = args.candidate
    metrics["corpus_path"] = str(args.corpus)

    output = json.dumps(metrics, indent=2, ensure_ascii=False)
    print(output)
    if args.out:
        args.out.write_text(output, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
