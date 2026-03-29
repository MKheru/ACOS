//! File service handlers: read, write, search

use std::io::{BufRead, ErrorKind};
use std::path::{Component, Path};

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
                if let Err(e) = validate_path(file_path) {
                    return JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, e);
                }
                // F4: Reject files larger than MAX_FILE_SIZE
                match std::fs::metadata(file_path) {
                    Ok(meta) if meta.len() > MAX_FILE_SIZE => {
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            INVALID_PARAMS,
                            "file too large (max 10 MiB)",
                        )
                    }
                    Err(e) => {
                        // F8: Use INVALID_PARAMS for not-found, INTERNAL_ERROR otherwise
                        let code = if e.kind() == ErrorKind::NotFound {
                            INVALID_PARAMS
                        } else {
                            INTERNAL_ERROR
                        };
                        // F7: Generic error message
                        return JsonRpcResponse::error(
                            request.id.clone(),
                            code,
                            format!("file operation failed: {}", e),
                        );
                    }
                    _ => {}
                }
                match std::fs::read_to_string(file_path) {
                    Ok(content) => {
                        let size = content.len();
                        JsonRpcResponse::success(
                            request.id.clone(),
                            json!({ "content": content, "size": size }),
                        )
                    }
                    Err(e) => {
                        // F8: Use INVALID_PARAMS for not-found, INTERNAL_ERROR otherwise
                        let code = if e.kind() == ErrorKind::NotFound {
                            INVALID_PARAMS
                        } else {
                            INTERNAL_ERROR
                        };
                        // F7: Generic error message
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
                if let Err(e) = validate_path(file_path) {
                    return JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, e);
                }
                let bytes = content.len();
                match std::fs::write(file_path, content) {
                    Ok(_) => JsonRpcResponse::success(
                        request.id.clone(),
                        json!({ "bytes_written": bytes, "path": file_path }),
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
                if let Err(e) = validate_path(search_path) {
                    return JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, e);
                }
                let dir = Path::new(search_path);
                if !dir.exists() {
                    return JsonRpcResponse::error(
                        request.id.clone(),
                        INVALID_PARAMS,
                        // F7: Generic message (no path disclosure)
                        "search path does not exist",
                    );
                }
                let mut files_scanned = 0usize;
                let mut matches: Vec<serde_json::Value> = Vec::new();
                let mut dirs_visited = 0usize;
                search_dir(dir, pattern, &mut files_scanned, &mut matches, 0, &mut dirs_visited);
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
