//! mcp-query — CLI tool to query MCP services on ACOS
//!
//! Usage:
//!   mcp-query <service> <json-rpc-request>
//!   mcp-query system '{"jsonrpc":"2.0","method":"info","id":1}'
//!   mcp-query process '{"jsonrpc":"2.0","method":"list","id":2}'
//!   mcp-query config '{"jsonrpc":"2.0","method":"set","params":{"key":"k","value":"v"},"id":3}'
//!
//! Shorthand (auto-wraps in JSON-RPC):
//!   mcp-query system info
//!   mcp-query process list
//!   mcp-query memory stats
//!   mcp-query config get hostname

use std::env;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::process;


fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: mcp-query <service> <method-or-json>");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  mcp-query system info");
        eprintln!("  mcp-query process list");
        eprintln!("  mcp-query memory stats");
        eprintln!("  mcp-query file read /etc/hostname");
        eprintln!("  mcp-query config set mykey myvalue");
        eprintln!("  mcp-query config get mykey");
        eprintln!("  mcp-query log write info 'hello world' shell");
        eprintln!("  mcp-query konsole view 0       # View konsole content");
        eprintln!("  mcp-query konsole watch 0      # Live watch (refreshes every 1s)");
        eprintln!("  mcp-query system '{{\"jsonrpc\":\"2.0\",\"method\":\"info\",\"id\":1}}'");
        process::exit(1);
    }

    let service = &args[1];

    if service == "konsole" && args.len() >= 4 && (args[2] == "view" || args[2] == "watch") {
        let konsole_id = &args[3];
        let is_watch = args[2] == "watch";

        loop {
            let read_json = format!(
                r#"{{"jsonrpc":"2.0","method":"read","params":{{"id":{}}},"id":1}}"#,
                konsole_id
            );

            let mut file = OpenOptions::new().read(true).write(true)
                .open(format!("mcp:{}", service)).unwrap();
            file.write_all(read_json.as_bytes()).unwrap();
            let mut buf = vec![0u8; 65536];
            let n = file.read(&mut buf).unwrap();
            let read_response = String::from_utf8_lossy(&buf[..n]).to_string();
            drop(file);

            let val: serde_json::Value = serde_json::from_str(&read_response).unwrap_or_default();

            print!("\x1b[2J\x1b[H");
            println!("\x1b[1;36m┌─ Konsole {} ─────────────────────────────────┐\x1b[0m", konsole_id);

            if let Some(lines) = val.pointer("/result/lines").and_then(|v| v.as_array()) {
                for line in lines {
                    if let Some(text) = line.as_str() {
                        println!("\x1b[36m│\x1b[0m {}", text);
                    }
                }
            }

            if let Some(cursor) = val.pointer("/result/cursor") {
                let row = cursor.get("row").and_then(|v| v.as_u64()).unwrap_or(0);
                let col = cursor.get("col").and_then(|v| v.as_u64()).unwrap_or(0);
                println!("\x1b[36m└─ cursor: ({},{}) ─────────────────────────────┘\x1b[0m", row, col);
            }

            if !is_watch {
                break;
            }

            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        return;
    }

    let is_stream = service == "llm" && args.get(2).map(|s| s.starts_with("stream")).unwrap_or(false);
    let request_json = if args[2].starts_with('{') {
        // Raw JSON-RPC passed directly
        args[2].clone()
    } else {
        // Handle "method {params}" as a single argument (e.g. 'generate {"prompt":"hello"}')
        let (method, inline_params) = if let Some(idx) = args[2].find(" {") {
            (&args[2][..idx], Some(args[2][idx + 1..].trim()))
        } else {
            (args[2].as_str(), None)
        };

        let params = if let Some(json_str) = inline_params {
            // Inline JSON params provided directly
            json_str.to_string()
        } else {
            // Build params from extra arguments
            build_params(service, method, &args[3..])
        };

        if params.is_empty() {
            format!(r#"{{"jsonrpc":"2.0","method":"{}","id":1}}"#, method)
        } else {
            format!(
                r#"{{"jsonrpc":"2.0","method":"{}","params":{},"id":1}}"#,
                method, params
            )
        }
    };

    let scheme_path = format!("mcp:{}", service);

    // Open the scheme path for read+write
    let mut file = match OpenOptions::new()
        .read(true)
        .write(true)
        .open(&scheme_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("mcp-query: failed to open '{}': {}", scheme_path, e);
            process::exit(1);
        }
    };

    // Write the JSON-RPC request
    if let Err(e) = file.write_all(request_json.as_bytes()) {
        eprintln!("mcp-query: write error: {}", e);
        process::exit(1);
    }

    // Read the response
    if is_stream {
        // Stream mode: read response JSON, extract jsonl field, print each line
        let mut buf = vec![0u8; 65536];
        match file.read(&mut buf) {
            Ok(n) if n > 0 => {
                let response = String::from_utf8_lossy(&buf[..n]);
                // Parse JSON-RPC response and extract jsonl field
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&response) {
                    if let Some(jsonl) = val.pointer("/result/jsonl").and_then(|v| v.as_str()) {
                        for line in jsonl.lines() {
                            println!("{}", line);
                        }
                    } else {
                        println!("{}", response);
                    }
                } else {
                    println!("{}", response);
                }
            }
            Ok(_) => {
                eprintln!("mcp-query: no response from service '{}'", service);
                process::exit(1);
            }
            Err(e) => {
                eprintln!("mcp-query: read error: {}", e);
                process::exit(1);
            }
        }
    } else {
        // Read with retry loop — LLM/AI responses can take 120+ seconds via Ollama
        let mut buf = vec![0u8; 262144];
        let mut total = 0;
        let max_retries = if service == "llm" || service == "ai" { 240 } else { 60 }; // 120s for LLM/AI, 30s otherwise
        for attempt in 0..max_retries {
            match file.read(&mut buf[total..]) {
                Ok(n) if n > 0 => {
                    total += n;
                    // Check if we have a complete JSON response
                    if buf[..total].ends_with(b"}") || buf[..total].ends_with(b"}\n") {
                        break;
                    }
                }
                Ok(_) if total > 0 => break, // EOF after some data
                Ok(_) if attempt < max_retries - 1 => {
                    // No data yet — wait and retry (mcpd processing)
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }
                Ok(_) => {
                    eprintln!("mcp-query: no response from service '{}' (timeout)", service);
                    process::exit(1);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock && attempt < max_retries - 1 => {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }
            Err(e) => {
                eprintln!("mcp-query: read error: {}", e);
                process::exit(1);
            }
            }
        }
        if total > 0 {
            let response = String::from_utf8_lossy(&buf[..total]);
            println!("{}", response);
        } else {
            eprintln!("mcp-query: no response from service '{}'", service);
            process::exit(1);
        }
    }
}

/// Build a JSON params object from shorthand arguments
fn build_params(service: &str, method: &str, extra: &[String]) -> String {
    match (service, method) {
        // file/read: mcp-query file read /path
        ("file", "read") if !extra.is_empty() => {
            format!(r#"{{"path":"{}"}}"#, extra[0])
        }
        // file/write: mcp-query file_write write /path content
        ("file_write", "write") if extra.len() >= 2 => {
            format!(r#"{{"path":"{}","content":"{}"}}"#, extra[0], extra[1..].join(" "))
        }
        // file_search: mcp-query file_search search pattern /path
        ("file_search", "search") if extra.len() >= 2 => {
            format!(r#"{{"pattern":"{}","path":"{}"}}"#, extra[0], extra[1])
        }
        // config/get: mcp-query config get key
        ("config", "get") if !extra.is_empty() => {
            format!(r#"{{"key":"{}"}}"#, extra[0])
        }
        // config/set: mcp-query config set key value
        ("config", "set") if extra.len() >= 2 => {
            format!(r#"{{"key":"{}","value":"{}"}}"#, extra[0], extra[1..].join(" "))
        }
        // config/delete: mcp-query config delete key
        ("config", "delete") if !extra.is_empty() => {
            format!(r#"{{"key":"{}"}}"#, extra[0])
        }
        // log/write: mcp-query log write level message source
        ("log", "write") if extra.len() >= 3 => {
            format!(
                r#"{{"level":"{}","message":"{}","source":"{}"}}"#,
                extra[0],
                extra[1..extra.len() - 1].join(" "),
                extra[extra.len() - 1]
            )
        }
        // log/read: mcp-query log read [count]
        ("log", "read") if !extra.is_empty() => {
            format!(r#"{{"count":{}}}"#, extra[0])
        }
        // llm/generate: mcp-query llm generate Hello world
        ("llm", "generate") if !extra.is_empty() => {
            let prompt = extra.join(" ");
            let prompt = prompt.trim_matches('"');
            let escaped = prompt.replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#"{{"prompt":"{}"}}"#, escaped)
        }
        // llm/stream: mcp-query llm stream Hello world
        ("llm", "stream") if !extra.is_empty() => {
            let prompt = extra.join(" ");
            let prompt = prompt.trim_matches('"');
            let escaped = prompt.replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#"{{"prompt":"{}"}}"#, escaped)
        }
        // llm/info: mcp-query llm info — no params needed
        // ai/ask: mcp-query ai ask "prompt text"
        ("ai", "ask") if !extra.is_empty() => {
            let prompt = extra.join(" ");
            let prompt = prompt.trim_matches('"');
            let escaped = prompt.replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#"{{"prompt":"{}"}}"#, escaped)
        }
        // ai/help: mcp-query ai help — no params needed

        // konsole: mcp-query konsole <method> [json-params]
        // konsole info 0, konsole read 0, konsole write 0 "text", konsole create agent test
        ("konsole", "info" | "read" | "clear" | "destroy") if !extra.is_empty() => {
            // First extra arg is konsole id
            format!(r#"{{"id":{}}}"#, extra[0])
        }
        ("konsole", "resize") if extra.len() >= 3 => {
            format!(r#"{{"id":{},"cols":{},"rows":{}}}"#, extra[0], extra[1], extra[2])
        }
        ("konsole", "write") if extra.len() >= 2 => {
            let data = extra[1..].join(" ");
            let escaped = data.replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#"{{"id":{},"data":"{}"}}"#, extra[0], escaped)
        }
        ("konsole", "create") if extra.len() >= 2 => {
            // mcp-query konsole create agent myname [cols] [rows]
            let cols = extra.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(80);
            let rows = extra.get(3).and_then(|s| s.parse::<u32>().ok()).unwrap_or(24);
            format!(r#"{{"type":"{}","owner":"{}","cols":{},"rows":{}}}"#, extra[0], extra[1], cols, rows)
        }
        ("konsole", "cursor") if extra.len() >= 3 => {
            format!(r#"{{"id":{},"row":{},"col":{}}}"#, extra[0], extra[1], extra[2])
        }
        ("konsole", "scroll") if extra.len() >= 2 => {
            format!(r#"{{"id":{},"lines":{}}}"#, extra[0], extra[1])
        }
        ("konsole", "search") if extra.len() >= 2 => {
            let pattern = extra[1..].join(" ");
            let escaped = pattern.replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#"{{"id":{},"pattern":"{}"}}"#, extra[0], escaped)
        }
        // display: mcp-query display <method> [params]
        ("display", "focus") if !extra.is_empty() => {
            format!(r#"{{"konsole_id":{}}}"#, extra[0])
        }
        ("display", "layout") if !extra.is_empty() => {
            // Pass raw JSON layout
            extra[0].clone()
        }

        // command/run: mcp-query command run <cmd...>
        ("command", "run") if !extra.is_empty() => {
            let cmd = extra.join(" ");
            let escaped = cmd.replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#"{{"cmd":"{}"}}"#, escaped)
        }
        // service/restart: mcp-query service restart <name>
        ("service", "restart") if !extra.is_empty() => {
            format!(r#"{{"name":"{}"}}"#, extra[0])
        }

        _ => {
            // Fallback: if extra[0] starts with '{', treat it as raw JSON params
            if !extra.is_empty() && extra[0].starts_with('{') {
                extra[0].clone()
            } else {
                String::new()
            }
        }
    }
}
