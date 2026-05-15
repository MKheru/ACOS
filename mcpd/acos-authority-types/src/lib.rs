//! Authority, capability, decision, and audit primitive types for ACOS.
//!
//! Foundation crate for the kernel authority shim (`mcpd-authority-shim`),
//! the capability fabric (`acos-system-capabilities`), and the Guardian
//! authority surface (`guardian-authority`).
//!
//! This crate has zero non-std dependencies to keep the dependency graph
//! acyclic and to allow the shim, fabric, and Guardian crates to depend on
//! it without circular references.

#![deny(missing_docs)]
#![deny(unsafe_code)]

mod audit;
mod capability;
mod decision;
mod policy;
mod scope;

pub use audit::AuditEvent;
pub use capability::{Capability, CapabilityGrant, CapabilityId};
pub use decision::{Decision, DenyCode, Severity, Verdict};
pub use policy::{HandlerPolicy, Policy};
pub use scope::AuthorityScope;
