//! MCP message router — dispatches requests to registered service handlers

use rustc_hash::FxHashMap;

use crate::handler::{HandlerPolicy, ServiceHandler};
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, DENIED_BY_POLICY, METHOD_NOT_FOUND};
use crate::McpPath;

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
    /// WS1.M4 — Before dispatching, consult the handler's `required_policy`
    /// for the requested method. If the policy is not [`HandlerPolicy::Public`]
    /// we refuse with `DENIED_BY_POLICY`: the capability/scope check that
    /// would *grant* a non-public policy lives in `mcpd-authority-shim`,
    /// which is not yet wired in. Until then, declaring a method as
    /// `RequiresCapability` or `GuardianOnly` is the explicit way to keep
    /// it un-reachable from untrusted callers.
    pub fn route(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        let handler = match self.services.get(&path.service) {
            Some(h) => h,
            None => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    METHOD_NOT_FOUND,
                    format!("Service '{}' not found", path.service),
                );
            }
        };

        match handler.required_policy(&request.method) {
            HandlerPolicy::Public => handler.handle(path, request),
            HandlerPolicy::RequiresCapability => JsonRpcResponse::error(
                request.id.clone(),
                DENIED_BY_POLICY,
                format!(
                    "method '{}.{}' requires a capability; authority shim not yet wired",
                    path.service, request.method
                ),
            ),
            HandlerPolicy::GuardianOnly => JsonRpcResponse::error(
                request.id.clone(),
                DENIED_BY_POLICY,
                format!(
                    "method '{}.{}' is reserved for the Guardian",
                    path.service, request.method
                ),
            ),
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
        }
    }

    /// Dispatch a request to a service by name (internal use, avoids deadlock)
    pub fn dispatch(&self, service: &str, method: &str, params: serde_json::Value) -> JsonRpcResponse {
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

    // -- WS1.M3 — default policy is Public -------------------------------------

    #[test]
    fn ws1m3_default_policy_is_public_for_unmodified_handlers() {
        // Use the built-in EchoHandler which does NOT override required_policy.
        let mut r = Router::new();
        r.register("echo", crate::handler::EchoHandler::new());
        let resp = r.route(&p("echo"), &req("echo"));
        assert!(resp.error.is_none(), "default Public must route normally; got {:?}", resp.error);
        assert!(resp.result.is_some());
    }

    // -- WS1.M4 — router denies non-Public policies ----------------------------

    #[test]
    fn ws1m4_public_method_dispatches_through_policy_handler() {
        let r = router_with_policy_handler();
        let resp = r.route(&p("polytest"), &req("pub_method"));
        assert!(resp.error.is_none(), "pub_method must succeed; got {:?}", resp.error);
        assert_eq!(resp.result.unwrap()["called"], "pub_method");
    }

    #[test]
    fn ws1m4_requires_capability_returns_denied_by_policy() {
        let r = router_with_policy_handler();
        let resp = r.route(&p("polytest"), &req("cap_method"));
        let err = resp.error.expect("RequiresCapability must deny");
        assert_eq!(err.code, DENIED_BY_POLICY);
        assert!(err.message.contains("capability"));
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

    #[test]
    fn ws1m4_policy_check_runs_before_handler_handle() {
        // If router consulted required_policy *after* handle(), the handler's
        // success response would leak through. Prove the order: cap_method
        // returns success body when called directly, but DENIED via router.
        let handler = PolicyTestHandler;
        let direct = handler.handle(&p("polytest"), &req("cap_method"));
        assert!(direct.result.is_some(), "handler itself returns ok for cap_method");

        let r = router_with_policy_handler();
        let routed = r.route(&p("polytest"), &req("cap_method"));
        assert!(routed.result.is_none(), "router must short-circuit cap_method");
        assert_eq!(routed.error.unwrap().code, DENIED_BY_POLICY);
    }
}
