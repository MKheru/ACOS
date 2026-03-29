# APEX Prompt — WS3: System Services

**Usage:** Copier tout le contenu entre les balises `---PROMPT START---` et `---PROMPT END---` et le coller après `/apex` dans Claude Code.

---PROMPT START---

Implement WS3 (System Services) for ACOS at /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os/

## Context
ACOS is an AI-native OS based on Redox OS (Rust micro-kernel). WS1 and WS2 are COMPLETE:
- WS1: OS boots with full ACOS branding in 4s via QEMU
- WS2: `mcp:` scheme is a REAL, NATIVE Redox scheme. 100% MCP spec conformity (9/9 methods). Latency 436ns. 792K binary. 24 tests pass.

Current state:
- mcpd daemon at components/mcpd/ — SchemeDaemon + SchemeSync, blocking event loop, privilege drop
- mcp_scheme library at components/mcp_scheme/ — McpScheme with Router, ServiceHandler trait, 24 tests
- Existing handlers: EchoHandler (echo, ping), SystemHandler (info, list_services), McpHandler (initialize, tools/list, tools/call, resources/list, resources/read, prompts/list, prompts/get, notifications/initialized)
- Dynamic service registration API: Router::register_service(), Router::unregister_service()
- Security hardening: max 1MiB request buffer, max 1024 handles, graceful error recovery
- Build workflow: inject_mcpd.sh → Podman cross-compile (15s) → redoxfs inject → QEMU boot (4s)
- The harness at harness/evaluate.py measures compile success, test pass rate, and boot time
- Architecture docs in architecture/ROADMAP.md (WS3 section has 15 tasks)
- Build journal in architecture/BUILD_JOURNAL.md
- Evolution memory in evolution/memory/ (WS2 rounds: conformity 1-8, perf 1-8)

## WS3 Objective
Replace Redox's native daemon functionality with MCP services inside mcpd. Every system interaction goes through `mcp://`.

After WS3, this must work from inside ACOS:
```
# From ion shell inside ACOS:
cat mcp://system/info <<< '{"jsonrpc":"2.0","method":"info","id":1}'
# → {"jsonrpc":"2.0","result":{"hostname":"acos","kernel":"acos-kernel","uptime":42},"id":1}

cat mcp://system/processes <<< '{"jsonrpc":"2.0","method":"list","id":2}'
# → {"jsonrpc":"2.0","result":{"processes":[{"pid":1,"name":"init"},{"pid":5,"name":"mcpd"},...]},"id":2}

cat mcp://file/read <<< '{"jsonrpc":"2.0","method":"read","params":{"path":"/etc/hostname"},"id":3}'
# → {"jsonrpc":"2.0","result":{"content":"acos\n","size":5},"id":3}
```

## WS3 Tasks (from ROADMAP.md)

### Phase A: Core system services (Dev — implement and test)
3.1 Service `system/info` — hostname, uptime, kernel version, memory stats
3.2 Service `system/processes` — list processes (read /scheme/sys/ or equivalent)
3.3 Service `system/memory` — allocation stats, memory pressure
3.10 Service `log` — structured centralized logging via MCP
3.11 Service `config` — system configuration key-value store

### Phase B: File services (Dev — implement and test)
3.4 Service `file/read` — read file content via `mcp://file/path/to/file`
3.5 Service `file/write` — write file content
3.6 Service `file/search` — content/metadata search (AutoResearch: optimize search algorithm)

### Phase C: Integration and hardening
3.9 Service `console` — terminal I/O via MCP (read input, write output)
3.15 Benchmark: latency of each service vs native Redox equivalent (AutoResearch)

### Deferred (requires network stack)
3.7, 3.8, 3.13, 3.14 — net/http, net/dns, remove ipcd, remove smolnetd

## Technical constraints

### ServiceHandler trait (from handler.rs)
```rust
pub trait ServiceHandler: Send + Sync {
    fn handle(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse;
    fn list_methods(&self) -> Vec<&str>;
}
```

Each new service is a struct implementing `ServiceHandler`, registered in McpScheme::new() via `router.register("service_name", Box::new(Handler))`.

### Accessing Redox system info from userspace
On Redox OS, system info is available through schemes:
- `/scheme/sys/` — kernel info (context, cpu, memory, uname, etc.)
- `/proc/` or `sys:context/` — process list
- Standard file I/O works (open/read/write/close on paths)

Since mcpd runs on Redox, these handlers can use `std::fs::read_to_string("/scheme/sys/uname")` etc. to gather system data.

**CRITICAL:** The handlers must work in TWO modes:
1. **Redox mode** (`#[cfg(target_os = "redox")]`): Read from real `/scheme/sys/`, `/proc/`, etc.
2. **Host-test mode** (`#[cfg(feature = "host-test")]`): Return mock/static data for Linux testing

### Cross-compilation workflow (same as WS2)
```bash
cd /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os
./scripts/inject_mcpd.sh
cd redox_base
podman run --rm \
    --cap-add SYS_ADMIN --device /dev/fuse --network=host \
    --volume "$(pwd):/mnt/redox:Z" \
    --volume "$(pwd)/build/podman:/root:Z" \
    --workdir /mnt/redox/recipes/other/mcpd/source \
    redox-base bash -c '
        export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
        export RUSTUP_TOOLCHAIN=redox
        cargo build --release --target x86_64-unknown-redox --features redox
    '
```

### Image injection + boot test (same as WS2)
```bash
MOUNT_DIR="/tmp/acos_mount_$$" && mkdir -p "$MOUNT_DIR"
build/fstools/bin/redoxfs build/x86_64/acos-bare/harddrive.img "$MOUNT_DIR" &
sleep 2
cp recipes/other/mcpd/source/target/x86_64-unknown-redox/release/mcpd "$MOUNT_DIR/usr/bin/mcpd"
fusermount3 -u "$MOUNT_DIR"
```

```bash
cd /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os
./harness/qemu_runner.sh redox_base/build/x86_64/acos-bare/harddrive.img 60
```

### Key Cargo.toml note
The `daemon` crate path is relative from the INJECTED location:
```toml
daemon = { path = "../../../core/base/source/daemon" }
```
And `syscall` must be aliased:
```toml
syscall = { version = "0.7", optional = true, package = "redox_syscall" }
```

## AutoResearch loop specification

### For file/search service (Phase B, task 3.6)
```
FOR iteration IN 1..15:
    1. Implement or improve search algorithm in handler
    2. Add test for the search functionality
    3. cargo test --features host-test (must pass)
    4. Measure: search time for pattern in 100 files (mock)
    5. Log to evolution/results/file_search.tsv
    6. Write evolution/memory/ws3_search_round_N.md
    7. If latency < 1ms for 100-file search → STOP
```

### For benchmark service (Phase C, task 3.15)
```
FOR each service (system/info, system/processes, system/memory, file/read, file/write):
    1. Benchmark via criterion: request → response roundtrip
    2. Compare latency vs native Redox equivalent (direct read from /scheme/sys/)
    3. Target: MCP overhead < 10μs per service call
    4. Log to evolution/results/ws3_service_bench.tsv
    5. Write evolution/memory/ws3_bench_round_N.md
```

## Agent team structure

| Agent | Model | Role | Mode |
|-------|-------|------|------|
| impl-system-services | sonnet | Implement system/info, system/processes, system/memory (Phase A: 3.1-3.3) | Dev |
| impl-file-services | sonnet | Implement file/read, file/write, file/search (Phase B: 3.4-3.6) | Dev |
| impl-support-services | sonnet | Implement log, config, console (Phase A/C: 3.10, 3.11, 3.9) | Dev |
| bench-services | sonnet | Benchmark all services, AutoResearch loop (Phase C: 3.15) | Dev + AutoResearch |

### Dependencies
- impl-system-services and impl-file-services can start in PARALLEL (different handler files)
- impl-support-services can start in PARALLEL (different handlers)
- bench-services DEPENDS ON all three impl agents (needs services to exist)
- Cross-compile + boot test: AFTER all impl agents complete

## Key reference code (agents must read these)

### Current mcp_scheme structure
```
components/mcp_scheme/src/lib.rs       — McpScheme, open/read/write/close, connection management
components/mcp_scheme/src/protocol.rs  — JsonRpcRequest/Response, McpPath
components/mcp_scheme/src/router.rs    — Router with FxHashMap, dispatches to ServiceHandler
components/mcp_scheme/src/handler.rs   — ServiceHandler trait, EchoHandler, SystemHandler, McpHandler
```

### How to add a new service (pattern to follow)
1. Create a struct implementing ServiceHandler in handler.rs (or a new file)
2. Implement `handle(&self, path, request) -> JsonRpcResponse` with method dispatch
3. Implement `list_methods(&self) -> Vec<&str>`
4. Register in McpScheme::new(): `router.register("service_name", Box::new(MyHandler))`
5. Add tests in lib.rs using the existing `service_roundtrip()` helper

### Existing test pattern (from lib.rs)
```rust
fn service_roundtrip(scheme: &mut McpScheme, service: &str, request_json: &str) -> serde_json::Value {
    let handle = scheme.open(service.as_bytes()).unwrap();
    scheme.write(handle, request_json.as_bytes()).unwrap();
    let mut buf = vec![0u8; 65536];
    let n = scheme.read(handle, &mut buf).unwrap();
    let response: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
    scheme.close(handle).unwrap();
    response
}
```

### Redox /scheme/sys/ files (available in Redox, must be mocked for host-test)
```
/scheme/sys/uname     → "ACOS 0.1.0"
/scheme/sys/context    → process list (one per line: pid name status)
/scheme/sys/memory     → memory stats (total, used, free)
/scheme/sys/cpu        → CPU info
/scheme/sys/uptime     → uptime in seconds
```

## Success criteria
- [ ] `system/info` returns hostname, kernel, uptime, memory (tested on host + QEMU)
- [ ] `system/processes` returns process list (tested on host with mock, QEMU with real data)
- [ ] `system/memory` returns memory stats
- [ ] `file/read` reads arbitrary files via MCP
- [ ] `file/write` writes arbitrary files via MCP
- [ ] `file/search` finds content in files (AutoResearch optimized)
- [ ] `log` service accepts structured log entries
- [ ] `config` service stores/retrieves key-value pairs
- [ ] All existing 24 tests still pass
- [ ] At least 15 new tests (2+ per service)
- [ ] Cross-compile succeeds with new services
- [ ] Boot still succeeds (ACOS_BOOT_OK in < 5s)
- [ ] Service latency < 10μs overhead vs native Redox calls (bench)
- [ ] evolution/memory/ has round entries for search and bench iterations
- [ ] evolution/results/ has TSV tracking for all AutoResearch loops

---PROMPT END---

## Notes pour la prochaine session

1. Assure-toi d'être dans le bon répertoire : `cd /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os`
2. Vérifie que les 24 tests passent : `cd components/mcp_scheme && cargo test --features host-test`
3. Vérifie que l'image boot : `./harness/qemu_runner.sh redox_base/build/x86_64/acos-bare/harddrive.img`
4. Lance : `/apex` puis colle tout le contenu entre les balises START/END
5. APEX va orchestrer ~4 agents en parallèle, puis un cross-compile + boot test
6. La tâche `console` (3.9) est la plus complexe — elle nécessite de comprendre comment ptyd/getty fonctionnent sur Redox
7. Les services réseau (3.7, 3.8, 3.13, 3.14) sont DÉFÉRÉS — ACOS n'a pas de carte réseau dans QEMU actuellement (smolnetd panic)
