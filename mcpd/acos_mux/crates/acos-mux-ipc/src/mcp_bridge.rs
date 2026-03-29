//! MCP 2.0 JSON-RPC bridge for ACOS-MUX tools integration.
//!
//! Translates between MCP JSON-RPC requests and ACOS-MUX ClientMessage operations.
//! Supports tools: split_pane, read_pane, write_pane, list_panes.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::messages::{ClientMessage, ServerMessage, SplitDirection};

/// JSON-RPC 2.0 version string.
pub const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC error code: Parse error.
pub const ERR_PARSE_ERROR: i32 = -32700;

/// JSON-RPC error code: Method not found.
pub const ERR_METHOD_NOT_FOUND: i32 = -32601;

/// JSON-RPC error code: Invalid params.
pub const ERR_INVALID_PARAMS: i32 = -32602;

/// JSON-RPC error code: Internal/server error (application error range).
pub const ERR_INTERNAL: i32 = -32000;

/// Tool name: split_pane — split a pane in a direction.
pub const TOOL_SPLIT_PANE: &str = "split_pane";

/// Tool name: read_pane — capture content of a pane.
pub const TOOL_READ_PANE: &str = "read_pane";

/// Tool name: write_pane — send input to a pane.
pub const TOOL_WRITE_PANE: &str = "write_pane";

/// Tool name: list_panes — list all panes in session.
pub const TOOL_LIST_PANES: &str = "list_panes";

/// MCP JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
    pub id: Value,
}

/// MCP JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
    pub id: Value,
}

/// MCP JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
}

impl McpError {
    /// Create a new error with code and message.
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// MCP JSON-RPC 2.0 notification (server-sent, no id field).
///
/// Used for asynchronous events like PTY output streaming.
/// Unlike responses, notifications don't require a response and have no `id` field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

impl McpNotification {
    /// Create a new notification with the given method and params.
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: method.into(),
            params,
        }
    }
}

/// Parse an MCP tool call request into a ClientMessage.
///
/// Extracts tool name from `req.method` and arguments from `req.params["arguments"]`.
///
/// Returns `Err(McpError)` if:
/// - Tool method is unknown (ERR_METHOD_NOT_FOUND)
/// - Required parameters are missing (ERR_INVALID_PARAMS)
/// - Direction is invalid (ERR_INVALID_PARAMS)
pub fn parse_tool_call(req: &McpRequest) -> Result<ClientMessage, McpError> {
    match req.method.as_str() {
        TOOL_SPLIT_PANE => {
            let args = req.params.get("arguments")
                .ok_or_else(|| McpError::new(ERR_INVALID_PARAMS, "missing arguments"))?;

            let direction_str = args.get("direction")
                .and_then(|v| v.as_str())
                .ok_or_else(|| McpError::new(ERR_INVALID_PARAMS, "missing direction"))?;

            let direction = match direction_str {
                "horizontal" | "h" => SplitDirection::Horizontal,
                "vertical" | "v" => SplitDirection::Vertical,
                _ => return Err(McpError::new(ERR_INVALID_PARAMS, "invalid direction")),
            };

            let size = if let Some(pv) = args.get("percent") {
                let v = pv.as_u64()
                    .ok_or_else(|| McpError::new(ERR_INVALID_PARAMS, "percent must be an integer"))?;
                if v > 100 {
                    return Err(McpError::new(ERR_INVALID_PARAMS, "percent must be 0-100"));
                }
                Some(v as u16)
            } else {
                None
            };

            Ok(ClientMessage::SplitPane { direction, size })
        }
        TOOL_READ_PANE => {
            let args = req.params.get("arguments")
                .ok_or_else(|| McpError::new(ERR_INVALID_PARAMS, "missing arguments"))?;

            let pane_id = args.get("pane_id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| McpError::new(ERR_INVALID_PARAMS, "missing pane_id"))?
                as u32;

            Ok(ClientMessage::CapturePane { pane_id })
        }
        TOOL_WRITE_PANE => {
            let args = req.params.get("arguments")
                .ok_or_else(|| McpError::new(ERR_INVALID_PARAMS, "missing arguments"))?;

            let pane_id = args.get("pane_id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| McpError::new(ERR_INVALID_PARAMS, "missing pane_id"))?
                as u32;

            let keys = args.get("keys")
                .and_then(|v| v.as_str())
                .ok_or_else(|| McpError::new(ERR_INVALID_PARAMS, "missing keys"))?
                .to_string();

            Ok(ClientMessage::SendKeys { pane_id, keys })
        }
        TOOL_LIST_PANES => {
            Ok(ClientMessage::ListPanes)
        }
        _ => Err(McpError::new(ERR_METHOD_NOT_FOUND, "unknown tool")),
    }
}

/// Format a ServerMessage as an MCP JSON-RPC response.
///
/// Maps ServerMessage variants to appropriate response data:
/// - `SpawnResult { pane_id }` → `{ "pane_id": <id> }`
/// - `PaneCaptured { pane_id, content }` → `{ "pane_id": <id>, "content": <text> }`
/// - `PaneList { panes }` → `{ "panes": [{ "id": ..., "title": ..., ... }] }`
/// - `Error { message }` → error response with ERR_INVALID_PARAMS
pub fn format_response(id: Value, msg: &ServerMessage) -> McpResponse {
    match msg {
        ServerMessage::SpawnResult { pane_id } => {
            McpResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                result: Some(json!({ "pane_id": pane_id })),
                error: None,
                id,
            }
        }
        ServerMessage::PaneCaptured { pane_id, content } => {
            McpResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                result: Some(json!({ "pane_id": pane_id, "content": content })),
                error: None,
                id,
            }
        }
        ServerMessage::PaneList { panes } => {
            let pane_data: Vec<Value> = panes.iter().map(|p| {
                json!({
                    "id": p.id,
                    "title": p.title,
                    "cols": p.cols,
                    "rows": p.rows,
                    "active": p.active,
                    "has_notification": p.has_notification,
                })
            }).collect();
            McpResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                result: Some(json!({ "panes": pane_data })),
                error: None,
                id,
            }
        }
        ServerMessage::Error { message } => {
            error_response(id, ERR_INTERNAL, message)
        }
        ServerMessage::Ack => {
            McpResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                result: Some(json!({})),
                error: None,
                id,
            }
        }
        // These variants are streaming notifications or non-response messages;
        // they are not valid as direct tool-call responses.
        ServerMessage::Pong
        | ServerMessage::Version { .. }
        | ServerMessage::Render { .. }
        | ServerMessage::SessionList { .. }
        | ServerMessage::PaneInfo { .. }
        | ServerMessage::PtyOutput { .. }
        | ServerMessage::LayoutChanged
        | ServerMessage::SessionEnded => {
            error_response(id, ERR_INTERNAL, "unsupported response type")
        }
    }
}

/// Create an error response with the given code and message.
pub fn error_response(id: Value, code: i32, message: &str) -> McpResponse {
    McpResponse {
        jsonrpc: JSONRPC_VERSION.to_string(),
        result: None,
        error: Some(McpError {
            code,
            message: message.to_string(),
        }),
        id,
    }
}

/// Convert a streaming ServerMessage to a notification.
///
/// Returns `Some(notification)` for streaming messages like `PtyOutput` and `LayoutChanged`.
/// Returns `None` for request-response messages that should be handled as normal responses.
pub fn to_notification(msg: &ServerMessage) -> Option<McpNotification> {
    match msg {
        ServerMessage::PtyOutput { pane_id, data } => {
            // LIMITATION: MCP is JSON-based, so binary PTY data is transmitted as a UTF-8 string
            // using lossy conversion. Invalid byte sequences are replaced with U+FFFD (replacement
            // character), which corrupts binary escape sequences. The correct long-term fix is to
            // base64-encode the data and add an "encoding": "base64" field, but that requires
            // adding a base64 dependency and a coordinated client-side change.
            Some(McpNotification::new(
                "pty_output",
                json!({ "pane_id": pane_id, "data": String::from_utf8_lossy(data).to_string() }),
            ))
        }
        ServerMessage::LayoutChanged => {
            Some(McpNotification::new("layout_changed", json!({})))
        }
        ServerMessage::SessionEnded => {
            Some(McpNotification::new("session_ended", json!({})))
        }
        _ => None,
    }
}

/// Return a list of all available tools with their schemas.
///
/// Each tool includes name, description, and input schema with properties
/// describing required and optional parameters.
pub fn tools_list() -> Vec<Value> {
    vec![
        json!({
            "name": TOOL_SPLIT_PANE,
            "description": "Split the focused pane in the specified direction",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "direction": {
                        "type": "string",
                        "description": "Direction: 'horizontal' or 'h' for left/right, 'vertical' or 'v' for top/bottom",
                        "enum": ["horizontal", "h", "vertical", "v"]
                    },
                    "percent": {
                        "type": "integer",
                        "description": "Optional size percentage (0-100)",
                        "minimum": 0,
                        "maximum": 100
                    }
                },
                "required": ["direction"]
            }
        }),
        json!({
            "name": TOOL_READ_PANE,
            "description": "Capture the visible text content of a pane",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pane_id": {
                        "type": "integer",
                        "description": "The pane ID to read from",
                        "minimum": 0
                    }
                },
                "required": ["pane_id"]
            }
        }),
        json!({
            "name": TOOL_WRITE_PANE,
            "description": "Send text or keystrokes to a specific pane",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pane_id": {
                        "type": "integer",
                        "description": "The pane ID to send input to",
                        "minimum": 0
                    },
                    "keys": {
                        "type": "string",
                        "description": "Text or keystrokes to send"
                    }
                },
                "required": ["pane_id", "keys"]
            }
        }),
        json!({
            "name": TOOL_LIST_PANES,
            "description": "List all panes in the active session with their metadata",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_split_pane_horizontal() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_SPLIT_PANE.to_string(),
            params: json!({
                "arguments": {
                    "direction": "horizontal",
                    "percent": 50
                }
            }),
            id: json!(1),
        };
        let msg = parse_tool_call(&req).unwrap();
        match msg {
            ClientMessage::SplitPane { direction, size } => {
                assert_eq!(direction, SplitDirection::Horizontal);
                assert_eq!(size, Some(50));
            }
            _ => panic!("expected SplitPane"),
        }
    }

    #[test]
    fn parse_split_pane_vertical() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_SPLIT_PANE.to_string(),
            params: json!({
                "arguments": {
                    "direction": "vertical"
                }
            }),
            id: json!(2),
        };
        let msg = parse_tool_call(&req).unwrap();
        match msg {
            ClientMessage::SplitPane { direction, size } => {
                assert_eq!(direction, SplitDirection::Vertical);
                assert_eq!(size, None);
            }
            _ => panic!("expected SplitPane"),
        }
    }

    #[test]
    fn parse_split_pane_h_shorthand() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_SPLIT_PANE.to_string(),
            params: json!({
                "arguments": {
                    "direction": "h"
                }
            }),
            id: json!(3),
        };
        let msg = parse_tool_call(&req).unwrap();
        match msg {
            ClientMessage::SplitPane { direction, .. } => {
                assert_eq!(direction, SplitDirection::Horizontal);
            }
            _ => panic!("expected SplitPane"),
        }
    }

    #[test]
    fn parse_split_pane_v_shorthand() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_SPLIT_PANE.to_string(),
            params: json!({
                "arguments": {
                    "direction": "v"
                }
            }),
            id: json!(4),
        };
        let msg = parse_tool_call(&req).unwrap();
        match msg {
            ClientMessage::SplitPane { direction, .. } => {
                assert_eq!(direction, SplitDirection::Vertical);
            }
            _ => panic!("expected SplitPane"),
        }
    }

    #[test]
    fn parse_read_pane() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_READ_PANE.to_string(),
            params: json!({
                "arguments": {
                    "pane_id": 42
                }
            }),
            id: json!(5),
        };
        let msg = parse_tool_call(&req).unwrap();
        match msg {
            ClientMessage::CapturePane { pane_id } => {
                assert_eq!(pane_id, 42);
            }
            _ => panic!("expected CapturePane"),
        }
    }

    #[test]
    fn parse_write_pane() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_WRITE_PANE.to_string(),
            params: json!({
                "arguments": {
                    "pane_id": 7,
                    "keys": "echo hello"
                }
            }),
            id: json!(6),
        };
        let msg = parse_tool_call(&req).unwrap();
        match msg {
            ClientMessage::SendKeys { pane_id, keys } => {
                assert_eq!(pane_id, 7);
                assert_eq!(keys, "echo hello");
            }
            _ => panic!("expected SendKeys"),
        }
    }

    #[test]
    fn parse_list_panes() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_LIST_PANES.to_string(),
            params: json!({ "arguments": {} }),
            id: json!(7),
        };
        let msg = parse_tool_call(&req).unwrap();
        match msg {
            ClientMessage::ListPanes => {}
            _ => panic!("expected ListPanes"),
        }
    }

    #[test]
    fn parse_unknown_tool() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: "unknown_tool".to_string(),
            params: json!({ "arguments": {} }),
            id: json!(8),
        };
        let err = parse_tool_call(&req).unwrap_err();
        assert_eq!(err.code, ERR_METHOD_NOT_FOUND);
        assert!(err.message.contains("unknown"));
    }

    #[test]
    fn parse_split_pane_invalid_direction() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_SPLIT_PANE.to_string(),
            params: json!({
                "arguments": {
                    "direction": "diagonal"
                }
            }),
            id: json!(9),
        };
        let err = parse_tool_call(&req).unwrap_err();
        assert_eq!(err.code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn parse_missing_arguments() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_READ_PANE.to_string(),
            params: json!({}),
            id: json!(10),
        };
        let err = parse_tool_call(&req).unwrap_err();
        assert_eq!(err.code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn format_response_spawn_result() {
        let msg = ServerMessage::SpawnResult { pane_id: 99 };
        let resp = format_response(json!(1), &msg);
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.error.is_none());
        assert_eq!(resp.result.as_ref().unwrap()["pane_id"], 99);
        assert_eq!(resp.id, json!(1));
    }

    #[test]
    fn format_response_pane_captured() {
        let msg = ServerMessage::PaneCaptured {
            pane_id: 5,
            content: "hello world".to_string(),
        };
        let resp = format_response(json!(2), &msg);
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.error.is_none());
        assert_eq!(resp.result.as_ref().unwrap()["pane_id"], 5);
        assert_eq!(resp.result.as_ref().unwrap()["content"], "hello world");
    }

    #[test]
    fn format_response_pane_list() {
        use crate::messages::PaneEntry;
        let panes = vec![
            PaneEntry {
                id: 1,
                title: "shell".to_string(),
                cols: 80,
                rows: 24,
                active: true,
                has_notification: false,
            },
            PaneEntry {
                id: 2,
                title: "editor".to_string(),
                cols: 80,
                rows: 24,
                active: false,
                has_notification: true,
            },
        ];
        let msg = ServerMessage::PaneList { panes };
        let resp = format_response(json!(3), &msg);
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.error.is_none());
        let pane_array = &resp.result.as_ref().unwrap()["panes"];
        assert_eq!(pane_array.as_array().unwrap().len(), 2);
        assert_eq!(pane_array[0]["id"], 1);
        assert_eq!(pane_array[1]["title"], "editor");
    }

    #[test]
    fn format_response_error() {
        let msg = ServerMessage::Error {
            message: "something went wrong".to_string(),
        };
        let resp = format_response(json!(4), &msg);
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        // F3: server-side errors use ERR_INTERNAL (-32000), not ERR_INVALID_PARAMS
        assert_eq!(resp.error.as_ref().unwrap().code, ERR_INTERNAL);
    }

    #[test]
    fn error_response_has_error_field() {
        let resp = error_response(json!(5), ERR_INVALID_PARAMS, "bad input");
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().message, "bad input");
    }

    #[test]
    fn error_response_serializes_without_null_result() {
        let resp = error_response(json!(6), ERR_PARSE_ERROR, "parse error");
        let json_str = serde_json::to_string(&resp).unwrap();
        // Should not contain "result": null
        assert!(!json_str.contains("\"result\":null"));
    }

    #[test]
    fn tools_list_returns_four_tools() {
        let tools = tools_list();
        assert_eq!(tools.len(), 4);
    }

    #[test]
    fn tools_list_contains_split_pane() {
        let tools = tools_list();
        let names: Vec<&str> = tools.iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&TOOL_SPLIT_PANE));
    }

    #[test]
    fn tools_list_contains_read_pane() {
        let tools = tools_list();
        let names: Vec<&str> = tools.iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&TOOL_READ_PANE));
    }

    #[test]
    fn tools_list_contains_write_pane() {
        let tools = tools_list();
        let names: Vec<&str> = tools.iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&TOOL_WRITE_PANE));
    }

    #[test]
    fn tools_list_contains_list_panes() {
        let tools = tools_list();
        let names: Vec<&str> = tools.iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&TOOL_LIST_PANES));
    }

    #[test]
    fn tools_list_each_has_description() {
        let tools = tools_list();
        for tool in tools {
            assert!(tool.get("description").is_some());
        }
    }

    #[test]
    fn tools_list_each_has_input_schema() {
        let tools = tools_list();
        for tool in tools {
            assert!(tool.get("inputSchema").is_some());
        }
    }

    #[test]
    fn mcp_response_has_jsonrpc_field() {
        let resp = format_response(json!(10), &ServerMessage::Ack);
        assert_eq!(resp.jsonrpc, "2.0");
    }

    #[test]
    fn mcp_error_new_constructor() {
        let err = McpError::new(ERR_INVALID_PARAMS, "test error");
        assert_eq!(err.code, ERR_INVALID_PARAMS);
        assert_eq!(err.message, "test error");
    }

    #[test]
    fn constants_have_correct_values() {
        assert_eq!(JSONRPC_VERSION, "2.0");
        assert_eq!(ERR_PARSE_ERROR, -32700);
        assert_eq!(ERR_METHOD_NOT_FOUND, -32601);
        assert_eq!(ERR_INVALID_PARAMS, -32602);
        // F3: application error range
        assert_eq!(ERR_INTERNAL, -32000);
    }

    // F2: percent range validation tests
    #[test]
    fn parse_split_pane_percent_max_valid() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_SPLIT_PANE.to_string(),
            params: json!({ "arguments": { "direction": "h", "percent": 100 } }),
            id: json!(20),
        };
        let msg = parse_tool_call(&req).unwrap();
        match msg {
            ClientMessage::SplitPane { size, .. } => assert_eq!(size, Some(100)),
            _ => panic!("expected SplitPane"),
        }
    }

    #[test]
    fn parse_split_pane_percent_zero_valid() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_SPLIT_PANE.to_string(),
            params: json!({ "arguments": { "direction": "h", "percent": 0 } }),
            id: json!(21),
        };
        let msg = parse_tool_call(&req).unwrap();
        match msg {
            ClientMessage::SplitPane { size, .. } => assert_eq!(size, Some(0)),
            _ => panic!("expected SplitPane"),
        }
    }

    #[test]
    fn parse_split_pane_percent_out_of_range_returns_error() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_SPLIT_PANE.to_string(),
            params: json!({ "arguments": { "direction": "h", "percent": 101 } }),
            id: json!(22),
        };
        let err = parse_tool_call(&req).unwrap_err();
        assert_eq!(err.code, ERR_INVALID_PARAMS);
        assert!(err.message.contains("0-100"));
    }

    #[test]
    fn parse_split_pane_percent_large_value_returns_error() {
        // Previously would silently truncate 70000 → 4464 via u16 wrapping
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: TOOL_SPLIT_PANE.to_string(),
            params: json!({ "arguments": { "direction": "v", "percent": 70000 } }),
            id: json!(23),
        };
        let err = parse_tool_call(&req).unwrap_err();
        assert_eq!(err.code, ERR_INVALID_PARAMS);
    }

    // F3: ERR_INTERNAL is used for server-side errors, not ERR_INVALID_PARAMS
    #[test]
    fn format_response_unsupported_variant_uses_internal_error() {
        let resp = format_response(json!(30), &ServerMessage::Pong);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, ERR_INTERNAL);
    }

    #[test]
    fn tool_name_constants_are_correct() {
        assert_eq!(TOOL_SPLIT_PANE, "split_pane");
        assert_eq!(TOOL_READ_PANE, "read_pane");
        assert_eq!(TOOL_WRITE_PANE, "write_pane");
        assert_eq!(TOOL_LIST_PANES, "list_panes");
    }

    #[test]
    fn mcp_notification_new() {
        let notif = McpNotification::new("test_method", json!({"key": "value"}));
        assert_eq!(notif.jsonrpc, "2.0");
        assert_eq!(notif.method, "test_method");
        assert_eq!(notif.params, json!({"key": "value"}));
    }

    #[test]
    fn mcp_notification_serialize() {
        let notif = McpNotification::new("my_event", json!({"id": 42}));
        let serialized = serde_json::to_string(&notif).unwrap();
        assert!(serialized.contains("\"jsonrpc\":\"2.0\""));
        assert!(serialized.contains("\"method\":\"my_event\""));
        assert!(serialized.contains("\"id\":42"));
        assert!(!serialized.contains("\"id\":null")); // No null id
    }

    #[test]
    fn mcp_notification_deserialize() {
        let json_str = r#"{"jsonrpc":"2.0","method":"event","params":{"x":1}}"#;
        let notif: McpNotification = serde_json::from_str(json_str).unwrap();
        assert_eq!(notif.jsonrpc, "2.0");
        assert_eq!(notif.method, "event");
        assert_eq!(notif.params, json!({"x": 1}));
    }

    #[test]
    fn mcp_notification_has_no_id_field() {
        let notif = McpNotification::new("test", json!({}));
        let serialized = serde_json::to_string(&notif).unwrap();
        // Verify the JSON doesn't have an "id" field (notifications have no id)
        assert!(!serialized.contains("\"id\""));
    }

    #[test]
    fn to_notification_pty_output() {
        let msg = ServerMessage::PtyOutput {
            pane_id: 3,
            data: vec![72, 101, 108, 108, 111], // "Hello"
        };
        let notif = to_notification(&msg).unwrap();
        assert_eq!(notif.method, "pty_output");
        assert_eq!(notif.params.get("pane_id").unwrap(), 3);
        assert!(notif.params.get("data").unwrap().as_str().unwrap().contains("Hello"));
    }

    #[test]
    fn to_notification_layout_changed() {
        let msg = ServerMessage::LayoutChanged;
        let notif = to_notification(&msg).unwrap();
        assert_eq!(notif.method, "layout_changed");
        assert_eq!(notif.params, json!({}));
    }

    #[test]
    fn to_notification_session_ended() {
        let msg = ServerMessage::SessionEnded;
        let notif = to_notification(&msg).unwrap();
        assert_eq!(notif.method, "session_ended");
        assert_eq!(notif.params, json!({}));
    }

    #[test]
    fn to_notification_returns_none_for_non_streaming_messages() {
        assert!(to_notification(&ServerMessage::Ack).is_none());
        assert!(to_notification(&ServerMessage::Pong).is_none());
        assert!(to_notification(&ServerMessage::Error { message: "test".to_string() }).is_none());
    }

    #[test]
    fn to_notification_with_empty_data() {
        let msg = ServerMessage::PtyOutput {
            pane_id: 1,
            data: vec![],
        };
        let notif = to_notification(&msg).unwrap();
        assert_eq!(notif.params.get("data").unwrap().as_str().unwrap(), "");
    }

    #[test]
    fn to_notification_pty_output_preserves_pane_id() {
        for pane_id in [0u32, 1, 42, 1000, u32::MAX] {
            let msg = ServerMessage::PtyOutput {
                pane_id,
                data: vec![65], // 'A'
            };
            let notif = to_notification(&msg).unwrap();
            assert_eq!(notif.params.get("pane_id").unwrap().as_u64().unwrap(), pane_id as u64);
        }
    }
}
