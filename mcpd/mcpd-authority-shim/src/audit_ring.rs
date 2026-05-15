//! Bounded append-only ring buffer of [`AuditEvent`]s.

use std::sync::Mutex;

use acos_authority_types::AuditEvent;

/// Append-only ring buffer of [`AuditEvent`]s with a hard capacity cap.
///
/// Once the buffer is full, new appends overwrite the oldest event. Reads
/// always see events in insertion order (oldest first) up to capacity.
///
/// **Invariant**: `append` never blocks indefinitely; if the internal lock
/// is poisoned it returns [`AuditRingError::Poisoned`] rather than
/// panicking, so callers can fail-closed.
pub struct AuditRing {
    inner: Mutex<RingInner>,
}

struct RingInner {
    capacity: usize,
    events: Vec<AuditEvent>,
    /// Index where the next append will write (also where the oldest
    /// event lives once the buffer has wrapped at least once).
    next: usize,
    /// Total number of events ever appended. Strictly monotonic. Used
    /// by callers to detect skipped events when reading subsets.
    total_appends: u64,
}

/// Errors that can be returned by [`AuditRing::append`].
#[derive(Debug)]
pub enum AuditRingError {
    /// The internal lock has been poisoned by a panic in another thread.
    /// Callers must treat this as fail-closed.
    Poisoned,
    /// The ring's capacity is zero — nothing can be stored.
    ZeroCapacity,
}

impl AuditRing {
    /// Create a new ring with the given capacity. A zero capacity is
    /// permitted at construction but every [`AuditRing::append`] on such a
    /// ring will return [`AuditRingError::ZeroCapacity`].
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(RingInner {
                capacity,
                events: Vec::with_capacity(capacity),
                next: 0,
                total_appends: 0,
            }),
        }
    }

    /// Append an event to the ring. Overwrites the oldest entry if the
    /// ring is at capacity.
    pub fn append(&self, event: AuditEvent) -> Result<(), AuditRingError> {
        let mut guard = self.inner.lock().map_err(|_| AuditRingError::Poisoned)?;
        if guard.capacity == 0 {
            return Err(AuditRingError::ZeroCapacity);
        }
        if guard.events.len() < guard.capacity {
            guard.events.push(event);
        } else {
            let idx = guard.next;
            guard.events[idx] = event;
            let cap = guard.capacity;
            guard.next = (idx + 1) % cap;
        }
        guard.total_appends = guard.total_appends.saturating_add(1);
        Ok(())
    }

    /// Number of events currently stored (≤ capacity).
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.events.len()).unwrap_or(0)
    }

    /// Return `true` if no events have been stored yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total number of append calls since creation (regardless of overwrite).
    pub fn total_appends(&self) -> u64 {
        self.inner.lock().map(|g| g.total_appends).unwrap_or(0)
    }

    /// Snapshot of the events currently stored, in insertion order
    /// (oldest first). Returns an empty vector if the lock is poisoned.
    pub fn snapshot(&self) -> Vec<AuditEvent> {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        if guard.events.len() < guard.capacity {
            guard.events.clone()
        } else {
            // Wrapped — re-order so oldest comes first.
            let mut out = Vec::with_capacity(guard.events.len());
            for i in 0..guard.events.len() {
                let idx = (guard.next + i) % guard.capacity;
                out.push(guard.events[idx].clone());
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acos_authority_types::{Verdict};

    fn ev(trace_id: u128, action: &str) -> AuditEvent {
        AuditEvent::new(
            trace_id,
            "test".to_string(),
            action.to_string(),
            Verdict::Allow,
            None,
            0,
        )
    }

    #[test]
    fn empty_ring_has_zero_length_and_no_events() {
        let r = AuditRing::with_capacity(4);
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
        assert_eq!(r.total_appends(), 0);
        assert!(r.snapshot().is_empty());
    }

    #[test]
    fn append_grows_until_capacity_then_overwrites_oldest() {
        let r = AuditRing::with_capacity(3);
        r.append(ev(1, "a")).unwrap();
        r.append(ev(2, "b")).unwrap();
        r.append(ev(3, "c")).unwrap();
        assert_eq!(r.len(), 3);
        let snap = r.snapshot();
        assert_eq!(
            snap.iter().map(|e| e.trace_id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        // Wrap: append d, e — oldest two (a, b) evicted.
        r.append(ev(4, "d")).unwrap();
        r.append(ev(5, "e")).unwrap();
        assert_eq!(r.len(), 3);
        let snap = r.snapshot();
        assert_eq!(
            snap.iter().map(|e| e.trace_id).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
        assert_eq!(r.total_appends(), 5);
    }

    #[test]
    fn zero_capacity_ring_refuses_every_append() {
        let r = AuditRing::with_capacity(0);
        let result = r.append(ev(1, "a"));
        assert!(matches!(result, Err(AuditRingError::ZeroCapacity)));
        assert_eq!(r.len(), 0);
        assert_eq!(r.total_appends(), 0);
    }

    #[test]
    fn total_appends_is_monotonic_even_after_wrap() {
        let r = AuditRing::with_capacity(2);
        for i in 0..10 {
            r.append(ev(i, "x")).unwrap();
        }
        assert_eq!(r.len(), 2);
        assert_eq!(r.total_appends(), 10);
    }
}
