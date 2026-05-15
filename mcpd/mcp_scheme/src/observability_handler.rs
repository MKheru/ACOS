//! `mcp://observability` — read-only window onto the authority audit ring.
//!
//! Three methods, all `RequiresCapability`:
//! * `recent { limit?: usize }` — last `limit` events (default 50),
//!   newest first.
//! * `by_trace { trace_id: string }` — events whose `trace_id` matches.
//!   `trace_id` is taken as a decimal string because JSON `Number`
//!   cannot losslessly represent `u128`.
//! * `stats` — counts (`current` size, `total_appends`, allow/deny).
//!
//! Because the same audit ring records every dispatch *including* calls
//! into this handler, reading `observability/recent` is itself audited.
//! Operators see exactly when their inspection happened, and against
//! which caller's grant.

use std::sync::Arc;

use serde_json::{json, Value};

use acos_authority_types::{AuditEvent, Verdict};
use mcpd_authority_shim::AuthorityShim;

use crate::handler::{HandlerPolicy, ServiceHandler};
use crate::protocol::{
    JsonRpcRequest, JsonRpcResponse, INVALID_PARAMS, METHOD_NOT_FOUND,
};
use crate::McpPath;

/// Default cap on the number of events returned by `recent` when the
/// caller does not specify a limit.
const DEFAULT_RECENT_LIMIT: usize = 50;

/// Hard ceiling on `recent`'s limit, regardless of caller request. Bounds
/// memory amplification when many callers ask for big slices simultaneously.
const MAX_RECENT_LIMIT: usize = 1000;

/// MCP handler that exposes the [`mcpd_authority_shim::AuditRing`] state.
pub struct ObservabilityHandler {
    shim: Arc<AuthorityShim>,
}

impl ObservabilityHandler {
    /// Construct a handler that reads from `shim`. The handler holds an
    /// `Arc` so the ring can outlive the handler instance.
    pub fn new(shim: Arc<AuthorityShim>) -> Self {
        Self { shim }
    }

    fn event_to_json(event: &AuditEvent) -> Value {
        // u128 trace_id is rendered as decimal string — JSON Number is
        // capped at 2^53. Callers parsing with a u128-aware library can
        // round-trip; humans get an unambiguous identifier.
        json!({
            "trace_id": event.trace_id.to_string(),
            "caller": event.caller,
            "action": event.action,
            "verdict": verdict_label(&event.verdict),
            "latency_us": event.latency_us,
        })
    }
}

fn verdict_label(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Allow => "allow".to_string(),
        Verdict::Deny(code) => format!("deny:{:?}", code),
        _ => "deny:unknown".to_string(),
    }
}

impl ServiceHandler for ObservabilityHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "recent" => {
                let requested = request
                    .params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(DEFAULT_RECENT_LIMIT);
                let limit = requested.min(MAX_RECENT_LIMIT);

                let snap = self.shim.audit_ring().snapshot();
                let total_in_ring = snap.len();
                let total_appends = self.shim.audit_ring().total_appends();

                // newest first, then bounded.
                let events: Vec<Value> = snap
                    .iter()
                    .rev()
                    .take(limit)
                    .map(Self::event_to_json)
                    .collect();

                JsonRpcResponse::success(
                    request.id.clone(),
                    json!({
                        "events": events,
                        "returned": events.len(),
                        "total_in_ring": total_in_ring,
                        "total_appends": total_appends,
                    }),
                )
            }

            "by_trace" => {
                let trace_str = match request.params.get("trace_id").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing 'trace_id' (decimal string)",
                        );
                    }
                };
                let trace_id: u128 = match trace_str.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "'trace_id' must be a decimal u128 string",
                        );
                    }
                };

                let events: Vec<Value> = self
                    .shim
                    .audit_ring()
                    .snapshot()
                    .iter()
                    .filter(|e| e.trace_id == trace_id)
                    .map(Self::event_to_json)
                    .collect();

                JsonRpcResponse::success(
                    request.id.clone(),
                    json!({
                        "trace_id": trace_str,
                        "events": events,
                        "count": events.len(),
                    }),
                )
            }

            "stats" => {
                let snap = self.shim.audit_ring().snapshot();
                let current = snap.len();
                let total_appends = self.shim.audit_ring().total_appends();
                let allow = snap
                    .iter()
                    .filter(|e| matches!(e.verdict, Verdict::Allow))
                    .count();
                let deny = current - allow;

                JsonRpcResponse::success(
                    request.id.clone(),
                    json!({
                        "current_in_ring": current,
                        "total_appends": total_appends,
                        "verdicts": { "allow": allow, "deny": deny },
                    }),
                )
            }

            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in observability service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["recent", "by_trace", "stats"]
    }

    /// Audit visibility is sensitive — even on a single-user box you don't
    /// want anonymous remote peers to enumerate uid/pid call history.
    /// Methods are all `RequiresCapability`; allowlist a uid via
    /// `scheme.capability_policy().allow_uid(uid)` to grant access.
    fn required_policy(&self, _method: &str) -> HandlerPolicy {
        HandlerPolicy::RequiresCapability
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acos_authority_types::{AuditEvent, DenyCode, Verdict};
    use mcpd_authority_shim::AuthorityShim;
    use serde_json::json;

    fn make_request(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id: Some(json!(1)),
        }
    }

    fn path() -> McpPath {
        McpPath::parse(b"observability/recent").unwrap()
    }

    fn shim_with_events(events: Vec<AuditEvent>) -> Arc<AuthorityShim> {
        let shim = Arc::new(AuthorityShim::new());
        for e in events {
            shim.record(e).unwrap();
        }
        shim
    }

    fn ev(trace_id: u128, action: &str, verdict: Verdict) -> AuditEvent {
        AuditEvent::new(trace_id, "test".to_string(), action.to_string(), verdict, None, 0)
    }

    #[test]
    fn declares_requires_capability_for_every_method() {
        let h = ObservabilityHandler::new(Arc::new(AuthorityShim::new()));
        for m in ["recent", "by_trace", "stats"] {
            assert!(matches!(h.required_policy(m), HandlerPolicy::RequiresCapability));
        }
    }

    #[test]
    fn recent_returns_newest_first_within_default_limit() {
        let shim = shim_with_events(vec![
            ev(1, "echo.echo", Verdict::Allow),
            ev(2, "file.read", Verdict::Allow),
            ev(3, "net.http_get", Verdict::Deny(DenyCode::PolicyForbids)),
        ]);
        let h = ObservabilityHandler::new(shim);
        let resp = h.handle(&path(), &make_request("recent", json!({})));
        let result = resp.result.unwrap();
        let events = result["events"].as_array().unwrap();
        // Newest first.
        assert_eq!(events[0]["trace_id"], "3");
        assert_eq!(events[1]["trace_id"], "2");
        assert_eq!(events[2]["trace_id"], "1");
        assert_eq!(result["returned"], 3);
        assert_eq!(result["total_in_ring"], 3);
    }

    #[test]
    fn recent_respects_limit_param() {
        let shim = shim_with_events(vec![
            ev(1, "a", Verdict::Allow),
            ev(2, "b", Verdict::Allow),
            ev(3, "c", Verdict::Allow),
            ev(4, "d", Verdict::Allow),
        ]);
        let h = ObservabilityHandler::new(shim);
        let resp = h.handle(&path(), &make_request("recent", json!({"limit": 2})));
        let result = resp.result.unwrap();
        assert_eq!(result["events"].as_array().unwrap().len(), 2);
        assert_eq!(result["returned"], 2);
        assert_eq!(result["total_in_ring"], 4);
    }

    #[test]
    fn recent_caps_limit_at_max() {
        // Even a huge requested limit cannot exceed MAX_RECENT_LIMIT.
        // We don't pre-populate that many — limit must clamp BEFORE the
        // ring size matters, so a small ring + huge request returns
        // exactly the ring size, not the limit.
        let shim = shim_with_events(vec![ev(1, "a", Verdict::Allow)]);
        let h = ObservabilityHandler::new(shim);
        let resp = h.handle(
            &path(),
            &make_request("recent", json!({"limit": 99999u64})),
        );
        let result = resp.result.unwrap();
        assert_eq!(result["events"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn by_trace_filters_to_one_event() {
        let shim = shim_with_events(vec![
            ev(1, "echo", Verdict::Allow),
            ev(2, "file", Verdict::Allow),
            ev(3, "net", Verdict::Allow),
        ]);
        let h = ObservabilityHandler::new(shim);
        let resp = h.handle(
            &path(),
            &make_request("by_trace", json!({"trace_id": "2"})),
        );
        let result = resp.result.unwrap();
        let events = result["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["action"], "file");
        assert_eq!(result["count"], 1);
    }

    #[test]
    fn by_trace_returns_empty_for_unknown_id() {
        let shim = shim_with_events(vec![ev(1, "a", Verdict::Allow)]);
        let h = ObservabilityHandler::new(shim);
        let resp = h.handle(
            &path(),
            &make_request("by_trace", json!({"trace_id": "99999"})),
        );
        let result = resp.result.unwrap();
        assert_eq!(result["events"].as_array().unwrap().len(), 0);
        assert_eq!(result["count"], 0);
    }

    #[test]
    fn by_trace_rejects_non_decimal_trace_id() {
        let shim = shim_with_events(vec![]);
        let h = ObservabilityHandler::new(shim);
        let resp = h.handle(
            &path(),
            &make_request("by_trace", json!({"trace_id": "abc-xyz"})),
        );
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn stats_counts_allow_and_deny() {
        let shim = shim_with_events(vec![
            ev(1, "a", Verdict::Allow),
            ev(2, "b", Verdict::Allow),
            ev(3, "c", Verdict::Deny(DenyCode::NoMatchingCapability)),
            ev(4, "d", Verdict::Deny(DenyCode::PolicyForbids)),
        ]);
        let h = ObservabilityHandler::new(shim);
        let resp = h.handle(&path(), &make_request("stats", json!({})));
        let result = resp.result.unwrap();
        assert_eq!(result["current_in_ring"], 4);
        assert_eq!(result["total_appends"], 4);
        assert_eq!(result["verdicts"]["allow"], 2);
        assert_eq!(result["verdicts"]["deny"], 2);
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let h = ObservabilityHandler::new(Arc::new(AuthorityShim::new()));
        let resp = h.handle(&path(), &make_request("delete_all", json!({})));
        let err = resp.error.expect("unknown method must error");
        assert_eq!(err.code, METHOD_NOT_FOUND);
    }

    #[test]
    fn list_methods_returns_three_read_only() {
        let h = ObservabilityHandler::new(Arc::new(AuthorityShim::new()));
        let methods = h.list_methods();
        assert_eq!(methods.len(), 3);
        assert!(methods.contains(&"recent"));
        assert!(methods.contains(&"by_trace"));
        assert!(methods.contains(&"stats"));
        // No mutating methods exposed.
        assert!(!methods.contains(&"clear"));
        assert!(!methods.contains(&"delete"));
        assert!(!methods.contains(&"write"));
    }
}
