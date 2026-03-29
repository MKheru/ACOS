//! System service handlers: system/info, system/processes, system/memory
//!
//! On Redox, system info is read from /scheme/sys/ at handler construction
//! time (before mcpd enters null namespace via setrens). Runtime calls serve
//! cached data. Process list is re-read on each call since /scheme/sys/context
//! may not be available after setrens — returns cached snapshot if unavailable.

use std::sync::Mutex;
use serde_json::{json, Value};

use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, METHOD_NOT_FOUND};
use crate::McpPath;

// ---------------------------------------------------------------------------
// SystemInfoHandler — system/info service
// ---------------------------------------------------------------------------

pub struct SystemInfoHandler {
    cached_info: Value,
}

impl SystemInfoHandler {
    pub fn new() -> Self {
        #[cfg(not(target_os = "redox"))]
        let cached_info = json!({
            "hostname": "acos",
            "kernel": "acos-kernel-0.1",
            "uptime": 42,
            "memory_total": 1073741824u64,
            "memory_free": 536870912u64,
        });

        #[cfg(target_os = "redox")]
        let cached_info = {
            // /scheme/sys/uname format: "OS\nversion\narch\nbuild_hash\n"
            let uname_raw = std::fs::read_to_string("/scheme/sys/uname")
                .unwrap_or_else(|_| "acos\n0.0.0\nx86_64\nunknown\n".into());
            let mut uname_lines = uname_raw.lines();
            let os_name = uname_lines.next().unwrap_or("acos").trim().to_string();
            let os_version = uname_lines.next().unwrap_or("0.0.0").trim().to_string();
            let arch = uname_lines.next().unwrap_or("x86_64").trim().to_string();
            let kernel = format!("{}-{}-{}", os_name, os_version, arch);

            // uptime: /scheme/sys/uptime does not exist on this kernel build
            // use 0 as placeholder
            let uptime: u64 = 0;

            let hostname = std::fs::read_to_string("/etc/hostname")
                .unwrap_or_else(|_| "acos".into())
                .trim()
                .to_string();

            // Get memory from read_memory_stats
            let mem = read_memory_stats();
            let memory_total = mem.get("total_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
            let memory_free = mem.get("free_bytes").and_then(|v| v.as_u64()).unwrap_or(0);

            json!({
                "hostname": hostname,
                "kernel": kernel,
                "uptime": uptime,
                "memory_total": memory_total,
                "memory_free": memory_free,
            })
        };

        SystemInfoHandler { cached_info }
    }
}

impl ServiceHandler for SystemInfoHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "info" => JsonRpcResponse::success(request.id.clone(), self.cached_info.clone()),
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in system/info service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["info"]
    }
}

// ---------------------------------------------------------------------------
// ProcessHandler — system/processes service
// ---------------------------------------------------------------------------

pub struct ProcessHandler {
    /// Snapshot taken at construction (before setrens)
    cached_procs: Mutex<Value>,
}

impl ProcessHandler {
    pub fn new() -> Self {
        #[cfg(not(target_os = "redox"))]
        let procs = json!([
            {"pid": 1, "name": "init"},
            {"pid": 5, "name": "mcpd"},
            {"pid": 10, "name": "ion"},
        ]);

        #[cfg(target_os = "redox")]
        let procs = read_process_list();

        ProcessHandler {
            cached_procs: Mutex::new(procs),
        }
    }
}

impl ServiceHandler for ProcessHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "list" => {
                // Try to read fresh data; fall back to cache if unavailable
                #[cfg(target_os = "redox")]
                {
                    let fresh = read_process_list();
                    if let Ok(mut cache) = self.cached_procs.lock() {
                        if fresh.as_array().map_or(false, |a| !a.is_empty()) {
                            *cache = fresh;
                        }
                    }
                }

                let procs = self.cached_procs.lock()
                    .map(|g| g.clone())
                    .unwrap_or_else(|e| e.into_inner().clone());
                JsonRpcResponse::success(request.id.clone(), procs)
            }
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in system/processes service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["list"]
    }
}

// ---------------------------------------------------------------------------
// MemoryHandler — system/memory service
// ---------------------------------------------------------------------------

pub struct MemoryHandler {
    cached_stats: Value,
}

impl MemoryHandler {
    pub fn new() -> Self {
        #[cfg(not(target_os = "redox"))]
        let cached_stats = json!({
            "total": 1073741824u64,
            "used": 536870912u64,
            "free": 536870912u64,
        });

        #[cfg(target_os = "redox")]
        let cached_stats = read_memory_stats();

        MemoryHandler { cached_stats }
    }
}

impl ServiceHandler for MemoryHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "stats" => {
                // Try fresh data on each call
                #[cfg(target_os = "redox")]
                {
                    let fresh = read_memory_stats();
                    return JsonRpcResponse::success(request.id.clone(), fresh);
                }
                #[cfg(not(target_os = "redox"))]
                JsonRpcResponse::success(request.id.clone(), self.cached_stats.clone())
            }
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in system/memory service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["stats"]
    }
}

// ---------------------------------------------------------------------------
// Helpers (ACOS kernel)
// ---------------------------------------------------------------------------

#[cfg(target_os = "redox")]
fn read_process_list() -> Value {
    let contents = std::fs::read_to_string("/scheme/sys/context")
        .unwrap_or_default();
    // /scheme/sys/context format (space-separated, variable columns):
    //   PID  PPID  EUID  EGID  STAT  CPU  AFFINITY  TIME         MEM   NAME
    //   30   1     0     0     UR+   #1             00:00:00.00  1 MB  /usr/bin/mcpd
    let list: Vec<Value> = contents
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 {
                return None;
            }
            let pid: u64 = match parts[0].parse() {
                Ok(p) => p,
                Err(_) => return None, // skip header
            };
            let name = parts.last().unwrap_or(&"?").to_string();

            // PPID — second column (may not always be present)
            let ppid: u64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);

            // STAT field — look for known status patterns
            let mut state = "unknown";
            for p in &parts[2..] {
                let first = p.chars().next().unwrap_or('?');
                if matches!(first, 'U' | 'R' | 'S' | 'B' | 'W' | 'Z' | 'T')
                    && p.len() <= 5
                    && !p.contains('/')
                {
                    state = match first {
                        'R' => "running",
                        'U' => "running",
                        'S' => "sleeping",
                        'B' => "blocked",
                        'W' => "waiting",
                        'Z' => "zombie",
                        'T' => "stopped",
                        _ => "unknown",
                    };
                    break;
                }
            }

            // MEM — find "N MB" or "N KB" pattern
            let mut mem = String::new();
            for (i, p) in parts.iter().enumerate() {
                if i > 0 && (*p == "KB" || *p == "MB" || *p == "GB") {
                    if let Some(val) = parts.get(i - 1) {
                        mem = format!("{} {}", val, p);
                    }
                    break;
                }
            }

            Some(json!({
                "pid": pid,
                "ppid": ppid,
                "name": name,
                "state": state,
                "memory": mem,
            }))
        })
        .collect();
    json!(list)
}

#[cfg(target_os = "redox")]
fn read_memory_stats() -> Value {
    // Source 1: /scheme/sys/meminfo (if kernel exposes it)
    if let Ok(contents) = std::fs::read_to_string("/scheme/sys/meminfo") {
        let mut total: u64 = 0;
        let mut free: u64 = 0;
        for line in contents.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                match parts[0].trim_end_matches(':') {
                    "MemTotal" => total = parts[1].parse().unwrap_or(0) * 1024,
                    "MemFree" | "MemAvailable" => {
                        let v = parts[1].parse().unwrap_or(0) * 1024;
                        if v > free { free = v; }
                    }
                    _ => {}
                }
            }
        }
        if total > 0 {
            return json!({
                "total_bytes": total,
                "used_bytes": total.saturating_sub(free),
                "free_bytes": free,
                "total_mb": total / (1024 * 1024),
                "used_mb": total.saturating_sub(free) / (1024 * 1024),
                "free_mb": free / (1024 * 1024),
            });
        }
    }

    // Source 2: estimate from process memory in /scheme/sys/context
    if let Ok(contents) = std::fs::read_to_string("/scheme/sys/context") {
        let mut total_proc_mem: u64 = 0;
        for line in contents.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            for (i, p) in parts.iter().enumerate() {
                if i > 0 {
                    match *p {
                        "MB" => {
                            if let Some(val) = parts.get(i - 1).and_then(|s| s.parse::<u64>().ok()) {
                                total_proc_mem += val * 1024 * 1024;
                            }
                        }
                        "KB" => {
                            if let Some(val) = parts.get(i - 1).and_then(|s| s.parse::<u64>().ok()) {
                                total_proc_mem += val * 1024;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // ACOS runs in QEMU with 2GB default
        let total: u64 = 2 * 1024 * 1024 * 1024;
        return json!({
            "total_bytes": total,
            "used_bytes": total_proc_mem,
            "free_bytes": total.saturating_sub(total_proc_mem),
            "total_mb": total / (1024 * 1024),
            "used_mb": total_proc_mem / (1024 * 1024),
            "free_mb": total.saturating_sub(total_proc_mem) / (1024 * 1024),
            "source": "estimated from process memory",
        });
    }

    // Fallback
    let total: u64 = 2 * 1024 * 1024 * 1024;
    json!({
        "total_bytes": total,
        "used_bytes": 0u64,
        "free_bytes": total,
        "total_mb": 2048u64,
        "used_mb": 0u64,
        "free_mb": 2048u64,
        "source": "default (2GB QEMU allocation)",
    })
}

