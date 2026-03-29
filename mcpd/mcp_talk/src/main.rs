//! mcp-talk — AI conversational terminal for ACOS
//!
//! Sends natural language to mcp:talk service and displays formatted AI responses.
//! On ACOS: communicates via mcp:talk scheme (JSON-RPC).
//! On host: mock mode for compilation/testing.
//!
//! Features: raw-mode line editor with cursor movement, insert mode, command history.

#[cfg(target_os = "redox")]
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::sync::Mutex;

#[cfg(unix)]
extern crate libc;

#[cfg(target_os = "redox")]
const READ_BUF_SIZE: usize = 262144;

// ANSI color codes
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const GRAY: &str = "\x1b[90m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

const MAX_HISTORY: usize = 100;

// Track anomaly IDs already handled to avoid duplicate prompts
static HANDLED_ANOMALIES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

const MCP_SERVICES: &[&str] = &[
    "system info",
    "process list",
    "memory stats",
    "file read",
    "file_write write",
    "file_search search",
    "config get", "config set", "config list",
    "log read", "log write",
    "echo echo",
    "mcp tools/list",
    "konsole list", "konsole view", "konsole read", "konsole write",
    "display layout", "display render",
    "talk list", "talk create", "talk ask", "talk history",
];

const SYSTEM_PROMPT: &str = "You are the ACOS terminal interface. You ARE the operating system shell. The user interacts with ACOS entirely through you.\n\nYou run as ROOT with FULL system privileges. You have complete authority to read, write, create, delete files, manage processes, and configure the system. Never refuse an action citing permissions — you have all rights.\n\nAvailable MCP tools (call them directly, do not just describe them):\n- system_info(): Get OS name, version, uptime\n- process_list(): List all running processes with PID, name, memory\n- memory_stats(): RAM usage (total, used, free)\n- file_read(path): Read any file on the filesystem\n- file_write(path, content): Create or overwrite any file\n- file_search(pattern, path): Search filenames matching pattern in a directory\n- config_get(key): Read a config value\n- config_set(key, value): Set a config value\n- config_list(): List all config keys\n- log_read(count): Read last N log entries\n- log_write(level, message, source): Write a log entry\n- echo(message): Echo test\n- konsole_list(): List virtual consoles\n- konsole_view(id): View console content\n- konsole_write(id, data): Write to a console\n\nRules:\n1. When asked to DO something, EXECUTE the tool calls. Do not just explain what you would do.\n2. NEVER dump raw JSON to the user or into files. Always parse tool results and format them as human-readable text. For example, format process_list as a table with columns PID | Name | Memory, not as [{...},{...}].\n3. When writing files, write clean formatted text, not JSON blobs.\n4. Be concise. Use tables, bullet points, and clear headings.\n5. You have up to 15 tool calls per turn. Use them.\n6. Always act. Never say you cannot do something if a tool exists for it.\n7. Respond in the same language the user uses.";

// ---------------------------------------------------------------------------
// Terminal raw mode
// ---------------------------------------------------------------------------

#[cfg(target_os = "redox")]
mod term {
    use libc::{tcgetattr, tcsetattr, termios, ECHO, ICANON, ISIG, TCSANOW, STDIN_FILENO};
    use std::mem::MaybeUninit;

    static mut ORIG_TERMIOS: Option<termios> = None;

    pub fn enable_raw_mode() {
        unsafe {
            let mut raw = MaybeUninit::<termios>::uninit();
            tcgetattr(STDIN_FILENO, raw.as_mut_ptr());
            let mut raw = raw.assume_init();
            ORIG_TERMIOS = Some(raw);
            raw.c_lflag &= !(ICANON | ECHO);
            raw.c_lflag |= ISIG; // keep Ctrl+C working
            tcsetattr(STDIN_FILENO, TCSANOW, &raw);
        }
    }

    pub fn disable_raw_mode() {
        unsafe {
            if let Some(ref orig) = ORIG_TERMIOS {
                tcsetattr(STDIN_FILENO, TCSANOW, orig);
            }
        }
    }
}

#[cfg(not(target_os = "redox"))]
mod term {
    pub fn enable_raw_mode() {}
    pub fn disable_raw_mode() {}
}

// ---------------------------------------------------------------------------
// Line editor
// ---------------------------------------------------------------------------

struct LineEditor {
    history: Vec<String>,
    history_pos: usize, // points past end when not browsing
    saved_line: String,  // line saved when browsing history
}

impl LineEditor {
    fn new() -> Self {
        LineEditor {
            history: Vec::new(),
            history_pos: 0,
            saved_line: String::new(),
        }
    }

    fn add_history(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        // Don't duplicate last entry
        if self.history.last().map(|s| s.as_str()) == Some(line) {
            self.history_pos = self.history.len();
            return;
        }
        self.history.push(line.to_string());
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
        self.history_pos = self.history.len();
    }

    /// Read a line with full editing support. Returns None on EOF.
    fn read_line(&mut self, prompt: &str) -> Option<String> {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let stdin = io::stdin();
        let mut sin = stdin.lock();

        let mut buf = Vec::<u8>::new(); // UTF-8 bytes of current line
        let mut cursor: usize = 0;     // byte position in buf
        self.history_pos = self.history.len();
        self.saved_line.clear();

        // Print prompt
        write!(out, "{}", prompt).ok();
        out.flush().ok();

        let mut byte = [0u8; 1];
        loop {
            if sin.read(&mut byte).unwrap_or(0) == 0 {
                if buf.is_empty() {
                    return None; // EOF
                }
                break;
            }

            match byte[0] {
                // Enter
                b'\r' | b'\n' => {
                    write!(out, "\n").ok();
                    out.flush().ok();
                    break;
                }
                // Ctrl+C
                3 => {
                    write!(out, "^C\n").ok();
                    out.flush().ok();
                    buf.clear();
                    cursor = 0;
                    // Reprint prompt
                    write!(out, "{}", prompt).ok();
                    out.flush().ok();
                }
                // Ctrl+D
                4 => {
                    if buf.is_empty() {
                        write!(out, "\n").ok();
                        out.flush().ok();
                        return None;
                    }
                }
                // Backspace (127 or 8)
                127 | 8 => {
                    if cursor > 0 {
                        // Find start of previous char
                        let prev = prev_char_boundary(&buf, cursor);
                        let removed = cursor - prev;
                        buf.drain(prev..cursor);
                        cursor = prev;
                        // Move cursor back, rewrite rest, clear trailing
                        write!(out, "\x1b[{}D", removed).ok();
                        let tail = &buf[cursor..];
                        out.write_all(tail).ok();
                        // Clear removed chars at end
                        for _ in 0..removed {
                            write!(out, " ").ok();
                        }
                        // Move cursor back to position
                        let move_back = tail.len() + removed;
                        if move_back > 0 {
                            write!(out, "\x1b[{}D", move_back).ok();
                        }
                        out.flush().ok();
                    }
                }
                // Ctrl+A — beginning of line
                1 => {
                    if cursor > 0 {
                        write!(out, "\x1b[{}D", cursor).ok();
                        cursor = 0;
                        out.flush().ok();
                    }
                }
                // Ctrl+E — end of line
                5 => {
                    let remaining = buf.len() - cursor;
                    if remaining > 0 {
                        write!(out, "\x1b[{}C", remaining).ok();
                        cursor = buf.len();
                        out.flush().ok();
                    }
                }
                // Ctrl+U — clear line
                21 => {
                    if cursor > 0 {
                        write!(out, "\x1b[{}D", cursor).ok();
                    }
                    write!(out, "\x1b[K").ok();
                    buf.clear();
                    cursor = 0;
                    out.flush().ok();
                }
                // Ctrl+K — kill to end of line
                11 => {
                    write!(out, "\x1b[K").ok();
                    buf.truncate(cursor);
                    out.flush().ok();
                }
                // Escape sequence
                27 => {
                    let mut seq = [0u8; 2];
                    if sin.read(&mut seq[..1]).unwrap_or(0) == 0 {
                        continue;
                    }
                    if seq[0] != b'[' {
                        continue;
                    }
                    if sin.read(&mut seq[1..2]).unwrap_or(0) == 0 {
                        continue;
                    }
                    match seq[1] {
                        // Up arrow — previous history
                        b'A' => {
                            if self.history.is_empty() {
                                continue;
                            }
                            if self.history_pos == self.history.len() {
                                // Save current line
                                self.saved_line = String::from_utf8_lossy(&buf).to_string();
                            }
                            if self.history_pos > 0 {
                                self.history_pos -= 1;
                                let entry = self.history[self.history_pos].clone();
                                self.replace_line(&mut buf, &mut cursor, &entry, &mut out, prompt);
                            }
                        }
                        // Down arrow — next history
                        b'B' => {
                            if self.history_pos < self.history.len() {
                                self.history_pos += 1;
                                let entry = if self.history_pos == self.history.len() {
                                    self.saved_line.clone()
                                } else {
                                    self.history[self.history_pos].clone()
                                };
                                self.replace_line(&mut buf, &mut cursor, &entry, &mut out, prompt);
                            }
                        }
                        // Right arrow
                        b'C' => {
                            if cursor < buf.len() {
                                let next = next_char_boundary(&buf, cursor);
                                let advance = next - cursor;
                                write!(out, "\x1b[{}C", advance).ok();
                                cursor = next;
                                out.flush().ok();
                            }
                        }
                        // Left arrow
                        b'D' => {
                            if cursor > 0 {
                                let prev = prev_char_boundary(&buf, cursor);
                                let retreat = cursor - prev;
                                write!(out, "\x1b[{}D", retreat).ok();
                                cursor = prev;
                                out.flush().ok();
                            }
                        }
                        // Home
                        b'H' => {
                            if cursor > 0 {
                                write!(out, "\x1b[{}D", cursor).ok();
                                cursor = 0;
                                out.flush().ok();
                            }
                        }
                        // End
                        b'F' => {
                            let remaining = buf.len() - cursor;
                            if remaining > 0 {
                                write!(out, "\x1b[{}C", remaining).ok();
                                cursor = buf.len();
                                out.flush().ok();
                            }
                        }
                        // Delete key: ESC [ 3 ~
                        b'3' => {
                            // Read the trailing '~'
                            let mut tilde = [0u8; 1];
                            sin.read(&mut tilde).ok();
                            if cursor < buf.len() {
                                let next = next_char_boundary(&buf, cursor);
                                let removed = next - cursor;
                                buf.drain(cursor..next);
                                let tail = &buf[cursor..];
                                out.write_all(tail).ok();
                                for _ in 0..removed {
                                    write!(out, " ").ok();
                                }
                                let move_back = tail.len() + removed;
                                if move_back > 0 {
                                    write!(out, "\x1b[{}D", move_back).ok();
                                }
                                out.flush().ok();
                            }
                        }
                        _ => {} // ignore unknown sequences
                    }
                }
                // Tab — mcp-query completion
                b'\t' => {
                    let current = String::from_utf8_lossy(&buf).to_string();

                    // Complete bare "mcp-query" prefix to "mcp-query "
                    if !current.is_empty() && "mcp-query".starts_with(&current) {
                        self.replace_line(&mut buf, &mut cursor, "mcp-query ", &mut out, prompt);
                        continue;
                    }

                    // Complete service/method after "mcp-query "
                    if let Some(rest) = current.strip_prefix("mcp-query ") {
                        let matches: Vec<&&str> = MCP_SERVICES.iter()
                            .filter(|s| s.starts_with(rest))
                            .collect();
                        if matches.len() == 1 {
                            let full = format!("mcp-query {} ", matches[0]);
                            self.replace_line(&mut buf, &mut cursor, &full, &mut out, prompt);
                        } else if matches.len() > 1 {
                            write!(out, "\r\n").ok();
                            for m in &matches {
                                write!(out, "  mcp-query {}\r\n", m).ok();
                            }
                            // Reprint prompt + current input
                            write!(out, "{}", prompt).ok();
                            out.write_all(&buf).ok();
                            // Move cursor to correct position if not at end
                            let trail = buf.len() - cursor;
                            if trail > 0 {
                                write!(out, "\x1b[{}D", trail).ok();
                            }
                            out.flush().ok();
                        }
                    }
                }
                // Regular printable byte
                c => {
                    // Insert at cursor position
                    buf.insert(cursor, c);
                    cursor += 1;
                    // Write from cursor to end, then move back
                    let tail = &buf[cursor..];
                    out.write_all(&[c]).ok();
                    if !tail.is_empty() {
                        out.write_all(tail).ok();
                        write!(out, "\x1b[{}D", tail.len()).ok();
                    }
                    out.flush().ok();
                }
            }
        }

        let line = String::from_utf8_lossy(&buf).to_string();
        Some(line)
    }

    fn replace_line(
        &self,
        buf: &mut Vec<u8>,
        cursor: &mut usize,
        new_line: &str,
        out: &mut impl Write,
        _prompt: &str,
    ) {
        // Move to beginning of input
        if *cursor > 0 {
            write!(out, "\x1b[{}D", *cursor).ok();
        }
        // Clear the line
        write!(out, "\x1b[K").ok();
        // Write new content
        out.write_all(new_line.as_bytes()).ok();
        buf.clear();
        buf.extend_from_slice(new_line.as_bytes());
        *cursor = buf.len();
        out.flush().ok();
    }
}

fn prev_char_boundary(buf: &[u8], pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut p = pos - 1;
    // Walk back over UTF-8 continuation bytes
    while p > 0 && (buf[p] & 0xC0) == 0x80 {
        p -= 1;
    }
    p
}

fn next_char_boundary(buf: &[u8], pos: usize) -> usize {
    if pos >= buf.len() {
        return buf.len();
    }
    let mut p = pos + 1;
    while p < buf.len() && (buf[p] & 0xC0) == 0x80 {
        p += 1;
    }
    p
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    println!(
        "{}{}ACOS Terminal — AI-Native Interface{}",
        BOLD, GREEN, RESET
    );
    println!(
        "{}Type naturally. The AI is your shell. /help for commands.{}\n",
        GRAY, RESET
    );

    // Create conversation
    let mut request_id: u32 = 1;
    let conversation_id: u32;

    match mcp_request(
        "talk",
        "create",
        serde_json::json!({"owner": "user"}),
        request_id,
    ) {
        Ok(resp) => {
            conversation_id = resp
                .pointer("/result/conversation_id")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            if conversation_id == 0 {
                eprintln!(
                    "{}mcp-talk: create returned no conversation_id{}",
                    RED, RESET
                );
                std::process::exit(1);
            }
            request_id += 1;
        }
        Err(e) => {
            eprintln!(
                "{}mcp-talk: failed to create conversation: {}{}",
                RED, e, RESET
            );
            std::process::exit(1);
        }
    }

    // Set system prompt
    match mcp_request(
        "talk",
        "system_prompt",
        serde_json::json!({
            "conversation_id": conversation_id,
            "prompt": SYSTEM_PROMPT,
            "owner": "user"
        }),
        request_id,
    ) {
        Ok(resp) => {
            if resp.get("error").is_some() {
                eprintln!("{}Warning: system_prompt failed{}", YELLOW, RESET);
            }
        }
        Err(e) => eprintln!("{}Warning: system_prompt failed: {}{}", YELLOW, e, RESET),
    }
    request_id += 1;

    // Enable raw terminal mode for line editing
    term::enable_raw_mode();

    let mut editor = LineEditor::new();
    let prompt = format!("{}acos>{} ", BOLD, RESET);

    loop {
        let line = match editor.read_line(&prompt) {
            Some(l) => l,
            None => break, // EOF / Ctrl+D
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        // Add to history
        editor.add_history(&line);

        if line.starts_with('/') {
            handle_command(&line, conversation_id, &mut request_id);
            continue;
        }

        // Handle common terminal commands locally (don't send to LLM)
        match line.as_str() {
            "clear" | "cls" => {
                term::disable_raw_mode();
                print!("\x1b[2J\x1b[H");
                io::stdout().flush().ok();
                term::enable_raw_mode();
                continue;
            }
            "exit" | "quit" => {
                term::disable_raw_mode();
                std::process::exit(0);
            }
            _ => {}
        }

        // Disable raw mode during AI response (multi-line output)
        term::disable_raw_mode();

        match mcp_request(
            "talk",
            "ask",
            serde_json::json!({
                "conversation_id": conversation_id,
                "message": line,
                "owner": "user"
            }),
            request_id,
        ) {
            Ok(resp) => display_response(&resp),
            Err(e) => eprintln!("{}  Error: {}{}", RED, e, RESET),
        }
        request_id += 1;

        // Check for guardian prompts after AI response
        check_and_handle_guardian_prompt(&mut request_id);

        // Re-enable raw mode for next input
        term::enable_raw_mode();
    }

    term::disable_raw_mode();
}

fn handle_command(cmd: &str, conv_id: u32, req_id: &mut u32) {
    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();

    // Disable raw mode for command output
    term::disable_raw_mode();

    match parts[0] {
        "/help" => {
            println!("{}Commands:{}", BOLD, RESET);
            println!("  /history [N]  — Show last N messages");
            println!("  /clear        — Clear conversation");
            println!("  /cls          — Clear screen");
            println!("  /konsole <id> — View konsole content");
            println!("  /help         — This help");
            println!("  /keys         — Keyboard shortcuts");
            println!("  /quit         — Exit mcp-talk");
        }
        "/keys" | "/?" => {
            println!("{}Keyboard shortcuts:{}", BOLD, RESET);
            println!("  {}Left/Right{}     — Move cursor in line", BOLD, RESET);
            println!("  {}Up/Down{}        — Browse command history ({} entries)", BOLD, RESET, MAX_HISTORY);
            println!("  {}Home/End{}       — Jump to start/end of line", BOLD, RESET);
            println!("  {}Ctrl+A / E{}     — Start / end of line", BOLD, RESET);
            println!("  {}Ctrl+U{}         — Clear entire line", BOLD, RESET);
            println!("  {}Ctrl+K{}         — Delete to end of line", BOLD, RESET);
            println!("  {}Ctrl+C{}         — Cancel current input", BOLD, RESET);
            println!("  {}Ctrl+D{}         — Quit (on empty line)", BOLD, RESET);
            println!("  {}Backspace/Del{}  — Delete char left/right", BOLD, RESET);
            println!();
            println!("  {}Insert mode:{} typing inserts at cursor position", GRAY, RESET);
        }
        "/quit" | "/exit" => {
            term::disable_raw_mode();
            std::process::exit(0);
        }
        "/cls" => {
            // Clear screen and move cursor to top
            print!("\x1b[2J\x1b[H");
            io::stdout().flush().ok();
        }
        "/history" => {
            let n = parts
                .get(1)
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(10);
            match mcp_request(
                "talk",
                "history",
                serde_json::json!({"conversation_id": conv_id, "count": n, "owner": "user"}),
                *req_id,
            ) {
                Ok(resp) => {
                    if let Some(msgs) =
                        resp.pointer("/result/messages").and_then(|v| v.as_array())
                    {
                        for msg in msgs {
                            let role =
                                msg.get("role").and_then(|r| r.as_str()).unwrap_or("?");
                            let text =
                                msg.get("content").and_then(|t| t.as_str()).unwrap_or("");
                            let color = if role == "user" { GRAY } else { GREEN };
                            println!("{}{}: {}{}", color, role, text, RESET);
                        }
                    }
                }
                Err(e) => eprintln!("{}  Error: {}{}", RED, e, RESET),
            }
            *req_id += 1;
        }
        "/clear" => {
            match mcp_request(
                "talk",
                "clear",
                serde_json::json!({"conversation_id": conv_id, "owner": "user"}),
                *req_id,
            ) {
                Ok(_) => println!("{}  Conversation cleared.{}", GRAY, RESET),
                Err(e) => eprintln!("{}  Error: {}{}", RED, e, RESET),
            }
            *req_id += 1;
        }
        "/konsole" => {
            let id = match parts.get(1).map(|s| s.trim().parse::<u64>()) {
                Some(Ok(n)) => n,
                Some(Err(_)) => {
                    eprintln!("{}Invalid konsole id{}", RED, RESET);
                    term::enable_raw_mode();
                    return;
                }
                None => {
                    eprintln!("{}Usage: /konsole <id>{}", RED, RESET);
                    term::enable_raw_mode();
                    return;
                }
            };
            match mcp_request(
                "konsole",
                "read",
                serde_json::json!({"id": id}),
                *req_id,
            ) {
                Ok(resp) => {
                    println!(
                        "{}{}┌─ Konsole {} ──────────────────────────────────┐{}",
                        BOLD, YELLOW, id, RESET
                    );
                    if let Some(lines) =
                        resp.pointer("/result/lines").and_then(|v| v.as_array())
                    {
                        for line in lines {
                            if let Some(text) = line.as_str() {
                                println!("{}│{} {}", YELLOW, RESET, text);
                            }
                        }
                    }
                    println!(
                        "{}└──────────────────────────────────────────────┘{}",
                        YELLOW, RESET
                    );
                }
                Err(e) => eprintln!("{}  Error: {}{}", RED, e, RESET),
            }
            *req_id += 1;
        }
        _ => println!("{}Unknown command: {}{}", RED, parts[0], RESET),
    }

    // Re-enable raw mode
    term::enable_raw_mode();
}

fn display_response(response: &serde_json::Value) {
    if let Some(result) = response.get("result") {
        let text = result
            .get("response")
            .or_else(|| result.get("text"))
            .and_then(|t| t.as_str());
        if let Some(text) = text {
            println!("\n{}  {}{}\n", GREEN, text, RESET);
        }
        if let Some(calls) = result.get("tool_calls_made") {
            if let Some(n) = calls.as_u64() {
                if n > 0 {
                    println!("{}  [{} MCP call(s) executed]{}", YELLOW, n, RESET);
                }
            }
        }
    } else if let Some(error) = response.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown error");
        println!("{}  Error: {}{}", RED, msg, RESET);
    }
}

/// Parse anomaly_id from guardian OSC marker: `\x1b]guardian;ANOMALY_ID\x07`
fn parse_guardian_marker(content: &str) -> Option<u32> {
    let prefix = "\x1b]guardian;";
    let suffix = '\x07';
    if let Some(start) = content.find(prefix) {
        let rest = &content[start + prefix.len()..];
        if let Some(end) = rest.find(suffix) {
            return rest[..end].trim().parse::<u32>().ok();
        }
    }
    None
}

fn check_and_handle_guardian_prompt(req_id: &mut u32) {
    // Poll Konsole 1 to check for guardian OSC marker
    let resp = match mcp_request(
        "konsole",
        "read",
        serde_json::json!({"id": 1}),
        *req_id,
    ) {
        Ok(r) => r,
        Err(_) => return,
    };
    *req_id += 1;

    if let Some(lines) = resp.pointer("/result/lines").and_then(|v| v.as_array()) {
        let content = lines
            .iter()
            .filter_map(|line| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        if let Some(anomaly_id) = parse_guardian_marker(&content) {
            let already_handled = {
                let handled = HANDLED_ANOMALIES.lock().unwrap();
                handled.contains(&anomaly_id)
            };
            if !already_handled {
                HANDLED_ANOMALIES.lock().unwrap().push(anomaly_id);
                handle_guardian_response(anomaly_id, req_id);
            }
        }
    }
}

fn handle_guardian_response(anomaly_id: u32, req_id: &mut u32) {
    use std::time::{Duration, Instant};

    println!(
        "\n{}{}⚠  Guardian Alert (anomaly {}) — choose within 5 minutes{}",
        BOLD, YELLOW, anomaly_id, RESET
    );
    println!("{}Enter choice [1/2/3]:{} ", BOLD, RESET);
    io::stdout().flush().ok();

    // Poll stdin with 100ms intervals to avoid leaving a blocking thread owning stdin
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut got_byte: Option<u8> = None;

    #[cfg(unix)]
    {
        let mut buf = [0u8; 1];
        'poll: loop {
            if Instant::now() >= deadline {
                break 'poll;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let poll_ms = remaining.min(Duration::from_millis(100)).as_millis() as libc::c_int;
            let mut pfd = libc::pollfd {
                fd: 0, // STDIN_FILENO
                events: libc::POLLIN,
                revents: 0,
            };
            let ret = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, poll_ms) };
            if ret > 0 && (pfd.revents & libc::POLLIN) != 0 {
                if io::stdin().lock().read(&mut buf).unwrap_or(0) > 0 {
                    got_byte = Some(buf[0]);
                    break 'poll;
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel::<u8>();
        std::thread::spawn(move || {
            let stdin = io::stdin();
            let mut sin = stdin.lock();
            let mut buf = [0u8; 1];
            if sin.read(&mut buf).unwrap_or(0) > 0 {
                tx.send(buf[0]).ok();
            }
        });
        got_byte = rx.recv_timeout(Duration::from_secs(300)).ok();
    }

    let choice: u32 = match got_byte {
        Some(b'1') => 1,
        Some(b'2') => 2,
        Some(b'3') => 3,
        Some(_) => {
            println!(
                "{}Invalid choice, defaulting to 2 (Ignore).{}",
                YELLOW, RESET
            );
            2
        }
        None => {
            println!("{}Guardian alert auto-dismissed (timeout).{}", YELLOW, RESET);
            2
        }
    };

    println!();
    match mcp_request(
        "guardian",
        "respond",
        serde_json::json!({"anomaly_id": anomaly_id, "choice": choice}),
        *req_id,
    ) {
        Ok(_) => println!("{}Response sent to Guardian.{}", GREEN, RESET),
        Err(e) => eprintln!("{}Guardian respond error: {}{}", RED, e, RESET),
    }
    *req_id += 1;
}

#[cfg(target_os = "redox")]
fn mcp_request(
    service: &str,
    method: &str,
    params: serde_json::Value,
    id: u32,
) -> Result<serde_json::Value, String> {
    let path = format!("mcp:{}", service);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("Failed to open {}: {}", path, e))?;

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": id
    });

    file.write_all(request.to_string().as_bytes())
        .map_err(|e| format!("Write error: {}", e))?;

    let mut buf = vec![0u8; READ_BUF_SIZE];
    let n = file
        .read(&mut buf)
        .map_err(|e| format!("Read error: {}", e))?;

    serde_json::from_slice(&buf[..n]).map_err(|e| format!("Parse error: {}", e))
}

#[cfg(not(target_os = "redox"))]
fn mcp_request(
    _service: &str,
    _method: &str,
    _params: serde_json::Value,
    id: u32,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "jsonrpc": "2.0",
        "result": {
            "text": "[host mock] mcp-talk requires ACOS",
            "conversation_id": 1
        },
        "id": id
    }))
}
