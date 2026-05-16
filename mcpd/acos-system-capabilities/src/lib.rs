//! Typed system capability grants for ACOS services.
//!
//! This crate is intentionally small and dependency-free. It defines the
//! capability vocabulary used by WS3 before enforcement is wired into the
//! router and authority shim.

#![deny(unsafe_code)]

use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};

/// A typed capability target for a system-facing service operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SystemCapability {
    /// Filesystem access scoped to a path grant.
    Path(PathGrant),
    /// Network host access scoped to host + optional resolved endpoint checks.
    Host(HostGrant),
    /// Network port access scoped to an inclusive range or single port.
    Port(PortGrant),
    /// Command execution scoped to an argv-pinned template.
    Command(CommandTemplate),
    /// Service lifecycle access scoped to a named service and action.
    Service(ServiceGrant),
}

/// Filesystem operation class covered by a [`PathGrant`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathAccess {
    /// Read-only access.
    Read,
    /// Write-only access.
    Write,
    /// Read/write access.
    ReadWrite,
}

impl PathAccess {
    fn allows(self, requested: PathAccess) -> bool {
        matches!(
            (self, requested),
            (PathAccess::ReadWrite, _)
                | (PathAccess::Read, PathAccess::Read)
                | (PathAccess::Write, PathAccess::Write)
        )
    }
}

/// A canonical-root filesystem grant.
///
/// `contains()` performs lexical normalization to reject `..` escapes. The
/// authority adapter is still expected to pass canonicalized paths once WS3
/// enforcement is wired, so this type does not re-open or trust raw JSON paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathGrant {
    root: PathBuf,
    access: PathAccess,
}

impl PathGrant {
    /// Create a path grant rooted at `root` for `access`.
    pub fn new(root: impl Into<PathBuf>, access: PathAccess) -> Self {
        Self {
            root: normalize_path(root.into()),
            access,
        }
    }

    /// Return the normalized grant root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return true when `target` is inside the grant root and access matches.
    pub fn contains(&self, target: impl AsRef<Path>, requested: PathAccess) -> bool {
        self.access.allows(requested) && normalize_path(target.as_ref()).starts_with(&self.root)
    }
}

/// Host matching strategy for [`HostGrant`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostMatch {
    /// Exact hostname match, ASCII case-insensitive.
    Exact(String),
    /// Domain suffix match. `example.org` matches `api.example.org` and itself.
    DomainSuffix(String),
}

/// A hostname grant with optional resolved-IP pinning and optional port bounds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostGrant {
    host: HostMatch,
    resolved_ips: Vec<IpAddr>,
    ports: Option<PortGrant>,
}

impl HostGrant {
    /// Create an exact-host grant.
    pub fn exact(host: impl Into<String>) -> Self {
        Self {
            host: HostMatch::Exact(normalize_host(host.into())),
            resolved_ips: Vec::new(),
            ports: None,
        }
    }

    /// Create a domain-suffix grant.
    pub fn suffix(suffix: impl Into<String>) -> Self {
        Self {
            host: HostMatch::DomainSuffix(normalize_host(suffix.into())),
            resolved_ips: Vec::new(),
            ports: None,
        }
    }

    /// Restrict this grant to one resolved endpoint IP.
    pub fn with_resolved_ip(mut self, ip: IpAddr) -> Self {
        self.resolved_ips.push(ip);
        self
    }

    /// Restrict this grant to a port grant.
    pub fn with_ports(mut self, ports: PortGrant) -> Self {
        self.ports = Some(ports);
        self
    }

    /// Return true when the requested host, resolved IP, and port are allowed.
    pub fn contains(&self, host: &str, resolved_ip: IpAddr, port: u16) -> bool {
        let requested = normalize_host(host);
        let host_ok = match &self.host {
            HostMatch::Exact(allowed) => requested == *allowed,
            HostMatch::DomainSuffix(suffix) => {
                requested == *suffix || requested.ends_with(&format!(".{suffix}"))
            }
        };
        let ip_ok = self.resolved_ips.is_empty() || self.resolved_ips.contains(&resolved_ip);
        let port_ok = self.ports.as_ref().is_none_or(|grant| grant.contains(port));
        host_ok && ip_ok && port_ok
    }
}

/// Inclusive TCP/UDP port grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortGrant {
    start: u16,
    end: u16,
}

impl PortGrant {
    /// Create a single-port grant.
    pub fn single(port: u16) -> Self {
        Self {
            start: port,
            end: port,
        }
    }

    /// Create an inclusive port-range grant.
    pub fn range(start: u16, end: u16) -> Self {
        assert!(start <= end, "port grant start must be <= end");
        Self { start, end }
    }

    /// Return true when `port` is in the grant range.
    pub fn contains(&self, port: u16) -> bool {
        self.start <= port && port <= self.end
    }
}

/// Pattern used by [`CommandTemplate`] for one argv position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArgPattern {
    /// Exact argument match.
    Exact(String),
    /// Decimal unsigned integer argument.
    UnsignedInteger,
    /// Any non-empty argument without path separators or shell metacharacters.
    Atom,
}

impl ArgPattern {
    fn matches(&self, arg: &str) -> bool {
        match self {
            ArgPattern::Exact(expected) => arg == expected,
            ArgPattern::UnsignedInteger => {
                !arg.is_empty() && arg.bytes().all(|b| b.is_ascii_digit())
            }
            ArgPattern::Atom => {
                !arg.is_empty()
                    && arg
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            }
        }
    }
}

/// Pinned command template: binary plus bounded argv schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandTemplate {
    binary: PathBuf,
    argv_schema: Vec<ArgPattern>,
}

impl CommandTemplate {
    /// Create a command template for `binary` and exact-length `argv_schema`.
    pub fn new(binary: impl Into<PathBuf>, argv_schema: Vec<ArgPattern>) -> Self {
        Self {
            binary: normalize_path(binary.into()),
            argv_schema,
        }
    }

    /// Return true when `binary` and `argv` match the pinned template exactly.
    pub fn contains(&self, binary: impl AsRef<Path>, argv: &[String]) -> bool {
        normalize_path(binary.as_ref()) == self.binary
            && argv.len() == self.argv_schema.len()
            && self
                .argv_schema
                .iter()
                .zip(argv.iter())
                .all(|(pattern, arg)| pattern.matches(arg))
    }
}

/// Service lifecycle actions that can be granted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceAction {
    /// Start a service.
    Start,
    /// Stop a service.
    Stop,
    /// Restart a service.
    Restart,
    /// Read status/log metadata without mutating service state.
    Status,
}

/// Capability grant for one service lifecycle action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceGrant {
    service: String,
    action: ServiceAction,
}

impl ServiceGrant {
    /// Create a service grant.
    pub fn new(service: impl Into<String>, action: ServiceAction) -> Self {
        Self {
            service: service.into(),
            action,
        }
    }

    /// Return true when the requested service and action match this grant.
    pub fn contains(&self, service: &str, action: ServiceAction) -> bool {
        self.service == service && self.action == action
    }
}

fn normalize_host(host: impl AsRef<str>) -> String {
    host.as_ref()
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn normalize_path(path: impl AsRef<Path>) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.as_ref().components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn path_grant_contains_child_and_rejects_escape() {
        let grant = PathGrant::new("/srv/data", PathAccess::ReadWrite);

        assert!(grant.contains("/srv/data/file.txt", PathAccess::Read));
        assert!(grant.contains("/srv/data/nested/../file.txt", PathAccess::Write));
        assert!(!grant.contains("/srv/data/../../etc/passwd", PathAccess::Read));
        assert!(!PathGrant::new("/srv/data", PathAccess::Read)
            .contains("/srv/data/file.txt", PathAccess::Write));
    }

    #[test]
    fn host_grant_contains_host_ip_and_port() {
        let allowed_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));
        let denied_ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9));
        let grant = HostGrant::suffix("example.org")
            .with_resolved_ip(allowed_ip)
            .with_ports(PortGrant::single(443));

        assert!(grant.contains("api.example.org", allowed_ip, 443));
        assert!(grant.contains("EXAMPLE.ORG.", allowed_ip, 443));
        assert!(!grant.contains("evil.test", allowed_ip, 443));
        assert!(!grant.contains("api.example.org", denied_ip, 443));
        assert!(!grant.contains("api.example.org", allowed_ip, 80));
    }

    #[test]
    fn port_grant_contains_range() {
        let grant = PortGrant::range(8000, 8080);

        assert!(grant.contains(8000));
        assert!(grant.contains(8080));
        assert!(!grant.contains(7999));
        assert!(!grant.contains(8081));
    }

    #[test]
    fn command_template_contains_pinned_argv() {
        let grant = CommandTemplate::new(
            "/usr/bin/systemctl",
            vec![
                ArgPattern::Exact("restart".into()),
                ArgPattern::Atom,
                ArgPattern::UnsignedInteger,
            ],
        );

        assert!(grant.contains(
            "/usr/bin/systemctl",
            &["restart".into(), "mcpd.service".into(), "30".into()]
        ));
        assert!(!grant.contains(
            "/usr/bin/systemctl",
            &["restart".into(), "mcpd.service".into(), "30s".into()]
        ));
        assert!(!grant.contains(
            "/bin/sh",
            &["restart".into(), "mcpd.service".into(), "30".into()]
        ));
    }

    #[test]
    fn service_grant_contains_service_action() {
        let grant = ServiceGrant::new("16_guardian", ServiceAction::Restart);

        assert!(grant.contains("16_guardian", ServiceAction::Restart));
        assert!(!grant.contains("16_guardian", ServiceAction::Stop));
        assert!(!grant.contains("mcpd", ServiceAction::Restart));
    }
}
