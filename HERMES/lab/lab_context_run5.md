## SMCP Run #5 — MCP Reputation Registry

The lab evaluates a Python module that maintains identity + reputation
state for MCP servers and modulates trust decisions based on accumulated
incident history. This is a behavioural defence layer that complements
the regex sanitiser (Patches 1+7+8) and the provenance gate (Run #4).

### Core idea

Each MCP has a stable UUID + declared capability manifest. As incidents
are observed (regex flags, capability violations, exfil attempts), they
reduce a per-MCP reputation score. The score determines a band
(BANNED / HIGH_SUSPICION / PROBATION / TRUSTED / VETERAN) which dictates
allowed actions. Recovery happens slowly (0.1/day after 7d clean
window). Critical incidents trigger 24h quarantine independent of
score.

### Required public API

```python
from enum import IntEnum
from dataclasses import dataclass, field
from typing import Optional

class IncidentSeverity(IntEnum):
    LOW = 1
    MEDIUM = 5
    HIGH = 15
    CRITICAL = 30

@dataclass(frozen=True)
class MCPIdentity:
    uuid: str
    name: str
    install_origin: str = ""
    declared_capabilities: frozenset[str] = frozenset()

class MCPReputationRegistry:
    def __init__(self, now_fn=...):
        ...
    def register(self, identity: MCPIdentity):
        ...
    def record_incident(self, uuid: str, severity: IncidentSeverity,
                        category: str, detector: str, sample: str = "") -> ...:
        ...
    def evaluate_action(self, uuid: str, action: str,
                        context: dict) -> tuple[bool, str, dict]:
        """action in {tool_call, sampling, structured_response}.
        context may include: requested_capability, user_authorised, depth.
        Returns (allow, reason, metadata)."""
        ...
    # Optional:
    def apply_recovery(self, uuid: str): ...
    def get_state(self, uuid: str): ...
    def to_json(self) -> str: ...
    @classmethod
    def from_json(cls, s: str, now_fn=...) -> "MCPReputationRegistry": ...
```

### Threat model

The corpus replays a per-scenario incident history at deterministic
timestamps, then asks for a verdict on a proposed action. The harness
checks the candidate's allow/block against `expected_block` per
scenario.

What we want the gate to enforce:

1. **BANNED band (score < 30)** — refuse every action.
2. **Quarantine** — independent override; CRITICAL incident → 24h hard
   block regardless of score.
3. **HIGH_SUSPICION (30 ≤ score < 60)** — refuse sampling, downgrade
   to read-only-only tool calls, structured_response allowed.
4. **PROBATION (60 ≤ score < 80)** — sampling requires user_authorised,
   tool_call requires capability ∈ declared_capabilities.
5. **TRUSTED + VETERAN (score ≥ 80)** — basic capability check still
   applies; otherwise allow.

Behaviour the v0 baseline already covers correctly. The Triad
optimisation targets are the EDGE CASES the corpus exposes:

  - cluster_escalation: 3+ MEDIUMs in 1h should escalate to a
    synthetic HIGH and trigger a partial quarantine (1h)
  - capability boundary inference (when capability isn't declared, is
    that grounds for HIGH penalty? Or just refusal of this call?)
  - non-linear decay (gamma rather than linear) so trust loss is
    quadratic on repeat offenders
  - incident classification refinement (regex flag → LOW first time,
    upgrade if repeated)
  - persistence robustness (atomic writes, schema migration)
  - cross-MCP contamination (when a chain involves multiple MCPs,
    propagate the lowest score across the chain)

### Constraints

- Python stdlib only (no pip dependencies).
- The harness builds MCPIdentity by keyword args (uuid, name,
  install_origin, declared_capabilities) — keep these field names.
- IncidentSeverity values are sortable IntEnum LOW=1, MEDIUM=5, HIGH=15,
  CRITICAL=30 — keep these names and values stable.
- evaluate_action signature must return a 3-tuple (bool, str, dict).
- p99 latency budget: ≤ 500 µs per evaluate_action call.

### Scoring

Same composite formula as previous runs: score = detection_rate -
2*false_positive_rate - latency_penalty. Higher is better.

The v0 baseline already scores ~0.87 on the corpus — above the SMCP
target. This run is about HARDENING the design (Tasks 1-6) rather than
chasing a couple more points. Promotions are valuable when they catch
the corpus edge cases or make the system materially harder for an
adversary to game.
