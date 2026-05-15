//! Audit event — immutable, append-only.

use std::time::SystemTime;

use crate::capability::CapabilityId;
use crate::decision::Verdict;
use crate::event_kind::EventKind;

/// Immutable audit event for the append-only ring buffer.
///
/// **Invariant**: there are no `&mut self` methods on this type. Fields are
/// public for read access but the type is constructed via [`AuditEvent::new`]
/// only. Downstream code must treat a constructed event as read-only.
///
/// `#[non_exhaustive]` prevents external code from creating instances
/// outside `new(...)` (which encodes the invariant of always taking a
/// timestamp at construction) and keeps the type forward-compatible.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AuditEvent {
    /// Trace identifier correlating events of the same request chain.
    pub trace_id: u128,
    /// When the event was constructed.
    pub at: SystemTime,
    /// Caller identity (session id, service name, or `"bootstrap"`).
    pub caller: String,
    /// Action attempted (typically `"<service>.<method>"`).
    pub action: String,
    /// Outcome of the authority check. For [`EventKind::Intent`] events
    /// this is a placeholder (`Verdict::Allow`) — read the paired
    /// `Result` event for the real outcome.
    pub verdict: Verdict,
    /// The capability that authorised the action, if any.
    pub capability_used: Option<CapabilityId>,
    /// Wall-clock latency of the authority check itself, in microseconds.
    /// Always `0` on Intent events (latency is unknown at intent time).
    pub latency_us: u64,
    /// WS3.M8 — Phase of the dual-phase audit pair. `Result` is the
    /// historical default for single-phase emitters and unit fixtures;
    /// `Intent` is set by [`AuditEvent::new_intent`].
    pub kind: EventKind,
}

impl AuditEvent {
    /// Construct a `Result`-phase audit event, recording the current time.
    ///
    /// Backward-compatible single-phase constructor: every audit event
    /// built via this entry point is tagged as `EventKind::Result`. Use
    /// [`AuditEvent::new_intent`] for the paired pre-dispatch event in
    /// WS3.M8 dual-phase emission.
    pub fn new(
        trace_id: u128,
        caller: String,
        action: String,
        verdict: Verdict,
        capability_used: Option<CapabilityId>,
        latency_us: u64,
    ) -> Self {
        Self::new_with_kind(
            trace_id,
            caller,
            action,
            verdict,
            capability_used,
            latency_us,
            EventKind::Result,
        )
    }

    /// WS3.M8 — Construct an `Intent`-phase audit event for the pre-dispatch
    /// side of a paired emission. Latency is recorded as 0 (unknown at
    /// intent time); verdict is conventionally `Verdict::Allow` as a
    /// placeholder — observers must read the matching `Result` event to
    /// learn the real outcome.
    pub fn new_intent(
        trace_id: u128,
        caller: String,
        action: String,
        capability_used: Option<CapabilityId>,
    ) -> Self {
        Self::new_with_kind(
            trace_id,
            caller,
            action,
            Verdict::Allow,
            capability_used,
            0,
            EventKind::Intent,
        )
    }

    /// Lower-level constructor exposing the `kind` parameter explicitly,
    /// used by both [`AuditEvent::new`] and [`AuditEvent::new_intent`].
    pub fn new_with_kind(
        trace_id: u128,
        caller: String,
        action: String,
        verdict: Verdict,
        capability_used: Option<CapabilityId>,
        latency_us: u64,
        kind: EventKind,
    ) -> Self {
        Self {
            trace_id,
            at: SystemTime::now(),
            caller,
            action,
            verdict,
            capability_used,
            latency_us,
            kind,
        }
    }
}
