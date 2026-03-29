//! ACOS IPC socket path conventions and utilities.

use std::path::PathBuf;
use crate::transport::Transport;

/// ACOS runtime directory where session sockets are stored.
/// Follows XDG base directory spec: `/run/acos-mux`
pub const ACOS_RUNTIME_DIR: &str = "/run/acos-mux";

/// Validates a session name contains only safe characters: `[a-zA-Z0-9_-]`.
///
/// Returns `Err` if the name is empty or contains any character outside that set,
/// preventing path traversal via `..`, `/`, null bytes, or other special sequences.
pub fn validate_session_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("session name must not be empty".to_string());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!(
            "session name {:?} contains invalid characters; only [a-zA-Z0-9_-] are allowed",
            name
        ));
    }
    Ok(())
}

/// Constructs the socket path for a given ACOS session.
///
/// Returns `{ACOS_RUNTIME_DIR}/{session}.sock`
/// Example: `acos_socket_path("main")` → `/run/acos-mux/main.sock`
///
/// # Panics
/// Panics if `session` contains characters outside `[a-zA-Z0-9_-]` — this is a
/// programmer error; callers must validate session names before constructing paths.
pub fn acos_socket_path(session: &str) -> PathBuf {
    validate_session_name(session)
        .unwrap_or_else(|e| panic!("acos_socket_path: {e}"));
    PathBuf::from(ACOS_RUNTIME_DIR).join(format!("{session}.sock"))
}

/// Constructs a local Transport connected to an ACOS session socket.
///
/// This is a convenience function that wraps `acos_socket_path` with
/// `Transport::Local` to create a ready-to-use transport.
pub fn acos_transport(session: &str) -> Transport {
    Transport::Local(acos_socket_path(session))
}

/// A named ACOS session with path and transport management.
#[derive(Debug, Clone)]
pub struct AcosSession {
    name: String,
}

impl AcosSession {
    /// Create a new ACOS session with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        AcosSession { name: name.into() }
    }

    /// Get the session name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the socket path for this session.
    pub fn socket_path(&self) -> PathBuf {
        acos_socket_path(&self.name)
    }

    /// Get the transport for this session.
    pub fn transport(&self) -> Transport {
        acos_transport(&self.name)
    }

    /// Connect to this session's IPC endpoint.
    pub fn connect(&self) -> Result<Box<dyn crate::transport::ReadWrite>, crate::transport::TransportError> {
        self.transport().connect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acos_runtime_dir_constant() {
        assert_eq!(ACOS_RUNTIME_DIR, "/run/acos-mux");
    }

    #[test]
    fn socket_path_basic() {
        let path = acos_socket_path("main");
        assert_eq!(path, PathBuf::from("/run/acos-mux/main.sock"));
    }

    #[test]
    fn socket_path_extension() {
        let path = acos_socket_path("dev");
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(filename, "dev.sock");
    }

    #[test]
    fn socket_path_different_sessions() {
        let path1 = acos_socket_path("session1");
        let path2 = acos_socket_path("session2");
        assert_ne!(path1, path2);
        assert_eq!(path1, PathBuf::from("/run/acos-mux/session1.sock"));
        assert_eq!(path2, PathBuf::from("/run/acos-mux/session2.sock"));
    }

    #[test]
    fn socket_path_contains_runtime_dir() {
        let path = acos_socket_path("test");
        assert!(path.starts_with(ACOS_RUNTIME_DIR));
    }

    #[test]
    fn socket_path_session_in_filename() {
        let session = "mysession";
        let path = acos_socket_path(session);
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(filename.contains(session));
    }

    #[test]
    fn acos_transport_is_local() {
        let transport = acos_transport("main");
        assert!(transport.is_local());
        assert!(!transport.is_ssh());
    }

    #[test]
    fn acos_transport_socket_path_matches() {
        let session = "work";
        let transport = acos_transport(session);
        let expected = acos_socket_path(session);
        assert_eq!(transport.socket_path(), &expected);
    }

    #[test]
    fn socket_path_with_special_chars() {
        let path = acos_socket_path("my-session_v1");
        assert_eq!(path, PathBuf::from("/run/acos-mux/my-session_v1.sock"));
    }

    // F1: path traversal prevention tests
    #[test]
    fn validate_session_name_accepts_valid() {
        assert!(validate_session_name("main").is_ok());
        assert!(validate_session_name("my-session_v1").is_ok());
        assert!(validate_session_name("ABC123").is_ok());
        assert!(validate_session_name("a").is_ok());
    }

    #[test]
    fn validate_session_name_rejects_path_traversal() {
        assert!(validate_session_name("../etc/evil").is_err());
        assert!(validate_session_name("../../root").is_err());
        assert!(validate_session_name("foo/bar").is_err());
        assert!(validate_session_name("foo\0bar").is_err());
        assert!(validate_session_name(".hidden").is_err());
        assert!(validate_session_name("").is_err());
    }

    #[test]
    #[should_panic(expected = "acos_socket_path")]
    fn socket_path_panics_on_path_traversal() {
        acos_socket_path("../etc/evil");
    }

    #[test]
    #[should_panic(expected = "acos_socket_path")]
    fn socket_path_panics_on_slash() {
        acos_socket_path("foo/bar");
    }

    #[test]
    fn multiple_transport_instances_are_equal() {
        let t1 = acos_transport("session");
        let t2 = acos_transport("session");
        assert_eq!(t1, t2);
    }

    #[test]
    fn acos_session_new_and_name() {
        let session = AcosSession::new("primary");
        assert_eq!(session.name(), "primary");
    }

    #[test]
    fn acos_session_socket_path() {
        let session = AcosSession::new("work");
        let expected = acos_socket_path("work");
        assert_eq!(session.socket_path(), expected);
    }

    #[test]
    fn acos_session_transport() {
        let session = AcosSession::new("test");
        let transport = session.transport();
        assert!(transport.is_local());
        let expected = acos_transport("test");
        assert_eq!(transport, expected);
    }

    #[test]
    fn acos_session_different_names_different_paths() {
        let session1 = AcosSession::new("alpha");
        let session2 = AcosSession::new("beta");
        assert_ne!(session1.socket_path(), session2.socket_path());
    }

    #[test]
    fn acos_session_clone() {
        let session1 = AcosSession::new("original");
        let session2 = session1.clone();
        assert_eq!(session1.name(), session2.name());
        assert_eq!(session1.socket_path(), session2.socket_path());
    }

    #[test]
    fn acos_session_into_string() {
        let session = AcosSession::new("myname".to_string());
        assert_eq!(session.name(), "myname");
    }
}
