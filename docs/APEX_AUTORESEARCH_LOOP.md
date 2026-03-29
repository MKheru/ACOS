# APEX Prompt — AutoResearch Loop Framework

**Usage:** Copier tout le contenu entre les balises `---PROMPT START---` et `---PROMPT END---` et le coller après `/apex` dans Claude Code.

---PROMPT START---

Build the AutoResearch autonomous iteration loop framework for ACOS at /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os/

## IMPORTANT: Naming Convention
**This OS is called ACOS (Agent-Centric Operating System).** Never refer to it as "Redox" in code comments, documentation, commit messages, or conversations.

## Context — What Exists Today

### Existing Infrastructure (partial, not connected)

**`evolution/loops/iterate.sh`** — Outer loop skeleton:
- Backs up source, runs evaluate.py, compares score, keeps or rollbacks
- But: must be called manually by an agent per iteration, doesn't loop
- But: doesn't modify code (expects agent to have already modified it)

**`harness/evaluate.py`** — Evaluation harness:
- Runs `cargo build` + `cargo test` on host, computes composite score
- But: hardcoded to `components/mcp_scheme` only
- But: QEMU integration = "not yet implemented" (placeholder)
- But: no lab-specific metric extraction

**`harness/qemu_runner.sh`** — QEMU headless boot test:
- Boots image, captures serial output, checks for `ACOS_BOOT_OK`
- But: cannot execute commands INSIDE ACOS (just checks boot)
- But: no network (uses `-net none`) — cannot talk to LLM proxy

**`evolution/results/*.tsv`** — Historical results from previous WS (manually recorded)

**`evolution/memory/round_*.md`** — Round memories (manually written by agent)

### What Has NEVER Worked
The full autonomous loop: **hypothesis → modify code → inject → cross-compile → boot QEMU → execute test inside ACOS → extract metric → decide → loop** has NEVER run end-to-end automatically. All previous WS "AutoResearch" rounds were manual agent sessions.

### Build & Test Workflow (proven, works)
```bash
# Host-only (fast, 2s)
cd components/mcp_scheme && cargo test --features host-test

# Cross-compile (15s)
bash scripts/inject_mcpd.sh
cd redox_base && podman run --rm \
    --cap-add SYS_ADMIN --device /dev/fuse --network=host \
    --volume "$(pwd):/mnt/redox:Z" \
    --volume "$(pwd)/build/podman:/root:Z" \
    --workdir /mnt/redox/recipes/other/mcpd/source \
    redox-base bash -c '
        export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
        export RUSTUP_TOOLCHAIN=redox
        cargo build --release --target x86_64-unknown-redox --no-default-features --features redox
    '

# Inject binary (3s)
MOUNT_DIR="/tmp/acos_mount" && mkdir -p "$MOUNT_DIR"
build/fstools/bin/redoxfs build/x86_64/acos-bare/harddrive.img "$MOUNT_DIR" &
sleep 2
cp recipes/other/mcpd/source/target/x86_64-unknown-redox/release/mcpd "$MOUNT_DIR/usr/bin/mcpd"
fusermount3 -u "$MOUNT_DIR"

# QEMU boot (4s to ACOS_BOOT_OK)
qemu-system-x86_64 -nographic -machine q35 -cpu host -enable-kvm -smp 4 -m 2048 \
    -serial stdio -drive file=build/x86_64/acos-bare/harddrive.img,format=raw,if=none,id=drv0 \
    -device nvme,drive=drv0,serial=ACOS \
    -bios /usr/share/edk2/ovmf/OVMF_CODE.fd \
    -net user,hostfwd=tcp::10022-:22 -device e1000,netdev=net0 -netdev user,id=net0
```

### Key Constraint: Running Commands Inside QEMU
The biggest missing piece. Options:
1. **Serial console automation** — Send keystrokes via QEMU monitor, read serial output
2. **SSH** — ACOS doesn't have sshd (yet)
3. **QEMU Monitor `sendkey`** — Send individual keystrokes, capture serial
4. **`-chardev` + socket** — Connect a Unix socket to QEMU serial, script reads/writes
5. **Init script injection** — Write a test script into the image that runs at boot and outputs results to serial

**Option 5 is the most reliable and simplest:**
- Mount image → write `/usr/lib/init.d/99_autotest` that executes `mcp-query` commands → unmount
- Boot QEMU headless with `-serial file:/tmp/serial.log`
- Parse serial output for metric markers: `AUTORESEARCH_METRIC:key=value`
- No network, no SSH, no keystroke emulation needed

### Key Files
```
harness/evaluate.py            — Current evaluation (to be replaced/extended)
harness/qemu_runner.sh         — Current QEMU runner (to be extended)
evolution/loops/iterate.sh     — Current iteration skeleton (to be replaced)
evolution/results/              — TSV result files
evolution/memory/               — Round memory files
scripts/inject_mcpd.sh         — Source injection into build tree
scripts/build_offline.sh       — Offline build pipeline
```

---

## Objective

Build a **complete, generic, autonomous AutoResearch loop** that can:

1. **Read a lab definition file** (YAML) specifying: metric, target, test commands, iteration budget
2. **Run host-only labs** (no QEMU): modify → compile → test → measure → loop
3. **Run QEMU labs** (full integration): modify → inject → cross-compile → inject binary → boot QEMU → execute tests inside ACOS → extract metrics → loop
4. **Be driven by an AI agent** (Claude Code) that reads results, generates hypotheses, modifies code, and calls the loop harness
5. **Record every iteration** with memory (what changed, what was the result, keep or rollback)
6. **Stop when target is met OR budget exhausted**

### The Loop Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                   AutoResearch Autonomous Loop                       │
│                                                                       │
│   ┌──────────────┐     ┌──────────────┐     ┌──────────────────┐   │
│   │  Lab Config   │────→│  AI Agent    │────→│  Code Modifier   │   │
│   │  (YAML)       │     │  (Claude)    │     │  (Edit files)    │   │
│   └──────────────┘     └──────┬───────┘     └────────┬─────────┘   │
│                               │                       │              │
│                               │ reads results         │ modifies     │
│                               │                       ▼              │
│   ┌──────────────┐     ┌──────┴───────┐     ┌──────────────────┐   │
│   │  Memory       │←────│  Evaluator   │←────│  Build Pipeline  │   │
│   │  (round_N.md) │     │  (metrics)   │     │  (inject+compile │   │
│   └──────────────┘     └──────────────┘     │   +inject image)  │   │
│                                              └────────┬─────────┘   │
│                                                       │              │
│                                              ┌────────▼─────────┐   │
│                                              │  Test Runner      │   │
│                                              │  (host or QEMU)   │   │
│                                              └──────────────────┘   │
│                                                                       │
│   CYCLE: ~2s (host-only) or ~30s (QEMU)                             │
│   BUDGET: N iterations per lab (defined in config)                    │
└─────────────────────────────────────────────────────────────────────┘
```

### How the AI Agent Drives the Loop

The loop is NOT a dumb script — the AI agent (Claude Code) IS the brain:

```
Claude Code session (launched once, runs autonomously):

1. Read lab config: evolution/labs/{lab_id}.yaml
2. Read previous round memory (if resuming)
3. FOR round = 1 to budget:
   a. THINK: analyze previous results, form hypothesis
   b. MODIFY: edit source code based on hypothesis
   c. CALL: bash harness/autoresearch.sh {lab_id} {round}
      → This script handles: host-test OR (inject → build → QEMU → extract metric)
      → Returns: AUTORESEARCH_RESULT:metric=value,status=pass|fail
   d. READ result from stdout
   e. RECORD: write evolution/memory/{lab_id}_round_{N}.md
   f. DECIDE: if metric >= target → STOP (success)
   g. DECIDE: if metric regressed → rollback (iterate.sh handles this)
4. Write final summary to evolution/labs/{lab_id}_summary.md
```

This means the AI agent runs in a SINGLE Claude Code session. The `autoresearch.sh` script is a deterministic tool it calls — the intelligence is in the agent.

---

## Implementation Plan

### Component 1: Lab Definition Format

Create `evolution/labs/` directory with YAML lab configs:

**`evolution/labs/example-host.yaml`** (host-only lab template):
```yaml
lab_id: example-host
description: "Example host-only lab for testing"
workstream: ws7
type: host  # host = no QEMU needed, qemu = full integration

# What to measure
metric:
  name: test_pass_rate
  unit: percent
  target: ">= 95"
  extract_from: harness_output  # or serial_log

# What component to modify
component: mcp_scheme
source_dir: components/mcp_scheme/src

# How to test (host mode)
host_test:
  compile: "cargo check --features host-test"
  test: "cargo test --features host-test"
  metric_command: "cargo test --features host-test 2>&1"
  metric_regex: "test result: ok\\. (\\d+) passed"

# Iteration budget
budget: 30
rollback_on_regression: true

# Files the AI agent is allowed to modify
allowed_files:
  - "src/konsole_handler.rs"
  - "src/lib.rs"
```

**`evolution/labs/example-qemu.yaml`** (QEMU lab template):
```yaml
lab_id: example-qemu
description: "Example QEMU integration lab"
workstream: ws7
type: qemu

metric:
  name: render_time_ms
  unit: ms
  target: "< 16"
  extract_from: serial_log
  marker: "AUTORESEARCH_METRIC:render_time_ms="

component: mcp_scheme
source_dir: components/mcp_scheme/src

# QEMU test: commands to run inside ACOS at boot
qemu_test:
  # Injected as /usr/lib/init.d/99_autotest
  boot_commands:
    - "mcp-query konsole create --type test --owner autotest"
    - "mcp-query konsole perf-test --id 0 --iterations 100"
  # Marker to look for in serial output
  success_marker: "AUTORESEARCH_DONE"
  timeout_seconds: 60

# Also run host tests first (fast fail)
host_test:
  compile: "cargo check --features host-test"
  test: "cargo test --features host-test"

budget: 50
rollback_on_regression: true

allowed_files:
  - "src/konsole_handler.rs"
  - "src/display_handler.rs"
```

### Component 2: `harness/autoresearch.sh` — The Universal Test Runner

This is the script the AI agent calls each iteration. It handles EVERYTHING deterministic:

```bash
#!/usr/bin/env bash
# Usage: ./harness/autoresearch.sh <lab_id> <round_number>
#
# Reads: evolution/labs/{lab_id}.yaml
# Does:
#   1. Parse lab config
#   2. Run host tests (compile + unit tests) — FAST FAIL
#   3. If type=qemu:
#      a. inject_mcpd.sh
#      b. podman cross-compile
#      c. Mount image, inject binary + autotest script
#      d. Boot QEMU headless, capture serial
#      e. Extract metric from serial output
#   4. Output: AUTORESEARCH_RESULT:metric=VALUE,status=pass|fail,round=N
#   5. If regression: rollback source from backup
#
# Exit codes:
#   0 = iteration complete (check AUTORESEARCH_RESULT for metric)
#   1 = compile failed
#   2 = host tests failed
#   3 = cross-compile failed
#   4 = QEMU boot failed
#   5 = metric extraction failed
```

**Key features:**
- Reads lab config YAML (using python3 one-liner or simple parser)
- Backs up source BEFORE running (for rollback)
- Fast-fail on host compile/test (don't waste 30s on QEMU if code doesn't compile)
- For QEMU labs: injects a custom `99_autotest` init script that runs the test commands and outputs `AUTORESEARCH_METRIC:key=value` to serial
- Parses serial log for metric markers
- Outputs standardized `AUTORESEARCH_RESULT:...` line for the AI agent to parse
- Appends to `evolution/results/{lab_id}.tsv`

### Component 3: QEMU Test Injection

For QEMU labs, the runner must inject a test script into the ACOS image:

```bash
# Mount image
MOUNT_DIR="/tmp/acos_mount" && mkdir -p "$MOUNT_DIR"
build/fstools/bin/redoxfs build/x86_64/acos-bare/harddrive.img "$MOUNT_DIR" &
sleep 2

# Inject mcpd binary
cp recipes/other/mcpd/source/target/x86_64-unknown-redox/release/mcpd "$MOUNT_DIR/usr/bin/mcpd"

# Inject autotest script (runs AFTER mcpd starts, outputs to serial)
cat > "$MOUNT_DIR/usr/lib/init.d/98_autotest" << 'TESTEOF'
requires_weak 15_mcp
# Wait for mcpd to be ready
sleep 2
# Run lab test commands (injected from YAML)
RESULT=$(mcp-query konsole perf-test --iterations 100 2>&1)
echo "AUTORESEARCH_METRIC:render_time_ms=$(echo $RESULT | grep -o '[0-9.]*')"
echo "AUTORESEARCH_DONE"
TESTEOF

# Unmount
fusermount3 -u "$MOUNT_DIR"
```

The autotest script is dynamically generated from the lab YAML `boot_commands`. It outputs metric markers to serial (captured by QEMU `-serial file:...`).

### Component 4: `harness/parse_lab.py` — Lab Config Parser

Small Python utility to parse YAML lab configs and generate:
- The autotest init script for QEMU injection
- The metric extraction regex
- The comparison logic (target parsing: `< 16`, `>= 95`, etc.)

### Component 5: Updated `evolution/loops/iterate.sh`

Rewrite to be called by `autoresearch.sh` for backup/rollback logic. Or merge into `autoresearch.sh` directly.

### Component 6: `harness/run_lab.py` — AI Agent Entry Point

This is the script that **launches the full autonomous loop**:

```bash
# Launch autonomous AutoResearch for a specific lab
# This starts a Claude Code session that runs N iterations autonomously
python3 harness/run_lab.py --lab ws7-ansi-parser --budget 30
```

It generates a Claude Code prompt and launches it:
```
You are an AutoResearch agent. Your mission: optimize {lab_id}.

Lab: {description}
Metric: {metric.name} ({metric.unit})
Target: {metric.target}
Budget: {budget} iterations
Allowed files: {allowed_files}

Previous results: {last 5 rounds from TSV}
Previous memory: {last round memory}

## Your Loop (repeat until target met or budget exhausted):

1. ANALYZE previous results. What pattern do you see?
2. HYPOTHESIZE: "If I change X, metric Y should improve because Z"
3. MODIFY: Edit the allowed files. Make ONE focused change per iteration.
4. RUN: bash harness/autoresearch.sh {lab_id} {round}
5. READ the output line: AUTORESEARCH_RESULT:metric=VALUE,status=...
6. RECORD: Write a brief note to evolution/memory/{lab_id}_round_{N}.md
7. DECIDE:
   - If metric meets target → STOP, write success summary
   - If metric improved → keep changes, continue
   - If metric regressed → changes were rolled back, try different approach
   - If budget exhausted → STOP, write summary of best result

## Rules:
- ONE change per iteration. Small, measurable, reversible.
- Always read the file before editing.
- Never skip running autoresearch.sh — no guessing if it works.
- If stuck after 3 regressions in a row, step back and rethink approach.
```

### Component 7: Pre-built Lab Configs for WS7

Create the actual lab YAML files for WS7:
- `evolution/labs/ws7-ansi-parser.yaml` (host-only)
- `evolution/labs/ws7-render-perf.yaml` (QEMU)
- `evolution/labs/ws7-layout-algo.yaml` (host-only)
- `evolution/labs/ws7-input-latency.yaml` (QEMU)
- `evolution/labs/ws7-scrollback-search.yaml` (host-only)

---

## Agent Team Structure

| Agent | Model | Role | Files Owned |
|-------|-------|------|-------------|
| impl-harness | opus | Core autoresearch.sh — universal test runner with host + QEMU modes | harness/autoresearch.sh |
| impl-qemu-inject | sonnet | QEMU test injection — dynamic autotest script generation + serial parsing | harness/qemu_inject.sh |
| impl-parser | sonnet | Lab config parser — YAML reader, target comparison, metric extraction | harness/parse_lab.py |
| impl-runner | opus | AI agent entry point — run_lab.py that generates Claude Code prompts and manages the session | harness/run_lab.py |
| impl-labs | haiku | Lab YAML configs for WS7 + example templates | evolution/labs/*.yaml |
| impl-update | haiku | Update existing iterate.sh + evaluate.py to integrate with new framework | evolution/loops/iterate.sh, harness/evaluate.py |

### Dependencies
```
impl-parser FIRST (unblocks everything — lab config format)
impl-harness DEPENDS ON impl-parser (needs to read lab configs)
impl-qemu-inject DEPENDS ON impl-parser (needs boot_commands from config)
impl-harness DEPENDS ON impl-qemu-inject (calls it for QEMU labs)
impl-runner DEPENDS ON impl-harness (orchestrates the full loop)
impl-labs DEPENDS ON impl-parser (uses the YAML format)
impl-update PARALLEL with impl-labs (independent cleanup)
```

**Spawn groups:**
- G0: impl-parser (unblocks everything)
- G1: impl-harness + impl-qemu-inject + impl-labs (parallel after parser)
- G2: impl-runner + impl-update (after harness)

---

## Success Criteria

### Must Work (end-to-end)
- [ ] `bash harness/autoresearch.sh ws7-ansi-parser 1` → runs host test, outputs AUTORESEARCH_RESULT
- [ ] `bash harness/autoresearch.sh ws7-render-perf 1` → injects, cross-compiles, boots QEMU, extracts metric from serial
- [ ] Rollback works: if metric regresses, source is restored
- [ ] Lab YAML format is generic: works for any WS, any component
- [ ] Results appended to `evolution/results/{lab_id}.tsv`
- [ ] Round memory written to `evolution/memory/{lab_id}_round_{N}.md`

### Must Work (AI agent loop)
- [ ] `python3 harness/run_lab.py --lab ws7-ansi-parser --budget 5` → launches Claude Code session that runs 5 autonomous iterations
- [ ] Agent reads previous results, modifies code, calls autoresearch.sh, reads result, decides
- [ ] Agent stops when target met OR budget exhausted
- [ ] Final summary written to `evolution/labs/{lab_id}_summary.md`

### Performance
- [ ] Host-only lab cycle: < 5s per iteration (compile + test + measure)
- [ ] QEMU lab cycle: < 45s per iteration (inject + compile + QEMU boot + test + measure)
- [ ] Serial output parsing: < 1s

---

## Critical Design Decisions

### 1. Serial-based test execution (not SSH, not sendkey)
Inject a test init script into the image that runs at boot. Output metrics to serial. Parse serial log after QEMU exits. This is:
- Deterministic (no timing-dependent keystroke simulation)
- Reliable (serial is always captured)
- Simple (no sshd, no network needed for testing)

### 2. AI agent = Claude Code in a single session
The agent runs in one long Claude Code session. It calls `autoresearch.sh` as a tool (Bash). This is simpler than launching/relaunching Claude Code per iteration. The agent maintains context across iterations within its session.

### 3. Fast-fail on host before QEMU
Every QEMU lab also runs host compile + tests first (2s). If that fails, skip the 30s QEMU cycle. This saves massive time on broken code.

### 4. One change per iteration
The AI agent is instructed to make ONE focused change. This makes it possible to attribute metric changes to specific modifications. If the agent makes 5 changes and the metric drops, it's impossible to know which one caused it.

### 5. QEMU networking for proxy tests
For labs that need the LLM proxy (AI tool calling performance), QEMU needs `-net user` with port forwarding. The autotest script must also start `llm-proxy.py` on the host before QEMU. Add a `needs_proxy: true` flag in lab config.

---

## Notes for Implementation

1. **YAML parsing in bash:** Use `python3 -c "import yaml; ..."` or a simple `grep/sed` parser for the subset of YAML we use. Don't add a heavy dependency.
2. **QEMU must exit after test:** The autotest script should call `shutdown` or the runner should kill QEMU after seeing `AUTORESEARCH_DONE` marker.
3. **Redox init ordering:** `98_autotest` runs after `15_mcp` (mcpd). Use `requires_weak 15_mcp` to ensure mcpd is up before running mcp-query commands.
4. **Serial log rotation:** Each round overwrites `/tmp/acos_serial_{lab_id}.log`. Keep the last 5 logs for debugging.
5. **Cross-compile caching:** After the first podman run, dependencies are cached. Subsequent compiles take ~5s instead of 15s. The framework should NOT clean the build cache between iterations.
6. **Rollback granularity:** Back up only `allowed_files` from the lab config, not the entire source tree. Faster and cleaner.
7. **run_lab.py can use Claude Code CLI:** `claude --print --dangerously-skip-permissions -p "You are an AutoResearch agent..."` to launch a headless Claude Code session. Check if `claude` CLI is available.

---PROMPT END---

## Notes pour la prochaine session

1. **C'est le WS le plus critique** — sans ce framework, tous les labs AutoResearch dans WS7+ restent manuels
2. **Tester d'abord avec un lab host-only** (ws7-ansi-parser) — pas besoin de QEMU pour valider le loop
3. **Le QEMU test injection est le point le plus délicat** — il faut que l'init script s'exécute APRÈS mcpd et que les métriques arrivent sur le serial
4. **Claude Code CLI** (`claude` command) est le moyen le plus simple de lancer l'agent — vérifier s'il est installé et fonctionne en mode headless
5. **Budget réaliste pour un premier test :** 5 itérations, pas 50 — valider que le loop fonctionne avant de le laisser tourner la nuit
6. **Ce framework sera réutilisé pour TOUS les futurs WS** — investissement qui se rentabilise immédiatement
