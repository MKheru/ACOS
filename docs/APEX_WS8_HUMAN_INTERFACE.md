# APEX Prompt — WS8: Human Interface — Terminal IA Conversationnel

**Usage:** Copier tout le contenu entre les balises `---PROMPT START---` et `---PROMPT END---` et le coller après `/apex` dans Claude Code.

---PROMPT START---

Implement WS8 (Human Interface) for ACOS at /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os/

## IMPORTANT: Naming Convention
**This OS is called ACOS (Agent-Centric Operating System).** Never refer to it as "Redox" in code comments, documentation, commit messages, or conversations. The micro-kernel is an internal implementation detail. The project, the OS, the brand = **ACOS**.

## Context — What's Already Built (WS1-WS7 COMPLETE)

- **WS1:** ACOS boots with full branding via QEMU (~4s)
- **WS2:** `mcp:` scheme is a native kernel scheme, 436ns latency, 1024 handles
- **WS3:** 10 MCP services active (system, process, memory, file, file_write, file_search, log, config, echo, mcp). 44 tests. `mcp-query` CLI works.
- **WS4:** LLM Runtime — Gemini 2.5 Flash via TCP proxy at 40 tok/s + local SmolLM-135M backup
- **WS5:** AI Supervisor — Gemini function calling with MCP tool execution. AiHandler dispatches internally via `Arc<Router>`. Multi-tool chains work.
- **WS7:** Konsole — Multi-console system with 14 MCP services. KonsoleHandler (list/create/destroy/read/write/resize/clear/info/scroll/cursor/search), DisplayHandler (layout/focus/render/info), InputRouter (hotkeys, focus cycling), Boot Konsoles (Root AI + User auto-created), AiKonsoleBridge (tool call audit trail on Konsole 0). 318 tests. `mcp-query konsole view/watch` for live ANSI rendering. QEMU validated.

### Current Architecture
```
┌───────────────────────────────────────────────────────┐
│  ACOS (QEMU, 2GB RAM, E1000 NIC)                      │
│                                                         │
│  ┌──────────┐    ┌──────────────────────────────┐      │
│  │ mcp-query│───→│           mcpd                │      │
│  │ (client) │    │  mcp:system   mcp:file        │      │
│  └──────────┘    │  mcp:process  mcp:config      │      │
│                  │  mcp:log      mcp:llm         │      │
│  ┌──────────┐    │  mcp:ai       mcp:echo        │      │
│  │ mcp-talk │    │  mcp:mcp      mcp:memory      │      │
│  │ (WS8 NEW)│───→│  mcp:konsole  mcp:display     │      │
│  └──────────┘    └──────────────────────────────┘      │
│                           │ TCP via tcp: scheme         │
│                           ▼                             │
│                  tcp:10.0.2.2:9999                      │
└───────────────────┼─────────────────────────────────────┘
                    │ QEMU user-mode networking
┌───────────────────┼─────────────────────────────────────┐
│  Host Linux        ▼                                     │
│              llm-proxy.py → Gemini 2.5 Flash API         │
└──────────────────────────────────────────────────────────┘
```

### Key Files
```
components/mcpd/               — MCP daemon (registers mcp: scheme)
components/mcp_scheme/         — ServiceHandler trait, Router, all handlers
  src/handler.rs               — ServiceHandler trait (Send + Sync)
  src/router.rs                — Router with dispatch() for internal calls
  src/ai_handler.rs            — AiHandler: function calling + tool dispatch
  src/llm_handler.rs           — LlmHandler: TCP proxy to Gemini API
  src/konsole_handler.rs       — KonsoleHandler: virtual consoles
  src/display_handler.rs       — DisplayHandler: layout + framebuffer
  src/input_router.rs          — InputRouter: keyboard routing + hotkeys
  src/boot_konsoles.rs         — Auto-create Konsole 0 (AI) + Konsole 1 (User)
  src/ai_konsole_bridge.rs     — AI activity logging to Konsole 0
  src/konsole_renderer.rs      — ANSI escape rendering for view/watch
  src/lib.rs                   — McpScheme struct, service registration
components/mcp_query/          — CLI tool to query MCP services
scripts/llm-proxy.py           — Host-side proxy (TCP → Gemini API)
scripts/inject_mcpd.sh         — Source sync + recipe update
redox_base/config/acos-bare.toml — ACOS image config
```

### ServiceHandler Pattern (for new handlers)
```rust
pub trait ServiceHandler: Send + Sync {
    fn handle(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse;
    fn list_methods(&self) -> Vec<&str>;
}
// Register in lib.rs McpScheme::new():
router.register("talk", TalkHandler::new());
```

### Build & Test Workflow
```bash
# 1. Host test (fast, no cross-compile)
cd components/mcp_scheme && cargo test --features host-test

# 2. Cross-compile for ACOS
cd ../.. && bash scripts/inject_mcpd.sh
cd redox_base && podman run --rm \
    --cap-add SYS_ADMIN --device /dev/fuse --network=host \
    --volume "$(pwd):/mnt/redox:Z" \
    --volume "$(pwd)/build/podman:/root:Z" \
    --workdir /mnt/redox/recipes/other/mcpd/source \
    redox-base \
    bash -c '
        export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
        export RUSTUP_TOOLCHAIN=redox
        cargo build --release --target x86_64-unknown-redox --no-default-features --features redox
    '

# 3. Inject binaries into image
MOUNT_DIR="/tmp/acos_mount" && mkdir -p "$MOUNT_DIR"
build/fstools/bin/redoxfs build/x86_64/acos-bare/harddrive.img "$MOUNT_DIR" &
sleep 3
cp recipes/other/mcpd/source/target/x86_64-unknown-redox/release/mcpd "$MOUNT_DIR/usr/bin/mcpd"
cp recipes/other/mcp_query/source/target/x86_64-unknown-redox/release/mcp-query "$MOUNT_DIR/usr/bin/mcp-query"
# Also inject mcp-talk binary:
cp recipes/other/mcp_talk/source/target/x86_64-unknown-redox/release/mcp-talk "$MOUNT_DIR/usr/bin/mcp-talk"
sync && sleep 1 && fusermount3 -u "$MOUNT_DIR"

# 4. Boot QEMU
make qemu CONFIG_NAME=acos-bare gpu=no kvm=yes
```

### Critical Build Lessons (from BUILD_JOURNAL)
- **`--no-default-features --features redox`** — MANDATORY for cross-compile (host-test + redox conflict)
- **DO NOT use `make r.mcpd`** — TUI blocks in pipe. Use direct `podman run`
- **`setrens(0,0)` is disabled** — handlers can read filesystem at runtime
- **Dual-mode handlers:** `#[cfg(not(target_os = "redox"))]` for host mock, `#[cfg(target_os = "redox")]` for real

---

## WS8 Objective

Create **mcp-talk** — an AI-native conversational terminal for ACOS. This replaces the traditional shell (ion) as the primary user interface. The user talks to ACOS through natural language, and ACOS responds by executing MCP tool calls and displaying results in Konsoles.

**This is NOT a chatbot.** mcp-talk is the primary interface to ACOS. Every user action goes through the AI — file management, process control, configuration, monitoring. The AI is the shell.

**After WS8, this must work from inside ACOS:**
```bash
# Launch the AI terminal (replaces ion shell)
mcp-talk

# User types in natural language:
acos> list all running processes
  AI: Executing mcp://process/list...
  PID   NAME       MEM    STATUS
  1     init       128K   running
  2     mcpd       2.1M   running
  3     smolnetd   512K   running
  4     mcp-talk   1.8M   running

acos> create a new file at /home/user/hello.txt with "Hello ACOS"
  AI: Writing to /home/user/hello.txt via mcp://file_write/write...
  ✓ File created (10 bytes)

acos> show me system memory
  AI: Querying mcp://memory/stats...
  Total: 2048 MB | Used: 156 MB | Free: 1892 MB

acos> watch the AI supervisor konsole
  AI: Opening live view of Konsole 0 (Root AI)...
  [switches to konsole watch mode, Ctrl+C to return]

acos> what happened in the last 5 minutes?
  AI: Reading mcp://log/read with count=50...
  [displays recent log entries with timestamps]
```

---

## Architecture

### Overview: 2 New Components

```
┌─────────────────────────────────────────────────────────────────┐
│                    ACOS with mcp-talk (WS8)                       │
│                                                                   │
│  ┌────────────────────────────────┐   ┌──────────────────────┐  │
│  │         mcp-talk                │   │       mcpd           │  │
│  │  ┌─────────────────────┐      │   │  mcp:talk (NEW)       │  │
│  │  │  Conversation Loop  │      │   │  mcp:konsole          │  │
│  │  │  ┌───────────────┐  │      │   │  mcp:ai               │  │
│  │  │  │ User Input    │──┼──────┼──→│  mcp:system ...       │  │
│  │  │  │ AI Response   │←─┼──────┼───│                       │  │
│  │  │  │ Tool Results  │  │      │   └──────────────────────┘  │
│  │  │  └───────────────┘  │      │                              │
│  │  │  History Manager    │      │                              │
│  │  │  Prompt Builder     │      │                              │
│  │  └─────────────────────┘      │                              │
│  └────────────────────────────────┘                              │
└─────────────────────────────────────────────────────────────────┘
```

### Component 1: TalkHandler (mcp:talk) — in mcpd

A new MCP service that manages conversation state and provides the AI backbone for mcp-talk.

```rust
pub struct TalkHandler {
    conversations: Mutex<Vec<Conversation>>,
    konsole_state: Arc<Mutex<Vec<Konsole>>>,  // shared — to read/write konsoles
}

pub struct Conversation {
    id: u32,
    history: Vec<Message>,
    created_at: String,
    owner: String,
}

pub struct Message {
    role: Role,       // User, Assistant, System, ToolResult
    content: String,
    timestamp: String,
}

pub enum Role { User, Assistant, System, ToolResult }
```

**MCP Methods:**
| Method | Params | Returns |
|--------|--------|---------|
| `ask` | {conversation_id, message} | {response, tool_calls: [...]} |
| `create` | {owner} | {conversation_id} |
| `history` | {conversation_id, count?} | {messages: [...]} |
| `list` | — | Array of {id, owner, message_count} |
| `clear` | {conversation_id} | {ok: true} |
| `system_prompt` | {conversation_id, prompt} | {ok: true} |

The `ask` method:
1. Appends user message to conversation history
2. Builds a prompt with system context + history + user message
3. Calls mcp:ai/ask internally (via shared router/dispatch)
4. The AI response may contain tool call requests
5. Executes tool calls via router dispatch
6. Formats results and returns to caller
7. Logs activity to Konsole 0 via AiKonsoleBridge

### Component 2: mcp-talk Binary — New CLI

A new binary (like mcp-query) that provides the interactive REPL.

```
components/mcp_talk/
├── Cargo.toml
└── src/
    └── main.rs
```

**Features:**
- REPL loop with `acos>` prompt
- Colorized output (ANSI escape sequences)
- Sends user input to mcp:talk/ask
- Displays AI responses with formatted tool results
- Special commands: `/history`, `/clear`, `/konsole <id>`, `/help`, `/quit`
- Conversation persistence across sessions (via mcp:talk/history)

```rust
fn main() {
    // 1. Create or resume conversation via mcp:talk/create
    // 2. Set system prompt with ACOS context
    // 3. REPL loop:
    //    - Read line from stdin
    //    - If starts with '/' → handle special command
    //    - Else → send to mcp:talk/ask
    //    - Parse response → display formatted output
    //    - If tool_calls in response → display each tool result
}
```

### System Prompt for ACOS AI Terminal

The system prompt tells the AI it IS the ACOS interface:

```
You are the ACOS terminal interface. The user interacts with the operating system through you.

Available MCP services you can call:
- system/info: Get OS information
- process/list: List running processes
- memory/stats: Memory usage
- file/read {path}: Read a file
- file_write/write {path, content}: Write a file
- file_search/search {pattern, path}: Search files
- config/get {key}: Read configuration
- config/set {key, value}: Set configuration
- log/read {count}: Read recent logs
- log/write {level, message, source}: Write a log entry
- konsole/list: List virtual consoles
- konsole/view {id}: View a console's content
- konsole/write {id, data}: Write to a console

When the user asks you to do something:
1. Determine which MCP service(s) to call
2. Execute the calls
3. Present the results clearly

Be concise. Show data in tables when appropriate. Use color (ANSI) for emphasis.
```

### Autocompletion IA (Phase B — AutoResearch)

As the user types, mcp-talk can send partial input to the AI for completion suggestions:

```
acos> show me the ru          → [Tab] → "show me the running processes"
acos> create a file at /ho    → [Tab] → "create a file at /home/user/"
```

This uses a lightweight LLM call (short prompt, 1-2 tokens) or a local cache of common completions.

---

## WS8 Phases & Tasks

### Phase A: TalkHandler + Conversation Model (core, no UI)
Build the conversation management service, testable on host.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 8.A1 | Create `talk_handler.rs` — TalkHandler with create/ask/history/list/clear/system_prompt methods | Medium | Dev |
| 8.A2 | Conversation model: Message with role/content/timestamp, Conversation with history + system prompt | Easy | Dev |
| 8.A3 | `ask` method integration: build prompt from history, call mcp:ai internally, parse tool calls, execute, format | Hard | Dev |
| 8.A4 | Register `talk` service in mcpd + lib.rs | Easy | Dev |
| 8.A5 | Host-mode unit tests: create conversation, ask, history, tool call chain | Medium | Dev |

### Phase B: mcp-talk REPL Binary
Build the interactive terminal client.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 8.B1 | Create `components/mcp_talk/` project — Cargo.toml, main.rs with REPL loop | Medium | Dev |
| 8.B2 | Colorized output: AI responses in green, tool results in yellow, errors in red, system in gray | Easy | Dev |
| 8.B3 | Special commands: `/history N`, `/clear`, `/konsole N` (switch to konsole watch), `/help`, `/quit` | Medium | Dev |
| 8.B4 | Tool call display: show each MCP call being made + result in structured format | Medium | Dev |
| 8.B5 | System prompt injection: on startup, set context about available services | Easy | Dev |
| 8.B6 | Cross-compile + inject into ACOS image + test in QEMU | Medium | Dev |

### Phase C: Conversational Intelligence
Make the AI terminal smarter.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 8.C1 | Multi-tool chains: AI can call multiple services in sequence for complex tasks | Medium | Dev |
| 8.C2 | Error handling: AI explains errors from failed tool calls and suggests alternatives | Easy | Dev |
| 8.C3 | Context awareness: AI remembers what was discussed earlier in the conversation | Medium | Dev |
| 8.C4 | Proactive suggestions: after showing results, AI suggests related actions | Easy | Dev |

### Phase D: Autocompletion IA (AutoResearch)
Smart completions as the user types.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 8.D1 | Completion engine: cache of common command patterns + MCP service names | Medium | Dev |
| 8.D2 | AI-powered completion: send partial input to LLM for smart suggestions | Hard | **AutoResearch** |
| 8.D3 | Tab-completion UX: show suggestions inline, accept with Tab/Right arrow | Medium | Dev |

### Phase E: Boot Integration
Make mcp-talk the default terminal.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 8.E1 | Auto-create Konsole 2 (Talk) at boot for mcp-talk | Easy | Dev |
| 8.E2 | Add mcp-talk to init system (starts after mcpd) | Easy | Dev |
| 8.E3 | Welcome message with ACOS branding and quick help | Easy | Dev |
| 8.E4 | Session persistence: resume conversation after reboot | Medium | Dev |

---

## Agent Team Structure

### IMPORTANT: tmux Window Management
**Quand l'orchestrateur lance un teammate via tmux, toujours créer une nouvelle fenêtre (window) avec un ratio de 50%.** Ne pas empiler les teammates dans le même pane.

| Agent | Model | Role | Phase |
|-------|-------|------|-------|
| impl-talk-handler | opus | TalkHandler + Conversation model + ask integration (8.A1-A3) | A |
| impl-register | haiku | Register talk service + CLI (8.A4) | A |
| impl-tests | sonnet | Host-mode unit tests (8.A5) | A |
| impl-mcp-talk | opus | mcp-talk REPL binary (8.B1-B5) | B |
| impl-intelligence | sonnet | Multi-tool + error handling + context (8.C1-C4) | C |
| impl-boot | haiku | Boot integration (8.E1-E3) | E |

### Dependencies
```
Phase A (talk handler) ──→ Phase B (REPL binary) ──→ Phase C (intelligence)
      │                                                     │
      └──────────────────────────────────→ Phase E (boot integration)
                                                            │
                                                     Phase D (autocompletion)
```

- Phase A FIRST (unblocks everything — REPL needs talk service)
- Phase B depends on A (needs mcp:talk to send messages)
- Phase C depends on B (needs working REPL to test intelligence)
- Phase D is independent AutoResearch (after B is working)
- Phase E depends on A + B (needs both service and binary)

---

## AutoResearch Lab Definitions

### Lab D — AI Autocompletion Accuracy
**Config:** `evolution/labs/ws8-autocomplete.yaml` (host-only)
**Run:** `/autoresearch_labs evolution/labs/ws8-autocomplete.yaml --budget 20`
- Metric: `completion_accuracy` (percent of correct completions from a test set)
- Target: >= 80%
- Tests completion against 100 partial commands with known correct completions
- Allowed files: `src/talk_handler.rs`

---

## Success Criteria

### Must Have (Phase A-B)
- [ ] `mcp-talk` launches and shows `acos>` prompt
- [ ] User can type natural language and get AI responses
- [ ] AI calls MCP services and shows results
- [ ] `/history` shows conversation
- [ ] `/konsole 0` shows Root AI konsole
- [ ] All existing 318+ tests still pass
- [ ] Cross-compile succeeds, ACOS boots with mcp-talk

### Should Have (Phase C-E)
- [ ] Multi-tool chains work (e.g., "list processes and show memory")
- [ ] AI explains errors and suggests alternatives
- [ ] mcp-talk starts automatically at boot
- [ ] Conversation resumes after restart

### Nice to Have (Phase D)
- [ ] Tab-completion with AI suggestions
- [ ] Inline completion display

---

## Notes

1. **mcp-talk is a SEPARATE binary** from mcpd and mcp-query. It has its own Cargo project in `components/mcp_talk/`.
2. **TalkHandler needs access to the AI dispatch** — same pattern as AiHandler. It calls mcp:ai/ask internally via the shared Arc<Router> or DispatchFn.
3. **The system prompt is critical** — it defines what the AI can do. Include all available MCP services and their methods.
4. **mcp-talk reads from stdin, writes to stdout** — it's a terminal program, not a daemon.
5. **On ACOS, stdin/stdout connect to the getty PTY** — mcp-talk runs inside Konsole 1 (User) or a dedicated Konsole 2 (Talk).
6. **Conversation history is in-memory** — persisted via mcp:talk/history. For session persistence across reboots, write to filesystem.
7. **The LLM proxy must be running on host** — mcp-talk → mcp:talk → mcp:ai → mcp:llm → tcp:10.0.2.2:9999 → llm-proxy.py → Gemini.

---PROMPT END---

## Notes pour la prochaine session

1. **Phase A est testable sur host** — pas besoin de QEMU pour le conversation model
2. **Phase B nécessite un nouveau Cargo project** — `components/mcp_talk/` avec son propre Cargo.toml et recipe
3. **Le system prompt est le coeur de WS8** — la qualité de l'interaction dépend de sa rédaction
4. **Multi-tool chains existent déjà** dans AiHandler (WS5) — TalkHandler les expose via conversation
5. **L'autocomplétion (Phase D) est le candidat idéal pour `/autoresearch_labs`** — métrique claire, itérations possibles
6. **TOUJOURS appeler l'OS "ACOS"**, jamais "Redox"
