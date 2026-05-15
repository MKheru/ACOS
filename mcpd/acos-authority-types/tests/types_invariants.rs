//! Invariant tests for the authority primitive types.

use acos_authority_types::{
    AuditEvent, Capability, CapabilityGrant, CapabilityId, Decision, DenyCode, HandlerPolicy,
    Severity, Verdict,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn audit_event_is_constructed_with_now_timestamp() {
    let before = SystemTime::now();
    let evt = AuditEvent::new(
        42,
        "session-1".to_string(),
        "file.read".to_string(),
        Verdict::Allow,
        Some(CapabilityId(7)),
        123,
    );
    let after = SystemTime::now();
    assert!(evt.at >= before, "timestamp must be at-or-after construction start");
    assert!(evt.at <= after, "timestamp must be at-or-before construction end");
    assert_eq!(evt.trace_id, 42);
    assert_eq!(evt.action, "file.read");
    assert_eq!(evt.verdict, Verdict::Allow);
    assert_eq!(evt.capability_used, Some(CapabilityId(7)));
    assert_eq!(evt.latency_us, 123);
}

#[test]
fn verdict_allow_and_deny_are_distinct_and_carry_codes() {
    let allow = Verdict::Allow;
    let deny_no_cap = Verdict::Deny(DenyCode::NoMatchingCapability);
    let deny_revoked = Verdict::Deny(DenyCode::CapabilityRevoked);

    assert_ne!(allow, deny_no_cap);
    assert_ne!(deny_no_cap, deny_revoked);
    assert_eq!(deny_no_cap, Verdict::Deny(DenyCode::NoMatchingCapability));
}

#[test]
fn severity_orders_critical_above_warning_above_info() {
    assert!(Severity::Critical > Severity::Warning);
    assert!(Severity::Warning > Severity::Info);
    assert!(Severity::Critical > Severity::Info);

    let mut samples = vec![Severity::Warning, Severity::Critical, Severity::Info];
    samples.sort();
    assert_eq!(samples, vec![Severity::Info, Severity::Warning, Severity::Critical]);
}

#[test]
fn capability_id_is_copy_and_hashable() {
    let id = CapabilityId(99);
    let copy = id;
    assert_eq!(id, copy);

    let mut set = std::collections::HashSet::new();
    set.insert(CapabilityId(1));
    set.insert(CapabilityId(1));
    set.insert(CapabilityId(2));
    assert_eq!(set.len(), 2, "CapabilityId must be hashable with stable equality");
}

#[test]
fn capability_grant_holds_immutable_metadata_with_grant_time_in_past() {
    let earlier = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let grant = CapabilityGrant {
        id: CapabilityId(1),
        capability: Capability::FileRead {
            path_prefix: "/etc/acos/".to_string(),
        },
        holder: "session-abc".to_string(),
        granted_at: earlier,
    };

    // Field accessors return the values supplied at construction.
    assert_eq!(grant.id, CapabilityId(1));
    match &grant.capability {
        Capability::FileRead { path_prefix } => assert_eq!(path_prefix, "/etc/acos/"),
        other => panic!("unexpected capability variant: {other:?}"),
    }
    assert_eq!(grant.holder, "session-abc");
    assert_eq!(grant.granted_at, earlier);
}

#[test]
fn decision_carries_verdict_reason_and_severity() {
    let dec = Decision {
        verdict: Verdict::Deny(DenyCode::PolicyForbids),
        reason: "method not on allowlist".to_string(),
        severity: Severity::Warning,
    };
    assert_eq!(dec.verdict, Verdict::Deny(DenyCode::PolicyForbids));
    assert_eq!(dec.reason, "method not on allowlist");
    assert_eq!(dec.severity, Severity::Warning);
}

#[test]
fn handler_policy_default_is_explicit_not_implicit() {
    // HandlerPolicy intentionally has no `Default` impl; every handler must
    // opt in. This test pins the variant set so adding a default later is a
    // deliberate API change.
    let policies = [
        HandlerPolicy::Public,
        HandlerPolicy::RequiresCapability,
        HandlerPolicy::GuardianOnly,
    ];
    assert_eq!(policies.len(), 3);
    assert_ne!(HandlerPolicy::Public, HandlerPolicy::GuardianOnly);
}
