//! Capability types: what an action holder may do.

use std::time::SystemTime;

/// Stable, opaque identifier for an issued [`CapabilityGrant`].
///
/// Identifiers are produced by the authority shim at grant time and remain
/// valid until the grant is revoked. The numeric value is implementation-
/// defined and must not be relied upon for ordering or hashing semantics
/// beyond equality.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CapabilityId(pub u64);

/// A capability authorising an action against a target resource.
///
/// Variants are intentionally narrow and target-typed. `#[non_exhaustive]`
/// keeps the enum open for future kernel-level capabilities without breaking
/// downstream `match` arms.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Capability {
    /// Invoke a specific MCP method on a specific service.
    InvokeMethod {
        /// MCP service name (e.g. `"file"`, `"net"`).
        service: String,
        /// Method on that service (e.g. `"read"`, `"http_get"`).
        method: String,
    },
    /// Read files whose canonical path starts with `path_prefix`.
    FileRead {
        /// Canonical path prefix the holder may read under.
        path_prefix: String,
    },
    /// Write files whose canonical path starts with `path_prefix`.
    FileWrite {
        /// Canonical path prefix the holder may write under.
        path_prefix: String,
    },
    /// Open a TCP connection to the given host:port tuple.
    NetTcpConnect {
        /// Hostname or IP literal authorised for connect.
        host: String,
        /// TCP port number.
        port: u16,
    },
    /// Read-only inspection (no side effects). Used for the boot Guardian
    /// scope; cannot be combined with mutating actions.
    ReadOnly,
}

/// A grant tying a [`Capability`] to a holder at a point in time.
///
/// Grants are issued by the authority shim. They are passed by reference to
/// handlers and are immutable; revocation is modelled by removing the grant
/// from the shim's registry, not by mutating the grant itself.
#[derive(Clone, Debug)]
pub struct CapabilityGrant {
    /// Stable identifier for this grant.
    pub id: CapabilityId,
    /// The capability authorised by this grant.
    pub capability: Capability,
    /// Holder name (typically a session id or service name).
    pub holder: String,
    /// When the grant was issued.
    pub granted_at: SystemTime,
}
