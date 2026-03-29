//! Guardian Handler: system health monitoring and anomaly management for ACOS
//!
//! Monitors process state, memory usage, and logs via MCP dispatch.
//! Detects anomalies, presents them for user response, and can execute
//! remediation actions.

use std::sync::Mutex;

use serde_json::{json, Value};

use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use crate::McpPath;

/// Dispatch function type: (service, method, params) -> JsonRpcResponse
type DispatchFn = Box<dyn Fn(&str, &str, Value) -> JsonRpcResponse + Send + Sync>;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum GuardianStatus {
    Nominal,
    Warning,
    Critical,
}

impl GuardianStatus {
    fn as_str(&self) -> &str {
        match self {
            GuardianStatus::Nominal => "nominal",
            GuardianStatus::Warning => "warning",
            GuardianStatus::Critical => "critical",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u64,
    pub name: String,
    pub state: String,
    pub memory: String,
}

#[derive(Clone, Debug)]
pub struct SystemSnapshot {
    pub process_count: usize,
    pub process_list: Vec<ProcessInfo>,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub memory_percent: f32,
    pub service_count: usize,
    pub log_errors_recent: usize,
    pub uptime_secs: u64,
}

impl Default for SystemSnapshot {
    fn default() -> Self {
        SystemSnapshot {
            process_count: 0,
            process_list: Vec::new(),
            memory_used_mb: 0,
            memory_total_mb: 0,
            memory_percent: 0.0,
            service_count: 0,
            log_errors_recent: 0,
            uptime_secs: 0,
        }
    }
}

pub struct GuardianState {
    pub status: GuardianStatus,
    pub last_check: String,
    pub checks_completed: u64,
    pub current_metrics: SystemSnapshot,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AnomalyType {
    ProcessCrash { pid: u64, name: String },
    MemoryThreshold { percent: f32, threshold: f32 },
    LogError { count: usize, sample: String },
    FileChange { path: String, change_type: String },
    ServiceDown { service: String },
    NetworkAnomaly { description: String, source_ip: String },
    HighTraffic { bytes_per_sec: u64, threshold: u64 },
    UnauthorizedConnection { host: String, port: u16 },
}

impl AnomalyType {
    fn as_str(&self) -> &str {
        match self {
            AnomalyType::ProcessCrash { .. } => "process_crash",
            AnomalyType::MemoryThreshold { .. } => "memory_threshold",
            AnomalyType::LogError { .. } => "log_error",
            AnomalyType::FileChange { .. } => "file_change",
            AnomalyType::ServiceDown { .. } => "service_down",
            AnomalyType::NetworkAnomaly { .. } => "network_anomaly",
            AnomalyType::HighTraffic { .. } => "high_traffic",
            AnomalyType::UnauthorizedConnection { .. } => "unauthorized_connection",
        }
    }

    fn to_json(&self) -> Value {
        match self {
            AnomalyType::ProcessCrash { pid, name } => {
                json!({"type": "process_crash", "pid": pid, "name": name})
            }
            AnomalyType::MemoryThreshold { percent, threshold } => {
                json!({"type": "memory_threshold", "percent": percent, "threshold": threshold})
            }
            AnomalyType::LogError { count, sample } => {
                json!({"type": "log_error", "count": count, "sample": sample})
            }
            AnomalyType::FileChange { path, change_type } => {
                json!({"type": "file_change", "path": path, "change_type": change_type})
            }
            AnomalyType::ServiceDown { service } => {
                json!({"type": "service_down", "service": service})
            }
            AnomalyType::NetworkAnomaly { description, source_ip } => {
                json!({"type": "network_anomaly", "description": description, "source_ip": source_ip})
            }
            AnomalyType::HighTraffic { bytes_per_sec, threshold } => {
                json!({"type": "high_traffic", "bytes_per_sec": bytes_per_sec, "threshold": threshold})
            }
            AnomalyType::UnauthorizedConnection { host, port } => {
                json!({"type": "unauthorized_connection", "host": host, "port": port})
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    fn as_str(&self) -> &str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "info" => Some(Severity::Info),
            "warning" => Some(Severity::Warning),
            "critical" => Some(Severity::Critical),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ResponseChoice {
    ApplyFix,
    Ignore,
    GiveInstructions,
}

impl ResponseChoice {
    fn as_str(&self) -> &str {
        match self {
            ResponseChoice::ApplyFix => "apply_fix",
            ResponseChoice::Ignore => "ignore",
            ResponseChoice::GiveInstructions => "give_instructions",
        }
    }

    fn from_u64(v: u64) -> Option<Self> {
        match v {
            1 => Some(ResponseChoice::ApplyFix),
            2 => Some(ResponseChoice::Ignore),
            3 => Some(ResponseChoice::GiveInstructions),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UserResponse {
    pub choice: ResponseChoice,
    pub instructions: Option<String>,
    pub responded_at: String,
}

pub struct Anomaly {
    pub id: u32,
    pub anomaly_type: AnomalyType,
    pub severity: Severity,
    pub description: String,
    pub detected_at: String,
    pub resolved: bool,
    pub resolution: Option<String>,
    pub user_response: Option<UserResponse>,
    pub ai_consultation: Option<String>,
}

impl Anomaly {
    fn to_json(&self) -> Value {
        let mut obj = json!({
            "id": self.id,
            "anomaly_type": self.anomaly_type.to_json(),
            "severity": self.severity.as_str(),
            "description": self.description,
            "detected_at": self.detected_at,
            "resolved": self.resolved,
        });
        if let Some(ref res) = self.resolution {
            obj["resolution"] = json!(res);
        }
        if let Some(ref ur) = self.user_response {
            obj["user_response"] = json!({
                "choice": ur.choice.as_str(),
                "instructions": ur.instructions,
                "responded_at": ur.responded_at,
            });
        }
        if let Some(ref ai) = self.ai_consultation {
            obj["ai_consultation"] = json!(ai);
        }
        obj
    }
}

pub struct GuardianConfig {
    pub poll_interval_secs: u32,
    pub memory_threshold_percent: f32,
    pub log_error_threshold: usize,
    pub watched_paths: Vec<String>,
    pub enabled: bool,
    pub network_monitoring: bool,
    pub traffic_threshold_bps: u64,
    pub blocked_hosts: Vec<String>,
    pub blocked_ports: Vec<u16>,
    pub ai_consultation_enabled: bool,
    pub llm_model: String,
}

impl Default for GuardianConfig {
    fn default() -> Self {
        GuardianConfig {
            poll_interval_secs: 30,
            memory_threshold_percent: 80.0,
            log_error_threshold: 5,
            watched_paths: Vec::new(),
            enabled: true,
            network_monitoring: true,
            traffic_threshold_bps: 10_000_000,
            blocked_hosts: Vec::new(),
            blocked_ports: Vec::new(),
            ai_consultation_enabled: true,
            llm_model: "qwen2.5:7b-instruct-q4_K_M".to_string(),
        }
    }
}

impl GuardianConfig {
    fn to_json(&self) -> Value {
        json!({
            "poll_interval_secs": self.poll_interval_secs,
            "memory_threshold_percent": self.memory_threshold_percent,
            "log_error_threshold": self.log_error_threshold,
            "watched_paths": self.watched_paths,
            "enabled": self.enabled,
            "network_monitoring": self.network_monitoring,
            "traffic_threshold_bps": self.traffic_threshold_bps,
            "blocked_hosts": self.blocked_hosts,
            "blocked_ports": self.blocked_ports,
            "ai_consultation_enabled": self.ai_consultation_enabled,
            "llm_model": self.llm_model,
        })
    }
}

// ---------------------------------------------------------------------------
// GuardianHandler
// ---------------------------------------------------------------------------

pub struct GuardianHandler {
    state: Mutex<GuardianState>,
    anomalies: Mutex<Vec<Anomaly>>,
    config: Mutex<GuardianConfig>,
    dispatch: DispatchFn,
    next_anomaly_id: Mutex<u32>,
    check_counter: Mutex<u64>,
    /// Monotonic counter used for rate-limiting AI consultations.
    consultation_counter: Mutex<u64>,
}

impl GuardianHandler {
    pub fn new(dispatch: DispatchFn) -> Self {
        GuardianHandler {
            state: Mutex::new(GuardianState {
                status: GuardianStatus::Nominal,
                last_check: String::new(),
                checks_completed: 0,
                current_metrics: SystemSnapshot::default(),
            }),
            anomalies: Mutex::new(Vec::new()),
            config: Mutex::new(GuardianConfig::default()),
            dispatch,
            next_anomaly_id: Mutex::new(1),
            check_counter: Mutex::new(0),
            consultation_counter: Mutex::new(0),
        }
    }

    fn timestamp(&self) -> String {
        let mut counter = self.check_counter.lock().unwrap_or_else(|e| e.into_inner());
        *counter += 1;
        // Try to get actual uptime via dispatch
        let resp = (self.dispatch)("system", "info", json!({}));
        if let Some(ref result) = resp.result {
            let data = result
                .get("text")
                .and_then(|t| t.as_str())
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or_else(|| result.clone());
            if let Some(secs) = data.get("uptime_secs").and_then(|v| v.as_u64()) {
                let mins = secs / 60;
                let rem = secs % 60;
                return format!("{}m {}s (check #{})", mins, rem, counter);
            }
        }
        format!("check #{}", counter)
    }

    fn sanitize_process_name(name: &str) -> Result<&str, String> {
        if name.is_empty() {
            return Err("process name cannot be empty".to_string());
        }
        for ch in name.chars() {
            if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                continue;
            }
            return Err(format!(
                "invalid character '{}' in process name '{}': only alphanumeric, hyphens, underscores, and dots allowed",
                ch, name
            ));
        }
        Ok(name)
    }

    fn has_active_anomaly_of_type(anomalies: &[Anomaly], anomaly_type_tag: &str) -> bool {
        anomalies
            .iter()
            .any(|a| !a.resolved && a.anomaly_type.as_str() == anomaly_type_tag)
    }

    /// Poll system services and build a health snapshot.
    fn handle_state(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let config = self.config.lock().unwrap_or_else(|e| e.into_inner());
        let threshold_mem = config.memory_threshold_percent;
        let threshold_log = config.log_error_threshold;
        drop(config);

        // Poll process list
        let proc_resp = (self.dispatch)("process", "list", json!({}));
        let mut process_list: Vec<ProcessInfo> = Vec::new();
        if let Some(ref result) = proc_resp.result {
            // Try parsing text field as JSON (process handler returns text)
            let data = result
                .get("text")
                .and_then(|t| t.as_str())
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or_else(|| result.clone());

            if let Some(procs) = data.get("processes").and_then(|p| p.as_array()) {
                for p in procs {
                    process_list.push(ProcessInfo {
                        pid: p.get("pid").and_then(|v| v.as_u64()).unwrap_or(0),
                        name: p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        state: p.get("state").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        memory: p.get("memory").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    });
                }
            }
        }

        // Poll memory stats
        let mem_resp = (self.dispatch)("memory", "stats", json!({}));
        let mut memory_used_mb: u64 = 0;
        let mut memory_total_mb: u64 = 0;
        let mut memory_percent: f32 = 0.0;
        if let Some(ref result) = mem_resp.result {
            let data = result
                .get("text")
                .and_then(|t| t.as_str())
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or_else(|| result.clone());

            memory_used_mb = data.get("used_mb").and_then(|v| v.as_u64()).unwrap_or(0);
            memory_total_mb = data.get("total_mb").and_then(|v| v.as_u64()).unwrap_or(0);
            memory_percent = data
                .get("percent")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32;
        }

        // Poll log errors
        let log_resp = (self.dispatch)("log", "read", json!({"level": "error", "count": 50}));
        let mut log_errors_recent: usize = 0;
        let mut log_error_sample = String::new();
        if let Some(ref result) = log_resp.result {
            let data = result
                .get("text")
                .and_then(|t| t.as_str())
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or_else(|| result.clone());

            if let Some(entries) = data.get("entries").and_then(|e| e.as_array()) {
                log_errors_recent = entries.len();
                if let Some(first) = entries.first() {
                    log_error_sample = first
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                }
            }
        }

        let snapshot = SystemSnapshot {
            process_count: process_list.len(),
            process_list,
            memory_used_mb,
            memory_total_mb,
            memory_percent,
            service_count: 0,
            log_errors_recent,
            uptime_secs: 0,
        };

        // Detect anomalies from this check (with deduplication)
        let ts = self.timestamp();
        let mut anomalies = self.anomalies.lock().unwrap_or_else(|e| e.into_inner());
        let mut next_id = self.next_anomaly_id.lock().unwrap_or_else(|e| e.into_inner());

        if memory_percent > threshold_mem {
            if Self::has_active_anomaly_of_type(&anomalies, "memory_threshold") {
                // Update existing anomaly description/timestamp instead of creating duplicate
                if let Some(existing) = anomalies.iter_mut().find(|a| {
                    !a.resolved && a.anomaly_type.as_str() == "memory_threshold"
                }) {
                    existing.description = format!(
                        "Memory usage {:.1}% exceeds threshold {:.1}%",
                        memory_percent, threshold_mem
                    );
                    existing.detected_at = ts.clone();
                }
            } else {
                let id = *next_id;
                *next_id += 1;
                anomalies.push(Anomaly {
                    id,
                    anomaly_type: AnomalyType::MemoryThreshold {
                        percent: memory_percent,
                        threshold: threshold_mem,
                    },
                    severity: if memory_percent > 95.0 {
                        Severity::Critical
                    } else {
                        Severity::Warning
                    },
                    description: format!(
                        "Memory usage {:.1}% exceeds threshold {:.1}%",
                        memory_percent, threshold_mem
                    ),
                    detected_at: ts.clone(),
                    resolved: false,
                    resolution: None,
                    user_response: None,
                    ai_consultation: None,
                });
            }
        }

        if log_errors_recent > threshold_log {
            if Self::has_active_anomaly_of_type(&anomalies, "log_error") {
                if let Some(existing) = anomalies.iter_mut().find(|a| {
                    !a.resolved && a.anomaly_type.as_str() == "log_error"
                }) {
                    existing.description = format!(
                        "{} recent log errors (threshold: {})",
                        log_errors_recent, threshold_log
                    );
                    existing.detected_at = ts.clone();
                }
            } else {
                let id = *next_id;
                *next_id += 1;
                anomalies.push(Anomaly {
                    id,
                    anomaly_type: AnomalyType::LogError {
                        count: log_errors_recent,
                        sample: log_error_sample,
                    },
                    severity: Severity::Warning,
                    description: format!(
                        "{} recent log errors (threshold: {})",
                        log_errors_recent, threshold_log
                    ),
                    detected_at: ts.clone(),
                    resolved: false,
                    resolution: None,
                    user_response: None,
                    ai_consultation: None,
                });
            }
        }

        drop(next_id);

        let has_critical = anomalies
            .iter()
            .any(|a| !a.resolved && a.severity == Severity::Critical);
        let has_warning = anomalies
            .iter()
            .any(|a| !a.resolved && a.severity == Severity::Warning);

        let status = if has_critical {
            GuardianStatus::Critical
        } else if has_warning {
            GuardianStatus::Warning
        } else {
            GuardianStatus::Nominal
        };

        drop(anomalies);

        // Update state
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.status = status.clone();
        state.last_check = ts;
        state.checks_completed += 1;
        state.current_metrics = snapshot;

        let process_json: Vec<Value> = state
            .current_metrics
            .process_list
            .iter()
            .map(|p| {
                json!({
                    "pid": p.pid,
                    "name": p.name,
                    "state": p.state,
                    "memory": p.memory,
                })
            })
            .collect();

        JsonRpcResponse::success(
            request.id.clone(),
            json!({
                "status": status.as_str(),
                "last_check": state.last_check,
                "checks_completed": state.checks_completed,
                "metrics": {
                    "process_count": state.current_metrics.process_count,
                    "processes": process_json,
                    "memory_used_mb": state.current_metrics.memory_used_mb,
                    "memory_total_mb": state.current_metrics.memory_total_mb,
                    "memory_percent": state.current_metrics.memory_percent,
                    "service_count": state.current_metrics.service_count,
                    "log_errors_recent": state.current_metrics.log_errors_recent,
                    "uptime_secs": state.current_metrics.uptime_secs,
                }
            }),
        )
    }

    /// List anomalies with optional filters.
    fn handle_anomalies(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let resolved_filter = request.params.get("resolved").and_then(|v| v.as_bool());
        let severity_filter = request
            .params
            .get("severity")
            .and_then(|v| v.as_str())
            .and_then(Severity::from_str);
        let limit = request
            .params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        let anomalies = self.anomalies.lock().unwrap_or_else(|e| e.into_inner());
        let filtered: Vec<Value> = anomalies
            .iter()
            .filter(|a| {
                if let Some(r) = resolved_filter {
                    if a.resolved != r {
                        return false;
                    }
                }
                if let Some(ref sev) = severity_filter {
                    if a.severity != *sev {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .map(|a| a.to_json())
            .collect();

        JsonRpcResponse::success(
            request.id.clone(),
            json!({ "anomalies": filtered, "total": filtered.len() }),
        )
    }

    /// Process a user response to an anomaly.
    fn handle_respond(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let anomaly_id = match request.params.get("anomaly_id").and_then(|v| v.as_u64()) {
            Some(id) => id as u32,
            None => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    "missing required parameter 'anomaly_id'",
                )
            }
        };

        let choice_num = match request.params.get("choice").and_then(|v| v.as_u64()) {
            Some(c) => c,
            None => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    "missing required parameter 'choice' (1=ApplyFix, 2=Ignore, 3=GiveInstructions)",
                )
            }
        };

        let choice = match ResponseChoice::from_u64(choice_num) {
            Some(c) => c,
            None => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    "invalid choice: must be 1 (ApplyFix), 2 (Ignore), or 3 (GiveInstructions)",
                )
            }
        };

        let instructions = request
            .params
            .get("instructions")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if choice == ResponseChoice::GiveInstructions && instructions.is_none() {
            return JsonRpcResponse::error(
                request.id.clone(),
                INVALID_PARAMS,
                "choice 3 (GiveInstructions) requires 'instructions' parameter",
            );
        }

        let mut anomalies = self.anomalies.lock().unwrap_or_else(|e| e.into_inner());
        let anomaly = match anomalies.iter_mut().find(|a| a.id == anomaly_id) {
            Some(a) => a,
            None => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    format!("anomaly {} not found", anomaly_id),
                )
            }
        };

        if anomaly.resolved {
            return JsonRpcResponse::error(
                request.id.clone(),
                INVALID_PARAMS,
                format!("anomaly {} is already resolved", anomaly_id),
            );
        }

        let respond_ts = self.timestamp();

        let action_taken = match &choice {
            ResponseChoice::ApplyFix => {
                // Execute remediation based on anomaly type
                let action = match &anomaly.anomaly_type {
                    AnomalyType::ProcessCrash { name, .. } => {
                        match Self::sanitize_process_name(name) {
                            Ok(safe_name) => {
                                let _resp = (self.dispatch)(
                                    "process",
                                    "spawn",
                                    json!({"command": safe_name}),
                                );
                                format!("attempted restart of '{}'", safe_name)
                            }
                            Err(e) => {
                                return JsonRpcResponse::error(
                                    request.id.clone(),
                                    INVALID_PARAMS,
                                    format!("cannot apply fix: {}", e),
                                );
                            }
                        }
                    }
                    AnomalyType::MemoryThreshold { .. } => {
                        "acknowledged memory warning — monitor for improvement".to_string()
                    }
                    AnomalyType::LogError { .. } => {
                        "acknowledged log errors — marked for review".to_string()
                    }
                    AnomalyType::ServiceDown { service } => {
                        match Self::sanitize_process_name(service) {
                            Ok(safe_name) => {
                                let _resp = (self.dispatch)(
                                    "process",
                                    "spawn",
                                    json!({"command": safe_name}),
                                );
                                format!("attempted restart of service '{}'", safe_name)
                            }
                            Err(e) => {
                                return JsonRpcResponse::error(
                                    request.id.clone(),
                                    INVALID_PARAMS,
                                    format!("cannot apply fix: {}", e),
                                );
                            }
                        }
                    }
                    AnomalyType::FileChange { path, .. } => {
                        format!("acknowledged file change at '{}'", path)
                    }
                    AnomalyType::NetworkAnomaly { source_ip, .. } => {
                        format!("acknowledged network anomaly from '{}'", source_ip)
                    }
                    AnomalyType::HighTraffic { bytes_per_sec, threshold } => {
                        format!(
                            "acknowledged high traffic: {} B/s (threshold {} B/s)",
                            bytes_per_sec, threshold
                        )
                    }
                    AnomalyType::UnauthorizedConnection { host, port } => {
                        format!("blocked unauthorized connection to {}:{}", host, port)
                    }
                };
                anomaly.resolved = true;
                anomaly.resolution = Some(action.clone());
                action
            }
            ResponseChoice::Ignore => {
                anomaly.resolved = true;
                anomaly.resolution = Some("ignored by user".to_string());
                "ignored by user".to_string()
            }
            ResponseChoice::GiveInstructions => {
                let instr = instructions.clone().unwrap_or_default();
                anomaly.resolved = true;
                anomaly.resolution = Some(format!("user instructions: {}", instr));
                format!("recorded instructions: {}", instr)
            }
        };

        anomaly.user_response = Some(UserResponse {
            choice,
            instructions,
            responded_at: respond_ts,
        });

        JsonRpcResponse::success(
            request.id.clone(),
            json!({
                "ok": true,
                "anomaly_id": anomaly_id,
                "action_taken": action_taken,
            }),
        )
    }

    /// Get or set guardian configuration.
    fn handle_config(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let key = request.params.get("key").and_then(|v| v.as_str());
        let value = request.params.get("value");

        let mut config = self.config.lock().unwrap_or_else(|e| e.into_inner());

        match (key, value) {
            // Set a config value
            (Some(k), Some(v)) => {
                match k {
                    "poll_interval_secs" => {
                        if let Some(n) = v.as_u64() {
                            config.poll_interval_secs = n as u32;
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "poll_interval_secs must be a number",
                            );
                        }
                    }
                    "memory_threshold_percent" => {
                        if let Some(n) = v.as_f64() {
                            config.memory_threshold_percent = n as f32;
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "memory_threshold_percent must be a number",
                            );
                        }
                    }
                    "log_error_threshold" => {
                        if let Some(n) = v.as_u64() {
                            config.log_error_threshold = n as usize;
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "log_error_threshold must be a number",
                            );
                        }
                    }
                    "watched_paths" => {
                        if let Some(arr) = v.as_array() {
                            config.watched_paths = arr
                                .iter()
                                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                                .collect();
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "watched_paths must be an array of strings",
                            );
                        }
                    }
                    "enabled" => {
                        if let Some(b) = v.as_bool() {
                            config.enabled = b;
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "enabled must be a boolean",
                            );
                        }
                    }
                    "network_monitoring" => {
                        if let Some(b) = v.as_bool() {
                            config.network_monitoring = b;
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "network_monitoring must be a boolean",
                            );
                        }
                    }
                    "traffic_threshold_bps" => {
                        if let Some(n) = v.as_u64() {
                            config.traffic_threshold_bps = n;
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "traffic_threshold_bps must be a number",
                            );
                        }
                    }
                    "blocked_hosts" => {
                        if let Some(arr) = v.as_array() {
                            config.blocked_hosts = arr
                                .iter()
                                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                                .collect();
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "blocked_hosts must be an array of strings",
                            );
                        }
                    }
                    "blocked_ports" => {
                        if let Some(arr) = v.as_array() {
                            let ports: Vec<u16> = arr
                                .iter()
                                .filter_map(|p| p.as_u64().map(|n| n as u16))
                                .collect();
                            config.blocked_ports = ports;
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "blocked_ports must be an array of numbers",
                            );
                        }
                    }
                    "ai_consultation_enabled" => {
                        if let Some(b) = v.as_bool() {
                            config.ai_consultation_enabled = b;
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "ai_consultation_enabled must be a boolean",
                            );
                        }
                    }
                    "llm_model" => {
                        if let Some(s) = v.as_str() {
                            if s.is_empty() {
                                return JsonRpcResponse::error(
                                    request.id.clone(),
                                    INVALID_PARAMS,
                                    "llm_model cannot be empty",
                                );
                            }
                            config.llm_model = s.to_string();
                        } else {
                            return JsonRpcResponse::error(
                                request.id.clone(),
                                INVALID_PARAMS,
                                "llm_model must be a string",
                            );
                        }
                    }
                    _ => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            format!("unknown config key '{}'", k),
                        );
                    }
                }
                JsonRpcResponse::success(
                    request.id.clone(),
                    json!({ "ok": true, "config": config.to_json() }),
                )
            }
            // Get a specific config value
            (Some(k), None) => {
                let val = match k {
                    "poll_interval_secs" => json!(config.poll_interval_secs),
                    "memory_threshold_percent" => json!(config.memory_threshold_percent),
                    "log_error_threshold" => json!(config.log_error_threshold),
                    "watched_paths" => json!(config.watched_paths),
                    "enabled" => json!(config.enabled),
                    "network_monitoring" => json!(config.network_monitoring),
                    "traffic_threshold_bps" => json!(config.traffic_threshold_bps),
                    "blocked_hosts" => json!(config.blocked_hosts),
                    "blocked_ports" => json!(config.blocked_ports),
                    "ai_consultation_enabled" => json!(config.ai_consultation_enabled),
                    "llm_model" => json!(config.llm_model),
                    _ => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            format!("unknown config key '{}'", k),
                        );
                    }
                };
                JsonRpcResponse::success(
                    request.id.clone(),
                    json!({ "key": k, "value": val }),
                )
            }
            // Return full config
            _ => JsonRpcResponse::success(
                request.id.clone(),
                json!({ "config": config.to_json() }),
            ),
        }
    }

    /// Return resolved anomalies (history).
    fn handle_history(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let count = request
            .params
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        let anomalies = self.anomalies.lock().unwrap_or_else(|e| e.into_inner());
        let resolved: Vec<Value> = anomalies
            .iter()
            .filter(|a| a.resolved)
            .rev()
            .take(count)
            .map(|a| a.to_json())
            .collect();

        JsonRpcResponse::success(
            request.id.clone(),
            json!({ "resolved_anomalies": resolved, "total": resolved.len() }),
        )
    }

    /// Handle a network event — checks against blocked hosts/ports and traffic threshold.
    /// Params: { event_type: "connection"|"traffic"|"anomaly", details: { ... } }
    fn handle_network_event(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let config = self.config.lock().unwrap_or_else(|e| e.into_inner());
        if !config.network_monitoring {
            return JsonRpcResponse::success(
                request.id.clone(),
                json!({ "ok": true, "action": "ignored", "reason": "network_monitoring disabled" }),
            );
        }

        let event_type = match request.params.get("event_type").and_then(|v| v.as_str()) {
            Some(et) if !et.is_empty() => et.to_string(),
            Some(_) => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    "event_type cannot be empty",
                );
            }
            None => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    "missing required parameter 'event_type'",
                );
            }
        };

        let details = match request.params.get("details") {
            Some(d) if d.is_object() => d.clone(),
            Some(_) => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    "'details' must be an object",
                );
            }
            None => {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    "missing required parameter 'details'",
                );
            }
        };

        // Capture config values before dropping the lock
        let traffic_threshold = config.traffic_threshold_bps;
        let blocked_hosts = config.blocked_hosts.clone();
        let blocked_ports = config.blocked_ports.clone();
        drop(config);

        let ts = self.timestamp();
        let mut anomalies = self.anomalies.lock().unwrap_or_else(|e| e.into_inner());
        let mut next_id = self.next_anomaly_id.lock().unwrap_or_else(|e| e.into_inner());

        match event_type.as_str() {
            "connection" => {
                let host = details.get("host").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let port = details.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;

                if host.is_empty() {
                    return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        "connection event requires 'host' in details",
                    );
                }

                // Check blocked hosts — exact match or suffix match (e.g. ".evil.com" matches "x.evil.com")
                let host_blocked = blocked_hosts.iter().any(|bh| {
                    if bh.starts_with('.') {
                        // Suffix match: ".evil.com" matches "sub.evil.com" and "evil.com"
                        host.ends_with(bh) || host == &bh[1..]
                    } else {
                        host == *bh
                    }
                });
                let port_blocked = blocked_ports.contains(&port);

                if host_blocked || port_blocked {
                    let id = *next_id;
                    *next_id += 1;
                    let desc = if host_blocked && port_blocked {
                        format!("Unauthorized connection to blocked host '{}' on blocked port {}", host, port)
                    } else if host_blocked {
                        format!("Unauthorized connection to blocked host '{}'", host)
                    } else {
                        format!("Unauthorized connection on blocked port {}", port)
                    };
                    anomalies.push(Anomaly {
                        id,
                        anomaly_type: AnomalyType::UnauthorizedConnection {
                            host: host.clone(),
                            port,
                        },
                        severity: Severity::Critical,
                        description: desc.clone(),
                        detected_at: ts,
                        resolved: false,
                        resolution: None,
                        user_response: None,
                        ai_consultation: None,
                    });
                    return JsonRpcResponse::success(
                        request.id.clone(),
                        json!({
                            "ok": true,
                            "action": "blocked",
                            "anomaly_id": id,
                            "description": desc,
                        }),
                    );
                }

                JsonRpcResponse::success(
                    request.id.clone(),
                    json!({ "ok": true, "action": "allowed", "host": host, "port": port }),
                )
            }
            "traffic" => {
                let bytes_per_sec = details.get("bytes_per_sec").and_then(|v| v.as_u64()).unwrap_or(0);
                let source_ip = details.get("source_ip").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();

                if bytes_per_sec > traffic_threshold {
                    let id = *next_id;
                    *next_id += 1;
                    let desc = format!(
                        "High traffic from '{}': {} B/s exceeds threshold {} B/s",
                        source_ip, bytes_per_sec, traffic_threshold
                    );
                    anomalies.push(Anomaly {
                        id,
                        anomaly_type: AnomalyType::HighTraffic {
                            bytes_per_sec,
                            threshold: traffic_threshold,
                        },
                        severity: if bytes_per_sec > traffic_threshold * 5 {
                            Severity::Critical
                        } else {
                            Severity::Warning
                        },
                        description: desc.clone(),
                        detected_at: ts,
                        resolved: false,
                        resolution: None,
                        user_response: None,
                        ai_consultation: None,
                    });
                    return JsonRpcResponse::success(
                        request.id.clone(),
                        json!({
                            "ok": true,
                            "action": "anomaly_created",
                            "anomaly_id": id,
                            "description": desc,
                        }),
                    );
                }

                JsonRpcResponse::success(
                    request.id.clone(),
                    json!({ "ok": true, "action": "within_threshold", "bytes_per_sec": bytes_per_sec }),
                )
            }
            "anomaly" => {
                let description = details.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let source_ip = details.get("source_ip").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();

                if description.is_empty() {
                    return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        "anomaly event requires non-empty 'description' in details",
                    );
                }

                let id = *next_id;
                *next_id += 1;
                anomalies.push(Anomaly {
                    id,
                    anomaly_type: AnomalyType::NetworkAnomaly {
                        description: description.clone(),
                        source_ip: source_ip.clone(),
                    },
                    severity: Severity::Warning,
                    description: description.clone(),
                    detected_at: ts,
                    resolved: false,
                    resolution: None,
                    user_response: None,
                    ai_consultation: None,
                });

                JsonRpcResponse::success(
                    request.id.clone(),
                    json!({
                        "ok": true,
                        "action": "anomaly_created",
                        "anomaly_id": id,
                        "description": description,
                    }),
                )
            }
            other => {
                JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    format!("unknown network event_type '{}'; expected 'connection', 'traffic', or 'anomaly'", other),
                )
            }
        }
    }

    /// Consult AI about an anomaly.
    /// Params: { anomaly_id: u32 } or { description: string, severity: string }
    /// Dispatches to net/llm_request, parses action from response.
    fn handle_consult(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let config = self.config.lock().unwrap_or_else(|e| e.into_inner());
        let ai_enabled = config.ai_consultation_enabled;
        let model = config.llm_model.clone();
        drop(config);

        // Build context: either from anomaly_id or from inline description
        let (anomaly_desc, severity_str, anomaly_id_opt) =
            if let Some(aid) = request.params.get("anomaly_id").and_then(|v| v.as_u64()) {
                let anomalies = self.anomalies.lock().unwrap_or_else(|e| e.into_inner());
                match anomalies.iter().find(|a| a.id == aid as u32) {
                    Some(a) => (a.description.clone(), a.severity.as_str().to_string(), Some(aid as u32)),
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            format!("anomaly {} not found", aid),
                        );
                    }
                }
            } else if let Some(desc) = request.params.get("description").and_then(|v| v.as_str()) {
                if desc.is_empty() {
                    return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        "description cannot be empty",
                    );
                }
                let sev = request.params.get("severity").and_then(|v| v.as_str()).unwrap_or("info").to_string();
                (desc.to_string(), sev, None)
            } else {
                return JsonRpcResponse::error(
                    request.id.clone(),
                    INVALID_PARAMS,
                    "consult requires 'anomaly_id' or 'description'",
                );
            };

        if !ai_enabled {
            // Static rule fallback when AI is disabled
            let action = Self::static_rule_action(&severity_str);
            return JsonRpcResponse::success(
                request.id.clone(),
                json!({
                    "action": action,
                    "reasoning": "AI consultation disabled — using static severity-based rule",
                    "ai_consulted": false,
                    "model": model,
                }),
            );
        }

        // Rate limiting: max 10 consultations per handler lifetime window
        // (simple counter; in production this would be time-windowed)
        {
            let mut counter = self.consultation_counter.lock().unwrap_or_else(|e| e.into_inner());
            *counter += 1;
            if *counter > 100 {
                let action = Self::static_rule_action(&severity_str);
                return JsonRpcResponse::success(
                    request.id.clone(),
                    json!({
                        "action": action,
                        "reasoning": "Rate limit exceeded — using static rule fallback",
                        "ai_consulted": false,
                        "model": model,
                    }),
                );
            }
        }

        // Build system prompt with severity-aware framing
        let system_prompt = match severity_str.as_str() {
            "critical" => format!(
                "You are a CRITICAL security analyst for ACOS. A critical anomaly has been detected: {}. \
                 Respond with EXACTLY one word on the first line: BLOCK, ALLOW, or MONITOR. \
                 Then explain your reasoning briefly.",
                anomaly_desc
            ),
            "warning" => format!(
                "You are a security analyst for ACOS. A warning-level anomaly has been detected: {}. \
                 Respond with EXACTLY one word on the first line: BLOCK, ALLOW, or MONITOR. \
                 Then explain your reasoning briefly.",
                anomaly_desc
            ),
            _ => format!(
                "You are a security analyst for ACOS. An informational anomaly has been detected: {}. \
                 Respond with EXACTLY one word on the first line: BLOCK, ALLOW, or MONITOR. \
                 Then explain your reasoning briefly.",
                anomaly_desc
            ),
        };

        // Dispatch to net/llm_request (OpenAI messages format)
        let llm_params = json!({
            "model": model,
            "messages": [{"role": "user", "content": system_prompt}],
        });
        let resp = (self.dispatch)("net", "llm_request", llm_params);

        // Parse the LLM response
        let (action, reasoning, consulted) = if let Some(ref result) = resp.result {
            let content = result
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("");

            if content.is_empty() {
                // LLM returned empty — fallback
                (Self::static_rule_action(&severity_str), "LLM returned empty response — static rule fallback".to_string(), false)
            } else {
                let first_line = content.lines().next().unwrap_or("").trim().to_uppercase();
                let action = if first_line.contains("BLOCK") {
                    "block"
                } else if first_line.contains("ALLOW") {
                    "allow"
                } else if first_line.contains("MONITOR") {
                    "monitor"
                } else {
                    // Couldn't parse action from LLM — use static fallback but mark as consulted
                    &Self::static_rule_action(&severity_str)
                };
                let reasoning = if content.lines().count() > 1 {
                    content.lines().skip(1).collect::<Vec<_>>().join(" ").trim().to_string()
                } else {
                    content.to_string()
                };
                (action.to_string(), reasoning, true)
            }
        } else {
            // Dispatch error — fallback
            let err_msg = resp.error.as_ref().map(|e| e.message.clone()).unwrap_or_else(|| "unknown error".to_string());
            (
                Self::static_rule_action(&severity_str),
                format!("LLM dispatch failed: {} — static rule fallback", err_msg),
                false,
            )
        };

        // Store consultation result on the anomaly if we have an id
        if let Some(aid) = anomaly_id_opt {
            let mut anomalies = self.anomalies.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(a) = anomalies.iter_mut().find(|a| a.id == aid) {
                a.ai_consultation = Some(format!("action={}, reasoning={}", action, reasoning));
            }
        }

        JsonRpcResponse::success(
            request.id.clone(),
            json!({
                "action": action,
                "reasoning": reasoning,
                "ai_consulted": consulted,
                "model": model,
            }),
        )
    }

    /// Static rule action based on severity — used as fallback when AI is unavailable.
    fn static_rule_action(severity: &str) -> String {
        match severity {
            "critical" => "block".to_string(),
            "warning" => "monitor".to_string(),
            _ => "allow".to_string(),
        }
    }

    /// Add an anomaly programmatically (used by monitoring loop).
    pub fn add_anomaly(
        &self,
        anomaly_type: AnomalyType,
        severity: Severity,
        description: String,
    ) -> u32 {
        let mut next_id = self.next_anomaly_id.lock().unwrap_or_else(|e| e.into_inner());
        let id = *next_id;
        *next_id += 1;
        drop(next_id);

        let detected_at = self.timestamp();
        let anomaly = Anomaly {
            id,
            anomaly_type,
            severity,
            description,
            detected_at,
            resolved: false,
            resolution: None,
            user_response: None,
            ai_consultation: None,
        };

        self.anomalies.lock().unwrap_or_else(|e| e.into_inner()).push(anomaly);
        id
    }
}

impl ServiceHandler for GuardianHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "state" => self.handle_state(request),
            "anomalies" => self.handle_anomalies(request),
            "respond" => self.handle_respond(request),
            "config" => self.handle_config(request),
            "history" => self.handle_history(request),
            "network_event" => self.handle_network_event(request),
            "consult" => self.handle_consult(request),
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in guardian service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["state", "anomalies", "respond", "config", "history", "network_event", "consult"]
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
        Box::new(|service, method, _params| {
            let result = match (service, method) {
                ("process", "list") => json!({
                    "text": "{\"processes\":[{\"pid\":1,\"name\":\"init\",\"state\":\"running\",\"memory\":\"2M\"},{\"pid\":2,\"name\":\"mcpd\",\"state\":\"running\",\"memory\":\"4M\"}]}"
                }),
                ("memory", "stats") => json!({
                    "text": "{\"used_mb\":512,\"total_mb\":1024,\"percent\":50.0}"
                }),
                ("log", "read") => json!({
                    "text": "{\"entries\":[]}"
                }),
                ("process", "spawn") => json!({
                    "text": "spawned"
                }),
                _ => json!({"text": "ok"}),
            };
            JsonRpcResponse::success(Some(json!(1)), result)
        })
    }

    fn high_memory_dispatch() -> DispatchFn {
        Box::new(|service, method, _params| {
            let result = match (service, method) {
                ("process", "list") => json!({
                    "text": "{\"processes\":[]}"
                }),
                ("memory", "stats") => json!({
                    "text": "{\"used_mb\":950,\"total_mb\":1024,\"percent\":92.8}"
                }),
                ("log", "read") => json!({
                    "text": "{\"entries\":[]}"
                }),
                _ => json!({"text": "ok"}),
            };
            JsonRpcResponse::success(Some(json!(1)), result)
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
        McpPath::parse(b"guardian/test").unwrap()
    }

    #[test]
    fn test_state_returns_nominal() {
        let handler = GuardianHandler::new(mock_dispatch());
        let req = make_request("state", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("status").unwrap().as_str().unwrap(), "nominal");
        assert_eq!(result.get("checks_completed").unwrap().as_u64().unwrap(), 1);
    }

    #[test]
    fn test_state_detects_memory_anomaly() {
        let handler = GuardianHandler::new(high_memory_dispatch());
        let req = make_request("state", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("status").unwrap().as_str().unwrap(), "warning");
    }

    #[test]
    fn test_anomalies_list() {
        let handler = GuardianHandler::new(mock_dispatch());
        handler.add_anomaly(
            AnomalyType::ServiceDown {
                service: "httpd".into(),
            },
            Severity::Critical,
            "httpd is down".into(),
        );

        let req = make_request("anomalies", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            list[0].get("severity").unwrap().as_str().unwrap(),
            "critical"
        );
    }

    #[test]
    fn test_anomalies_filter_resolved() {
        let handler = GuardianHandler::new(mock_dispatch());
        handler.add_anomaly(
            AnomalyType::ServiceDown {
                service: "httpd".into(),
            },
            Severity::Critical,
            "httpd is down".into(),
        );

        let req = make_request("anomalies", json!({"resolved": true}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_respond_apply_fix() {
        let handler = GuardianHandler::new(mock_dispatch());
        let id = handler.add_anomaly(
            AnomalyType::ServiceDown {
                service: "httpd".into(),
            },
            Severity::Critical,
            "httpd is down".into(),
        );

        let req = make_request("respond", json!({"anomaly_id": id, "choice": 1}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("ok").unwrap().as_bool().unwrap(), true);
        assert!(result
            .get("action_taken")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("restart"));

        // Should be resolved now
        let req = make_request("anomalies", json!({"resolved": false}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_respond_ignore() {
        let handler = GuardianHandler::new(mock_dispatch());
        let id = handler.add_anomaly(
            AnomalyType::LogError {
                count: 10,
                sample: "error".into(),
            },
            Severity::Warning,
            "too many errors".into(),
        );

        let req = make_request("respond", json!({"anomaly_id": id, "choice": 2}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.result.is_some());
    }

    #[test]
    fn test_respond_give_instructions() {
        let handler = GuardianHandler::new(mock_dispatch());
        let id = handler.add_anomaly(
            AnomalyType::ProcessCrash {
                pid: 42,
                name: "test".into(),
            },
            Severity::Critical,
            "process crashed".into(),
        );

        let req = make_request(
            "respond",
            json!({"anomaly_id": id, "choice": 3, "instructions": "restart with --safe flag"}),
        );
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert!(result
            .get("action_taken")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("--safe flag"));
    }

    #[test]
    fn test_respond_missing_instructions() {
        let handler = GuardianHandler::new(mock_dispatch());
        let id = handler.add_anomaly(
            AnomalyType::ProcessCrash {
                pid: 42,
                name: "test".into(),
            },
            Severity::Critical,
            "process crashed".into(),
        );

        let req = make_request("respond", json!({"anomaly_id": id, "choice": 3}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_respond_already_resolved() {
        let handler = GuardianHandler::new(mock_dispatch());
        let id = handler.add_anomaly(
            AnomalyType::ServiceDown {
                service: "httpd".into(),
            },
            Severity::Critical,
            "httpd down".into(),
        );

        let req = make_request("respond", json!({"anomaly_id": id, "choice": 2}));
        handler.handle(&path(), &req);

        let req = make_request("respond", json!({"anomaly_id": id, "choice": 1}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_respond_invalid_anomaly() {
        let handler = GuardianHandler::new(mock_dispatch());
        let req = make_request("respond", json!({"anomaly_id": 999, "choice": 1}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_config_get_all() {
        let handler = GuardianHandler::new(mock_dispatch());
        let req = make_request("config", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let cfg = result.get("config").unwrap();
        assert_eq!(cfg.get("poll_interval_secs").unwrap().as_u64().unwrap(), 30);
        assert_eq!(cfg.get("enabled").unwrap().as_bool().unwrap(), true);
    }

    #[test]
    fn test_config_get_key() {
        let handler = GuardianHandler::new(mock_dispatch());
        let req = make_request("config", json!({"key": "memory_threshold_percent"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("value").unwrap().as_f64().unwrap(), 80.0);
    }

    #[test]
    fn test_config_set_key() {
        let handler = GuardianHandler::new(mock_dispatch());
        let req = make_request(
            "config",
            json!({"key": "poll_interval_secs", "value": 60}),
        );
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("ok").unwrap().as_bool().unwrap(), true);

        // Verify it changed
        let req = make_request("config", json!({"key": "poll_interval_secs"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("value").unwrap().as_u64().unwrap(), 60);
    }

    #[test]
    fn test_config_unknown_key() {
        let handler = GuardianHandler::new(mock_dispatch());
        let req = make_request("config", json!({"key": "nonexistent"}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_history_returns_resolved() {
        let handler = GuardianHandler::new(mock_dispatch());
        let id = handler.add_anomaly(
            AnomalyType::ServiceDown {
                service: "httpd".into(),
            },
            Severity::Critical,
            "httpd down".into(),
        );

        // Resolve it
        let req = make_request("respond", json!({"anomaly_id": id, "choice": 2}));
        handler.handle(&path(), &req);

        let req = make_request("history", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result
            .get("resolved_anomalies")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn test_history_empty() {
        let handler = GuardianHandler::new(mock_dispatch());
        let req = make_request("history", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result
            .get("resolved_anomalies")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_unknown_method() {
        let handler = GuardianHandler::new(mock_dispatch());
        let req = make_request("nonexistent", json!({}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, METHOD_NOT_FOUND);
    }

    #[test]
    fn test_list_methods() {
        let handler = GuardianHandler::new(mock_dispatch());
        let methods = handler.list_methods();
        assert_eq!(methods.len(), 7);
        assert!(methods.contains(&"state"));
        assert!(methods.contains(&"anomalies"));
        assert!(methods.contains(&"respond"));
        assert!(methods.contains(&"config"));
        assert!(methods.contains(&"history"));
        assert!(methods.contains(&"network_event"));
        assert!(methods.contains(&"consult"));
    }

    #[test]
    fn test_anomalies_empty_list() {
        let handler = GuardianHandler::new(mock_dispatch());
        let req = make_request("anomalies", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 0);
        assert_eq!(result.get("total").unwrap().as_u64().unwrap(), 0);
    }

    #[test]
    fn test_anomalies_filter_by_severity() {
        let handler = GuardianHandler::new(mock_dispatch());
        // Add a critical anomaly
        handler.add_anomaly(
            AnomalyType::ServiceDown {
                service: "httpd".into(),
            },
            Severity::Critical,
            "httpd is down".into(),
        );
        // Add a warning anomaly
        handler.add_anomaly(
            AnomalyType::LogError {
                count: 5,
                sample: "error".into(),
            },
            Severity::Warning,
            "some errors".into(),
        );

        // Filter by warning severity
        let req = make_request("anomalies", json!({"severity": "warning"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].get("severity").unwrap().as_str().unwrap(), "warning");

        // Filter by critical severity
        let req = make_request("anomalies", json!({"severity": "critical"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].get("severity").unwrap().as_str().unwrap(), "critical");
    }

    #[test]
    fn test_anomalies_filter_by_limit() {
        let handler = GuardianHandler::new(mock_dispatch());
        // Add 5 anomalies
        for i in 0..5 {
            handler.add_anomaly(
                AnomalyType::LogError {
                    count: i,
                    sample: format!("error {}", i),
                },
                Severity::Warning,
                format!("log error {}", i),
            );
        }

        // Request with limit 2
        let req = make_request("anomalies", json!({"limit": 2}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 2);

        // Request with limit 10 (should return all 5)
        let req = make_request("anomalies", json!({"limit": 10}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 5);
    }

    #[test]
    fn test_anomalies_filter_severity_and_resolved() {
        let handler = GuardianHandler::new(mock_dispatch());
        // Add critical anomaly
        let id1 = handler.add_anomaly(
            AnomalyType::ServiceDown {
                service: "httpd".into(),
            },
            Severity::Critical,
            "httpd is down".into(),
        );
        // Add warning anomaly
        let _id2 = handler.add_anomaly(
            AnomalyType::LogError {
                count: 5,
                sample: "error".into(),
            },
            Severity::Warning,
            "some errors".into(),
        );

        // Resolve the critical anomaly
        let req = make_request("respond", json!({"anomaly_id": id1, "choice": 2}));
        handler.handle(&path(), &req);

        // Filter by warning and unresolved
        let req = make_request(
            "anomalies",
            json!({"severity": "warning", "resolved": false}),
        );
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].get("severity").unwrap().as_str().unwrap(), "warning");

        // Filter by critical and resolved
        let req = make_request(
            "anomalies",
            json!({"severity": "critical", "resolved": true}),
        );
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].get("severity").unwrap().as_str().unwrap(), "critical");
        assert_eq!(list[0].get("resolved").unwrap().as_bool().unwrap(), true);
    }

    #[test]
    fn test_respond_invalid_choice() {
        let handler = GuardianHandler::new(mock_dispatch());
        let id = handler.add_anomaly(
            AnomalyType::ProcessCrash {
                pid: 42,
                name: "test".into(),
            },
            Severity::Critical,
            "process crashed".into(),
        );

        // Try choice 4 (invalid)
        let req = make_request("respond", json!({"anomaly_id": id, "choice": 4}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_respond_missing_anomaly_id() {
        let handler = GuardianHandler::new(mock_dispatch());
        let req = make_request("respond", json!({"choice": 1}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_respond_missing_choice() {
        let handler = GuardianHandler::new(mock_dispatch());
        let id = handler.add_anomaly(
            AnomalyType::ProcessCrash {
                pid: 42,
                name: "test".into(),
            },
            Severity::Critical,
            "process crashed".into(),
        );

        let req = make_request("respond", json!({"anomaly_id": id}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    #[test]
    fn test_config_set_then_get() {
        let handler = GuardianHandler::new(mock_dispatch());

        // Set a value
        let req = make_request(
            "config",
            json!({"key": "memory_threshold_percent", "value": 75}),
        );
        let resp = handler.handle(&path(), &req);
        assert!(resp.result.is_some());

        // Get it back
        let req = make_request("config", json!({"key": "memory_threshold_percent"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("value").unwrap().as_f64().unwrap(), 75.0);
    }

    #[test]
    fn test_history_mixed_resolved_unresolved() {
        let handler = GuardianHandler::new(mock_dispatch());

        // Add 3 anomalies
        let id1 = handler.add_anomaly(
            AnomalyType::ServiceDown {
                service: "httpd".into(),
            },
            Severity::Critical,
            "httpd down".into(),
        );
        let id2 = handler.add_anomaly(
            AnomalyType::LogError {
                count: 10,
                sample: "error".into(),
            },
            Severity::Warning,
            "too many errors".into(),
        );
        let id3 = handler.add_anomaly(
            AnomalyType::ProcessCrash {
                pid: 99,
                name: "worker".into(),
            },
            Severity::Warning,
            "process crashed".into(),
        );

        // Resolve 2 of them
        let req = make_request("respond", json!({"anomaly_id": id1, "choice": 2}));
        handler.handle(&path(), &req);
        let req = make_request("respond", json!({"anomaly_id": id3, "choice": 2}));
        handler.handle(&path(), &req);

        // Get history (resolved only)
        let req = make_request("history", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result
            .get("resolved_anomalies")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(list.len(), 2);

        // Verify id2 is still unresolved in anomalies list
        let req = make_request("anomalies", json!({"resolved": false}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Adversary Task 4 Tests — Network monitoring + AI consultation
    // -----------------------------------------------------------------------

    fn mock_dispatch_with_llm() -> DispatchFn {
        Box::new(|service, method, _params| {
            let result = match (service, method) {
                ("process", "list") => json!({
                    "text": "{\"processes\":[]}"
                }),
                ("memory", "stats") => json!({
                    "text": "{\"used_mb\":512,\"total_mb\":1024,\"percent\":50.0}"
                }),
                ("log", "read") => json!({
                    "text": "{\"entries\":[]}"
                }),
                ("net", "llm_request") => json!({
                    "message": {"role": "assistant", "content": "MONITOR\nThe anomaly seems benign but warrants observation."},
                    "model": "qwen2.5:7b-instruct-q4_K_M",
                    "finish_reason": "stop"
                }),
                ("system", "info") => json!({
                    "text": "{\"uptime_secs\": 300}"
                }),
                _ => json!({"text": "ok"}),
            };
            JsonRpcResponse::success(Some(json!(1)), result)
        })
    }

    fn mock_dispatch_llm_block() -> DispatchFn {
        Box::new(|service, method, _params| {
            let result = match (service, method) {
                ("net", "llm_request") => json!({
                    "message": {"role": "assistant", "content": "BLOCK\nThis looks like a serious threat."},
                    "model": "qwen2.5:7b-instruct-q4_K_M",
                    "finish_reason": "stop"
                }),
                ("system", "info") => json!({"text": "{\"uptime_secs\":100}"}),
                _ => json!({"text": "ok"}),
            };
            JsonRpcResponse::success(Some(json!(1)), result)
        })
    }

    fn mock_dispatch_llm_error() -> DispatchFn {
        Box::new(|service, method, _params| {
            match (service, method) {
                ("net", "llm_request") => {
                    JsonRpcResponse::error(Some(json!(1)), INTERNAL_ERROR, "LLM unavailable")
                }
                ("system", "info") => {
                    JsonRpcResponse::success(Some(json!(1)), json!({"text": "{\"uptime_secs\":100}"}))
                }
                _ => JsonRpcResponse::success(Some(json!(1)), json!({"text": "ok"})),
            }
        })
    }

    fn mock_dispatch_llm_empty() -> DispatchFn {
        Box::new(|service, method, _params| {
            let result = match (service, method) {
                ("net", "llm_request") => json!({
                    "message": {"role": "assistant", "content": ""},
                    "model": "qwen2.5:7b-instruct-q4_K_M",
                    "finish_reason": "stop"
                }),
                ("system", "info") => json!({"text": "{\"uptime_secs\":100}"}),
                _ => json!({"text": "ok"}),
            };
            JsonRpcResponse::success(Some(json!(1)), result)
        })
    }

    // -- Test 1: network_event connection to blocked host creates anomaly
    #[test]
    fn test_network_event_blocked_host() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        // Configure blocked hosts
        let req = make_request("config", json!({"key": "blocked_hosts", "value": ["evil.com", "malware.net"]}));
        handler.handle(&path(), &req);

        let req = make_request("network_event", json!({
            "event_type": "connection",
            "details": {"host": "evil.com", "port": 443}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "blocked");
        assert!(result.get("anomaly_id").is_some());

        // Verify anomaly was created
        let req = make_request("anomalies", json!({"resolved": false}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            list[0].get("anomaly_type").unwrap().get("type").unwrap().as_str().unwrap(),
            "unauthorized_connection"
        );
    }

    // -- Test 2: network_event connection to blocked port
    #[test]
    fn test_network_event_blocked_port() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("config", json!({"key": "blocked_ports", "value": [22, 23]}));
        handler.handle(&path(), &req);

        let req = make_request("network_event", json!({
            "event_type": "connection",
            "details": {"host": "safe.example.com", "port": 22}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "blocked");
    }

    // -- Test 3: network_event connection allowed when not blocked
    #[test]
    fn test_network_event_connection_allowed() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("network_event", json!({
            "event_type": "connection",
            "details": {"host": "google.com", "port": 443}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "allowed");
    }

    // -- Test 4: network_event high traffic creates anomaly
    #[test]
    fn test_network_event_high_traffic() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        // Default threshold is 10M
        let req = make_request("network_event", json!({
            "event_type": "traffic",
            "details": {"bytes_per_sec": 20_000_000, "source_ip": "10.0.0.5"}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "anomaly_created");
    }

    // -- Test 5: traffic within threshold is fine
    #[test]
    fn test_network_event_traffic_within_threshold() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("network_event", json!({
            "event_type": "traffic",
            "details": {"bytes_per_sec": 5_000_000, "source_ip": "10.0.0.5"}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "within_threshold");
    }

    // -- Test 6: network_event with empty event_type
    #[test]
    fn test_network_event_empty_event_type() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("network_event", json!({
            "event_type": "",
            "details": {}
        }));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("empty"));
    }

    // -- Test 7: network_event missing event_type
    #[test]
    fn test_network_event_missing_event_type() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("network_event", json!({"details": {}}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    // -- Test 8: network_event anomaly type with empty description
    #[test]
    fn test_network_event_anomaly_empty_description() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("network_event", json!({
            "event_type": "anomaly",
            "details": {"description": "", "source_ip": "10.0.0.1"}
        }));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("non-empty"));
    }

    // -- Test 9: network_event unknown event_type
    #[test]
    fn test_network_event_unknown_type() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("network_event", json!({
            "event_type": "quantum_flux",
            "details": {}
        }));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("quantum_flux"));
    }

    // -- Test 10: network_event disabled via config
    #[test]
    fn test_network_event_monitoring_disabled() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("config", json!({"key": "network_monitoring", "value": false}));
        handler.handle(&path(), &req);

        let req = make_request("network_event", json!({
            "event_type": "connection",
            "details": {"host": "evil.com", "port": 80}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "ignored");
    }

    // -- Test 11: consult with anomaly_id — AI returns MONITOR
    #[test]
    fn test_consult_with_anomaly_id() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let id = handler.add_anomaly(
            AnomalyType::NetworkAnomaly {
                description: "suspicious traffic".into(),
                source_ip: "10.0.0.1".into(),
            },
            Severity::Warning,
            "suspicious traffic pattern".into(),
        );

        let req = make_request("consult", json!({"anomaly_id": id}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "monitor");
        assert_eq!(result.get("ai_consulted").unwrap().as_bool().unwrap(), true);
        assert!(result.get("model").is_some());

        // Verify ai_consultation was stored on the anomaly
        let req = make_request("anomalies", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert!(list[0].get("ai_consultation").is_some());
    }

    // -- Test 12: consult with inline description (no anomaly_id)
    #[test]
    fn test_consult_inline_description() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("consult", json!({
            "description": "port scan detected from 192.168.1.100",
            "severity": "warning"
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("ai_consulted").unwrap().as_bool().unwrap(), true);
        assert!(["block", "allow", "monitor"].contains(
            &result.get("action").unwrap().as_str().unwrap()
        ));
    }

    // -- Test 13: consult with AI disabled — static fallback
    #[test]
    fn test_consult_ai_disabled() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("config", json!({"key": "ai_consultation_enabled", "value": false}));
        handler.handle(&path(), &req);

        let req = make_request("consult", json!({
            "description": "test anomaly",
            "severity": "critical"
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("ai_consulted").unwrap().as_bool().unwrap(), false);
        // Critical severity -> static rule should be "block"
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "block");
    }

    // -- Test 14: consult with nonexistent anomaly_id
    #[test]
    fn test_consult_nonexistent_anomaly() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("consult", json!({"anomaly_id": 9999}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("9999"));
    }

    // -- Test 15: consult with empty description
    #[test]
    fn test_consult_empty_description() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("consult", json!({"description": ""}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    // -- Test 16: consult missing both anomaly_id and description
    #[test]
    fn test_consult_missing_params() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("consult", json!({}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
    }

    // -- Test 17: consult when LLM returns error — fallback to static rule
    #[test]
    fn test_consult_llm_error_fallback() {
        let handler = GuardianHandler::new(mock_dispatch_llm_error());
        let req = make_request("consult", json!({
            "description": "suspicious activity",
            "severity": "critical"
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("ai_consulted").unwrap().as_bool().unwrap(), false);
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "block");
        assert!(result.get("reasoning").unwrap().as_str().unwrap().contains("failed"));
    }

    // -- Test 18: consult when LLM returns empty content — fallback
    #[test]
    fn test_consult_llm_empty_response() {
        let handler = GuardianHandler::new(mock_dispatch_llm_empty());
        let req = make_request("consult", json!({
            "description": "something odd",
            "severity": "warning"
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("ai_consulted").unwrap().as_bool().unwrap(), false);
        // Warning -> static rule = "monitor"
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "monitor");
    }

    // -- Test 19: LLM returns BLOCK action
    #[test]
    fn test_consult_llm_block_action() {
        let handler = GuardianHandler::new(mock_dispatch_llm_block());
        let req = make_request("consult", json!({
            "description": "DDoS detected",
            "severity": "critical"
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "block");
        assert_eq!(result.get("ai_consulted").unwrap().as_bool().unwrap(), true);
    }

    // -- Test 20: new AnomalyType JSON serialization
    #[test]
    fn test_new_anomaly_types_json() {
        let na = AnomalyType::NetworkAnomaly {
            description: "weird packets".into(),
            source_ip: "192.168.1.1".into(),
        };
        let j = na.to_json();
        assert_eq!(j.get("type").unwrap().as_str().unwrap(), "network_anomaly");
        assert_eq!(j.get("source_ip").unwrap().as_str().unwrap(), "192.168.1.1");

        let ht = AnomalyType::HighTraffic {
            bytes_per_sec: 50_000_000,
            threshold: 10_000_000,
        };
        let j = ht.to_json();
        assert_eq!(j.get("type").unwrap().as_str().unwrap(), "high_traffic");
        assert_eq!(j.get("bytes_per_sec").unwrap().as_u64().unwrap(), 50_000_000);

        let uc = AnomalyType::UnauthorizedConnection {
            host: "evil.com".into(),
            port: 8080,
        };
        let j = uc.to_json();
        assert_eq!(j.get("type").unwrap().as_str().unwrap(), "unauthorized_connection");
        assert_eq!(j.get("port").unwrap().as_u64().unwrap(), 8080);
    }

    // -- Test 21: config get/set for new network fields
    #[test]
    fn test_config_new_network_fields() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());

        // Check defaults
        let req = make_request("config", json!({}));
        let resp = handler.handle(&path(), &req);
        let cfg = resp.result.unwrap().get("config").unwrap().clone();
        assert_eq!(cfg.get("network_monitoring").unwrap().as_bool().unwrap(), true);
        assert_eq!(cfg.get("traffic_threshold_bps").unwrap().as_u64().unwrap(), 10_000_000);
        assert_eq!(cfg.get("ai_consultation_enabled").unwrap().as_bool().unwrap(), true);
        assert_eq!(cfg.get("llm_model").unwrap().as_str().unwrap(), "qwen2.5:7b-instruct-q4_K_M");
        assert!(cfg.get("blocked_hosts").unwrap().as_array().unwrap().is_empty());
        assert!(cfg.get("blocked_ports").unwrap().as_array().unwrap().is_empty());

        // Set and verify
        let req = make_request("config", json!({"key": "traffic_threshold_bps", "value": 5000000}));
        handler.handle(&path(), &req);
        let req = make_request("config", json!({"key": "traffic_threshold_bps"}));
        let resp = handler.handle(&path(), &req);
        assert_eq!(resp.result.unwrap().get("value").unwrap().as_u64().unwrap(), 5_000_000);

        // Set llm_model
        let req = make_request("config", json!({"key": "llm_model", "value": "llama3:8b"}));
        handler.handle(&path(), &req);
        let req = make_request("config", json!({"key": "llm_model"}));
        let resp = handler.handle(&path(), &req);
        assert_eq!(resp.result.unwrap().get("value").unwrap().as_str().unwrap(), "llama3:8b");
    }

    // -- Test 22: config set llm_model to empty string rejected
    #[test]
    fn test_config_llm_model_empty_rejected() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("config", json!({"key": "llm_model", "value": ""}));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("empty"));
    }

    // -- Test 23: blocked host suffix matching (.evil.com matches sub.evil.com)
    #[test]
    fn test_network_event_blocked_host_suffix() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("config", json!({"key": "blocked_hosts", "value": [".evil.com"]}));
        handler.handle(&path(), &req);

        // sub.evil.com should match .evil.com suffix
        let req = make_request("network_event", json!({
            "event_type": "connection",
            "details": {"host": "sub.evil.com", "port": 80}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "blocked");

        // evil.com itself should also match .evil.com
        let req = make_request("network_event", json!({
            "event_type": "connection",
            "details": {"host": "evil.com", "port": 80}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "blocked");

        // notevil.com should NOT match
        let req = make_request("network_event", json!({
            "event_type": "connection",
            "details": {"host": "notevil.com", "port": 80}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "allowed");
    }

    // -- Test 24: connection event missing host
    #[test]
    fn test_network_event_connection_missing_host() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("network_event", json!({
            "event_type": "connection",
            "details": {"port": 80}
        }));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("host"));
    }

    // -- Test 25: high traffic critical severity (5x threshold)
    #[test]
    fn test_network_event_traffic_critical_severity() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        // 60M is > 5 * 10M threshold -> critical
        let req = make_request("network_event", json!({
            "event_type": "traffic",
            "details": {"bytes_per_sec": 60_000_000, "source_ip": "10.0.0.5"}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "anomaly_created");

        // Verify the severity is critical
        let req = make_request("anomalies", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        assert_eq!(list[0].get("severity").unwrap().as_str().unwrap(), "critical");
    }

    // -- Test 26: network_event details must be object
    #[test]
    fn test_network_event_details_not_object() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("network_event", json!({
            "event_type": "connection",
            "details": "not an object"
        }));
        let resp = handler.handle(&path(), &req);
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("object"));
    }

    // -- Test 27: static rule fallback for different severities
    #[test]
    fn test_static_rule_fallback_severities() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        // Disable AI to force static fallback
        let req = make_request("config", json!({"key": "ai_consultation_enabled", "value": false}));
        handler.handle(&path(), &req);

        // Info -> allow
        let req = make_request("consult", json!({"description": "test", "severity": "info"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "allow");

        // Warning -> monitor
        let req = make_request("consult", json!({"description": "test", "severity": "warning"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "monitor");

        // Critical -> block
        let req = make_request("consult", json!({"description": "test", "severity": "critical"}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "block");
    }

    // -- Test 28: network anomaly event creates proper NetworkAnomaly type
    #[test]
    fn test_network_event_anomaly_creates_correct_type() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());
        let req = make_request("network_event", json!({
            "event_type": "anomaly",
            "details": {"description": "weird packets detected", "source_ip": "10.0.0.99"}
        }));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert_eq!(result.get("action").unwrap().as_str().unwrap(), "anomaly_created");

        // Verify anomaly type
        let req = make_request("anomalies", json!({}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        let list = result.get("anomalies").unwrap().as_array().unwrap();
        let at = list[0].get("anomaly_type").unwrap();
        assert_eq!(at.get("type").unwrap().as_str().unwrap(), "network_anomaly");
        assert_eq!(at.get("source_ip").unwrap().as_str().unwrap(), "10.0.0.99");
    }

    // -- Test 29: respond to network anomaly types with ApplyFix
    #[test]
    fn test_respond_apply_fix_network_anomaly_types() {
        let handler = GuardianHandler::new(mock_dispatch_with_llm());

        // Add an UnauthorizedConnection anomaly
        let id = handler.add_anomaly(
            AnomalyType::UnauthorizedConnection {
                host: "evil.com".into(),
                port: 8080,
            },
            Severity::Critical,
            "blocked connection".into(),
        );
        let req = make_request("respond", json!({"anomaly_id": id, "choice": 1}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert!(result.get("action_taken").unwrap().as_str().unwrap().contains("blocked"));

        // Add HighTraffic and ignore it
        let id2 = handler.add_anomaly(
            AnomalyType::HighTraffic {
                bytes_per_sec: 50_000_000,
                threshold: 10_000_000,
            },
            Severity::Warning,
            "too much traffic".into(),
        );
        let req = make_request("respond", json!({"anomaly_id": id2, "choice": 1}));
        let resp = handler.handle(&path(), &req);
        let result = resp.result.unwrap();
        assert!(result.get("action_taken").unwrap().as_str().unwrap().contains("high traffic"));
    }
}
