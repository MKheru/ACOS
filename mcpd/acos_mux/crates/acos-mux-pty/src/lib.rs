//! Pseudo-terminal abstraction layer (unix, windows, and ACOS).

// On Redox, cfg(unix) is also true (Redox implements POSIX), but nix::pty
// is excluded. Gate unix.rs to non-Redox only.
#[cfg(all(unix, not(target_os = "redox")))]
pub mod unix;

#[cfg(windows)]
pub mod windows;

// On non-Redox Unix hosts the `acos` feature uses the nix/forkpty backend.
// On Redox the `acos` feature uses the native pty: scheme backend.
#[cfg(all(feature = "acos", not(target_os = "redox")))]
pub mod acos;

#[cfg(all(feature = "acos", target_os = "redox"))]
pub mod acos_redox;

pub mod cmdbuilder;

pub use cmdbuilder::CommandBuilder;

#[cfg(all(unix, not(target_os = "redox"), not(feature = "acos")))]
pub use unix::{ExitStatus, PtyError, PtySize, UnixPty};

#[cfg(windows)]
pub use windows::{ExitStatus, PtyError, PtySize, WinPty};

#[cfg(all(feature = "acos", not(target_os = "redox")))]
pub use acos::{AcosPty, ExitStatus, PtyError, PtySize};

#[cfg(all(feature = "acos", target_os = "redox"))]
pub use acos_redox::{AcosPty, ExitStatus, PtyError, PtySize};

/// Platform-specific PTY type alias.
#[cfg(all(unix, not(target_os = "redox"), not(feature = "acos")))]
pub type NativePty = UnixPty;

/// Platform-specific PTY type alias.
#[cfg(windows)]
pub type NativePty = WinPty;

/// Platform-specific PTY type alias.
#[cfg(feature = "acos")]
pub type NativePty = AcosPty;

use std::io::{Read, Write};

/// Trait for interacting with a pseudo-terminal.
pub trait Pty: Read + Write {
    /// Resize the PTY to the given dimensions.
    fn resize(&self, size: PtySize) -> Result<(), PtyError>;

    /// Return the PID of the child process.
    fn child_pid(&self) -> u32;

    /// Check whether the child process is still running.
    fn is_alive(&self) -> bool;
}

#[cfg(all(unix, not(target_os = "redox"), not(feature = "acos")))]
impl Pty for UnixPty {
    fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        self.resize(size)
    }

    fn child_pid(&self) -> u32 {
        self.child_pid()
    }

    fn is_alive(&self) -> bool {
        self.is_alive()
    }
}

#[cfg(windows)]
impl Pty for WinPty {
    fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        self.resize(size)
    }

    fn child_pid(&self) -> u32 {
        self.child_pid()
    }

    fn is_alive(&self) -> bool {
        self.is_alive()
    }
}

#[cfg(feature = "acos")]
impl Pty for AcosPty {
    fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        self.resize(size)
    }

    fn child_pid(&self) -> u32 {
        self.child_pid()
    }

    fn is_alive(&self) -> bool {
        self.is_alive()
    }
}
