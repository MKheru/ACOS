//! Authority scope — under which power a check is performed.

/// The authority scope under which a check is performed.
///
/// `GuardianBootstrap` is a constrained scope used at boot only. By
/// construction it must not be able to:
/// * modify the static policy,
/// * disable the audit ring buffer,
/// * self-grant capabilities,
/// * bypass the shim's deny-by-default wrapper.
///
/// Enforcement of these constraints lives in `mcpd-authority-shim`; this
/// crate only declares the type so the shim and the Guardian agree on the
/// scope name.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum AuthorityScope {
    /// Normal userspace mcpd request path.
    UserspaceMcpd,
    /// Kernel-level enforcement boundary (Phase 2, conditional on the
    /// upstream micro-kernel ACOS#2 issue).
    KernelBoundary,
    /// Boot-time Guardian bootstrap scope (constrained, see type doc).
    GuardianBootstrap,
}
