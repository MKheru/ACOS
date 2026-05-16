//! Signed Guardian decision emission.

use acos_authority_types::Decision;

use crate::action_bridge::ActionRequest;

/// In-memory Guardian signing key.
///
/// This skeleton deliberately avoids external crypto dependencies. The type
/// exposes a deterministic keyed digest suitable for tests and for wiring the
/// authority surface; production-strength signing can replace the internals
/// without changing the `DecisionEmitter` API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardianKey {
    bytes: [u8; 32],
}

impl GuardianKey {
    /// Create a key from 32 bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    fn sign(&self, payload: &[u8]) -> [u8; 32] {
        let mut state = self.bytes;
        for (idx, byte) in payload.iter().enumerate() {
            let slot = idx % state.len();
            state[slot] = state[slot]
                .wrapping_add(*byte)
                .rotate_left((idx % 8) as u32)
                ^ self.bytes[(idx * 7) % self.bytes.len()];
        }
        state
    }
}

/// Decision plus deterministic Guardian signature.
#[derive(Clone, Debug)]
pub struct SignedDecision {
    /// Monotonic emitter-local sequence number.
    pub sequence: u64,
    /// Action request the decision was made for.
    pub request: ActionRequest,
    /// Authority decision returned by policy.
    pub decision: Decision,
    /// Keyed signature over sequence, request, and decision fields.
    pub signature: [u8; 32],
}

/// Emits signed decision records.
pub struct DecisionEmitter {
    key: GuardianKey,
    next_sequence: u64,
}

impl DecisionEmitter {
    /// Create an emitter using the provided Guardian key.
    pub const fn new(key: GuardianKey) -> Self {
        Self {
            key,
            next_sequence: 0,
        }
    }

    /// Emit one signed decision and advance the sequence counter.
    pub fn emit(&mut self, request: ActionRequest, decision: Decision) -> SignedDecision {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let payload = decision_payload(sequence, &request, &decision);
        let signature = self.key.sign(payload.as_bytes());
        SignedDecision {
            sequence,
            request,
            decision,
            signature,
        }
    }
}

fn decision_payload(sequence: u64, request: &ActionRequest, decision: &Decision) -> String {
    format!(
        "seq={sequence};action={};caller={};mut={};verdict={:?};reason={};severity={:?}",
        request.action,
        request.caller,
        request.mutates_state,
        decision.verdict,
        decision.reason,
        decision.severity
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use acos_authority_types::{Decision, Severity, Verdict};

    #[test]
    fn decision_emitter_signs_and_sequences_records() {
        let key = GuardianKey::from_bytes([7; 32]);
        let mut emitter = DecisionEmitter::new(key);
        let request = ActionRequest::new("observability.recent", "guardian", false);
        let decision = Decision {
            verdict: Verdict::Allow,
            reason: "ok".to_string(),
            severity: Severity::Info,
        };

        let first = emitter.emit(request.clone(), decision.clone());
        let second = emitter.emit(request, decision);

        assert_eq!(first.sequence, 0);
        assert_eq!(second.sequence, 1);
        assert_ne!(first.signature, [0; 32]);
        assert_ne!(first.signature, second.signature);
    }
}
