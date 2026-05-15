//! Authority decision types — outcome of a capability check.

/// Outcome of a capability check.
///
/// `Deny` carries a structured [`DenyCode`] so that handlers and observers
/// can react to denial reason without parsing free-form strings.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Verdict {
    /// The action is authorised.
    Allow,
    /// The action is refused; reason given by [`DenyCode`].
    Deny(DenyCode),
}

/// Standardised denial codes emitted by the authority shim.
///
/// `#[non_exhaustive]` permits adding new failure modes (e.g.
/// `RateLimitExceeded`) without breaking downstream `match` arms.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum DenyCode {
    /// Caller holds no capability matching the requested action.
    NoMatchingCapability,
    /// A matching capability existed but was revoked before the check.
    CapabilityRevoked,
    /// A static policy (e.g. boot policy hash) explicitly forbids the action.
    PolicyForbids,
    /// The action is attempted from the bootstrap scope, which forbids it.
    BootstrapScopeViolation,
    /// The check itself exceeded its time budget.
    TimeoutExceeded,
    /// The audit ring buffer append failed; fail-closed on audit failure.
    AuditAppendFailed,
}

/// Authority decision emitted before action dispatch.
///
/// A `Decision` is recorded in the audit ring buffer regardless of verdict;
/// dispatch happens only when `verdict == Allow`.
#[derive(Clone, Debug)]
pub struct Decision {
    /// Allow or Deny with structured reason.
    pub verdict: Verdict,
    /// Free-form explanation for logs and operators (not user-visible).
    pub reason: String,
    /// Severity for log filtering and ring-buffer retention priority.
    pub severity: Severity,
}

/// Severity for audit, log filtering, and ring-buffer retention priority.
///
/// Ordering: `Info < Warning < Critical`. Retention policies reserve
/// capacity for `Critical` events under flooding.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    /// Routine event (e.g. allowed read).
    Info,
    /// Notable event (e.g. denied write, capability mismatch).
    Warning,
    /// Security-relevant or fail-closed event (e.g. audit append failure).
    Critical,
}
