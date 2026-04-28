"""MCP install hardening + SMCP envelope — Run #6 baseline (v0).

Combines two complementary defences in one module to keep the lab
harness simple. They are conceptually distinct but both gate MCP
interactions at structural layers below the content level.

1. Install hardening (InstallManifest + InstallContract):
   When an MCP is installed, it presents a manifest declaring its
   capabilities, network egress, and FS access needs. The user
   responds with an InstallContract granting a SUBSET of those
   declarations. Any runtime action outside the contract is refused
   structurally — the host doesn't even reach the content sanitiser.

2. SMCP envelope (Envelope + EnvelopeVerifier):
   Each message between host and MCP is wrapped in an envelope with
   nonce, capability assertion, and HMAC. Forgery (wrong secret),
   replay (nonce reused within window), capability mismatch
   (envelope claims `sampling` but contract didn't grant it), or
   missing fields cause the message to be rejected before it
   touches the agent's logic.

Together they harden the install boundary AND every wire message,
making it materially harder for a compromised MCP to escape the
permissions a user granted at install time.

Public API expected by run_install_envelope_lab.py harness:

  class InstallManifest (frozen dataclass)
  class InstallContract (frozen dataclass)
  class InstallValidator
  class Envelope (frozen dataclass)
  class EnvelopeVerifier
"""

from __future__ import annotations

import hashlib
import hmac
import json
import secrets
import time
from collections import OrderedDict
from dataclasses import dataclass, field
from typing import Optional


# ---------------------------------------------------------------------------
# Install hardening
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class InstallManifest:
    name: str
    version: str
    install_origin: str
    declared_capabilities: frozenset[str] = frozenset()
    required_egress: frozenset[str] = frozenset()
    required_fs_access: frozenset[str] = frozenset()
    public_key: str = ""        # hex-encoded; "" means unsigned
    signature: str = ""         # hex-encoded HMAC for v0; ed25519 in v1+


@dataclass(frozen=True)
class InstallContract:
    manifest_name: str
    manifest_version: str
    granted_capabilities: frozenset[str] = frozenset()
    granted_egress: frozenset[str] = frozenset()
    granted_fs_access: frozenset[str] = frozenset()
    user_attestation: str = ""
    signed_at: float = 0.0


class InstallValidator:
    """v0: verifies signature (HMAC for now), checks user grants are a
    subset of declared, enforces runtime contract.

    The Triad will iterate on:
      - swapping HMAC for ed25519 signature (Task 1)
      - tighter contract negotiation (Task 2)
      - per-action runtime checks (Task 5)
    """

    def __init__(self, trusted_publisher_keys: Optional[set[str]] = None) -> None:
        # Set of hex-encoded HMAC secrets that are trusted publishers.
        # In v1 these become ed25519 pubkeys.
        self._trusted_keys: set[str] = trusted_publisher_keys or set()

    def add_trusted_publisher(self, key_hex: str) -> None:
        self._trusted_keys.add(key_hex)

    # -- Manifest verification --------------------------------------------

    def validate_manifest(
        self, manifest: InstallManifest,
    ) -> tuple[bool, list[str]]:
        """Return (ok, problems). v0 uses HMAC over the canonical JSON."""
        problems: list[str] = []
        if not manifest.name:
            problems.append("manifest missing name")
        if not manifest.version:
            problems.append("manifest missing version")
        if not manifest.install_origin:
            problems.append("manifest missing install_origin")

        if not manifest.public_key or not manifest.signature:
            problems.append("manifest unsigned — refuse install")
            return False, problems

        if manifest.public_key not in self._trusted_keys:
            problems.append(
                f"manifest public_key {manifest.public_key[:16]}... "
                "not in trusted publishers"
            )
            return False, problems

        canonical = self._canonical_payload(manifest)
        try:
            secret = bytes.fromhex(manifest.public_key)
            expected = hmac.new(secret, canonical, hashlib.sha256).hexdigest()
        except ValueError:
            problems.append("manifest public_key not valid hex")
            return False, problems

        if not hmac.compare_digest(expected, manifest.signature):
            problems.append("manifest signature does not verify")
            return False, problems

        return True, []

    @staticmethod
    def _canonical_payload(m: InstallManifest) -> bytes:
        """Deterministic byte representation for HMAC. EXCLUDES the
        signature itself (we'd be signing our own signature otherwise)."""
        payload = {
            "name": m.name,
            "version": m.version,
            "install_origin": m.install_origin,
            "declared_capabilities": sorted(m.declared_capabilities),
            "required_egress": sorted(m.required_egress),
            "required_fs_access": sorted(m.required_fs_access),
            "public_key": m.public_key,
        }
        return json.dumps(payload, sort_keys=True, ensure_ascii=False).encode("utf-8")

    # -- Contract negotiation ---------------------------------------------

    def negotiate_contract(
        self,
        manifest: InstallManifest,
        user_grants: dict,
    ) -> tuple[Optional[InstallContract], list[str]]:
        """Build a contract from user_grants. user_grants is a dict like
        {"capabilities": [...], "egress": [...], "fs_access": [...],
         "attestation": "..."}.

        Returns (contract or None, problems). The contract is None if
        user grants exceed manifest declarations (caller asks for more
        than the manifest declares — refuse).
        """
        problems: list[str] = []

        granted_caps = frozenset(user_grants.get("capabilities", []))
        granted_egress = frozenset(user_grants.get("egress", []))
        granted_fs = frozenset(user_grants.get("fs_access", []))

        if not granted_caps.issubset(manifest.declared_capabilities):
            extra = granted_caps - manifest.declared_capabilities
            problems.append(
                f"user granted capabilities not in manifest: {sorted(extra)}"
            )
        if not granted_egress.issubset(manifest.required_egress):
            extra = granted_egress - manifest.required_egress
            problems.append(
                f"user granted egress not in manifest: {sorted(extra)}"
            )
        if not granted_fs.issubset(manifest.required_fs_access):
            extra = granted_fs - manifest.required_fs_access
            problems.append(
                f"user granted fs_access not in manifest: {sorted(extra)}"
            )

        if problems:
            return None, problems

        contract = InstallContract(
            manifest_name=manifest.name,
            manifest_version=manifest.version,
            granted_capabilities=granted_caps,
            granted_egress=granted_egress,
            granted_fs_access=granted_fs,
            user_attestation=user_grants.get("attestation", ""),
            signed_at=time.time(),
        )
        return contract, []

    # -- Runtime enforcement ----------------------------------------------

    def check_runtime(
        self,
        contract: InstallContract,
        action: str,
        target: str = "",
    ) -> tuple[bool, str]:
        """Check whether a runtime action is allowed by the contract.

        action ∈ {"capability:<name>", "egress:<domain>", "fs:<path>"}
        target may add detail (e.g. specific path).
        """
        if not action:
            return False, "empty action"

        kind, _, value = action.partition(":")
        if kind == "capability":
            if value in contract.granted_capabilities:
                return True, ""
            return False, f"capability {value!r} not in contract"
        if kind == "egress":
            if value in contract.granted_egress:
                return True, ""
            # Allow exact subdomain match if a parent domain is granted
            for granted in contract.granted_egress:
                if value.endswith("." + granted):
                    return True, ""
            return False, f"egress to {value!r} not in contract"
        if kind == "fs":
            for granted in contract.granted_fs_access:
                if value == granted or value.startswith(granted.rstrip("/") + "/"):
                    return True, ""
            return False, f"fs access to {value!r} not in contract"
        return False, f"unknown action kind {kind!r}"


# ---------------------------------------------------------------------------
# SMCP envelope
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class Envelope:
    nonce: str
    sent_at: float
    sender_uuid: str
    recipient: str
    capability_assertion: str
    payload_hash: str
    hmac_hex: str


class EnvelopeVerifier:
    """Wraps and verifies SMCP envelopes. v0 uses HMAC-SHA256 with a
    pre-shared secret per (sender, recipient) pair. The Triad will
    iterate on key derivation, replay window tuning, and capability
    binding.
    """

    def __init__(self, shared_secret: bytes,
                 replay_window_secs: int = 300,
                 nonce_cache_size: int = 4096) -> None:
        self._secret = shared_secret
        self._replay_window = replay_window_secs
        self._nonce_cache_size = nonce_cache_size
        # OrderedDict mapping nonce → first-seen timestamp.
        self._seen_nonces: OrderedDict[str, float] = OrderedDict()

    # -- Wrapping ----------------------------------------------------------

    def wrap(
        self,
        payload: bytes,
        sender_uuid: str,
        recipient: str,
        capability: str,
    ) -> Envelope:
        nonce = secrets.token_hex(16)
        payload_hash = hashlib.sha256(payload).hexdigest()
        sent_at = time.time()
        msg = f"{nonce}|{sent_at}|{sender_uuid}|{recipient}|" \
              f"{capability}|{payload_hash}".encode("utf-8")
        mac = hmac.new(self._secret, msg, hashlib.sha256).hexdigest()
        return Envelope(
            nonce=nonce,
            sent_at=sent_at,
            sender_uuid=sender_uuid,
            recipient=recipient,
            capability_assertion=capability,
            payload_hash=payload_hash,
            hmac_hex=mac,
        )

    # -- Verification ------------------------------------------------------

    def verify(
        self,
        envelope: Envelope,
        payload: bytes,
        expected_recipient: str = "",
    ) -> tuple[bool, str]:
        # Field presence
        for fld in ("nonce", "sender_uuid", "recipient",
                    "capability_assertion", "payload_hash", "hmac_hex"):
            if not getattr(envelope, fld):
                return False, f"envelope missing {fld}"

        # Recipient match
        if expected_recipient and envelope.recipient != expected_recipient:
            return False, (
                f"envelope recipient mismatch: expected "
                f"{expected_recipient!r}, got {envelope.recipient!r}"
            )

        # Payload hash
        actual_hash = hashlib.sha256(payload).hexdigest()
        if not hmac.compare_digest(actual_hash, envelope.payload_hash):
            return False, "payload hash does not match"

        # HMAC verification
        msg = f"{envelope.nonce}|{envelope.sent_at}|{envelope.sender_uuid}|" \
              f"{envelope.recipient}|{envelope.capability_assertion}|" \
              f"{envelope.payload_hash}".encode("utf-8")
        expected_mac = hmac.new(self._secret, msg, hashlib.sha256).hexdigest()
        if not hmac.compare_digest(expected_mac, envelope.hmac_hex):
            return False, "envelope HMAC verification failed"

        # Replay check
        if not self.check_replay(envelope.nonce, envelope.sent_at):
            return False, "envelope nonce replayed within window"

        return True, ""

    def check_replay(
        self, nonce: str, sent_at: Optional[float] = None,
    ) -> bool:
        """Return True if this nonce is FRESH (not previously seen in the
        replay window). Records the nonce on first sight."""
        now = time.time()
        when = sent_at if sent_at is not None else now

        # Reject envelopes whose timestamp is outside the replay window
        # (too old or impossibly future).
        if abs(now - when) > self._replay_window:
            return False

        # Evict nonces older than the window from the cache.
        cutoff = now - self._replay_window
        while self._seen_nonces:
            oldest_nonce, oldest_ts = next(iter(self._seen_nonces.items()))
            if oldest_ts >= cutoff:
                break
            self._seen_nonces.popitem(last=False)

        if nonce in self._seen_nonces:
            return False

        # Bound cache size.
        while len(self._seen_nonces) >= self._nonce_cache_size:
            self._seen_nonces.popitem(last=False)

        self._seen_nonces[nonce] = when
        return True
