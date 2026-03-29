//! Support service handlers: logging and configuration

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use serde_json::json;

use crate::protocol::{JsonRpcRequest, JsonRpcResponse, INVALID_PARAMS, METHOD_NOT_FOUND};
use crate::McpPath;
use crate::handler::ServiceHandler;

// ---------------------------------------------------------------------------
// LogHandler — in-memory ring buffer of log entries
// ---------------------------------------------------------------------------

struct LogEntry {
    level: String,
    message: String,
    source: String,
    timestamp_secs: u64,
}

pub struct LogHandler {
    entries: Mutex<Vec<LogEntry>>,
}

impl LogHandler {
    pub fn new() -> Self {
        LogHandler {
            entries: Mutex::new(Vec::new()),
        }
    }
}

impl ServiceHandler for LogHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "write" => {
                let level = match request.params.get("level").and_then(|v| v.as_str()) {
                    Some(l) => l.to_string(),
                    None => return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        "Missing required param: level",
                    ),
                };
                let message = match request.params.get("message").and_then(|v| v.as_str()) {
                    Some(m) => m.to_string(),
                    None => return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        "Missing required param: message",
                    ),
                };
                let source = request.params
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let timestamp_secs = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                let mut entries = match self.entries.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                entries.push(LogEntry { level, message, source, timestamp_secs });
                if entries.len() > 1000 {
                    let excess = entries.len() - 1000;
                    entries.drain(..excess);
                }
                let index = entries.len() - 1;
                JsonRpcResponse::success(request.id.clone(), json!({"ok": true, "index": index}))
            }
            "read" => {
                let count = {
                    let n = request.params
                        .get("count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    // F9: Default to 10 when count is 0 or missing
                    if n == 0 { 10 } else { n }
                };
                let entries = match self.entries.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let start = entries.len().saturating_sub(count);
                let result: Vec<serde_json::Value> = entries[start..]
                    .iter()
                    .map(|e| json!({
                        "level": e.level,
                        "message": e.message,
                        "source": e.source,
                        "timestamp_secs": e.timestamp_secs,
                    }))
                    .collect();
                JsonRpcResponse::success(request.id.clone(), json!(result))
            }
            "list" => {
                JsonRpcResponse::success(
                    request.id.clone(),
                    json!({"levels": ["debug", "info", "warn", "error"]}),
                )
            }
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in log service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["write", "read", "list"]
    }
}

// ---------------------------------------------------------------------------
// ConfigHandler — in-memory key/value configuration store
// ---------------------------------------------------------------------------

pub struct ConfigHandler {
    store: Mutex<HashMap<String, serde_json::Value>>,
}

impl ConfigHandler {
    pub fn new() -> Self {
        ConfigHandler {
            store: Mutex::new(HashMap::new()),
        }
    }
}

impl ServiceHandler for ConfigHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "get" => {
                let key = match request.params.get("key").and_then(|v| v.as_str()) {
                    Some(k) => k.to_string(),
                    None => return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        "Missing required param: key",
                    ),
                };
                let store = match self.store.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                match store.get(&key) {
                    Some(value) => JsonRpcResponse::success(
                        request.id.clone(),
                        json!({"key": key, "value": value}),
                    ),
                    None => JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        format!("Key not found: {}", key),
                    ),
                }
            }
            "set" => {
                let key = match request.params.get("key").and_then(|v| v.as_str()) {
                    Some(k) => k.to_string(),
                    None => return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        "Missing required param: key",
                    ),
                };
                let value = match request.params.get("value") {
                    Some(v) => v.clone(),
                    None => return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        "Missing required param: value",
                    ),
                };
                let mut store = match self.store.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                store.insert(key, value);
                JsonRpcResponse::success(request.id.clone(), json!({"ok": true}))
            }
            "list" => {
                let store = match self.store.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let keys: Vec<&str> = store.keys().map(|k| k.as_str()).collect();
                let count = keys.len();
                JsonRpcResponse::success(request.id.clone(), json!({"keys": keys, "count": count}))
            }
            "delete" => {
                let key = match request.params.get("key").and_then(|v| v.as_str()) {
                    Some(k) => k.to_string(),
                    None => return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        "Missing required param: key",
                    ),
                };
                let mut store = match self.store.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let existed = store.remove(&key).is_some();
                JsonRpcResponse::success(request.id.clone(), json!({"ok": true, "existed": existed}))
            }
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in config service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["get", "set", "list", "delete"]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::protocol::JsonRpcRequest;

    fn req(method: &str, params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id: Some(json!(1)),
        }
    }

    fn dummy_path() -> McpPath {
        McpPath {
            service: "log".into(),
            resource: vec![],
        }
    }

    // --- LogHandler tests ---

    #[test]
    fn log_write_and_read() {
        let h = LogHandler::new();
        let path = dummy_path();
        let resp = h.handle(&path, &req("write", json!({
            "level": "info",
            "message": "hello",
            "source": "test"
        })));
        assert!(resp.error.is_none());
        assert_eq!(resp.result.as_ref().unwrap()["ok"], json!(true));

        let resp2 = h.handle(&path, &req("read", json!({"count": 5})));
        let entries = resp2.result.unwrap();
        let arr = entries.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["message"], json!("hello"));
        assert_eq!(arr[0]["level"], json!("info"));
    }

    #[test]
    fn log_list_levels() {
        let h = LogHandler::new();
        let path = dummy_path();
        let resp = h.handle(&path, &req("list", json!({})));
        let levels = resp.result.unwrap()["levels"].clone();
        assert!(levels.as_array().unwrap().contains(&json!("info")));
        assert!(levels.as_array().unwrap().contains(&json!("error")));
    }

    #[test]
    fn log_write_missing_params_returns_error() {
        let h = LogHandler::new();
        let path = dummy_path();
        let resp = h.handle(&path, &req("write", json!({"level": "info"})));
        assert!(resp.error.is_some());
    }

    #[test]
    fn log_caps_at_1000() {
        let h = LogHandler::new();
        let path = dummy_path();
        for i in 0..1010u32 {
            h.handle(&path, &req("write", json!({
                "level": "debug",
                "message": format!("msg {}", i),
                "source": "test"
            })));
        }
        let resp = h.handle(&path, &req("read", json!({"count": 2000})));
        let arr = resp.result.unwrap().as_array().unwrap().clone();
        assert!(arr.len() <= 1000);
    }

    // --- ConfigHandler tests ---

    #[test]
    fn config_set_get_delete() {
        let h = ConfigHandler::new();
        let path = McpPath { service: "config".into(), resource: vec![] };

        let r = h.handle(&path, &req("set", json!({"key": "hostname", "value": "acos"})));
        assert_eq!(r.result.unwrap()["ok"], json!(true));

        let r = h.handle(&path, &req("get", json!({"key": "hostname"})));
        assert_eq!(r.result.unwrap()["value"], json!("acos"));

        let r = h.handle(&path, &req("delete", json!({"key": "hostname"})));
        assert_eq!(r.result.unwrap()["existed"], json!(true));

        let r = h.handle(&path, &req("delete", json!({"key": "hostname"})));
        assert_eq!(r.result.unwrap()["existed"], json!(false));
    }

    #[test]
    fn config_list() {
        let h = ConfigHandler::new();
        let path = McpPath { service: "config".into(), resource: vec![] };
        h.handle(&path, &req("set", json!({"key": "a", "value": 1})));
        h.handle(&path, &req("set", json!({"key": "b", "value": 2})));
        let r = h.handle(&path, &req("list", json!({})));
        let result = r.result.unwrap();
        assert_eq!(result["count"], json!(2));
        let keys = result["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn config_get_missing_key_returns_error() {
        let h = ConfigHandler::new();
        let path = McpPath { service: "config".into(), resource: vec![] };
        let r = h.handle(&path, &req("get", json!({"key": "nonexistent"})));
        assert!(r.error.is_some());
    }

    #[test]
    fn config_unknown_method_returns_error() {
        let h = ConfigHandler::new();
        let path = McpPath { service: "config".into(), resource: vec![] };
        let r = h.handle(&path, &req("unknown", json!({})));
        assert!(r.error.is_some());
    }
}
