# APEX Prompt — WS5: AI Supervisor (acosd)

Implement WS5 (AI Supervisor) for ACOS

## IMPORTANT: Naming Convention
**This OS is called ACOS (Agent-Centric Operating System).** Never refer to it as "Redox" in code comments, documentation, commit messages, or conversations. The micro-kernel is an internal implementation detail. The project, the OS, the brand = **ACOS**.

## Context — What's Already Built (WS1-WS4 COMPLETE)

- **WS1:** ACOS boots with full branding via QEMU (~14s)
- **WS2:** `mcp:` scheme is a native kernel scheme, 436ns latency
- **WS3:** 10 MCP services active (system, process, memory, file, file_write, file_search, log, config, echo, mcp). 44 tests. `mcp-query` CLI works.
- **WS4:** LLM Runtime — **COMPLETE with two backends:**
  - **Local inference:** SmolLM-135M (from-scratch Rust engine, 2.3 tok/s CPU) — functional but low quality, useful for offline mode
  - **API proxy (PRIMARY):** Gemini 2.5 Flash via TCP proxy at **40 tok/s** — intelligent, coherent responses

### Current Architecture (WS4)
```
┌───────────────────────────────────────────────────┐
│  ACOS (QEMU, 2GB RAM, E1000 NIC)                  │
│                                                     │
│  ┌──────────┐    ┌──────────┐                      │
│  │ mcp-query│───→│   mcpd   │                      │
│  │ (client) │    │  mcp:llm │──→ LlmHandler        │
│  └──────────┘    └──────────┘     │                 │
│                                    │ TCP via ACOS    │
│                                    │ tcp: scheme     │
│                                    ▼                 │
│                          tcp:10.0.2.2:9999           │
└──────────────────────────┼───────────────────────────┘
                           │ QEMU user-mode networking
┌──────────────────────────┼───────────────────────────┐
│  Host Linux               ▼                           │
│                    llm-proxy.py                       │
│                    (TCP :9999)                        │
│                         │                             │
│                         ▼                             │
│                    Gemini 2.5 Flash API               │
│                    (x-goog-api-key header)            │
└───────────────────────────────────────────────────────┘
```

### What Already Works (verified in QEMU)
```bash
# From ion shell inside ACOS:
mcp-query llm info
# → {"model_name":"gemini-2.5-flash","quantization":"API","backend":"host-proxy"}

mcp-query llm generate Hello I am ACOS
# → {"text":"ACOS is an Agent-Centric Operating System, built with a Rust-based
#    micro-kernel architecture...","tokens_per_sec":40.3}

mcp-query system info     # → kernel info
mcp-query process list    # → running processes
mcp-query file read /etc/hostname  # → "acos"
mcp-query config set key value     # → OK
mcp-query log write info "message" shell  # → logged
```

### Build & Run Workflow
```bash
# 1. Start LLM proxy on host
cd projects/agent_centric_os
python3 scripts/llm-proxy.py  # Listens TCP :9999, calls Gemini API

# 2. Build and boot ACOS
cd redox_base
make qemu CONFIG_NAME=acos-bare gpu=no kvm=yes

# 3. Cross-compile cycle (when code changes)
bash scripts/inject_mcpd.sh   # Sync sources to recipe
podman run ... cargo build --release --target x86_64-unknown-redox --no-default-features --features redox
# Mount image, inject binary, unmount, reboot QEMU
```

### Key Files
```
components/mcpd/               — MCP daemon (registers mcp: scheme)
components/mcp_scheme/         — ServiceHandler trait, Router, all handlers
  src/llm_handler.rs           — LlmHandler: TCP proxy to Gemini API
  src/handler.rs               — ServiceHandler trait definition
  src/protocol.rs              — JsonRpcRequest/Response
  src/system_handlers.rs       — SystemInfo, Process, Memory handlers
  src/file_handlers.rs         — FileRead, FileWrite, FileSearch handlers
  src/support_handlers.rs      — Log, Config handlers
components/mcp_query/          — CLI tool to query MCP services
components/llm_engine/         — Local LLM inference engine (backup, offline mode)
scripts/llm-proxy.py           — Host-side proxy (TCP → Gemini API)
scripts/inject_mcpd.sh         — Source sync + recipe update
redox_base/config/acos-bare.toml — ACOS image config (network + MCP init)
```

### Network Configuration (required for LLM proxy)
```
Init order: 00_drivers → 10_net (smolnetd + dhcpd) → 15_mcp (mcpd) → 99_acos_ready
QEMU: user-mode networking with E1000 NIC (default)
Host accessible from ACOS at: 10.0.2.2
LLM proxy port: 9999
```

## WS5 Objective

Create an **AI Supervisor** that can **interact with ACOS MCP services** — the AI reads system state, executes commands, chains multiple actions, and responds in natural language.

**The LLM is already integrated (WS4).** WS5 focuses on **tool calling** — making the AI actually DO things in ACOS.

**After WS5, this must work from inside ACOS:**
```bash
mcp-query ai ask "what processes are running?"
# → AI calls mcp://process/list, formats the answer naturally

mcp-query ai ask "read the hostname file"
# → AI calls mcp://file/read {path: "/etc/hostname"}, returns content

mcp-query ai ask "set config theme to dark and log that I changed it"
# → AI chains: mcp://config/set + mcp://log/write, confirms both

mcp-query ai ask "what's the system status?"
# → AI calls system/info + process/list + memory/stats, synthesizes

mcp-query ai ask "create a file /tmp/hello.txt with content 'Hello ACOS'"
# → AI calls mcp://file_write/write {path, content}, confirms
```

## Architecture

### Approach: LLM with Function Calling

Since we already have Gemini 2.5 Flash (which supports function calling natively), the AI supervisor uses **LLM function calling** — NOT a rule engine.

The flow:
```
User prompt → LlmHandler → Gemini API (with tool definitions) → function_call response
    → Execute MCP tool call → Feed result back to Gemini → Final natural language response
```

### Implementation: AiHandler inside mcpd

**Register a new "ai" service in mcpd** that orchestrates the LLM + MCP tools:

```rust
pub struct AiHandler;

impl ServiceHandler for AiHandler {
    fn handle(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        // 1. Get user query from params
        // 2. Send to LLM proxy with tool definitions
        // 3. If LLM returns a function_call → execute via MCP
        // 4. Feed tool result back to LLM
        // 5. Return final text response
    }
}
```

**Key difference from LlmHandler:** AiHandler calls the LLM proxy with **tool definitions** (Gemini function calling format), then executes the returned tool calls against the MCP bus.

### Tool Definitions for Gemini

The proxy sends these tool definitions to Gemini:
```json
{
  "tools": [{
    "function_declarations": [
      {"name": "system_info", "description": "Get ACOS system information (kernel, uptime, etc.)"},
      {"name": "process_list", "description": "List running processes"},
      {"name": "memory_stats", "description": "Get memory usage statistics"},
      {"name": "file_read", "description": "Read a file", "parameters": {"path": {"type": "string"}}},
      {"name": "file_write", "description": "Write content to a file", "parameters": {"path": {"type": "string"}, "content": {"type": "string"}}},
      {"name": "file_search", "description": "Search for files", "parameters": {"pattern": {"type": "string"}, "path": {"type": "string"}}},
      {"name": "config_get", "description": "Get a config value", "parameters": {"key": {"type": "string"}}},
      {"name": "config_set", "description": "Set a config value", "parameters": {"key": {"type": "string"}, "value": {"type": "string"}}},
      {"name": "config_list", "description": "List all config keys"},
      {"name": "log_write", "description": "Write to system log", "parameters": {"level": {"type": "string"}, "message": {"type": "string"}, "source": {"type": "string"}}},
      {"name": "log_read", "description": "Read recent log entries", "parameters": {"count": {"type": "integer"}}}
    ]
  }]
}
```

### Deadlock Avoidance

AiHandler is inside mcpd. If it opens `mcp:process` via fd, it deadlocks (mcpd is single-threaded and already handling the current request).

**Solution: Direct internal dispatch.** AiHandler gets a reference to the Router and dispatches tool calls internally without going through the kernel scheme:

```rust
impl AiHandler {
    fn execute_tool(&self, router: &Router, tool: &str, params: Value) -> Value {
        let (service, method) = parse_tool_name(tool); // "file_read" → ("file", "read")
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method,
            params,
            id: Some(Value::Number(99.into())),
        };
        let path = McpPath::parse(service.as_bytes()).unwrap();
        let response = router.route(&path, &request);
        response.result.unwrap_or(Value::Null)
    }
}
```

If direct Router access is too complex (borrow checker), fall back to **spawning mcp-query as a subprocess**:
```rust
let output = std::process::Command::new("mcp-query")
    .args(&["system", &json_request])
    .output()?;
```

### Changes to llm-proxy.py

The proxy needs to be updated to:
1. Accept a new `"ai_ask"` method that includes tool definitions
2. Send tools to Gemini API via `tools` parameter
3. Handle Gemini's `function_call` responses
4. Return structured response with function calls for ACOS to execute

## WS5 Tasks

### Phase A: AI Tool Calling (core)
5.1 Update llm-proxy.py: add tool definitions, handle Gemini function_call responses
5.2 Create AiHandler in mcp_scheme: "ask" method, calls proxy with tools
5.3 Tool call executor: parse function_call, dispatch to MCP service
5.4 Result feedback: send tool results back to LLM for final answer
5.5 Register "ai" service in mcpd, add "ai" to mcp-query CLI shorthand

### Phase B: Multi-step & Context
5.6 Multi-turn: support LLM returning multiple function_calls in sequence
5.7 Context memory: session state between queries (ring buffer or file)
5.8 Error handling: graceful errors when tool calls fail

### Phase C: Security & Polish
5.9 Permission model: restrict which tools the AI can call
5.10 Audit log: every AI action logged to mcp://log automatically
5.11 System prompt refinement: optimize for ACOS-specific tool calling accuracy

## Agent Team Structure

| Agent | Model | Role |
|-------|-------|------|
| impl-proxy | sonnet | Update llm-proxy.py with Gemini function calling (5.1) |
| impl-ai-handler | opus | Create AiHandler + tool executor + result feedback (5.2-5.5) |
| impl-multi-step | sonnet | Multi-turn, context, error handling (5.6-5.8) |
| impl-security | sonnet | Permissions + audit log (5.9-5.11) |

### Dependencies
- impl-proxy FIRST (unblocks everything)
- impl-ai-handler DEPENDS ON impl-proxy
- impl-multi-step DEPENDS ON impl-ai-handler
- impl-security DEPENDS ON impl-ai-handler

## Success Criteria
- [ ] `mcp-query ai ask "what processes are running?"` → natural language answer with real data
- [ ] `mcp-query ai ask "read /etc/hostname"` → returns file content naturally
- [ ] `mcp-query ai ask "set config theme dark and log it"` → chains 2 MCP calls
- [ ] `mcp-query ai ask "what's the system status?"` → synthesizes multiple services
- [ ] Tool calls visible in proxy logs (for debugging)
- [ ] Every AI action logged to mcp://log
- [ ] All existing 44 mcp_scheme tests still pass
- [ ] Cross-compile succeeds, ACOS boots with AI service
- [ ] mcp-query ai ask "..." works end-to-end in QEMU at reasonable speed

## Notes pour la prochaine session

1. **Le LLM est déjà intégré** — on n'a PAS besoin de Phase C (LLM evaluation). Gemini fonctionne.
2. **Le réseau fonctionne** — TCP via 10.0.2.2:9999, smolnetd + dhcpd dans init
3. **Le proxy (llm-proxy.py) doit être mis à jour** pour supporter le function calling Gemini
4. **L'architecture AI est simple** : User → AiHandler → proxy (avec tools) → Gemini → function_call → execute MCP → feedback → réponse finale
5. **Deadlock** : AiHandler est dans mcpd, il ne peut pas ouvrir `mcp:` via fd. Solution : dispatch interne via Router ou subprocess mcp-query.
6. **TOUJOURS appeler l'OS "ACOS"**, jamais "Redox".
7. **Lancer le proxy AVANT QEMU** : `python3 scripts/llm-proxy.py` sur l'hôte
