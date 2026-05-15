//! File service handlers: read, write, search

use std::io::{BufRead, ErrorKind, Read};
use std::path::{Component, Path, PathBuf};

use serde_json::json;

use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use crate::McpPath;

/// Maximum file size allowed for read (10 MiB)
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum recursion depth for directory search
const MAX_SEARCH_DEPTH: usize = 16;

/// Maximum directories visited during search
const MAX_DIRS_VISITED: usize = 200;

#[cfg(not(target_os = "redox"))]
const ALLOWED_ROOT: &str = "/tmp";

#[cfg(target_os = "redox")]
const ALLOWED_ROOT: &str = "/";

fn validate_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("path must not be empty");
    }
    let p = Path::new(path);
    // Check components for '..' (prevents traversal via component, not substring)
    for component in p.components() {
        if component == Component::ParentDir {
            return Err("path traversal with '..' is not allowed");
        }
    }
    // Reject absolute paths outside the allowed root
    if p.is_absolute() && !p.starts_with(ALLOWED_ROOT) {
        return Err("path is outside allowed root");
    }
    Ok(())
}

/// WS3.M5 — Resolve `path` to its canonical form (following symlinks) and
/// confirm the resolved path stays within [`ALLOWED_ROOT`].
///
/// This is the core defence against path-TOCTOU and symlink-escape:
/// `validate_path` only inspects the *string*, so a symlink whose name lives
/// in `/tmp` but whose target is `/etc/shadow` would slip through. Resolving
/// the path *before* opening forces the symlink to be evaluated once, here.
///
/// Returns the canonical `PathBuf` on success. Callers must use this
/// returned path for every subsequent filesystem operation (never the raw
/// request string) so a later swap of the symlink cannot redirect the
/// open/read pair.
fn resolve_safe_path(path: &str) -> Result<PathBuf, String> {
    validate_path(path).map_err(String::from)?;

    let canonical = std::fs::canonicalize(path)
        .map_err(|e| format!("cannot resolve path: {}", e))?;

    if !canonical.starts_with(ALLOWED_ROOT) {
        return Err(format!(
            "resolved path '{}' escapes allowed root '{}'",
            canonical.display(),
            ALLOWED_ROOT
        ));
    }
    Ok(canonical)
}

/// WS3.M5 (write variant) — Resolve a writable target.
///
/// The file may not yet exist, so canonicalising the path itself would fail.
/// Instead we canonicalise the *parent directory*, validate it stays within
/// [`ALLOWED_ROOT`], then refuse to follow a symlink that already lives at
/// the target name (a writer expecting a regular file should not silently
/// clobber whatever a pre-existing symlink points to).
///
/// Returns `(parent_canonical, basename)` — caller writes to
/// `parent_canonical.join(basename)`.
fn resolve_safe_write_target(path: &str) -> Result<PathBuf, String> {
    validate_path(path).map_err(String::from)?;

    let p = Path::new(path);
    let parent = p.parent().ok_or("path has no parent directory".to_string())?;
    let basename = p
        .file_name()
        .ok_or("path has no file name component".to_string())?;

    // If parent is empty (relative bare filename), treat it as ALLOWED_ROOT.
    let parent = if parent.as_os_str().is_empty() {
        Path::new(ALLOWED_ROOT)
    } else {
        parent
    };

    let canonical_parent = std::fs::canonicalize(parent)
        .map_err(|e| format!("cannot resolve parent directory: {}", e))?;

    if !canonical_parent.starts_with(ALLOWED_ROOT) {
        return Err(format!(
            "resolved parent '{}' escapes allowed root '{}'",
            canonical_parent.display(),
            ALLOWED_ROOT
        ));
    }

    let target = canonical_parent.join(basename);

    // Refuse to follow an existing symlink at the target name.
    // `symlink_metadata` does NOT traverse the final symlink, unlike `metadata`.
    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            return Err(format!(
                "refusing to write through symlink at '{}'",
                target.display()
            ));
        }
    }

    Ok(target)
}

// ---------------------------------------------------------------------------
// FileReadHandler
// ---------------------------------------------------------------------------

pub struct FileReadHandler;

impl FileReadHandler {
    pub fn new() -> Self {
        FileReadHandler
    }
}

impl ServiceHandler for FileReadHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "read" => {
                let file_path = match request.params.get("path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required param: path",
                        )
                    }
                };

                // WS3.M5 — Resolve symlinks once, then use the canonical
                // path for every following operation. Closes the symlink-
                // escape vector that `validate_path` alone could not catch.
                let canonical = match resolve_safe_path(file_path) {
                    Ok(p) => p,
                    Err(e) => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            e,
                        );
                    }
                };

                // WS3.M5 — Open exactly ONCE on the canonical path. From
                // here on we operate on the FD, so a post-open symlink
                // swap cannot redirect us.
                let mut file = match std::fs::File::open(&canonical) {
                    Ok(f) => f,
                    Err(e) => {
                        let code = if e.kind() == ErrorKind::NotFound {
                            INVALID_PARAMS
                        } else {
                            INTERNAL_ERROR
                        };
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            code,
                            format!("file operation failed: {}", e),
                        );
                    }
                };

                // Size check via fstat on the open FD (not a second path-
                // resolution pass).
                let meta = match file.metadata() {
                    Ok(m) => m,
                    Err(e) => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INTERNAL_ERROR,
                            format!("file operation failed: {}", e),
                        );
                    }
                };
                if meta.len() > MAX_FILE_SIZE {
                    return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        "file too large (max 10 MiB)",
                    );
                }

                // Read from the same FD with a hard cap as defence in
                // depth — even if the file grew between fstat and read,
                // `take` will truncate at `MAX_FILE_SIZE`.
                let mut content = String::new();
                match (&mut file).take(MAX_FILE_SIZE).read_to_string(&mut content) {
                    Ok(_) => {
                        let size = content.len();
                        return JsonRpcResponse::success(
                            request.id.clone(),
                            json!({ "content": content, "size": size }),
                        );
                    }
                    Err(e) => {
                        let code = if e.kind() == ErrorKind::NotFound {
                            INVALID_PARAMS
                        } else {
                            INTERNAL_ERROR
                        };
                        JsonRpcResponse::error(
                            request.id.clone(),
                            code,
                            format!("file operation failed: {}", e),
                        )
                    }
                }
            }
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in file service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["read"]
    }
}

// ---------------------------------------------------------------------------
// FileWriteHandler
// ---------------------------------------------------------------------------

pub struct FileWriteHandler;

impl FileWriteHandler {
    pub fn new() -> Self {
        FileWriteHandler
    }
}

impl ServiceHandler for FileWriteHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "write" => {
                let file_path = match request.params.get("path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required param: path",
                        )
                    }
                };
                let content = match request.params.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required param: content",
                        )
                    }
                };

                // WS3.M5 — Canonicalise the parent directory, refuse to
                // clobber a pre-existing symlink at the target name.
                let target = match resolve_safe_write_target(file_path) {
                    Ok(t) => t,
                    Err(e) => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            e,
                        );
                    }
                };

                let bytes = content.len();
                match std::fs::write(&target, content) {
                    Ok(_) => JsonRpcResponse::success(
                        request.id.clone(),
                        json!({ "bytes_written": bytes, "path": target.display().to_string() }),
                    ),
                    // F7: Generic error message
                    Err(e) => JsonRpcResponse::error(
                        request.id.clone(),
                        INTERNAL_ERROR,
                        format!("file operation failed: {}", e),
                    ),
                }
            }
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in file service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["write"]
    }
}

// ---------------------------------------------------------------------------
// FileSearchHandler
// ---------------------------------------------------------------------------

pub struct FileSearchHandler;

impl FileSearchHandler {
    pub fn new() -> Self {
        FileSearchHandler
    }
}

fn search_dir(
    dir: &Path,
    pattern: &str,
    files_scanned: &mut usize,
    matches: &mut Vec<serde_json::Value>,
    depth: usize,
    dirs_visited: &mut usize,
) {
    // F3: Enforce depth and dirs-visited limits
    if depth > MAX_SEARCH_DEPTH || *dirs_visited >= MAX_DIRS_VISITED {
        return;
    }
    if *files_scanned >= 100 || matches.len() >= 1000 {
        return;
    }
    *dirs_visited += 1;

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if *files_scanned >= 100 || matches.len() >= 1000 {
            break;
        }
        let path = entry.path();

        // F3: Use symlink_metadata to detect and skip symlinks
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }

        if meta.is_dir() {
            search_dir(&path, pattern, files_scanned, matches, depth + 1, dirs_visited);
        } else if meta.is_file() {
            *files_scanned += 1;
            if let Ok(file) = std::fs::File::open(&path) {
                let reader = std::io::BufReader::new(file);
                for (idx, line_result) in reader.lines().enumerate() {
                    if matches.len() >= 1000 {
                        break;
                    }
                    if let Ok(line) = line_result {
                        if line.contains(pattern) {
                            matches.push(json!({
                                "file": path.to_string_lossy(),
                                "line": idx + 1,
                                "content": line,
                            }));
                        }
                    }
                }
            }
        }
    }
}

impl ServiceHandler for FileSearchHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "search" => {
                let pattern = match request.params.get("pattern").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required param: pattern",
                        )
                    }
                };
                let search_path = match request.params.get("path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "missing required param: path",
                        )
                    }
                };
                // WS3.M5 (search variant) — canonicalise the search root so a
                // symlink at the entry point cannot redirect the traversal
                // outside ALLOWED_ROOT. The in-traversal symlink skip handles
                // the rest of the tree.
                let canonical = match resolve_safe_path(search_path) {
                    Ok(p) => p,
                    Err(e) => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            e,
                        );
                    }
                };
                let mut files_scanned = 0usize;
                let mut matches: Vec<serde_json::Value> = Vec::new();
                let mut dirs_visited = 0usize;
                search_dir(&canonical, pattern, &mut files_scanned, &mut matches, 0, &mut dirs_visited);
                let count = matches.len();
                JsonRpcResponse::success(
                    request.id.clone(),
                    json!({ "matches": matches, "count": count }),
                )
            }
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in file service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec!["search"]
    }
}

// ---------------------------------------------------------------------------
// Tests — WS3.M5 (Path TOCTOU defence: canonicalize-then-FD)
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(not(target_os = "redox"))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write as _;

    /// Unique temp file path inside /tmp. Each test name produces a stable
    /// suffix so parallel test runs don't clobber.
    fn unique_tmp_path(tag: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        PathBuf::from(format!("/tmp/acos_ws3m5_{}_{}_{}.txt", tag, pid, nanos))
    }

    fn make_request(method: &str, params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id: Some(json!(1)),
        }
    }

    fn mcp_path() -> McpPath {
        McpPath::parse(b"file/read").unwrap()
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn read_accepts_normal_file_in_allowed_root() {
        let path = unique_tmp_path("normal");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "hello acos").unwrap();
        drop(f);

        let h = FileReadHandler::new();
        let resp = h.handle(
            &mcp_path(),
            &make_request("read", json!({"path": path.to_str().unwrap()})),
        );
        let res = match resp.result {
            Some(r) => r,
            None => {
                cleanup(&path);
                panic!("expected ok, got error: {:?}", resp.error);
            }
        };
        assert!(res["content"].as_str().unwrap().contains("hello acos"));
        cleanup(&path);
    }

    #[test]
    #[cfg(unix)]
    fn read_rejects_symlink_escape_to_outside_allowed_root() {
        // Set up: symlink whose name is in /tmp but whose target is outside
        // (a stable host file). validate_path() lets the symlink through
        // because the *string* starts with /tmp. canonicalize() catches it.
        let link_path = unique_tmp_path("symlink_escape");
        cleanup(&link_path); // in case of stale
        std::os::unix::fs::symlink("/etc/hosts", &link_path).unwrap();

        let h = FileReadHandler::new();
        let resp = h.handle(
            &mcp_path(),
            &make_request("read", json!({"path": link_path.to_str().unwrap()})),
        );

        cleanup(&link_path);

        let err = resp.error.expect("symlink escape must be rejected");
        assert!(
            err.message.contains("escapes allowed root")
                || err.message.contains("cannot resolve"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    #[cfg(unix)]
    fn read_accepts_symlink_within_allowed_root() {
        // Symlink in /tmp pointing to another file in /tmp — legitimate use,
        // must still work.
        let real = unique_tmp_path("inner_target");
        let link = unique_tmp_path("inner_link");
        std::fs::write(&real, "inside-allowed-root").unwrap();
        cleanup(&link);
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let h = FileReadHandler::new();
        let resp = h.handle(
            &mcp_path(),
            &make_request("read", json!({"path": link.to_str().unwrap()})),
        );

        cleanup(&link);
        cleanup(&real);

        let res = resp.result.expect("legitimate /tmp symlink must succeed");
        assert!(res["content"].as_str().unwrap().contains("inside-allowed-root"));
    }

    #[test]
    fn read_rejects_traversal_via_dot_dot() {
        let h = FileReadHandler::new();
        let resp = h.handle(
            &mcp_path(),
            &make_request("read", json!({"path": "/tmp/../etc/hosts"})),
        );
        let err = resp.error.expect("'..' traversal must be rejected");
        assert!(
            err.message.contains("'..'") || err.message.contains("traversal"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn read_rejects_path_outside_allowed_root_syntactically() {
        let h = FileReadHandler::new();
        let resp = h.handle(
            &mcp_path(),
            &make_request("read", json!({"path": "/etc/hosts"})),
        );
        let err = resp.error.expect("path outside /tmp must be rejected");
        assert!(
            err.message.contains("outside allowed root"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn read_returns_not_found_for_missing_file() {
        let h = FileReadHandler::new();
        let resp = h.handle(
            &mcp_path(),
            &make_request("read", json!({"path": "/tmp/this_file_does_not_exist_acos_ws3m5"})),
        );
        let err = resp.error.expect("missing file must error");
        // canonicalize() returns NotFound — message contains "cannot resolve".
        assert!(
            err.message.contains("cannot resolve") || err.message.contains("not found"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn resolve_safe_path_returns_canonical_form() {
        // Smoke test of the helper directly: real file in /tmp resolves OK.
        let path = unique_tmp_path("resolve_smoke");
        std::fs::write(&path, b"x").unwrap();

        let canonical = resolve_safe_path(path.to_str().unwrap())
            .expect("resolve must succeed for real /tmp file");
        assert!(canonical.starts_with("/tmp"));
        assert_eq!(canonical.file_name(), path.file_name());
        cleanup(&path);
    }

    // --- FileWriteHandler — WS3.M5 write-side ---

    fn write_path() -> McpPath {
        McpPath::parse(b"file_write/write").unwrap()
    }

    #[test]
    fn write_creates_file_in_allowed_root() {
        let path = unique_tmp_path("write_normal");
        let h = FileWriteHandler::new();
        let resp = h.handle(
            &write_path(),
            &make_request(
                "write",
                json!({"path": path.to_str().unwrap(), "content": "acos-write"}),
            ),
        );
        let _res = resp.result.expect("write to /tmp must succeed");
        // Verify the file actually got written and contains expected content.
        let actual = std::fs::read_to_string(&path).unwrap();
        assert_eq!(actual, "acos-write");
        cleanup(&path);
    }

    #[test]
    #[cfg(unix)]
    fn write_refuses_to_clobber_existing_symlink_target() {
        // A symlink already lives at the target name. A naive `fs::write`
        // would follow it and clobber whatever it points to (e.g.
        // /etc/passwd). resolve_safe_write_target must refuse.
        let link = unique_tmp_path("write_symlink");
        let bait = unique_tmp_path("write_symlink_bait");
        std::fs::write(&bait, b"original").unwrap();
        cleanup(&link);
        std::os::unix::fs::symlink(&bait, &link).unwrap();

        let h = FileWriteHandler::new();
        let resp = h.handle(
            &write_path(),
            &make_request(
                "write",
                json!({"path": link.to_str().unwrap(), "content": "MALICIOUS"}),
            ),
        );

        let err = resp.error.clone();
        // Bait file must NOT have been modified by the write attempt.
        let bait_after = std::fs::read_to_string(&bait).unwrap();

        cleanup(&link);
        cleanup(&bait);

        let err = err.expect("write through existing symlink must be refused");
        assert!(
            err.message.contains("symlink"),
            "unexpected message: {}",
            err.message
        );
        assert_eq!(bait_after, "original", "bait file must not be clobbered");
    }

    #[test]
    fn write_rejects_traversal_via_dot_dot() {
        let h = FileWriteHandler::new();
        let resp = h.handle(
            &write_path(),
            &make_request(
                "write",
                json!({"path": "/tmp/../etc/acos_writetest", "content": "x"}),
            ),
        );
        let err = resp.error.expect("'..' in write path must be rejected");
        assert!(
            err.message.contains("'..'") || err.message.contains("traversal"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn write_rejects_path_outside_allowed_root() {
        let h = FileWriteHandler::new();
        let resp = h.handle(
            &write_path(),
            &make_request(
                "write",
                json!({"path": "/etc/acos_writetest", "content": "x"}),
            ),
        );
        let err = resp.error.expect("write outside /tmp must be rejected");
        assert!(
            err.message.contains("outside allowed root"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn resolve_safe_write_target_for_new_file_uses_canonical_parent() {
        let target = unique_tmp_path("write_helper_smoke");
        let resolved = resolve_safe_write_target(target.to_str().unwrap())
            .expect("resolve_safe_write_target must succeed for new /tmp file");
        // Parent component canonicalised; basename preserved.
        assert!(resolved.starts_with("/tmp"));
        assert_eq!(resolved.file_name(), target.file_name());
    }

    // --- FileSearchHandler — WS3.M5 search-root canonicalisation ---

    fn search_mcp_path() -> McpPath {
        McpPath::parse(b"file_search/search").unwrap()
    }

    #[test]
    #[cfg(unix)]
    fn search_rejects_symlink_root_escaping_allowed_root() {
        // A symlink named under /tmp but pointing to /etc — search root would
        // otherwise descend into /etc and leak host config.
        let link = unique_tmp_path("search_escape_root");
        cleanup(&link);
        std::os::unix::fs::symlink("/etc", &link).unwrap();

        let h = FileSearchHandler::new();
        let resp = h.handle(
            &search_mcp_path(),
            &make_request(
                "search",
                json!({
                    "path": link.to_str().unwrap(),
                    "pattern": "root"
                }),
            ),
        );

        cleanup(&link);

        let err = resp.error.expect("symlink-root search must be rejected");
        assert!(
            err.message.contains("escapes allowed root"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn search_succeeds_on_legitimate_tmp_dir() {
        // Create a temp directory in /tmp with one file that contains a known
        // pattern, then search it.
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let dir = PathBuf::from(format!("/tmp/acos_ws3m5_searchok_{}_{}", pid, nanos));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("hit.txt");
        std::fs::write(&f, "ACOS-MARKER-IN-FILE\n").unwrap();

        let h = FileSearchHandler::new();
        let resp = h.handle(
            &search_mcp_path(),
            &make_request(
                "search",
                json!({
                    "path": dir.to_str().unwrap(),
                    "pattern": "ACOS-MARKER-IN-FILE"
                }),
            ),
        );

        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_dir(&dir);

        let res = resp.result.expect("legitimate search must succeed");
        let count = res["count"].as_u64().unwrap();
        assert_eq!(count, 1, "expected one match; got result: {:?}", res);
    }

    #[test]
    fn search_rejects_path_outside_allowed_root() {
        let h = FileSearchHandler::new();
        let resp = h.handle(
            &search_mcp_path(),
            &make_request(
                "search",
                json!({
                    "path": "/etc",
                    "pattern": "root"
                }),
            ),
        );
        let err = resp.error.expect("search outside /tmp must be rejected");
        assert!(
            err.message.contains("outside allowed root"),
            "unexpected message: {}",
            err.message
        );
    }
}
