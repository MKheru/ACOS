//! Boot konsole initialization — creates default konsoles at ACOS startup.

use std::sync::{Arc, Mutex};

use crate::konsole_handler::{Konsole, KonsoleType};
use crate::display_handler::{LayoutNode, Direction};

/// Initialize boot konsoles: Konsole 0 (Root AI) and Konsole 1 (User).
///
/// Called once during McpScheme initialization before any service is started.
///
/// Display layout: 50/50 vertical split with Konsole 1 (mcp-talk/user) on LEFT
/// and Konsole 0 (guardian) on RIGHT. This layout is applied by acos-guardian
/// at startup via mcp://display/layout — it cannot be set here because
/// DisplayHandler is not accessible from this module.
///
/// Target layout JSON:
/// ```json
/// {"type":"split","direction":"vertical","ratio":[1,1],
///  "first":{"type":"leaf","konsole_id":1},
///  "second":{"type":"leaf","konsole_id":0}}
/// ```
pub fn init_boot_konsoles(konsole_state: &Arc<Mutex<Vec<Konsole>>>) {
    let mut konsoles = konsole_state.lock().unwrap_or_else(|e| e.into_inner());

    // Konsole 0: Root AI — always visible, owned by acosd
    let mut root = Konsole::new(0, KonsoleType::RootAi, "acosd".to_string(), 80, 24);
    root.write_data("\x1b[1;36m╔══════════════════════════════════════╗\x1b[0m\n");
    root.write_data("\x1b[1;36m║     ACOS AI Supervisor — Active     ║\x1b[0m\n");
    root.write_data("\x1b[1;36m╚══════════════════════════════════════╝\x1b[0m\n\n");
    konsoles.push(root);

    // Konsole 1: User — default user console
    let user = Konsole::new(1, KonsoleType::User, "login".to_string(), 80, 24);
    konsoles.push(user);
}

/// Create the boot display layout: 50/50 vertical split with Konsole 1 (left) and Konsole 0 (right).
pub fn boot_display_layout() -> LayoutNode {
    LayoutNode::Split {
        direction: Direction::Vertical,
        ratio: (50, 50),
        first: Box::new(LayoutNode::Leaf { konsole_id: 1 }),
        second: Box::new(LayoutNode::Leaf { konsole_id: 0 }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boot_konsoles_created() {
        let state: Arc<Mutex<Vec<Konsole>>> = Arc::new(Mutex::new(Vec::new()));
        init_boot_konsoles(&state);

        let konsoles = state.lock().unwrap();
        assert_eq!(konsoles.len(), 2);

        assert_eq!(konsoles[0].id, 0);
        assert_eq!(konsoles[0].konsole_type, KonsoleType::RootAi);
        assert_eq!(konsoles[0].owner, "acosd");

        assert_eq!(konsoles[1].id, 1);
        assert_eq!(konsoles[1].konsole_type, KonsoleType::User);
        assert_eq!(konsoles[1].owner, "login");
    }
}
