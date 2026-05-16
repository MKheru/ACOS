//! Action bridge between Guardian policy and signed decision emission.

use acos_authority_types::{Decision, Verdict};

use crate::decision_emitter::{DecisionEmitter, SignedDecision};
use crate::policy_engine::PolicyEngine;

/// Minimal typed request presented to Guardian authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRequest {
    /// Action name, normally `<service>.<method>`.
    pub action: String,
    /// Caller label associated with the request.
    pub caller: String,
    /// Whether the action can mutate system state or perform egress.
    pub mutates_state: bool,
}

impl ActionRequest {
    /// Construct a new action request.
    pub fn new(action: impl Into<String>, caller: impl Into<String>, mutates_state: bool) -> Self {
        Self {
            action: action.into(),
            caller: caller.into(),
            mutates_state,
        }
    }
}

/// Bridge errors returned before dispatching an action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BridgeError {
    /// The policy returned a deny verdict; the signed record is attached so it
    /// can still be audited by the caller.
    Denied(SignedDecisionSummary),
}

/// Comparable summary of a signed decision for bridge errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedDecisionSummary {
    /// Sequence number of the emitted record.
    pub sequence: u64,
    /// Signature of the emitted record.
    pub signature: [u8; 32],
}

impl From<&SignedDecision> for SignedDecisionSummary {
    fn from(value: &SignedDecision) -> Self {
        Self {
            sequence: value.sequence,
            signature: value.signature,
        }
    }
}

/// Policy + emitter bridge that always emits a decision before returning.
pub struct ActionBridge<P: PolicyEngine> {
    policy: P,
    emitter: DecisionEmitter,
}

impl<P: PolicyEngine> ActionBridge<P> {
    /// Create a new action bridge.
    pub const fn new(policy: P, emitter: DecisionEmitter) -> Self {
        Self { policy, emitter }
    }

    /// Evaluate `request`, emit a signed decision, and return allow/deny.
    pub fn authorize(&mut self, request: ActionRequest) -> Result<SignedDecision, BridgeError> {
        let decision: Decision = self.policy.evaluate(&request);
        let signed = self.emitter.emit(request, decision);
        match signed.decision.verdict {
            Verdict::Allow => Ok(signed),
            Verdict::Deny(_) => Err(BridgeError::Denied(SignedDecisionSummary::from(&signed))),
            _ => Err(BridgeError::Denied(SignedDecisionSummary::from(&signed))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision_emitter::GuardianKey;
    use crate::policy_engine::InlineGuardianPolicy;

    #[test]
    fn action_bridge_emits_allow_before_dispatch() {
        let key = GuardianKey::from_bytes([9; 32]);
        let policy = InlineGuardianPolicy::default();
        let mut bridge = ActionBridge::new(policy, DecisionEmitter::new(key));

        let signed = bridge
            .authorize(ActionRequest::new(
                "observability.recent",
                "guardian",
                false,
            ))
            .unwrap();

        assert_eq!(signed.sequence, 0);
        assert!(matches!(signed.decision.verdict, Verdict::Allow));
    }

    #[test]
    fn action_bridge_returns_signed_denial_summary() {
        let key = GuardianKey::from_bytes([10; 32]);
        let policy = InlineGuardianPolicy::default();
        let mut bridge = ActionBridge::new(policy, DecisionEmitter::new(key));

        let err = bridge
            .authorize(ActionRequest::new("service.restart", "guardian", true))
            .unwrap_err();

        match err {
            BridgeError::Denied(summary) => {
                assert_eq!(summary.sequence, 0);
                assert_ne!(summary.signature, [0; 32]);
            }
        }
    }
}
