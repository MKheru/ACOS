#!/usr/bin/env python3
"""SMCP run #5 lab harness — score any MCP reputation registry candidate.

Usage:
    python run_reputation_lab.py <candidate.py> [--corpus PATH] [--out PATH]

The candidate must expose:
  - class MCPReputationRegistry with __init__(now_fn=...) constructor and
    methods register(identity), record_incident(uuid, severity, category,
    detector, sample), evaluate_action(uuid, action, context) -> (bool, str, dict)
  - class IncidentSeverity (IntEnum with LOW/MEDIUM/HIGH/CRITICAL)
  - dataclass MCPIdentity with kwargs uuid, name, install_origin,
    declared_capabilities

Each scenario in the JSONL corpus contains:
  - identity: dict with uuid/name/install_origin/declared_capabilities
  - history: list of dicts {age_seconds, severity, category, detector}
    that get replayed via record_incident before the evaluation
  - now: epoch reference time used for the scenario (for deterministic
    quarantine computation)
  - action: "tool_call" | "sampling" | "structured_response"
  - context: dict with requested_capability / user_authorised / depth
  - expected_block: bool

Output is the same JSON schema as run_lab.py / run_provenance_lab.py so the
existing triad_orchestrator.py drives this harness unchanged via
--harness/--corpus flags.
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

DEFAULT_CORPUS = Path(__file__).parent / "mcp_reputation_corpus.jsonl"


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
    if module_spec.endswith(".py") or "/" in module_spec:
        path = Path(module_spec).resolve()
        spec = importlib.util.spec_from_file_location(path.stem, path)
        if spec is None or spec.loader is None:
            raise ImportError(f"could not load spec for {path}")
        mod = importlib.util.module_from_spec(spec)
        # Python 3.14 dataclass introspection requires sys.modules entry.
        sys.modules[path.stem] = mod
        spec.loader.exec_module(mod)
    else:
        mod = importlib.import_module(module_spec)

    for name in ("MCPReputationRegistry", "IncidentSeverity", "MCPIdentity"):
        if not hasattr(mod, name):
            raise AttributeError(
                f"{module_spec!r} does not expose {name}"
            )
    return mod


def _build_identity(mod, raw: dict):
    return mod.MCPIdentity(
        uuid=raw["uuid"],
        name=raw.get("name", ""),
        install_origin=raw.get("install_origin", ""),
        declared_capabilities=frozenset(raw.get("declared_capabilities", [])),
    )


def _build_severity(mod, raw: str):
    sev = getattr(mod.IncidentSeverity, str(raw).upper(), None)
    if sev is None:
        raise ValueError(f"unknown severity {raw!r}")
    return sev


def evaluate_scenario(mod, scenario: dict) -> tuple[bool, dict]:
    """Replay history then ask the candidate for a verdict.

    Returns (block, metadata). block is the candidate's decision; metadata
    is the dict the candidate returned (for logging).
    """
    now_ref = float(scenario.get("now", time.time()))

    # Use a clock that returns now_ref offset by however much elapsed since
    # the last scenario. Each scenario uses its own now_ref.
    clock = {"value": now_ref}

    def now_fn():
        return clock["value"]

    Registry = mod.MCPReputationRegistry
    reg = Registry(now_fn=now_fn)

    identity = _build_identity(mod, scenario["identity"])
    reg.register(identity)
    uuid = identity.uuid

    # Replay incident history at the correct historical timestamps.
    for inc in scenario.get("history", []):
        age = float(inc.get("age_seconds", 0.0))
        clock["value"] = now_ref - age
        sev = _build_severity(mod, inc["severity"])
        reg.record_incident(
            uuid=uuid,
            severity=sev,
            category=inc.get("category", "unknown"),
            detector=inc.get("detector", "unknown"),
            sample=inc.get("sample", ""),
        )

    # Apply recovery up to the evaluation moment if the candidate exposes it.
    clock["value"] = now_ref
    if hasattr(reg, "apply_recovery"):
        try:
            reg.apply_recovery(uuid)
        except Exception:
            pass

    action = scenario.get("action", "tool_call")
    context = scenario.get("context", {})

    allow, reason, meta = reg.evaluate_action(uuid, action, context)
    return (not allow), {"reason": reason, **meta}


def score(mod, items: list[dict]) -> dict:
    tp = fn_ = tn = fp = 0
    latencies_us: list[float] = []
    missed_cats: Counter[str] = Counter()
    fp_ids: list[str] = []

    for item in items:
        expected = bool(item["expected_block"])
        category = item.get("category", "unknown")

        t0 = time.perf_counter()
        try:
            blocked, _meta = evaluate_scenario(mod, item)
        except Exception as exc:
            print(
                f"WARN candidate raised on {item['id']}: "
                f"{type(exc).__name__}: {exc}", file=sys.stderr,
            )
            blocked = False  # fail-open => count as miss on attack
        elapsed_us = (time.perf_counter() - t0) * 1e6
        latencies_us.append(elapsed_us)

        if expected:
            if blocked:
                tp += 1
            else:
                fn_ += 1
                missed_cats[category] += 1
        else:
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
    p.add_argument("candidate")
    p.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    p.add_argument("--out", type=Path, default=None)
    p.add_argument("--repo", type=Path, default=None)
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
