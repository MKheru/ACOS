//! [`EventKind`] — Intent vs Result phase of a dual-phase audit pair.
//!
//! Every authoritative dispatch emits two events that share the same
//! `trace_id`:
//! * [`EventKind::Intent`] before the handler runs — "X was about to
//!   happen", verdict is unknown / placeholder.
//! * [`EventKind::Result`] after the handler returns (or panics) —
//!   carries the real [`crate::Verdict`] and the measured `latency_us`.
//!
//! A missing `Result` for a known `Intent` means the dispatch hung,
//! panicked, or was aborted before completing, and is itself an
//! anomaly worth surfacing.

/// Phase tag attached to every [`crate::AuditEvent`].
///
/// `Result` is the default for backward-compatible single-phase
/// emitters (pre-WS3.M8 code paths and unit-test fixtures that build
/// events ad hoc).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum EventKind {
    /// "About to dispatch" — emitted before `Handler::handle` is called.
    /// The paired [`EventKind::Result`] is the source of truth for the
    /// verdict; `Intent.verdict` is a placeholder.
    Intent,
    /// "Dispatch returned" — carries the real verdict and the measured
    /// latency. Pair this with the preceding `Intent` via `trace_id`.
    Result,
}

impl Default for EventKind {
    fn default() -> Self {
        EventKind::Result
    }
}

impl EventKind {
    /// Stable lowercase label for serialization (e.g. JSON output).
    pub fn label(&self) -> &'static str {
        match self {
            EventKind::Intent => "intent",
            EventKind::Result => "result",
            _ => "unknown",
        }
    }
}
