# APEX Prompt — WS7: Konsole — Multi-Console Natif & Display Manager

Implement WS7 (Konsole) for ACOS

## IMPORTANT: Naming Convention
**This OS is called ACOS (Agent-Centric Operating System).** Never refer to it as "Redox" in code comments, documentation, commit messages, or conversations. The micro-kernel is an internal implementation detail. The project, the OS, the brand = **ACOS**.

## Context — What's Already Built (WS1-WS5 COMPLETE)

- **WS1:** ACOS boots with full branding via QEMU (~4s)
- **WS2:** `mcp:` scheme is a native kernel scheme, 436ns latency, 1024 handles
- **WS3:** 10 MCP services active (system, process, memory, file, file_write, file_search, log, config, echo, mcp). 44 tests. `mcp-query` CLI works.
- **WS4:** LLM Runtime — Gemini 2.5 Flash via TCP proxy at 40 tok/s + local SmolLM-135M backup
- **WS5:** AI Supervisor — Gemini function calling with MCP tool execution. AiHandler dispatches internally via `Arc<Router>`. Multi-tool chains work (tested: process_list, file_read + system_info, config_set + log_write).

### Current Architecture
```
┌───────────────────────────────────────────────────┐
│  ACOS (QEMU, 2GB RAM, E1000 NIC)                  │
│                                                     │
│  ┌──────────┐    ┌──────────────────────────┐      │
│  │ mcp-query│───→│        mcpd              │      │
│  │ (client) │    │  mcp:system  mcp:file    │      │
│  └──────────┘    │  mcp:process mcp:config  │      │
│                  │  mcp:log     mcp:llm     │      │
│                  │  mcp:ai     mcp:echo     │      │
│                  │  mcp:mcp   mcp:memory    │      │
│                  └──────────────────────────┘      │
│                           │ TCP via tcp: scheme     │
│                           ▼                         │
│                  tcp:10.0.2.2:9999                  │
└───────────────────┼─────────────────────────────────┘
                    │ QEMU user-mode networking
┌───────────────────┼─────────────────────────────────┐
│  Host Linux        ▼                                 │
│              llm-proxy.py → Gemini 2.5 Flash API     │
└──────────────────────────────────────────────────────┘
```

### Key Files
```
components/mcpd/               — MCP daemon (registers mcp: scheme)
components/mcp_scheme/         — ServiceHandler trait, Router, all handlers
  src/handler.rs               — ServiceHandler trait (Send + Sync)
  src/router.rs                — Router with dispatch() for internal calls
  src/ai_handler.rs            — AiHandler: function calling + tool dispatch
  src/llm_handler.rs           — LlmHandler: TCP proxy to Gemini API
  src/system_handlers.rs       — SystemInfo, Process, Memory handlers
  src/file_handlers.rs         — FileRead, FileWrite, FileSearch handlers
  src/support_handlers.rs      — Log, Config handlers
  src/lib.rs                   — McpScheme struct, McpPath, service registration
  src/scheme_bridge.rs         — Adapts McpScheme to Redox SchemeSync trait
  src/protocol.rs              — JsonRpcRequest/Response
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
router.register("konsole", KonsoleHandler::new());
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

# 3. Inject binary into image
MOUNT_DIR="/tmp/acos_mount" && mkdir -p "$MOUNT_DIR"
build/fstools/bin/redoxfs build/x86_64/acos-bare/harddrive.img "$MOUNT_DIR" &
sleep 2
cp recipes/other/mcpd/source/target/x86_64-unknown-redox/release/mcpd "$MOUNT_DIR/usr/bin/mcpd"
fusermount3 -u "$MOUNT_DIR"

# 4. Boot QEMU
make qemu CONFIG_NAME=acos-bare gpu=no kvm=yes
```

### Critical Build Lessons (from BUILD_JOURNAL)
- **`--no-default-features --features redox`** — MANDATORY for cross-compile (host-test + redox conflict)
- **DO NOT use `make r.mcpd`** — TUI blocks in pipe. Use direct `podman run`
- **`setrens(0,0)` is disabled** — handlers can read filesystem at runtime
- **Dual-mode handlers:** `#[cfg(not(target_os = "redox"))]` for host mock, `#[cfg(target_os = "redox")]` for real

### AutoResearch Loop Framework (available for WS7 labs)

The autonomous iteration loop framework is fully operational. Lab YAML configs drive automated hypothesis→modify→test→measure→decide cycles.

**Key scripts:**
```bash
# Validate a lab config
python3 harness/parse_lab.py ws7-ansi-parser validate

# Run a single iteration manually
bash harness/autoresearch.sh ws7-ansi-parser 1

# Launch autonomous Claude Code session (N iterations)
python3 harness/run_lab.py --lab ws7-ansi-parser --budget 30

# Dry-run: see generated agent prompt without launching
python3 harness/run_lab.py --lab ws7-ansi-parser --budget 5 --dry-run
```

**Pre-built WS7 lab configs** (in `evolution/labs/`):
| Lab | Type | Metric | Target |
|-----|------|--------|--------|
| `ws7-ansi-parser` | host | test_pass_rate | >= 95% |
| `ws7-render-perf` | qemu | render_time_ms | < 16ms |
| `ws7-layout-algo` | host | layout_score | >= 90% |
| `ws7-input-latency` | qemu | input_latency_ms | < 50ms |
| `ws7-scrollback-search` | host | search_time_ms | < 100ms |

**How to use in a phase:** After implementing a feature (e.g., ANSI parser), run the corresponding lab to optimize it autonomously:
```bash
# After Phase A is implemented, optimize ANSI accuracy:
python3 harness/run_lab.py --lab ws7-ansi-parser --budget 30
# → Claude Code runs 30 iterations, modifying allowed_files, testing, measuring, improving
```

Host-only labs (ansi-parser, layout-algo, scrollback-search) run in ~2s/iteration. QEMU labs (render-perf, input-latency) run in ~30s/iteration.

---

## WS7 Objective

Create a **native multi-console system** (Konsole) where each console is an MCP scheme. The AI supervisor gets a permanent Root console. Users and agents each get their own consoles. A display manager handles layout (splits, focus, resize).

**This is NOT tmux.** Konsole is a kernel-level multiplexer — each console is a `mcp://konsole/N` resource. No PTY emulation layer. Direct framebuffer rendering.

**After WS7, this must work from inside ACOS:**
```bash
# List consoles
mcp-query konsole list
# → [{"id": 0, "type": "root_ai", "owner": "acosd"}, {"id": 1, "type": "user", "owner": "login"}]

# Read console content
mcp-query konsole read --id 1
# → {"lines": ["user@acos:~$ ", "..."], "cursor": {"row": 0, "col": 12}}

# Create a new agent console
mcp-query konsole create --type agent --owner "claude"
# → {"id": 2, "type": "agent", "cols": 80, "rows": 24}

# Write to a console
mcp-query konsole write --id 0 '{"data": "AI monitoring active\n"}'

# Set layout
mcp-query display layout '{"type":"split","direction":"horizontal","ratio":[30,70],"left":{"konsole":0},"right":{"konsole":1}}'

# Resize
mcp-query konsole resize --id 1 '{"cols": 120, "rows": 40}'
```

---

## Architecture

### Overview: 3 New Components

```
┌─────────────────────────────────────────────────────────────────┐
│                    ACOS with Konsole (WS7)                       │
│                                                                   │
│  ┌────────────┐  ┌──────────────┐  ┌──────────────────────────┐ │
│  │   Input     │  │   Display    │  │         mcpd             │ │
│  │   Router    │→ │   Manager    │← │  mcp:konsole (NEW)       │ │
│  │ (keyboard)  │  │ (framebuf)   │  │  mcp:display (NEW)       │ │
│  └────────────┘  └──────────────┘  │  mcp:system, mcp:ai ...  │ │
│         ↑               ↑          └──────────────────────────┘ │
│     keyboard         framebuffer                                 │
│     events           /dev/fb0                                    │
└─────────────────────────────────────────────────────────────────┘
```

### Component 1: KonsoleHandler (mcp:konsole)

Each console is a virtual terminal with:
- **Text buffer:** ring buffer of lines (scrollback)
- **Cursor position:** row, col
- **Dimensions:** cols, rows
- **Type:** root_ai | user | agent | service
- **Owner:** process name or "acosd"
- **Attributes:** foreground/background colors per cell

```rust
pub struct Konsole {
    id: u32,
    konsole_type: KonsoleType,
    owner: String,
    cols: u32,
    rows: u32,
    buffer: Vec<Vec<Cell>>,       // rows × cols grid
    scrollback: VecDeque<Vec<Cell>>, // scrollback history
    cursor_row: u32,
    cursor_col: u32,
    dirty: bool,                  // needs re-render
}

pub struct Cell {
    ch: char,
    fg: Color,
    bg: Color,
    bold: bool,
}

pub struct KonsoleHandler {
    konsoles: Mutex<Vec<Konsole>>,
}
```

**MCP Methods:**
| Method | Params | Returns |
|--------|--------|---------|
| `list` | — | Array of {id, type, owner, cols, rows} |
| `create` | {type, owner, cols?, rows?} | {id, type, cols, rows} |
| `destroy` | {id} | {ok: true} |
| `read` | {id, from_line?, count?} | {lines: [...], cursor: {row, col}} |
| `write` | {id, data} | {ok: true, bytes_written} |
| `resize` | {id, cols, rows} | {ok: true} |
| `info` | {id} | Full konsole metadata |
| `clear` | {id} | {ok: true} |
| `scroll` | {id, lines} | {ok: true} |
| `cursor` | {id, row, col} | {ok: true} |

### Component 2: DisplayHandler (mcp:display)

Manages the **physical rendering** of konsoles to the framebuffer.

```rust
pub struct DisplayHandler {
    layout: Mutex<LayoutNode>,
    framebuffer: Mutex<Framebuffer>,
    focused_konsole: Mutex<u32>,
}

pub enum LayoutNode {
    Leaf { konsole_id: u32 },
    Split {
        direction: Direction,  // Horizontal | Vertical
        ratio: (u32, u32),     // e.g. (30, 70)
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}
```

**MCP Methods:**
| Method | Params | Returns |
|--------|--------|---------|
| `layout` | {LayoutNode JSON} | {ok: true} |
| `focus` | {konsole_id} | {ok: true, previous} |
| `render` | — | Triggers full re-render |
| `info` | — | {width, height, focused, layout} |

### Component 3: Input Router

Routes keyboard events to the focused konsole. On Redox, keyboard input comes from `display:input` or the console driver.

```rust
// In mcpd main loop or a separate thread:
// 1. Read keyboard events
// 2. Check focused_konsole from DisplayHandler
// 3. Append to konsole's input buffer
// 4. If the konsole is running a shell, forward to the shell's stdin
```

### Framebuffer Access on Redox

Redox exposes framebuffer via the `display:` scheme:
```
display:input  — keyboard/mouse events
display:0      — framebuffer for screen 0
```

The framebuffer is a raw pixel buffer. For text rendering, we need a **bitmap font** (8x16 pixels per glyph). No GPU needed — pure software rendering.

```rust
pub struct Framebuffer {
    width: u32,
    height: u32,
    stride: u32,
    data: Vec<u32>,  // ARGB32 pixels
}

impl Framebuffer {
    fn draw_char(&mut self, x: u32, y: u32, ch: char, fg: u32, bg: u32);
    fn draw_konsole(&mut self, konsole: &Konsole, x: u32, y: u32, w: u32, h: u32);
}
```

### Deadlock Avoidance

KonsoleHandler and DisplayHandler are both inside mcpd (same as AiHandler). They share data via `Mutex<>`. Since mcpd processes one request at a time (single-threaded scheme loop), mutex contention should be minimal. However:

- KonsoleHandler and DisplayHandler should NOT call each other through the router (same deadlock risk as WS5)
- Solution: Share state via `Arc<Mutex<KonsoleState>>` passed to both handlers at init time
- The display render loop should be triggered by a dirty flag, not by direct handler calls

---

## WS7 Phases & Tasks

### Phase A: Konsole Data Model (core, no rendering)
Build the in-memory konsole system, testable on host without framebuffer.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 7.A1 | Create `konsole_handler.rs` — KonsoleHandler with list/create/destroy/read/write/resize/clear/info/scroll/cursor methods | Medium | Dev |
| 7.A2 | Cell model with char + fg/bg/bold, ring buffer scrollback (configurable depth, default 1000 lines) | Medium | Dev |
| 7.A3 | ANSI escape sequence parser — handle \033[...m (colors, bold, reset), \033[H (cursor move), \033[2J (clear) | Hard | Dev |
| 7.A4 | Register `konsole` service in mcpd + add `mcp-query konsole` CLI shorthand | Easy | Dev |
| 7.A5 | Host-mode unit tests: create/write/read/resize/scrollback, ANSI parsing | Medium | Dev |

**AutoResearch Lab A — ANSI Parsing Accuracy:**
```bash
# After implementing ANSI parser, run autonomous optimization:
python3 harness/run_lab.py --lab ws7-ansi-parser --budget 30
# Config: evolution/labs/ws7-ansi-parser.yaml | Metric: test_pass_rate >= 95%
```

### Phase B: Framebuffer Rendering
Software text renderer on the Redox framebuffer.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 7.B1 | Bitmap font: embed 8x16 CP437 font as const array (256 glyphs × 16 bytes = 4KB) | Easy | Dev |
| 7.B2 | Framebuffer struct: open `display:0` on Redox, mmap pixel buffer, draw_char/draw_rect/fill | Hard | Dev |
| 7.B3 | Konsole renderer: given a Konsole + screen region (x,y,w,h), render all visible cells to framebuffer | Hard | Dev |
| 7.B4 | Cursor rendering: blinking block cursor at konsole cursor position | Easy | Dev |
| 7.B5 | Dirty-flag rendering: only re-render konsoles that changed since last frame | Medium | Dev |

**AutoResearch Lab B — Render Performance:**
```bash
python3 harness/run_lab.py --lab ws7-render-perf --budget 50
# Config: evolution/labs/ws7-render-perf.yaml | Metric: render_time_ms < 16ms
```

### Phase C: Layout Engine & Display Manager
Split-screen layout with focus management.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 7.C1 | Create `display_handler.rs` — DisplayHandler with layout/focus/render/info methods | Medium | Dev |
| 7.C2 | LayoutNode tree: recursive split (horizontal/vertical) with ratio, leaf = konsole_id | Medium | Dev |
| 7.C3 | Layout calculator: given screen dimensions + LayoutNode tree, compute pixel rect for each konsole | Hard | **AutoResearch** |
| 7.C4 | Border rendering: draw borders between split panes, highlight focused pane | Easy | Dev |
| 7.C5 | Register `display` service in mcpd + CLI shorthand | Easy | Dev |
| 7.C6 | Default layout on boot: Root IA (left 30%) + User console (right 70%) | Easy | Dev |

**AutoResearch Lab C — Layout Algorithm:**
```bash
python3 harness/run_lab.py --lab ws7-layout-algo --budget 30
# Config: evolution/labs/ws7-layout-algo.yaml | Metric: layout_score >= 90%
```

### Phase D: Input Routing & Keyboard
Route keyboard to focused konsole, with hotkeys for console switching.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 7.D1 | Read keyboard events from Redox input scheme (`display:input` or `input:` scheme) | Hard | Dev |
| 7.D2 | Input router: forward keystrokes to focused konsole's write buffer | Medium | Dev |
| 7.D3 | Hotkey system: Ctrl+Alt+N = switch to konsole N, Ctrl+Alt+C = create new konsole | Medium | Dev |
| 7.D4 | Focus cycling: Ctrl+Alt+Left/Right = previous/next konsole | Easy | Dev |
| 7.D5 | Shell integration: konsole write buffer → pipe to shell stdin (ion) for user konsoles | Hard | Dev |

**AutoResearch Lab D — Input Latency:**
```bash
python3 harness/run_lab.py --lab ws7-input-latency --budget 50
# Config: evolution/labs/ws7-input-latency.yaml | Metric: input_latency_ms < 50ms
```

### Phase E: Konsole Root IA & Integration
The AI supervisor's permanent console + integration with WS5 AiHandler.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 7.E1 | Auto-create Konsole 0 (Root IA) at boot, owned by "acosd", always visible | Easy | Dev |
| 7.E2 | Auto-create Konsole 1 (User) at boot, launches ion shell | Medium | Dev |
| 7.E3 | AiHandler integration: AI tool calls write status to Konsole 0 (audit trail) | Medium | Dev |
| 7.E4 | AI monitoring display: periodic system stats written to Konsole 0 (CPU, RAM, services) | Medium | Dev |
| 7.E5 | Cross-console notifications: services can send alerts to Konsole 0 | Easy | Dev |

### Phase F: Polish & Advanced Features
Optional improvements after core is working.

| Task | Description | Complexity | Mode |
|------|-------------|------------|------|
| 7.F1 | Scrollback search: `/pattern` to search through konsole history | Medium | **AutoResearch** |
| 7.F2 | Session recording: capture timestamped input/output to file for replay | Medium | Dev |
| 7.F3 | Detach/reattach: disconnect from a konsole without destroying it | Medium | Dev |
| 7.F4 | Rich rendering: markdown-like formatting (headers, bold, tables) in Root IA konsole | Hard | **AutoResearch** |
| 7.F5 | Color themes: configurable color palette via mcp://config | Easy | Dev |

**AutoResearch Lab F — Scrollback Search:**
```bash
python3 harness/run_lab.py --lab ws7-scrollback-search --budget 20
# Config: evolution/labs/ws7-scrollback-search.yaml | Metric: search_time_ms < 100ms
```

---

## Agent Team Structure

### IMPORTANT: tmux Window Management
**Quand l'orchestrateur lance un teammate via tmux, toujours créer une nouvelle fenêtre (window) avec un ratio de 50%.** Ne pas empiler les teammates dans le même pane — chaque teammate doit avoir sa propre fenêtre tmux avec un split 50% pour une lisibilité optimale.

| Agent | Model | Role | Phase |
|-------|-------|------|-------|
| impl-konsole | opus | KonsoleHandler + Cell model + ANSI parser (7.A1-A3) | A |
| impl-register-a | haiku | Register konsole service + CLI (7.A4) | A |
| impl-tests | sonnet | Host-mode unit tests (7.A5) | A |
| impl-framebuffer | opus | Framebuffer + font + renderer (7.B1-B5) | B |
| impl-display | sonnet | DisplayHandler + LayoutNode + layout calc (7.C1-C5) | C |
| impl-input | sonnet | Input routing + hotkeys + shell pipe (7.D1-D5) | D |
| impl-integration | sonnet | Root IA konsole + AiHandler integration (7.E1-E5) | E |

### Dependencies
```
Phase A (data model) ──→ Phase B (rendering) ──→ Phase C (layout)
      │                                               │
      └─────────────────────────────────────→ Phase D (input)
                                                      │
                                               Phase E (integration)
                                                      │
                                               Phase F (polish)
```

- Phase A FIRST (unblocks everything — all subsequent phases need konsole data model)
- Phase B depends on A (needs Konsole struct to render)
- Phase C depends on B (needs framebuffer to draw layout borders)
- Phase D depends on A (needs konsole write buffer) — can start in parallel with B
- Phase E depends on A + C (needs konsoles + layout working)
- Phase F is optional, after E

---

## AutoResearch Lab Definitions

Each lab is driven by the **AutoResearch framework** (`harness/autoresearch.sh`) using YAML configs in `evolution/labs/`.

**To run a lab autonomously:**
```bash
python3 harness/run_lab.py --lab ws7-ansi-parser --budget 30
```

**To run a single iteration manually:**
```bash
bash harness/autoresearch.sh ws7-ansi-parser 1
```

**What happens per iteration:**
```
┌──────────────────────────────────────────────────────────┐
│        AutoResearch Iteration (autoresearch.sh)            │
│                                                            │
│  1. BACKUP: save allowed_files from lab YAML               │
│  2. HOST TEST: cargo check + test (fast-fail, ~2s)         │
│  3. [QEMU only] INJECT + CROSS-COMPILE + BOOT (~30s)      │
│  4. EXTRACT METRIC: from test output or serial log         │
│  5. CHECK TARGET: parse_lab.py compares to target          │
│  6. ROLLBACK if regression (automatic)                     │
│  7. RECORD: evolution/results/{lab_id}.tsv + memory file   │
│  8. OUTPUT: AUTORESEARCH_RESULT:metric=VALUE,status=...    │
│                                                            │
│  Host cycle: ~2s  |  QEMU cycle: ~30s                      │
└──────────────────────────────────────────────────────────┘
```

**Results & memory:**
- TSV results: `evolution/results/{lab_id}.tsv`
- Round memory: `evolution/memory/{lab_id}_round_{N}.md`
- Lab summary: `evolution/labs/{lab_id}_summary.md` (written at end)

### Lab A — ANSI Parser Accuracy
**Config:** `evolution/labs/ws7-ansi-parser.yaml` (host-only)
**Run:** `python3 harness/run_lab.py --lab ws7-ansi-parser --budget 30`
- Metric: `test_pass_rate` (percent) — target >= 95%
- Tests ANSI CSI/SGR/OSC escape sequence parsing accuracy
- Allowed files: `src/konsole_handler.rs`, `src/lib.rs`

### Lab B — Render Performance
**Config:** `evolution/labs/ws7-render-perf.yaml` (QEMU)
**Run:** `python3 harness/run_lab.py --lab ws7-render-perf --budget 50`
- Metric: `render_time_ms` — target < 16ms (60fps)
- Tests full-screen render of 80×24 konsole via serial marker
- Allowed files: `src/konsole_handler.rs`, `src/display_handler.rs`

### Lab C — Layout Algorithm
**Config:** `evolution/labs/ws7-layout-algo.yaml` (host-only)
**Run:** `python3 harness/run_lab.py --lab ws7-layout-algo --budget 30`
- Metric: `layout_score` (percent) — target >= 90%
- Tests layout recalculation correctness (no gaps, no overlaps)
- Allowed files: `src/display_handler.rs`, `src/lib.rs`

### Lab D — Input Latency
**Config:** `evolution/labs/ws7-input-latency.yaml` (QEMU)
**Run:** `python3 harness/run_lab.py --lab ws7-input-latency --budget 50`
- Metric: `input_latency_ms` — target < 50ms
- Tests keypress-to-screen-update end-to-end latency
- Allowed files: `src/konsole_handler.rs`, `src/display_handler.rs`, `src/lib.rs`

### Lab F — Scrollback Search
**Config:** `evolution/labs/ws7-scrollback-search.yaml` (host-only)
**Run:** `python3 harness/run_lab.py --lab ws7-scrollback-search --budget 20`
- Metric: `search_time_ms` — target < 100ms
- Tests pattern search in 10K-line scrollback buffer
- Allowed files: `src/konsole_handler.rs`

---

## Success Criteria

### Must Have (Phase A-C)
- [ ] `mcp-query konsole list` → returns active konsoles
- [ ] `mcp-query konsole create` → creates a new konsole with text buffer
- [ ] `mcp-query konsole write` → writes text with ANSI color support
- [ ] `mcp-query konsole read` → returns current konsole content
- [ ] Framebuffer rendering: text visible on QEMU screen (not just serial)
- [ ] Split layout: 2+ konsoles visible simultaneously with borders
- [ ] All existing 44 mcp_scheme tests still pass
- [ ] Cross-compile succeeds, ACOS boots with Konsole service

### Should Have (Phase D-E)
- [ ] Keyboard input routes to focused konsole
- [ ] Ctrl+Alt+N hotkeys switch between konsoles
- [ ] Konsole 0 (Root IA) shows AI activity
- [ ] Konsole 1 (User) runs ion shell
- [ ] Input-to-display latency < 16ms

### Nice to Have (Phase F)
- [ ] Scrollback search < 50ms for 10K lines
- [ ] Session recording/replay
- [ ] Color themes via mcp://config

---

## Notes

1. **Framebuffer access on Redox:** `display:` scheme. Check if `fbcond` (framebuffer console daemon) is running — it may conflict. May need to replace or disable `fbcond` for custom rendering.
2. **Font embedding:** No filesystem font loading needed. Embed a CP437 8x16 bitmap font as a `const [u8; 4096]` array directly in the binary.
3. **No GPU required.** Pure software rendering to linear framebuffer. QEMU provides VGA or virtio-gpu.
4. **Shell integration:** On Redox, shells (ion) read from stdin and write to stdout. The konsole must create a pseudo-terminal pair (ptyd) and connect the shell's stdin/stdout to it.
5. **mcpd is single-threaded.** KonsoleHandler and DisplayHandler share state via `Arc<Mutex<>>`, not through router dispatch (same pattern as AiHandler).
6. **The render loop** should be event-driven (dirty flag), NOT polling. Render only when a konsole changes.

## Notes pour la prochaine session

1. **Phase A est testable sur host** — pas besoin de QEMU pour développer le data model et l'ANSI parser
2. **Phase B nécessite QEMU** — le framebuffer n'existe que dans l'environnement Redox
3. **`fbcond` est un potentiel bloqueur** — c'est le daemon framebuffer console actuel de Redox. Il faudra soit le remplacer, soit le désactiver dans acos-bare.toml
4. **Les Labs AutoResearch host-testable (A, C, F) peuvent tourner en parallèle** pendant que le dev (B, D) avance
5. **La Konsole Root IA est le vrai différenciateur** — c'est ce qui rend ACOS unique vs tmux/screen
6. **TOUJOURS appeler l'OS "ACOS"**, jamais "Redox"
