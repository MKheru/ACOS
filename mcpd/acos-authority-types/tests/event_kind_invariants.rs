//! Invariant tests for [`EventKind`] and the dual-phase [`AuditEvent`]
//! constructors `new` (Result-kind) and `new_intent` (Intent-kind).

use acos_authority_types::{AuditEvent, EventKind, Verdict};

#[test]
fn default_kind_is_result() {
    let k: EventKind = Default::default();
    assert_eq!(k, EventKind::Result);
}

#[test]
fn event_kind_labels_are_stable() {
    assert_eq!(EventKind::Intent.label(), "intent");
    assert_eq!(EventKind::Result.label(), "result");
}

#[test]
fn legacy_new_constructor_emits_result_kind() {
    let evt = AuditEvent::new(
        1,
        "test".into(),
        "echo.echo".into(),
        Verdict::Allow,
        None,
        17,
    );
    assert_eq!(evt.kind, EventKind::Result);
    assert_eq!(evt.latency_us, 17);
}

#[test]
fn new_intent_constructor_emits_intent_kind_with_zero_latency() {
    let evt = AuditEvent::new_intent(42, "test".into(), "file.read".into(), None);
    assert_eq!(evt.kind, EventKind::Intent);
    // Latency is unknown at intent time — recorded as 0 by contract.
    assert_eq!(evt.latency_us, 0);
    // Verdict is a placeholder Allow on Intent events.
    assert_eq!(evt.verdict, Verdict::Allow);
    assert_eq!(evt.trace_id, 42);
    assert_eq!(evt.action, "file.read");
}

#[test]
fn intent_and_result_pair_share_trace_id_for_correlation() {
    let intent = AuditEvent::new_intent(99, "session-1".into(), "net.http_get".into(), None);
    let result = AuditEvent::new(
        99,
        "session-1".into(),
        "net.http_get".into(),
        Verdict::Allow,
        None,
        2500,
    );
    assert_eq!(intent.trace_id, result.trace_id);
    assert_ne!(intent.kind, result.kind);
}

#[test]
fn new_with_kind_lets_caller_pick_phase() {
    let intent = AuditEvent::new_with_kind(
        1,
        "x".into(),
        "y".into(),
        Verdict::Allow,
        None,
        0,
        EventKind::Intent,
    );
    let result = AuditEvent::new_with_kind(
        1,
        "x".into(),
        "y".into(),
        Verdict::Allow,
        None,
        12,
        EventKind::Result,
    );
    assert_eq!(intent.kind, EventKind::Intent);
    assert_eq!(result.kind, EventKind::Result);
}
