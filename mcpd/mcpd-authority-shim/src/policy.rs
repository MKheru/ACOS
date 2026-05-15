//! Minimal [`CapabilityPolicy`] — uid-allowlist based capability evaluator.
//!
//! Today the policy is a flat allowlist of uids that may invoke any
//! method declared `RequiresCapability` by its handler. Tomorrow the
//! same surface will gain per-service, per-method, and per-resource
//! grants without breaking call sites.

use std::collections::HashSet;
use std::sync::RwLock;

use acos_authority_types::{CallerContext, DenyCode, Verdict};

/// Capability evaluator consulted by the router when a handler declares
/// a method as `RequiresCapability` or `GuardianOnly`.
///
/// **Default posture is deny**: a freshly constructed `CapabilityPolicy`
/// rejects every caller until at least one uid is allowlisted via
/// [`CapabilityPolicy::allow_uid`]. Anonymous callers
/// ([`CallerContext::is_anonymous`]) are always denied — they have no
/// kernel-attached identity and therefore no allowlist match is
/// semantically possible.
pub struct CapabilityPolicy {
    inner: RwLock<PolicyInner>,
}

struct PolicyInner {
    allowed_uids: HashSet<u32>,
    /// `true` if the Guardian scope has been bootstrapped and may grant
    /// itself the few `GuardianOnly` methods. Default `false` — set
    /// once by the bootstrap path.
    guardian_active: bool,
}

impl CapabilityPolicy {
    /// Construct an empty (deny-by-default) policy.
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(PolicyInner {
                allowed_uids: HashSet::new(),
                guardian_active: false,
            }),
        }
    }

    /// Add `uid` to the allowlist. Idempotent.
    pub fn allow_uid(&self, uid: u32) {
        if let Ok(mut g) = self.inner.write() {
            g.allowed_uids.insert(uid);
        }
    }

    /// Remove `uid` from the allowlist. No-op if it was not present.
    pub fn revoke_uid(&self, uid: u32) {
        if let Ok(mut g) = self.inner.write() {
            g.allowed_uids.remove(&uid);
        }
    }

    /// Mark the Guardian scope as active. From this point on,
    /// `GuardianOnly` methods are reachable for the Guardian itself —
    /// but only when the caller is the Guardian's uid (which the caller
    /// must independently allowlist). Set once by the boot path.
    pub fn activate_guardian(&self) {
        if let Ok(mut g) = self.inner.write() {
            g.guardian_active = true;
        }
    }

    /// Snapshot the current allowed uid set. For diagnostics; not used
    /// on the hot path.
    pub fn allowed_uid_snapshot(&self) -> Vec<u32> {
        self.inner
            .read()
            .map(|g| {
                let mut v: Vec<u32> = g.allowed_uids.iter().copied().collect();
                v.sort();
                v
            })
            .unwrap_or_default()
    }

    /// Decide whether `caller` may invoke `service`.`method` under the
    /// given handler policy. The caller is assumed to have been
    /// captured at handle-open time and threaded down to this point.
    ///
    /// Today the decision is uid-only (no per-method discrimination);
    /// `_service` and `_method` are accepted now so future refinements
    /// can add finer grants without breaking call sites.
    pub fn can_invoke(
        &self,
        caller: &CallerContext,
        _service: &str,
        _method: &str,
    ) -> Verdict {
        if caller.is_anonymous() {
            return Verdict::Deny(DenyCode::NoMatchingCapability);
        }
        let guard = match self.inner.read() {
            Ok(g) => g,
            Err(_) => return Verdict::Deny(DenyCode::AuditAppendFailed),
        };
        if guard.allowed_uids.contains(&caller.uid) {
            Verdict::Allow
        } else {
            Verdict::Deny(DenyCode::NoMatchingCapability)
        }
    }

    /// Decide whether `caller` may invoke a `GuardianOnly` method.
    /// Requires both the Guardian scope to be active AND the caller's
    /// uid to be on the allowlist (no separate Guardian-uid concept yet).
    pub fn can_invoke_guardian_only(
        &self,
        caller: &CallerContext,
        _service: &str,
        _method: &str,
    ) -> Verdict {
        if caller.is_anonymous() {
            return Verdict::Deny(DenyCode::BootstrapScopeViolation);
        }
        let guard = match self.inner.read() {
            Ok(g) => g,
            Err(_) => return Verdict::Deny(DenyCode::AuditAppendFailed),
        };
        if !guard.guardian_active {
            return Verdict::Deny(DenyCode::BootstrapScopeViolation);
        }
        if guard.allowed_uids.contains(&caller.uid) {
            Verdict::Allow
        } else {
            Verdict::Deny(DenyCode::NoMatchingCapability)
        }
    }
}

impl Default for CapabilityPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(uid: u32) -> CallerContext {
        CallerContext::from_parts(uid, 100, 1234)
    }

    #[test]
    fn empty_policy_denies_every_named_caller() {
        let p = CapabilityPolicy::new();
        assert!(matches!(
            p.can_invoke(&named(1000), "file", "read"),
            Verdict::Deny(DenyCode::NoMatchingCapability)
        ));
    }

    #[test]
    fn empty_policy_denies_anonymous_caller() {
        let p = CapabilityPolicy::new();
        let anon = CallerContext::anonymous();
        assert!(matches!(
            p.can_invoke(&anon, "echo", "echo"),
            Verdict::Deny(DenyCode::NoMatchingCapability)
        ));
    }

    #[test]
    fn allow_uid_then_can_invoke_returns_allow() {
        let p = CapabilityPolicy::new();
        p.allow_uid(1000);
        assert!(matches!(
            p.can_invoke(&named(1000), "file", "read"),
            Verdict::Allow
        ));
        // Different uid still denied.
        assert!(matches!(
            p.can_invoke(&named(1001), "file", "read"),
            Verdict::Deny(DenyCode::NoMatchingCapability)
        ));
    }

    #[test]
    fn revoke_uid_returns_caller_to_denied() {
        let p = CapabilityPolicy::new();
        p.allow_uid(1000);
        assert!(matches!(p.can_invoke(&named(1000), "x", "y"), Verdict::Allow));
        p.revoke_uid(1000);
        assert!(matches!(
            p.can_invoke(&named(1000), "x", "y"),
            Verdict::Deny(DenyCode::NoMatchingCapability)
        ));
    }

    #[test]
    fn guardian_only_requires_both_scope_and_allowlist() {
        let p = CapabilityPolicy::new();
        let caller = named(0);
        // No scope, no allowlist → deny.
        assert!(matches!(
            p.can_invoke_guardian_only(&caller, "guardian", "consult"),
            Verdict::Deny(DenyCode::BootstrapScopeViolation)
        ));
        // Scope active but no allowlist → deny.
        p.activate_guardian();
        assert!(matches!(
            p.can_invoke_guardian_only(&caller, "guardian", "consult"),
            Verdict::Deny(DenyCode::NoMatchingCapability)
        ));
        // Both present → allow.
        p.allow_uid(0);
        assert!(matches!(
            p.can_invoke_guardian_only(&caller, "guardian", "consult"),
            Verdict::Allow
        ));
    }

    #[test]
    fn allowed_uid_snapshot_returns_sorted_set() {
        let p = CapabilityPolicy::new();
        p.allow_uid(1001);
        p.allow_uid(0);
        p.allow_uid(1000);
        let snap = p.allowed_uid_snapshot();
        assert_eq!(snap, vec![0, 1000, 1001]);
    }
}
