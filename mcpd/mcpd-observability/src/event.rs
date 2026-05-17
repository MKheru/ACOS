//! Observation event model for ACOS MCP ingress traces.

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TRACE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_SPAN_ID: AtomicU64 = AtomicU64::new(1);

/// Correlates all events emitted while routing one MCP request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraceId(pub u64);

impl TraceId {
    /// Allocate a monotonically increasing in-process trace id.
    pub fn next() -> Self {
        Self(NEXT_TRACE_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Identifies one span inside a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpanId(pub u64);

impl SpanId {
    /// Allocate a monotonically increasing in-process span id.
    pub fn next() -> Self {
        Self(NEXT_SPAN_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Kind of observation event emitted by router ingress tracing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationKind {
    Intent,
    Result,
    Anomaly,
}

/// Final status recorded for a routed MCP request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    Ok,
    Error,
    IncompleteOrPanic,
}

/// One append-only observation emitted by mcpd routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationEvent {
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub kind: ObservationKind,
    pub service: String,
    pub method: String,
    pub latency_us: Option<u64>,
    pub error_code: Option<i64>,
    pub status: Option<ResultStatus>,
    pub anomaly_type: Option<String>,
    pub severity: Option<String>,
    pub description: Option<String>,
    pub details_json: Option<String>,
}

impl ObservationEvent {
    pub fn intent(trace_id: TraceId, span_id: SpanId, service: &str, method: &str) -> Self {
        Self {
            trace_id,
            span_id,
            kind: ObservationKind::Intent,
            service: service.to_string(),
            method: method.to_string(),
            latency_us: None,
            error_code: None,
            status: None,
            anomaly_type: None,
            severity: None,
            description: None,
            details_json: None,
        }
    }

    pub fn result(
        trace_id: TraceId,
        span_id: SpanId,
        service: &str,
        method: &str,
        status: ResultStatus,
        latency_us: u64,
        error_code: Option<i64>,
    ) -> Self {
        Self {
            trace_id,
            span_id,
            kind: ObservationKind::Result,
            service: service.to_string(),
            method: method.to_string(),
            latency_us: Some(latency_us),
            error_code,
            status: Some(status),
            anomaly_type: None,
            severity: None,
            description: None,
            details_json: None,
        }
    }

    pub fn anomaly(
        trace_id: TraceId,
        span_id: SpanId,
        anomaly_type: &str,
        severity: &str,
        description: &str,
        details_json: String,
    ) -> Self {
        Self {
            trace_id,
            span_id,
            kind: ObservationKind::Anomaly,
            service: "guardian".to_string(),
            method: "anomaly".to_string(),
            latency_us: None,
            error_code: None,
            status: None,
            anomaly_type: Some(anomaly_type.to_string()),
            severity: Some(severity.to_string()),
            description: Some(description.to_string()),
            details_json: Some(details_json),
        }
    }
}
