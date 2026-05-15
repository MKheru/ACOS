//! Caller identity attached to an MCP connection.

/// Identity of the kernel-level caller that opened an MCP connection.
///
/// On Redox this is derived from `redox_scheme::CallerCtx` at `openat`
/// time. On other hosts (and in unit tests) it can be constructed via
/// [`CallerContext::anonymous`] or [`CallerContext::from_parts`] so the
/// rest of the code is platform-agnostic.
///
/// **Why store this on the connection rather than the request?** The
/// kernel hands us caller identity once at handle open time; it never
/// changes for the lifetime of the handle. Storing it on the connection
/// matches that lifecycle and makes per-call lookups cheap (no syscall,
/// no map indirection).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallerContext {
    /// User id of the opening process. `u32::MAX` marks an anonymous /
    /// host-side stub identity (no Redox CallerCtx ever set this).
    pub uid: u32,
    /// Group id of the opening process. Same sentinel as `uid`.
    pub gid: u32,
    /// Process id of the opening process.
    pub pid: u32,
}

impl CallerContext {
    /// Sentinel value used by [`CallerContext::anonymous`] for fields the
    /// platform does not provide. Distinct from a legitimate `0` value
    /// (which on Redox indicates `root`).
    pub const UNKNOWN: u32 = u32::MAX;

    /// Anonymous caller — no identity attached.
    ///
    /// Used on host builds (Linux unit tests, mock mode) where no Redox
    /// `CallerCtx` exists, and as the fallback in backward-compatible
    /// `open()` entry points that do not yet thread identity.
    pub const fn anonymous() -> Self {
        Self {
            uid: Self::UNKNOWN,
            gid: Self::UNKNOWN,
            pid: Self::UNKNOWN,
        }
    }

    /// Construct a caller from raw uid/gid/pid components.
    ///
    /// Use this in tests to simulate different concurrent callers; the
    /// Redox bridge has its own `from_redox` constructor when the
    /// `redox` feature is on.
    pub const fn from_parts(uid: u32, gid: u32, pid: u32) -> Self {
        Self { uid, gid, pid }
    }

    /// `true` if this caller has no kernel-attached identity (i.e. was
    /// constructed by [`CallerContext::anonymous`]).
    pub fn is_anonymous(&self) -> bool {
        self.uid == Self::UNKNOWN && self.gid == Self::UNKNOWN && self.pid == Self::UNKNOWN
    }

    /// Compact human-readable label, used by audit events when there is
    /// no richer per-call identity available. Anonymous callers render
    /// as `"-"` for compactness.
    pub fn label(&self) -> String {
        if self.is_anonymous() {
            "-".to_string()
        } else {
            format!("uid={},gid={},pid={}", self.uid, self.gid, self.pid)
        }
    }
}

impl Default for CallerContext {
    fn default() -> Self {
        Self::anonymous()
    }
}
