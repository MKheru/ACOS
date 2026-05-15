//! Policy types — handler-level default and static policy document.

/// Default policy attached to an MCP handler method.
///
/// The router consults this default *before* looking at per-caller
/// capability grants. `Public` skips the capability check entirely;
/// `RequiresCapability` triggers a grant lookup; `GuardianOnly` denies
/// everything except the `GuardianBootstrap` or `KernelBoundary` scopes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HandlerPolicy {
    /// Anyone may invoke; no capability required.
    Public,
    /// Caller must hold a matching capability.
    RequiresCapability,
    /// Reserved for the Guardian itself; denied to userspace callers.
    GuardianOnly,
}

/// A static policy document captured at boot.
///
/// Typically the hash of concatenated policy files (`HERMES.md`, `SOUL.md`,
/// `/etc/acos/policy.md`) so that the shim can refuse to start if the policy
/// surface has been tampered with between builds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Policy {
    /// SHA-256 digest of the canonical policy bytes.
    pub hash: [u8; 32],
    /// Human-readable description of the policy source (filenames, etc.).
    pub source: String,
}
