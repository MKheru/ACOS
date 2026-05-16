//! Fixed-capacity in-memory observation ring.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use crate::event::ObservationEvent;

const DEFAULT_EVENT_CAPACITY: usize = 4096;

/// Preallocated fixed-capacity observation store.
#[derive(Debug)]
pub struct ObservationRing {
    cap: usize,
    events: VecDeque<ObservationEvent>,
}

impl ObservationRing {
    pub fn with_capacity(cap: usize) -> Self {
        let cap = cap.max(1);
        Self {
            cap,
            events: VecDeque::with_capacity(cap),
        }
    }

    pub fn push(&mut self, event: ObservationEvent) {
        if self.events.len() == self.cap {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    pub fn recent(&self, limit: usize) -> Vec<ObservationEvent> {
        let len = self.events.len();
        let start = len.saturating_sub(limit);
        self.events.iter().skip(start).cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }
}

static GLOBAL_OBSERVATION_RING: OnceLock<Mutex<ObservationRing>> = OnceLock::new();

fn global_ring() -> &'static Mutex<ObservationRing> {
    GLOBAL_OBSERVATION_RING
        .get_or_init(|| Mutex::new(ObservationRing::with_capacity(DEFAULT_EVENT_CAPACITY)))
}

/// Append an observation to the process-global mcpd ring.
pub fn append_event(event: ObservationEvent) {
    if let Ok(mut ring) = global_ring().lock() {
        ring.push(event);
    }
}

/// Return the newest `limit` events from the process-global ring.
pub fn recent_events(limit: usize) -> Vec<ObservationEvent> {
    global_ring()
        .lock()
        .map(|ring| ring.recent(limit))
        .unwrap_or_default()
}

/// Test/support hook: clear all global observations.
pub fn clear_events() {
    if let Ok(mut ring) = global_ring().lock() {
        ring.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ObservationEvent, ObservationKind, SpanId, TraceId};

    #[test]
    fn ring_buffer_evicts_oldest_at_cap() {
        let mut ring = ObservationRing::with_capacity(2);
        ring.push(ObservationEvent::intent(TraceId(1), SpanId(1), "svc", "a"));
        ring.push(ObservationEvent::intent(TraceId(2), SpanId(2), "svc", "b"));
        ring.push(ObservationEvent::intent(TraceId(3), SpanId(3), "svc", "c"));

        let events = ring.recent(10);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].trace_id, TraceId(2));
        assert_eq!(events[1].trace_id, TraceId(3));
        assert_eq!(events[1].kind, ObservationKind::Intent);
    }
}
