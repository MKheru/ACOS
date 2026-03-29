# Session 2026-03-29 — Ion + mcpd + Ollama Integration

## Summary

Three AutoResearch v2 labs completed in one session. ACOS now has a functional MCP bus with 19 live services, direct network access via its own NIC, and local LLM integration via Ollama (phi4-mini + qwen2.5).

## Labs Completed

| Lab | Tasks | Tests | Key Result |
|-----|-------|-------|------------|
| ion-acos v1 | 8/8 | 240 | MCP builtins (mcp/guardian/agent-loop) registered in ion shell |
| ion-acos v2 | 5/5 | 308 | Real JSON-RPC 2.0 protocol, net interface, guardian real methods |
| mcpd-net-guardian | 6/6 | 439 | Net service, Ollama LLM routing, Guardian AI consultation |

## Architecture After This Session

```
mcp-talk → talk → ai → net → Ollama (qwen2.5, tool calling)
                   ↓
               net → Ollama (phi4-mini, fast chat)

guardian → net → Ollama (AI consultation for ambiguous decisions)
         → process/memory/log (health monitoring)

ion shell:
  mcp list         → mcpd services/list → probe each → [live/down]
  mcp call X.Y     → open("mcp:X") → JSON-RPC 2.0 → response
  mcp net http/dns/ping/status/tcp → direct NIC access
  guardian state/anomalies/config/history/respond → mcpd guardian handler
  ion --agent      → JSON Lines protocol for machine interaction
```

### Services (19, all verified [live] in QEMU)

system, process, memory, file, file_write, file_search, log, config, echo, mcp, llm, command, service, konsole, display, ai, talk, guardian, net

## Problems Found & Fixed

### 1. Transport Protocol (ion-acos v1 → v2)

**Problem:** ion's Redox transport used `open("/scheme/mcp/service.method")` — wrong path format.
**Root cause:** Code was written without testing against real mcpd.
**Fix:** `open("mcp:SERVICE")` + `split_endpoint("echo.ping")` → service="echo", method="ping" + JSON-RPC 2.0 write/read.
**Verified:** `mcp call echo.ping` returns `{"result":"pong"}` from real mcpd in QEMU.

### 2. Hardcoded LLM Proxy (mcpd-net-guardian)

**Problem:** `llm_handler.rs` and `ai_handler.rs` both hardcoded `const PROXY_ADDR = "tcp:10.0.2.2:9999"`.
**Root cause:** Original design assumed external proxy script (llm-proxy.py) on host.
**Fix:** Created `net_handler.rs` as central network service. llm/ai handlers dispatch to `net.llm_request` which uses curl to call Ollama's OpenAI-compatible API.
**Verified:** `llm.info` returns `{"model_name":"phi4-mini","status":"connected"}` in QEMU.

### 3. Hardcoded Service List (ion mcp list)

**Problem:** `KNOWN_SERVICES` was a `const &[&str]` with 18 hardcoded names. Adding a service required code change.
**Root cause:** Treated as "optional" to make dynamic.
**Fix:** Removed constant entirely. `mcp list` now calls `mcpd services/list` which probes each service at runtime. McpHandler got dispatch closure for probing. Display shows `[live]` or `[down]`.
**Verified:** All 19 services show `[live]` in QEMU, including `net` which wasn't in the old list.

### 4. curl Not Found (ion mcp net http)

**Problem:** `Command::new("curl")` → "No such file or directory" in ACOS.
**Root cause:** ACOS PATH doesn't include /usr/bin in all contexts. Also curl wasn't in acos-bare.toml.
**Fix:** Added `curl = {}` to acos-bare.toml. Changed to `Command::new("/usr/bin/curl")`.
**Verified:** `mcp net http get http://10.0.2.2:11434/api/version` returns Ollama version.

### 5. DNS Scheme Doesn't Exist (ion mcp net dns)

**Problem:** `open("dns:example.com")` → "No such device (os error 19)".
**Root cause:** Redox's smolnetd registers tcp:, udp:, icmp:, ip:, netcfg: but NOT dns:.
**Fix:** Use `Command::new("dns")` — Redox has a `dns` CLI tool that uses libc `getaddrinfo()`.
**Verified:** `mcp net dns resolve example.com` → `104.18.26.120` in QEMU.

### 6. Serial Test Driver Timing (acos_qemu.py)

**Problem:** Automated QEMU tests returned "echo" for all commands after the first few.
**Root cause:** `vm.run()` sent end marker 1s after command. For Ollama commands (5-30s), the marker arrived before the response, corrupting all subsequent captures.
**Fix:** Wait for shell prompt return (`root:~#`) before sending end marker. Also flush stale buffer before each command.
**Verified:** 8/8 automated tests pass, including `ai.ask` at 29.7s.

### 7. mcp-query Empty Response for Slow Commands

**Problem:** `mcp-query llm 'generate {"prompt":"hi"}'` returned "no response from service".
**Root cause:** Single `read()` call returned 0 bytes because mcpd was still processing (curl → Ollama → response → buffer). The scheme fd is synchronous — write() blocks until response ready, but read() returns 0 if called before write() completes from the client's perspective.
**Fix:** Retry loop in mcp-query with 500ms sleep, up to 30s total wait.
**Status:** Partially verified. mcp-talk works perfectly (keeps connection open). mcp-query retry needs more QEMU testing.

### 8. Guardian Wrong Method Names (ion guardian)

**Problem:** `guardian status` → "Method 'status' not found in guardian service".
**Root cause:** ion used `status/ask/log` but mcpd guardian has `state/anomalies/respond/config/history`.
**Fix:** Updated guardian.rs to use real method names from `guardian_handler.rs:901-916`.
**Verified:** `guardian state` returns `{"status":"nominal","checks_completed":1,...}` in QEMU.

## Ollama Integration

- **Host:** Ollama 0.18.3 on `0.0.0.0:11434`
- **Models:** phi4-mini (3.8B, 89 tok/s, no tools), qwen2.5:7b-instruct-q4_K_M (7.6B, tool calling)
- **GPU:** RTX 3050 8GB — activates on every LLM request from ACOS
- **Access from ACOS:** `10.0.2.2:11434` via QEMU user-mode networking
- **API:** OpenAI-compatible `/v1/chat/completions`
- **Tool calling verified on host:** qwen2.5 correctly generates tool calls (0.66s for single, 4.6s for multi)

## Principles Established

1. **Everything is MCP** — network, display, files, AI are all MCP services
2. **Guardian is the brain** — supervises all services, AI consultation for decisions
3. **Zero hardcoded values** — services discovered at runtime, paths verified, no constants
4. **Real QEMU testing** — no mock-validated features, cross-compile → inject → boot → verify
5. **Direct network access** — ACOS uses its own NIC, no proxy scripts on host

## Files Modified

### ion shell (`recipes/core/ion/source/`)
- `src/lib/builtins/mcp.rs` (1695→1688 lines) — dynamic list, curl path, dns fix
- `src/lib/builtins/guardian.rs` (641 lines) — real mcpd methods
- `src/lib/builtins/mod.rs` (904 lines) — with_mcp() registration

### mcpd (`recipes/other/mcpd/source/mcp_scheme/src/`)
- `net_handler.rs` (819 lines, NEW) — central network service + Ollama
- `llm_handler.rs` (371 lines) — routes through net, no PROXY_ADDR
- `ai_handler.rs` (1323 lines) — routes through net, qwen2.5 tool calling
- `guardian_handler.rs` (2629 lines) — network anomalies, AI consultation
- `handler.rs` — McpHandler services/list with dispatch probing
- `lib.rs` (3820 lines) — 19 services, dispatch graph

### Infrastructure
- `config/acos-bare.toml` — `curl = {}` added
- `scripts/acos_qemu.py` — serial driver prompt-wait fix
- `scripts/qemu-verify-ion.py` — QEMU verification script

## Test Counts

| Component | Before | After |
|-----------|--------|-------|
| ion | 219 | 308 (+89) |
| mcpd | 327 | 438 (+111) |
| Total | 546 | 746 (+200) |
