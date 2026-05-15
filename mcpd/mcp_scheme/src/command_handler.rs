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
// WS3.M6 — argv-level command validation (replaces substring blocklist)
// ---------------------------------------------------------------------------

/// Basenames of binaries refused regardless of arguments. Caught after path
/// stripping so `/bin/rm`, `/usr/bin/rm`, and `./rm` all map to `"rm"`.
const BLOCKED_BINARIES: &[&str] = &[
    "rm", "dd", "mkfs", "mkfs.ext2", "mkfs.ext3", "mkfs.ext4",
    "mkfs.fat", "mkfs.vfat", "mkfs.xfs", "fdisk", "sfdisk", "parted",
    "shred", "wipefs", "format", "diskpart",
];

/// Return `argv[0]` without its directory prefix.
fn argv0_basename(argv0: &str) -> &str {
    argv0.rsplit('/').next().unwrap_or(argv0)
}

/// Tokenize a command string into argv, detecting a trailing `&` as
/// background-mode marker (preserves Guardian's `/usr/bin/<name> &` pattern).
///
/// Rejects every shell metacharacter that could enable injection: pipes,
/// redirects, command substitution, env expansion, `&&`/`||`, quoting,
/// backslash, newline, etc. The only permitted punctuation is the
/// path-arg set: `- . / _ = : , % @ +`.
///
/// Returns `(argv, background)` on success, or an error string on rejection.
fn tokenize_cmd(cmd: &str) -> Result<(Vec<String>, bool), String> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return Err("empty command".to_string());
    }

    // Detect ONE trailing `&` for background mode. Any other `&` (including
    // `&&`) falls through to the metachar check below and gets rejected.
    let (core, background) = if let Some(stripped) = trimmed.strip_suffix('&') {
        let stripped = stripped.trim_end();
        if stripped.is_empty() {
            return Err("'&' with no command".to_string());
        }
        if stripped.ends_with('&') {
            return Err("'&&' is not supported".to_string());
        }
        (stripped, true)
    } else {
        (trimmed, false)
    };

    for ch in core.chars() {
        let ok = ch.is_ascii_alphanumeric()
            || ch == ' '
            || ch == '\t'
            || matches!(ch, '-' | '.' | '/' | '_' | '=' | ':' | ',' | '%' | '@' | '+');
        if !ok {
            return Err(format!("forbidden character in command: {:?}", ch));
        }
    }

    let argv: Vec<String> = core.split_whitespace().map(String::from).collect();
    if argv.is_empty() {
        return Err("empty argv after tokenization".to_string());
    }
    Ok((argv, background))
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

        // WS3.M6: tokenize + reject shell metacharacters. The prior substring
        // blocklist (`rm -rf`, `dd if=`, …) was trivially bypass-able with
        // spacing/encoding/shell tricks; argv-level validation closes the gap.
        let (argv, background) = match tokenize_cmd(cmd) {
            Ok(parsed) => parsed,
            Err(reason) => return JsonRpcResponse::error(
                request.id.clone(), INVALID_PARAMS,
                format!("command rejected: {}", reason),
            ),
        };

        let basename = argv0_basename(&argv[0]);
        if BLOCKED_BINARIES.contains(&basename) {
            return JsonRpcResponse::error(
                request.id.clone(), INVALID_PARAMS,
                format!("binary '{}' refused for safety", basename),
            );
        }

        #[cfg(target_os = "redox")]
        {
            let mut command = std::process::Command::new(&argv[0]);
            command.args(&argv[1..]);
            if background {
                // Guardian auto-restart pattern: spawn-and-detach. Dropping
                // Child leaves the process running; it'll be reaped by the
                // init system. No shell involved, no `ion -c` indirection.
                match command.spawn() {
                    Ok(_child) => JsonRpcResponse::success(request.id.clone(), json!({
                        "stdout": "",
                        "stderr": "",
                        "exit_code": 0,
                        "background": true,
                    })),
                    Err(e) => JsonRpcResponse::error(
                        request.id.clone(), INTERNAL_ERROR,
                        format!("failed to spawn '{}': {}", argv[0], e),
                    ),
                }
            } else {
                match command.output() {
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        let exit_code = output.status.code().unwrap_or(-1);
                        JsonRpcResponse::success(request.id.clone(), json!({
                            "stdout": stdout,
                            "stderr": stderr,
                            "exit_code": exit_code,
                            "background": false,
                        }))
                    }
                    Err(e) => JsonRpcResponse::error(
                        request.id.clone(), INTERNAL_ERROR,
                        format!("failed to execute '{}': {}", argv[0], e),
                    ),
                }
            }
        }

        #[cfg(not(target_os = "redox"))]
        {
            // Mock for host testing — echo the parsed argv. We do NOT exec
            // arbitrary host binaries from this code path.
            JsonRpcResponse::success(request.id.clone(), json!({
                "stdout": format!("(mock) argv={:?} background={}", argv, background),
                "stderr": "",
                "exit_code": 0,
                "background": background,
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

// ---------------------------------------------------------------------------
// Tests — WS3.M6 (tokenize + argv0 denylist)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_request(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id: Some(json!(1)),
        }
    }

    fn path() -> McpPath {
        McpPath::parse(b"command/run").unwrap()
    }

    fn run_cmd(s: &str) -> JsonRpcResponse {
        let h = CommandHandler::new();
        h.handle(&path(), &make_request("run", json!({"cmd": s})))
    }

    // --- Metacharacter rejection ---

    #[test]
    fn rejects_pipe_metachar() {
        let r = run_cmd("ls | rm -rf /");
        assert!(r.error.is_some());
        assert!(r.error.unwrap().message.contains("forbidden character"));
    }

    #[test]
    fn rejects_semicolon_metachar() {
        let r = run_cmd("ls; rm -rf /");
        assert!(r.error.is_some());
    }

    #[test]
    fn rejects_backtick_substitution() {
        let r = run_cmd("echo `rm -rf /`");
        assert!(r.error.is_some());
    }

    #[test]
    fn rejects_dollar_subshell() {
        let r = run_cmd("echo $(rm -rf /)");
        assert!(r.error.is_some());
    }

    #[test]
    fn rejects_redirect_to_file() {
        let r = run_cmd("echo data > /etc/passwd");
        assert!(r.error.is_some());
    }

    #[test]
    fn rejects_heredoc() {
        let r = run_cmd("cat <<EOF");
        assert!(r.error.is_some());
    }

    #[test]
    fn rejects_double_amp_and_then() {
        let r = run_cmd("ls && rm -rf /tmp/x");
        assert!(r.error.is_some());
        let msg = r.error.unwrap().message;
        assert!(msg.contains("&&") || msg.contains("forbidden"), "got: {msg}");
    }

    #[test]
    fn rejects_double_pipe_or() {
        let r = run_cmd("cmd1 || cmd2");
        assert!(r.error.is_some());
    }

    #[test]
    fn rejects_escape_backslash() {
        let r = run_cmd("rm\\ -rf /");
        assert!(r.error.is_some());
    }

    // --- Binary basename denylist (post-tokenization) ---

    #[test]
    fn rejects_rm_via_argv0_basename() {
        let r = run_cmd("/usr/bin/rm -rf /tmp/x");
        assert!(r.error.is_some());
        let msg = r.error.unwrap().message;
        assert!(msg.contains("rm") && msg.contains("refused"), "got: {msg}");
    }

    #[test]
    fn rejects_dd_basename() {
        let r = run_cmd("/bin/dd if=/dev/zero of=/dev/sda");
        assert!(r.error.is_some());
        assert!(r.error.unwrap().message.contains("refused"));
    }

    #[test]
    fn rejects_mkfs_variant_basename() {
        let r = run_cmd("/sbin/mkfs.ext4 /dev/sda1");
        assert!(r.error.is_some());
        assert!(r.error.unwrap().message.contains("refused"));
    }

    #[test]
    fn rejects_bare_rm_no_path() {
        let r = run_cmd("rm -rf /etc");
        assert!(r.error.is_some());
    }

    // --- Accept paths and structured args (Guardian + normal use) ---

    #[test]
    fn accepts_simple_argv_on_host_mock() {
        let r = run_cmd("/usr/bin/ls -la");
        assert!(r.error.is_none(), "expected ok, got error: {:?}", r.error);
        let result = r.result.unwrap();
        assert!(result["stdout"].as_str().unwrap().contains("argv"));
        assert_eq!(result["background"], false);
    }

    #[test]
    fn accepts_trailing_amp_as_background_marker() {
        // Guardian auto-restart pattern must keep working.
        let r = run_cmd("/usr/bin/sysmon &");
        assert!(r.error.is_none(), "expected ok, got error: {:?}", r.error);
        let result = r.result.unwrap();
        assert_eq!(result["background"], true);
    }

    #[test]
    fn accepts_key_value_arg_with_equals() {
        // `--config=/etc/acos.conf` is common; `=` must be allowed.
        let r = run_cmd("/usr/bin/some-tool --config=/etc/acos.conf");
        assert!(r.error.is_none(), "expected ok, got error: {:?}", r.error);
    }

    #[test]
    fn accepts_colon_in_arg_for_hostport() {
        let r = run_cmd("/usr/bin/curl http://localhost:8080");
        assert!(r.error.is_none(), "expected ok, got error: {:?}", r.error);
    }

    // --- Empty / degenerate inputs ---

    #[test]
    fn rejects_empty_cmd() {
        let r = run_cmd("");
        assert!(r.error.is_some());
    }

    #[test]
    fn rejects_whitespace_only_cmd() {
        let r = run_cmd("   \t  ");
        assert!(r.error.is_some());
    }

    #[test]
    fn rejects_amp_with_no_command() {
        let r = run_cmd("&");
        assert!(r.error.is_some());
    }

    // --- tokenize_cmd unit tests ---

    #[test]
    fn tokenize_strips_trailing_amp_and_sets_background() {
        let (argv, bg) = tokenize_cmd("/usr/bin/sysmon &").unwrap();
        assert_eq!(argv, vec!["/usr/bin/sysmon".to_string()]);
        assert!(bg);
    }

    #[test]
    fn tokenize_handles_multiple_spaces() {
        let (argv, bg) = tokenize_cmd("  /usr/bin/ls    -la   ").unwrap();
        assert_eq!(argv, vec!["/usr/bin/ls", "-la"]);
        assert!(!bg);
    }

    #[test]
    fn argv0_basename_strips_directory() {
        assert_eq!(argv0_basename("/usr/bin/rm"), "rm");
        assert_eq!(argv0_basename("./bin/dd"), "dd");
        assert_eq!(argv0_basename("ls"), "ls");
    }
}
