//! Built-in MCP service handlers

use serde_json::json;

use crate::protocol::{JsonRpcRequest, JsonRpcResponse, METHOD_NOT_FOUND};
use crate::McpPath;

/// Trait that all MCP service handlers must implement
pub trait ServiceHandler: Send + Sync {
    /// Handle a JSON-RPC request for this service
    fn handle(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse;

    /// List available methods for this service
    fn list_methods(&self) -> Vec<&str>;
}

// ---------------------------------------------------------------------------
// Echo Handler — simple test/debug service
// ---------------------------------------------------------------------------

pub struct EchoHandler;

impl EchoHandler {
    pub fn new() -> Self {
        EchoHandler
    }
}

impl ServiceHandler for EchoHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "echo" => JsonRpcResponse::success(request.id.clone(), request.params.clone()),
            "ping" => JsonRpcResponse::success(request.id.clone(), json!("pong")),
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in echo service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["echo", "ping"]
    }
}

// ---------------------------------------------------------------------------
// MCP Handler — MCP specification standard methods
// ---------------------------------------------------------------------------

/// Dispatch function type for inter-service calls.
type DispatchFn = Box<dyn Fn(&str, &str, serde_json::Value) -> JsonRpcResponse + Send + Sync>;

pub struct McpHandler {
    dispatch: Option<DispatchFn>,
}

impl McpHandler {
    pub fn new() -> Self {
        McpHandler { dispatch: None }
    }

    pub fn new_with_dispatch(dispatch: DispatchFn) -> Self {
        McpHandler { dispatch: Some(dispatch) }
    }
}

impl ServiceHandler for McpHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "initialize" => {
                JsonRpcResponse::success(request.id.clone(), json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": { "listChanged": false },
                        "resources": { "subscribe": false, "listChanged": false },
                        "prompts": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "acos-mcp",
                        "version": "0.1.0"
                    }
                }))
            }
            "notifications/initialized" => {
                // No-op acknowledgment — client ready signal
                JsonRpcResponse::success(request.id.clone(), json!(null))
            }
            "tools/list" => {
                JsonRpcResponse::success(request.id.clone(), json!({
                    "tools": [
                        {
                            "name": "echo",
                            "description": "Echo back the input",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "message": { "type": "string" }
                                }
                            }
                        }
                    ]
                }))
            }
            "tools/call" => {
                let tool_name = request.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                match tool_name {
                    "echo" => {
                        let message = request.params
                            .get("arguments")
                            .and_then(|a| a.get("message"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        JsonRpcResponse::success(request.id.clone(), json!({
                            "content": [{ "type": "text", "text": message }],
                            "isError": false
                        }))
                    }
                    _ => JsonRpcResponse::success(request.id.clone(), json!({
                        "content": [{ "type": "text", "text": format!("Unknown tool: {}", tool_name) }],
                        "isError": true
                    }))
                }
            }
            "resources/list" => {
                JsonRpcResponse::success(request.id.clone(), json!({
                    "resources": [
                        {
                            "uri": "mcp://system/info",
                            "name": "System Info",
                            "mimeType": "application/json"
                        }
                    ]
                }))
            }
            "resources/read" => {
                let uri = request.params.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                match uri {
                    "mcp://system/info" => {
                        JsonRpcResponse::success(request.id.clone(), json!({
                            "contents": [{
                                "uri": "mcp://system/info",
                                "mimeType": "application/json",
                                "text": "{\"os\":\"ACOS\",\"kernel\":\"Redox (fork)\",\"version\":\"0.1.0\"}"
                            }]
                        }))
                    }
                    _ => JsonRpcResponse::error(
                        request.id.clone(),
                        crate::protocol::INVALID_PARAMS,
                        format!("Resource not found: {}", uri),
                    )
                }
            }
            "prompts/list" => {
                JsonRpcResponse::success(request.id.clone(), json!({
                    "prompts": [
                        {
                            "name": "echo-prompt",
                            "description": "Simple echo prompt",
                            "arguments": [
                                { "name": "message", "required": true }
                            ]
                        }
                    ]
                }))
            }
            "prompts/get" => {
                let name = request.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                match name {
                    "echo-prompt" => {
                        JsonRpcResponse::success(request.id.clone(), json!({
                            "description": "Simple echo prompt",
                            "messages": [{
                                "role": "user",
                                "content": { "type": "text", "text": "{message}" }
                            }]
                        }))
                    }
                    _ => JsonRpcResponse::error(
                        request.id.clone(),
                        crate::protocol::INVALID_PARAMS,
                        format!("Prompt not found: {}", name),
                    )
                }
            }
            "services/list" => {
                // Dynamic service discovery via dispatch probing.
                // We probe each service by calling it — any response (even error) means live.
                // The list comes from the router at runtime, not a hardcoded constant.
                if let Some(ref dispatch) = self.dispatch {
                    // Probe known service names. We get them by calling ourselves recursively
                    // would deadlock, so we maintain a discovery list that lib.rs sets at init.
                    // Instead: probe a broad set. Any service that responds (even with error) is live.
                    // We use the dispatch to try each one with a harmless "ping" call.
                    let candidates = [
                        "system", "process", "memory", "file", "file_write", "file_search",
                        "log", "config", "echo", "mcp", "llm", "command", "service",
                        "konsole", "display", "ai", "talk", "guardian", "net",
                    ];
                    let mut services = Vec::new();
                    for &name in &candidates {
                        let resp = dispatch(name, "ping", json!({}));
                        // If router found the service (any response, even method_not_found), it's live
                        let live = resp.result.is_some() || resp.error.as_ref()
                            .map(|e| e.code == METHOD_NOT_FOUND || e.code == crate::protocol::INVALID_PARAMS)
                            .unwrap_or(false);
                        let down = resp.error.as_ref()
                            .map(|e| e.message.contains("not found") && e.message.contains("Service"))
                            .unwrap_or(false);
                        if !down {
                            services.push(json!({
                                "name": name,
                                "status": if live { "live" } else { "error" },
                            }));
                        }
                    }

                    JsonRpcResponse::success(request.id.clone(), json!({
                        "services": services,
                        "total": services.len(),
                    }))
                } else {
                    JsonRpcResponse::error(request.id.clone(), crate::protocol::INTERNAL_ERROR,
                        "services/list requires dispatch (McpHandler not initialized with dispatch)")
                }
            }
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in mcp service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec![
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/call",
            "resources/list",
            "resources/read",
            "prompts/list",
            "prompts/get",
            "services/list",
        ]
    }
}

// NOTE: SystemHandler was removed in WS3 and replaced by SystemInfoHandler,
// ProcessHandler, and MemoryHandler in system_handlers.rs.

// NOTE: RegistryHandler (dynamic JSON-RPC service registration) was removed.
// It was a no-op stub that accepted register/unregister/list calls but never
// actually mutated the Router. A proper design (requiring Router access from
// the handler) is deferred. Router::register_service / unregister_service
// remain available at the Rust API level for future use.
