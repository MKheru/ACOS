#!/usr/bin/env python3
"""SMCP run #6 lab harness — score MCP install + envelope candidates.

The corpus has heterogeneous scenarios, each with a `kind` tag that
selects the API to exercise:

  - validate_manifest          → InstallValidator.validate_manifest
  - negotiate_contract         → InstallValidator.negotiate_contract
  - check_runtime              → InstallValidator.check_runtime
  - verify_envelope            → EnvelopeVerifier.verify (raw envelope)
  - verify_envelope_with_wrap  → wrap then verify (with optional payload mismatch)
  - verify_envelope_replay     → wrap once, send N times, expect rejection on 2nd+
  - verify_envelope_two_distinct → wrap twice, send both, expect both pass

Auto-magic markers in the corpus:
  - signature == "AUTOSIGN" → harness recomputes a valid HMAC over the
    canonical manifest payload using the trusted key (lets the corpus
    encode "valid signature" without baking real hex)
  - payload_hash == "AUTOHASH" → harness computes sha256(payload)
  - hmac_hex == "AUTOMAC" → harness computes a valid HMAC over the
    envelope fields using the shared secret

These markers let scenarios be readable JSON without precomputed hex.
A candidate that drifts the canonicalisation will fail validate
because the AUTOSIGN/AUTOMAC values stop matching.

Output schema is identical to run_lab.py / run_provenance_lab.py /
run_reputation_lab.py so triad_orchestrator.py can drive this harness
unchanged via --harness/--corpus.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac as _hmac
import importlib
import importlib.util
import json
import statistics
import sys
import time
from collections import Counter
from pathlib import Path
from typing import Any

DEFAULT_CORPUS = (
    Path(__file__).parent / "mcp_install_envelope_corpus.jsonl"
)


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
        sys.modules[path.stem] = mod
        spec.loader.exec_module(mod)
    else:
        mod = importlib.import_module(module_spec)
    for name in ("InstallManifest", "InstallContract", "InstallValidator",
                 "Envelope", "EnvelopeVerifier"):
        if not hasattr(mod, name):
            raise AttributeError(f"{module_spec!r} does not expose {name}")
    return mod


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _build_manifest(mod, raw: dict, trusted_keys: list[str]):
    """Builds an InstallManifest. If signature == 'AUTOSIGN', recomputes
    a valid HMAC using the manifest's public_key (which must be in
    trusted_keys for the validate_manifest scenario to expect ok)."""
    M = mod.InstallManifest
    sig = raw.get("signature", "")
    pub = raw.get("public_key", "")
    if sig == "AUTOSIGN" and pub:
        canonical = mod.InstallValidator._canonical_payload(M(
            name=raw["name"],
            version=raw["version"],
            install_origin=raw["install_origin"],
            declared_capabilities=frozenset(raw.get("declared_capabilities", [])),
            required_egress=frozenset(raw.get("required_egress", [])),
            required_fs_access=frozenset(raw.get("required_fs_access", [])),
            public_key=pub,
            signature="",
        ))
        try:
            secret = bytes.fromhex(pub)
        except ValueError:
            sig = ""
        else:
            sig = _hmac.new(secret, canonical, hashlib.sha256).hexdigest()
    return M(
        name=raw["name"],
        version=raw["version"],
        install_origin=raw["install_origin"],
        declared_capabilities=frozenset(raw.get("declared_capabilities", [])),
        required_egress=frozenset(raw.get("required_egress", [])),
        required_fs_access=frozenset(raw.get("required_fs_access", [])),
        public_key=pub,
        signature=sig,
    )


def _build_contract(mod, raw: dict):
    C = mod.InstallContract
    return C(
        manifest_name=raw["manifest_name"],
        manifest_version=raw["manifest_version"],
        granted_capabilities=frozenset(raw.get("granted_capabilities", [])),
        granted_egress=frozenset(raw.get("granted_egress", [])),
        granted_fs_access=frozenset(raw.get("granted_fs_access", [])),
        user_attestation=raw.get("user_attestation", ""),
        signed_at=float(raw.get("signed_at", 0.0)),
    )


def _build_envelope(mod, raw: dict, payload: bytes, secret: bytes):
    """Builds an Envelope. Resolves AUTOHASH / AUTOMAC if present."""
    E = mod.Envelope
    payload_hash = raw.get("payload_hash", "")
    if payload_hash == "AUTOHASH":
        payload_hash = hashlib.sha256(payload).hexdigest()
    hmac_hex = raw.get("hmac_hex", "")
    if hmac_hex == "AUTOMAC":
        msg = (
            f"{raw['nonce']}|{raw['sent_at']}|{raw['sender_uuid']}|"
            f"{raw['recipient']}|{raw['capability_assertion']}|"
            f"{payload_hash}"
        ).encode("utf-8")
        hmac_hex = _hmac.new(secret, msg, hashlib.sha256).hexdigest()
    return E(
        nonce=raw["nonce"],
        sent_at=float(raw["sent_at"]),
        sender_uuid=raw["sender_uuid"],
        recipient=raw["recipient"],
        capability_assertion=raw["capability_assertion"],
        payload_hash=payload_hash,
        hmac_hex=hmac_hex,
    )


# ---------------------------------------------------------------------------
# Per-kind scenario evaluation
# ---------------------------------------------------------------------------

def _run_scenario(mod, scenario: dict) -> bool:
    """Returns True if the candidate's verdict is BLOCK."""
    kind = scenario["kind"]
    trusted_keys = scenario.get("trusted_keys", [])

    if kind == "validate_manifest":
        validator = mod.InstallValidator(set(trusted_keys))
        for k in trusted_keys:
            validator.add_trusted_publisher(k)
        manifest = _build_manifest(mod, scenario["manifest"], trusted_keys)
        ok, _problems = validator.validate_manifest(manifest)
        return not ok

    if kind == "negotiate_contract":
        validator = mod.InstallValidator(set(trusted_keys))
        for k in trusted_keys:
            validator.add_trusted_publisher(k)
        manifest = _build_manifest(mod, scenario["manifest"], trusted_keys)
        contract, _problems = validator.negotiate_contract(
            manifest, scenario["user_grants"]
        )
        return contract is None

    if kind == "check_runtime":
        validator = mod.InstallValidator()
        contract = _build_contract(mod, scenario["contract"])
        ok, _reason = validator.check_runtime(
            contract,
            scenario["action"],
            scenario.get("target", ""),
        )
        return not ok

    if kind == "verify_envelope":
        secret_hex = scenario.get("shared_secret_hex", "")
        secret = bytes.fromhex(secret_hex) if secret_hex else b"\x00"
        verifier = mod.EnvelopeVerifier(secret)
        payload = bytes.fromhex(scenario.get("payload_hex", ""))
        envelope = _build_envelope(mod, scenario["envelope"], payload, secret)
        ok, _reason = verifier.verify(
            envelope, payload,
            expected_recipient=scenario.get("expected_recipient", ""),
        )
        return not ok

    if kind == "verify_envelope_with_wrap":
        secret = bytes.fromhex(scenario["shared_secret_hex"])
        verifier = mod.EnvelopeVerifier(secret)
        wrap_payload = bytes.fromhex(scenario["wrap_payload_hex"])
        verify_payload = bytes.fromhex(scenario["verify_payload_hex"])
        envelope = verifier.wrap(
            wrap_payload,
            scenario["sender"],
            scenario["recipient"],
            scenario["capability"],
        )
        ok, _reason = verifier.verify(
            envelope, verify_payload,
            expected_recipient=scenario.get("expected_recipient", ""),
        )
        return not ok

    if kind == "verify_envelope_replay":
        secret = bytes.fromhex(scenario["shared_secret_hex"])
        verifier = mod.EnvelopeVerifier(secret)
        payload = bytes.fromhex(scenario["payload_hex"])
        envelope = verifier.wrap(
            payload, scenario["sender"], scenario["recipient"],
            scenario["capability"],
        )
        send_count = int(scenario.get("send_count", 2))
        any_blocked = False
        for i in range(send_count):
            ok, _reason = verifier.verify(envelope, payload)
            if not ok and i > 0:
                # Replay rejection on second send is the expected attack.
                any_blocked = True
                break
        return any_blocked

    if kind == "verify_envelope_two_distinct":
        secret = bytes.fromhex(scenario["shared_secret_hex"])
        verifier = mod.EnvelopeVerifier(secret)
        payload = bytes.fromhex(scenario["payload_hex"])
        # Two distinct envelopes — both should pass.
        e1 = verifier.wrap(payload, scenario["sender"], scenario["recipient"],
                           scenario["capability"])
        ok1, _ = verifier.verify(e1, payload)
        e2 = verifier.wrap(payload, scenario["sender"], scenario["recipient"],
                           scenario["capability"])
        ok2, _ = verifier.verify(e2, payload)
        return not (ok1 and ok2)  # blocked if either failed

    raise ValueError(f"unknown scenario kind: {kind!r}")


# ---------------------------------------------------------------------------
# Scoring
# ---------------------------------------------------------------------------

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
            blocked = _run_scenario(mod, item)
        except Exception as exc:
            print(
                f"WARN candidate raised on {item['id']}: "
                f"{type(exc).__name__}: {exc}", file=sys.stderr,
            )
            blocked = False
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
        "latency_p50_us": round(statistics.median(latencies_us), 2)
        if latencies_us else 0.0,
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
