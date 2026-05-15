//! WS1.M5 — Boot-time policy verification.
//!
//! The daemon binary calls [`verify_from_env`] before doing anything
//! else. The helper reads three environment variables:
//!
//! * `ACOS_SKIP_POLICY_VERIFY` — if set (any value), the check is
//!   bypassed and a warning is reported. Intended for development.
//! * `ACOS_POLICY_HASH` — expected SHA-256 of the policy file as a
//!   64-character hex string. Required for verification.
//! * `ACOS_POLICY_PATH` — path to the policy bytes. Defaults to
//!   `/etc/acos/policy.md`.
//!
//! The function never panics and never exits — it returns a
//! [`BootGateOutcome`] and lets the caller decide what to do.
//! `mcpd/src/main.rs` typically exits non-zero on [`BootGateOutcome::Failed`]
//! (under `cfg(feature = "production")`) or prints a warning and
//! continues on [`BootGateOutcome::Skipped`].

use acos_authority_types::{AuthorityScope, Policy};

use crate::bootstrap::{CapabilityZeroBootstrap, CapabilityZeroError};

/// Outcome of [`verify_from_env`]. The caller decides whether to abort.
#[derive(Debug, Eq, PartialEq)]
pub enum BootGateOutcome {
    /// Policy file matched the expected hash. Safe to proceed.
    Verified,
    /// Verification was skipped because the environment did not
    /// supply the required inputs (development default). Carries the
    /// reason so the caller can log it. Production deployments must
    /// promote this to a failure.
    Skipped(&'static str),
    /// Verification was attempted but failed. Carries the reason.
    /// The caller MUST refuse to proceed.
    Failed(BootGateError),
}

/// Concrete failure reasons returned inside [`BootGateOutcome::Failed`].
#[derive(Debug, Eq, PartialEq)]
pub enum BootGateError {
    /// `ACOS_POLICY_HASH` was not a 64-character hex string.
    InvalidHashFormat,
    /// `ACOS_POLICY_PATH` could not be read (missing, permission denied, …).
    /// The string is the underlying io::Error message for diagnostics.
    PolicyFileUnreadable(String),
    /// The policy file's SHA-256 did not match `ACOS_POLICY_HASH`.
    HashMismatch,
    /// The bootstrap rejected the verification for a scope reason.
    /// In practice this should not happen since [`verify_from_env`]
    /// always uses `GuardianBootstrap`, but it is forwarded for
    /// completeness.
    BootstrapRejected,
}

/// Parse a 64-character ASCII-hex string into a 32-byte SHA-256 digest.
/// Returns [`BootGateError::InvalidHashFormat`] on any malformed input.
pub fn parse_hex_hash(hex: &str) -> Result<[u8; 32], BootGateError> {
    if hex.len() != 64 {
        return Err(BootGateError::InvalidHashFormat);
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let pair = &hex[i * 2..i * 2 + 2];
        *byte = u8::from_str_radix(pair, 16).map_err(|_| BootGateError::InvalidHashFormat)?;
    }
    Ok(out)
}

/// The default path looked up if `ACOS_POLICY_PATH` is not set.
pub const DEFAULT_POLICY_PATH: &str = "/etc/acos/policy.md";

/// Verify the on-disk policy file matches the env-pinned hash.
///
/// See module documentation for the env-var contract. The function is
/// side-effect-free apart from reading the policy file; the caller
/// owns the decision to log or exit.
pub fn verify_from_env() -> BootGateOutcome {
    // Dev override: skip everything.
    if std::env::var_os("ACOS_SKIP_POLICY_VERIFY").is_some() {
        return BootGateOutcome::Skipped("ACOS_SKIP_POLICY_VERIFY set");
    }

    // No hash configured → skip with explicit reason; production caller
    // promotes to failure.
    let expected_hex = match std::env::var("ACOS_POLICY_HASH") {
        Ok(v) if !v.is_empty() => v,
        _ => return BootGateOutcome::Skipped("ACOS_POLICY_HASH not set"),
    };

    let expected_hash = match parse_hex_hash(&expected_hex) {
        Ok(h) => h,
        Err(e) => return BootGateOutcome::Failed(e),
    };

    let policy_path = std::env::var("ACOS_POLICY_PATH")
        .unwrap_or_else(|_| DEFAULT_POLICY_PATH.to_string());
    let policy_bytes = match std::fs::read(&policy_path) {
        Ok(b) => b,
        Err(e) => {
            return BootGateOutcome::Failed(BootGateError::PolicyFileUnreadable(format!(
                "{}: {}",
                policy_path, e
            )))
        }
    };

    let policy = Policy {
        hash: expected_hash,
        source: format!("{} (hash from ACOS_POLICY_HASH)", policy_path),
    };
    let bootstrap = CapabilityZeroBootstrap::new(policy, AuthorityScope::GuardianBootstrap);
    match bootstrap.verify(&policy_bytes) {
        Ok(()) => BootGateOutcome::Verified,
        Err(CapabilityZeroError::PolicyHashMismatch) => {
            BootGateOutcome::Failed(BootGateError::HashMismatch)
        }
        Err(CapabilityZeroError::WrongScope) => {
            BootGateOutcome::Failed(BootGateError::BootstrapRejected)
        }
    }
}

/// Lower-level entry point used by the unit tests: verify with
/// explicit parameters instead of the environment. Returns the same
/// outcome enum so tests can pin exact behaviour.
pub fn verify_with(expected_hex: &str, policy_bytes: &[u8]) -> BootGateOutcome {
    let expected_hash = match parse_hex_hash(expected_hex) {
        Ok(h) => h,
        Err(e) => return BootGateOutcome::Failed(e),
    };
    let policy = Policy {
        hash: expected_hash,
        source: "verify_with (test entry)".to_string(),
    };
    let bootstrap = CapabilityZeroBootstrap::new(policy, AuthorityScope::GuardianBootstrap);
    match bootstrap.verify(policy_bytes) {
        Ok(()) => BootGateOutcome::Verified,
        Err(CapabilityZeroError::PolicyHashMismatch) => {
            BootGateOutcome::Failed(BootGateError::HashMismatch)
        }
        Err(CapabilityZeroError::WrongScope) => {
            BootGateOutcome::Failed(BootGateError::BootstrapRejected)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-256 of "abc" (FIPS test vector).
    const ABC_SHA256_HEX: &str =
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn parse_hex_hash_accepts_valid_64char_input() {
        let h = parse_hex_hash(ABC_SHA256_HEX).unwrap();
        assert_eq!(h[0], 0xba);
        assert_eq!(h[1], 0x78);
        assert_eq!(h[31], 0xad);
    }

    #[test]
    fn parse_hex_hash_rejects_short_input() {
        assert_eq!(parse_hex_hash("abc"), Err(BootGateError::InvalidHashFormat));
    }

    #[test]
    fn parse_hex_hash_rejects_non_hex_chars() {
        let bad = "g".repeat(64);
        assert_eq!(parse_hex_hash(&bad), Err(BootGateError::InvalidHashFormat));
    }

    #[test]
    fn parse_hex_hash_rejects_wrong_length_even_if_hex_valid() {
        let bad = "ab".repeat(31); // 62 chars
        assert_eq!(parse_hex_hash(&bad), Err(BootGateError::InvalidHashFormat));
    }

    #[test]
    fn verify_with_returns_verified_on_matching_bytes() {
        let outcome = verify_with(ABC_SHA256_HEX, b"abc");
        assert_eq!(outcome, BootGateOutcome::Verified);
    }

    #[test]
    fn verify_with_returns_hash_mismatch_on_wrong_bytes() {
        let outcome = verify_with(ABC_SHA256_HEX, b"abd");
        assert_eq!(outcome, BootGateOutcome::Failed(BootGateError::HashMismatch));
    }

    #[test]
    fn verify_with_returns_invalid_hash_on_malformed_hex() {
        let outcome = verify_with("not-hex", b"abc");
        assert_eq!(
            outcome,
            BootGateOutcome::Failed(BootGateError::InvalidHashFormat)
        );
    }

    #[test]
    fn verify_with_handles_empty_policy_bytes() {
        // SHA-256("") known vector.
        let empty_hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let outcome = verify_with(empty_hex, b"");
        assert_eq!(outcome, BootGateOutcome::Verified);
    }
}
