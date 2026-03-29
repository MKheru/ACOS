//! Redox SchemeSync bridge for McpScheme
//!
//! Adapts McpScheme's API to the redox_scheme::SchemeSync trait,
//! following the pattern used by randd and other Redox scheme daemons.

#[cfg(any(target_os = "redox", feature = "redox"))]
mod inner {
    use std::collections::HashMap;
    use redox_scheme::{
        scheme::SchemeSync,
        CallerCtx, OpenResult,
    };
    use syscall::schemev2::NewFdFlags;
    use syscall::data::Stat;
    use syscall::flag::MODE_FILE;
    use syscall::{Error, Result, EACCES, EBADF, ENOENT};
    use crate::McpScheme;

    /// Sentinel handle ID used for the scheme root (returned by scheme_root()).
    /// McpScheme assigns real handles starting at 1, so 0 is safe as a sentinel.
    const ROOT_HANDLE: usize = 0;

    /// Bridge that implements SchemeSync by delegating to McpScheme.
    ///
    /// Stores the path string per open handle so that fpath() can reconstruct
    /// the full "mcp:{path}" string without re-parsing.
    pub struct McpSchemeBridge {
        inner: McpScheme,
        /// Maps Redox handle id (usize) → original path string
        paths: HashMap<usize, String>,
    }

    impl McpSchemeBridge {
        pub fn new() -> Self {
            McpSchemeBridge {
                inner: McpScheme::new(),
                paths: HashMap::new(),
            }
        }

        /// True if `id` is the root handle returned by scheme_root().
        fn is_root(&self, id: usize) -> bool {
            id == ROOT_HANDLE
        }
    }

    /// Convert our negative-errno i32 errors into syscall::Error.
    fn map_err(e: i32) -> Error {
        // Our errors are stored as negative libc constants (e.g. -libc::ENOENT).
        // Redox syscall::Error expects a positive errno value.
        let errno = if e < 0 { -e } else { e };
        Error::new(errno)
    }

    impl SchemeSync for McpSchemeBridge {
        /// Called once to create the scheme root handle.
        /// Returns a handle ID that callers will pass to openat() as `dirfd`.
        fn scheme_root(&mut self) -> Result<usize> {
            Ok(ROOT_HANDLE)
        }

        // NOTE: CallerCtx (UID/GID/PID) intentionally not checked.
        // The mcp: scheme is world-accessible by design, like rand: and null:.
        // Access control is handled at the MCP service layer, not the scheme layer.
        fn openat(
            &mut self,
            dirfd: usize,
            path: &str,
            _flags: usize,
            _fcntl_flags: u32,
            _ctx: &CallerCtx,
        ) -> Result<OpenResult> {
            if !self.is_root(dirfd) {
                return Err(Error::new(EACCES));
            }

            let path_bytes = path.as_bytes();
            match self.inner.open(path_bytes) {
                Ok(id) => {
                    self.paths.insert(id, path.to_string());
                    Ok(OpenResult::ThisScheme {
                        number: id,
                        flags: NewFdFlags::empty(),
                    })
                }
                Err(e) => Err(map_err(e)),
            }
        }

        fn read(
            &mut self,
            id: usize,
            buf: &mut [u8],
            _offset: u64,
            _fcntl_flags: u32,
            _ctx: &CallerCtx,
        ) -> Result<usize> {
            self.inner.read(id, buf).map_err(map_err)
        }

        fn write(
            &mut self,
            id: usize,
            buf: &[u8],
            _offset: u64,
            _fcntl_flags: u32,
            _ctx: &CallerCtx,
        ) -> Result<usize> {
            self.inner.write(id, buf).map_err(map_err)
        }

        fn fpath(&mut self, id: usize, buf: &mut [u8], _ctx: &CallerCtx) -> Result<usize> {
            let stored = self.paths.get(&id).ok_or(Error::new(EBADF))?;
            let full = format!("mcp:{}", stored);
            let bytes = full.as_bytes();
            let len = core::cmp::min(buf.len(), bytes.len());
            buf[..len].copy_from_slice(&bytes[..len]);
            Ok(len)
        }

        fn fstat(&mut self, id: usize, stat: &mut Stat, _ctx: &CallerCtx) -> Result<()> {
            if !self.paths.contains_key(&id) && !self.is_root(id) {
                return Err(Error::new(EBADF));
            }
            stat.st_mode = MODE_FILE as u16;
            Ok(())
        }

        fn on_close(&mut self, id: usize) {
            if !self.is_root(id) {
                let _ = self.inner.close(id);
                self.paths.remove(&id);
            }
        }
    }
}

#[cfg(any(target_os = "redox", feature = "redox"))]
pub use inner::McpSchemeBridge;
