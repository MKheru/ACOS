//! Guardian policy engine and reactive audit observer.

use acos_authority_types::{Decision, DenyCode, Severity, Verdict};

use crate::action_bridge::ActionRequest;
use crate::audit_reader::AuditSnapshot;

/// Shared interface for Guardian policy implementations.
pub trait PolicyEngine {
    /// Evaluate an action and return the authority decision to emit.
    fn evaluate(&self, request: &ActionRequest) -> Decision;
}

/// Inline policy used before dynamic Guardian rules are available.
///
/// The default posture is deny-by-default for mutating actions while allowing
/// read-only observations. This keeps the split explicit: inline policy decides
/// the current action; [`ReactiveGuardianObserver`] derives follow-up signals
/// from audit history.
#[derive(Clone, Debug)]
pub struct InlineGuardianPolicy {
    allowed_read_prefixes: Vec<String>,
}

impl InlineGuardianPolicy {
    /// Construct the conservative built-in Guardian policy.
    pub fn conservative() -> Self {
        Self {
            allowed_read_prefixes: vec!["observability.".to_string(), "guardian.read".to_string()],
        }
    }

    fn read_is_allowed(&self, action: &str) -> bool {
        self.allowed_read_prefixes
            .iter()
            .any(|prefix| action.starts_with(prefix))
    }
}

impl Default for InlineGuardianPolicy {
    fn default() -> Self {
        Self::conservative()
    }
}

impl PolicyEngine for InlineGuardianPolicy {
    fn evaluate(&self, request: &ActionRequest) -> Decision {
        if !request.mutates_state || self.read_is_allowed(&request.action) {
            Decision {
                verdict: Verdict::Allow,
                reason: format!("inline guardian allow: {}", request.action),
                severity: Severity::Info,
            }
        } else {
            Decision {
                verdict: Verdict::Deny(DenyCode::NoMatchingCapability),
                reason: format!("inline guardian deny mutable action: {}", request.action),
                severity: Severity::Warning,
            }
        }
    }
}

/// Reactive observer over audit history.
///
/// It does not decide the current action. It reports whether retained audit
/// history contains enough security signal for Guardian follow-up.
#[derive(Clone, Copy, Debug)]
pub struct ReactiveGuardianObserver {
    denial_threshold: usize,
}

impl ReactiveGuardianObserver {
    /// Create an observer that triggers after `denial_threshold` denied results.
    pub const fn new(denial_threshold: usize) -> Self {
        Self { denial_threshold }
    }

    /// Count retained denied result events and return true when threshold is met.
    pub fn should_escalate(&self, snapshot: &AuditSnapshot) -> bool {
        let denied = snapshot
            .events
            .iter()
            .filter(|event| matches!(event.verdict, Verdict::Deny(_)))
            .count();
        denied >= self.denial_threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acos_authority_types::{AuditEvent, Verdict};

    #[test]
    fn inline_policy_allows_read_only_observation() {
        let policy = InlineGuardianPolicy::default();
        let request = ActionRequest::new("observability.recent", "guardian", false);

        let decision = policy.evaluate(&request);

        assert!(matches!(decision.verdict, Verdict::Allow));
        assert_eq!(decision.severity, Severity::Info);
    }

    #[test]
    fn inline_policy_denies_mutating_action_by_default() {
        let policy = InlineGuardianPolicy::default();
        let request = ActionRequest::new("service.restart", "guardian", true);

        let decision = policy.evaluate(&request);

        assert!(matches!(
            decision.verdict,
            Verdict::Deny(DenyCode::NoMatchingCapability)
        ));
        assert_eq!(decision.severity, Severity::Warning);
    }

    #[test]
    fn reactive_observer_escalates_after_denial_threshold() {
        let events = vec![
            AuditEvent::new(1, "a".into(), "x".into(), Verdict::Allow, None, 1),
            AuditEvent::new(
                2,
                "b".into(),
                "y".into(),
                Verdict::Deny(DenyCode::PolicyForbids),
                None,
                1,
            ),
        ];
        let snapshot = AuditSnapshot {
            events,
            total_appends: 2,
            dropped_events: 0,
        };

        assert!(ReactiveGuardianObserver::new(1).should_escalate(&snapshot));
        assert!(!ReactiveGuardianObserver::new(2).should_escalate(&snapshot));
    }
}
