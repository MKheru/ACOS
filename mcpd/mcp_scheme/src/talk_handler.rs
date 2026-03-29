//! Talk Handler: conversational AI terminal for ACOS
//!
//! Manages conversations with history, system prompts, and dispatches
//! to mcp:ai for LLM responses. Core service for the mcp-talk terminal.

use std::sync::Mutex;

use serde_json::{json, Value};

use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, INVALID_PARAMS, METHOD_NOT_FOUND};
use crate::McpPath;

/// Dispatch function type: (service, method, params) -> JsonRpcResponse
type DispatchFn = Box<dyn Fn(&str, &str, Value) -> JsonRpcResponse + Send + Sync>;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_HISTORY_LEN: usize = 200;
const MAX_PROMPT_LEN: usize = 32768;
const MAX_MESSAGE_LEN: usize = 16384;
const MAX_CONVERSATIONS: usize = 100;
const ACCESS_DENIED: i64 = -32001;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    User,
    Assistant,
    System,
    ToolResult,
}

impl Role {
    fn as_str(&self) -> &str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
            Role::ToolResult => "tool_result",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub timestamp: String,
}

pub struct Conversation {
    pub id: u32,
    pub history: Vec<Message>,
    pub system_prompt: String,
    pub owner: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// DRY helpers (F12)
// ---------------------------------------------------------------------------

fn extract_conv_id(params: &Value, request_id: &Option<Value>) -> Result<u32, JsonRpcResponse> {
    match params.get("conversation_id").and_then(|v| v.as_u64()) {
        Some(id) => Ok(id as u32),
        None => Err(JsonRpcResponse::error(
            request_id.clone(),
            INVALID_PARAMS,
            "missing required parameter 'conversation_id'",
        )),
    }
}

fn extract_string(
    params: &Value,
    key: &str,
    request_id: &Option<Value>,
) -> Result<String, JsonRpcResponse> {
    match params.get(key).and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => Err(JsonRpcResponse::error(
            request_id.clone(),
            INVALID_PARAMS,
            format!("missing or empty '{}' parameter", key),
        )),
    }
}

fn extract_owner(params: &Value, request_id: &Option<Value>) -> Result<String, JsonRpcResponse> {
    match params.get("owner").and_then(|v| v.as_str()) {
        Some(o) if !o.is_empty() => Ok(o.to_string()),
        _ => Err(JsonRpcResponse::error(
            request_id.clone(),
            INVALID_PARAMS,
            "missing required parameter 'owner'",
        )),
    }
}

fn check_owner(
    conv: &Conversation,
    owner: &str,
    request_id: &Option<Value>,
) -> Result<(), JsonRpcResponse> {
    if conv.owner != owner {
        Err(JsonRpcResponse::error(
            request_id.clone(),
            ACCESS_DENIED,
            "access denied: owner mismatch",
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TalkHandler
// ---------------------------------------------------------------------------

pub struct TalkHandler {
    conversations: Mutex<Vec<Conversation>>,
    dispatch: DispatchFn,
    next_id: Mutex<u32>,
}

impl TalkHandler {
    pub fn new(dispatch: DispatchFn) -> Self {
        TalkHandler {
            conversations: Mutex::new(Vec::new()),
            dispatch,
            next_id: Mutex::new(1),
        }
    }

    fn handle_create(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let owner = request
            .params
            .get("owner")
            .and_then(|v| v.as_str())
            .unwrap_or("anonymous")
            .to_string();

        // F8: conversation count limit
        let mut convs = self.conversations.lock().unwrap();
        if convs.len() >= MAX_CONVERSATIONS {
            return JsonRpcResponse::error(
                request.id.clone(),
                INVALID_PARAMS,
                format!(
                    "conversation limit reached (max {})",
                    MAX_CONVERSATIONS
                ),
            );
        }

        let mut next_id = self.next_id.lock().unwrap();
        let id = *next_id;
        *next_id += 1;

        let conv = Conversation {
            id,
            history: Vec::new(),
            system_prompt: String::new(),
            owner,
            created_at: String::new(),
        };

        convs.push(conv);

        JsonRpcResponse::success(request.id.clone(), json!({ "conversation_id": id }))
    }

    fn handle_ask(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let conv_id = match extract_conv_id(&request.params, &request.id) {
            Ok(id) => id,
            Err(e) => return e,
        };

        let owner = match extract_owner(&request.params, &request.id) {
            Ok(o) => o,
            Err(e) => return e,
        };

        let message = match extract_string(&request.params, "message", &request.id) {
            Ok(m) => m,
            Err(e) => return e,
        };

        // F9: message length limit
        if message.len() > MAX_MESSAGE_LEN {
            return JsonRpcResponse::error(
                request.id.clone(),
                INVALID_PARAMS,
                format!("message too long (max {} bytes)", MAX_MESSAGE_LEN),
            );
        }

        // F6: hold lock for entire duration
        let mut convs = self.conversations.lock().unwrap();
        let conv = match convs.iter_mut().find(|c| c.id == conv_id) {
            Some(c) => c,
            None => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    format!("conversation {} not found", conv_id),
                )
            }
        };

        // F3: access control
        if let Err(e) = check_owner(conv, &owner, &request.id) {
            return e;
        }

        // F10: prompt injection mitigation — wrap user message
        let wrapped_message = format!(
            "[USER_MESSAGE_START]{}[USER_MESSAGE_END]",
            message
        );

        conv.history.push(Message {
            role: Role::User,
            content: message.clone(),
            timestamp: String::new(),
        });

        // F4: trim history if over limit
        if conv.history.len() > MAX_HISTORY_LEN {
            let excess = conv.history.len() - MAX_HISTORY_LEN;
            conv.history.drain(..excess);
        }

        // Build prompt from system_prompt + history
        let mut prompt = String::new();
        if !conv.system_prompt.is_empty() {
            prompt.push_str(&conv.system_prompt);
            prompt.push_str("\n\n");
        }
        for msg in &conv.history {
            match msg.role {
                Role::User => {
                    prompt.push_str("User: ");
                    // F10: use wrapped delimiter for last user message, raw for history
                    if msg.content == message {
                        prompt.push_str(&wrapped_message);
                    } else {
                        prompt.push_str(&format!(
                            "[USER_MESSAGE_START]{}[USER_MESSAGE_END]",
                            msg.content
                        ));
                    }
                    prompt.push('\n');
                }
                Role::Assistant => {
                    prompt.push_str("Assistant: ");
                    prompt.push_str(&msg.content);
                    prompt.push('\n');
                }
                _ => {}
            }
        }

        // F4: truncate prompt to MAX_PROMPT_LEN
        if prompt.len() > MAX_PROMPT_LEN {
            prompt.truncate(MAX_PROMPT_LEN);
        }

        // Dispatch to ai service
        let ai_response = (self.dispatch)("ai", "ask", json!({ "prompt": prompt }));

        // F5: only append assistant message if dispatch succeeded with non-empty text
        let response_text = ai_response
            .result
            .as_ref()
            .and_then(|r| r.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        let tool_calls = ai_response
            .result
            .as_ref()
            .and_then(|r| r.get("tool_calls"))
            .cloned()
            .unwrap_or(json!([]));

        if ai_response.error.is_some() || response_text.is_empty() {
            // F5: don't corrupt history — remove the user message we just added
            conv.history.pop();
            return if let Some(err) = ai_response.error {
                JsonRpcResponse::error(
                    request.id.clone(),
                    err.code,
                    err.message,
                )
            } else {
                JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    "AI returned empty response",
                )
            };
        }

        conv.history.push(Message {
            role: Role::Assistant,
            content: response_text.clone(),
            timestamp: String::new(),
        });

        JsonRpcResponse::success(
            request.id.clone(),
            json!({
                "response": response_text,
                "tool_calls": tool_calls,
            }),
        )
    }

    fn handle_history(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let conv_id = match extract_conv_id(&request.params, &request.id) {
            Ok(id) => id,
            Err(e) => return e,
        };

        let owner = match extract_owner(&request.params, &request.id) {
            Ok(o) => o,
            Err(e) => return e,
        };

        let count = request
            .params
            .get("count")
            .and_then(|v| v.as_u64())
            .map(|c| c as usize);

        let convs = self.conversations.lock().unwrap();
        let conv = match convs.iter().find(|c| c.id == conv_id) {
            Some(c) => c,
            None => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    format!("conversation {} not found", conv_id),
                )
            }
        };

        // F3: access control
        if let Err(e) = check_owner(conv, &owner, &request.id) {
            return e;
        }

        // F13: efficient history retrieval via slice
        let n = count.unwrap_or(conv.history.len());
        let slice = &conv.history[conv.history.len().saturating_sub(n)..];
        let messages: Vec<Value> = slice
            .iter()
            .map(|m| {
                json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                    "timestamp": m.timestamp,
                })
            })
            .collect();

        JsonRpcResponse::success(request.id.clone(), json!({ "messages": messages }))
    }

    fn handle_list(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let convs = self.conversations.lock().unwrap();
        let list: Vec<Value> = convs
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "owner": c.owner,
                    "message_count": c.history.len(),
                })
            })
            .collect();

        JsonRpcResponse::success(request.id.clone(), json!({ "conversations": list }))
    }

    fn handle_clear(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let conv_id = match extract_conv_id(&request.params, &request.id) {
            Ok(id) => id,
            Err(e) => return e,
        };

        let owner = match extract_owner(&request.params, &request.id) {
            Ok(o) => o,
            Err(e) => return e,
        };

        let mut convs = self.conversations.lock().unwrap();
        match convs.iter_mut().find(|c| c.id == conv_id) {
            Some(conv) => {
                // F3: access control
                if let Err(e) = check_owner(conv, &owner, &request.id) {
                    return e;
                }
                conv.history.clear();
                JsonRpcResponse::success(request.id.clone(), json!({ "ok": true }))
            }
            None => JsonRpcResponse::error(
                request.id.clone(),
                INVALID_PARAMS,
                format!("conversation {} not found", conv_id),
            ),
        }
    }

    fn handle_system_prompt(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let conv_id = match extract_conv_id(&request.params, &request.id) {
            Ok(id) => id,
            Err(e) => return e,
        };

        let owner = match extract_owner(&request.params, &request.id) {
            Ok(o) => o,
            Err(e) => return e,
        };

        let prompt = match extract_string(&request.params, "prompt", &request.id) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let mut convs = self.conversations.lock().unwrap();
        match convs.iter_mut().find(|c| c.id == conv_id) {
            Some(conv) => {
                // F3: access control
                if let Err(e) = check_owner(conv, &owner, &request.id) {
                    return e;
                }
                conv.system_prompt = prompt;
                JsonRpcResponse::success(request.id.clone(), json!({ "ok": true }))
            }
            None => JsonRpcResponse::error(
                request.id.clone(),
                INVALID_PARAMS,
                format!("conversation {} not found", conv_id),
            ),
        }
    }
}

impl ServiceHandler for TalkHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "create" => self.handle_create(request),
            "ask" | "send" => self.handle_ask(request),
            "history" => self.handle_history(request),
            "list" => self.handle_list(request),
            "clear" => self.handle_clear(request),
            "system_prompt" => self.handle_system_prompt(request),
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in talk service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["create", "ask", "send", "history", "list", "clear", "system_prompt"]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mock_dispatch() -> DispatchFn {
        Box::new(|_service, _method, _params| {
            JsonRpcResponse::success(
                Some(json!(1)),
                json!({
                    "text": "Mock AI response",
                    "tool_calls": [],
                }),
            )
        })
    }

    fn failing_dispatch() -> DispatchFn {
        Box::new(|_service, _method, _params| {
            JsonRpcResponse::error(Some(json!(1)), INVALID_PARAMS, "AI service unavailable")
        })
    }

    fn empty_dispatch() -> DispatchFn {
        Box::new(|_service, _method, _params| {
            JsonRpcResponse::success(
                Some(json!(1)),
                json!({
                    "text": "",
                    "tool_calls": [],
                }),
            )
        })
    }

    fn make_request(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id: Some(json!(1)),
        }
    }

    fn path() -> McpPath {
        McpPath::parse(b"talk/test").unwrap()
    }

    #[test]
    fn test_create_returns_conversation_id() {
        let handler = TalkHandler::new(mock_dispatch());
        let req = make_request("create", json!({ "owner": "user1" }));
        let resp = handler.handle(&path(), &req);
        let id = resp
            .result
            .unwrap()
            .get("conversation_id")
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn test_ask_with_mock_dispatch() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "ask",
            json!({
                "conversation_id": 1,
                "message": "Hello AI",
                "owner": "user1"
            }),
        );
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(
            result.get("response").unwrap().as_str().unwrap(),
            "Mock AI response"
        );
    }

    #[test]
    fn test_history_returns_messages() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Hi", "owner": "user1" }),
        );
        handler.handle(&path(), &req);

        let req = make_request(
            "history",
            json!({ "conversation_id": 1, "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        let messages = resp
            .result
            .unwrap()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(messages.len(), 2); // user + assistant
        assert_eq!(
            messages[0].get("role").unwrap().as_str().unwrap(),
            "user"
        );
        assert_eq!(
            messages[1].get("role").unwrap().as_str().unwrap(),
            "assistant"
        );
    }

    #[test]
    fn test_list_returns_conversations() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "alice" }));
        handler.handle(&path(), &req);
        let req = make_request("create", json!({ "owner": "bob" }));
        handler.handle(&path(), &req);

        let req = make_request("list", json!({}));
        let resp = handler.handle(&path(), &req);
        let convs = resp
            .result
            .unwrap()
            .get("conversations")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(convs.len(), 2);
        assert_eq!(
            convs[0].get("owner").unwrap().as_str().unwrap(),
            "alice"
        );
        assert_eq!(
            convs[1].get("owner").unwrap().as_str().unwrap(),
            "bob"
        );
    }

    #[test]
    fn test_clear_empties_history() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Hi", "owner": "user1" }),
        );
        handler.handle(&path(), &req);

        let req = make_request(
            "clear",
            json!({ "conversation_id": 1, "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        assert_eq!(
            resp.result.unwrap().get("ok").unwrap().as_bool().unwrap(),
            true
        );

        let req = make_request(
            "history",
            json!({ "conversation_id": 1, "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        let messages = resp
            .result
            .unwrap()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(messages.len(), 0);
    }

    #[test]
    fn test_system_prompt() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "system_prompt",
            json!({
                "conversation_id": 1,
                "prompt": "You are a helpful ACOS assistant.",
                "owner": "user1"
            }),
        );
        let resp = handler.handle(&path(), &req);
        assert_eq!(
            resp.result.unwrap().get("ok").unwrap().as_bool().unwrap(),
            true
        );
    }

    #[test]
    fn test_ask_appends_to_history() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Hello", "owner": "user1" }),
        );
        handler.handle(&path(), &req);

        let req = make_request(
            "history",
            json!({ "conversation_id": 1, "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        let messages = resp
            .result
            .unwrap()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].get("role").unwrap().as_str().unwrap(),
            "user"
        );
        assert_eq!(
            messages[0].get("content").unwrap().as_str().unwrap(),
            "Hello"
        );
        assert_eq!(
            messages[1].get("role").unwrap().as_str().unwrap(),
            "assistant"
        );
        assert_eq!(
            messages[1].get("content").unwrap().as_str().unwrap(),
            "Mock AI response"
        );
    }

    #[test]
    fn test_multiple_conversations_are_independent() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "alice" }));
        handler.handle(&path(), &req);
        let req = make_request("create", json!({ "owner": "bob" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Alice msg", "owner": "alice" }),
        );
        handler.handle(&path(), &req);

        // conv 2 should be empty
        let req = make_request(
            "history",
            json!({ "conversation_id": 2, "owner": "bob" }),
        );
        let resp = handler.handle(&path(), &req);
        let messages = resp
            .result
            .unwrap()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(messages.len(), 0);

        // conv 1 should have 2 messages
        let req = make_request(
            "history",
            json!({ "conversation_id": 1, "owner": "alice" }),
        );
        let resp = handler.handle(&path(), &req);
        let messages = resp
            .result
            .unwrap()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn test_ask_nonexistent_conversation() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request(
            "ask",
            json!({ "conversation_id": 999, "message": "Hello", "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_clear_nonexistent_conversation() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request(
            "clear",
            json!({ "conversation_id": 999, "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_history_with_count() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        // Ask twice to get 4 messages (2 user + 2 assistant)
        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "First", "owner": "user1" }),
        );
        handler.handle(&path(), &req);
        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Second", "owner": "user1" }),
        );
        handler.handle(&path(), &req);

        // Request only last 2 messages
        let req = make_request(
            "history",
            json!({ "conversation_id": 1, "count": 2, "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        let messages = resp
            .result
            .unwrap()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].get("role").unwrap().as_str().unwrap(),
            "user"
        );
        assert_eq!(
            messages[0].get("content").unwrap().as_str().unwrap(),
            "Second"
        );
    }

    #[test]
    fn test_conversation_preserves_context() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Turn 1", "owner": "user1" }),
        );
        handler.handle(&path(), &req);
        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Turn 2", "owner": "user1" }),
        );
        handler.handle(&path(), &req);

        let req = make_request(
            "history",
            json!({ "conversation_id": 1, "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        let messages = resp
            .result
            .unwrap()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        // 2 user + 2 assistant = 4
        assert_eq!(messages.len(), 4);
        assert_eq!(
            messages[0].get("content").unwrap().as_str().unwrap(),
            "Turn 1"
        );
        assert_eq!(
            messages[2].get("content").unwrap().as_str().unwrap(),
            "Turn 2"
        );
    }

    // --- New tests for review findings ---

    #[test]
    fn test_access_denied_wrong_owner() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "alice" }));
        handler.handle(&path(), &req);

        // Bob tries to ask on alice's conversation
        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Hi", "owner": "bob" }),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(err.code, ACCESS_DENIED);
    }

    #[test]
    fn test_access_denied_history() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "alice" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "history",
            json!({ "conversation_id": 1, "owner": "bob" }),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_access_denied_clear() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "alice" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "clear",
            json!({ "conversation_id": 1, "owner": "bob" }),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_access_denied_system_prompt() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "alice" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "system_prompt",
            json!({ "conversation_id": 1, "prompt": "hack", "owner": "bob" }),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_conversation_limit() {
        let handler = TalkHandler::new(mock_dispatch());

        for i in 0..MAX_CONVERSATIONS {
            let req = make_request("create", json!({ "owner": format!("user{}", i) }));
            let resp = handler.handle(&path(), &req);
            assert!(resp.result.is_some());
        }

        // Next one should fail
        let req = make_request("create", json!({ "owner": "overflow" }));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_message_length_limit() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        let long_msg = "x".repeat(MAX_MESSAGE_LEN + 1);
        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": long_msg, "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_ai_failure_does_not_corrupt_history() {
        let handler = TalkHandler::new(failing_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Hello", "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());

        // History should be empty — failed message rolled back
        let req = make_request(
            "history",
            json!({ "conversation_id": 1, "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        let messages = resp
            .result
            .unwrap()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(messages.len(), 0);
    }

    #[test]
    fn test_empty_ai_response_does_not_corrupt_history() {
        let handler = TalkHandler::new(empty_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Hello", "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());

        let req = make_request(
            "history",
            json!({ "conversation_id": 1, "owner": "user1" }),
        );
        let resp = handler.handle(&path(), &req);
        let messages = resp
            .result
            .unwrap()
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(messages.len(), 0);
    }

    #[test]
    fn test_ask_missing_owner() {
        let handler = TalkHandler::new(mock_dispatch());

        let req = make_request("create", json!({ "owner": "user1" }));
        handler.handle(&path(), &req);

        let req = make_request(
            "ask",
            json!({ "conversation_id": 1, "message": "Hello" }),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }
}
