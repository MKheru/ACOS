//! ACOS PTY backend — mirrors unix.rs but targets the ACOS `pty:` namespace.
//!
//! On a Linux/Unix development host the implementation delegates to the same
//! `nix::pty::forkpty` mechanism as `unix.rs`.  The distinct struct name
//! (`AcosPty`) and module boundary keep ACOS-specific semantics separate from
//! the generic Unix backend so that a future native `pty:` scheme open can be
//! substituted when running on bare ACOS without touching the trait surface.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};

use nix::libc;
use nix::pty::{ForkptyResult, Winsize, forkpty};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{Pid, execvp};

use crate::cmdbuilder::CommandBuilder;

// ── PtySize ──────────────────────────────────────────────────────────────────

/// PTY dimensions for the ACOS backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    /// Number of rows (lines) in the terminal.
    pub rows: u16,
    /// Number of columns (characters per line) in the terminal.
    pub cols: u16,
    /// Width of the terminal in pixels (0 if unknown).
    pub pixel_width: u16,
    /// Height of the terminal in pixels (0 if unknown).
    pub pixel_height: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl PtySize {
    fn to_winsize(self) -> Winsize {
        Winsize {
            ws_row: self.rows,
            ws_col: self.cols,
            ws_xpixel: self.pixel_width,
            ws_ypixel: self.pixel_height,
        }
    }
}

// ── PtyError ─────────────────────────────────────────────────────────────────

/// Errors that can occur when working with ACOS PTYs.
#[derive(Debug)]
pub enum PtyError {
    /// An I/O error occurred.
    Io(io::Error),
    /// A nix errno error occurred.
    Nix(nix::errno::Errno),
    /// The command or one of its arguments is invalid.
    InvalidCommand(String),
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PtyError::Io(e) => write!(f, "I/O error: {e}"),
            PtyError::Nix(e) => write!(f, "nix error: {e}"),
            PtyError::InvalidCommand(msg) => write!(f, "invalid command: {msg}"),
        }
    }
}

impl std::error::Error for PtyError {}

impl From<io::Error> for PtyError {
    fn from(e: io::Error) -> Self {
        PtyError::Io(e)
    }
}

impl From<nix::errno::Errno> for PtyError {
    fn from(e: nix::errno::Errno) -> Self {
        PtyError::Nix(e)
    }
}

// ── ExitStatus ───────────────────────────────────────────────────────────────

/// The exit status of a child process running inside an ACOS PTY.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    /// The process exited with the given code.
    Code(i32),
    /// The process was killed by the given signal.
    Signal(i32),
}

impl ExitStatus {
    /// Returns `true` if the process exited with code 0.
    pub fn success(&self) -> bool {
        matches!(self, ExitStatus::Code(0))
    }
}

impl std::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitStatus::Code(c) => write!(f, "exit code {c}"),
            ExitStatus::Signal(s) => write!(f, "signal {s}"),
        }
    }
}

// ── AcosPty ──────────────────────────────────────────────────────────────────

/// An ACOS pseudo-terminal backed by a master fd and a child process.
///
/// On a native ACOS kernel this would open `pty:` via the MCP bus; on a
/// Linux dev host it uses `forkpty` so that the full test suite runs without
/// modification.
pub struct AcosPty {
    master: File,
    child_pid: Pid,
}

impl AcosPty {
    /// Spawn a child process inside a new ACOS PTY.
    ///
    /// Uses `forkpty` on the dev host.  A future ACOS-native implementation
    /// would instead open `pty:new` and issue a `spawn` MCP request.
    pub fn spawn(cmd: &CommandBuilder, size: PtySize) -> Result<Self, PtyError> {
        let winsize = size.to_winsize();

        // Safety: forkpty is unsafe because of the fork. Only async-signal-safe
        // operations (chdir, execvp) are called in the child path.
        let result = unsafe { forkpty(&winsize, None)? };

        match result {
            ForkptyResult::Parent { child, master } => {
                let raw_fd = master.into_raw_fd();
                let master_file = unsafe { File::from_raw_fd(raw_fd) };
                Ok(AcosPty {
                    master: master_file,
                    child_pid: child,
                })
            }
            ForkptyResult::Child => {
                let child_result = (|| -> Result<(), PtyError> {
                    for (key, val) in cmd.env_map() {
                        unsafe { std::env::set_var(key, val) };
                    }

                    if let Some(cwd) = cmd.cwd_path() {
                        let _ = std::env::set_current_dir(cwd);
                    }

                    if !cmd.env_map().contains_key("TERM") {
                        unsafe { std::env::set_var("TERM", "xterm-256color") };
                    }

                    let argv = cmd.argv_cstrings()
                        .map_err(|e| PtyError::InvalidCommand(e.to_string()))?;
                    let program = cmd.program_cstr()
                        .map_err(|e| PtyError::InvalidCommand(e.to_string()))?;
                    let _ = execvp(&program, &argv);
                    Ok(())
                })();

                if child_result.is_err() {
                    unsafe { libc::_exit(127) };
                }
                unsafe { libc::_exit(127) };
            }
        }
    }

    /// Resize the PTY window.
    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        let ws = size.to_winsize();
        let fd = self.master.as_raw_fd();
        let ret = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &raw const ws) };
        if ret < 0 {
            return Err(PtyError::Io(io::Error::last_os_error()));
        }
        Ok(())
    }

    /// Return the child process PID.
    pub fn child_pid(&self) -> u32 {
        self.child_pid.as_raw() as u32
    }

    /// Wait for the child process to exit, returning its status.
    pub fn wait(&self) -> Result<ExitStatus, PtyError> {
        match waitpid(self.child_pid, None)? {
            WaitStatus::Exited(_, code) => Ok(ExitStatus::Code(code)),
            WaitStatus::Signaled(_, sig, _) => Ok(ExitStatus::Signal(sig as i32)),
            _ => Ok(ExitStatus::Code(-1)),
        }
    }

    /// Check if the child process is still alive (non-blocking).
    pub fn is_alive(&self) -> bool {
        matches!(
            waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG)),
            Ok(WaitStatus::StillAlive)
        )
    }

    /// Return a reference to the underlying master file.
    pub fn master_file(&self) -> &File {
        &self.master
    }

    /// Return the raw fd of the master side.
    pub fn master_raw_fd(&self) -> i32 {
        self.master.as_raw_fd()
    }
}

impl Read for AcosPty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.master.read(buf)
    }
}

impl Write for AcosPty {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.master.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.master.flush()
    }
}

impl Drop for AcosPty {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.child_pid.as_raw(), libc::SIGHUP);
            // Reap the zombie so we don't accumulate defunct processes.
            libc::waitpid(self.child_pid.as_raw(), std::ptr::null_mut(), 0);
        }
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_size_default() {
        let s = PtySize::default();
        assert_eq!(s.rows, 24);
        assert_eq!(s.cols, 80);
        assert_eq!(s.pixel_width, 0);
        assert_eq!(s.pixel_height, 0);
    }

    #[test]
    fn test_exit_code_success() {
        assert!(ExitStatus::Code(0).success());
    }

    #[test]
    fn test_exit_code_failure() {
        assert!(!ExitStatus::Code(1).success());
    }

    #[test]
    fn test_exit_code_display() {
        assert_eq!(format!("{}", ExitStatus::Code(42)), "exit code 42");
    }

    #[test]
    fn test_exit_signal_display() {
        assert_eq!(format!("{}", ExitStatus::Signal(9)), "signal 9");
    }

    #[test]
    fn test_ptyerror_io_display() {
        let e = PtyError::Io(io::Error::from(io::ErrorKind::NotFound));
        let s = format!("{e}");
        assert!(s.contains("I/O"), "expected 'I/O' in '{s}'");
    }

    #[test]
    fn test_ptyerror_nix_display() {
        let e = PtyError::Nix(nix::errno::Errno::EPERM);
        let s = format!("{e}");
        assert!(s.contains("nix"), "expected 'nix' in '{s}'");
    }

    #[test]
    fn test_ptyerror_invalid_command() {
        let e = PtyError::InvalidCommand("bad\0cmd".into());
        let s = format!("{e}");
        assert!(s.contains("invalid"), "expected 'invalid' in '{s}'");
    }

    #[test]
    fn test_command_builder_program() {
        let cb = CommandBuilder::new("bash");
        assert_eq!(cb.program(), "bash");
    }

    #[test]
    fn test_command_builder_default_shell() {
        let cb = CommandBuilder::default_shell();
        assert!(!cb.program().is_empty(), "default shell program must not be empty");
    }

    #[test]
    fn test_pty_size_custom() {
        let s = PtySize { rows: 50, cols: 120, pixel_width: 0, pixel_height: 0 };
        assert_eq!(s.rows, 50);
        assert_eq!(s.cols, 120);
    }

    #[test]
    fn test_exit_status_signal_not_success() {
        assert!(!ExitStatus::Signal(9).success());
    }

    #[test]
    fn test_ptyerror_from_io() {
        let io_err = io::Error::from(io::ErrorKind::PermissionDenied);
        let pty_err: PtyError = io_err.into();
        assert!(format!("{pty_err}").contains("I/O"));
    }

    #[test]
    fn test_ptyerror_from_errno() {
        let pty_err: PtyError = nix::errno::Errno::EACCES.into();
        assert!(format!("{pty_err}").contains("nix"));
    }

    #[test]
    fn test_command_builder_args() {
        let mut cb = CommandBuilder::new("echo");
        cb.arg("hello").arg("world");
        assert_eq!(cb.args().len(), 2);
    }

    #[test]
    fn test_command_builder_env() {
        let mut cb = CommandBuilder::new("sh");
        cb.env("TERM", "xterm-256color");
        assert_eq!(cb.env_map().get("TERM"), Some(&"xterm-256color".to_string()));
    }
}
