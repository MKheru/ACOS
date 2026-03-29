# APEX Prompt — WS2: MCP Bus Implementation

**Usage:** Copier tout le contenu entre les balises `---PROMPT START---` et `---PROMPT END---` et le coller après `/apex` dans Claude Code.

---PROMPT START---

Implement WS2 (MCP Bus) for ACOS at /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os/

## Context
ACOS is an AI-native OS based on Redox OS (Rust micro-kernel). WS1 is COMPLETE — the OS boots with full ACOS branding in 4 seconds via QEMU. Zero user-visible "Redox" strings remain.

Current state:
- mcpd daemon exists at components/mcpd/ (cross-compiled for Redox, 727K static ELF)
- mcpd currently runs in "Linux test mode" (stdin/stdout) because it doesn't use redox_scheme crate
- mcp_scheme library exists at components/mcp_scheme/ (JSON-RPC 2.0, router, handlers, 9 tests, score baseline 399)
- Build workflow: compile in Podman → inject into image → boot QEMU (20s per iteration)
- The harness at harness/evaluate.py measures compile success, test pass rate, and boot time
- Architecture docs in architecture/ROADMAP.md (WS2 section has 11 tasks)
- Build journal in architecture/BUILD_JOURNAL.md (all build issues and solutions)
- Evolution memory in evolution/memory/ (rounds 14-17)
- .config has REPO_BINARY=0 (compile from source, no Redox server dependency)

## WS2 Objective
Make the `mcp:` scheme a REAL, NATIVE Redox scheme that any process can open/read/write.

After WS2, this must work from inside ACOS:
```
# From ion shell inside ACOS:
cat mcp://echo <<< '{"jsonrpc":"2.0","method":"ping","id":1}'
# Should return: {"jsonrpc":"2.0","result":"pong","id":1}
```

## WS2 Tasks (from ROADMAP.md)

### Phase A: Make it work (Dev classique — implement once, test, done)
2.1 Register `mcp:` via Socket::create("mcp") in mcpd using redox_scheme crate
2.2 Implement open("mcp://service/resource") → dispatch to handler via McpScheme
2.3 Implement write() → send JSON-RPC request to the MCP router
2.4 Implement read() → receive JSON-RPC response
2.5 Integration test from ion shell inside ACOS: `cat mcp://echo`
2.10 Dynamic service registry (services can register/unregister at runtime)

### Phase B: MCP spec conformity (AutoResearch — iterate until 100%)
2.6 Implement MCP standard methods: initialize, tools/list, resources/list, prompts/list
    → AutoResearch metric: % of MCP spec methods implemented (target 100%)
    → Each iteration: add a method → test → measure conformity → keep if improved

### Phase C: Performance optimization (AutoResearch — iterate for best score)
2.7 Optimize IPC latency (target < 10μs round-trip)
2.8 Optimize throughput (target > 100K msg/s)
2.9 Support 100+ concurrent MCP connections
2.11 Optional binary protocol (MessagePack instead of JSON for hot paths)
    → AutoResearch metrics: latency (μs), throughput (msg/s)
    → Each iteration: modify code → benchmark → measure → keep/rollback

## Technical constraints

### Cross-compilation
```bash
cd /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os/redox_base
podman run --rm \
    --cap-add SYS_ADMIN --device /dev/fuse --network=host \
    --volume "$(pwd):/mnt/redox:Z" \
    --volume "$(pwd)/build/podman:/root:Z" \
    --workdir /mnt/redox/recipes/other/mcpd/source \
    redox-base bash -c '
        export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
        export RUSTUP_TOOLCHAIN=redox
        cargo build --release --target x86_64-unknown-redox
    '
```

### Injection into image
```bash
MOUNT_DIR="/tmp/acos_mount_$$" && mkdir -p "$MOUNT_DIR"
build/fstools/bin/redoxfs build/x86_64/acos-bare/harddrive.img "$MOUNT_DIR" &
sleep 2
cp recipes/other/mcpd/source/target/x86_64-unknown-redox/release/mcpd "$MOUNT_DIR/usr/bin/mcpd"
printf 'requires_weak 00_base\nnowait mcpd\n' > "$MOUNT_DIR/usr/lib/init.d/15_mcp"
printf 'echo ACOS_BOOT_OK\n' > "$MOUNT_DIR/usr/lib/init.d/99_acos_ready"
fusermount3 -u "$MOUNT_DIR"
```

### Source injection (before cross-compile)
```bash
./scripts/inject_mcpd.sh  # copies components/mcpd and components/mcp_scheme into redox_base/recipes/other/mcpd/source/
```

### QEMU boot test
```bash
cd /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os
./harness/qemu_runner.sh redox_base/build/x86_64/acos-bare/harddrive.img 60
```

### Redox scheme API (from sysroot)
- `redox_scheme::Socket::<V2>::create("mcp")` — register the scheme with the kernel
- `socket.next_request(SignalBehavior::Restart)` — get next request from kernel
- `RequestKind::Call(call)` — a scheme call (open/read/write/close/etc.)
- `call.handle_scheme_mut(&mut scheme)` — dispatch to SchemeMut trait implementation
- `socket.write_responses(&[response], SignalBehavior::Restart)` — send response back
- `redox_daemon::Daemon::new(|daemon| { ... daemon.ready() ... })` — daemonize and signal readiness

### Key Cargo.toml dependencies for Redox mode
```toml
[target.'cfg(target_os = "redox")'.dependencies]
redox_scheme = "0.11"
redox_daemon = "0.1"
libredox = "0.1"
```

### mcpd must bridge between redox_scheme and mcp_scheme
The redox_scheme crate provides the SchemeMut trait with methods:
- open(path, flags, uid, gid) → Result<usize>  (return handle ID)
- read(id, buf) → Result<usize>
- write(id, buf) → Result<usize>
- close(id) → Result<usize>
- fpath(id, buf) → Result<usize>

Our McpScheme already has equivalent methods. The mcpd daemon must:
1. Implement SchemeMut for a wrapper struct that delegates to McpScheme
2. Call daemon.ready() after Socket::create("mcp")
3. Run the main event loop: next_request → handle → write_response

## AutoResearch loop specification

### For conformity agent (Phase B)
```
FOR iteration IN 1..20:
    1. Add next unimplemented MCP method to mcp_scheme/src/handler.rs
    2. Add test in mcp_scheme/src/lib.rs
    3. cargo test --features host-test (must pass)
    4. Count implemented methods / total MCP methods = conformity %
    5. Log to evolution/results/mcp_conformity.tsv
    6. Write evolution/memory/ws2_conformity_round_N.md
    7. If all methods done → STOP
```

MCP standard methods to implement:
- initialize (handshake)
- tools/list (list available tools)
- tools/call (execute a tool)
- resources/list (list available resources)
- resources/read (read a resource)
- prompts/list (list available prompts)
- prompts/get (get a prompt template)
- notifications/initialized (client ready signal)
- ping (already done)

### For performance agent (Phase C)
```
FOR iteration IN 1..30:
    1. Modify mcp_scheme code (try: buffer sizes, allocation strategy, serialization, etc.)
    2. cargo test --features host-test (must still pass)
    3. cargo bench --features host-test (measure latency + throughput)
    4. Parse criterion output for timing
    5. score = (1000 / latency_us) * (throughput / 1000)
    6. IF score > previous_best → KEEP, update results.tsv
    7. IF score <= previous_best → ROLLBACK (git checkout)
    8. Write evolution/memory/ws2_perf_round_N.md with what was tried and result
```

Benchmark file: components/mcp_scheme/benches/ipc_latency.rs (already exists)

## Agent team structure

| Agent | Model | Role | Mode |
|-------|-------|------|------|
| explorer-redox-api | sonnet | Research redox_scheme/redox_daemon API from ipcd and randd source code | Read-only |
| impl-mcpd-native | sonnet | Implement real Redox scheme in mcpd (Phase A: tasks 2.1-2.5) | Dev |
| impl-mcp-spec | sonnet | Add MCP standard methods (Phase B: task 2.6) — AutoResearch loop on conformity | Dev + AutoResearch |
| impl-mcp-perf | sonnet | Optimize latency/throughput (Phase C: tasks 2.7-2.9) — AutoResearch loop on benchmarks | Dev + AutoResearch |

### Dependencies
- impl-mcpd-native DEPENDS ON explorer-redox-api (needs API knowledge)
- impl-mcp-spec can start in PARALLEL (works on mcp_scheme lib, not mcpd)
- impl-mcp-perf DEPENDS ON impl-mcp-spec (needs methods to exist before optimizing)

## Key reference code (agents must read these)

### ipcd (real Redox scheme daemon — the pattern to follow)
```
recipes/core/base/source/ipcd/src/main.rs
- Uses redox_daemon::Daemon::new()
- Creates Socket::<V2>::create("chan") and Socket::<V2>::create("shm")
- Main loop: next_request → handle_scheme_block_mut → write_response
- Uses event::EventQueue for multiplexing
```

### randd (simpler example)
```
recipes/core/base/source/randd/src/main.rs
- Socket::<V2>::create("rand")
- Implements SchemeMut trait directly
- Simpler event loop: next_request → handle_scheme_mut → write_responses
```

### Current mcpd
```
components/mcpd/src/main.rs
- Has placeholder #[cfg(feature = "redox")] mod redox_daemon
- Linux test mode works (stdin/stdout JSON-RPC)
- Needs real Redox implementation
```

### Current mcp_scheme
```
components/mcp_scheme/src/lib.rs     — McpScheme struct with open/read/write/close
components/mcp_scheme/src/protocol.rs — JsonRpcRequest/Response types
components/mcp_scheme/src/router.rs   — Router dispatches to handlers
components/mcp_scheme/src/handler.rs  — ServiceHandler trait, EchoHandler, SystemHandler
```

## Success criteria
- [ ] `mcp:` scheme registered and visible in ACOS (check /scheme/ directory)
- [ ] `cat mcp://echo` works from ion shell in QEMU
- [ ] mcpd prints "mcp: scheme registered" in boot log (not "Linux test mode")
- [ ] All 9 existing tests still pass
- [ ] MCP conformity ≥ 80% (8+ standard methods implemented)
- [ ] IPC latency < 50μs (host benchmark)
- [ ] Boot still succeeds (ACOS_BOOT_OK in < 5s)
- [ ] evolution/memory/ has round entries for each AutoResearch iteration
- [ ] evolution/results/ has TSV tracking for conformity and performance

---PROMPT END---

## Notes pour la prochaine session

1. Assure-toi d'être dans le bon répertoire : `cd /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os`
2. Vérifie que le Podman container existe : `ls redox_base/build/container.tag`
3. Vérifie que l'image boot : `./harness/qemu_runner.sh redox_base/build/x86_64/acos-bare/harddrive.img`
4. Lance : `/apex` puis colle tout le contenu entre les balises START/END
5. APEX va orchestrer ~4 agents en parallèle avec des boucles AutoResearch autonomes
