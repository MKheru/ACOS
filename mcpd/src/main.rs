//! mcpd — MCP Daemon for ACOS
//!
//! This is the userspace daemon that registers the `mcp:` scheme with
//! the Redox OS kernel and serves MCP (Model Context Protocol) requests.
//!
//! On Redox OS:
//!   - Registers scheme via `Socket::create("mcp")`
//!   - Listens for open/read/write/close requests from any process
//!   - Routes JSON-RPC messages through the MCP router
//!
//! On Linux (development):
//!   - Runs a simple stdin/stdout test loop for debugging

#[cfg(feature = "redox")]
mod redox_daemon {
    use std::process;
    use daemon::SchemeDaemon;
    use mcp_scheme::scheme_bridge::McpSchemeBridge;
    use redox_scheme::{scheme::{SchemeState, SchemeSync}, RequestKind, SignalBehavior, Socket};

    fn run(daemon: SchemeDaemon) -> ! {
        let socket = Socket::create().expect("mcpd: failed to create mcp: scheme");

        let mut state = SchemeState::new();
        let mut bridge = McpSchemeBridge::new();

        println!("╔══════════════════════════════════════════════════╗");
        println!("║  ACOS v0.9.0 — Agent-Centric Operating System    ║");
        println!("║  MCP Daemon (mcpd) — 16 services active           ║");
        println!("║                                                    ║");
        println!("║  WS1: Kernel identity + branding                   ║");
        println!("║  WS2: mcp: native scheme (436ns, 1024 handles)     ║");
        println!("║  WS3: 10 core services (system,file,process,...)   ║");
        println!("║  WS4: LLM Runtime (Gemini 2.5 Flash + SmolLM)     ║");
        println!("║  WS5: AI Supervisor (function calling + dispatch)  ║");
        println!("║  WS7: Konsole (multi-console + display manager)    ║");
        println!("║       + Input Router + AI Konsole Bridge           ║");
        println!("║  WS8: Human Interface (mcp-talk AI terminal)       ║");
        println!("║  WS9: AI Guardian (autonomous system monitor)      ║");
        println!("╚══════════════════════════════════════════════════╝");
        println!("mcp: scheme registered — services: system, process, memory, file, file_write, file_search, log, config, echo, mcp, llm, ai, konsole, display, talk, guardian");

        let _ = daemon.ready_sync_scheme(&socket, &mut bridge);

        // NOTE: setrens(0,0) removed — entering null namespace blocks access to
        // /scheme/sys/ and /etc/ which are needed for MCP service handlers.
        // The handlers read system info at construction time (before this point),
        // but file operations requested via mcp-query (e.g. "file read /etc/hostname")
        // also require filesystem access at handler call time.
        // TODO: Re-evaluate security posture; for now functionality > sandboxing.
        // libredox::call::setrens(0, 0).expect("mcpd: failed to enter null namespace");

        loop {
            let request = match socket.next_request(SignalBehavior::Restart) {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(e) => {
                    eprintln!("mcpd: error reading request: {}", e);
                    continue;
                }
            };
            match request.kind() {
                RequestKind::Call(call) => {
                    let response = call.handle_sync(&mut bridge, &mut state);
                    if let Err(e) = socket.write_response(response, SignalBehavior::Restart) {
                        eprintln!("mcpd: error writing response: {}", e);
                        continue;
                    }
                }
                RequestKind::OnClose { id } => bridge.on_close(id),
                _ => {}
            }
        }

        process::exit(0);
    }

    pub fn start() {
        SchemeDaemon::new(run);
    }
}

#[cfg(not(feature = "redox"))]
mod linux_test {
    use mcp_scheme::McpScheme;
    use std::io::{self, BufRead, Write};

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("mcpd: running in Linux test mode (stdin/stdout)");
        eprintln!("mcpd: type JSON-RPC requests, one per line");
        eprintln!("mcpd: services: {:?}", McpScheme::new().list_services());
        eprintln!("---");

        let mut scheme = McpScheme::new();
        // Open a default echo connection for testing
        let handle = scheme.open(b"echo").expect("failed to open echo service");

        let stdin = io::stdin();
        let mut stdout = io::stdout();

        for line in stdin.lock().lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            // Write request
            match scheme.write(handle, line.as_bytes()) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("mcpd: write error: {}", e);
                    continue;
                }
            }

            // Read response
            let mut buf = vec![0u8; 65536];
            match scheme.read(handle, &mut buf) {
                Ok(n) if n > 0 => {
                    stdout.write_all(&buf[..n])?;
                    stdout.write_all(b"\n")?;
                    stdout.flush()?;
                }
                Ok(_) => eprintln!("mcpd: no response"),
                Err(e) => eprintln!("mcpd: read error: {}", e),
            }
        }

        scheme.close(handle).map_err(|e| format!("close error: {}", e))?;
        Ok(())
    }
}

fn main() {
    #[cfg(feature = "redox")]
    redox_daemon::start();

    #[cfg(not(feature = "redox"))]
    {
        if let Err(e) = linux_test::run() {
            eprintln!("mcpd: fatal error: {}", e);
            std::process::exit(1);
        }
    }
}
