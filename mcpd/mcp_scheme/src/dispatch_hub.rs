//! DispatchHub — safe binding of inter-service dispatch callbacks.
//!
//! Solves the chicken-and-egg of `Arc<Router>` ↔ handlers that need to
//! dispatch back into the Router. Handlers receive a clonable `DispatchHub`
//! at construction time; the hub is bound to the Router via a `Weak`
//! reference *after* the Router is wrapped in `Arc`. Dispatches before
//! binding fail-closed with `INTERNAL_ERROR` rather than panic.
//!
//! Replaces the prior `unsafe { (*router_ptr).register(...) }` pattern that
//! cast an `Arc<Router>` interior to `*mut Router` and mutated through it
//! after the `Arc` had been cloned. That construction relied on "single-
//! threaded init" assumptions that the type system could not enforce.

use std::sync::{Arc, RwLock, Weak};

use mcpd_authority_shim::CapabilityPolicy;
use serde_json::Value;

use crate::protocol::{JsonRpcResponse, INTERNAL_ERROR};
use crate::router::{current_caller, Router};

/// Clonable dispatcher backed by a `Weak<Router>` set after construction.
///
/// Cloning is cheap (shared `Arc<RwLock<...>>` handle). All clones see the
/// same binding, so binding once after construction suffices for every
/// handler that captured a clone.
///
/// WS2.M3 phase 3 — also carries an optional `Weak<CapabilityPolicy>`
/// bound by [`DispatchHub::bind_policy`]. When present, internal
/// dispatches forward both the thread-local caller (set by the
/// surrounding `Router::route_with_caller` call) and the policy into
/// the nested route, so a `RequiresCapability` method invoked across
/// services is evaluated against the originating identity instead of
/// failing for lack of a caller.
#[derive(Clone)]
pub struct DispatchHub {
    inner: Arc<RwLock<Option<Weak<Router>>>>,
    policy: Arc<RwLock<Option<Weak<CapabilityPolicy>>>>,
}

impl DispatchHub {
    /// Create an unbound hub.
    ///
    /// Dispatches will fail-closed with `INTERNAL_ERROR` until [`bind`] is
    /// called with a concrete `Arc<Router>`.
    ///
    /// [`bind`]: DispatchHub::bind
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(None)),
            policy: Arc::new(RwLock::new(None)),
        }
    }

    /// Bind the hub to a router so subsequent dispatches reach it.
    ///
    /// Stores a [`Weak`] reference to avoid creating a strong cycle
    /// `Router → Handler → DispatchHub → Router`. The caller retains the
    /// `Arc<Router>` for the lifetime of the process.
    pub fn bind(&self, router: &Arc<Router>) {
        // The lock is held only for the duration of the assignment. We swallow
        // a poisoned lock by overwriting; poisoning here would mean a prior
        // panic during bind, and we want startup to recover.
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|p| p.into_inner());
        *guard = Some(Arc::downgrade(router));
    }

    /// WS2.M3 phase 3 — Bind the hub to a [`CapabilityPolicy`] so
    /// internal dispatches can evaluate `RequiresCapability` methods
    /// against the originating caller (read from the router's
    /// thread-local) instead of failing for lack of a policy.
    ///
    /// Optional: if unbound, internal dispatches still work for
    /// `Public` methods (the historical behaviour); `RequiresCapability`
    /// methods will deny — same as before this phase.
    pub fn bind_policy(&self, policy: &Arc<CapabilityPolicy>) {
        let mut guard = self
            .policy
            .write()
            .unwrap_or_else(|p| p.into_inner());
        *guard = Some(Arc::downgrade(policy));
    }

    /// Dispatch a service+method call through the bound router.
    ///
    /// Returns `INTERNAL_ERROR` if:
    /// - the hub has not yet been bound,
    /// - the router has been dropped (`Weak::upgrade` fails),
    /// - the internal lock is poisoned.
    ///
    /// WS2.M3 phase 3 — Reads the thread-local caller installed by the
    /// surrounding `Router::route_with_caller` (if any) and forwards
    /// it alongside the bound policy (if any). This makes
    /// `RequiresCapability` methods reachable from internal callers
    /// that hold the originating identity.
    ///
    /// Never panics.
    pub fn dispatch(&self, service: &str, method: &str, params: Value) -> JsonRpcResponse {
        // Acquire the read lock, clone the Weak, and release the lock before
        // calling router.dispatch() to avoid holding it across an outbound
        // call that might recurse into another DispatchHub method.
        let weak_router = {
            let guard = match self.inner.read() {
                Ok(g) => g,
                Err(_) => {
                    return JsonRpcResponse::error(
                        None,
                        INTERNAL_ERROR,
                        "DispatchHub lock poisoned".to_string(),
                    );
                }
            };
            match guard.as_ref() {
                Some(w) => w.clone(),
                None => {
                    return JsonRpcResponse::error(
                        None,
                        INTERNAL_ERROR,
                        "DispatchHub not bound".to_string(),
                    );
                }
            }
        };
        let weak_policy = match self.policy.read() {
            Ok(g) => g.as_ref().cloned(),
            Err(_) => None,
        };

        let router = match weak_router.upgrade() {
            Some(r) => r,
            None => {
                return JsonRpcResponse::error(
                    None,
                    INTERNAL_ERROR,
                    "Router dropped before DispatchHub call".to_string(),
                );
            }
        };
        let policy = weak_policy.and_then(|w| w.upgrade());
        let caller = current_caller();

        // Build a synthetic McpPath + JsonRpcRequest, mirroring what
        // `Router::dispatch` did, but call `route_with_caller` directly
        // so caller + policy actually reach the decision point.
        use crate::protocol::JsonRpcRequest;
        use crate::McpPath;
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
        router.route_with_caller(&path, &request, caller.as_ref(), policy.as_deref())
    }
}

impl Default for DispatchHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dispatch_before_bind_returns_internal_error() {
        let hub = DispatchHub::new();
        let resp = hub.dispatch("echo", "echo", json!({"text": "hi"}));
        assert!(resp.error.is_some(), "unbound hub must fail-closed");
        let err = resp.error.unwrap();
        assert_eq!(err.code, INTERNAL_ERROR);
        assert!(err.message.contains("not bound"), "expected 'not bound' in message, got: {}", err.message);
    }

    #[test]
    fn dispatch_after_router_drop_returns_internal_error() {
        let hub = DispatchHub::new();
        let router = Arc::new(Router::new());
        hub.bind(&router);
        drop(router);
        let resp = hub.dispatch("echo", "echo", json!({"text": "hi"}));
        assert!(resp.error.is_some(), "dropped router must fail-closed");
        let err = resp.error.unwrap();
        assert_eq!(err.code, INTERNAL_ERROR);
        assert!(
            err.message.contains("dropped") || err.message.contains("not bound"),
            "expected 'dropped' in message, got: {}",
            err.message
        );
    }
}
