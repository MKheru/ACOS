## SMCP Run #6 — MCP install hardening + SMCP envelope

The lab evaluates a Python module that combines two structural
defences:

### Install hardening
At install time, an MCP presents an `InstallManifest` (signed name,
version, install_origin, declared capabilities, network egress
needs, FS access needs). The user negotiates a subset via an
`InstallContract`. At runtime, every MCP action goes through
`InstallValidator.check_runtime(contract, action)` and is refused if
outside the contract.

### SMCP envelope
Above the raw MCP wire protocol, every host↔MCP message is wrapped
in an `Envelope` carrying a nonce, capability assertion, payload
hash, and HMAC. `EnvelopeVerifier.verify(envelope, payload)` rejects
forgeries (wrong HMAC), tampering (hash mismatch), replays (nonce
re-used), stale timestamps (outside the replay window), recipient
mismatches.

### Required public API

```python
@dataclass(frozen=True)
class InstallManifest:
    name: str
    version: str
    install_origin: str
    declared_capabilities: frozenset[str] = frozenset()
    required_egress: frozenset[str] = frozenset()
    required_fs_access: frozenset[str] = frozenset()
    public_key: str = ""        # hex
    signature: str = ""         # hex (HMAC v0; ed25519 in future)

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
    def __init__(self, trusted_publisher_keys: set[str] = None): ...
    def add_trusted_publisher(self, key_hex: str): ...
    def validate_manifest(self, m: InstallManifest) -> tuple[bool, list[str]]: ...
    def negotiate_contract(self, m, user_grants: dict) -> tuple[Optional[InstallContract], list[str]]: ...
    def check_runtime(self, contract, action: str, target: str = "") -> tuple[bool, str]: ...
    @staticmethod
    def _canonical_payload(m: InstallManifest) -> bytes: ...   # used by harness for AUTOSIGN

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
    def __init__(self, shared_secret: bytes,
                 replay_window_secs: int = 300,
                 nonce_cache_size: int = 4096): ...
    def wrap(self, payload: bytes, sender_uuid: str, recipient: str,
             capability: str) -> Envelope: ...
    def verify(self, envelope: Envelope, payload: bytes,
               expected_recipient: str = "") -> tuple[bool, str]: ...
    def check_replay(self, nonce: str, sent_at: Optional[float] = None) -> bool: ...
```

### Constraints

- Python stdlib only (`hashlib`, `hmac`, `secrets`, `time`,
  `collections`, `dataclasses` — no pip dependencies).
- Field names listed above must remain stable; the harness builds
  these dataclasses by keyword arguments.
- `InstallValidator._canonical_payload` is a static helper the harness
  uses to materialise valid AUTOSIGN signatures — keep it stable or
  the AUTOSIGN scenarios will fail with the new candidate.
- p99 latency budget: ≤ 500 µs per scenario evaluation.

### Threat model + corpus categories

Install hardening:
  - unsigned_manifest, untrusted_publisher, forged_signature
  - user_grants_exceed_manifest
  - runtime_capability_outside_contract / runtime_egress_outside_contract /
    runtime_fs_outside_contract
  - **path_traversal_in_fs** (v0 misses — fs:/repo/../etc/passwd)
  - egress_wildcard_escape, egress_homoglyph
  - manifest_capability_inflation_post_install (warning surface)

Envelope:
  - missing_hmac, forged_hmac, payload_hash_mismatch
  - **replay** (v0 catches simple), **old_nonce_after_eviction** (v0 misses)
  - recipient_mismatch, stale_timestamp, future_timestamp
  - case_sensitive_hmac_attack
  - **capability_assertion_outside_contract** (v0 misses — cross-module
    wiring)

### What "incrementally improving" means here

Run #5 confirmed that LLM coder models truncate when asked to
rewrite ~300+ line modules. Run #6 tasks are **fragment-scoped**:
each task targets ONE function or method, not the whole module.
The candidate replaces only the named function and keeps the rest
unchanged. Bring back ALL other code unchanged — the harness will
fail if other parts disappear.

### Scoring

Same composite formula: score = detection_rate - 2*false_positive_rate
- latency_penalty. Higher is better. Baseline v0: detection 0.85,
fpr 0.0, composite 0.85 — exactly at SMCP threshold. The Triad
must beat it without losing any benign.
