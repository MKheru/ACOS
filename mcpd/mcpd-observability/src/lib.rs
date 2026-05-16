//! ACOS mcpd observability primitives.
//!
//! WS11.M3 starts with redaction: traces must be whitelist-only and any
//! non-whitelisted parameter value is replaced by a boot-salted hash.

pub mod redact;
