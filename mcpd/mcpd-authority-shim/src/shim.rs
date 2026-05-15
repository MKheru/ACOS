//! Top-level [`AuthorityShim`] that wires the audit ring and the boot
//! bootstrap into a single handle.

use acos_authority_types::AuditEvent;

use crate::audit_ring::{AuditRing, AuditRingError};

/// Default audit ring capacity — chosen to keep memory bounded
/// (~1 MiB at this size of `AuditEvent`) while remaining large enough
/// to retain a meaningful action history during normal operation.
pub const DEFAULT_AUDIT_RING_CAPACITY: usize = 4096;

/// Glue object that owns the audit ring and offers a single entry
/// point ([`AuthorityShim::record`]) for handlers and the router to
/// log decisions.
///
/// In the transitional state this crate intentionally does **not** make
/// allow/deny decisions itself — that work lives in
/// `mcp-scheme::router::Router::route` via `required_policy()`. The shim
/// is the future home for that logic once
/// [`acos_authority_types::Decision`] and `CallerContext` are threaded
/// through the dispatch path (WS2.M3 + WS3 capability fabric).
pub struct AuthorityShim {
    ring: AuditRing,
}

impl AuthorityShim {
    /// Create a shim with the default audit ring capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_AUDIT_RING_CAPACITY)
    }

    /// Create a shim with a custom audit ring capacity.
    pub fn with_capacity(audit_capacity: usize) -> Self {
        Self {
            ring: AuditRing::with_capacity(audit_capacity),
        }
    }

    /// Record an [`AuditEvent`] in the ring. Returns the underlying
    /// error so the caller can fail-closed if the append fails.
    pub fn record(&self, event: AuditEvent) -> Result<(), AuditRingError> {
        self.ring.append(event)
    }

    /// Borrow the audit ring for read access (snapshots, length).
    pub fn audit_ring(&self) -> &AuditRing {
        &self.ring
    }
}

impl Default for AuthorityShim {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acos_authority_types::Verdict;

    #[test]
    fn shim_records_event_and_exposes_via_ring() {
        let shim = AuthorityShim::new();
        let evt = AuditEvent::new(
            42,
            "router".to_string(),
            "file.read".to_string(),
            Verdict::Allow,
            None,
            17,
        );
        shim.record(evt).unwrap();
        let snap = shim.audit_ring().snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].trace_id, 42);
        assert_eq!(snap[0].action, "file.read");
        assert_eq!(snap[0].latency_us, 17);
    }

    #[test]
    fn shim_with_zero_capacity_fails_closed_on_record() {
        let shim = AuthorityShim::with_capacity(0);
        let evt = AuditEvent::new(
            1,
            "router".to_string(),
            "noop".to_string(),
            Verdict::Allow,
            None,
            0,
        );
        let result = shim.record(evt);
        assert!(matches!(result, Err(AuditRingError::ZeroCapacity)));
    }
}
