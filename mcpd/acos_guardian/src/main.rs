//! acos-guardian — Simple system monitor for ACOS
//! Prints monitoring data to stdout every 30 seconds.

#[cfg(target_os = "redox")]
use std::io::Read;
use std::io::Write;
use std::thread;
use std::time::Duration;

// ANSI escape codes
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

const POLL_INTERVAL: u64 = 30;
const MEMORY_WARN_PERCENT: f64 = 80.0;
const MEMORY_CRIT_PERCENT: f64 = 95.0;
const LOG_ERROR_THRESHOLD: usize = 5;

// Known MCP services to health-check: (service, method) pairs
// Each service is checked using its primary method, not "state" (which most don't implement)
const KNOWN_SERVICES: &[(&str, &str)] = &[
    ("system", "info"),
    ("process", "list"),
    ("log", "read"),
    ("config", "list"),
    ("file", "read"),
];

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    fn label(&self) -> &str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Critical => "CRITICAL",
        }
    }

    fn color(&self) -> &str {
        match self {
            Severity::Info => CYAN,
            Severity::Warning => YELLOW,
            Severity::Critical => RED,
        }
    }
}

#[derive(Clone, Debug)]
struct ProcessInfo {
    pid: u64,
    name: String,
}

#[derive(Clone, Debug)]
struct SystemSnapshot {
    processes: Vec<ProcessInfo>,
    memory_used_mb: u64,
    memory_total_mb: u64,
    memory_percent: f64,
    log_error_count: usize,
    uptime_secs: u64,
    service_count: usize,
    services_up: Vec<String>,
}

#[derive(Clone, Debug)]
struct Anomaly {
    detector: String,
    severity: Severity,
    description: String,
    #[allow(dead_code)]
    suggestion: String,
}

// ---------------------------------------------------------------------------
// MCP communication (dual-mode)
// ---------------------------------------------------------------------------

#[cfg(target_os = "redox")]
fn mcp_call(
    service: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    use std::fs::OpenOptions;

    let path = format!("mcp:{}", service);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("Failed to open {}: {}", path, e))?;

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });

    file.write_all(request.to_string().as_bytes())
        .map_err(|e| format!("Write failed: {}", e))?;

    let mut buf = vec![0u8; 65536];
    let n = file.read(&mut buf).map_err(|e| format!("Read failed: {}", e))?;
    let response: serde_json::Value =
        serde_json::from_slice(&buf[..n]).map_err(|e| format!("Parse failed: {}", e))?;

    if let Some(result) = response.get("result") {
        // mcpd wraps results in {"text": "<json>"}
        if let Some(text) = result.get("text").and_then(|t| t.as_str()) {
            // Try to parse the text as JSON
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) {
                return Ok(parsed);
            }
            return Ok(serde_json::json!(text));
        }
        Ok(result.clone())
    } else if let Some(error) = response.get("error") {
        Err(format!("MCP error: {}", error))
    } else {
        Err("Unknown response".to_string())
    }
}

#[cfg(not(target_os = "redox"))]
fn mcp_call(
    service: &str,
    method: &str,
    _params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    // Mock responses for host testing
    match (service, method) {
        ("process", "list") => Ok(serde_json::json!([
            {"pid": 1, "ppid": 0, "name": "kernel", "state": "running", "memory": "4MB"},
            {"pid": 2, "ppid": 1, "name": "initfs", "state": "running", "memory": "1MB"},
            {"pid": 3, "ppid": 1, "name": "ramfs", "state": "running", "memory": "2MB"},
            {"pid": 4, "ppid": 1, "name": "acpid", "state": "running", "memory": "1MB"},
            {"pid": 5, "ppid": 1, "name": "pcid", "state": "running", "memory": "3MB"},
            {"pid": 6, "ppid": 1, "name": "nvmed", "state": "running", "memory": "2MB"},
            {"pid": 7, "ppid": 1, "name": "smolnetd", "state": "running", "memory": "5MB"},
            {"pid": 8, "ppid": 1, "name": "mcpd", "state": "running", "memory": "8MB"},
            {"pid": 9, "ppid": 8, "name": "konsole", "state": "running", "memory": "12MB"},
            {"pid": 10, "ppid": 1, "name": "vesad", "state": "running", "memory": "6MB"},
            {"pid": 11, "ppid": 1, "name": "logd", "state": "running", "memory": "2MB"},
            {"pid": 12, "ppid": 1, "name": "acos-guardian", "state": "running", "memory": "3MB"},
        ])),
        ("memory", "stats") => Ok(serde_json::json!({
            "total_bytes": 2147483648_u64,
            "used_bytes": 163577856_u64,
            "free_bytes": 1983905792_u64,
            "total_mb": 2048,
            "used_mb": 156,
            "free_mb": 1892
        })),
        ("log", "read") => Ok(serde_json::json!([
            {"level": "info", "message": "System boot complete", "source": "kernel", "timestamp_secs": 100},
            {"level": "info", "message": "MCP bus initialized", "source": "mcpd", "timestamp_secs": 102},
            {"level": "warn", "message": "Slow disk response", "source": "nvmed", "timestamp_secs": 200},
        ])),
        ("system", "info") => Ok(serde_json::json!({
            "uptime_secs": 263,
            "hostname": "acos-dev",
            "version": "0.1.0"
        })),
        ("guardian", "state") => Ok(serde_json::json!({"status": "nominal"})),
        ("guardian", "anomalies") => Ok(serde_json::json!([])),
        ("guardian", "config") => Ok(serde_json::json!({"ok": true})),
        ("guardian", "respond") => Ok(serde_json::json!({"ok": true})),
        ("konsole", "write") => Ok(serde_json::json!({"ok": true})),
        _ => Ok(serde_json::json!({"text": "mock response"})),
    }
}

// ---------------------------------------------------------------------------
// Metric polling
// ---------------------------------------------------------------------------

fn poll_metrics() -> SystemSnapshot {
    // Process list
    let processes = match mcp_call("process", "list", serde_json::json!({})) {
        Ok(val) => {
            if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|p| {
                        Some(ProcessInfo {
                            pid: p.get("pid")?.as_u64()?,
                            name: sanitize_for_terminal(p.get("name")?.as_str()?),
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            }
        }
        Err(_) => Vec::new(),
    };

    // Memory stats
    let (used_mb, total_mb, percent) =
        match mcp_call("memory", "stats", serde_json::json!({})) {
            Ok(val) => {
                let used = val.get("used_mb").and_then(|v| v.as_u64()).unwrap_or(0);
                let total = val.get("total_mb").and_then(|v| v.as_u64()).unwrap_or(1);
                let pct = if total > 0 {
                    (used as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                (used, total, pct)
            }
            Err(_) => (0, 0, 0.0),
        };

    // Log errors
    let log_error_count =
        match mcp_call("log", "read", serde_json::json!({"count": 50})) {
            Ok(val) => {
                if let Some(arr) = val.as_array() {
                    arr.iter()
                        .filter(|e| {
                            e.get("level")
                                .and_then(|l| l.as_str())
                                .map(|l| l == "error")
                                .unwrap_or(false)
                        })
                        .count()
                } else {
                    0
                }
            }
            Err(_) => 0,
        };

    // Uptime
    let uptime_secs =
        match mcp_call("system", "info", serde_json::json!({})) {
            Ok(val) => val.get("uptime_secs").and_then(|v| v.as_u64()).unwrap_or(0),
            Err(_) => 0,
        };

    // Service count (how many respond to their primary method)
    let services_up: Vec<String> = KNOWN_SERVICES
        .iter()
        .filter(|(svc, method)| {
            let params = if *method == "read" {
                serde_json::json!({"path": "/etc/hostname"})
            } else if *method == "read" && *svc == "log" {
                serde_json::json!({"count": 1})
            } else {
                serde_json::json!({})
            };
            mcp_call(svc, method, params).is_ok()
        })
        .map(|(svc, _)| svc.to_string())
        .collect();
    let service_count = services_up.len();

    SystemSnapshot {
        processes,
        memory_used_mb: used_mb,
        memory_total_mb: total_mb,
        memory_percent: percent,
        log_error_count,
        uptime_secs,
        service_count,
        services_up,
    }
}

// ---------------------------------------------------------------------------
// Anomaly detectors
// ---------------------------------------------------------------------------

fn detect_anomalies(
    current: &SystemSnapshot,
    previous: &Option<SystemSnapshot>,
) -> Vec<Anomaly> {
    let mut anomalies = Vec::new();

    // 1. ProcessCrash — PIDs present before but missing now
    if let Some(ref prev) = previous {
        let prev_pids: std::collections::HashSet<u64> =
            prev.processes.iter().map(|p| p.pid).collect();
        let curr_pids: std::collections::HashSet<u64> =
            current.processes.iter().map(|p| p.pid).collect();

        for missing_pid in prev_pids.difference(&curr_pids) {
            if let Some(info) = prev.processes.iter().find(|p| p.pid == *missing_pid) {
                anomalies.push(Anomaly {
                    detector: "ProcessCrash".to_string(),
                    severity: Severity::Critical,
                    description: format!(
                        "Process \"{}\" (PID {}) has disappeared",
                        info.name, info.pid
                    ),
                    suggestion: format!("Restart {}", info.name),
                });
            }
        }
    }

    // 2. MemoryThreshold — warn at 80%, critical at 95%
    if current.memory_percent > MEMORY_CRIT_PERCENT {
        anomalies.push(Anomaly {
            detector: "MemoryThreshold".to_string(),
            severity: Severity::Critical,
            description: format!(
                "Memory usage critical: {:.1}% ({}/{} MB)",
                current.memory_percent, current.memory_used_mb, current.memory_total_mb
            ),
            suggestion: "Kill non-essential processes to free memory".to_string(),
        });
    } else if current.memory_percent > MEMORY_WARN_PERCENT {
        anomalies.push(Anomaly {
            detector: "MemoryThreshold".to_string(),
            severity: Severity::Warning,
            description: format!(
                "Memory usage high: {:.1}% ({}/{} MB)",
                current.memory_percent, current.memory_used_mb, current.memory_total_mb
            ),
            suggestion: "Monitor memory usage closely".to_string(),
        });
    }

    // 3. LogError — too many error-level log entries
    if current.log_error_count > LOG_ERROR_THRESHOLD {
        anomalies.push(Anomaly {
            detector: "LogError".to_string(),
            severity: Severity::Warning,
            description: format!(
                "{} error-level log entries (threshold: {})",
                current.log_error_count, LOG_ERROR_THRESHOLD
            ),
            suggestion: "Review recent logs for recurring errors".to_string(),
        });
    }

    // 4. FileChange — check guardian-watched paths for modifications
    match mcp_call(
        "guardian",
        "anomalies",
        serde_json::json!({"resolved": false}),
    ) {
        Ok(val) => {
            if let Some(arr) = val.as_array() {
                for entry in arr {
                    let atype = entry
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    if atype == "file_change" {
                        let desc = entry
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Watched file modified");
                        anomalies.push(Anomaly {
                            detector: "FileChange".to_string(),
                            severity: Severity::Info,
                            description: sanitize_for_terminal(desc),
                            suggestion: "Verify file change is expected".to_string(),
                        });
                    }
                }
            }
        }
        Err(_) => {}
    }

    // 5. ServiceDown — use health-check results from poll_metrics (no re-ping)
    for (svc, _method) in KNOWN_SERVICES {
        if !current.services_up.iter().any(|s| s == svc) {
            anomalies.push(Anomaly {
                detector: "ServiceDown".to_string(),
                severity: Severity::Critical,
                description: format!("MCP service \"{}\" is not responding", svc),
                suggestion: format!("Restart {} service", svc),
            });
        }
    }

    anomalies
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn pad_right(s: &str, width: usize) -> String {
    let char_count = s.chars().count();
    if char_count >= width {
        s.chars().take(width).collect()
    } else {
        format!("{}{}", s, " ".repeat(width - char_count))
    }
}

fn sanitize_for_terminal(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n').collect()
}

// ---------------------------------------------------------------------------
// Auto-response — take actions on anomalies
// ---------------------------------------------------------------------------

fn respond_to_anomalies(anomalies: &[Anomaly]) -> Vec<String> {
    let mut actions = Vec::new();

    for anomaly in anomalies {
        match (anomaly.detector.as_str(), &anomaly.severity) {
            // ProcessCrash + Critical → log alert and attempt restart via command
            ("ProcessCrash", Severity::Critical) => {
                let msg = format!("AUTO-HEAL: Process crash detected — {}", anomaly.description);
                let _ = mcp_call("log", "write", serde_json::json!({
                    "level": "error",
                    "message": msg,
                    "source": "guardian"
                }));
                actions.push(format!("Logged alert: {}", anomaly.description));

                // Try to identify and restart the crashed process
                // Extract process name from description
                if let Some(name) = anomaly.description.split('"').nth(1) {
                    let restart_cmd = format!("/usr/bin/{} &", name);
                    match mcp_call("command", "run", serde_json::json!({"cmd": restart_cmd})) {
                        Ok(_) => {
                            actions.push(format!("Attempted restart of {}", name));
                            let _ = mcp_call("log", "write", serde_json::json!({
                                "level": "info",
                                "message": format!("AUTO-HEAL: Restarted {}", name),
                                "source": "guardian"
                            }));
                        }
                        Err(e) => {
                            actions.push(format!("Failed to restart {}: {}", name, e));
                        }
                    }
                }
            }

            // MemoryThreshold + Critical → log alert + list top memory consumers
            ("MemoryThreshold", Severity::Critical) => {
                let _ = mcp_call("log", "write", serde_json::json!({
                    "level": "error",
                    "message": format!("MEMORY CRITICAL: {}", anomaly.description),
                    "source": "guardian"
                }));
                actions.push(format!("Logged memory alert: {}", anomaly.description));
            }

            // MemoryThreshold + Warning → just log
            ("MemoryThreshold", Severity::Warning) => {
                let _ = mcp_call("log", "write", serde_json::json!({
                    "level": "warning",
                    "message": format!("MEMORY WARNING: {}", anomaly.description),
                    "source": "guardian"
                }));
                actions.push("Logged memory warning".to_string());
            }

            // ServiceDown + Critical → attempt service restart
            ("ServiceDown", Severity::Critical) => {
                // Don't try to restart guardian (that's us)
                if anomaly.description.contains("guardian") {
                    continue;
                }

                let _ = mcp_call("log", "write", serde_json::json!({
                    "level": "error",
                    "message": format!("SERVICE DOWN: {}", anomaly.description),
                    "source": "guardian"
                }));
                actions.push(format!("Logged service alert: {}", anomaly.description));
            }

            // LogError — write summary to log
            ("LogError", _) => {
                let _ = mcp_call("log", "write", serde_json::json!({
                    "level": "warning",
                    "message": format!("LOG AUDIT: {}", anomaly.description),
                    "source": "guardian"
                }));
                actions.push("Logged error audit".to_string());
            }

            // FileChange — acknowledge
            ("FileChange", _) => {
                actions.push(format!("Acknowledged: {}", anomaly.description));
            }

            _ => {}
        }
    }

    actions
}

// ---------------------------------------------------------------------------
// Main — autonomous guardian daemon
// ---------------------------------------------------------------------------

fn main() {
    // Guardian writes to a log file instead of stderr to avoid polluting terminal panes.
    // Use GUARDIAN_LOG env var or default to /tmp/guardian.log
    let log_path = std::env::var("GUARDIAN_LOG")
        .unwrap_or_else(|_| "/tmp/guardian.log".to_string());

    let mut log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();

    macro_rules! glog {
        ($($arg:tt)*) => {
            if let Some(ref mut f) = log_file {
                let _ = writeln!(f, $($arg)*);
                let _ = f.flush();
            }
        }
    }

    glog!("ACOS Guardian v2 started — autonomous mode, polling every {}s", POLL_INTERVAL);

    let mut previous: Option<SystemSnapshot> = None;
    let mut cycle: u64 = 0;

    loop {
        cycle += 1;
        let snapshot = poll_metrics();
        let anomalies = detect_anomalies(&snapshot, &previous);

        // Auto-respond to anomalies
        let actions = respond_to_anomalies(&anomalies);

        // Report to guardian MCP service
        let status = if anomalies.is_empty() { "nominal" } else {
            if anomalies.iter().any(|a| a.severity == Severity::Critical) { "critical" }
            else { "warning" }
        };

        let _ = mcp_call("guardian", "respond", serde_json::json!({
            "cycle": cycle,
            "status": status,
            "anomaly_count": anomalies.len(),
            "action_count": actions.len(),
            "processes": snapshot.processes.len(),
            "memory_percent": snapshot.memory_percent as u64,
        }));

        // Write to log file (not to stderr/terminal)
        glog!("── Guardian [cycle {}] ──────────────────", cycle);
        glog!("  Procs:  {} running", snapshot.processes.len());
        glog!("  Memory: {}/{} MB ({}%)",
            snapshot.memory_used_mb, snapshot.memory_total_mb, snapshot.memory_percent as u64);
        glog!("  Svcs:   {} active", snapshot.service_count);

        if anomalies.is_empty() {
            glog!("  Status: NOMINAL");
        } else {
            for a in &anomalies {
                glog!("  [{}] {}: {}", a.severity.label(), a.detector, a.description);
            }
            if !actions.is_empty() {
                glog!("  Actions taken:");
                for action in &actions {
                    glog!("    -> {}", action);
                }
            }
        }

        previous = Some(snapshot);
        thread::sleep(Duration::from_secs(POLL_INTERVAL));
    }
}
