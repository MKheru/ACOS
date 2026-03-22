# APEX Prompt — WS5: AI Supervisor (acosd)

**Usage:** Copier tout le contenu entre les balises `---PROMPT START---` et `---PROMPT END---` et le coller après `/apex` dans Claude Code.

---PROMPT START---

Implement WS5 (AI Supervisor) for ACOS at /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os/

## Context
ACOS is an AI-native OS based on a Rust micro-kernel. WS1, WS2, and WS3 are COMPLETE:
- WS1: OS boots with full ACOS branding in 4s via QEMU
- WS2: `mcp:` scheme is a REAL, NATIVE ACOS kernel scheme. 100% MCP spec. Latency 436ns.
- WS3: 10 MCP services active (system, process, memory, file, file_write, file_search, log, config, echo, mcp). 44 tests. All < 10μs. `mcp-query` CLI tool works.

Current state:
- mcpd daemon at components/mcpd/ — serves all MCP services via mcp: scheme
- mcp_scheme library at components/mcp_scheme/ — 10 handlers, Router, ServiceHandler trait
- mcp-query CLI at components/mcp_query/ — open+write+read on scheme fd
- No network stack (smolnetd crashes, no NIC in QEMU)
- No local LLM runtime yet (WS4 not started)
- Build workflow: inject_mcpd.sh → Podman cross-compile (15s) → redoxfs inject → QEMU boot (4s)

## WS5 Objective
Create `acosd` — an AI supervisor daemon that listens on the MCP bus, understands natural language, generates tool calls, and orchestrates system services.

**After WS5, this must work from inside ACOS:**
```
# From ion shell inside ACOS:
mcp-query ai ask "what processes are running?"
# → The AI reads mcp://process/list, formats the answer in natural language

mcp-query ai ask "read the hostname file"
# → The AI calls mcp://file/read with path=/etc/hostname, returns the content

mcp-query ai ask "set config theme to dark and log that I changed it"
# → The AI chains: mcp://config/set + mcp://log/write, confirms both actions

mcp-query ai ask "what kernel version is this?"
# → The AI calls mcp://system/info, extracts kernel field, responds naturally
```

## Architecture Decision: LLM Backend

Since ACOS has no network stack and WS4 (local LLM) is not yet implemented, the AI supervisor uses a **host-bridge architecture**:

```
┌─────────────────────────────────────────────┐
│  QEMU (ACOS)                                 │
│                                               │
│  ┌──────────┐    ┌──────────┐                │
│  │ mcp-query│───→│  acosd   │                │
│  │ (client) │    │ (daemon) │                │
│  └──────────┘    └────┬─────┘                │
│                       │ mcp:ai scheme        │
│                       │                       │
│  ┌───────────────────┐│                       │
│  │  mcpd (mcp: bus)  ││ tool calls            │
│  │  ├─ system        ││ via mcp:              │
│  │  ├─ process       │◄────────────────┐     │
│  │  ├─ file          │                  │     │
│  │  ├─ config        │                  │     │
│  │  └─ log           │                  │     │
│  └───────────────────┘                  │     │
│                                          │     │
│  acosd reads/writes to mcp: scheme      │     │
│  for tool calls (same as mcp-query)     │     │
└──────────────────────────────────────────┘
```

### Phase 1: Embedded Rule Engine (no LLM needed)
The AI supervisor starts as a **deterministic rule engine** that:
- Parses natural language commands using keyword matching + patterns
- Maps them to MCP tool calls
- Executes the calls and formats responses

This is NOT an LLM — it's a structured command interpreter. But it proves the architecture: user → acosd → MCP tool calls → response.

### Phase 2: LLM Integration (AutoResearch)
Once the rule engine works, iterate to add real LLM inference:
- Option A: Cross-compile a tiny LLM (SmolLM 135M / Phi-3 Mini) for ACOS (WS4)
- Option B: QEMU virtio-serial bridge to host LLM (bypass network)
- Option C: Embed a small transformer in Rust (no_std compatible)

AutoResearch loop: try each option, measure tokens/sec, pick the best.

## WS5 Tasks

### Phase A: AI Daemon foundation (Dev)
5.1 Create `acosd` daemon — registers `ai` service in mcpd, handles "ask" method
5.2 Command parser — extract intent + entities from natural language input
5.3 Tool call generator — map parsed intent to MCP service + method + params
5.4 Tool call executor — open mcp:<service>, write request, read response (like mcp-query)
5.5 Response formatter — convert raw JSON-RPC response to human-readable text

### Phase B: Multi-step reasoning (Dev + AutoResearch)
5.6 Chain planner — decompose complex requests into multiple tool calls
5.7 Context memory — remember previous interactions within a session (ring buffer)
5.8 Error recovery — retry failed tool calls, explain errors to user

### Phase C: LLM Integration (AutoResearch — requires lab iterations)
5.9 LLM backend abstraction — trait for text generation (rule engine vs LLM)
5.10 Evaluate LLM options for ACOS (cross-compile feasibility)
5.11 Integrate best LLM option, benchmark tokens/sec
5.12 Prompt engineering — system prompt for ACOS tool calling

### Phase D: Security (Dev)
5.13 Permission model — acosd can only call services the user authorized
5.14 Audit log — every AI action logged to mcp://log

## Technical constraints

### ServiceHandler trait (same as WS3)
acosd registers as a new service handler in mcpd:
```rust
router.register("ai", Box::new(AiHandler::new()));
```

The AiHandler receives requests like:
```json
{"jsonrpc":"2.0","method":"ask","params":{"query":"what processes are running?"},"id":1}
```

And internally calls other MCP services by opening scheme fds:
```rust
// Inside acosd, to call another MCP service:
let fd = std::fs::OpenOptions::new().read(true).write(true).open("mcp:process")?;
fd.write_all(b'{"jsonrpc":"2.0","method":"list","id":99}')?;
let mut buf = vec![0u8; 65536];
let n = fd.read(&mut buf)?;
// Parse response...
```

**CRITICAL:** acosd runs INSIDE mcpd (as a handler), so it can directly use the Router to dispatch internal calls without going through the scheme fd. This is faster:
```rust
// Direct internal dispatch (preferred):
let request = JsonRpcRequest { method: "list".into(), ... };
let response = self.router.route(&McpPath::parse(b"process")?, &request);
```

However, this requires either:
A. Passing a reference to Router into AiHandler (borrow checker challenge with &self)
B. Making AiHandler a special handler that gets Router access
C. Using the scheme fd approach (slower but simpler, ~10μs per call)

**Recommended:** Start with option C (scheme fd) for simplicity. Optimize to internal dispatch in a later round.

### BUT WAIT — acosd as handler inside mcpd has a problem:
If AiHandler opens `mcp:process` via fd, it goes through the kernel scheme dispatch, back into mcpd — which is already handling the current request (single-threaded blocking event loop). This will DEADLOCK.

**Solutions:**
1. **Separate daemon:** acosd is a separate binary, not a handler inside mcpd. It opens mcp: fds like any other process. Simple, no deadlock.
2. **Internal dispatch:** AiHandler calls Router directly without going through the kernel. No fd, no deadlock. But needs Router reference.
3. **Thread pool:** mcpd uses multiple threads so one thread can handle the ai request while others handle the tool call sub-requests.

**Recommended: Option 1 (separate daemon)**
- Simplest architecture
- No borrow checker fights
- acosd is just like mcp-query but persistent and with a rule engine
- Registers its OWN scheme `ai:` (separate from `mcp:`)
- User queries: `mcp-query ai ask "..."` → opens `ai:` scheme → acosd handles it

Wait — registering a second scheme requires a second `Socket::create("ai")` and a second SchemeDaemon. Let's keep it simpler:

**SIMPLEST APPROACH: acosd as a standalone CLI tool**
```
mcp-query ai "what processes are running?"
→ acosd binary is invoked
→ parses the query
→ calls mcp-query process list (or opens mcp:process fd directly)
→ formats and prints the response
```

No daemon needed for Phase A. Just a binary that:
1. Parses args
2. Maps to MCP calls
3. Executes them (open+write+read on mcp: fds)
4. Formats output

Later (Phase C) it becomes a daemon.

### Cross-compilation workflow (same as WS3)
```bash
cd /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os
# Create acosd component
# Copy to redox build tree
# Cross-compile with podman
# Inject into image
# Boot and test
```

## AutoResearch loop specifications

### For command parser (Phase A, task 5.2)
```
FOR iteration IN 1..10:
    1. Define/expand pattern matching rules
    2. Add test cases (commands + expected MCP calls)
    3. cargo test --features host-test (must pass)
    4. Measure: % of test commands correctly parsed
    5. Log to evolution/results/ws5_parser.tsv
    6. Write evolution/memory/ws5_parser_round_N.md
    7. If accuracy > 95% on 50+ test cases → STOP
```

### For LLM integration (Phase C, task 5.10-5.11)
```
FOR each LLM option (SmolLM-135M, Phi3-mini, TinyLlama, rule-engine):
    1. Attempt cross-compile for x86_64-unknown-redox
    2. If compile succeeds: measure tokens/sec in QEMU
    3. If compile fails: document blockers
    4. Log to evolution/results/ws5_llm_eval.tsv
    5. Write evolution/memory/ws5_llm_round_N.md

    IMPORTANT: This is a TRUE AutoResearch lab.
    - Create a git branch for each attempt
    - Commit after each iteration (success or failure)
    - The git history IS the research log
    - Failed attempts are valuable data — commit them too
    - Use evolution/memory/ to record what was tried, what worked, what didn't

    Target: > 5 tokens/sec on CPU in QEMU, < 2GB RAM
```

### For tool call accuracy (Phase B, task 5.6)
```
FOR iteration IN 1..15:
    1. Define complex multi-step test scenarios
    2. Run through the planner
    3. Verify correct MCP calls generated in correct order
    4. Measure: % of scenarios producing correct call chains
    5. Log to evolution/results/ws5_toolcall.tsv
    6. Write evolution/memory/ws5_toolcall_round_N.md
    7. If accuracy > 90% on 30+ scenarios → STOP
```

## Agent team structure

| Agent | Model | Role | Mode |
|-------|-------|------|------|
| impl-ai-core | sonnet | Implement acosd binary + command parser (Phase A: 5.1-5.5) | Dev |
| impl-ai-reasoning | sonnet | Implement chain planner + context memory (Phase B: 5.6-5.8) | Dev |
| research-llm | opus | Evaluate LLM options for ACOS, AutoResearch loop (Phase C: 5.9-5.12) | AutoResearch |
| impl-ai-security | sonnet | Permission model + audit log (Phase D: 5.13-5.14) | Dev |

### Dependencies
- impl-ai-core can start IMMEDIATELY (no deps)
- impl-ai-reasoning DEPENDS ON impl-ai-core (needs parser + executor)
- research-llm can start in PARALLEL (independent research)
- impl-ai-security DEPENDS ON impl-ai-core (needs base daemon)
- Cross-compile + boot test: AFTER impl-ai-core completes

## Key reference code (agents must read these)

### Current mcp_scheme structure
```
components/mcp_scheme/src/lib.rs       — McpScheme, open/read/write/close
components/mcp_scheme/src/protocol.rs  — JsonRpcRequest/Response
components/mcp_scheme/src/router.rs    — Router dispatches to ServiceHandler
components/mcp_scheme/src/handler.rs   — ServiceHandler trait, EchoHandler, McpHandler
components/mcp_scheme/src/system_handlers.rs  — SystemInfoHandler, ProcessHandler, MemoryHandler
components/mcp_scheme/src/file_handlers.rs    — FileReadHandler, FileWriteHandler, FileSearchHandler
components/mcp_scheme/src/support_handlers.rs — LogHandler, ConfigHandler
```

### mcp-query pattern (how to call MCP from a separate binary)
```rust
// From components/mcp_query/src/main.rs:
let mut file = OpenOptions::new().read(true).write(true).open("mcp:system")?;
file.write_all(b'{"jsonrpc":"2.0","method":"info","id":1}')?;
let mut buf = vec![0u8; 65536];
let n = file.read(&mut buf)?;
println!("{}", String::from_utf8_lossy(&buf[..n]));
```

### Available MCP services (for tool calling)
```
mcp:system   → methods: info
mcp:process  → methods: list
mcp:memory   → methods: stats
mcp:file     → methods: read (params: {path})
mcp:file_write → methods: write (params: {path, content})
mcp:file_search → methods: search (params: {pattern, path})
mcp:log      → methods: write (params: {level, message, source}), read (params: {count}), list
mcp:config   → methods: get (params: {key}), set (params: {key, value}), list, delete (params: {key})
mcp:echo     → methods: echo, ping
mcp:mcp      → methods: initialize, tools/list, tools/call, resources/list, resources/read, prompts/list, prompts/get
```

## Success criteria
- [ ] `mcp-query ai ask "what processes are running?"` returns formatted process list
- [ ] `mcp-query ai ask "read /etc/hostname"` returns file content
- [ ] `mcp-query ai ask "set config theme to dark"` executes config/set
- [ ] `mcp-query ai ask "what kernel version?"` extracts from system/info
- [ ] Multi-step: `mcp-query ai ask "set config x to y and log it"` chains 2 calls
- [ ] Command parser accuracy > 95% on 50+ test cases
- [ ] All existing 44 mcp_scheme tests still pass
- [ ] Cross-compile succeeds
- [ ] Boot still succeeds (ACOS_BOOT_OK in < 5s)
- [ ] acosd binary < 1MB
- [ ] evolution/memory/ has round entries for parser and toolcall iterations
- [ ] (Phase C bonus) LLM generates responses at > 5 tok/s in QEMU

---PROMPT END---

## Notes pour la prochaine session

1. WS5 Phase A (rule engine) est indépendant de WS4 — on peut le faire maintenant
2. WS5 Phase C (LLM) nécessitera soit WS4, soit un bridge host → QEMU
3. L'approche recommandée est : standalone binary `acosd` (pas un handler dans mcpd) pour éviter les deadlocks
4. Le pattern mcp-query (open+write+read sur fd) est réutilisable pour les tool calls
5. La tâche la plus complexe est le command parser — c'est un vrai sujet AutoResearch
6. Pour le lab AutoResearch LLM : chaque tentative = un commit git, même les échecs

## Mon avis sur WS4 vs WS5

**WS5 Phase A peut se faire SANS WS4.** Le rule engine ne nécessite aucun LLM. C'est un pattern matcher + MCP caller. Quand il marchera, on aura l'architecture complète : user → AI → MCP tool calls → response.

**WS4 (LLM Runtime) sera nécessaire pour WS5 Phase C.** Mais c'est un chantier très dur (cross-compile d'un moteur d'inférence pour ACOS). On peut le déférer.

**Recommandation :** Faire WS5 Phase A+B d'abord (rule engine + chain planner), puis WS4 quand on voudra passer au vrai LLM.
