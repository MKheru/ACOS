//! Net service handler: central network layer for ACOS
//!
//! ALL network traffic in ACOS goes through this MCP service, making every
//! connection visible to Guardian. Methods:
//!   - http_get, http_post, tcp_connect, dns_resolve, ping
//!   - status, firewall_check, llm_request
//!
//! On Redox: real network via curl/tcp:/dns CLI tools
//! On host:  mock responses for testing

use serde_json::{json, Value};

use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use crate::McpPath;

/// Maximum response body size (1 MiB).
#[allow(dead_code)]
const MAX_RESPONSE_BYTES: usize = 1_048_576;

/// Maximum URL length.
const MAX_URL_LEN: usize = 8192;

/// Default LLM host (Ollama via QEMU user-mode networking).
const DEFAULT_LLM_HOST: &str = "10.0.2.2:11434";

/// Default LLM model (supports tool calling).
const DEFAULT_LLM_MODEL: &str = "qwen2.5:7b-instruct-q4_K_M";

/// Allowed URL schemes.
const ALLOWED_SCHEMES: &[&str] = &["http://", "https://"];

pub struct NetHandler {
    llm_host: String,
    llm_model: String,
}

impl NetHandler {
    pub fn new() -> Self {
        let llm_host = std::env::var("LLM_HOST").unwrap_or_else(|_| DEFAULT_LLM_HOST.to_string());
        let llm_model =
            std::env::var("LLM_MODEL").unwrap_or_else(|_| DEFAULT_LLM_MODEL.to_string());
        NetHandler {
            llm_host,
            llm_model,
        }
    }

    /// Validate that a URL uses an allowed scheme and is within length limits.
    fn validate_url(url: &str) -> Result<(), String> {
        if url.len() > MAX_URL_LEN {
            return Err(format!("URL too long ({} bytes, max {})", url.len(), MAX_URL_LEN));
        }
        if !ALLOWED_SCHEMES.iter().any(|s| url.starts_with(s)) {
            return Err(format!(
                "URL scheme not allowed (must start with http:// or https://): {}",
                &url[..url.len().min(60)]
            ));
        }
        Ok(())
    }

    /// Sanitize a hostname: only allow alphanumeric, dots, hyphens, colons (for port).
    fn sanitize_hostname(host: &str) -> Result<String, String> {
        if host.is_empty() || host.len() > 253 {
            return Err("invalid hostname length".to_string());
        }
        for ch in host.chars() {
            if !ch.is_ascii_alphanumeric() && ch != '.' && ch != '-' && ch != ':' {
                return Err(format!("invalid character in hostname: '{}'", ch));
            }
        }
        Ok(host.to_string())
    }

    /// Truncate a string to MAX_RESPONSE_BYTES.
    #[allow(dead_code)]
    fn truncate(s: &str) -> &str {
        if s.len() <= MAX_RESPONSE_BYTES {
            s
        } else {
            // Find a valid UTF-8 boundary
            let mut end = MAX_RESPONSE_BYTES;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            &s[..end]
        }
    }

    // -----------------------------------------------------------------------
    // Platform-specific implementations
    // -----------------------------------------------------------------------

    fn do_http_get(&self, _url: &str, _headers: &Value) -> Result<Value, String> {
        #[cfg(target_os = "redox")]
        {
            use std::process::Command;
            let output = Command::new("/usr/bin/curl")
                .args(["-s", "-D", "-", _url])
                .output()
                .map_err(|e| format!("curl exec failed: {}", e))?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stdout = Self::truncate(&stdout);

            // Parse: headers end at first blank line, then body
            let (raw_headers, body) = if let Some(idx) = stdout.find("\r\n\r\n") {
                (&stdout[..idx], &stdout[idx + 4..])
            } else if let Some(idx) = stdout.find("\n\n") {
                (&stdout[..idx], &stdout[idx + 2..])
            } else {
                ("", stdout)
            };

            // Extract status code from first line e.g. "HTTP/1.1 200 OK"
            let status_code = raw_headers
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(0);

            Ok(json!({
                "status_code": status_code,
                "body": body,
                "headers": raw_headers,
            }))
        }

        #[cfg(not(target_os = "redox"))]
        {
            let _ = _headers;
            Ok(json!({
                "status_code": 200,
                "body": "<html><body>mock</body></html>",
                "headers": "Content-Type: text/html",
            }))
        }
    }

    fn do_http_post(
        &self,
        url: &str,
        body: &str,
        content_type: &str,
        _headers: &Value,
    ) -> Result<Value, String> {
        #[cfg(target_os = "redox")]
        {
            use std::process::Command;
            let ct_header = format!("Content-Type: {}", content_type);
            let output = Command::new("/usr/bin/curl")
                .args(["-s", "-X", "POST", "-H", &ct_header, "-d", body, url])
                .output()
                .map_err(|e| format!("curl exec failed: {}", e))?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stdout = Self::truncate(&stdout);

            Ok(json!({
                "status_code": if output.status.success() { 200 } else { 500 },
                "body": stdout,
            }))
        }

        #[cfg(not(target_os = "redox"))]
        {
            let _ = (_headers, content_type);
            // Mock: if posting to an Ollama-like endpoint, return OpenAI-compatible response
            if url.contains("/v1/chat/completions") {
                // Parse the request body to get model info
                let parsed: Value = serde_json::from_str(body).unwrap_or(json!({}));
                let model = parsed
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&self.llm_model);
                Ok(json!({
                    "status_code": 200,
                    "body": serde_json::to_string(&json!({
                        "choices": [{
                            "message": {
                                "role": "assistant",
                                "content": "mock response"
                            },
                            "finish_reason": "stop"
                        }],
                        "model": model,
                    })).unwrap(),
                }))
            } else {
                Ok(json!({
                    "status_code": 200,
                    "body": "mock post response",
                }))
            }
        }
    }

    fn do_tcp_connect(&self, host: &str, port: u16, data: &str) -> Result<Value, String> {
        #[cfg(target_os = "redox")]
        {
            use std::io::{Read, Write};
            let addr = format!("tcp:{}:{}", host, port);
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&addr)
                .map_err(|e| format!("tcp connect failed ({}): {}", addr, e))?;

            file.write_all(data.as_bytes())
                .map_err(|e| format!("tcp write failed: {}", e))?;
            file.flush()
                .map_err(|e| format!("tcp flush failed: {}", e))?;

            let mut buf = vec![0u8; MAX_RESPONSE_BYTES];
            let mut total = 0;
            loop {
                match file.read(&mut buf[total..]) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        // Check for complete response
                        if buf[..total].ends_with(b"\n") || buf[..total].ends_with(b"}") {
                            break;
                        }
                        if total >= MAX_RESPONSE_BYTES {
                            break;
                        }
                    }
                    Err(e) => return Err(format!("tcp read failed: {}", e)),
                }
            }

            let response =
                String::from_utf8(buf[..total].to_vec()).map_err(|e| format!("not UTF-8: {}", e))?;
            Ok(json!({ "response": response }))
        }

        #[cfg(not(target_os = "redox"))]
        {
            let _ = (host, port, data);
            // Mock returns valid JSON-RPC for llm/ai dispatch chain tests
            Ok(json!({ "response": "mock tcp response" }))
        }
    }

    fn do_dns_resolve(&self, hostname: &str) -> Result<Value, String> {
        #[cfg(target_os = "redox")]
        {
            use std::process::Command;
            let output = Command::new("dns")
                .arg(hostname)
                .output()
                .map_err(|e| format!("dns command failed: {}", e))?;

            let ip = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if ip.is_empty() {
                return Err(format!("dns resolve failed for {}", hostname));
            }
            Ok(json!({ "ip": ip }))
        }

        #[cfg(not(target_os = "redox"))]
        {
            let _ = hostname;
            Ok(json!({ "ip": "93.184.216.34" }))
        }
    }

    fn do_ping(&self, host: &str, count: u32) -> Result<Value, String> {
        #[cfg(target_os = "redox")]
        {
            use std::process::Command;
            let count_str = count.to_string();
            let output = Command::new("ping")
                .args(["-c", &count_str, host])
                .output()
                .map_err(|e| format!("ping failed: {}", e))?;

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            Ok(json!({
                "success": output.status.success(),
                "output": Self::truncate(&stdout),
            }))
        }

        #[cfg(not(target_os = "redox"))]
        {
            let _ = (host, count);
            Ok(json!({
                "success": true,
                "output": "PING mock: 64 bytes, time=1ms",
            }))
        }
    }

    fn do_status(&self) -> Value {
        #[cfg(target_os = "redox")]
        {
            let read_file = |path: &str| -> String {
                std::fs::read_to_string(path)
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            };
            json!({
                "ip": read_file("/etc/net/ip"),
                "subnet": read_file("/etc/net/ip_subnet"),
                "gateway": read_file("/etc/net/ip_router"),
                "llm_backend": {
                    "type": "ollama",
                    "host": self.llm_host,
                    "model": self.llm_model,
                },
            })
        }

        #[cfg(not(target_os = "redox"))]
        {
            json!({
                "ip": "10.0.2.15",
                "subnet": "255.255.255.0",
                "gateway": "10.0.2.2",
                "llm_backend": {
                    "type": "ollama",
                    "host": self.llm_host,
                    "model": self.llm_model,
                },
            })
        }
    }

    fn do_firewall_check(&self, _host: &str, _port: Option<u16>) -> Value {
        // Always allowed for now — Guardian integration in task 4
        json!({
            "allowed": true,
            "rule": "default-allow (guardian integration pending)",
        })
    }

    /// The key method: route LLM requests to Ollama via OpenAI-compatible API.
    fn do_llm_request(
        &self,
        model: &str,
        messages: &Value,
        tools: &Value,
        max_tokens: Option<u64>,
    ) -> Result<Value, String> {
        let url = format!("http://{}/v1/chat/completions", self.llm_host);

        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
        });

        if !tools.is_null() && tools.is_array() && !tools.as_array().unwrap().is_empty() {
            body["tools"] = tools.clone();
        }
        if let Some(mt) = max_tokens {
            body["max_tokens"] = json!(mt);
        }

        let body_str = serde_json::to_string(&body)
            .map_err(|e| format!("failed to serialize LLM request: {}", e))?;

        let post_result =
            self.do_http_post(&url, &body_str, "application/json", &json!({}))?;

        let status = post_result
            .get("status_code")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let response_body = post_result
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if status != 200 {
            return Err(format!(
                "LLM request failed (status {}): {}",
                status,
                &response_body[..response_body.len().min(200)]
            ));
        }

        // Parse the OpenAI-compatible response
        let parsed: Value = serde_json::from_str(response_body)
            .map_err(|e| format!("failed to parse LLM response: {} (body: {})", e, &response_body[..response_body.len().min(200)]))?;

        // Extract choices[0].message
        let message = parsed
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .cloned()
            .unwrap_or(json!({"content": "", "role": "assistant"}));

        Ok(json!({
            "message": message,
            "model": parsed.get("model").cloned().unwrap_or(json!(model)),
            "finish_reason": parsed.get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("finish_reason"))
                .cloned()
                .unwrap_or(json!("stop")),
        }))
    }
}

impl ServiceHandler for NetHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "http_get" => {
                let url = match request.params.get("url").and_then(|v| v.as_str()) {
                    Some(u) => u,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required parameter 'url'",
                        )
                    }
                };
                if let Err(e) = Self::validate_url(url) {
                    return JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, e);
                }
                let headers = request.params.get("headers").cloned().unwrap_or(json!({}));
                match self.do_http_get(url, &headers) {
                    Ok(v) => JsonRpcResponse::success(request.id.clone(), v),
                    Err(e) => JsonRpcResponse::error(request.id.clone(), INTERNAL_ERROR, e),
                }
            }

            "http_post" => {
                let url = match request.params.get("url").and_then(|v| v.as_str()) {
                    Some(u) => u,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required parameter 'url'",
                        )
                    }
                };
                if let Err(e) = Self::validate_url(url) {
                    return JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, e);
                }
                let body = request
                    .params
                    .get("body")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let content_type = request
                    .params
                    .get("content_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("application/json");
                let headers = request.params.get("headers").cloned().unwrap_or(json!({}));
                match self.do_http_post(url, body, content_type, &headers) {
                    Ok(v) => JsonRpcResponse::success(request.id.clone(), v),
                    Err(e) => JsonRpcResponse::error(request.id.clone(), INTERNAL_ERROR, e),
                }
            }

            "tcp_connect" => {
                let host = match request.params.get("host").and_then(|v| v.as_str()) {
                    Some(h) => h,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required parameter 'host'",
                        )
                    }
                };
                if let Err(e) = Self::sanitize_hostname(host) {
                    return JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, e);
                }
                let port = match request.params.get("port").and_then(|v| v.as_u64()) {
                    Some(p) if p <= 65535 => p as u16,
                    Some(_) => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "port must be 0-65535",
                        )
                    }
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required parameter 'port'",
                        )
                    }
                };
                let data = request
                    .params
                    .get("data")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match self.do_tcp_connect(host, port, data) {
                    Ok(v) => JsonRpcResponse::success(request.id.clone(), v),
                    Err(e) => JsonRpcResponse::error(request.id.clone(), INTERNAL_ERROR, e),
                }
            }

            "dns_resolve" => {
                let hostname = match request.params.get("hostname").and_then(|v| v.as_str()) {
                    Some(h) => h,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required parameter 'hostname'",
                        )
                    }
                };
                if let Err(e) = Self::sanitize_hostname(hostname) {
                    return JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, e);
                }
                match self.do_dns_resolve(hostname) {
                    Ok(v) => JsonRpcResponse::success(request.id.clone(), v),
                    Err(e) => JsonRpcResponse::error(request.id.clone(), INTERNAL_ERROR, e),
                }
            }

            "ping" => {
                let host = match request.params.get("host").and_then(|v| v.as_str()) {
                    Some(h) => h,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required parameter 'host'",
                        )
                    }
                };
                if let Err(e) = Self::sanitize_hostname(host) {
                    return JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, e);
                }
                let count = request
                    .params
                    .get("count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3)
                    .min(10) as u32;
                match self.do_ping(host, count) {
                    Ok(v) => JsonRpcResponse::success(request.id.clone(), v),
                    Err(e) => JsonRpcResponse::error(request.id.clone(), INTERNAL_ERROR, e),
                }
            }

            "status" => {
                let result = self.do_status();
                JsonRpcResponse::success(request.id.clone(), result)
            }

            "firewall_check" => {
                let host = match request.params.get("host").and_then(|v| v.as_str()) {
                    Some(h) => h,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required parameter 'host'",
                        )
                    }
                };
                let port = request.params.get("port").and_then(|v| v.as_u64()).map(|p| p as u16);
                let result = self.do_firewall_check(host, port);
                JsonRpcResponse::success(request.id.clone(), result)
            }

            "llm_request" => {
                let model = request
                    .params
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&self.llm_model);
                let messages = match request.params.get("messages") {
                    Some(m) if m.is_array() => m,
                    Some(_) => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "'messages' must be an array",
                        )
                    }
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required parameter 'messages'",
                        )
                    }
                };
                let tools = request.params.get("tools").cloned().unwrap_or(json!(null));
                let max_tokens = request.params.get("max_tokens").and_then(|v| v.as_u64());
                match self.do_llm_request(model, messages, &tools, max_tokens) {
                    Ok(v) => JsonRpcResponse::success(request.id.clone(), v),
                    Err(e) => JsonRpcResponse::error(request.id.clone(), INTERNAL_ERROR, e),
                }
            }

            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in net service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec![
            "http_get",
            "http_post",
            "tcp_connect",
            "dns_resolve",
            "ping",
            "status",
            "firewall_check",
            "llm_request",
        ]
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::JsonRpcRequest;
    use serde_json::json;

    fn make_request(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(json!(1)),
        }
    }

    fn handler() -> NetHandler {
        NetHandler::new()
    }

    fn path() -> McpPath {
        McpPath {
            service: "net".to_string(),
            resource: vec![],
        }
    }

    #[test]
    fn test_http_get() {
        let h = handler();
        let req = make_request("http_get", json!({"url": "https://example.com"}));
        let resp = h.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["status_code"], 200);
        assert!(result["body"].as_str().unwrap().contains("mock"));
    }

    #[test]
    fn test_http_get_invalid_scheme() {
        let h = handler();
        let req = make_request("http_get", json!({"url": "ftp://example.com/file"}));
        let resp = h.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("scheme not allowed"));
    }

    #[test]
    fn test_http_post() {
        let h = handler();
        let req = make_request(
            "http_post",
            json!({"url": "https://example.com/api", "body": "{\"key\":\"val\"}"}),
        );
        let resp = h.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["status_code"], 200);
    }

    #[test]
    fn test_tcp_connect() {
        let h = handler();
        let req = make_request(
            "tcp_connect",
            json!({"host": "10.0.2.2", "port": 9999, "data": "hello"}),
        );
        let resp = h.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert!(result["response"].as_str().is_some());
    }

    #[test]
    fn test_tcp_connect_missing_port() {
        let h = handler();
        let req = make_request("tcp_connect", json!({"host": "10.0.2.2", "data": "hello"}));
        let resp = h.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_dns_resolve() {
        let h = handler();
        let req = make_request("dns_resolve", json!({"hostname": "example.com"}));
        let resp = h.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["ip"], "93.184.216.34");
    }

    #[test]
    fn test_ping() {
        let h = handler();
        let req = make_request("ping", json!({"host": "10.0.2.2", "count": 1}));
        let resp = h.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["success"], true);
    }

    #[test]
    fn test_status() {
        let h = handler();
        let req = make_request("status", json!({}));
        let resp = h.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["ip"], "10.0.2.15");
        assert_eq!(result["llm_backend"]["type"], "ollama");
        assert!(result["llm_backend"]["host"].as_str().is_some());
    }

    #[test]
    fn test_firewall_check() {
        let h = handler();
        let req = make_request("firewall_check", json!({"host": "example.com", "port": 443}));
        let resp = h.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result["allowed"], true);
    }

    #[test]
    fn test_llm_request() {
        let h = handler();
        let req = make_request(
            "llm_request",
            json!({
                "model": "qwen2.5:7b-instruct-q4_K_M",
                "messages": [{"role": "user", "content": "hello"}],
            }),
        );
        let resp = h.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert!(result["message"]["content"].as_str().is_some());
        assert_eq!(result["message"]["content"], "mock response");
    }

    #[test]
    fn test_llm_request_missing_messages() {
        let h = handler();
        let req = make_request("llm_request", json!({"model": "test"}));
        let resp = h.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_llm_request_with_tools() {
        let h = handler();
        let req = make_request(
            "llm_request",
            json!({
                "messages": [{"role": "user", "content": "what time is it?"}],
                "tools": [{"type": "function", "function": {"name": "get_time"}}],
            }),
        );
        let resp = h.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert!(result["message"].is_object());
    }

    #[test]
    fn test_unknown_method() {
        let h = handler();
        let req = make_request("nonexistent", json!({}));
        let resp = h.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_list_methods() {
        let h = handler();
        let methods = h.list_methods();
        assert_eq!(methods.len(), 8);
        assert!(methods.contains(&"http_get"));
        assert!(methods.contains(&"llm_request"));
    }

    #[test]
    fn test_hostname_sanitization() {
        assert!(NetHandler::sanitize_hostname("example.com").is_ok());
        assert!(NetHandler::sanitize_hostname("10.0.2.2:11434").is_ok());
        assert!(NetHandler::sanitize_hostname("evil;rm -rf /").is_err());
        assert!(NetHandler::sanitize_hostname("").is_err());
    }

    #[test]
    fn test_url_validation() {
        assert!(NetHandler::validate_url("https://example.com").is_ok());
        assert!(NetHandler::validate_url("http://10.0.2.2:11434/api").is_ok());
        assert!(NetHandler::validate_url("ftp://evil.com").is_err());
        assert!(NetHandler::validate_url("file:///etc/passwd").is_err());
    }
}
