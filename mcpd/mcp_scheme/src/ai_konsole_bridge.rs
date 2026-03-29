//! Bridge that writes AI activity to Konsole 0 (Root AI).

use std::sync::{Arc, Mutex};

use crate::konsole_handler::Konsole;

/// Writes AI activity lines to the Root AI konsole (id = 0).
pub struct AiKonsoleBridge {
    konsole_state: Arc<Mutex<Vec<Konsole>>>,
    root_konsole_id: u32,
}

impl AiKonsoleBridge {
    pub fn new(konsole_state: Arc<Mutex<Vec<Konsole>>>) -> Self {
        Self { konsole_state, root_konsole_id: 0 }
    }

    /// Write a plain activity line to the Root konsole.
    pub fn log_activity(&self, message: &str) {
        if let Ok(mut konsoles) = self.konsole_state.lock() {
            if let Some(root) = konsoles.iter_mut().find(|k| k.id == self.root_konsole_id) {
                root.write_data(message);
                root.write_data("\n");
            }
        }
    }

    /// Log an AI tool call with yellow colored output.
    pub fn log_tool_call(&self, service: &str, method: &str) {
        let msg = format!("\x1b[33m▶ AI call:\x1b[0m {}/{}", service, method);
        self.log_activity(&msg);
    }

    /// Log an AI tool result with green (success) or red (failure) indicator.
    pub fn log_tool_result(&self, service: &str, success: bool) {
        let status = if success { "\x1b[32m✓\x1b[0m" } else { "\x1b[31m✗\x1b[0m" };
        let msg = format!("  {} {}", status, service);
        self.log_activity(&msg);
    }

    /// Log periodic system stats in dim style.
    pub fn log_system_stats(&self, stats: &str) {
        let msg = format!("\x1b[90m[stats]\x1b[0m {}", stats);
        self.log_activity(&msg);
    }

    /// Cross-console notification: alert from a named source in bold magenta.
    pub fn notify(&self, source: &str, message: &str) {
        let msg = format!("\x1b[1;35m⚡ [{}]\x1b[0m {}", source, message);
        self.log_activity(&msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot_konsoles::init_boot_konsoles;

    fn make_state() -> Arc<Mutex<Vec<Konsole>>> {
        let state = Arc::new(Mutex::new(Vec::new()));
        init_boot_konsoles(&state);
        state
    }

    /// Collect all printable chars from konsole 0's buffer.
    fn read_konsole0_text(state: &Arc<Mutex<Vec<Konsole>>>) -> String {
        let konsoles = state.lock().unwrap();
        let root = konsoles.iter().find(|k| k.id == 0).expect("konsole 0 missing");
        root.buffer
            .iter()
            .flat_map(|row| row.iter().map(|c| c.ch))
            .collect::<String>()
    }

    #[test]
    fn test_ai_bridge_log() {
        let state = make_state();
        let bridge = AiKonsoleBridge::new(Arc::clone(&state));
        bridge.log_activity("hello bridge");

        let text = read_konsole0_text(&state);
        assert!(text.contains("hello bridge"), "expected 'hello bridge' in konsole 0");
    }

    #[test]
    fn test_ai_bridge_tool_call() {
        let state = make_state();
        let bridge = AiKonsoleBridge::new(Arc::clone(&state));
        bridge.log_tool_call("system", "info");

        let text = read_konsole0_text(&state);
        assert!(text.contains("system/info"), "expected 'system/info' in konsole 0");
    }

    #[test]
    fn test_ai_bridge_notify() {
        let state = make_state();
        let bridge = AiKonsoleBridge::new(Arc::clone(&state));
        bridge.notify("watchdog", "CPU overload");

        let text = read_konsole0_text(&state);
        assert!(text.contains("watchdog"), "expected source 'watchdog' in konsole 0");
        assert!(text.contains("CPU overload"), "expected message in konsole 0");
    }
}
