# APEX Prompt — WS9B: Split Terminal — tmux + Guardian TUI fonctionnel

Implement WS9B (Split Terminal) for ACOS

## IMPORTANT: Naming Convention
**This OS is called ACOS (Agent-Centric Operating System).** Never refer to it as "Redox" in code comments, documentation, commit messages, or conversations.

## Context — What's Already Built

### WS9 Status (partially working)
- **GuardianHandler** (mcp:guardian) — 16th MCP service, 5 methods, 327 tests ✓
- **acos-guardian binary** — exists but TUI split is BROKEN (blocks terminal)
- **mcp-talk** — works standalone, AI chat via Gemini ✓
- **Boot** — ACOS boots, mcpd starts with 16 services including guardian ✓
- **Branding** — ACOS branding mostly done, some "Redox" remnants in login binary

### What DOESN'T work
- acos-guardian TUI blocks the terminal when launched
- No split screen (mcp-talk left / guardian right)
- Init scripts conflict (mcp-talk and acos-guardian fight for terminal)
- Metrics show 0 in guardian (mcp_call response parsing broken)

### Root Cause Analysis
The fundamental problem: there is NO terminal multiplexer in ACOS. We tried to fake a split with ANSI escape codes in acos-guardian but it doesn't work because:
1. No proper PTY management
2. No subprocess terminal isolation
3. Both processes fight for stdin/stdout
4. fbcond renders a single text terminal, no native split

### The Solution: tmux
tmux is a proper terminal multiplexer that:
- Creates virtual terminals via PTY
- Splits the screen using real PTY + ANSI rendering
- Manages keyboard input routing between panes
- tmux 3.4 source is already in `recipes/wip/terminal/tmux/`

### Architecture After WS9B
```
┌────────────────────────────────────────────────────────────┐
│  QEMU Framebuffer (fbcond)                                  │
│  ┌──────────────────────────┬──────────────────────────┐   │
│  │ tmux pane 0 (left)       │ tmux pane 1 (right)      │   │
│  │                          │                           │   │
│  │ ACOS Terminal            │ ╔════════════════════╗    │   │
│  │ ═══════════════          │ ║   ACOS GUARDIAN    ║    │   │
│  │                          │ ║ ● Health: NOMINAL  ║    │   │
│  │ acos> hello              │ ╠════════════════════╣    │   │
│  │ AI: Hello! How can I     │ ║ Procs:  14 running ║    │   │
│  │ help you today?          │ ║ Memory: 156/2048   ║    │   │
│  │                          │ ║ Uptime: 4m 23s     ║    │   │
│  │ acos> _                  │ ║ Svcs:   16 active  ║    │   │
│  │                          │ ╠════════════════════╣    │   │
│  │                          │ ║ ✓ All checks OK    ║    │   │
│  │                          │ ╚════════════════════╝    │   │
│  └──────────────────────────┴──────────────────────────┘   │
└────────────────────────────────────────────────────────────┘
```

### Terminal Rendering Pipeline
```
vesad (display.vesa://)     ← VESA framebuffer driver
    ↑
fbcond (fbcon://)           ← text console, ANSI terminal emulation
    ↑
ptyd (pty://)               ← pseudo-terminal daemon (AVAILABLE)
    ↑
tmux                        ← terminal multiplexer (TO ADD)
    ├── pane 0: mcp-talk    ← AI conversational terminal
    └── pane 1: acos-guardian ← system monitor (simple output, no TUI hacks)
```

### Key Files
```
redox_base/recipes/wip/terminal/tmux/    — tmux recipe (WIP, needs completion)
components/acos_guardian/src/main.rs      — guardian binary (needs revert to simple mode)
components/mcp_talk/src/main.rs           — AI terminal (works)
components/mcpd/src/main.rs               — MCP daemon (works, 16 services)
components/mcp_scheme/src/guardian_handler.rs — guardian service (works, 327 tests)
redox_base/config/acos-bare.toml          — boot config + init scripts
scripts/inject_mcpd.sh                    — source sync + recipe update
```

### Build & Test Workflow
```bash
# 1. Host test (fast)
cd components/mcp_scheme && cargo test --features host-test

# 2. Cross-compile
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

# 3. Inject into image
MOUNT_DIR="/tmp/acos_mount" && mkdir -p "$MOUNT_DIR"
build/fstools/bin/redoxfs build/x86_64/acos-bare/harddrive.img "$MOUNT_DIR" &
sleep 3
cp <binaries> "$MOUNT_DIR/usr/bin/"
sync && sleep 1 && fusermount3 -u "$MOUNT_DIR"

# 4. Boot QEMU (GRAPHICAL mode for split screen)
qemu-system-x86_64 \
  -enable-kvm -cpu host -m 2048 -smp 4 \
  -bios /usr/share/OVMF/OVMF_CODE.fd \
  -machine q35 -vga std \
  -drive file=build/x86_64/acos-bare/harddrive.img,format=raw,if=none,id=drv0 \
  -device nvme,drive=drv0,serial=NVME_SERIAL \
  -device e1000,netdev=net0 -netdev user,id=net0
```

### Critical Build Lessons
- **`--no-default-features --features redox`** for mcpd cross-compile
- **DO NOT use `make r.mcpd`** — TUI blocks in pipe
- **`make qemu` rebuilds the image** and erases injected binaries — use direct QEMU command
- **`-vga std`** is required to see the framebuffer (not `-vga none -nographic`)
- **mcpd response format**: `{"result": {"text": "<json-string>"}}` — parse the `text` field
- **Use `gio trash` not `rm -rf`** — rm -rf is blocked by hooks

---

## WS9B Objective

Get a **working split terminal** in ACOS where:
- **LEFT pane**: mcp-talk (AI conversational terminal)
- **RIGHT pane**: acos-guardian (system monitoring dashboard)
- User can type in the left pane, guardian updates every 30s on the right
- Boot directly into this split view (no manual login required)

---

## WS9B Phases & Tasks

### Phase 1: Stabilize (unblock the system)

| Task | Description | Complexity |
|------|-------------|------------|
| 9B.1 | **Revert acos-guardian to simple mode** — Remove ALL TUI/split code. Guardian becomes a simple background daemon that prints monitoring data to stdout every 30s (not a TUI). Keep mcp_call, poll_metrics, detect_anomalies, format output as simple text lines. NO ANSI cursor positioning, NO screen clearing, NO subprocess management. | Easy |
| 9B.2 | **Fix acos-guardian mcp_call response parsing** — Parse mcpd's `{"result": {"text": "<json>"}}` format correctly. The `text` field contains a JSON string that needs double-parsing. | Easy |
| 9B.3 | **Fix init scripts** — Remove `20_guardian` and `25_talk` init scripts from acos-bare.toml. These cause conflicts. After tmux is working, guardian and mcp-talk will be launched BY tmux, not by init directly. Keep only `15_mcp` for mcpd. | Easy |
| 9B.4 | **Test basic boot** — QEMU boots, mcpd starts with 16 services, login works, `mcp-talk` works when launched manually, `acos-guardian` works when launched manually (prints monitoring to stdout), no blocking. | Easy |

### Phase 2: Port tmux to ACOS

| Task | Description | Complexity |
|------|-------------|------------|
| 9B.5 | **Research tmux WIP recipe** — Read `recipes/wip/terminal/tmux/recipe.toml`. Understand what's there, what's missing. Check dependencies (libevent, ncurses). Check if libevent and ncurses are already in the Redox recipes. | Medium |
| 9B.6 | **Complete tmux recipe** — Get tmux 3.4 to compile for x86_64-unknown-redox. This may require: patching tmux source for Redox compatibility, ensuring libevent/ncurses are available, creating proper recipe.toml with dependencies. Alternative: if tmux is too complex, try **dvtm** (4000 LOC, simpler deps) or **mtm** (1000 LOC, minimal). | Hard |
| 9B.7 | **Test tmux in ACOS** — Inject tmux binary into image. Boot QEMU. Login. Run `tmux`. Verify: split-pane works (`Ctrl-B %`), both panes accept input, can run different commands in each pane. | Medium |

### Phase 3: Integrate Guardian + mcp-talk via tmux

| Task | Description | Complexity |
|------|-------------|------------|
| 9B.8 | **Create acos-session script** — A shell script `/usr/bin/acos-session` that: (1) starts tmux with 2 panes (vertical split 50/50), (2) left pane runs `mcp-talk`, (3) right pane runs `acos-guardian`, (4) focuses left pane for user input. Script content: `tmux new-session -d -s acos "mcp-talk" \; split-window -h "acos-guardian" \; select-pane -t 0 \; attach` | Easy |
| 9B.9 | **Auto-launch acos-session at boot** — Modify init or login config so that after login, instead of dropping to ion shell, the system launches `acos-session`. This could be done via: (a) setting tmux as the login shell, (b) adding acos-session to /etc/profile, (c) modifying the getty/login flow, or (d) an init.d script that runs after login. | Medium |
| 9B.10 | **Polish guardian output for right pane** — acos-guardian output should be optimized for a ~40-col terminal pane. Use simple text with ANSI colors (no box-drawing if terminal doesn't support it). Refresh display with clear + reprint every 30s. Show: health status, process count, memory, uptime, services, recent activity log. | Easy |

### Phase 4: Branding & Polish

| Task | Description | Complexity |
|------|-------------|------------|
| 9B.11 | **Remove ALL "Redox" from user-visible text** — Patch /etc/issue, /etc/motd in the image directly (the recipe override doesn't work because userutils installs after). Also check the login binary for hardcoded "Redox OS" and "Welcome to Redox OS!" strings — patch the binary with sed if needed, or override the files post-install. | Medium |
| 9B.12 | **Boot banner** — Update mcpd banner to show WS9B with tmux mention. | Easy |
| 9B.13 | **Final QEMU test** — Boot ACOS. System auto-launches into split view. Left: mcp-talk with `acos>` prompt. Right: guardian showing real metrics. User can chat with AI on the left. Guardian updates every 30s on the right. Screenshot the result. | Easy |

### Phase 5 (optional): tmux alternative if port fails

If tmux is too complex to port (libevent/ncurses issues), fallback plan:

| Task | Description | Complexity |
|------|-------------|------------|
| 9B.F1 | **Write acos-mux** — A minimal Rust terminal multiplexer (~500 lines) that uses Redox's `pty://` scheme. Opens 2 PTYs, spawns mcp-talk on one and acos-guardian on the other, reads both outputs, renders side-by-side using ANSI cursor positioning, forwards keyboard to active pane. This is essentially dvtm-lite in Rust, purpose-built for ACOS. | Hard |

---

## Dependencies

```
Phase 1 (stabilize) → Phase 2 (tmux port) → Phase 3 (integration) → Phase 4 (polish)
                                    │
                                    └──→ Phase 5 (fallback if tmux fails)
```

- Phase 1 is BLOCKING — must unblock the system first
- Phase 2 is the critical path — tmux port
- Phase 3 depends on Phase 2 (needs working tmux)
- Phase 4 is independent polish
- Phase 5 is ONLY if Phase 2 fails

## Agent Team Structure

| Agent | Model | Role | Phase |
|-------|-------|------|-------|
| impl-stabilize | sonnet | Revert guardian + fix init + test boot (9B.1-9B.4) | 1 |
| impl-tmux-port | opus | Research + complete tmux recipe + test (9B.5-9B.7) | 2 |
| impl-integration | sonnet | acos-session script + auto-launch + polish output (9B.8-9B.10) | 3 |
| impl-branding | haiku | Remove Redox remnants + banner update (9B.11-9B.12) | 4 |

## Success Criteria

### MUST HAVE
- [ ] ACOS boots without blocking
- [ ] `mcp-talk` works standalone (AI chat)
- [ ] `acos-guardian` works standalone (prints monitoring to stdout)
- [ ] tmux (or equivalent multiplexer) is available in ACOS
- [ ] Split screen: mcp-talk LEFT, guardian RIGHT
- [ ] User can type in mcp-talk pane while guardian updates
- [ ] Guardian shows REAL metrics (process count, memory, uptime, services)
- [ ] No "Redox" visible anywhere to the user

### SHOULD HAVE
- [ ] Auto-launch into split view at boot (no manual tmux command)
- [ ] Guardian detects anomalies and shows warnings
- [ ] Clean transitions (no screen garbage when refreshing)

### NICE TO HAVE
- [ ] Keyboard shortcut to switch between panes
- [ ] Guardian sends alerts to mcp-talk pane

## Notes

1. **tmux depends on libevent + ncurses** — check if these are in Redox recipes already
2. **If tmux port is too hard**, write a minimal Rust multiplexer using `pty://` scheme
3. **ALWAYS test in QEMU graphical mode** (`-vga std`, NOT `-nographic`)
4. **The image rebuild problem**: `make qemu` erases injected binaries. Always use direct QEMU command for testing.
5. **mcpd response format**: result.text contains a JSON string — double-parse it
6. **PTY support exists**: `ptyd` registers `pty://` scheme, both Ion and fbcond use it

