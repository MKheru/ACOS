//! Read-only audit access for Guardian observers.

use acos_authority_types::{AuditEvent, EventKind, Verdict};
use mcpd_authority_shim::AuditRing;

/// Immutable snapshot extracted from the authority audit ring.
#[derive(Clone, Debug)]
pub struct AuditSnapshot {
    /// Events retained by the ring, oldest first.
    pub events: Vec<AuditEvent>,
    /// Total append count observed when the snapshot was taken.
    pub total_appends: u64,
    /// Number of events that were already overwritten by ring retention.
    pub dropped_events: u64,
}

impl AuditSnapshot {
    /// Return true when at least one retained result event denied an action.
    pub fn contains_denial(&self) -> bool {
        self.events.iter().any(|event| {
            matches!(event.kind, EventKind::Result) && matches!(event.verdict, Verdict::Deny(_))
        })
    }
}

/// Read-only adapter over [`AuditRing`] for Guardian policy/observer code.
pub struct AuditReader<'a> {
    ring: &'a AuditRing,
}

impl<'a> AuditReader<'a> {
    /// Create a reader over an existing authority audit ring.
    pub const fn new(ring: &'a AuditRing) -> Self {
        Self { ring }
    }

    /// Capture retained events plus overwrite metadata without mutating the ring.
    pub fn snapshot(&self) -> AuditSnapshot {
        let events = self.ring.snapshot();
        let total_appends = self.ring.total_appends();
        let dropped_events = total_appends.saturating_sub(events.len() as u64);
        AuditSnapshot {
            events,
            total_appends,
            dropped_events,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acos_authority_types::Verdict;

    fn event(trace_id: u128) -> AuditEvent {
        AuditEvent::new(
            trace_id,
            "test".into(),
            "file.read".into(),
            Verdict::Allow,
            None,
            7,
        )
    }

    #[test]
    fn audit_reader_reports_snapshot_and_dropped_count() {
        let ring = AuditRing::with_capacity(2);
        ring.append(event(1)).unwrap();
        ring.append(event(2)).unwrap();
        ring.append(event(3)).unwrap();

        let snapshot = AuditReader::new(&ring).snapshot();

        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.total_appends, 3);
        assert_eq!(snapshot.dropped_events, 1);
        assert_eq!(snapshot.events[0].trace_id, 2);
        assert_eq!(snapshot.events[1].trace_id, 3);
    }
}
