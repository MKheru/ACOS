//! LLM service handler: mcp://llm/generate, mcp://llm/info
//!
//! Routes ALL LLM requests through net.llm_request, which handles
//! platform-specific connectivity (Ollama on host, QEMU bridge on ACOS).
//! The net handler returns OpenAI-compatible responses.

use serde_json::{json, Value};

use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, METHOD_NOT_FOUND, INVALID_PARAMS, INTERNAL_ERROR};
use crate::McpPath;

/// Dispatch function type: (service, method, params) -> JsonRpcResponse
type DispatchFn = Box<dyn Fn(&str, &str, Value) -> JsonRpcResponse + Send + Sync>;

const MAX_PROMPT_LEN: usize = 32768;

/// MCP service handler for LLM inference via net.llm_request dispatch.
pub struct LlmHandler {
    dispatch: DispatchFn,
}

impl LlmHandler {
    pub fn new(dispatch: DispatchFn) -> Self {
        LlmHandler { dispatch }
    }

    /// Extract text content from net.llm_request response.
    ///
    /// The net handler returns an OpenAI-compatible structure. We try:
    /// 1. choices[0].message.content (full OpenAI format)
    /// 2. message.content (simplified format from net handler mock)
    /// 3. content (direct content field)
    fn extract_content(result: &Value) -> Option<&str> {
        // OpenAI full format: {"choices": [{"message": {"content": "..."}}]}
        result.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
        // Simplified format: {"message": {"content": "..."}}
        .or_else(|| {
            result.get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
        })
        // Direct content field
        .or_else(|| {
            result.get("content")
                .and_then(|c| c.as_str())
        })
    }
}

impl ServiceHandler for LlmHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "generate" | "stream" => {
                let prompt = match request.params.get("prompt").and_then(|v| v.as_str()) {
                    Some(p) if !p.is_empty() && p.len() <= MAX_PROMPT_LEN => p,
                    Some(p) if p.is_empty() => return JsonRpcResponse::error(
                        request.id.clone(), INVALID_PARAMS, "prompt must not be empty",
                    ),
                    Some(p) => return JsonRpcResponse::error(
                        request.id.clone(), INVALID_PARAMS,
                        format!("prompt too long ({} bytes, max {})", p.len(), MAX_PROMPT_LEN),
                    ),
                    None => return JsonRpcResponse::error(
                        request.id.clone(), INVALID_PARAMS, "missing required parameter 'prompt' (string)",
                    ),
                };

                let max_tokens = request.params
                    .get("max_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(256)
                    .min(2048) as usize;

                // Build OpenAI messages format
                let messages = json!([{"role": "user", "content": prompt}]);

                // Dispatch to net.llm_request
                let resp = (self.dispatch)("net", "llm_request", json!({
                    "model": "phi4-mini",
                    "messages": messages,
                    "max_tokens": max_tokens,
                }));

                // Check for dispatch error
                if let Some(err) = &resp.error {
                    return JsonRpcResponse::error(
                        request.id.clone(), INTERNAL_ERROR,
                        format!("LLM dispatch error: {}", err.message),
                    );
                }

                // Extract content from the response
                match &resp.result {
                    Some(result) => {
                        if let Some(content) = Self::extract_content(result) {
                            JsonRpcResponse::success(request.id.clone(), json!({
                                "text": content,
                                "model": result.get("model").and_then(|m| m.as_str()).unwrap_or("phi4-mini"),
                                "finish_reason": result.get("finish_reason").and_then(|f| f.as_str()).unwrap_or("stop"),
                            }))
                        } else {
                            // Return the raw result if we can't extract content
                            JsonRpcResponse::success(request.id.clone(), result.clone())
                        }
                    }
                    None => JsonRpcResponse::error(
                        request.id.clone(), INTERNAL_ERROR, "empty response from net.llm_request",
                    ),
                }
            }

            "info" => {
                // Dispatch info query to net.llm_request
                let resp = (self.dispatch)("net", "llm_request", json!({
                    "model": "phi4-mini",
                    "messages": [{"role": "user", "content": "info"}],
                    "max_tokens": 1,
                }));

                match (&resp.result, &resp.error) {
                    (Some(result), None) => {
                        JsonRpcResponse::success(request.id.clone(), json!({
                            "model_name": result.get("model").and_then(|m| m.as_str()).unwrap_or("phi4-mini"),
                            "backend": "net.llm_request",
                            "status": "connected",
                        }))
                    }
                    _ => JsonRpcResponse::success(request.id.clone(), json!({
                        "model_name": "phi4-mini",
                        "backend": "net.llm_request",
                        "status": "disconnected",
                    })),
                }
            }

            _ => JsonRpcResponse::error(
                request.id.clone(), METHOD_NOT_FOUND,
                format!("Method '{}' not found in llm service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["generate", "info", "stream"]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Mock dispatch returning simplified net handler format
    fn mock_dispatch() -> DispatchFn {
        Box::new(|service, method, _params| {
            assert_eq!(service, "net");
            assert_eq!(method, "llm_request");
            JsonRpcResponse::success(
                None,
                json!({
                    "message": {"role": "assistant", "content": "mock response"},
                    "model": "phi4-mini",
                    "finish_reason": "stop",
                }),
            )
        })
    }

    /// Mock dispatch returning full OpenAI choices format
    fn mock_dispatch_openai() -> DispatchFn {
        Box::new(|_service, _method, _params| {
            JsonRpcResponse::success(
                None,
                json!({
                    "choices": [{
                        "message": {"role": "assistant", "content": "openai format response"},
                        "finish_reason": "stop",
                    }],
                    "model": "phi4-mini",
                }),
            )
        })
    }

    /// Mock dispatch that returns an error
    fn failing_dispatch() -> DispatchFn {
        Box::new(|_service, _method, _params| {
            JsonRpcResponse::error(None, INTERNAL_ERROR, "net service unavailable")
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
        McpPath::parse(b"llm/test").unwrap()
    }

    // --- Test 1: generate with simplified net response ---
    #[test]
    fn test_generate_extracts_content_from_message() {
        let handler = LlmHandler::new(mock_dispatch());
        let req = make_request("generate", json!({"prompt": "Hello"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["text"], "mock response");
        assert_eq!(result["model"], "phi4-mini");
        assert_eq!(result["finish_reason"], "stop");
    }

    // --- Test 2: generate with full OpenAI choices format ---
    #[test]
    fn test_generate_extracts_content_from_openai_choices() {
        let handler = LlmHandler::new(mock_dispatch_openai());
        let req = make_request("generate", json!({"prompt": "Hello"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["text"], "openai format response");
    }

    // --- Test 3: generate with dispatch error ---
    #[test]
    fn test_generate_dispatch_error() {
        let handler = LlmHandler::new(failing_dispatch());
        let req = make_request("generate", json!({"prompt": "Hello"}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("LLM dispatch error"));
    }

    // --- Test 4: generate with empty prompt ---
    #[test]
    fn test_generate_empty_prompt() {
        let handler = LlmHandler::new(mock_dispatch());
        let req = make_request("generate", json!({"prompt": ""}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    // --- Test 5: generate with missing prompt ---
    #[test]
    fn test_generate_missing_prompt() {
        let handler = LlmHandler::new(mock_dispatch());
        let req = make_request("generate", json!({}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    // --- Test 6: generate with prompt too long ---
    #[test]
    fn test_generate_prompt_too_long() {
        let handler = LlmHandler::new(mock_dispatch());
        let long = "x".repeat(MAX_PROMPT_LEN + 1);
        let req = make_request("generate", json!({"prompt": long}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("too long"));
    }

    // --- Test 7: info method returns model info ---
    #[test]
    fn test_info_connected() {
        let handler = LlmHandler::new(mock_dispatch());
        let req = make_request("info", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["model_name"], "phi4-mini");
        assert_eq!(result["backend"], "net.llm_request");
        assert_eq!(result["status"], "connected");
    }

    // --- Test 8: info with failing dispatch ---
    #[test]
    fn test_info_disconnected() {
        let handler = LlmHandler::new(failing_dispatch());
        let req = make_request("info", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["status"], "disconnected");
    }

    // --- Test 9: unknown method ---
    #[test]
    fn test_unknown_method() {
        let handler = LlmHandler::new(mock_dispatch());
        let req = make_request("nonexistent", json!({}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, METHOD_NOT_FOUND);
    }

    // --- Test 10: stream method works like generate ---
    #[test]
    fn test_stream_method() {
        let handler = LlmHandler::new(mock_dispatch());
        let req = make_request("stream", json!({"prompt": "Hi"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["text"], "mock response");
    }

    // --- Test 11: max_tokens is capped at 2048 ---
    #[test]
    fn test_max_tokens_capped() {
        let captured_params = std::sync::Arc::new(std::sync::Mutex::new(json!(null)));
        let params_clone = captured_params.clone();
        let dispatch: DispatchFn = Box::new(move |_service, _method, params| {
            *params_clone.lock().unwrap() = params.clone();
            JsonRpcResponse::success(
                None,
                json!({"message": {"role": "assistant", "content": "ok"}, "model": "phi4-mini"}),
            )
        });
        let handler = LlmHandler::new(dispatch);
        let req = make_request("generate", json!({"prompt": "Hi", "max_tokens": 99999}));
        handler.handle(&path(), &req);
        let captured = captured_params.lock().unwrap();
        assert_eq!(captured["max_tokens"], 2048);
    }

    // --- Test 12: dispatch receives correct model and messages ---
    #[test]
    fn test_dispatch_params_format() {
        let captured_params = std::sync::Arc::new(std::sync::Mutex::new(json!(null)));
        let params_clone = captured_params.clone();
        let dispatch: DispatchFn = Box::new(move |service, method, params| {
            assert_eq!(service, "net");
            assert_eq!(method, "llm_request");
            *params_clone.lock().unwrap() = params.clone();
            JsonRpcResponse::success(
                None,
                json!({"message": {"role": "assistant", "content": "ok"}, "model": "phi4-mini"}),
            )
        });
        let handler = LlmHandler::new(dispatch);
        let req = make_request("generate", json!({"prompt": "test prompt", "max_tokens": 100}));
        handler.handle(&path(), &req);
        let captured = captured_params.lock().unwrap();
        assert_eq!(captured["model"], "phi4-mini");
        assert_eq!(captured["messages"][0]["role"], "user");
        assert_eq!(captured["messages"][0]["content"], "test prompt");
        assert_eq!(captured["max_tokens"], 100);
    }

    // --- Test 13: list_methods ---
    #[test]
    fn test_list_methods() {
        let handler = LlmHandler::new(mock_dispatch());
        let methods = handler.list_methods();
        assert!(methods.contains(&"generate"));
        assert!(methods.contains(&"info"));
        assert!(methods.contains(&"stream"));
    }
}
