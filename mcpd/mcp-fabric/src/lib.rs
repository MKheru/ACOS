//! Capability fabric primitives for MCP service authorization.
//!
//! WS2.M6 introduces this crate as the future extraction point for the MCP
//! capability graph. It does not enforce policy in `mcp-scheme` yet; it gives
//! later WS3/WS9 work a typed graph of service nodes and grants to build on.

#![deny(unsafe_code)]
#![deny(missing_docs)]

use std::collections::BTreeMap;

use acos_authority_types::{CallerContext, CapabilityId};
use acos_system_capabilities::SystemCapability;

/// A stable node in the MCP capability fabric.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct FabricNodeId(String);

impl FabricNodeId {
    /// Create a node id from a non-empty ASCII identifier.
    pub fn new(id: impl Into<String>) -> Result<Self, FabricError> {
        let id = id.into();
        if id.is_empty() {
            return Err(FabricError::InvalidNodeId);
        }
        let valid = id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        });
        if !valid {
            return Err(FabricError::InvalidNodeId);
        }
        Ok(Self(id))
    }

    /// Borrow the node id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A service node and its required system capabilities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceNode {
    id: FabricNodeId,
    capability_id: CapabilityId,
    grants: Vec<SystemCapability>,
}

impl ServiceNode {
    /// Create a service node with no grants.
    pub fn new(id: FabricNodeId, capability_id: CapabilityId) -> Self {
        Self {
            id,
            capability_id,
            grants: Vec::new(),
        }
    }

    /// Attach one system capability grant to this service node.
    pub fn with_grant(mut self, grant: SystemCapability) -> Self {
        self.grants.push(grant);
        self
    }

    /// Return the node id.
    pub fn id(&self) -> &FabricNodeId {
        &self.id
    }

    /// Return the authority capability id associated with this node.
    pub fn capability_id(&self) -> &CapabilityId {
        &self.capability_id
    }

    /// Return attached system grants.
    pub fn grants(&self) -> &[SystemCapability] {
        &self.grants
    }
}

/// Minimal deterministic graph of MCP service capability nodes.
#[derive(Clone, Debug, Default)]
pub struct CapabilityFabric {
    nodes: BTreeMap<FabricNodeId, ServiceNode>,
}

impl CapabilityFabric {
    /// Create an empty fabric graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a service node.
    pub fn insert(&mut self, node: ServiceNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    /// Return true when a node exists.
    pub fn contains(&self, id: &FabricNodeId) -> bool {
        self.nodes.contains_key(id)
    }

    /// Return a node by id.
    pub fn get(&self, id: &FabricNodeId) -> Option<&ServiceNode> {
        self.nodes.get(id)
    }

    /// Number of nodes in the fabric graph.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Return true when the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Return node ids in deterministic order.
    pub fn node_ids(&self) -> Vec<&str> {
        self.nodes.keys().map(FabricNodeId::as_str).collect()
    }
}

/// A fabric evaluation request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FabricRequest {
    /// Caller asking for access.
    pub caller: CallerContext,
    /// Target service node.
    pub node: FabricNodeId,
    /// Requested method name.
    pub method: String,
}

impl FabricRequest {
    /// Create a fabric request.
    pub fn new(caller: CallerContext, node: FabricNodeId, method: impl Into<String>) -> Self {
        Self {
            caller,
            node,
            method: method.into(),
        }
    }
}

/// Errors raised by fabric primitives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FabricError {
    /// Node id is empty or contains unsupported characters.
    InvalidNodeId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use acos_system_capabilities::{PathAccess, PathGrant};

    fn cap(id: u64) -> CapabilityId {
        CapabilityId(id)
    }

    #[test]
    fn fabric_node_ids_are_validated() {
        assert!(FabricNodeId::new("file.write").is_ok());
        assert_eq!(
            FabricNodeId::new("FileWrite"),
            Err(FabricError::InvalidNodeId)
        );
        assert_eq!(FabricNodeId::new(""), Err(FabricError::InvalidNodeId));
    }

    #[test]
    fn fabric_registers_service_node_with_grant() {
        let node_id = FabricNodeId::new("file.write").unwrap();
        let node = ServiceNode::new(node_id.clone(), cap(7)).with_grant(SystemCapability::Path(
            PathGrant::new("/tmp", PathAccess::Write),
        ));

        let mut fabric = CapabilityFabric::new();
        fabric.insert(node);

        let stored = fabric.get(&node_id).expect("node exists");
        assert_eq!(*stored.capability_id(), CapabilityId(7));
        assert_eq!(stored.grants().len(), 1);
        assert_eq!(fabric.node_ids(), vec!["file.write"]);
    }

    #[test]
    fn fabric_request_preserves_caller_and_method() {
        let caller = CallerContext::from_parts(1000, 100, 42);
        let request = FabricRequest::new(
            caller,
            FabricNodeId::new("guardian.act").unwrap(),
            "restart_service",
        );

        assert_eq!(request.node.as_str(), "guardian.act");
        assert_eq!(request.method, "restart_service");
        assert_eq!(request.caller, caller);
    }
}
