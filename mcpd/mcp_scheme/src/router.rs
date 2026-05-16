//! MCP message router — dispatches requests to registered service handlers
//! WS11.M2 trace instrumentation lives in this module.

use std::cell::RefCell;
use std::time::Instant;

use acos_authority_types::{CallerContext, Verdict};
use mcpd_authority_shim::CapabilityPolicy;
use mcpd_observability::{append_event, ObservationEvent, ResultStatus, SpanId, TraceId};
use rustc_hash::FxHashMap;

use crate::handler::{HandlerPolicy, ServiceHandler};
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, DENIED_BY_POLICY, METHOD_NOT_FOUND};
use crate::McpPath;

// ---------------------------------------------------------------------------
// WS2.M3 phase 3 — caller propagation across internal (DispatchHub) calls.
//
// When a handler invokes another service through `DispatchHub`, the
// originating caller's identity must travel with the call: a method
// declared `RequiresCapability` would otherwise be unreachable from any
// internal dispatcher (which has no caller of its own). The router
// installs the caller in a thread-local before invoking the handler;
// `DispatchHub::dispatch` reads it and forwards it into the nested
// `route_with_caller` call.
//
// A RAII guard restores the previous value on Drop, so nested
// invocations (A → B → C) save/restore correctly even across panics.
// ---------------------------------------------------------------------------

thread_local! {
    static CURRENT_CALLER: RefCell<Option<CallerContext>> = const { RefCell::new(None) };
}

/// Snapshot the caller currently set by the surrounding [`Router::route_with_caller`]
/// call, if any. `DispatchHub` uses this to thread the originating
/// identity into nested service calls.
pub fn current_caller() -> Option<CallerContext> {
    CURRENT_CALLER.with(|c| *c.borrow())
}

/// RAII guard that installs `caller` as the current thread-local and
/// restores the previous value on drop — including on panic.
pub(crate) struct CallerGuard {
    prev: Option<CallerContext>,
}

impl CallerGuard {
    pub(crate) fn new(caller: Option<CallerContext>) -> Self {
        let prev = CURRENT_CALLER.with(|c| {
            let old = *c.borrow();
            *c.borrow_mut() = caller;
            old
        });
        Self { prev }
    }
}

impl Drop for CallerGuard {
    fn drop(&mut self) {
        let prev = self.prev;
        CURRENT_CALLER.with(|c| *c.borrow_mut() = prev);
    }
}

/// WS11.M2 — RAII ingress trace for one routed JSON-RPC request.
///
/// Construction appends an `Intent`; `record_result` appends the matching
/// `Result`. If a handler panics or a future early-return path forgets to
/// record, `Drop` emits `IncompleteOrPanic` so the trace never hangs silently.
pub(crate) struct TraceGuard {
    trace_id: TraceId,
    span_id: SpanId,
    service: String,
    method: String,
    started_at: Instant,
    completed: bool,
}

impl TraceGuard {
    pub(crate) fn new(path: &McpPath, request: &JsonRpcRequest) -> Self {
        let trace_id = TraceId::next();
        let span_id = SpanId::next();
        append_event(ObservationEvent::intent(
            trace_id,
            span_id,
            &path.service,
            &request.method,
        ));
        Self {
            trace_id,
            span_id,
            service: path.service.clone(),
            method: request.method.clone(),
            started_at: Instant::now(),
            completed: false,
        }
    }

    pub(crate) fn record_result(&mut self, response: &JsonRpcResponse) {
        if self.completed {
            return;
        }
        let (status, error_code) = match response.error.as_ref() {
            Some(error) => (ResultStatus::Error, Some(error.code)),
            None => (ResultStatus::Ok, None),
        };
        self.emit_result(status, error_code);
    }

    fn emit_result(&mut self, status: ResultStatus, error_code: Option<i64>) {
        self.completed = true;
        let latency_us = self
            .started_at
            .elapsed()
            .as_micros()
            .min(u128::from(u64::MAX)) as u64;
        append_event(ObservationEvent::result(
            self.trace_id,
            self.span_id,
            &self.service,
            &self.method,
            status,
            latency_us,
            error_code,
        ));
    }
}

impl Drop for TraceGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.emit_result(ResultStatus::IncompleteOrPanic, None);
        }
    }
}

/// Routes MCP requests to the appropriate service handler
pub struct Router {
    services: FxHashMap<String, Box<dyn ServiceHandler>>,
}

impl Router {
    pub fn new() -> Self {
        Router {
            services: FxHashMap::default(),
        }
    }

    /// Register a service handler
    pub fn register(&mut self, name: &str, handler: impl ServiceHandler + 'static) {
        self.services.insert(name.to_string(), Box::new(handler));
    }

    /// Check if a service is registered
    pub fn has_service(&self, name: &str) -> bool {
        self.services.contains_key(name)
    }

    /// List all registered service names
    pub fn list_services(&self) -> Vec<&str> {
        self.services.keys().map(|s| s.as_str()).collect()
    }

    /// Register a service handler dynamically (takes ownership of handler)
    pub fn register_service(&mut self, name: &str, handler: Box<dyn ServiceHandler>) {
        self.services.insert(name.to_string(), handler);
    }

    /// Unregister a service handler by name
    pub fn unregister_service(&mut self, name: &str) -> bool {
        self.services.remove(name).is_some()
    }

    /// Route a request to the appropriate handler.
    ///
    /// Back-compat entry: no caller identity, no capability policy.
    /// Equivalent to [`Router::route_with_caller`] with both `None`.
    /// Used by `DispatchHub` for internal service-to-service calls and
    /// by unit tests that do not exercise the capability layer.
    pub fn route(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        self.route_with_caller(path, request, None, None)
    }

    /// WS2.M3 (phase 2) — Route a request with full authority context.
    ///
    /// The decision flow:
    /// 1. Service lookup — `METHOD_NOT_FOUND` on miss.
    /// 2. Handler declares `required_policy(method)`.
    ///    - `Public` → dispatch.
    ///    - `RequiresCapability` → consult `policy.can_invoke(caller, ...)`
    ///      if both are provided. Deny when caller or policy is missing
    ///      (cannot evaluate without both).
    ///    - `GuardianOnly` → consult `policy.can_invoke_guardian_only(...)`
    ///      under the same provided-both rule.
    /// 3. `Verdict::Allow` → dispatch; `Verdict::Deny(...)` → return
    ///    `DENIED_BY_POLICY` with the reason embedded in the message.
    pub fn route_with_caller(
        &self,
        path: &McpPath,
        request: &JsonRpcRequest,
        caller: Option<&CallerContext>,
        policy: Option<&CapabilityPolicy>,
    ) -> JsonRpcResponse {
        let mut trace = TraceGuard::new(path, request);

        let handler = match self.services.get(&path.service) {
            Some(h) => h,
            None => {
                let response = JsonRpcResponse::error(
                    request.id.clone(),
                    METHOD_NOT_FOUND,
                    format!("Service '{}' not found", path.service),
                );
                trace.record_result(&response);
                return response;
            }
        };

        // WS2.M3 phase 3 — install the caller in a thread-local so any
        // `DispatchHub::dispatch` call made by the handler sees the
        // originating identity (and not an anonymous default).
        let _guard = CallerGuard::new(caller.copied());

        let response = match handler.required_policy(&request.method) {
            HandlerPolicy::Public => handler.handle(path, request),

            HandlerPolicy::RequiresCapability => {
                let verdict = match (caller, policy) {
                    (Some(c), Some(p)) => p.can_invoke(c, &path.service, &request.method),
                    _ => Verdict::Deny(acos_authority_types::DenyCode::NoMatchingCapability),
                };
                match verdict {
                    Verdict::Allow => handler.handle(path, request),
                    Verdict::Deny(code) => JsonRpcResponse::error(
                        request.id.clone(),
                        DENIED_BY_POLICY,
                        format!(
                            "method '{}.{}' denied: {:?}",
                            path.service, request.method, code
                        ),
                    ),
                    _ => JsonRpcResponse::error(
                        request.id.clone(),
                        DENIED_BY_POLICY,
                        format!(
                            "method '{}.{}' denied: unrecognised verdict",
                            path.service, request.method
                        ),
                    ),
                }
            }

            HandlerPolicy::GuardianOnly => {
                let verdict = match (caller, policy) {
                    (Some(c), Some(p)) => {
                        p.can_invoke_guardian_only(c, &path.service, &request.method)
                    }
                    _ => Verdict::Deny(acos_authority_types::DenyCode::BootstrapScopeViolation),
                };
                match verdict {
                    Verdict::Allow => handler.handle(path, request),
                    Verdict::Deny(code) => JsonRpcResponse::error(
                        request.id.clone(),
                        DENIED_BY_POLICY,
                        format!(
                            "method '{}.{}' (GuardianOnly) denied: {:?}",
                            path.service, request.method, code
                        ),
                    ),
                    _ => JsonRpcResponse::error(
                        request.id.clone(),
                        DENIED_BY_POLICY,
                        format!(
                            "method '{}.{}' (GuardianOnly) denied: unrecognised verdict",
                            path.service, request.method
                        ),
                    ),
                }
            }

            // HandlerPolicy is `#[non_exhaustive]`. Any variant added in the
            // future is denied here until the router is updated to handle it.
            _ => JsonRpcResponse::error(
                request.id.clone(),
                DENIED_BY_POLICY,
                format!(
                    "method '{}.{}' has an unrecognised policy variant",
                    path.service, request.method
                ),
            ),
        };
        trace.record_result(&response);
        response
    }

    /// Dispatch a request to a service by name (internal use, avoids deadlock)
    pub fn dispatch(
        &self,
        service: &str,
        method: &str,
        params: serde_json::Value,
    ) -> JsonRpcResponse {
        let path = McpPath {
            service: service.to_string(),
            resource: Vec::new(),
        };
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: method.to_string(),
            params,
            id: None,
        };
        self.route(&path, &request)
    }
}

// ---------------------------------------------------------------------------
// Tests — WS1.M3 (required_policy default) + WS1.M4 (router denies non-public)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};

    fn req(method: &str) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params: json!({}),
            id: Some(json!(1)),
        }
    }

    fn p(svc: &str) -> McpPath {
        McpPath::parse(svc.as_bytes()).unwrap()
    }

    // -- Test handler that lets us pin each policy variant ---------------------

    struct PolicyTestHandler;
    impl ServiceHandler for PolicyTestHandler {
        fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
            JsonRpcResponse::success(request.id.clone(), json!({"called": request.method}))
        }
        fn list_methods(&self) -> Vec<&str> {
            vec!["pub_method", "cap_method", "guardian_method"]
        }
        fn required_policy(&self, method: &str) -> HandlerPolicy {
            match method {
                "cap_method" => HandlerPolicy::RequiresCapability,
                "guardian_method" => HandlerPolicy::GuardianOnly,
                _ => HandlerPolicy::Public,
            }
        }
    }

    fn router_with_policy_handler() -> Router {
        let mut r = Router::new();
        r.register("polytest", PolicyTestHandler);
        r
    }

    fn trace_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn trace_events_for(service: &str, method: &str) -> Vec<mcpd_observability::ObservationEvent> {
        mcpd_observability::recent_events(1024)
            .into_iter()
            .filter(|event| event.service == service && event.method == method)
            .collect()
    }

    // -- WS11.M2 — Router ingress TraceGuard -----------------------------------

    #[test]
    fn trace_intent_and_result_for_echo() {
        let _lock = trace_test_lock().lock().unwrap();
        mcpd_observability::clear_events();
        let mut r = Router::new();
        r.register("trace_echo", crate::handler::EchoHandler::new());

        let resp = r.route(&p("trace_echo"), &req("echo"));
        assert!(resp.error.is_none(), "echo must succeed: {:?}", resp.error);

        let events = trace_events_for("trace_echo", "echo");
        assert_eq!(events.len(), 2, "expected Intent + Result, got {events:?}");
        assert_eq!(events[0].kind, mcpd_observability::ObservationKind::Intent);
        assert_eq!(events[1].kind, mcpd_observability::ObservationKind::Result);
        assert_eq!(events[0].trace_id, events[1].trace_id);
        assert_eq!(events[0].span_id, events[1].span_id);
        assert_eq!(events[1].status, Some(mcpd_observability::ResultStatus::Ok));
        assert!(events[1].latency_us.is_some());
        assert_eq!(events[1].error_code, None);
    }

    #[test]
    fn trace_result_records_error_code() {
        let _lock = trace_test_lock().lock().unwrap();
        mcpd_observability::clear_events();
        let r = Router::new();

        let resp = r.route(&p("trace_missing"), &req("echo"));
        let err = resp.error.expect("missing service must error");
        assert_eq!(err.code, METHOD_NOT_FOUND);

        let events = trace_events_for("trace_missing", "echo");
        assert_eq!(events.len(), 2, "expected Intent + Result, got {events:?}");
        assert_eq!(events[1].kind, mcpd_observability::ObservationKind::Result);
        assert_eq!(
            events[1].status,
            Some(mcpd_observability::ResultStatus::Error)
        );
        assert_eq!(events[1].error_code, Some(METHOD_NOT_FOUND));
        assert!(events[1].latency_us.is_some());
    }

    #[test]
    fn panic_in_handler_emits_incomplete_result() {
        let _lock = trace_test_lock().lock().unwrap();
        mcpd_observability::clear_events();

        struct PanicHandler;
        impl ServiceHandler for PanicHandler {
            fn handle(&self, _path: &McpPath, _request: &JsonRpcRequest) -> JsonRpcResponse {
                panic!("intentional WS11.M2 panic probe");
            }

            fn list_methods(&self) -> Vec<&str> {
                vec!["explode"]
            }
        }

        let mut r = Router::new();
        r.register("trace_panic", PanicHandler);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = r.route(&p("trace_panic"), &req("explode"));
        }));
        assert!(outcome.is_err(), "panic probe should still unwind");

        let events = trace_events_for("trace_panic", "explode");
        let result = events
            .iter()
            .find(|event| event.kind == mcpd_observability::ObservationKind::Result)
            .expect("panic trace must include an incomplete Result event");
        assert_eq!(
            result.status,
            Some(mcpd_observability::ResultStatus::IncompleteOrPanic)
        );
        assert_eq!(result.error_code, None);
        assert!(result.latency_us.is_some());
    }

    // -- WS1.M3 — default policy is Public -------------------------------------

    #[test]
    fn ws1m3_default_policy_is_public_for_unmodified_handlers() {
        // Use the built-in EchoHandler which does NOT override required_policy.
        let mut r = Router::new();
        r.register("echo", crate::handler::EchoHandler::new());
        let resp = r.route(&p("echo"), &req("echo"));
        assert!(
            resp.error.is_none(),
            "default Public must route normally; got {:?}",
            resp.error
        );
        assert!(resp.result.is_some());
    }

    // -- WS1.M4 — router denies non-Public policies ----------------------------

    #[test]
    fn ws1m4_public_method_dispatches_through_policy_handler() {
        let r = router_with_policy_handler();
        let resp = r.route(&p("polytest"), &req("pub_method"));
        assert!(
            resp.error.is_none(),
            "pub_method must succeed; got {:?}",
            resp.error
        );
        assert_eq!(resp.result.unwrap()["called"], "pub_method");
    }

    #[test]
    fn ws1m4_requires_capability_returns_denied_by_policy() {
        let r = router_with_policy_handler();
        let resp = r.route(&p("polytest"), &req("cap_method"));
        let err = resp.error.expect("RequiresCapability must deny");
        assert_eq!(err.code, DENIED_BY_POLICY);
        // WS2.M3 phase 2 — message changed to include the DenyCode variant
        // (NoMatchingCapability). Match case-insensitively so the test
        // does not pin one phrasing forever.
        assert!(
            err.message.to_lowercase().contains("capability"),
            "expected 'capability' in error message; got '{}'",
            err.message
        );
    }

    #[test]
    fn ws1m4_guardian_only_returns_denied_by_policy() {
        let r = router_with_policy_handler();
        let resp = r.route(&p("polytest"), &req("guardian_method"));
        let err = resp.error.expect("GuardianOnly must deny");
        assert_eq!(err.code, DENIED_BY_POLICY);
        assert!(err.message.contains("Guardian"));
    }

    #[test]
    fn ws1m4_unknown_service_still_returns_method_not_found_not_denied() {
        // The new policy check must not shadow the existing service-not-found path.
        let r = router_with_policy_handler();
        let resp = r.route(&p("nonexistent"), &req("anything"));
        let err = resp.error.expect("unknown service must error");
        assert_eq!(err.code, METHOD_NOT_FOUND);
        assert_ne!(err.code, DENIED_BY_POLICY);
    }

    // -- WS2.M3 phase 2 — caller-aware policy decisions ---------------------

    #[test]
    fn ws2m3_route_with_caller_denies_anonymous_on_requires_capability() {
        let r = router_with_policy_handler();
        let policy = CapabilityPolicy::new();
        policy.allow_uid(1000); // even with an allowlisted uid, anonymous loses
        let anon = CallerContext::anonymous();
        let resp = r.route_with_caller(
            &p("polytest"),
            &req("cap_method"),
            Some(&anon),
            Some(&policy),
        );
        let err = resp.error.expect("anonymous caller must be denied");
        assert_eq!(err.code, DENIED_BY_POLICY);
        assert!(err.message.to_lowercase().contains("capability"));
    }

    #[test]
    fn ws2m3_route_with_caller_denies_when_uid_not_allowlisted() {
        let r = router_with_policy_handler();
        let policy = CapabilityPolicy::new();
        // Empty allowlist — no uid is granted.
        let caller = CallerContext::from_parts(1000, 100, 42);
        let resp = r.route_with_caller(
            &p("polytest"),
            &req("cap_method"),
            Some(&caller),
            Some(&policy),
        );
        assert_eq!(
            resp.error.expect("uid not allowlisted must be denied").code,
            DENIED_BY_POLICY
        );
    }

    #[test]
    fn ws2m3_route_with_caller_allows_when_uid_is_allowlisted() {
        let r = router_with_policy_handler();
        let policy = CapabilityPolicy::new();
        policy.allow_uid(1000);
        let caller = CallerContext::from_parts(1000, 100, 42);
        let resp = r.route_with_caller(
            &p("polytest"),
            &req("cap_method"),
            Some(&caller),
            Some(&policy),
        );
        assert!(
            resp.error.is_none(),
            "allowlisted uid must reach handler; got {:?}",
            resp.error
        );
        assert_eq!(resp.result.unwrap()["called"], "cap_method");
    }

    #[test]
    fn ws2m3_route_with_caller_denies_when_caller_or_policy_missing() {
        let r = router_with_policy_handler();
        let caller = CallerContext::from_parts(1000, 100, 42);
        let policy = CapabilityPolicy::new();
        policy.allow_uid(1000);
        // Missing policy.
        assert_eq!(
            r.route_with_caller(&p("polytest"), &req("cap_method"), Some(&caller), None)
                .error
                .unwrap()
                .code,
            DENIED_BY_POLICY
        );
        // Missing caller.
        assert_eq!(
            r.route_with_caller(&p("polytest"), &req("cap_method"), None, Some(&policy))
                .error
                .unwrap()
                .code,
            DENIED_BY_POLICY
        );
    }

    #[test]
    fn ws2m3_route_guardian_only_requires_scope_and_uid() {
        let r = router_with_policy_handler();
        let policy = CapabilityPolicy::new();
        let caller = CallerContext::from_parts(0, 0, 1);

        // Scope inactive, uid not allowlisted → deny.
        let resp = r.route_with_caller(
            &p("polytest"),
            &req("guardian_method"),
            Some(&caller),
            Some(&policy),
        );
        assert_eq!(resp.error.unwrap().code, DENIED_BY_POLICY);

        // Scope active alone → still deny (uid not allowlisted).
        policy.activate_guardian();
        let resp = r.route_with_caller(
            &p("polytest"),
            &req("guardian_method"),
            Some(&caller),
            Some(&policy),
        );
        assert_eq!(resp.error.unwrap().code, DENIED_BY_POLICY);

        // Both present → allow.
        policy.allow_uid(0);
        let resp = r.route_with_caller(
            &p("polytest"),
            &req("guardian_method"),
            Some(&caller),
            Some(&policy),
        );
        assert!(
            resp.error.is_none(),
            "GuardianOnly must allow when scope+uid match; got {:?}",
            resp.error
        );
    }

    // -- WS2.M3 phase 3 — CallerGuard thread-local mechanics ---------------

    #[test]
    fn caller_guard_sets_and_restores_thread_local() {
        // No caller in scope by default.
        assert!(current_caller().is_none());
        {
            let _g = CallerGuard::new(Some(CallerContext::from_parts(1000, 100, 42)));
            assert_eq!(current_caller().map(|c| c.uid), Some(1000));
        }
        // Guard dropped → back to None.
        assert!(current_caller().is_none());
    }

    #[test]
    fn caller_guard_nested_saves_and_restores_correctly() {
        let outer = CallerContext::from_parts(1000, 100, 1);
        let inner = CallerContext::from_parts(1001, 100, 2);
        {
            let _g_outer = CallerGuard::new(Some(outer));
            assert_eq!(current_caller().map(|c| c.uid), Some(1000));
            {
                let _g_inner = CallerGuard::new(Some(inner));
                assert_eq!(current_caller().map(|c| c.uid), Some(1001));
            }
            // Inner dropped → restored to outer.
            assert_eq!(current_caller().map(|c| c.uid), Some(1000));
        }
        // Both dropped → None.
        assert!(current_caller().is_none());
    }

    #[test]
    fn caller_guard_overrides_previous_some_with_none() {
        // Some(X) → None → Some(X) restoration.
        let outer = CallerContext::from_parts(1000, 100, 1);
        let _g_outer = CallerGuard::new(Some(outer));
        {
            let _g_inner = CallerGuard::new(None);
            assert!(current_caller().is_none());
        }
        assert_eq!(current_caller().map(|c| c.uid), Some(1000));
    }

    #[test]
    fn route_with_caller_installs_caller_into_thread_local_during_handle() {
        // Use a custom handler that reads `current_caller()` and embeds
        // the uid into its response — proves the router set the
        // thread-local before invoking handle.
        struct CallerReadingHandler;
        impl ServiceHandler for CallerReadingHandler {
            fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
                let uid = current_caller().map(|c| c.uid).unwrap_or(u32::MAX);
                JsonRpcResponse::success(request.id.clone(), json!({"observed_uid": uid}))
            }
            fn list_methods(&self) -> Vec<&str> {
                vec!["whoami"]
            }
        }

        let mut r = Router::new();
        r.register("readcaller", CallerReadingHandler);
        let caller = CallerContext::from_parts(1000, 100, 42);
        let resp = r.route_with_caller(&p("readcaller"), &req("whoami"), Some(&caller), None);
        assert_eq!(resp.result.unwrap()["observed_uid"], 1000);
        // After the call, thread-local is back to its prior state.
        assert!(current_caller().is_none());
    }

    #[test]
    fn ws2m3_public_method_ignores_caller_and_policy() {
        // Public methods short-circuit before any policy check —
        // anonymous caller + empty policy must still get through.
        let r = router_with_policy_handler();
        let policy = CapabilityPolicy::new();
        let anon = CallerContext::anonymous();
        let resp = r.route_with_caller(
            &p("polytest"),
            &req("pub_method"),
            Some(&anon),
            Some(&policy),
        );
        assert!(
            resp.error.is_none(),
            "Public method must be unconditional; got {:?}",
            resp.error
        );
    }

    #[test]
    fn ws1m4_policy_check_runs_before_handler_handle() {
        // If router consulted required_policy *after* handle(), the handler's
        // success response would leak through. Prove the order: cap_method
        // returns success body when called directly, but DENIED via router.
        let handler = PolicyTestHandler;
        let direct = handler.handle(&p("polytest"), &req("cap_method"));
        assert!(
            direct.result.is_some(),
            "handler itself returns ok for cap_method"
        );

        let r = router_with_policy_handler();
        let routed = r.route(&p("polytest"), &req("cap_method"));
        assert!(
            routed.result.is_none(),
            "router must short-circuit cap_method"
        );
        assert_eq!(routed.error.unwrap().code, DENIED_BY_POLICY);
    }
}
