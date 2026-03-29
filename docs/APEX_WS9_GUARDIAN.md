# APEX Prompt — WS9: AI Guardian — Autonomous System Monitor

Implement WS9 (AI Guardian) for ACOS

## IMPORTANT: Naming Convention
**This OS is called ACOS (Agent-Centric Operating System).** Never refer to it as "Redox" in code comments, documentation, commit messages, or conversations. The micro-kernel is an internal implementation detail. The project, the OS, the brand = **ACOS**.

## Context — What's Already Built (WS1-WS8 COMPLETE)

- **WS1:** ACOS boots with full branding via QEMU (~4s)
- **WS2:** `mcp:` scheme is a native kernel scheme, 436ns latency, 1024 handles
- **WS3:** 10 MCP services active (system, process, memory, file, file_write, file_search, log, config, echo, mcp). 44 tests. `mcp-query` CLI works.
- **WS4:** LLM Runtime — Gemini 2.5 Flash via TCP proxy at 40 tok/s + local SmolLM-135M backup
- **WS5:** AI Supervisor — Gemini function calling with MCP tool execution. AiHandler dispatches internally via `Arc<Router>`. Multi-tool chains work.
- **WS7:** Konsole — Multi-console system with 14 MCP services. KonsoleHandler (list/create/destroy/read/write/resize/clear/info/scroll/cursor/search), DisplayHandler (layout/focus/render/info), InputRouter (hotkeys, focus cycling), Boot Konsoles (Root AI + User auto-created), AiKonsoleBridge (tool call audit trail on Konsole 0). 318 tests. QEMU validated.
- **WS8:** Human Interface — mcp-talk REPL binary (AI-native conversational terminal). TalkHandler with 6 methods (create/ask/history/list/clear/system_prompt). Raw-mode line editor with arrow keys, history, Ctrl shortcuts. Colorized output. System prompt engineering. Security hardening (ownership, history cap, msg limit, conv limit, injection mitigation). 300 tests. QEMU validated.

### Current Architecture
```
┌───────────────────────────────────────────────────────────────────┐
│  ACOS (QEMU, 2GB RAM, E1000 NIC)                                   │
│                                                                     │
│  ┌────────────┐  ┌────────────┐   ┌──────────────────────────────┐│
│  │ mcp-query  │  │  mcp-talk  │   │           mcpd               ││
│  │ (CLI tool) │  │ (AI term)  │   │  mcp:system   mcp:file       ││
│  └─────┬──────┘  └─────┬──────┘   │  mcp:process  mcp:config     ││
│        │               │          │  mcp:log      mcp:llm        ││
│        └───────┬───────┘          │  mcp:ai       mcp:echo       ││
│                │                  │  mcp:mcp      mcp:memory     ││
│  ┌─────────────┐                  │  mcp:konsole  mcp:display    ││
│  │acos-guardian│─────────────────→│  mcp:talk                    ││
│  │  (WS9 NEW) │                  │  mcp:guardian  (WS9 NEW)     ││
│  └─────────────┘                  └──────────────────────────────┘│
│                                          │ TCP via tcp: scheme     │
│                                          ▼                         │
│                                 tcp:10.0.2.2:9999                  │
└──────────────────────┼─────────────────────────────────────────────┘
                       │ QEMU user-mode networking
┌──────────────────────┼─────────────────────────────────────────────┐
│  Host Linux           ▼                                             │
│                 llm-proxy.py → Gemini 2.5 Flash API                 │
└─────────────────────────────────────────────────────────────────────┘
```

### Boot Architecture (WS9 Target)
```
Boot → kernel → init
  ├── mcpd (scheme mcp)           ← MCP daemon, all services
  ├── getty → login
  │     └── DisplayHandler splits console 50/50 vertical
  │           ├── LEFT  (Konsole 1): mcp-talk    ← User interactive terminal
  │           └── RIGHT (Konsole 0): acos-guardian ← Autonomous monitor
  └── acos-guardian auto-launched after mcpd
```

### Key Files
```
components/mcpd/               — MCP daemon (registers mcp: scheme)
components/mcp_scheme/         — ServiceHandler trait, Router, all handlers
  src/handler.rs               — ServiceHandler trait (Send + Sync)
  src/router.rs                — Router with dispatch() for internal calls
  src/ai_handler.rs            — AiHandler: function calling + tool dispatch
  src/llm_handler.rs           — LlmHandler: TCP proxy to Gemini API
  src/talk_handler.rs          — TalkHandler: conversation management
  src/konsole_handler.rs       — KonsoleHandler: virtual consoles
  src/display_handler.rs       — DisplayHandler: layout + framebuffer
  src/input_router.rs          — InputRouter: keyboard routing + hotkeys
  src/boot_konsoles.rs         — Auto-create boot konsoles
  src/ai_konsole_bridge.rs     — AI activity logging to Konsole 0
  src/lib.rs                   — McpScheme struct, service registration
components/mcp_query/          — CLI tool to query MCP services
components/mcp_talk/           — AI conversational terminal
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
router.register("guardian", GuardianHandler::new());
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
cp recipes/other/mcp_talk/source/target/x86_64-unknown-redox/release/mcp-talk "$MOUNT_DIR/usr/bin/mcp-talk"
# NEW: inject acos-guardian
cp recipes/other/acos_guardian/source/target/x86_64-unknown-redox/release/acos-guardian "$MOUNT_DIR/usr/bin/acos-guardian"
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

## WS9 Objective

Create **acos-guardian** — an autonomous AI system monitor for ACOS. The Guardian is NOT an interactive chatbot. It's an autonomous agent that runs in a monitoring loop, silently watching system health, and only interrupts the user when it detects an anomaly that requires attention.

**This is the first autonomous AI process in ACOS** — it operates independently, without user prompting, making ACOS a truly agent-centric OS where AI watches over the system 24/7.

**After WS9, this is what ACOS looks like at boot:**

```
┌──────────────────────────────────┬──────────────────────────────────┐
│ mcp-talk (User Terminal)          │ ACOS Guardian                    │
│ ═══════════════════════           │ ═══════════════════              │
│                                   │ ● System Health: NOMINAL         │
│ acos> list running processes      │                                  │
│   AI: Executing mcp://process... │ ┌──────────────────────────────┐ │
│   PID  NAME       MEM   STATUS   │ │ Processes:  12 running       │ │
│   1    init       128K  running  │ │ Memory:     156/2048 MB (7%) │ │
│   2    mcpd       2.1M  running  │ │ Uptime:     4m 23s           │ │
│   3    mcp-talk   1.8M  running  │ │ Services:   15 active        │ │
│   4    guardian   1.2M  running  │ │ Last check: 12s ago          │ │
│                                   │ └──────────────────────────────┘ │
│ acos> _                           │                                  │
│                                   │ [No anomalies detected]          │
│                                   │                                  │
├──────────────────────────────────┤                                  │
│                                   │                                  │
│                                   │                                  │
└──────────────────────────────────┴──────────────────────────────────┘
```

**When an anomaly is detected:**
```
┌──────────────────────────────────┬──────────────────────────────────┐
│ mcp-talk (User Terminal)          │ ACOS Guardian                    │
│ ═══════════════════════           │ ═══════════════════              │
│                                   │ ⚠ ANOMALY DETECTED               │
│ ┌──────────────────────────────┐ │                                  │
│ │ 🔔 Guardian Alert:           │ │ Type: Process Crash              │
│ │                              │ │ Process: smolnetd (PID 7)       │
│ │ Process "smolnetd" (PID 7)  │ │ Time: 11:42:03                   │
│ │ has crashed unexpectedly.    │ │ Impact: Network services down    │
│ │                              │ │                                  │
│ │ Suggested action: Restart    │ │ Analysis:                        │
│ │ the network daemon.          │ │ smolnetd panicked at             │
│ │                              │ │ "No network adapter found"       │
│ │ [1] Yes, restart smolnetd   │ │ This is a known issue in QEMU    │
│ │ [2] No, ignore              │ │ without E1000 pass-through.      │
│ │ [3] Give instructions       │ │                                  │
│ │                              │ │ Recommendation: Restart with     │
│ └──────────────────────────────┘ │ network adapter check disabled.  │
│                                   │                                  │
│ Choose [1/2/3]: _                 │                                  │
└──────────────────────────────────┴──────────────────────────────────┘
```

---

## Architecture

### Overview: 2 New Components

```
┌───────────────────────────────────────────────────────────────────────┐
│                    ACOS with AI Guardian (WS9)                          │
│                                                                         │
│  ┌────────────────────────────────────┐   ┌────────────────────────┐  │
│  │         acos-guardian               │   │        mcpd            │  │
│  │  ┌──────────────────────────────┐  │   │                        │  │
│  │  │   Monitoring Loop (30s)      │  │   │  mcp:guardian (NEW)    │  │
│  │  │   ┌────────────────────┐     │  │   │    ├ state             │  │
│  │  │   │ Poll Metrics       │─────┼──┼──→│    ├ anomalies         │  │
│  │  │   │ Detect Anomalies   │     │  │   │    ├ respond           │  │
│  │  │   │ Update Display     │     │  │   │    ├ config            │  │
│  │  │   │ Prompt if needed   │     │  │   │    └ history           │  │
│  │  │   └────────────────────┘     │  │   │                        │  │
│  │  │   Anomaly Engine             │  │   │  mcp:system            │  │
│  │  │   ┌────────────────────┐     │  │   │  mcp:process           │  │
│  │  │   │ ProcessCrash       │     │  │   │  mcp:memory            │  │
│  │  │   │ MemoryThreshold    │     │  │   │  mcp:log               │  │
│  │  │   │ LogErrors          │     │  │   │  mcp:file              │  │
│  │  │   │ FileChanges        │     │  │   │  mcp:konsole           │  │
│  │  │   │ ServiceDown        │     │  │   │  mcp:display           │  │
│  │  │   └────────────────────┘     │  │   │  mcp:ai                │  │
│  │  └──────────────────────────────┘  │   │  mcp:talk              │  │
│  └────────────────────────────────────┘   └────────────────────────┘  │
│                                                                         │
│  ┌────────────────────────────────────┐                                │
│  │         mcp-talk                    │                                │
│  │  Receives guardian prompts via      │                                │
│  │  mcp:konsole inter-console msgs    │                                │
│  └────────────────────────────────────┘                                │
└───────────────────────────────────────────────────────────────────────┘
```

### Component 1: GuardianHandler (mcp:guardian) — in mcpd

A new MCP service that manages Guardian state, anomaly history, and user responses.

```rust
pub struct GuardianHandler {
    state: Mutex<GuardianState>,
    anomalies: Mutex<Vec<Anomaly>>,
    config: Mutex<GuardianConfig>,
}

pub struct GuardianState {
    status: GuardianStatus,        // Nominal, Warning, Critical
    last_check: String,            // ISO timestamp
    checks_completed: u64,
    current_metrics: SystemSnapshot,
}

pub struct SystemSnapshot {
    process_count: usize,
    process_list: Vec<ProcessInfo>,    // PID, name, state, memory
    memory_used_mb: u64,
    memory_total_mb: u64,
    memory_percent: f32,
    service_count: usize,
    log_errors_recent: usize,         // errors in last 5 minutes
    file_changes_recent: Vec<String>, // files modified since last check
}

pub struct Anomaly {
    id: u32,
    anomaly_type: AnomalyType,
    severity: Severity,              // Info, Warning, Critical
    description: String,
    detected_at: String,
    resolved: bool,
    resolution: Option<String>,
    user_response: Option<UserResponse>,
}

pub enum AnomalyType {
    ProcessCrash { pid: u32, name: String },
    MemoryThreshold { percent: f32, threshold: f32 },
    LogError { count: usize, sample: String },
    FileChange { path: String, change_type: String },
    ServiceDown { service: String },
}

pub enum Severity { Info, Warning, Critical }

pub struct UserResponse {
    choice: ResponseChoice,
    instructions: Option<String>,
    responded_at: String,
}

pub enum ResponseChoice { ApplyFix, Ignore, GiveInstructions }

pub struct GuardianConfig {
    poll_interval_secs: u32,        // default 30
    memory_threshold_percent: f32,   // default 80.0
    log_error_threshold: usize,      // default 5 in 5 min
    watched_paths: Vec<String>,      // paths to monitor for changes
    enabled: bool,
}
```

**MCP Methods:**
| Method | Params | Returns |
|--------|--------|---------|
| `state` | — | Current GuardianState + SystemSnapshot |
| `anomalies` | {resolved?, severity?, limit?} | Array of Anomaly |
| `respond` | {anomaly_id, choice, instructions?} | {ok: true, action_taken} |
| `config` | {key, value} | Updated config |
| `history` | {count?} | Array of past anomalies with resolutions |

### Component 2: acos-guardian Binary — New Autonomous Agent

A new binary that runs the autonomous monitoring loop.

```
components/acos_guardian/
├── Cargo.toml
└── src/
    └── main.rs
```

**Core Loop:**
```rust
fn main() {
    // 1. Register with mcpd: mcp:guardian/config (set defaults)
    // 2. Write to Konsole 0: "ACOS Guardian started"
    // 3. Autonomous loop:
    loop {
        // a. Poll system metrics via MCP services
        let snapshot = poll_metrics();  // process/list, memory/stats, log/read

        // b. Run anomaly detection engine
        let anomalies = detect_anomalies(&snapshot, &previous_snapshot);

        // c. Update display on Konsole 0 (right panel)
        update_display(&snapshot, &anomalies);

        // d. If anomaly detected → send interactive prompt to user
        if !anomalies.is_empty() {
            for anomaly in &anomalies {
                // Write anomaly to mcp:guardian/anomalies
                report_anomaly(anomaly);
                // Send prompt to mcp-talk via mcp:konsole cross-console
                send_user_prompt(anomaly);
            }
        }

        // e. Check for user responses
        check_responses();

        // f. Sleep for poll_interval
        sleep(config.poll_interval_secs);
    }
}
```

### Inter-Console Communication Protocol

The Guardian (Konsole 0, right) communicates with mcp-talk (Konsole 1, left) via a structured message protocol through `mcp:konsole/write`:

```rust
// Guardian sends a prompt to the user's konsole
struct GuardianPrompt {
    anomaly_id: u32,
    title: String,
    description: String,
    suggested_action: String,
    choices: Vec<Choice>,
}

struct Choice {
    key: String,      // "1", "2", "3"
    label: String,    // "Yes, restart smolnetd"
    action: String,   // MCP command to execute
}
```

The prompt is rendered as a bordered box in the user's Konsole. mcp-talk detects Guardian prompts and switches to response mode (numeric choice input).

### Anomaly Detection Engine

Each detector runs every poll cycle and compares current snapshot to previous:

| Detector | Trigger Condition | Severity | Suggested Action |
|----------|-------------------|----------|-----------------|
| **ProcessCrash** | Process in previous list missing from current | Critical | Restart the process |
| **MemoryThreshold** | memory_percent > config threshold (80%) | Warning | Identify top consumers, suggest cleanup |
| **LogErrors** | > N error-level log entries in last 5 min | Warning | Show errors, suggest investigation |
| **FileChange** | Watched path modified unexpectedly | Info | Show diff, ask if intentional |
| **ServiceDown** | MCP service not responding to ping | Critical | Restart mcpd or specific service |

### Display Format (Konsole 0 — Right Panel)

```
╔══════════════════════════════════╗
║       ACOS GUARDIAN              ║
║   ● System Health: NOMINAL      ║
╠══════════════════════════════════╣
║ Processes:  12 running           ║
║ Memory:     156/2048 MB (7%)     ║
║ Uptime:     4m 23s               ║
║ Services:   15 active            ║
║ Last check: 12s ago              ║
╠══════════════════════════════════╣
║ Recent Activity:                 ║
║ 11:40:01 ✓ All checks passed    ║
║ 11:40:31 ✓ All checks passed    ║
║ 11:41:01 ✓ All checks passed    ║
╚══════════════════════════════════╝
```

When anomaly:
```
╔══════════════════════════════════╗
║       ACOS GUARDIAN              ║
║   ⚠ System Health: WARNING      ║
╠══════════════════════════════════╣
║ ⚠ ANOMALY #3: Process Crash     ║
║ Process: smolnetd (PID 7)       ║
║ Time: 11:42:03                   ║
║ Impact: Network services down    ║
║                                  ║
║ → Prompt sent to user terminal   ║
║   Awaiting response...           ║
╠══════════════════════════════════╣
║ Processes:  11 running (-1)      ║
║ Memory:     154/2048 MB (7%)     ║
╚══════════════════════════════════╝
```

### Boot Integration — Split Console

At boot, after mcpd starts:

1. **DisplayHandler** creates a 50/50 vertical split layout automatically
2. **Konsole 0** (right) is assigned to `acos-guardian`
3. **Konsole 1** (left) is assigned to `mcp-talk` (replaces ion as default shell)
4. **InputRouter** focuses Konsole 1 (user side) by default
5. User can switch focus with existing hotkeys (Alt+Left/Right)

Boot init sequence:
```
# In /usr/lib/init.d/15_mcp:
scheme mcp mcpd

# In /usr/lib/init.d/20_guardian:
nowait acos-guardian

# In /usr/lib/init.d/30_talk (replaces getty for Konsole 1):
# mcp-talk is launched as the default shell
```

### System Prompt for Guardian AI

```
You are the ACOS System Guardian. Your sole objective is to maintain system health and optimal performance.

You are NOT interactive. You do NOT chat with the user. You monitor silently and only interrupt when something needs attention.

Your capabilities:
- Poll system metrics every 30 seconds
- Detect anomalies: process crashes, memory spikes, log errors, unexpected file changes, service failures
- Generate clear, actionable alerts with suggested fixes
- Execute approved remediation actions via MCP tool calls

When you detect an anomaly:
1. Classify severity (Info/Warning/Critical)
2. Analyze root cause using available system data
3. Generate a concise alert with suggested action
4. Present choices to the user (fix/ignore/instruct)
5. Execute the chosen action if approved

Rules:
- NEVER take destructive actions without user approval
- ALWAYS explain what you found and why it matters
- Keep status display concise — no verbose output
- Log all anomalies and resolutions for audit trail
```

---

## WS9 Phases & Tasks

### Phase A: GuardianHandler + State Model (core, no UI)
Build the guardian state management service, testable on host.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 9.A1 | Create `guardian_handler.rs` — GuardianHandler with state/anomalies/respond/config/history methods | Medium | Dev |
| 9.A2 | Data model: GuardianState, SystemSnapshot, Anomaly, AnomalyType, Severity, UserResponse, GuardianConfig | Medium | Dev |
| 9.A3 | `state` method: aggregate data from process/memory/log/system handlers via router dispatch | Medium | Dev |
| 9.A4 | `respond` method: process user response, execute approved actions via router dispatch | Hard | Dev |
| 9.A5 | Register `guardian` service in mcpd + lib.rs | Easy | Dev |
| 9.A6 | Host-mode unit tests: state polling, anomaly creation, response handling, config CRUD | Medium | Dev |

### Phase B: acos-guardian Binary + Monitoring Loop
Build the autonomous agent binary.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 9.B1 | Create `components/acos_guardian/` project — Cargo.toml, main.rs with autonomous loop | Medium | Dev |
| 9.B2 | Metric polling: call mcp:process/list, mcp:memory/stats, mcp:log/read, mcp:system/info every 30s | Medium | Dev |
| 9.B3 | Snapshot diffing: compare current vs previous snapshot, detect changes | Medium | Dev |
| 9.B4 | Display rendering: write formatted status to Konsole 0 via mcp:konsole/write | Medium | Dev |
| 9.B5 | Guardian system prompt: set via mcp:guardian/config at startup | Easy | Dev |
| 9.B6 | Cross-compile + inject into ACOS image + test in QEMU | Medium | Dev |

### Phase C: Anomaly Detection Engine
Implement the 5 anomaly detectors.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 9.C1 | ProcessCrash detector: compare process lists, detect missing PIDs | Medium | Dev |
| 9.C2 | MemoryThreshold detector: check memory_percent against configurable threshold | Easy | Dev |
| 9.C3 | LogError detector: count error-level log entries in rolling window | Medium | Dev |
| 9.C4 | FileChange detector: track watched paths, detect modifications | Medium | Dev |
| 9.C5 | ServiceDown detector: ping MCP services, detect non-responsive | Medium | Dev |
| 9.C6 | Anomaly severity classification and prioritization | Easy | Dev |

### Phase D: Interactive Prompt System
User interaction for anomaly resolution.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 9.D1 | GuardianPrompt rendering: bordered box with choices in user Konsole | Medium | Dev |
| 9.D2 | mcp-talk integration: detect guardian prompts, enter response mode | Hard | Dev |
| 9.D3 | Response routing: user choice → mcp:guardian/respond → execute action | Medium | Dev |
| 9.D4 | Action execution: restart process, clear logs, acknowledge file changes | Medium | Dev |
| 9.D5 | Timeout handling: auto-escalate if user doesn't respond within 5 min | Easy | Dev |

### Phase E: Boot Integration
Split console + auto-launch.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 9.E1 | DisplayHandler: native 50/50 vertical split at boot | Medium | Dev |
| 9.E2 | Init script: auto-launch acos-guardian after mcpd | Easy | Dev |
| 9.E3 | Init script: launch mcp-talk as default shell (replace ion) | Medium | Dev |
| 9.E4 | Boot banner update: WS9 line, guardian in service list | Easy | Dev |
| 9.E5 | inject_mcpd.sh: add acos-guardian binary to injection pipeline | Easy | Dev |
| 9.E6 | Full boot test: QEMU validates split console + both binaries running | Medium | Dev |

### Phase F: AutoResearch — Anomaly Detection Accuracy

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 9.F1 | Create anomaly simulation test suite: inject faults, measure detection rate | Hard | **AutoResearch** |
| 9.F2 | Optimize detection thresholds: minimize false positives while catching real issues | Hard | **AutoResearch** |
| 9.F3 | Optimize poll interval: balance responsiveness vs CPU overhead | Medium | **AutoResearch** |

---

## Agent Team Structure

### IMPORTANT: tmux Window Management
**Quand l'orchestrateur lance un teammate via tmux, toujours créer une nouvelle fenêtre (window) avec un ratio de 50%.** Ne pas empiler les teammates dans le même pane.

| Agent | Model | Role | Phase |
|-------|-------|------|-------|
| impl-guardian-handler | opus | GuardianHandler + state model + respond logic (9.A1-A4) | A |
| impl-register | haiku | Register guardian service + data model (9.A5, 9.A2) | A |
| impl-tests | sonnet | Host-mode unit tests (9.A6) | A |
| impl-guardian-binary | opus | acos-guardian binary + monitoring loop + display (9.B1-B5) | B |
| impl-detectors | sonnet | All 5 anomaly detectors (9.C1-C6) | C |
| impl-prompts | opus | Interactive prompt system + mcp-talk integration (9.D1-D4) | D |
| impl-boot | haiku | Boot integration (9.E1-E6) | E |

### Dependencies
```
Phase A (guardian handler) ──→ Phase B (guardian binary) ──→ Phase C (detectors)
      │                              │                            │
      │                              └──→ Phase D (prompts) ──────┤
      │                                                           │
      └──────────────────────────────────→ Phase E (boot integration)
                                                                  │
                                                           Phase F (AutoResearch)
```

- Phase A FIRST (unblocks everything — binary needs guardian service)
- Phase B depends on A (needs mcp:guardian to store state)
- Phase C depends on B (needs monitoring loop to plug detectors into)
- Phase D depends on B (needs guardian binary + user konsole)
- Phase E depends on A + B (needs both service and binary)
- Phase F is independent AutoResearch (after C is working)

---

## AutoResearch Lab Definitions

### Lab F1 — Anomaly Detection Accuracy
**Config:** `evolution/labs/ws9-anomaly-detection.yaml` (host + QEMU)
**Run:** `/autoresearch_labs evolution/labs/ws9-anomaly-detection.yaml --budget 20`
- Metric: `detection_rate` (percent of injected anomalies correctly detected)
- Target: >= 95%
- False positive rate: < 5%
- Tests against 50 simulated anomaly scenarios (process crashes, memory spikes, log floods, file tampering, service timeouts)
- Allowed files: `src/guardian_handler.rs`

### Lab F2 — Poll Interval Optimization
**Config:** `evolution/labs/ws9-poll-interval.yaml` (QEMU)
**Run:** `/autoresearch_labs evolution/labs/ws9-poll-interval.yaml --budget 10`
- Metric: `detection_latency_seconds` (time from anomaly injection to detection)
- Constraint: CPU overhead < 2% during monitoring
- Target: detection_latency < 35s with < 2% CPU
- Tests by injecting anomalies at random times and measuring detection delay
- Allowed files: `src/main.rs` (acos-guardian)

---

## Success Criteria

### Must Have (Phase A-B)
- [ ] `acos-guardian` binary compiles and runs in QEMU
- [ ] Guardian writes status display to Konsole 0 every 30s
- [ ] `mcp:guardian/state` returns current system snapshot
- [ ] `mcp:guardian/anomalies` returns anomaly history
- [ ] All existing 300+ tests still pass
- [ ] New unit tests for GuardianHandler pass

### Should Have (Phase C-D)
- [ ] ProcessCrash detector catches missing processes
- [ ] MemoryThreshold detector triggers at configurable percent
- [ ] LogError detector catches error bursts
- [ ] Interactive prompts appear in user Konsole when anomaly detected
- [ ] User can respond with choice (fix/ignore/instruct)
- [ ] Approved actions execute correctly

### Must Have (Phase E)
- [ ] Boot shows 50/50 vertical split: mcp-talk LEFT, guardian RIGHT
- [ ] Guardian auto-starts after mcpd
- [ ] mcp-talk is the default shell (not ion)
- [ ] inject_mcpd.sh handles 4 binaries

### Nice to Have (Phase F)
- [ ] Anomaly detection rate >= 95%
- [ ] False positive rate < 5%
- [ ] Detection latency < 35s
- [ ] CPU overhead < 2% during monitoring

---

## Notes

1. **acos-guardian is a SEPARATE binary** from mcpd, mcp-query, and mcp-talk. It has its own Cargo project in `components/acos_guardian/`.
2. **GuardianHandler needs access to system data** — it calls mcp:process/list, mcp:memory/stats, mcp:log/read internally via the shared Arc<Router> or DispatchFn.
3. **The Guardian is NOT interactive** — it does NOT have a conversation. It monitors and alerts. The user interacts via mcp-talk.
4. **Inter-console communication** uses mcp:konsole/write to send structured prompts from Konsole 0 to Konsole 1.
5. **The Guardian has its own system prompt** — but it's for the AI analysis of anomalies, not for chat.
6. **mcp-talk must be enhanced** to detect and render Guardian prompts (bordered choice boxes).
7. **The LLM proxy must be running on host** — guardian uses AI for anomaly analysis: acos-guardian → mcp:ai/ask → mcp:llm → tcp:10.0.2.2:9999 → Gemini.
8. **mcp-talk replaces ion as default shell** — this is a WS9 boot change. Users interact with ACOS exclusively through natural language.

## Notes pour la prochaine session

1. **Phase A est testable sur host** — pas besoin de QEMU pour le state model
2. **Phase B nécessite un nouveau Cargo project** — `components/acos_guardian/` avec son propre Cargo.toml et recipe
3. **Phase C est le coeur de WS9** — la qualité de la détection d'anomalies est le différenciateur
4. **Phase D nécessite des modifications dans mcp-talk** — attention à la coordination avec WS8
5. **Phase E change le boot flow** — mcp-talk remplace ion, tester soigneusement le boot
6. **TOUJOURS appeler l'OS "ACOS"**, jamais "Redox"
