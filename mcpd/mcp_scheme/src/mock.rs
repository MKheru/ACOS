//! Mock implementations for testing on Linux host (without Redox syscalls)
//!
//! When building with `--features host-test`, this module provides
//! simulated scheme registration so we can test the MCP logic
//! without needing the Redox kernel.

use crate::McpScheme;

/// Simulates the Redox scheme daemon main loop on a Linux host.
/// Useful for development and testing before deploying to QEMU.
pub fn run_mock_scheme() -> McpScheme {
    McpScheme::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::JsonRpcRequest;

    #[test]
    fn test_mock_full_lifecycle() {
        let mut scheme = run_mock_scheme();

        // List services
        let services = scheme.list_services();
        assert!(services.contains(&"echo"));
        assert!(services.contains(&"system"));

        // Open echo
        let id = scheme.open(b"echo").unwrap();

        // Send ping
        let req = serde_json::to_vec(&JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: "ping".into(),
            params: serde_json::Value::Null,
            id: Some(serde_json::json!(42)),
        }).unwrap();

        scheme.write(id, &req).unwrap();

        let mut buf = vec![0u8; 4096];
        let n = scheme.read(id, &mut buf).unwrap();
        let resp: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();

        assert_eq!(resp["result"], "pong");
        assert_eq!(resp["id"], 42);

        scheme.close(id).unwrap();
    }
}
