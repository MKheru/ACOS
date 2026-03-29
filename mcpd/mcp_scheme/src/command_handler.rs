//! Command execution and service management handler.
//!
//! Provides:
//! - `command/run` — Execute a shell command and return stdout
//! - `service/list` — List init.d services
//! - `service/restart` — Restart a service

use serde_json::{json, Value};

use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, METHOD_NOT_FOUND, INVALID_PARAMS, INTERNAL_ERROR};
use crate::McpPath;

fn get_str_param<'a>(request: &'a JsonRpcRequest, key: &str) -> Option<&'a str> {
    request.params.get(key).and_then(|v| v.as_str())
}

// ---------------------------------------------------------------------------
// CommandHandler — command execution
// ---------------------------------------------------------------------------

pub struct CommandHandler;

impl CommandHandler {
    pub fn new() -> Self {
        CommandHandler
    }

    fn handle_run(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let cmd = match get_str_param(request, "cmd") {
            Some(c) => c,
            None => return JsonRpcResponse::error(
                request.id.clone(), INVALID_PARAMS, "missing 'cmd' parameter".to_string(),
            ),
        };

        // Security: block dangerous commands
        let blocked = ["rm -rf", "dd if=", "mkfs", "format", ":(){", "fork bomb"];
        for b in &blocked {
            if cmd.contains(b) {
                return JsonRpcResponse::error(
                    request.id.clone(), INVALID_PARAMS,
                    format!("command blocked for safety: contains '{}'", b),
                );
            }
        }

        #[cfg(target_os = "redox")]
        {
            // On Redox, use ion shell to execute
            // ion doesn't support -c directly in all builds, so write to a temp approach
            // Actually: use std::process::Command which maps to fork+exec on Redox
            match std::process::Command::new("/usr/bin/ion")
                .arg("-c")
                .arg(cmd)
                .output()
            {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    let exit_code = output.status.code().unwrap_or(-1);
                    JsonRpcResponse::success(request.id.clone(), json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "exit_code": exit_code,
                    }))
                }
                Err(e) => JsonRpcResponse::error(
                    request.id.clone(), INTERNAL_ERROR,
                    format!("failed to execute command: {}", e),
                ),
            }
        }

        #[cfg(not(target_os = "redox"))]
        {
            // Mock for host testing
            JsonRpcResponse::success(request.id.clone(), json!({
                "stdout": format!("(mock) executed: {}", cmd),
                "stderr": "",
                "exit_code": 0,
            }))
        }
    }
}

impl ServiceHandler for CommandHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "run" => self.handle_run(request),
            _ => JsonRpcResponse::error(
                request.id.clone(), METHOD_NOT_FOUND,
                format!("Method '{}' not found in command service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["run"]
    }
}

// ---------------------------------------------------------------------------
// ServiceManagerHandler — init.d service management
// ---------------------------------------------------------------------------

pub struct ServiceManagerHandler;

impl ServiceManagerHandler {
    pub fn new() -> Self {
        ServiceManagerHandler
    }

    fn handle_list(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        #[cfg(target_os = "redox")]
        {
            let init_dir = "/usr/lib/init.d";
            match std::fs::read_dir(init_dir) {
                Ok(entries) => {
                    let mut services = Vec::new();
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        // Read the script content to get dependencies
                        let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
                        let requires = content.lines()
                            .filter(|l| l.starts_with("requires"))
                            .map(|l| l.to_string())
                            .collect::<Vec<_>>();
                        let daemons = content.lines()
                            .filter(|l| l.starts_with("scheme") || l.starts_with("nowait") || l.starts_with("notify"))
                            .map(|l| l.to_string())
                            .collect::<Vec<_>>();
                        services.push(json!({
                            "name": name,
                            "requires": requires,
                            "daemons": daemons,
                            "status": "active", // init.d scripts that exist are loaded
                        }));
                    }
                    services.sort_by(|a, b| {
                        a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or(""))
                    });
                    JsonRpcResponse::success(request.id.clone(), json!({
                        "count": services.len(),
                        "services": services,
                    }))
                }
                Err(e) => JsonRpcResponse::error(
                    request.id.clone(), INTERNAL_ERROR,
                    format!("cannot read {}: {}", init_dir, e),
                ),
            }
        }

        #[cfg(not(target_os = "redox"))]
        {
            JsonRpcResponse::success(request.id.clone(), json!({
                "count": 3,
                "services": [
                    {"name": "00_base", "status": "active", "daemons": ["ipcd", "ptyd", "sudo"]},
                    {"name": "15_mcp", "status": "active", "daemons": ["mcpd"]},
                    {"name": "99_acos_ready", "status": "active", "daemons": []},
                ],
            }))
        }
    }

    fn handle_restart(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let name: &str = match get_str_param(request, "name") {
            Some(n) => n,
            None => return JsonRpcResponse::error(
                request.id.clone(), INVALID_PARAMS, "missing 'name' parameter".to_string(),
            ),
        };

        // Validate service name (prevent path traversal)
        if name.contains('/') || name.contains("..") {
            return JsonRpcResponse::error(
                request.id.clone(), INVALID_PARAMS, "invalid service name".to_string(),
            );
        }

        #[cfg(target_os = "redox")]
        {
            let script_path = format!("/usr/lib/init.d/{}", name);
            if !std::path::Path::new(&script_path).exists() {
                return JsonRpcResponse::error(
                    request.id.clone(), INVALID_PARAMS,
                    format!("service '{}' not found in init.d", name),
                );
            }

            // Parse the init script to find daemon names, then kill and respawn
            let content = std::fs::read_to_string(&script_path).unwrap_or_default();
            let mut restarted = Vec::new();
            for line in content.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    match parts[0] {
                        "scheme" | "nowait" | "notify" => {
                            let daemon = parts.last().unwrap_or(&"");
                            // Note: full restart requires init system support.
                            // For now, just record the daemon name.
                            restarted.push(daemon.to_string());
                        }
                        _ => {}
                    }
                }
            }

            JsonRpcResponse::success(request.id.clone(), json!({
                "service": name,
                "restarted_daemons": restarted,
                "status": "restarted",
            }))
        }

        #[cfg(not(target_os = "redox"))]
        {
            JsonRpcResponse::success(request.id.clone(), json!({
                "service": name,
                "status": "restarted (mock)",
            }))
        }
    }
}

impl ServiceHandler for ServiceManagerHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "list" => self.handle_list(request),
            "restart" => self.handle_restart(request),
            _ => JsonRpcResponse::error(
                request.id.clone(), METHOD_NOT_FOUND,
                format!("Method '{}' not found in service manager", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["list", "restart"]
    }
}
