//! Closed WS13 UI action contract.
//!
//! This module deliberately defines a finite enum before any HTTP or WebSocket
//! gateway exists. A future action gateway must deserialize into this enum and
//! map each variant to ACOS authority checks; it must not accept generic
//! `execute`, `rpc`, `method`, shell, or free-form command payloads.

use serde::{Deserialize, Serialize};

/// Deterministic authority target associated with each [`UiAction`].
///
/// The variants are intentionally coarse until the WS1/WS2 authority shim
/// exposes the final `CapabilityTarget` API. Keeping this as a closed enum
/// prevents the WS13 gateway from introducing stringly-typed capability names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiCapabilityTarget {
    KonsoleFocus,
    KonsoleInput,
    DisplayLayout,
    ObservabilityRead,
    GuardianAcknowledge,
}

/// Closed set of actions the remote WS13 UI may request.
///
/// This is a data contract only: no handler dispatch, no HTTP endpoint, and no
/// authorization decision is performed here. The future action gateway must
/// call [`UiAction::capability_target`] and revalidate authority per message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum UiAction {
    /// Focus a visible Konsole pane by its ACOS Konsole id.
    FocusKonsole { id: u32 },
    /// Send literal user input to a Konsole. This is not a shell command API.
    SendKonsoleInput { id: u32, text: String },
    /// Select the two Konsoles shown in the default split view.
    SetDisplaySplit { left: u32, right: u32 },
    /// Read the most recent observability events for the dashboard.
    RequestObservabilityRecent { limit: u16 },
    /// Read one trace by id for drill-down UI.
    RequestTrace { trace_id: String },
    /// Acknowledge that the human has seen a Guardian alert.
    AcknowledgeGuardianAlert { anomaly_id: String },
}

impl UiAction {
    /// Return the authority target that must be checked before dispatching the
    /// action. This match is intentionally exhaustive: adding a new action
    /// without deciding its capability target fails compilation here.
    pub fn capability_target(&self) -> UiCapabilityTarget {
        match self {
            UiAction::FocusKonsole { .. } => UiCapabilityTarget::KonsoleFocus,
            UiAction::SendKonsoleInput { .. } => UiCapabilityTarget::KonsoleInput,
            UiAction::SetDisplaySplit { .. } => UiCapabilityTarget::DisplayLayout,
            UiAction::RequestObservabilityRecent { .. } => UiCapabilityTarget::ObservabilityRead,
            UiAction::RequestTrace { .. } => UiCapabilityTarget::ObservabilityRead,
            UiAction::AcknowledgeGuardianAlert { .. } => UiCapabilityTarget::GuardianAcknowledge,
        }
    }

    /// Stable action kind string for audit/trace labels.
    pub fn kind(&self) -> &'static str {
        match self {
            UiAction::FocusKonsole { .. } => "focus_konsole",
            UiAction::SendKonsoleInput { .. } => "send_konsole_input",
            UiAction::SetDisplaySplit { .. } => "set_display_split",
            UiAction::RequestObservabilityRecent { .. } => "request_observability_recent",
            UiAction::RequestTrace { .. } => "request_trace",
            UiAction::AcknowledgeGuardianAlert { .. } => "acknowledge_guardian_alert",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{UiAction, UiCapabilityTarget};

    fn sample_actions() -> Vec<(UiAction, UiCapabilityTarget, &'static str)> {
        vec![
            (
                UiAction::FocusKonsole { id: 1 },
                UiCapabilityTarget::KonsoleFocus,
                "focus_konsole",
            ),
            (
                UiAction::SendKonsoleInput {
                    id: 1,
                    text: "help".to_string(),
                },
                UiCapabilityTarget::KonsoleInput,
                "send_konsole_input",
            ),
            (
                UiAction::SetDisplaySplit { left: 1, right: 0 },
                UiCapabilityTarget::DisplayLayout,
                "set_display_split",
            ),
            (
                UiAction::RequestObservabilityRecent { limit: 100 },
                UiCapabilityTarget::ObservabilityRead,
                "request_observability_recent",
            ),
            (
                UiAction::RequestTrace {
                    trace_id: "trace-1".to_string(),
                },
                UiCapabilityTarget::ObservabilityRead,
                "request_trace",
            ),
            (
                UiAction::AcknowledgeGuardianAlert {
                    anomaly_id: "anom-1".to_string(),
                },
                UiCapabilityTarget::GuardianAcknowledge,
                "acknowledge_guardian_alert",
            ),
        ]
    }

    #[test]
    fn ui_action_exhaustiveness_maps_every_variant_to_authority_target() {
        for (action, expected_target, expected_kind) in sample_actions() {
            assert_eq!(action.capability_target(), expected_target);
            assert_eq!(action.kind(), expected_kind);
        }
    }

    #[test]
    fn ui_action_rejects_free_form_execute_payload() {
        let payload = r#"{"type":"execute","payload":{"command":"rm -rf /"}}"#;
        let parsed = serde_json::from_str::<UiAction>(payload);
        assert!(parsed.is_err());
    }

    #[test]
    fn ui_action_round_trips_known_variant() {
        let action = UiAction::SetDisplaySplit { left: 1, right: 0 };
        let encoded = serde_json::to_string(&action).expect("serialize ui action");
        let decoded: UiAction = serde_json::from_str(&encoded).expect("deserialize ui action");
        assert_eq!(decoded, action);
    }
}
