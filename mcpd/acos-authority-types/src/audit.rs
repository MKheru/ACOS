//! Audit event — immutable, append-only.

use std::time::SystemTime;

use crate::capability::CapabilityId;
use crate::decision::Verdict;

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
    /// Outcome of the authority check.
    pub verdict: Verdict,
    /// The capability that authorised the action, if any.
    pub capability_used: Option<CapabilityId>,
    /// Wall-clock latency of the authority check itself, in microseconds.
    pub latency_us: u64,
}

impl AuditEvent {
    /// Construct an audit event, recording the current time.
    ///
    /// This is the only public constructor; combined with `#[non_exhaustive]`
    /// it enforces that every event carries a timestamp and that no future
    /// field can be silently default-constructed without a corresponding
    /// update here.
    pub fn new(
        trace_id: u128,
        caller: String,
        action: String,
        verdict: Verdict,
        capability_used: Option<CapabilityId>,
        latency_us: u64,
    ) -> Self {
        Self {
            trace_id,
            at: SystemTime::now(),
            caller,
            action,
            verdict,
            capability_used,
            latency_us,
        }
    }
}
