//! ACOS mcpd observability primitives.
//!
//! WS11.M3 starts with redaction: traces must be whitelist-only and any
//! non-whitelisted parameter value is replaced by a boot-salted hash.

pub mod event;
pub mod redact;
pub mod ring;

pub use event::{ObservationEvent, ObservationKind, ResultStatus, SpanId, TraceId};
pub use ring::{append_event, clear_events, recent_events, ObservationRing};
