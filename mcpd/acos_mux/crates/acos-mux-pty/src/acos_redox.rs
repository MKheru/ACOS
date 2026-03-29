//! ACOS PTY backend for the Redox target.
//!
//! On Redox OS, PTYs are allocated via the `pty:` scheme rather than
//! POSIX forkpty. This module provides a minimal implementation that
//! opens a pty: master, forks a child process, and redirects its stdio.
//!
//! Only compiled when `target_os = "redox"` and `feature = "acos"`.

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};

use crate::cmdbuilder::CommandBuilder;

// ---------------------------------------------------------------------------
// Minimal inline fpath syscall — avoids depending on the redox_syscall crate
// in a cfg-conditional context where rustc sometimes fails to resolve it.
//
// SYS_FPATH = SYS_CLASS_FILE | SYS_ARG_MSLICE | 928
//           = 0x2000_0000 | 0x0200_0000 | 928
//           = 0x2200_03A0
// syscall3(SYS_FPATH, fd, buf_ptr, buf_len) -> nbytes or error
// ---------------------------------------------------------------------------

/// Call fpath(fd, buf) using the Redox syscall ABI (x86_64: syscall instruction).
/// Returns the number of bytes written to buf, or 0 on error.
unsafe fn redox_fpath(fd: usize, buf: &mut [u8]) -> usize {
    const SYS_FPATH: usize = 0x2200_03A0;
    let mut ret: usize = SYS_FPATH;
    // Rust 2024: unsafe operations inside an unsafe fn still require an
    // explicit `unsafe` block.
    unsafe {
        core::arch::asm!(
            "syscall",
            inout("rax") ret,
            in("rdi") fd,
            in("rsi") buf.as_mut_ptr(),
            in("rdx") buf.len(),
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    // On Redox, a negative result (isize < 0) is an error.
    if (ret as isize) < 0 { 0 } else { ret }
}

// ---------------------------------------------------------------------------
// Public types (same surface as acos.rs on Unix)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }
    }
}

/// Error type for PTY operations.
#[derive(Debug)]
pub enum PtyError {
    Io(io::Error),
    InvalidCommand(String),
    Spawn(String),
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PtyError::Io(e)             => write!(f, "PTY I/O error: {e}"),
            PtyError::InvalidCommand(s) => write!(f, "PTY invalid command: {s}"),
            PtyError::Spawn(s)          => write!(f, "PTY spawn error: {s}"),
        }
    }
}

impl std::error::Error for PtyError {}

impl From<io::Error> for PtyError {
    fn from(e: io::Error) -> Self { Self::Io(e) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus(pub i32);

impl ExitStatus {
    pub fn success(self) -> bool { self.0 == 0 }
}

// ---------------------------------------------------------------------------
// AcosPty — Redox pty: scheme implementation
// ---------------------------------------------------------------------------

pub struct AcosPty {
    /// Master side of the PTY (multiplexer reads/writes here).
    master: File,
    /// PID of the child shell process.
    pid: u32,
    /// Stored size (resize is a no-op stub for now).
    _size: PtySize,
}

impl AcosPty {
    /// Allocate a new PTY and spawn the shell command.
    pub fn spawn(cmd: &CommandBuilder, size: PtySize) -> Result<Self, PtyError> {
        // Match relibc's openpty() sequence: open master via "/scheme/pty"
        // (filesystem path), not "pty:" (scheme syntax). relibc uses this exact
        // path in pty/redox.rs — the two forms may behave differently in ptyd.
        let master = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("/scheme/pty")
            .map_err(|e| PtyError::Spawn(format!("open /scheme/pty failed: {e}")))?;

        // Derive the slave path from fpath.
        // On Redox, fpath returns "/scheme/pty/N". Use that filesystem path
        // directly for the slave open — relibc's openpty() uses "/scheme/pty"
        // as the master path, so "/scheme/pty/N" is the canonical slave form.
        let slave_path = {
            use std::os::unix::io::AsRawFd;
            let fd = master.as_raw_fd();
            let mut buf = [0u8; 4096];
            let n = unsafe { redox_fpath(fd as usize, &mut buf) };
            let fpath_str = String::from_utf8_lossy(&buf[..n]).to_string();
            // fpath returns "/scheme/pty/N" — use it verbatim as the slave path.
            fpath_str
        };
        // Debug: log paths
        let _ = std::fs::write("/tmp/acos-mux-pty-debug.txt",
            format!("slave_path='{}', program='{}'\n", slave_path, cmd.program()));

        // Build argv CStrings.
        let program = CString::new(cmd.program())
            .map_err(|_| PtyError::InvalidCommand("program contains nul byte".into()))?;
        let arg_cstrings: Vec<CString> = cmd
            .args()
            .iter()
            .map(|a| {
                CString::new(a.as_str())
                    .map_err(|_| PtyError::InvalidCommand("arg contains nul byte".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let slave_cstr = CString::new(slave_path.as_str())
            .map_err(|_| PtyError::Spawn("slave path nul byte".into()))?;

        // Open the slave in the parent BEFORE fork so the master Rc<Pty> is
        // definitely alive when PtySubTerm's Weak<Pty> is upgraded in ptyd.
        // Both parent and child inherit this fd across fork; parent closes its
        // copy after fork, child uses it for stdio.
        let slave_fd = unsafe { open(slave_cstr.as_ptr() as *const u8, 2) };
        if slave_fd < 0 {
            return Err(PtyError::Spawn(format!(
                "pre-fork slave open failed: path={}",
                slave_path
            )));
        }

        // Safety: fork+exec sequence — safe to call at this point.
        let master_fd = {
            use std::os::unix::io::AsRawFd;
            master.as_raw_fd()
        };
        let child_pid = unsafe {
            redox_fork_and_exec(slave_fd, master_fd, &program, &arg_cstrings)?
        };

        // Parent: close slave fd — child has its own copy.
        unsafe { close(slave_fd) };

        Ok(Self { master, pid: child_pid, _size: size })
    }

    /// Return the raw file descriptor of the master side.
    pub fn master_raw_fd(&self) -> i32 {
        use std::os::unix::io::AsRawFd;
        self.master.as_raw_fd()
    }

    /// Resize the PTY. Stub — full resize requires Redox ioctl bindings.
    pub fn resize(&self, _size: PtySize) -> Result<(), PtyError> { Ok(()) }

    pub fn child_pid(&self) -> u32 { self.pid }

    pub fn is_alive(&self) -> bool {
        // Safety: waitpid with WNOHANG is safe to call anytime.
        unsafe {
            let mut status: i32 = 0;
            // WNOHANG = 1 on most Unix systems including Redox.
            let ret = libc_waitpid(self.pid as i32, &mut status as *mut i32, 1);
            // 0 = still running; > 0 = exited; -1 = error (no such process = dead)
            ret == 0
        }
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

// ---------------------------------------------------------------------------
// Low-level fork/exec helper (edition 2024: extern blocks must be `unsafe`)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Redox dup(fd, "pgrp") — sets process group / controlling terminal.
// On Redox, dup(fd, buf, buf_len) is a three-argument syscall where the
// extra bytes select the sub-operation. relibc exposes this as dup2 with a
// special flag, but the simplest portable path is a raw syscall.
//
// SYS_DUP = SYS_CLASS_FILE | SYS_ARG_MREF | 41
//         = 0x2000_0000 | 0x0200_0000 | 41
//         = 0x2200_0029
// syscall3(SYS_DUP, fd, buf_ptr, buf_len) -> new_fd or error
// ---------------------------------------------------------------------------

/// Call Redox dup(fd, buf) to open a virtual sub-resource of a file descriptor.
/// Used to call dup(slave_fd, "pgrp") which sets the process group / controlling terminal.
/// Returns the new fd (>= 0) on success, negative on error.
unsafe fn redox_dup_path(fd: i32, path: &[u8]) -> i32 {
    const SYS_DUP: usize = 0x2200_0029;
    let mut ret: usize = SYS_DUP;
    unsafe {
        core::arch::asm!(
            "syscall",
            inout("rax") ret,
            in("rdi") fd as usize,
            in("rsi") path.as_ptr(),
            in("rdx") path.len(),
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    ret as i32
}

// Safety: these are standard C library functions with well-known semantics.
unsafe extern "C" {
    fn fork() -> i32;
    fn setsid() -> i32;
    fn open(path: *const u8, flags: i32, ...) -> i32;
    fn dup2(oldfd: i32, newfd: i32) -> i32;
    fn close(fd: i32) -> i32;
    fn execvp(file: *const u8, argv: *const *const u8) -> i32;
    fn _exit(status: i32) -> !;
    fn waitpid(pid: i32, wstatus: *mut i32, options: i32) -> i32;
}

/// Fork a child, connect its stdio to the already-open slave PTY fd, and exec
/// the command. `slave_fd` must have been opened in the parent before calling
/// this function (guarantees the master Rc<Pty> is alive during slave open).
/// `master_fd` is the master side; the child closes it so only the parent holds
/// the master reference.
///
/// Returns the child PID on success.
///
/// # Safety
/// Caller must ensure no other threads are active (or use async-signal-safe
/// functions) in the child between fork and exec.
unsafe fn redox_fork_and_exec(
    slave_fd: i32,
    master_fd: i32,
    program: &CString,
    args: &[CString],
) -> Result<u32, PtyError> {
    let pid = unsafe { fork() };
    if pid < 0 {
        return Err(PtyError::Spawn("fork() failed".into()));
    }

    if pid == 0 {
        // ---- Child process ----
        // CRITICAL: After fork(), minimize Rust code before exec.
        // The allocator, thread-locals, and other global state may be
        // inconsistent. Only use raw syscalls and pre-allocated data.
        unsafe {
            close(master_fd);
            setsid();

            dup2(slave_fd, 0);
            dup2(slave_fd, 1);
            dup2(slave_fd, 2);

            // Set controlling terminal (Redox-specific)
            let pgrp_fd = redox_dup_path(0, b"pgrp");
            if pgrp_fd >= 0 {
                close(pgrp_fd);
            }

            if slave_fd > 2 {
                close(slave_fd);
            }

            // Build argv on the stack (no heap allocation).
            // Max 16 args should be plenty for a shell invocation.
            let mut argv_buf: [*const u8; 18] = [std::ptr::null(); 18];
            argv_buf[0] = program.as_ptr() as *const u8;
            let mut i = 1;
            for a in args {
                if i >= 17 { break; }
                argv_buf[i] = a.as_ptr() as *const u8;
                i += 1;
            }
            argv_buf[i] = std::ptr::null();

            execvp(program.as_ptr() as *const u8, argv_buf.as_ptr());
            _exit(127);
        }
    }

    // ---- Parent process ----
    Ok(pid as u32)
}

/// Thin wrapper around waitpid(2).
unsafe fn libc_waitpid(pid: i32, status: *mut i32, options: i32) -> i32 {
    unsafe { waitpid(pid, status, options) }
}
