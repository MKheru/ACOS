//! Authority, capability, decision, and audit primitive types for ACOS.
//!
//! Foundation crate for the kernel authority shim (`mcpd-authority-shim`),
//! the capability fabric (`acos-system-capabilities`), and the Guardian
//! authority surface (`guardian-authority`).
//!
//! This crate has zero non-std dependencies to keep the dependency graph
//! acyclic and to allow the shim, fabric, and Guardian crates to depend on
//! it without circular references.
//!
//! ## WS1.M7 — Anomaly and AuditEvent surfaces
//!
//! Guardian anomalies remain a display surface: they are mutable operational
//! records used to show current and historical alerts to users. Authority audit
//! records remain an append-only surface: every new Guardian anomaly must also
//! emit an immutable [`AuditEvent`] so policy/audit consumers can reason about
//! the same event without depending on display-state mutation. This is a
//! wrapper, not a replacement: Guardian UI reads anomalies; authority and
//! observability readers consume audit events.

#![deny(missing_docs)]
#![deny(unsafe_code)]

mod audit;
mod caller;
mod capability;
mod decision;
mod event_kind;
mod policy;
mod scope;

pub use audit::AuditEvent;
pub use caller::CallerContext;
pub use capability::{Capability, CapabilityGrant, CapabilityId};
pub use decision::{Decision, DenyCode, Severity, Verdict};
pub use event_kind::EventKind;
pub use policy::{HandlerPolicy, Policy};
pub use scope::AuthorityScope;
