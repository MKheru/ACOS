//! MCP message router — dispatches requests to registered service handlers

use rustc_hash::FxHashMap;

use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, METHOD_NOT_FOUND};
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

    /// Route a request to the appropriate handler
    pub fn route(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match self.services.get(&path.service) {
            Some(handler) => handler.handle(path, request),
            None => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Service '{}' not found", path.service),
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
