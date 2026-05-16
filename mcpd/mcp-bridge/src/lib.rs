//! Boot-safe MCP bridge primitives.
//!
//! `mcp-bridge` is the extraction point for MCP handle/session plumbing that
//! must stay independent from service handlers. WS2.M6 keeps this crate small:
//! it defines typed session and service identifiers plus a bounded registry used
//! by future bridge code, without moving the existing 19 service handlers yet.

#![deny(unsafe_code)]
#![deny(missing_docs)]

use std::collections::BTreeMap;
use std::fmt;

/// Stable identifier for an open MCP bridge session.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct BridgeSessionId(u64);

impl BridgeSessionId {
    /// Create a non-zero session id.
    pub fn new(raw: u64) -> Option<Self> {
        (raw != 0).then_some(Self(raw))
    }

    /// Return the raw numeric id.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Canonical MCP service name accepted by the bridge registry.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct ServiceName(String);

impl ServiceName {
    /// Validate and store a service name.
    ///
    /// Names are intentionally narrow: ASCII lowercase, digits, `_`, and `-`.
    /// This matches the current built-in services (`file_write`, `file_search`,
    /// etc.) and avoids URL/path ambiguity when the bridge later owns parsing.
    pub fn new(name: impl Into<String>) -> Result<Self, BridgeError> {
        let name = name.into();
        if name.is_empty() {
            return Err(BridgeError::InvalidServiceName);
        }
        let valid = name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'));
        if !valid {
            return Err(BridgeError::InvalidServiceName);
        }
        Ok(Self(name))
    }

    /// Borrow the service name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Metadata tracked by the bridge for one registered service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceDescriptor {
    name: ServiceName,
    /// Whether the service is expected to be reachable through `open()`.
    pub openable: bool,
}

impl ServiceDescriptor {
    /// Create a descriptor for an openable service.
    pub fn openable(name: ServiceName) -> Self {
        Self {
            name,
            openable: true,
        }
    }

    /// Return the descriptor's service name.
    pub fn name(&self) -> &ServiceName {
        &self.name
    }
}

/// Deterministic service registry used by bridge tests and future extraction.
#[derive(Clone, Debug, Default)]
pub struct BridgeRegistry {
    services: BTreeMap<ServiceName, ServiceDescriptor>,
}

impl BridgeRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a service descriptor, replacing an older descriptor with the
    /// same service name.
    pub fn register(&mut self, descriptor: ServiceDescriptor) {
        self.services.insert(descriptor.name.clone(), descriptor);
    }

    /// Return true when a service exists in the registry.
    pub fn has_service(&self, name: &str) -> bool {
        ServiceName::new(name.to_owned())
            .ok()
            .is_some_and(|name| self.services.contains_key(&name))
    }

    /// Return the number of registered services.
    pub fn len(&self) -> usize {
        self.services.len()
    }

    /// Return true when no service is registered.
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    /// Return registered service names in deterministic order.
    pub fn service_names(&self) -> Vec<&str> {
        self.services.keys().map(ServiceName::as_str).collect()
    }
}

/// Errors raised by bridge primitives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgeError {
    /// A service name is empty or contains unsupported characters.
    InvalidServiceName,
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILTIN_SERVICES: [&str; 19] = [
        "system",
        "process",
        "memory",
        "file",
        "file_write",
        "file_search",
        "log",
        "config",
        "echo",
        "mcp",
        "command",
        "service",
        "net",
        "konsole",
        "display",
        "ai",
        "talk",
        "guardian",
        "llm",
    ];

    #[test]
    fn bridge_session_id_rejects_zero() {
        assert_eq!(BridgeSessionId::new(0), None);
        assert_eq!(BridgeSessionId::new(7).unwrap().get(), 7);
    }

    #[test]
    fn registry_tracks_all_current_mcp_services() {
        let mut registry = BridgeRegistry::new();
        for service in BUILTIN_SERVICES {
            let name = ServiceName::new(service).unwrap();
            registry.register(ServiceDescriptor::openable(name));
        }

        assert_eq!(registry.len(), 19);
        assert!(registry.has_service("guardian"));
        assert!(registry.has_service("file_write"));
        assert!(!registry.has_service("missing"));
    }

    #[test]
    fn service_name_rejects_url_ambiguous_input() {
        assert_eq!(
            ServiceName::new("../file"),
            Err(BridgeError::InvalidServiceName)
        );
        assert_eq!(
            ServiceName::new("File"),
            Err(BridgeError::InvalidServiceName)
        );
        assert_eq!(ServiceName::new(""), Err(BridgeError::InvalidServiceName));
    }
}
