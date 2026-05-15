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

use serde_json::Value;

use crate::protocol::{JsonRpcResponse, INTERNAL_ERROR};
use crate::router::Router;

/// Clonable dispatcher backed by a `Weak<Router>` set after construction.
///
/// Cloning is cheap (shared `Arc<RwLock<...>>` handle). All clones see the
/// same binding, so binding once after construction suffices for every
/// handler that captured a clone.
#[derive(Clone)]
pub struct DispatchHub {
    inner: Arc<RwLock<Option<Weak<Router>>>>,
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

    /// Dispatch a service+method call through the bound router.
    ///
    /// Returns `INTERNAL_ERROR` if:
    /// - the hub has not yet been bound,
    /// - the router has been dropped (`Weak::upgrade` fails),
    /// - the internal lock is poisoned.
    ///
    /// Never panics.
    pub fn dispatch(&self, service: &str, method: &str, params: Value) -> JsonRpcResponse {
        // Acquire the read lock, clone the Weak, and release the lock before
        // calling router.dispatch() to avoid holding it across an outbound
        // call that might recurse into another DispatchHub method.
        let weak = {
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
        match weak.upgrade() {
            Some(router) => router.dispatch(service, method, params),
            None => JsonRpcResponse::error(
                None,
                INTERNAL_ERROR,
                "Router dropped before DispatchHub call".to_string(),
            ),
        }
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
