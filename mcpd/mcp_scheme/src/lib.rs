//! MCP Scheme Handler for ACOS (Agent-Centric OS)
//!
//! This crate implements the `mcp:` scheme for Redox OS, enabling native
//! Model Context Protocol communication through the kernel's URI-based IPC.
//!
//! # Architecture
//!
//! In Redox OS, everything is accessed via URL schemes (e.g., `file:`, `tcp:`).
//! The `mcp:` scheme extends this to provide semantic IPC for AI agents:
//!
//! - `mcp://system/config` → system configuration
//! - `mcp://ui/browser/tab/1` → browser tab control
//! - `mcp://system/processes` → process management
//!
//! # Protocol
//!
//! Messages follow JSON-RPC 2.0 (as per MCP specification):
//! - Write a JSON-RPC request to the file descriptor
//! - Read the JSON-RPC response back

pub mod protocol;
pub mod router;
pub mod handler;

mod system_handlers;
mod file_handlers;
mod support_handlers;
mod llm_handler;
pub mod net_handler;
pub mod ai_handler;
pub mod konsole_handler;
pub mod display_handler;
pub mod input_router;
pub mod boot_konsoles;
pub mod ai_konsole_bridge;
pub mod konsole_renderer;
pub mod talk_handler;
pub mod guardian_handler;
pub mod command_handler;

use system_handlers::{SystemInfoHandler, ProcessHandler, MemoryHandler};
use file_handlers::{FileReadHandler, FileWriteHandler, FileSearchHandler};
use support_handlers::{LogHandler, ConfigHandler};
use llm_handler::LlmHandler;
use net_handler::NetHandler;
use command_handler::{CommandHandler, ServiceManagerHandler};
use ai_handler::AiHandler;
use konsole_handler::KonsoleHandler;
use display_handler::DisplayHandler;
use talk_handler::TalkHandler;
use guardian_handler::GuardianHandler;

#[cfg(target_os = "redox")]
pub mod scheme_bridge;

#[cfg(feature = "host-test")]
pub mod mock;

use std::sync::{Arc, Mutex};
use rustc_hash::FxHashMap;

/// Maximum number of simultaneous open connections.
const MAX_CONNECTIONS: usize = 1024;

/// Maximum size of an accumulated request buffer (1 MiB).
const MAX_REQUEST_SIZE: usize = 1_048_576;

/// A unique handle for an open MCP connection
pub type HandleId = usize;

/// Represents an MCP resource path, parsed from the scheme URL
#[derive(Debug, Clone, PartialEq)]
pub struct McpPath {
    /// The service name (e.g., "system", "ui", "agent")
    pub service: String,
    /// The resource path within the service (e.g., "config", "processes")
    pub resource: Vec<String>,
}

impl McpPath {
    /// Parse a path like "system/config/network" into McpPath
    pub fn parse(path: &[u8]) -> Option<Self> {
        let path_str = std::str::from_utf8(path).ok()?;
        let path_str = path_str.trim_start_matches('/');

        let mut parts = path_str.splitn(2, '/');
        let service = parts.next()?.to_string();

        if service.is_empty() {
            return None;
        }

        let resource = parts
            .next()
            .map(|r| r.split('/').map(String::from).collect())
            .unwrap_or_default();

        Some(McpPath { service, resource })
    }

    /// Reconstruct the path as a string
    pub fn to_uri(&self) -> String {
        if self.resource.is_empty() {
            format!("mcp://{}", self.service)
        } else {
            format!("mcp://{}/{}", self.service, self.resource.join("/"))
        }
    }
}

/// The main MCP scheme handler state
pub struct McpScheme {
    /// Active connections: handle_id → (path, request_buffer, response_buffer)
    connections: FxHashMap<HandleId, McpConnection>,
    /// Next handle ID to assign
    next_id: HandleId,
    /// Registered service handlers (Arc for internal dispatch by AiHandler)
    router: std::sync::Arc<router::Router>,
}

/// An active MCP connection
struct McpConnection {
    path: McpPath,
    request_buf: Vec<u8>,
    response_buf: Vec<u8>,
    response_pos: usize,
}

impl McpScheme {
    pub fn new() -> Self {
        let mut router = router::Router::new();

        // Register built-in services
        router.register("system", SystemInfoHandler::new());
        router.register("process", ProcessHandler::new());
        router.register("memory", MemoryHandler::new());
        router.register("file", FileReadHandler::new());
        router.register("file_write", FileWriteHandler::new());
        router.register("file_search", FileSearchHandler::new());
        router.register("log", LogHandler::new());
        router.register("config", ConfigHandler::new());
        router.register("echo", handler::EchoHandler::new());
        router.register("mcp", handler::McpHandler::new()); // Replaced with dispatch version below
        router.register("command", CommandHandler::new());
        router.register("service", ServiceManagerHandler::new());
        router.register("net", NetHandler::new());
        // NOTE: llm, ai, talk, guardian registered AFTER Arc wrap (Phase 3) — they need dispatch

        // WS7: Konsole — shared state between KonsoleHandler and DisplayHandler
        let konsole_state = Arc::new(Mutex::new(Vec::new()));
        router.register("konsole", KonsoleHandler::new(Arc::clone(&konsole_state)));

        // Create DisplayHandler and set boot layout before registering
        let display_handler = DisplayHandler::new(Arc::clone(&konsole_state));

        // Boot: create Konsole 0 (Root AI) and Konsole 1 (User)
        boot_konsoles::init_boot_konsoles(&konsole_state);

        // Set display layout: 50/50 vertical split (Konsole 1 left, Konsole 0 right)
        display_handler.set_layout(boot_konsoles::boot_display_layout());

        // Register DisplayHandler after layout is set
        router.register("display", display_handler);

        // AI konsole bridge for writing AI activity to Konsole 0
        let ai_bridge = std::sync::Arc::new(ai_konsole_bridge::AiKonsoleBridge::new(konsole_state));

        // Wrap router in Arc for internal dispatch (avoids deadlock)
        let router = std::sync::Arc::new(router);

        // Register AI handler with internal dispatch via Arc<Router>
        let router_clone_ai = std::sync::Arc::clone(&router);
        let dispatch_ai: Box<dyn Fn(&str, &str, serde_json::Value) -> protocol::JsonRpcResponse + Send + Sync> =
            Box::new(move |service: &str, method: &str, params: serde_json::Value| {
                router_clone_ai.dispatch(service, method, params)
            });

        // Dispatch for talk handler
        let router_clone_talk = std::sync::Arc::clone(&router);
        let dispatch_talk: Box<dyn Fn(&str, &str, serde_json::Value) -> protocol::JsonRpcResponse + Send + Sync> =
            Box::new(move |service: &str, method: &str, params: serde_json::Value| {
                router_clone_talk.dispatch(service, method, params)
            });

        // Dispatch for guardian handler
        let router_clone_guardian = std::sync::Arc::clone(&router);
        let dispatch_guardian: Box<dyn Fn(&str, &str, serde_json::Value) -> protocol::JsonRpcResponse + Send + Sync> =
            Box::new(move |service: &str, method: &str, params: serde_json::Value| {
                router_clone_guardian.dispatch(service, method, params)
            });

        // Dispatch for llm handler
        let router_clone_llm = std::sync::Arc::clone(&router);
        let dispatch_llm: Box<dyn Fn(&str, &str, serde_json::Value) -> protocol::JsonRpcResponse + Send + Sync> =
            Box::new(move |service: &str, method: &str, params: serde_json::Value| {
                router_clone_llm.dispatch(service, method, params)
            });

        // We need to insert AiHandler into the Arc<Router> — use unsafe or reconstruct.
        // Since Arc::get_mut fails (we have router_clone), we use register_service via
        // a brief unsafe approach: we know router_clone is not being used concurrently yet.
        // Alternative: use the router's interior mutability or reconstruct.
        //
        // Simplest safe approach: drop the clone, get_mut, re-create the clone.
        // But we already moved router_clone into the closure...
        //
        // Better approach: Use a two-phase init with Option<Arc<Router>> in AiHandler.
        // But simplest working fix: store dispatch in a separate Arc<Mutex> and set it after.

        // Actually, let's just reconstruct: build a new router with AI included.
        // We can't easily add to Arc<Router> after cloning. Instead, use a raw pointer
        // approach that's safe because we're still in single-threaded init.
        let router_ptr = std::sync::Arc::as_ptr(&router) as *mut router::Router;
        // SAFETY: We are in single-threaded init, no other references are actively used.
        // The closures capture their respective router clones but won't be called until after init completes.
        // Dispatch for mcp handler (services/list needs to probe other services)
        let router_clone_mcp = std::sync::Arc::clone(&router);
        let dispatch_mcp: Box<dyn Fn(&str, &str, serde_json::Value) -> protocol::JsonRpcResponse + Send + Sync> =
            Box::new(move |service: &str, method: &str, params: serde_json::Value| {
                router_clone_mcp.dispatch(service, method, params)
            });

        unsafe {
            (*router_ptr).register("ai", AiHandler::new(dispatch_ai, Some(ai_bridge)));
            (*router_ptr).register("talk", TalkHandler::new(dispatch_talk));
            (*router_ptr).register("guardian", GuardianHandler::new(dispatch_guardian));
            (*router_ptr).register("llm", LlmHandler::new(dispatch_llm));
            // Re-register mcp with dispatch for services/list
            (*router_ptr).register("mcp", handler::McpHandler::new_with_dispatch(dispatch_mcp));
        }

        McpScheme {
            connections: FxHashMap::default(),
            next_id: 1,
            router,
        }
    }

    /// Open a new connection to an MCP resource
    pub fn open(&mut self, path: &[u8]) -> Result<HandleId, i32> {
        let mcp_path = McpPath::parse(path).ok_or(-libc::ENOENT)?;

        // Verify the service exists
        if !self.router.has_service(&mcp_path.service) {
            return Err(-libc::ENOENT);
        }

        // F2: Enforce max connection limit
        if self.connections.len() >= MAX_CONNECTIONS {
            return Err(-libc::ENOMEM);
        }

        // F12: Find a free ID that is not already in use (handles wrap-around)
        let id = {
            let mut candidate = self.next_id;
            let mut attempts = 0;
            loop {
                if !self.connections.contains_key(&candidate) {
                    break candidate;
                }
                candidate = candidate.wrapping_add(1);
                if candidate == 0 {
                    candidate = 1;
                }
                attempts += 1;
                if attempts >= MAX_CONNECTIONS {
                    return Err(-libc::ENOMEM);
                }
            }
        };
        self.next_id = id.wrapping_add(1);
        if self.next_id == 0 {
            self.next_id = 1;
        }

        self.connections.insert(id, McpConnection {
            path: mcp_path,
            request_buf: Vec::new(),
            response_buf: Vec::new(),
            response_pos: 0,
        });

        Ok(id)
    }

    /// Write a JSON-RPC request to the connection
    pub fn write(&mut self, id: HandleId, buf: &[u8]) -> Result<usize, i32> {
        let conn = self.connections.get_mut(&id).ok_or(-libc::EBADF)?;

        // Common case: no partial data buffered — parse directly from input (avoids copy)
        if conn.request_buf.is_empty() {
            if let Ok(request) = serde_json::from_slice::<protocol::JsonRpcRequest>(buf) {
                let response = self.router.route(&conn.path, &request);
                conn.response_buf.clear();
                // F5: Handle serialization failure with a JSON-RPC error response
                if serde_json::to_writer(&mut conn.response_buf, &response).is_err() {
                    conn.response_buf.clear();
                    conn.response_buf.extend_from_slice(
                        b"{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"Internal error\"},\"id\":null}"
                    );
                }
                conn.response_pos = 0;
                return Ok(buf.len());
            }
        }

        // Slow path: accumulate partial writes
        // F1: Enforce max request buffer size to prevent unbounded memory growth
        if conn.request_buf.len() + buf.len() > MAX_REQUEST_SIZE {
            conn.request_buf.clear();
            return Err(-libc::ENOMEM);
        }
        conn.request_buf.extend_from_slice(buf);
        if let Ok(request) = serde_json::from_slice::<protocol::JsonRpcRequest>(&conn.request_buf) {
            let response = self.router.route(&conn.path, &request);
            conn.response_buf.clear();
            // F5: Handle serialization failure with a JSON-RPC error response
            if serde_json::to_writer(&mut conn.response_buf, &response).is_err() {
                conn.response_buf.clear();
                conn.response_buf.extend_from_slice(
                    b"{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"Internal error\"},\"id\":null}"
                );
            }
            conn.response_pos = 0;
            conn.request_buf.clear();
        }

        Ok(buf.len())
    }

    /// Read a JSON-RPC response from the connection
    pub fn read(&mut self, id: HandleId, buf: &mut [u8]) -> Result<usize, i32> {
        let conn = self.connections.get_mut(&id).ok_or(-libc::EBADF)?;

        if conn.response_pos >= conn.response_buf.len() {
            return Ok(0); // No data available
        }

        let remaining = &conn.response_buf[conn.response_pos..];
        let to_copy = remaining.len().min(buf.len());
        buf[..to_copy].copy_from_slice(&remaining[..to_copy]);
        conn.response_pos += to_copy;

        Ok(to_copy)
    }

    /// Close a connection
    pub fn close(&mut self, id: HandleId) -> Result<(), i32> {
        self.connections.remove(&id).ok_or(-libc::EBADF)?;
        Ok(())
    }

    /// List registered services
    pub fn list_services(&self) -> Vec<&str> {
        self.router.list_services()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_path() {
        let path = McpPath::parse(b"system/config").unwrap();
        assert_eq!(path.service, "system");
        assert_eq!(path.resource, vec!["config"]);
    }

    #[test]
    fn test_parse_path_service_only() {
        let path = McpPath::parse(b"echo").unwrap();
        assert_eq!(path.service, "echo");
        assert!(path.resource.is_empty());
    }

    #[test]
    fn test_parse_path_deep() {
        let path = McpPath::parse(b"ui/browser/tab/1").unwrap();
        assert_eq!(path.service, "ui");
        assert_eq!(path.resource, vec!["browser", "tab", "1"]);
    }

    #[test]
    fn test_parse_empty() {
        assert!(McpPath::parse(b"").is_none());
    }

    #[test]
    fn test_to_uri() {
        let path = McpPath::parse(b"system/config").unwrap();
        assert_eq!(path.to_uri(), "mcp://system/config");
    }

    #[test]
    fn test_open_close() {
        let mut scheme = McpScheme::new();
        let id = scheme.open(b"echo").unwrap();
        assert!(scheme.close(id).is_ok());
    }

    #[test]
    fn test_open_nonexistent() {
        let mut scheme = McpScheme::new();
        assert!(scheme.open(b"nonexistent_service").is_err());
    }

    #[test]
    fn test_echo_roundtrip() {
        let mut scheme = McpScheme::new();
        let id = scheme.open(b"echo").unwrap();

        let request = r#"{"jsonrpc":"2.0","method":"echo","params":{"message":"hello"},"id":1}"#;
        let written = scheme.write(id, request.as_bytes()).unwrap();
        assert_eq!(written, request.len());

        let mut buf = vec![0u8; 4096];
        let read = scheme.read(id, &mut buf).unwrap();
        assert!(read > 0);

        let response: protocol::JsonRpcResponse = serde_json::from_slice(&buf[..read]).unwrap();
        assert_eq!(response.id, Some(serde_json::Value::Number(1.into())));
        assert!(response.error.is_none());

        scheme.close(id).unwrap();
    }

    fn mcp_roundtrip(method: &str, params: serde_json::Value) -> protocol::JsonRpcResponse {
        service_roundtrip("mcp", method, params)
    }

    fn service_roundtrip(service: &str, method: &str, params: serde_json::Value) -> protocol::JsonRpcResponse {
        let mut scheme = McpScheme::new();
        let id = scheme.open(service.as_bytes()).unwrap();
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });
        let req_str = serde_json::to_string(&request).unwrap();
        scheme.write(id, req_str.as_bytes()).unwrap();
        let mut buf = vec![0u8; 8192];
        let read = scheme.read(id, &mut buf).unwrap();
        scheme.close(id).unwrap();
        serde_json::from_slice(&buf[..read]).unwrap()
    }

    #[test]
    fn test_mcp_initialize() {
        let resp = mcp_roundtrip("initialize", serde_json::json!({}));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "acos-mcp");
    }

    #[test]
    fn test_mcp_notifications_initialized() {
        let resp = mcp_roundtrip("notifications/initialized", serde_json::json!({}));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_mcp_tools_list() {
        let resp = mcp_roundtrip("tools/list", serde_json::json!({}));
        assert!(resp.error.is_none());
        let tools = resp.result.unwrap()["tools"].clone();
        assert!(tools.is_array());
        assert!(!tools.as_array().unwrap().is_empty());
    }

    #[test]
    fn test_mcp_tools_call() {
        let resp = mcp_roundtrip("tools/call", serde_json::json!({
            "name": "echo",
            "arguments": { "message": "hello" }
        }));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["text"], "hello");
    }

    #[test]
    fn test_mcp_resources_list() {
        let resp = mcp_roundtrip("resources/list", serde_json::json!({}));
        assert!(resp.error.is_none());
        let resources = resp.result.unwrap()["resources"].clone();
        assert!(resources.is_array());
        assert!(!resources.as_array().unwrap().is_empty());
    }

    #[test]
    fn test_mcp_resources_read() {
        let resp = mcp_roundtrip("resources/read", serde_json::json!({
            "uri": "mcp://system/info"
        }));
        assert!(resp.error.is_none());
        let contents = resp.result.unwrap()["contents"].clone();
        assert!(contents.is_array());
        assert_eq!(contents[0]["uri"], "mcp://system/info");
    }

    #[test]
    fn test_mcp_prompts_list() {
        let resp = mcp_roundtrip("prompts/list", serde_json::json!({}));
        assert!(resp.error.is_none());
        let prompts = resp.result.unwrap()["prompts"].clone();
        assert!(prompts.is_array());
        assert!(!prompts.as_array().unwrap().is_empty());
    }

    #[test]
    fn test_mcp_prompts_get() {
        let resp = mcp_roundtrip("prompts/get", serde_json::json!({
            "name": "echo-prompt"
        }));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["messages"].is_array());
        assert_eq!(result["description"], "Simple echo prompt");
    }

    #[test]
    fn test_router_register_service() {
        let mut router = router::Router::new();
        let echo = handler::EchoHandler::new();
        router.register_service("custom_echo", Box::new(echo));

        assert!(router.has_service("custom_echo"));
        let services = router.list_services();
        assert!(services.contains(&"custom_echo"));
    }

    #[test]
    fn test_router_unregister_service() {
        let mut router = router::Router::new();
        let echo = handler::EchoHandler::new();
        router.register_service("temp_service", Box::new(echo));

        assert!(router.has_service("temp_service"));
        let result = router.unregister_service("temp_service");
        assert!(result);
        assert!(!router.has_service("temp_service"));
    }

    #[test]
    fn test_router_unregister_nonexistent() {
        let mut router = router::Router::new();
        let result = router.unregister_service("nonexistent");
        assert!(!result);
    }

    #[test]
    fn test_max_connections_limit() {
        let mut scheme = McpScheme::new();
        let mut handles = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            let id = scheme.open(b"echo").expect("should open within limit");
            handles.push(id);
        }
        // One more should fail
        assert_eq!(scheme.open(b"echo"), Err(-libc::ENOMEM));
        // After closing one, a new open should succeed
        scheme.close(handles[0]).unwrap();
        assert!(scheme.open(b"echo").is_ok());
    }

    #[test]
    fn test_request_buf_overflow_rejected() {
        let mut scheme = McpScheme::new();
        let id = scheme.open(b"echo").unwrap();
        // Force slow path by writing partial (non-parseable) data first
        let partial = b"{\"jsonrpc\":";
        scheme.write(id, partial).unwrap();
        // Now send enough data to exceed MAX_REQUEST_SIZE
        let filler = vec![b'x'; MAX_REQUEST_SIZE];
        let result = scheme.write(id, &filler);
        assert_eq!(result, Err(-libc::ENOMEM));
        scheme.close(id).unwrap();
    }

    #[test]
    fn test_next_id_wraps_and_skips_zero() {
        let mut scheme = McpScheme::new();
        // Force next_id to usize::MAX - 1 so wrapping can be observed
        scheme.next_id = usize::MAX;
        let id1 = scheme.open(b"echo").unwrap();
        assert_eq!(id1, usize::MAX);
        // After wrapping, next_id should be 1 (skipping 0)
        assert_eq!(scheme.next_id, 1);
        scheme.close(id1).unwrap();
    }

    #[test]
    fn test_registry_service_not_registered() {
        // RegistryHandler was removed; mcp: scheme no longer exposes "registry"
        let mut scheme = McpScheme::new();
        assert!(scheme.open(b"registry").is_err());
        // Router register/unregister still work at the Rust API level
        let services = scheme.list_services();
        assert!(!services.contains(&"registry"));
    }

    // -----------------------------------------------------------------------
    // New service tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_system_info() {
        let resp = service_roundtrip("system", "info", serde_json::json!({}));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["hostname"], "acos");
        assert!(result["uptime"].is_number());
    }

    #[test]
    fn test_process_list() {
        let resp = service_roundtrip("process", "list", serde_json::json!({}));
        assert!(resp.error.is_none());
        assert!(resp.result.unwrap().is_array());
    }

    #[test]
    fn test_memory_stats() {
        let resp = service_roundtrip("memory", "stats", serde_json::json!({}));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["total"].is_number());
        assert!(result["free"].is_number());
    }

    #[test]
    fn test_file_read() {
        let dir = std::env::temp_dir();
        let path = dir.join("acos_test_read.txt");
        std::fs::write(&path, "hello acos").unwrap();
        let resp = service_roundtrip("file", "read", serde_json::json!({"path": path.to_str().unwrap()}));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["content"], "hello acos");
        assert_eq!(result["size"], 10);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_file_write() {
        let dir = std::env::temp_dir();
        let path = dir.join("acos_test_write.txt");
        let resp = service_roundtrip("file_write", "write", serde_json::json!({"path": path.to_str().unwrap(), "content": "test data"}));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["bytes_written"], 9);
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "test data");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_file_read_path_traversal_rejected() {
        let resp = service_roundtrip("file", "read", serde_json::json!({"path": "../../../etc/passwd"}));
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_log_write_and_read() {
        let mut scheme = McpScheme::new();
        let id = scheme.open(b"log").unwrap();
        let req = serde_json::json!({"jsonrpc":"2.0","method":"write","params":{"level":"info","message":"test msg","source":"test"},"id":7});
        scheme.write(id, serde_json::to_string(&req).unwrap().as_bytes()).unwrap();
        let mut buf = vec![0u8; 4096];
        let n = scheme.read(id, &mut buf).unwrap();
        assert!(n > 0);

        let req2 = serde_json::json!({"jsonrpc":"2.0","method":"read","params":{"count":5},"id":8});
        scheme.write(id, serde_json::to_string(&req2).unwrap().as_bytes()).unwrap();
        let mut buf2 = vec![0u8; 4096];
        let n2 = scheme.read(id, &mut buf2).unwrap();
        let resp2: protocol::JsonRpcResponse = serde_json::from_slice(&buf2[..n2]).unwrap();
        assert!(resp2.error.is_none());
        let arr = resp2.result.unwrap();
        assert!(arr.is_array());
        scheme.close(id).unwrap();
    }

    #[test]
    fn test_log_list_levels() {
        let resp = service_roundtrip("log", "list", serde_json::json!({}));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["levels"].is_array());
    }

    #[test]
    fn test_config_set_and_get() {
        let mut scheme = McpScheme::new();
        let id = scheme.open(b"config").unwrap();
        let req = serde_json::json!({"jsonrpc":"2.0","method":"set","params":{"key":"hostname","value":"acos"},"id":10});
        scheme.write(id, serde_json::to_string(&req).unwrap().as_bytes()).unwrap();
        let mut buf = vec![0u8; 4096];
        scheme.read(id, &mut buf).unwrap();

        let req2 = serde_json::json!({"jsonrpc":"2.0","method":"get","params":{"key":"hostname"},"id":11});
        scheme.write(id, serde_json::to_string(&req2).unwrap().as_bytes()).unwrap();
        let mut buf2 = vec![0u8; 4096];
        let n2 = scheme.read(id, &mut buf2).unwrap();
        let resp2: protocol::JsonRpcResponse = serde_json::from_slice(&buf2[..n2]).unwrap();
        assert!(resp2.error.is_none());
        assert_eq!(resp2.result.unwrap()["value"], "acos");
        scheme.close(id).unwrap();
    }

    #[test]
    fn test_config_list() {
        let mut scheme = McpScheme::new();
        let id = scheme.open(b"config").unwrap();
        let req = serde_json::json!({"jsonrpc":"2.0","method":"set","params":{"key":"k1","value":"v1"},"id":12});
        scheme.write(id, serde_json::to_string(&req).unwrap().as_bytes()).unwrap();
        let mut buf = vec![0u8; 4096];
        scheme.read(id, &mut buf).unwrap();

        let req2 = serde_json::json!({"jsonrpc":"2.0","method":"list","params":{},"id":13});
        scheme.write(id, serde_json::to_string(&req2).unwrap().as_bytes()).unwrap();
        let mut buf2 = vec![0u8; 4096];
        let n2 = scheme.read(id, &mut buf2).unwrap();
        let resp2: protocol::JsonRpcResponse = serde_json::from_slice(&buf2[..n2]).unwrap();
        assert!(resp2.error.is_none());
        assert!(resp2.result.unwrap()["count"].as_u64().unwrap() >= 1);
        scheme.close(id).unwrap();
    }

    #[test]
    fn test_config_delete() {
        let mut scheme = McpScheme::new();
        let id = scheme.open(b"config").unwrap();
        let req = serde_json::json!({"jsonrpc":"2.0","method":"set","params":{"key":"temp","value":"val"},"id":14});
        scheme.write(id, serde_json::to_string(&req).unwrap().as_bytes()).unwrap();
        let mut buf = vec![0u8; 4096];
        scheme.read(id, &mut buf).unwrap();

        let req2 = serde_json::json!({"jsonrpc":"2.0","method":"delete","params":{"key":"temp"},"id":15});
        scheme.write(id, serde_json::to_string(&req2).unwrap().as_bytes()).unwrap();
        let mut buf2 = vec![0u8; 4096];
        let n2 = scheme.read(id, &mut buf2).unwrap();
        let resp2: protocol::JsonRpcResponse = serde_json::from_slice(&buf2[..n2]).unwrap();
        assert!(resp2.error.is_none());
        assert_eq!(resp2.result.unwrap()["ok"], true);
        scheme.close(id).unwrap();
    }

    #[test]
    fn test_file_search() {
        let dir = std::env::temp_dir().join("acos_search_test");
        std::fs::create_dir_all(&dir).ok();
        std::fs::write(dir.join("a.txt"), "hello world\nfoo bar").unwrap();
        std::fs::write(dir.join("b.txt"), "another hello line").unwrap();
        let resp = service_roundtrip("file_search", "search", serde_json::json!({"pattern": "hello", "path": dir.to_str().unwrap()}));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result["count"].as_u64().unwrap() >= 2);
        std::fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // KonsoleHandler + DisplayHandler direct unit tests
    // -----------------------------------------------------------------------

    use crate::handler::ServiceHandler;
    use crate::konsole_handler::{KonsoleHandler, Konsole, Color};
    use crate::display_handler::{Direction, LayoutNode, Rect, calculate_layout};

    fn kpath() -> McpPath {
        McpPath { service: "konsole".to_string(), resource: vec![] }
    }

    fn kreq(method: &str, params: serde_json::Value) -> protocol::JsonRpcRequest {
        protocol::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(serde_json::json!(1)),
        }
    }

    fn konsole_create_default(h: &KonsoleHandler) -> u32 {
        let resp = h.handle(&kpath(), &kreq("create", serde_json::json!({"type": "user", "owner": "test"})));
        resp.result.unwrap()["id"].as_u64().unwrap() as u32
    }

    fn konsole_create_sized(h: &KonsoleHandler, cols: u32, rows: u32) -> u32 {
        let resp = h.handle(&kpath(), &kreq("create", serde_json::json!({"type": "user", "owner": "test", "cols": cols, "rows": rows})));
        resp.result.unwrap()["id"].as_u64().unwrap() as u32
    }

    // --- Konsole lifecycle ---

    #[test]
    fn test_konsole_create_and_list() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("list", serde_json::json!({})));
        assert!(resp.error.is_none());
        let arr = resp.result.unwrap();
        let arr = arr.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], id);
        assert_eq!(arr[0]["type"], "user");
        assert_eq!(arr[0]["owner"], "test");
    }

    #[test]
    fn test_konsole_destroy() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("destroy", serde_json::json!({"id": id})));
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["ok"], true);
        let list_resp = h.handle(&kpath(), &kreq("list", serde_json::json!({})));
        assert_eq!(list_resp.result.unwrap().as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_konsole_write_and_read() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let wr = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "Hello"})));
        assert!(wr.error.is_none());
        let rd = h.handle(&kpath(), &kreq("read", serde_json::json!({"id": id})));
        assert!(rd.error.is_none());
        let lines = rd.result.unwrap()["lines"].clone();
        assert_eq!(lines[0], "Hello");
    }

    #[test]
    fn test_konsole_write_with_newlines() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "line1\r\nline2\r\nline3"})));
        let rd = h.handle(&kpath(), &kreq("read", serde_json::json!({"id": id})));
        let lines = rd.result.unwrap()["lines"].clone();
        assert_eq!(lines[0], "line1");
        assert_eq!(lines[1], "line2");
        assert_eq!(lines[2], "line3");
    }

    // --- ANSI parsing ---

    #[test]
    fn test_ansi_parser_color_fg() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[31mRed\x1b[0m"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Red);
        assert_eq!(k.buffer[0][1].fg, Color::Red);
        assert_eq!(k.buffer[0][2].fg, Color::Red);
        assert_eq!(k.current_fg, Color::Default);
    }

    #[test]
    fn test_ansi_parser_color_bg() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[42mGreen\x1b[0m"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        for i in 0..5 {
            assert_eq!(k.buffer[0][i].bg, Color::Green);
        }
        assert_eq!(k.current_bg, Color::Default);
    }

    #[test]
    fn test_ansi_parser_bold() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1mBold\x1b[0m"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        for i in 0..4 {
            assert!(k.buffer[0][i].bold);
        }
        assert!(!k.current_bold);
    }

    #[test]
    fn test_ansi_parser_cursor_move() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        // ESC[5;10H → row=4, col=9 (1-indexed → 0-indexed)
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[5;10H"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 4);
        assert_eq!(k.cursor_col, 9);
    }

    #[test]
    fn test_ansi_parser_clear_screen() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "Hello World"})));
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[2J"})));
        let rd = h.handle(&kpath(), &kreq("read", serde_json::json!({"id": id})));
        let lines = rd.result.unwrap()["lines"].clone();
        for line in lines.as_array().unwrap() {
            assert_eq!(line.as_str().unwrap(), "");
        }
    }

    #[test]
    fn test_ansi_parser_multi_param_sgr() {
        // \x1b[1;31m — bold + red in one sequence
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;31mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert!(k.buffer[0][0].bold);
        assert_eq!(k.buffer[0][0].fg, Color::Red);
    }

    #[test]
    fn test_ansi_parser_reset_shorthand() {
        // \x1b[m with no param should reset
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[31m\x1b[mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Default);
        assert!(!k.buffer[0][0].bold);
    }

    #[test]
    fn test_ansi_parser_empty_first_param() {
        // \x1b[;31m — empty first param, red second param
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[;31mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Red);
    }

    #[test]
    fn test_ansi_parser_tab_advance() {
        // \t should advance cursor to next 8-col boundary
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        // Write "AB\t" → cursor at col 2, tab moves to col 8
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "AB\t"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 8);
    }

    #[test]
    fn test_ansi_parser_tab_from_col_zero() {
        // \t from col 0 should move to col 8
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\t"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 8);
    }

    #[test]
    fn test_ansi_parser_cursor_save_restore() {
        // \x1b[s saves, \x1b[u restores
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        // Move to row=2, col=5, save, move to 0,0, restore
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[3;6H\x1b[s\x1b[1;1H\x1b[u"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 2);
        assert_eq!(k.cursor_col, 5);
    }

    #[test]
    fn test_ansi_parser_osc_sequence_no_crash() {
        // \x1b]0;title\x07 — OSC should be consumed without crash
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b]0;My Title\x07Hello"})));
        assert!(resp.error.is_none());
        // "Hello" should appear at buffer start
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].ch, 'H');
    }

    #[test]
    fn test_ansi_parser_long_csi_buffer_aborts() {
        // Sending 100+ digit CSI should abort at MAX_ANSI_BUF without crash
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let long_csi = format!("\x1b[{}m", "1".repeat(100));
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": long_csi})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_ansi_parser_unknown_csi_final_no_crash() {
        // \x1b[?25h — unknown CSI with ? parameter prefix, should not crash
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[?25hOK"})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_ansi_parser_mixed_text_and_escape() {
        // hello\x1b[31mworld\x1b[0m! — mixed text and escape sequences
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "hello\x1b[31mworld\x1b[0m!"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        // "hello" at 0-4: default color
        for i in 0..5 {
            assert_eq!(k.buffer[0][i].fg, Color::Default);
        }
        // "world" at 5-9: red
        for i in 5..10 {
            assert_eq!(k.buffer[0][i].fg, Color::Red);
        }
        // "!" at 10: default
        assert_eq!(k.buffer[0][10].fg, Color::Default);
    }

    // --- ANSI SGR: individual fg color codes ---

    #[test]
    fn test_ansi_parser_sgr_fg_black() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[30mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Black);
    }

    #[test]
    fn test_ansi_parser_sgr_fg_green() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[32mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Green);
    }

    #[test]
    fn test_ansi_parser_sgr_fg_yellow() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[33mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Yellow);
    }

    #[test]
    fn test_ansi_parser_sgr_fg_blue() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[34mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Blue);
    }

    #[test]
    fn test_ansi_parser_sgr_fg_magenta() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[35mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Magenta);
    }

    #[test]
    fn test_ansi_parser_sgr_fg_cyan() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[36mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Cyan);
    }

    #[test]
    fn test_ansi_parser_sgr_fg_white() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[37mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::White);
    }

    #[test]
    fn test_ansi_parser_sgr_fg_default() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[31m\x1b[39mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Default);
    }

    // --- ANSI SGR: bg colors ---

    #[test]
    fn test_ansi_parser_sgr_bg_black() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[40mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::Black);
    }

    #[test]
    fn test_ansi_parser_sgr_bg_red() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[41mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::Red);
    }

    #[test]
    fn test_ansi_parser_sgr_bg_green() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[42mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::Green);
    }

    #[test]
    fn test_ansi_parser_sgr_bg_yellow() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[43mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::Yellow);
    }

    #[test]
    fn test_ansi_parser_sgr_bg_blue() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[44mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::Blue);
    }

    #[test]
    fn test_ansi_parser_sgr_bg_magenta() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[45mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::Magenta);
    }

    #[test]
    fn test_ansi_parser_sgr_bg_cyan() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[46mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::Cyan);
    }

    #[test]
    fn test_ansi_parser_sgr_bg_white() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[47mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::White);
    }

    #[test]
    fn test_ansi_parser_sgr_bg_default() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[41m\x1b[49mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::Default);
    }

    // --- ANSI SGR: bright fg ---

    #[test]
    fn test_ansi_parser_sgr_bright_fg_black() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[90mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::BrightBlack);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_fg_red() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[91mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::BrightRed);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_fg_green() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[92mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::BrightGreen);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_fg_yellow() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[93mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::BrightYellow);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_fg_blue() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[94mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::BrightBlue);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_fg_magenta() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[95mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::BrightMagenta);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_fg_cyan() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[96mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::BrightCyan);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_fg_white() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[97mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::BrightWhite);
    }

    // --- ANSI SGR: bright bg ---

    #[test]
    fn test_ansi_parser_sgr_bright_bg_black() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[100mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::BrightBlack);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_bg_red() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[101mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::BrightRed);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_bg_green() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[102mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::BrightGreen);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_bg_yellow() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[103mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::BrightYellow);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_bg_blue() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[104mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::BrightBlue);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_bg_magenta() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[105mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::BrightMagenta);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_bg_cyan() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[106mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::BrightCyan);
    }

    #[test]
    fn test_ansi_parser_sgr_bright_bg_white() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[107mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::BrightWhite);
    }

    // --- ANSI CSI cursor movements ---

    #[test]
    fn test_ansi_parser_cursor_up_1() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[5;1H\x1b[1A"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 3);
    }

    #[test]
    fn test_ansi_parser_cursor_up_clamped() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[2;1H\x1b[5A"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 0);
    }

    #[test]
    fn test_ansi_parser_cursor_down_1() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[3;1H\x1b[2B"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 4);
    }

    #[test]
    fn test_ansi_parser_cursor_down_clamped() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 5);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[5;1H\x1b[10B"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 4);
    }

    #[test]
    fn test_ansi_parser_cursor_right_1() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;1H\x1b[5C"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 5);
    }

    #[test]
    fn test_ansi_parser_cursor_right_clamped() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 10, 5);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;1H\x1b[100C"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 9);
    }

    #[test]
    fn test_ansi_parser_cursor_left_1() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;10H\x1b[3D"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 6);
    }

    #[test]
    fn test_ansi_parser_cursor_left_clamped() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;3H\x1b[10D"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 0);
    }

    // --- ANSI CSI position (H/f) ---

    #[test]
    fn test_ansi_parser_cursor_home() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[10;20H\x1b[H"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 0);
        assert_eq!(k.cursor_col, 0);
    }

    #[test]
    fn test_ansi_parser_cursor_position_row_only() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[5H"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 4);
        assert_eq!(k.cursor_col, 0);
    }

    #[test]
    fn test_ansi_parser_cursor_position_clamped() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 10, 5);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[100;200H"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 4);
        assert_eq!(k.cursor_col, 9);
    }

    // --- ANSI CSI erase ---

    #[test]
    fn test_ansi_parser_erase_to_end_of_screen() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "HELLO\x1b[1;3H\x1b[0J"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        // chars after cursor should be cleared
        assert_eq!(k.buffer[0][2].ch, ' ');
    }

    #[test]
    fn test_ansi_parser_erase_to_start_of_screen() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "HELLO\x1b[1;4H\x1b[1J"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].ch, ' ');
    }

    #[test]
    fn test_ansi_parser_erase_line() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "HELLO\x1b[1;3H\x1b[K"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][2].ch, ' ');
        assert_eq!(k.buffer[0][4].ch, ' ');
    }

    // --- Tab stops ---

    #[test]
    fn test_ansi_parser_tab_from_col_8() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;9H\t"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 16);
    }

    #[test]
    fn test_ansi_parser_tab_from_col_7() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;8H\t"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 8);
    }

    #[test]
    fn test_ansi_parser_multiple_tabs() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\t\t"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 16);
    }

    // --- Cursor save/restore variations ---

    #[test]
    fn test_ansi_parser_save_restore_no_save() {
        // Restore without save should not panic
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[u"})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_ansi_parser_save_restore_preserves_col() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;20H\x1b[s\x1b[1;1H\x1b[u"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 19);
    }

    #[test]
    fn test_ansi_parser_save_restore_preserves_row() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[10;1H\x1b[s\x1b[1;1H\x1b[u"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 9);
    }

    // --- OSC variations ---

    #[test]
    fn test_ansi_parser_osc_long_title() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b]0;A very long window title that goes on and on\x07"})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_ansi_parser_osc_type_2() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b]2;icon name\x07text"})));
        assert!(resp.error.is_none());
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].ch, 't');
    }

    #[test]
    fn test_ansi_parser_osc_followed_by_sgr() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b]0;title\x07\x1b[31mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Red);
    }

    // --- Backspace ---

    #[test]
    fn test_ansi_parser_backspace_moves_left() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "ABC\x08"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 2);
    }

    #[test]
    fn test_ansi_parser_backspace_at_col0_no_underflow() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x08\x08\x08"})));
        assert!(resp.error.is_none());
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_col, 0);
    }

    // --- Multi-param SGR variants ---

    #[test]
    fn test_ansi_parser_sgr_reset_then_color() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;32;41mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert!(k.buffer[0][0].bold);
        assert_eq!(k.buffer[0][0].fg, Color::Green);
        assert_eq!(k.buffer[0][0].bg, Color::Red);
    }

    #[test]
    fn test_ansi_parser_sgr_0_resets_all() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;31;41m\x1b[0mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert!(!k.buffer[0][0].bold);
        assert_eq!(k.buffer[0][0].fg, Color::Default);
        assert_eq!(k.buffer[0][0].bg, Color::Default);
    }

    #[test]
    fn test_ansi_parser_sgr_bold_with_bg() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1;44mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert!(k.buffer[0][0].bold);
        assert_eq!(k.buffer[0][0].bg, Color::Blue);
    }

    // --- Long CSI variations ---

    #[test]
    fn test_ansi_parser_long_csi_semicolons() {
        // Long sequence of semicolons — should abort cleanly
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let long_csi = format!("\x1b[{}m", ";".repeat(100));
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": long_csi})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_ansi_parser_incomplete_escape_no_crash() {
        // ESC at end of string — should not crash
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "text\x1b"})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_ansi_parser_incomplete_csi_no_crash() {
        // ESC[ at end of string — should not crash
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "text\x1b["})));
        assert!(resp.error.is_none());
    }

    // --- Unknown CSI variants ---

    #[test]
    fn test_ansi_parser_unknown_csi_l() {
        // \x1b[?25l — hide cursor, should not crash
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[?25l"})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_ansi_parser_unknown_csi_r() {
        // \x1b[0c — device attributes, should not crash
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[0c"})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_ansi_parser_unknown_csi_p() {
        // \x1b[!p — soft reset, should not crash
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[!p"})));
        assert!(resp.error.is_none());
    }

    // --- Newline and CR ---

    #[test]
    fn test_ansi_parser_newline_advances_row() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "A\nB"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.cursor_row, 1);
    }

    #[test]
    fn test_ansi_parser_cr_resets_col() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "HELLO\rX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].ch, 'X');
    }

    #[test]
    fn test_ansi_parser_crlf_sequence() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "A\r\nB"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[1][0].ch, 'B');
    }

    // --- Scrollback with ANSI ---

    #[test]
    fn test_ansi_parser_color_survives_scroll() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 3);
        // Write 4 colored lines to force scroll
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[31mA\r\nB\r\nC\r\nD"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.scrollback.len(), 1);
    }

    #[test]
    fn test_ansi_parser_dirty_flag_set() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[31mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert!(k.dirty);
    }

    // --- Stress / edge cases ---

    #[test]
    fn test_ansi_parser_many_colors_sequence() {
        // Write multiple color changes in sequence
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let data = "\x1b[31mA\x1b[32mB\x1b[33mC\x1b[34mD\x1b[35mE\x1b[36mF\x1b[37mG";
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": data})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Red);
        assert_eq!(k.buffer[0][6].fg, Color::White);
    }

    #[test]
    fn test_ansi_parser_bold_persists_across_chars() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[1mABCDE"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        for i in 0..5 {
            assert!(k.buffer[0][i].bold, "char {} should be bold", i);
        }
    }

    #[test]
    fn test_ansi_parser_color_persists_across_chars() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[36mHELLO"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        for i in 0..5 {
            assert_eq!(k.buffer[0][i].fg, Color::Cyan, "char {} should be cyan", i);
        }
    }

    #[test]
    fn test_ansi_parser_escape_resets_partial_csi() {
        // ESC[ then non-CSI byte should complete sequence with that byte as final
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[X"})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_ansi_parser_alternating_text_and_color() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "A\x1b[31mB\x1b[0mC"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Default);
        assert_eq!(k.buffer[0][1].fg, Color::Red);
        assert_eq!(k.buffer[0][2].fg, Color::Default);
    }

    #[test]
    fn test_ansi_parser_sgr_fg_red_explicit() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[31mR"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Red);
        assert_eq!(k.buffer[0][0].ch, 'R');
    }

    #[test]
    fn test_ansi_parser_write_empty_string_no_crash() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        let resp = h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": ""})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_ansi_parser_sgr_bright_bg_black_text_over() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_default(&h);
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "\x1b[100m\x1b[97mX"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::BrightBlack);
        assert_eq!(k.buffer[0][0].fg, Color::BrightWhite);
    }

    // --- Scrollback ---

    #[test]
    fn test_scrollback_overflow() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 5);
        // 7 lines, 6 newlines — last 2 scroll line1 and line2 into scrollback
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": "line1\r\nline2\r\nline3\r\nline4\r\nline5\r\nline6\r\nline7"})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.scrollback.len(), 2);
    }

    #[test]
    fn test_scrollback_limit() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 5);
        {
            let mut konsoles = h.konsoles.lock().unwrap();
            let k = konsoles.iter_mut().find(|k| k.id == id).unwrap();
            k.scrollback_limit = 3;
        }
        let data = (1u32..=20).map(|i| format!("line{}", i)).collect::<Vec<_>>().join("\r\n");
        h.handle(&kpath(), &kreq("write", serde_json::json!({"id": id, "data": data})));
        let konsoles = h.konsoles.lock().unwrap();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.scrollback.len(), 3);
    }

    // --- Resize ---

    #[test]
    fn test_konsole_resize() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let id = konsole_create_sized(&h, 80, 24);
        let resp = h.handle(&kpath(), &kreq("resize", serde_json::json!({"id": id, "cols": 120, "rows": 40})));
        assert!(resp.error.is_none());
        let list = h.handle(&kpath(), &kreq("list", serde_json::json!({})));
        let arr = list.result.unwrap();
        let konsole = arr.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap().clone();
        assert_eq!(konsole["cols"], 120);
        assert_eq!(konsole["rows"], 40);
    }

    // --- Layout engine ---

    #[test]
    fn test_layout_single_leaf() {
        let node = LayoutNode::Leaf { konsole_id: 0 };
        let rect = Rect { x: 0, y: 0, w: 640, h: 480 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 0);
        let r = result[0].1;
        assert_eq!((r.x, r.y, r.w, r.h), (0, 0, 640, 480));
    }

    #[test]
    fn test_layout_horizontal_split() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (30, 70),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 640, h: 480 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
        let r0 = result[0].1;
        let r1 = result[1].1;
        // first_w = 640*30/100 = 192, border = 1, second_w = 447
        assert_eq!(r0.w, 192);
        assert_eq!(r1.x, 193);
        assert_eq!(r0.w + 1 + r1.w, 640);
        assert_eq!(r0.h, 480);
        assert_eq!(r1.h, 480);
    }

    #[test]
    fn test_layout_nested_split() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 640, h: 480 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 3);
        for i in 0..result.len() {
            for j in (i + 1)..result.len() {
                let a = result[i].1;
                let b = result[j].1;
                let no_overlap = (a.x + a.w <= b.x) || (b.x + b.w <= a.x)
                    || (a.y + a.h <= b.y) || (b.y + b.h <= a.y);
                assert!(no_overlap, "Rects {:?} and {:?} overlap", a, b);
            }
        }
    }

    // --- Helper for no-overlap assertion ---

    fn assert_no_overlap(result: &[(u32, Rect)]) {
        for i in 0..result.len() {
            for j in (i + 1)..result.len() {
                let a = result[i].1;
                let b = result[j].1;
                let no_overlap = (a.x + a.w <= b.x) || (b.x + b.w <= a.x)
                    || (a.y + a.h <= b.y) || (b.y + b.h <= a.y);
                assert!(no_overlap, "Rects {:?} and {:?} overlap", a, b);
            }
        }
    }

    fn assert_positive_area(result: &[(u32, Rect)]) {
        for (id, r) in result {
            assert!(r.w > 0, "konsole {} has zero width", id);
            assert!(r.h > 0, "konsole {} has zero height", id);
        }
    }

    // --- Vertical split basic ---

    #[test]
    fn test_layout_vertical_split() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (50, 50),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 800, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
        let r0 = result[0].1;
        let r1 = result[1].1;
        assert_eq!(r0.h, 300);
        assert_eq!(r1.y, 301);
        assert_eq!(r0.h + 1 + r1.h, 600);
        assert_eq!(r0.w, 800);
        assert_eq!(r1.w, 800);
    }

    // --- Ratio variations (horizontal) ---

    #[test]
    fn test_layout_hsplit_10_90() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (10, 90),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 1000, h: 500 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 100);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 1000);
    }

    #[test]
    fn test_layout_hsplit_1_3() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 3),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 400, h: 300 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 100);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 400);
    }

    #[test]
    fn test_layout_hsplit_2_3() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (2, 3),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 500, h: 400 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 200);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 500);
    }

    #[test]
    fn test_layout_hsplit_99_1() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (99, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 1000, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 990);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 1000);
    }

    // --- Ratio variations (vertical) ---

    #[test]
    fn test_layout_vsplit_10_90() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (10, 90),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 800, h: 1000 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 100);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 1000);
    }

    #[test]
    fn test_layout_vsplit_1_3() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 3),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 400, h: 400 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 100);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 400);
    }

    #[test]
    fn test_layout_vsplit_3_1() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (3, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 400, h: 400 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 300);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 400);
    }

    // --- Offset rects ---

    #[test]
    fn test_layout_leaf_with_offset() {
        let node = LayoutNode::Leaf { konsole_id: 5 };
        let rect = Rect { x: 100, y: 200, w: 300, h: 400 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 5);
        assert_eq!((result[0].1.x, result[0].1.y, result[0].1.w, result[0].1.h), (100, 200, 300, 400));
    }

    #[test]
    fn test_layout_hsplit_with_offset() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 50, y: 30, w: 200, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.x, 50);
        assert_eq!(result[0].1.y, 30);
        assert_eq!(result[1].1.y, 30);
        assert!(result[1].1.x > 50);
    }

    #[test]
    fn test_layout_vsplit_with_offset() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 10, y: 20, w: 400, h: 200 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.x, 10);
        assert_eq!(result[0].1.y, 20);
        assert_eq!(result[1].1.x, 10);
        assert!(result[1].1.y > 20);
    }

    // --- Edge case: zero ratio ---

    #[test]
    fn test_layout_zero_ratio() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (0, 0),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 640, h: 480 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 0);
    }

    // --- Edge case: 1px width/height ---

    #[test]
    fn test_layout_leaf_1px() {
        let node = LayoutNode::Leaf { konsole_id: 0 };
        let rect = Rect { x: 0, y: 0, w: 1, h: 1 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1.w, 1);
        assert_eq!(result[0].1.h, 1);
    }

    #[test]
    fn test_layout_hsplit_small_width() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 3, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_layout_vsplit_small_height() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 100, h: 3 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
    }

    // --- konsole_id preservation ---

    #[test]
    fn test_layout_konsole_ids_preserved() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 42 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 99 }),
        };
        let rect = Rect { x: 0, y: 0, w: 640, h: 480 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].0, 42);
        assert_eq!(result[1].0, 99);
    }

    // --- Deep nesting ---

    #[test]
    fn test_layout_deep_nesting_4_levels() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
            }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 3 }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 800, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 4);
        assert_no_overlap(&result);
        assert_positive_area(&result);
    }

    #[test]
    fn test_layout_deep_nesting_left_heavy() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Split {
                    direction: Direction::Horizontal,
                    ratio: (1, 1),
                    first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
                    second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
            }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 3 }),
        };
        let rect = Rect { x: 0, y: 0, w: 1024, h: 768 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 4);
        assert_no_overlap(&result);
    }

    // --- Grid-like layouts (alternating H/V splits) ---

    #[test]
    fn test_layout_2x2_grid() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
            }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 3 }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 800, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 4);
        assert_no_overlap(&result);
        assert_positive_area(&result);
    }

    #[test]
    fn test_layout_3_column() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 2),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 900, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 3);
        assert_no_overlap(&result);
        assert_positive_area(&result);
        // All should be on the same row (same y, same h)
        assert_eq!(result[0].1.y, result[1].1.y);
        assert_eq!(result[1].1.y, result[2].1.y);
        assert_eq!(result[0].1.h, result[1].1.h);
    }

    #[test]
    fn test_layout_3_row() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 2),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 800, h: 900 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 3);
        assert_no_overlap(&result);
        assert_positive_area(&result);
        // All should be in the same column (same x, same w)
        assert_eq!(result[0].1.x, result[1].1.x);
        assert_eq!(result[1].1.x, result[2].1.x);
        assert_eq!(result[0].1.w, result[1].1.w);
    }

    // --- Border accounting ---

    #[test]
    fn test_layout_border_horizontal() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 101, h: 50 };
        let result = calculate_layout(&node, rect);
        // first_w + border(1) + second_w = total
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 101);
    }

    #[test]
    fn test_layout_border_vertical() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 50, h: 101 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 101);
    }

    #[test]
    fn test_layout_border_second_pane_starts_after_border() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 200, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[1].1.x, result[0].1.w + 1);
    }

    #[test]
    fn test_layout_border_vsplit_second_starts_after_border() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 100, h: 200 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[1].1.y, result[0].1.h + 1);
    }

    // --- Large ratios ---

    #[test]
    fn test_layout_hsplit_large_ratio() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1000, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 1001, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
        assert!(result[0].1.w > result[1].1.w);
    }

    #[test]
    fn test_layout_vsplit_large_ratio() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1000),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 100, h: 1001 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
        assert!(result[1].1.h > result[0].1.h);
    }

    // --- Equal ratio symmetry ---

    #[test]
    fn test_layout_hsplit_equal_even_width() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 201, h: 100 };
        let result = calculate_layout(&node, rect);
        // With border: first_w + 1 + second_w = 201
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 201);
    }

    #[test]
    fn test_layout_vsplit_equal_even_height() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 100, h: 201 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 201);
    }

    // --- Various screen sizes ---

    #[test]
    fn test_layout_1080p_leaf() {
        let node = LayoutNode::Leaf { konsole_id: 0 };
        let rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 1920);
        assert_eq!(result[0].1.h, 1080);
    }

    #[test]
    fn test_layout_4k_split() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 3840, h: 2160 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 3840);
    }

    #[test]
    fn test_layout_small_200x150() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 200, h: 150 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 150);
    }

    // --- Y/X coordinate consistency ---

    #[test]
    fn test_layout_hsplit_y_preserved() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 10, y: 20, w: 400, h: 300 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.y, 20);
        assert_eq!(result[1].1.y, 20);
        assert_eq!(result[0].1.h, 300);
        assert_eq!(result[1].1.h, 300);
    }

    #[test]
    fn test_layout_vsplit_x_preserved() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 50, y: 100, w: 400, h: 300 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.x, 50);
        assert_eq!(result[1].1.x, 50);
        assert_eq!(result[0].1.w, 400);
        assert_eq!(result[1].1.w, 400);
    }

    // --- Deeply nested: 3x2 grid ---

    #[test]
    fn test_layout_3x2_grid() {
        let row = |ids: [u32; 3]| -> LayoutNode {
            LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 2),
                first: Box::new(LayoutNode::Leaf { konsole_id: ids[0] }),
                second: Box::new(LayoutNode::Split {
                    direction: Direction::Horizontal,
                    ratio: (1, 1),
                    first: Box::new(LayoutNode::Leaf { konsole_id: ids[1] }),
                    second: Box::new(LayoutNode::Leaf { konsole_id: ids[2] }),
                }),
            }
        };
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(row([0, 1, 2])),
            second: Box::new(row([3, 4, 5])),
        };
        let rect = Rect { x: 0, y: 0, w: 900, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 6);
        assert_no_overlap(&result);
        assert_positive_area(&result);
    }

    // --- IDE-like layout ---

    #[test]
    fn test_layout_ide_like() {
        // sidebar | (editor / terminal)
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 4),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),  // sidebar
            second: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (3, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),  // editor
                second: Box::new(LayoutNode::Leaf { konsole_id: 2 }),  // terminal
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 1280, h: 720 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 3);
        assert_no_overlap(&result);
        assert_positive_area(&result);
        // Sidebar should be narrow
        assert!(result[0].1.w < result[1].1.w);
    }

    // --- Result ordering ---

    #[test]
    fn test_layout_result_order_depth_first() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 10 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 20 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 30 }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 800, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].0, 10);
        assert_eq!(result[1].0, 20);
        assert_eq!(result[2].0, 30);
    }

    // --- Odd-number widths/heights (rounding) ---

    #[test]
    fn test_layout_hsplit_odd_width() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 99, h: 50 };
        let result = calculate_layout(&node, rect);
        // Should not lose pixels
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 99);
    }

    #[test]
    fn test_layout_vsplit_odd_height() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 50, h: 99 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 99);
    }

    // --- Prime number dimensions ---

    #[test]
    fn test_layout_hsplit_prime_width() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 2),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 997, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 997);
    }

    #[test]
    fn test_layout_vsplit_prime_height() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 2),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 100, h: 997 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 997);
    }

    // --- Width=2 edge case ---

    #[test]
    fn test_layout_hsplit_width_2() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 2, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_layout_vsplit_height_2() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 100, h: 2 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
    }

    // --- Horizontal split: height preserved ---

    #[test]
    fn test_layout_hsplit_both_panes_full_height() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 3),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 800, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 600);
        assert_eq!(result[1].1.h, 600);
    }

    // --- Vertical split: width preserved ---

    #[test]
    fn test_layout_vsplit_both_panes_full_width() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 3),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 800, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 800);
        assert_eq!(result[1].1.w, 800);
    }

    // --- First pane starts at rect origin ---

    #[test]
    fn test_layout_hsplit_first_at_origin() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 77, y: 88, w: 500, h: 400 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.x, 77);
        assert_eq!(result[0].1.y, 88);
    }

    #[test]
    fn test_layout_vsplit_first_at_origin() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 33, y: 44, w: 500, h: 400 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.x, 33);
        assert_eq!(result[0].1.y, 44);
    }

    // --- Asymmetric nested splits ---

    #[test]
    fn test_layout_left_deep_right_leaf() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 2),
                first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
                second: Box::new(LayoutNode::Split {
                    direction: Direction::Horizontal,
                    ratio: (1, 1),
                    first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                    second: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
                }),
            }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 3 }),
        };
        let rect = Rect { x: 0, y: 0, w: 1200, h: 800 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 4);
        assert_no_overlap(&result);
        assert_positive_area(&result);
    }

    // --- Multiple leaves with different IDs ---

    #[test]
    fn test_layout_five_pane_split() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 4),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Split {
                    direction: Direction::Horizontal,
                    ratio: (1, 1),
                    first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                    second: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
                }),
                second: Box::new(LayoutNode::Split {
                    direction: Direction::Horizontal,
                    ratio: (1, 1),
                    first: Box::new(LayoutNode::Leaf { konsole_id: 3 }),
                    second: Box::new(LayoutNode::Leaf { konsole_id: 4 }),
                }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 1024, h: 768 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 5);
        assert_no_overlap(&result);
        assert_positive_area(&result);
    }

    // --- Ratio edge: equal parts ---

    #[test]
    fn test_layout_hsplit_50_50() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (50, 50),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 1000, h: 500 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 500);
    }

    #[test]
    fn test_layout_vsplit_50_50() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (50, 50),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 500, h: 1000 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 500);
    }

    // --- Coverage gap: ratio (1,1) with exact halves ---

    #[test]
    fn test_layout_hsplit_exact_half_600() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 600, h: 400 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 300);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 600);
    }

    #[test]
    fn test_layout_vsplit_exact_half_400() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 400, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 300);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 600);
    }

    // --- Stress: 8 panes ---

    #[test]
    fn test_layout_8_pane_binary_tree() {
        fn make_row(ids: [u32; 4]) -> LayoutNode {
            LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Split {
                    direction: Direction::Horizontal,
                    ratio: (1, 1),
                    first: Box::new(LayoutNode::Leaf { konsole_id: ids[0] }),
                    second: Box::new(LayoutNode::Leaf { konsole_id: ids[1] }),
                }),
                second: Box::new(LayoutNode::Split {
                    direction: Direction::Horizontal,
                    ratio: (1, 1),
                    first: Box::new(LayoutNode::Leaf { konsole_id: ids[2] }),
                    second: Box::new(LayoutNode::Leaf { konsole_id: ids[3] }),
                }),
            }
        }
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(make_row([0, 1, 2, 3])),
            second: Box::new(make_row([4, 5, 6, 7])),
        };
        let rect = Rect { x: 0, y: 0, w: 1600, h: 900 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 8);
        assert_no_overlap(&result);
        assert_positive_area(&result);
    }

    // --- Ratio proportionality ---

    #[test]
    fn test_layout_hsplit_ratio_proportional() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 3),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 800, h: 600 };
        let result = calculate_layout(&node, rect);
        // first should be roughly 1/4 of 800 = 200
        assert_eq!(result[0].1.w, 200);
    }

    #[test]
    fn test_layout_vsplit_ratio_proportional() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 3),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 600, h: 800 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 200);
    }

    // --- Leaf count ---

    #[test]
    fn test_layout_single_leaf_count() {
        let node = LayoutNode::Leaf { konsole_id: 0 };
        let rect = Rect { x: 0, y: 0, w: 100, h: 100 };
        assert_eq!(calculate_layout(&node, rect).len(), 1);
    }

    #[test]
    fn test_layout_two_leaf_count() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        assert_eq!(calculate_layout(&node, Rect { x: 0, y: 0, w: 100, h: 100 }).len(), 2);
    }

    // --- Zero-width / zero-height rects for leaf ---

    #[test]
    fn test_layout_leaf_zero_width() {
        let node = LayoutNode::Leaf { konsole_id: 0 };
        let rect = Rect { x: 0, y: 0, w: 0, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1.w, 0);
    }

    #[test]
    fn test_layout_leaf_zero_height() {
        let node = LayoutNode::Leaf { konsole_id: 0 };
        let rect = Rect { x: 0, y: 0, w: 100, h: 0 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1.h, 0);
    }

    // --- Verify no gaps: total coverage ---

    #[test]
    fn test_layout_hsplit_total_width_coverage_various_ratios() {
        for (a, b) in [(1,1), (1,2), (1,3), (2,3), (1,4), (3,7), (5,5)] {
            let node = LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (a, b),
                first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
            };
            let rect = Rect { x: 0, y: 0, w: 1000, h: 500 };
            let result = calculate_layout(&node, rect);
            assert_eq!(result[0].1.w + 1 + result[1].1.w, 1000,
                "Width coverage failed for ratio {}:{}", a, b);
        }
    }

    #[test]
    fn test_layout_vsplit_total_height_coverage_various_ratios() {
        for (a, b) in [(1,1), (1,2), (1,3), (2,3), (1,4), (3,7), (5,5)] {
            let node = LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (a, b),
                first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
            };
            let rect = Rect { x: 0, y: 0, w: 500, h: 1000 };
            let result = calculate_layout(&node, rect);
            assert_eq!(result[0].1.h + 1 + result[1].1.h, 1000,
                "Height coverage failed for ratio {}:{}", a, b);
        }
    }

    // --- Various rect origins ---

    #[test]
    fn test_layout_hsplit_high_offset() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 1000, y: 2000, w: 400, h: 300 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.x, 1000);
        assert!(result[1].1.x > 1000);
        assert_eq!(result[0].1.y, 2000);
        assert_eq!(result[1].1.y, 2000);
    }

    #[test]
    fn test_layout_vsplit_high_offset() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 500, y: 750, w: 400, h: 300 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.y, 750);
        assert!(result[1].1.y > 750);
        assert_eq!(result[0].1.x, 500);
        assert_eq!(result[1].1.x, 500);
    }

    // --- Mixed direction triple split ---

    #[test]
    fn test_layout_v_then_h_triple() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 2),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 600, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 3);
        assert_no_overlap(&result);
        assert_positive_area(&result);
    }

    // --- Consistency: same input same output ---

    #[test]
    fn test_layout_deterministic() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 2),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 640, h: 480 };
        let r1 = calculate_layout(&node, rect);
        let r2 = calculate_layout(&node, rect);
        assert_eq!(r1[0].1.x, r2[0].1.x);
        assert_eq!(r1[0].1.w, r2[0].1.w);
        assert_eq!(r1[1].1.x, r2[1].1.x);
        assert_eq!(r1[1].1.w, r2[1].1.w);
    }

    // --- Nested: all-vertical chain ---

    #[test]
    fn test_layout_vertical_chain_4() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 3),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 2),
                first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                second: Box::new(LayoutNode::Split {
                    direction: Direction::Vertical,
                    ratio: (1, 1),
                    first: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
                    second: Box::new(LayoutNode::Leaf { konsole_id: 3 }),
                }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 800, h: 800 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 4);
        assert_no_overlap(&result);
        assert_positive_area(&result);
        // All panes should have same width
        for r in &result {
            assert_eq!(r.1.w, 800);
        }
    }

    // --- Nested: all-horizontal chain ---

    #[test]
    fn test_layout_horizontal_chain_4() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 3),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 2),
                first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                second: Box::new(LayoutNode::Split {
                    direction: Direction::Horizontal,
                    ratio: (1, 1),
                    first: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
                    second: Box::new(LayoutNode::Leaf { konsole_id: 3 }),
                }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 1200, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 4);
        assert_no_overlap(&result);
        assert_positive_area(&result);
        // All panes should have same height
        for r in &result {
            assert_eq!(r.1.h, 600);
        }
    }

    // --- Square rect ---

    #[test]
    fn test_layout_square_2x2() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
            }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 3 }),
            }),
        };
        let rect = Rect { x: 0, y: 0, w: 500, h: 500 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 4);
        assert_no_overlap(&result);
    }

    // --- Ratio (1, 9) extremes ---

    #[test]
    fn test_layout_hsplit_1_9() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 9),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 1000, h: 500 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 100);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 1000);
    }

    #[test]
    fn test_layout_vsplit_1_9() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 9),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 500, h: 1000 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 100);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 1000);
    }

    // --- Non-zero offset with nested ---

    #[test]
    fn test_layout_nested_with_offset() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
            }),
        };
        let rect = Rect { x: 100, y: 50, w: 800, h: 600 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 3);
        assert_no_overlap(&result);
        // All rects should be within bounds
        for (_, r) in &result {
            assert!(r.x >= 100);
            assert!(r.y >= 50);
            assert!(r.x + r.w <= 900);
            assert!(r.y + r.h <= 650);
        }
    }

    // --- Large konsole IDs ---

    #[test]
    fn test_layout_large_konsole_ids() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: u32::MAX }),
            second: Box::new(LayoutNode::Leaf { konsole_id: u32::MAX - 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 640, h: 480 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].0, u32::MAX);
        assert_eq!(result[1].0, u32::MAX - 1);
    }

    // --- Ratio: same as (2, 2) should match (1, 1) ---

    #[test]
    fn test_layout_ratio_normalization() {
        let mk = |a, b| {
            let node = LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (a, b),
                first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
            };
            calculate_layout(&node, Rect { x: 0, y: 0, w: 800, h: 600 })
        };
        let r11 = mk(1, 1);
        let r22 = mk(2, 2);
        let r55 = mk(5, 5);
        assert_eq!(r11[0].1.w, r22[0].1.w);
        assert_eq!(r11[0].1.w, r55[0].1.w);
    }

    // --- Ratio: (2, 4) same as (1, 2) ---

    #[test]
    fn test_layout_ratio_normalization_unequal() {
        let mk = |a, b| {
            let node = LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (a, b),
                first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
            };
            calculate_layout(&node, Rect { x: 0, y: 0, w: 900, h: 600 })
        };
        let r12 = mk(1, 2);
        let r24 = mk(2, 4);
        assert_eq!(r12[0].1.w, r24[0].1.w);
    }

    // --- Wide landscape ---

    #[test]
    fn test_layout_ultrawide() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 5120, h: 1440 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 5120);
    }

    // --- Tall portrait ---

    #[test]
    fn test_layout_tall_portrait() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 1080, h: 2340 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 2340);
    }

    // --- Cross-check: hsplit doesn't affect y dimension ---

    #[test]
    fn test_layout_hsplit_no_y_split() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 400, h: 300 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.y, 0);
        assert_eq!(result[1].1.y, 0);
    }

    // --- Cross-check: vsplit doesn't affect x dimension ---

    #[test]
    fn test_layout_vsplit_no_x_split() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 400, h: 300 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.x, 0);
        assert_eq!(result[1].1.x, 0);
    }

    // --- Width 1 with split ---

    #[test]
    fn test_layout_hsplit_width_1() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 1, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_layout_vsplit_height_1() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 100, h: 1 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
    }

    // --- 6 pane complex layout ---

    #[test]
    fn test_layout_6_pane_complex() {
        // (A | (B / C)) / (D | E | F)
        let top = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
            }),
        };
        let bottom = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 2),
            first: Box::new(LayoutNode::Leaf { konsole_id: 3 }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Horizontal,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 4 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 5 }),
            }),
        };
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (2, 1),
            first: Box::new(top),
            second: Box::new(bottom),
        };
        let rect = Rect { x: 0, y: 0, w: 1200, h: 900 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 6);
        assert_no_overlap(&result);
        assert_positive_area(&result);
    }

    // --- Bounds check: all results within input rect ---

    #[test]
    fn test_layout_all_within_bounds() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 2),
            first: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 1),
                first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
            }),
            second: Box::new(LayoutNode::Split {
                direction: Direction::Vertical,
                ratio: (1, 2),
                first: Box::new(LayoutNode::Leaf { konsole_id: 2 }),
                second: Box::new(LayoutNode::Leaf { konsole_id: 3 }),
            }),
        };
        let outer = Rect { x: 50, y: 50, w: 700, h: 500 };
        let result = calculate_layout(&node, outer);
        for (_, r) in &result {
            assert!(r.x >= outer.x);
            assert!(r.y >= outer.y);
            assert!(r.x + r.w <= outer.x + outer.w);
            assert!(r.y + r.h <= outer.y + outer.h);
        }
    }

    // --- Tiny splits ---

    #[test]
    fn test_layout_hsplit_ratio_1_100() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 100),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 1010, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 10);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 1010);
    }

    #[test]
    fn test_layout_vsplit_ratio_1_100() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 100),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 100, h: 1010 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 10);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 1010);
    }

    // --- Same konsole_id in multiple leaves ---

    #[test]
    fn test_layout_duplicate_konsole_id() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
        };
        let rect = Rect { x: 0, y: 0, w: 400, h: 300 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, 0);
        assert_eq!(result[1].0, 0);
    }

    // --- Ratio (0, N) edge case ---

    #[test]
    fn test_layout_hsplit_ratio_0_n() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (0, 1),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 400, h: 300 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1.w, 0);
    }

    #[test]
    fn test_layout_hsplit_ratio_n_0() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 0),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 400, h: 300 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1.w, 400);
    }

    // --- Max u32 rect dimensions ---

    #[test]
    fn test_layout_leaf_max_dimensions() {
        let node = LayoutNode::Leaf { konsole_id: 0 };
        let rect = Rect { x: 0, y: 0, w: 10000, h: 10000 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 10000);
        assert_eq!(result[0].1.h, 10000);
    }

    // --- Rounding behavior tests ---

    #[test]
    fn test_layout_hsplit_rounding_floor() {
        // 7 * 1/3 = 2.33 -> floor to 2
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (1, 2),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 7, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 2);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 7);
    }

    #[test]
    fn test_layout_vsplit_rounding_floor() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (1, 2),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 100, h: 7 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 2);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 7);
    }

    // --- Non-divisible ratios ---

    #[test]
    fn test_layout_hsplit_ratio_3_7_width_100() {
        let node = LayoutNode::Split {
            direction: Direction::Horizontal,
            ratio: (3, 7),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 100, h: 50 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.w, 30);
        assert_eq!(result[0].1.w + 1 + result[1].1.w, 100);
    }

    #[test]
    fn test_layout_vsplit_ratio_3_7_height_100() {
        let node = LayoutNode::Split {
            direction: Direction::Vertical,
            ratio: (3, 7),
            first: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
            second: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        };
        let rect = Rect { x: 0, y: 0, w: 50, h: 100 };
        let result = calculate_layout(&node, rect);
        assert_eq!(result[0].1.h, 30);
        assert_eq!(result[0].1.h + 1 + result[1].1.h, 100);
    }

    // --- Task 5: Registration order verification ---

    #[test]
    fn test_task5_all_expected_services_registered() {
        let scheme = McpScheme::new();
        let expected = [
            "system", "process", "memory", "file", "file_write", "file_search",
            "log", "config", "echo", "mcp", "command", "service", "net",
            "konsole", "display", "ai", "talk", "guardian", "llm",
        ];
        for name in &expected {
            assert!(
                scheme.router.has_service(name),
                "service '{}' not registered", name
            );
        }
    }

    #[test]
    fn test_task5_service_count() {
        let scheme = McpScheme::new();
        let expected = [
            "system", "process", "memory", "file", "file_write", "file_search",
            "log", "config", "echo", "mcp", "command", "service", "net",
            "konsole", "display", "ai", "talk", "guardian", "llm",
        ];
        // Verify all 19 exist
        let mut count = 0;
        for name in &expected {
            if scheme.router.has_service(name) {
                count += 1;
            }
        }
        assert_eq!(count, 19, "expected 19 services registered");
    }

    #[test]
    fn test_task5_dispatch_chains_reachable() {
        let scheme = McpScheme::new();
        // talk → ai chain
        assert!(scheme.router.has_service("talk"));
        assert!(scheme.router.has_service("ai"));
        // llm → net chain
        assert!(scheme.router.has_service("llm"));
        assert!(scheme.router.has_service("net"));
        // guardian → net + process + memory
        assert!(scheme.router.has_service("guardian"));
        assert!(scheme.router.has_service("process"));
        assert!(scheme.router.has_service("memory"));
    }

    // -----------------------------------------------------------------------
    // Task 6: Integration — verify no PROXY_ADDR in llm_handler or ai_handler
    // -----------------------------------------------------------------------
    #[test]
    fn test_no_proxy_addr_anywhere() {
        let llm_src = include_str!("llm_handler.rs");
        let ai_src = include_str!("ai_handler.rs");
        assert!(
            !llm_src.contains("PROXY_ADDR"),
            "llm_handler.rs must not contain PROXY_ADDR — all LLM traffic routes through net"
        );
        assert!(
            !ai_src.contains("PROXY_ADDR"),
            "ai_handler.rs must not contain PROXY_ADDR — all LLM traffic routes through net"
        );
    }

    // --- Task 6: Integration tests — full dispatch chains ---

    /// Helper: dispatch a JSON-RPC request through the full McpScheme open/write/read/close chain.
    fn dispatch_via_scheme(scheme: &mut McpScheme, service: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
        let handle = scheme.open(service.as_bytes()).unwrap_or_else(|_| panic!("open {} failed", service));
        let req = serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1});
        let req_bytes = serde_json::to_vec(&req).unwrap();
        scheme.write(handle, &req_bytes).expect("write failed");
        let mut buf = vec![0u8; 65536];
        let n = scheme.read(handle, &mut buf).expect("read failed");
        scheme.close(handle).expect("close failed");
        let resp: protocol::JsonRpcResponse = serde_json::from_slice(&buf[..n]).unwrap();
        resp.result.unwrap_or_else(|| {
            let err = resp.error.as_ref().map(|e| e.message.as_str()).unwrap_or("unknown");
            panic!("dispatch {}/{} error: {}", service, method, err);
        })
    }

    #[test]
    fn test_integ_echo_roundtrip() {
        let mut scheme = McpScheme::new();
        let result = dispatch_via_scheme(&mut scheme, "echo", "echo", serde_json::json!({"msg": "ping"}));
        assert_eq!(result["msg"], "ping");
    }

    #[test]
    fn test_integ_config_roundtrip() {
        let mut scheme = McpScheme::new();
        dispatch_via_scheme(&mut scheme, "config", "set", serde_json::json!({"key": "integ.k", "value": "integ_v"}));
        let result = dispatch_via_scheme(&mut scheme, "config", "get", serde_json::json!({"key": "integ.k"}));
        assert!(result.to_string().contains("integ_v"));
    }

    #[test]
    fn test_integ_log_roundtrip() {
        let mut scheme = McpScheme::new();
        dispatch_via_scheme(&mut scheme, "log", "write", serde_json::json!({"level": "info", "message": "integ_entry", "source": "test"}));
        let result = dispatch_via_scheme(&mut scheme, "log", "read", serde_json::json!({"count": 10}));
        assert!(result.to_string().contains("integ_entry"));
    }

    #[test]
    fn test_integ_guardian_state() {
        let mut scheme = McpScheme::new();
        let result = dispatch_via_scheme(&mut scheme, "guardian", "state", serde_json::json!({}));
        assert!(["nominal", "warning", "critical"].contains(&result["status"].as_str().unwrap()));
    }

    #[test]
    fn test_integ_guardian_config_network_fields() {
        let mut scheme = McpScheme::new();
        let result = dispatch_via_scheme(&mut scheme, "guardian", "config", serde_json::json!({}));
        let config = &result["config"];
        assert!(config["network_monitoring"].is_boolean());
        assert!(config["ai_consultation_enabled"].is_boolean());
        assert!(config["llm_model"].is_string());
    }

    #[test]
    fn test_integ_llm_info_graceful() {
        let mut scheme = McpScheme::new();
        let result = dispatch_via_scheme(&mut scheme, "llm", "info", serde_json::json!({}));
        assert!(result.get("model_name").is_some() || result.get("backend").is_some());
    }

    #[test]
    fn test_integ_llm_missing_prompt() {
        let mut scheme = McpScheme::new();
        let handle = scheme.open(b"llm").unwrap();
        let req = serde_json::json!({"jsonrpc":"2.0","method":"generate","params":{},"id":1});
        scheme.write(handle, &serde_json::to_vec(&req).unwrap()).unwrap();
        let mut buf = vec![0u8; 65536];
        let n = scheme.read(handle, &mut buf).unwrap();
        scheme.close(handle).unwrap();
        let resp: protocol::JsonRpcResponse = serde_json::from_slice(&buf[..n]).unwrap();
        assert_eq!(resp.error.unwrap().code, protocol::INVALID_PARAMS);
    }

    #[test]
    fn test_integ_mcp_initialize() {
        let mut scheme = McpScheme::new();
        let result = dispatch_via_scheme(&mut scheme, "mcp", "initialize", serde_json::json!({}));
        assert_eq!(result["protocolVersion"], "2024-11-05");
    }

    #[test]
    fn test_integ_net_status() {
        let mut scheme = McpScheme::new();
        let result = dispatch_via_scheme(&mut scheme, "net", "status", serde_json::json!({}));
        assert_eq!(result["ip"], "10.0.2.15");
        assert_eq!(result["llm_backend"]["type"], "ollama");
    }

    #[test]
    fn test_integ_net_llm_request_mock() {
        let mut scheme = McpScheme::new();
        let result = dispatch_via_scheme(&mut scheme, "net", "llm_request", serde_json::json!({
            "messages": [{"role": "user", "content": "hello"}],
            "model": "phi4-mini"
        }));
        assert_eq!(result["message"]["content"], "mock response");
    }

    #[test]
    fn test_integ_nonexistent_service() {
        let mut scheme = McpScheme::new();
        assert_eq!(scheme.open(b"nonexistent_xyz"), Err(-libc::ENOENT));
    }

    #[test]
    fn test_integ_all_19_services_openable() {
        let mut scheme = McpScheme::new();
        let services = [
            "system", "process", "memory", "file", "file_write", "file_search",
            "log", "config", "echo", "mcp", "command", "service", "net",
            "konsole", "display", "ai", "talk", "guardian", "llm",
        ];
        for svc in &services {
            let h = scheme.open(svc.as_bytes());
            assert!(h.is_ok(), "Failed to open '{}'", svc);
            scheme.close(h.unwrap()).unwrap();
        }
    }

    #[test]
    fn test_integ_write_closed_handle_ebadf() {
        let mut scheme = McpScheme::new();
        let handle = scheme.open(b"echo").unwrap();
        scheme.close(handle).unwrap();
        let req = serde_json::to_vec(&serde_json::json!({"jsonrpc":"2.0","method":"echo","params":{},"id":1})).unwrap();
        assert_eq!(scheme.write(handle, &req), Err(-libc::EBADF));
    }

    #[test]
    fn test_integ_double_close_ebadf() {
        let mut scheme = McpScheme::new();
        let handle = scheme.open(b"echo").unwrap();
        scheme.close(handle).unwrap();
        assert_eq!(scheme.close(handle), Err(-libc::EBADF));
    }
}
