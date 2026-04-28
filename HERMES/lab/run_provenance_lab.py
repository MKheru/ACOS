#!/usr/bin/env python3
"""SMCP run #4 lab harness — score any candidate provenance gate.

Usage:
    python run_provenance_lab.py <candidate.py> [--corpus PATH] [--out PATH]

The candidate must expose ``should_block_llm_call(trace, intent)`` returning
``(block: bool, reason: str)`` and the helpers ``Tag``, ``Message``,
``ProvenanceSource`` (with the same enum values as the baseline). The
harness reconstructs the Tag/Message graph from the JSONL trace
(propagating `parent_idx` to wire the parent chain) and asks the
candidate for a verdict on each scenario. The verdict is compared to
``expected_block`` and the standard run_lab.py JSON metric block is
emitted on stdout, so the existing triad_orchestrator.py works
unchanged with --harness pointing at this script.

Output schema is identical to run_lab.py:
  detection_rate / false_positive_rate / precision / recall / f1 /
  latency_p50_us / latency_p99_us / missed_categories / false_positive_ids

Mapping:
  attack scenario + correctly blocked = TP
  attack scenario + not blocked       = FN (missed)
  benign scenario + correctly allowed = TN
  benign scenario + incorrectly blocked = FP

Composite (same formula as the regex run): higher is better.
  score = detection_rate - 2*false_positive_rate - latency_penalty
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
from typing import Any

DEFAULT_CORPUS = Path(__file__).parent / "provenance_corpus.jsonl"


def load_corpus(path: Path) -> list[dict]:
    items: list[dict] = []
    with path.open(encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                items.append(json.loads(line))
            except json.JSONDecodeError as exc:
                raise ValueError(
                    f"corpus line {n} malformed: {exc}"
                ) from exc
    return items


def load_candidate(module_spec: str):
    """Load a candidate provenance module from a .py path or dotted name."""
    if module_spec.endswith(".py") or "/" in module_spec:
        path = Path(module_spec).resolve()
        spec = importlib.util.spec_from_file_location(path.stem, path)
        if spec is None or spec.loader is None:
            raise ImportError(f"could not load spec for {path}")
        mod = importlib.util.module_from_spec(spec)
        # Register before exec_module — required by dataclass introspection
        # in Python 3.14 (dataclasses.py:_is_type reads sys.modules).
        sys.modules[path.stem] = mod
        spec.loader.exec_module(mod)
    else:
        mod = importlib.import_module(module_spec)

    for name in ("should_block_llm_call", "Tag", "Message", "ProvenanceSource"):
        if not hasattr(mod, name):
            raise AttributeError(
                f"{module_spec!r} does not expose {name}"
            )
    return mod


def build_trace(mod, raw_trace: list[dict]) -> list[Any]:
    """Reconstruct the candidate-typed Message list from the JSONL trace.

    `parent_idx` (if present) wires the parent Tag of one message to the
    Tag of an earlier message in the same trace.
    """
    Tag = mod.Tag
    Message = mod.Message
    PS = mod.ProvenanceSource

    tags: list[Any] = []
    messages: list[Any] = []
    for i, raw in enumerate(raw_trace):
        try:
            source = PS(raw["source"])
        except ValueError:
            source = PS.UNKNOWN
        parent = None
        if "parent_idx" in raw and raw["parent_idx"] is not None:
            j = int(raw["parent_idx"])
            if 0 <= j < len(tags):
                parent = tags[j]
        tag = Tag(
            source=source,
            server_name=raw.get("server"),
            tool_name=raw.get("tool"),
            parent=parent,
        )
        tags.append(tag)
        messages.append(Message(role=raw["role"], content=raw["content"], tag=tag))
    return messages


def score(mod, items: list[dict]) -> dict:
    tp = fn_ = tn = fp = 0
    latencies_us: list[float] = []
    missed_cats: Counter[str] = Counter()
    fp_ids: list[str] = []

    for item in items:
        trace = build_trace(mod, item["trace"])
        intent = item.get("intent", "sampling")
        expected = bool(item["expected_block"])
        category = item.get("category", "unknown")

        t0 = time.perf_counter()
        try:
            blocked, _reason = mod.should_block_llm_call(trace, intent=intent)
        except Exception as exc:
            print(
                f"WARN candidate raised on {item['id']}: "
                f"{type(exc).__name__}: {exc}", file=sys.stderr,
            )
            blocked = False  # fail-open => count as miss on attack
        elapsed_us = (time.perf_counter() - t0) * 1e6
        latencies_us.append(elapsed_us)

        if expected:  # attack scenario, block expected
            if blocked:
                tp += 1
            else:
                fn_ += 1
                missed_cats[category] += 1
        else:  # benign scenario, allow expected
            if blocked:
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
    f1 = (2 * precision * recall / (precision + recall)) \
        if (precision + recall) else 0.0

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
        "latency_p50_us": (
            round(statistics.median(latencies_us), 2)
            if latencies_us else 0.0
        ),
        "latency_p99_us": (
            round(statistics.quantiles(latencies_us, n=100)[98], 2)
            if len(latencies_us) >= 100
            else round(max(latencies_us), 2)
        ),
        "missed_categories": dict(missed_cats),
        "false_positive_ids": fp_ids,
    }


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("candidate", help="dotted module path or .py file")
    p.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    p.add_argument("--out", type=Path, default=None)
    p.add_argument("--repo", type=Path, default=None,
                   help="optional repo root to add to sys.path")
    args = p.parse_args()

    if args.repo and args.repo.exists() and str(args.repo) not in sys.path:
        sys.path.insert(0, str(args.repo))

    if not args.corpus.exists():
        print(f"ERROR corpus not found: {args.corpus}", file=sys.stderr)
        return 2

    items = load_corpus(args.corpus)
    mod = load_candidate(args.candidate)
    metrics = score(mod, items)
    metrics["candidate"] = args.candidate
    metrics["corpus_path"] = str(args.corpus)

    text = json.dumps(metrics, indent=2, ensure_ascii=False)
    print(text)
    if args.out:
        args.out.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
