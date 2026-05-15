//! Deny-by-default authority shim + append-only audit ring buffer.
//!
//! The shim is the single point where `mcpd` answers the question
//! "*is this caller allowed to perform this action?*" before the action
//! reaches a handler. Three concrete contracts live here:
//!
//! 1. **[`AuditRing`]** — bounded, append-only buffer of [`AuditEvent`]s.
//!    Survives a failing append by returning `Err`; callers must treat
//!    that as a fail-closed signal (refuse the action) so an attacker who
//!    saturates the ring cannot "go silent" by exhausting capacity.
//!
//! 2. **[`CapabilityZeroBootstrap`]** — the constrained scope used at boot
//!    to plant the first capabilities. Carries a [`Policy`] hash so the
//!    shim can refuse to start if the on-disk policy file has been
//!    tampered with between builds (see [`CapabilityZeroBootstrap::verify`]).
//!
//! 3. **[`AuthorityShim`]** — orchestrates the above. Today only the audit
//!    ring is wired; the policy check itself stays in the router's
//!    `required_policy()` path (WS1.M4) until [`CallerContext`] threading
//!    lands (WS2.M3). The shim is intentionally usable as a *no-op log
//!    sink* in this transitional state so it can be wired into mcpd now
//!    and gain teeth incrementally.
//!
//! The crate has no dependency on the wider `mcp-scheme` crate to keep
//! the dependency graph acyclic (`mcp-scheme` → `mcpd-authority-shim` →
//! `acos-authority-types`).

#![deny(missing_docs)]
#![deny(unsafe_code)]

mod audit_ring;
mod boot_gate;
mod bootstrap;
mod policy;
mod shim;

pub use audit_ring::{AuditRing, AuditRingError};
pub use boot_gate::{
    parse_hex_hash, verify_from_env, verify_with, BootGateError, BootGateOutcome,
    DEFAULT_POLICY_PATH,
};
pub use bootstrap::{CapabilityZeroBootstrap, CapabilityZeroError};
pub use policy::CapabilityPolicy;
pub use shim::AuthorityShim;
