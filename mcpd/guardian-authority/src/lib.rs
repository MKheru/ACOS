//! Guardian authority crate for ACOS.
//!
//! This crate is the Guardian-side authority surface introduced by WS9.M4.
//! It intentionally stays dependency-light and does not call into `mcp-scheme`:
//! consumers pass typed action requests in and receive authority decisions,
//! audit observations, or signed decision records out.

#![deny(unsafe_code)]
#![deny(missing_docs)]

pub mod action_bridge;
pub mod audit_reader;
pub mod decision_emitter;
pub mod policy_engine;

pub use action_bridge::{ActionBridge, ActionRequest, BridgeError};
pub use audit_reader::{AuditReader, AuditSnapshot};
pub use decision_emitter::{DecisionEmitter, GuardianKey, SignedDecision};
pub use policy_engine::{InlineGuardianPolicy, PolicyEngine, ReactiveGuardianObserver};
