# APEX Prompt — WS4: LLM Runtime (llmd)

**Usage:** Copier tout le contenu entre les balises `---PROMPT START---` et `---PROMPT END---` et le coller après `/apex` dans Claude Code.

---PROMPT START---

Implement WS4 (LLM Runtime) for ACOS at /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os/

## Context
ACOS is an AI-native OS built on a Rust micro-kernel. WS1, WS2, and WS3 are COMPLETE:
- WS1: ACOS boots with full branding in 4s via QEMU
- WS2: `mcp:` scheme is a REAL, NATIVE kernel scheme. 100% MCP spec. Latency 436ns.
- WS3: 10 MCP services active (system, process, memory, file, file_write, file_search, log, config, echo, mcp). 44 tests. `mcp-query` CLI works. All verified in QEMU.

Current state:
- mcpd daemon at components/mcpd/ — serves all MCP services
- mcp_scheme library at components/mcp_scheme/ — ServiceHandler trait, Router
- mcp-query CLI at components/mcp_query/ — queries MCP services from ion shell
- QEMU config: x86_64, 2GB RAM, no GPU, no network (smolnetd crashes)
- Cross-compile target: x86_64-unknown-redox (Rust toolchain in Podman container)
- The Podman container has: Rust redox toolchain, GCC cross-compiler, all build deps
- ACOS kernel is Rust-based micro-kernel (no Linux compat layer, no glibc, uses relibc)

## WS4 Objective
A local LLM inference engine runs natively inside ACOS, generating tokens from prompts. Exposed as MCP service `mcp://llm/generate`.

**After WS4, this must work from inside ACOS:**
```
# From ion shell:
mcp-query llm generate "Hello, I am ACOS, an AI-native operating system."
# → Generates continuation tokens, prints response text

mcp-query llm info
# → {"model":"smollm-135m-q4","tokens_per_sec":12.5,"ram_mb":180}
```

## The Core Challenge

Cross-compiling an LLM inference engine for x86_64-unknown-redox is HARD because:
1. **No glibc** — ACOS uses relibc (ACOS libc). Most C/C++ libraries assume glibc.
2. **No GPU drivers** — CPU-only inference in QEMU (no Vulkan, no CUDA)
3. **No network** — Model weights must be baked into the disk image or loaded from file
4. **Limited RAM** — 2GB in QEMU, model + OS must fit
5. **No mmap** — relibc has limited mmap support; some inference engines rely on it

### Candidate engines (AutoResearch evaluation)

| Engine | Language | Pros | Cons | Feasibility |
|--------|----------|------|------|-------------|
| **Candle** (HuggingFace) | Rust pure | No C deps, cargo build, GGUF support | May use mmap, threading | ★★★ Best bet |
| **llama.cpp** | C++ | Mature, GGUF, optimized | C++ C++ cross-compile for ACOS is painful | ★★ Possible |
| **Burn** | Rust | Clean API, multiple backends | Young, may lack quantized model support | ★★ Possible |
| **Custom minimal** | Rust | Full control, no deps | Must implement transformer from scratch | ★ Hard |
| **ONNX Runtime** | C++ | Standard format | Heavy deps, unlikely to cross-compile | ✗ Unlikely |

### Candidate models (must fit in <1GB RAM quantized)

| Model | Params | Q4 Size | Quality | Speed (est.) |
|-------|--------|---------|---------|-------------|
| **SmolLM-135M** | 135M | ~80MB | Basic | ★★★ Fast (>20 tok/s) |
| **TinyLlama-1.1B** | 1.1B | ~600MB | Good | ★★ Moderate (~8 tok/s) |
| **Qwen2-0.5B** | 0.5B | ~300MB | Decent | ★★★ Fast (~15 tok/s) |
| **Phi-3-mini-4k** | 3.8B | ~2GB | Excellent | ✗ Too large for 2GB RAM |
| **SmolLM2-360M** | 360M | ~200MB | Decent | ★★★ Fast (~15 tok/s) |

**Recommended starting point:** SmolLM-135M with Candle (Rust pure, smallest model, fastest iteration).

## WS4 Tasks (from ROADMAP.md)

### Phase A: Engine evaluation (AutoResearch — Lab iterations with git)
4.1 Attempt cross-compile Candle for x86_64-unknown-redox
4.2 Attempt cross-compile llama.cpp for x86_64-unknown-redox (fallback)
4.3 Benchmark: tokens/second on CPU in QEMU

### Phase B: Model integration (Dev + AutoResearch)
4.4 Download and convert SmolLM-135M to GGUF/safetensors
4.5 Evaluate model candidates: SmolLM-135M, Qwen2-0.5B, SmolLM2-360M
4.6 Create `llmd` daemon — loads model, serves inference requests

### Phase C: MCP integration (Dev)
4.7 Expose inference via `mcp://llm/generate` (register as MCP service in mcpd)
4.8 Token streaming via `mcp://llm/stream` (incremental read)
4.9 Model info via `mcp://llm/info` (model name, speed, RAM usage)

### Phase D: Optimization (AutoResearch)
4.10 Optimize inference speed (target > 5 tok/s in QEMU)
4.11 Optimize RAM usage (target < 1GB for model + runtime)
4.12 Hot-swap model support (load/unload without reboot)

## Technical approach

### Architecture
```
┌─────────────────────────────────────────────┐
│  ACOS (QEMU, 2GB RAM, x86_64, CPU only)     │
│                                               │
│  ┌──────────┐    ┌──────────┐                │
│  │ mcp-query│───→│   mcpd   │                │
│  │ (client) │    │  mcp:llm │───→ LlmHandler │
│  └──────────┘    └──────────┘     │           │
│                                    ▼           │
│                              ┌──────────┐     │
│                              │  llmd    │     │
│                              │ (engine) │     │
│                              │ SmolLM   │     │
│                              │ 135M Q4  │     │
│                              └──────────┘     │
│                                               │
│  Model weights: /usr/share/llm/smollm.gguf    │
└───────────────────────────────────────────────┘
```

### Option A: LlmHandler inside mcpd (preferred if no deadlock)
The LLM handler lives inside mcpd as another ServiceHandler. Inference is synchronous (blocking — OK for small models). No separate daemon needed.

```rust
pub struct LlmHandler {
    model: CandelModel,  // Or whatever engine wins evaluation
}

impl ServiceHandler for LlmHandler {
    fn handle(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "generate" => {
                let prompt = request.params["prompt"].as_str().unwrap_or("");
                let max_tokens = request.params.get("max_tokens")
                    .and_then(|v| v.as_u64()).unwrap_or(64);
                let result = self.model.generate(prompt, max_tokens);
                JsonRpcResponse::success(request.id.clone(), json!({
                    "text": result.text,
                    "tokens_generated": result.count,
                    "tokens_per_sec": result.speed,
                }))
            }
            "info" => { /* model metadata */ }
            _ => { /* method not found */ }
        }
    }
}
```

No deadlock risk because LlmHandler doesn't call other MCP services — it only runs inference.

### Option B: Separate llmd daemon
If the model is too slow and blocks mcpd's event loop for too long, llmd runs as a separate process with its own scheme `llm:`. But this requires a second Socket::create and SchemeDaemon.

**Start with Option A. Switch to B only if blocking is a problem.**

### Cross-compilation strategy for Candle

Candle is pure Rust → `cargo build --target x86_64-unknown-redox` should work in theory.

**Potential blockers:**
1. `memmap2` crate — used for model loading. ACOS has limited mmap. Fix: use `read` instead.
2. `rayon` crate — parallel inference. ACOS threading may have issues. Fix: single-threaded mode.
3. `half` crate — f16 support. Should work (pure Rust).
4. `safetensors` crate — model format. Pure Rust, should work.
5. `tokenizers` crate — HuggingFace tokenizer. Has C deps (oniguruma). Fix: use simple tokenizer or pre-tokenize.

**AutoResearch approach:** Try to compile, fix each blocker, iterate until it works.

### Model weight injection
Model weights (.gguf or .safetensors) must be baked into the ACOS disk image:

```bash
# After cross-compile, before boot:
MOUNT_DIR="/tmp/acos_mount_$$" && mkdir -p "$MOUNT_DIR"
build/fstools/bin/redoxfs build/x86_64/acos-bare/harddrive.img "$MOUNT_DIR" &
sleep 2
mkdir -p "$MOUNT_DIR/usr/share/llm"
cp smollm-135m-q4.gguf "$MOUNT_DIR/usr/share/llm/"
cp target/x86_64-unknown-redox/release/mcpd "$MOUNT_DIR/usr/bin/mcpd"
fusermount3 -u "$MOUNT_DIR"
```

**Disk space:** The image is 196MB. SmolLM-135M Q4 is ~80MB. Plenty of room. May need to increase `filesystem_size` in acos.toml for larger models.

### Build workflow
```bash
cd /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os

# 1. Develop engine in components/llm_engine/ (test on host first)
cd components/llm_engine
cargo test  # Host tests with mock model or tiny weights

# 2. Inject into mcpd (add LlmHandler to mcp_scheme)
./scripts/inject_mcpd.sh

# 3. Cross-compile
cd redox_base
podman run --rm \
    --cap-add SYS_ADMIN --device /dev/fuse --network=host \
    --volume "$(pwd):/mnt/redox:Z" \
    --volume "$(pwd)/build/podman:/root:Z" \
    --workdir /mnt/redox/recipes/other/mcpd/source \
    redox-base bash -c '
        export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
        export RUSTUP_TOOLCHAIN=redox
        cargo build --release --target x86_64-unknown-redox --no-default-features --features redox
    '

# 4. Inject binary + model weights into image
# 5. Boot and test: mcp-query llm generate "Hello ACOS"
```

## AutoResearch loop specifications

### For engine cross-compile (Phase A, tasks 4.1-4.2)
```
CRITICAL: This is a TRUE AutoResearch lab with git-backed iterations.

FOR each engine candidate (candle, llama.cpp, burn, custom):
    1. Create git branch: git checkout -b ws4/try-{engine}
    2. Setup Cargo.toml / build scripts for the engine
    3. Attempt: cargo build --target x86_64-unknown-redox (in Podman)
    4. IF compile error:
       a. Analyze the error (missing syscall? C dep? mmap?)
       b. Attempt fix (patch dep, feature flag, fork crate)
       c. git commit -m "ws4/{engine}: fix {error} — {what was tried}"
       d. Retry from step 3 (max 10 iterations per engine)
    5. IF compile succeeds:
       a. git commit -m "ws4/{engine}: COMPILE SUCCESS"
       b. Inject into image, boot, test inference
       c. Measure tokens/sec, RAM usage
       d. git commit -m "ws4/{engine}: bench {tok/s} tok/s, {ram}MB RAM"
    6. Log to evolution/results/ws4_engine_eval.tsv:
       round  engine  status  error_or_metric  timestamp
    7. Write evolution/memory/ws4_engine_round_N.md

    IMPORTANT GIT DISCIPLINE:
    - EVERY attempt gets a commit, even failures
    - The git history IS the research log
    - Failed attempts are valuable data
    - Branch per engine, merge winner into main
    - Use descriptive commit messages: "ws4/candle: memmap2 fails on redox,
      patched to use std::fs::read instead"
```

### For model evaluation (Phase B, tasks 4.4-4.5)
```
FOR each model (SmolLM-135M, Qwen2-0.5B, SmolLM2-360M):
    1. Download quantized weights (GGUF Q4_K_M or safetensors)
    2. Host benchmark: tokens/sec on Linux (baseline)
    3. Inject into ACOS image
    4. QEMU benchmark: tokens/sec, RAM usage, load time
    5. Log to evolution/results/ws4_model_eval.tsv
    6. Write evolution/memory/ws4_model_round_N.md
    7. If any model achieves > 5 tok/s in QEMU with < 1GB RAM → SELECT it

    Selection criteria (weighted):
    - Speed in QEMU: 40% weight (must be > 5 tok/s)
    - RAM usage: 30% weight (must be < 1GB)
    - Quality: 20% weight (coherent text generation)
    - Load time: 10% weight (must be < 10s)
```

### For inference optimization (Phase D, tasks 4.10-4.11)
```
FOR iteration IN 1..10:
    1. Profile: where is time spent? (tokenization? attention? sampling?)
    2. Apply optimization (quantization, KV cache, batch prefill, SIMD)
    3. Rebuild, inject, boot, benchmark
    4. git commit with benchmark result
    5. Log to evolution/results/ws4_optim.tsv
    6. Write evolution/memory/ws4_optim_round_N.md
    7. If > 10 tok/s in QEMU → STOP (stretch goal achieved)
```

## Agent team structure

| Agent | Model | Role | Mode |
|-------|-------|------|------|
| research-engine | opus | Cross-compile evaluation: try Candle, llama.cpp, Burn (Phase A: 4.1-4.2) | AutoResearch |
| impl-llm-service | sonnet | Create LlmHandler + MCP integration once engine works (Phase C: 4.7-4.9) | Dev |
| research-models | sonnet | Model evaluation + benchmarks (Phase B: 4.4-4.5) | AutoResearch |
| research-optim | sonnet | Inference optimization loop (Phase D: 4.10-4.11) | AutoResearch |

### Dependencies
- research-engine must complete FIRST (need a compiling engine)
- impl-llm-service DEPENDS ON research-engine (needs working engine)
- research-models DEPENDS ON research-engine (needs engine to load models)
- research-optim DEPENDS ON research-models (needs selected model)
- This is SEQUENTIAL, not parallel (each phase feeds the next)

### Exception: research-engine can parallelize internally
Within Phase A, multiple engine candidates can be evaluated in parallel:
- Agent 1: try Candle
- Agent 2: try llama.cpp
- Agent 3: try Burn
First one to compile wins → move to Phase B with that engine.

## Key constraints

### relibc limitations (ACOS's libc)
- `mmap`: exists but may not support MAP_PRIVATE on all file types
- `pthread`: basic support, but thread-local storage may be limited
- `dlopen`: not available (static linking only)
- `fork`: not available (use posix_spawn or direct syscall)
- `signal`: basic support

### QEMU constraints
- 2GB RAM (configurable in acos.toml: `filesystem_size`)
- No GPU (software rendering only)
- No network (smolnetd crashes)
- Serial output for debugging (captured by qemu_runner.sh)

### Model weight distribution
- Weights must be downloaded on the host (has network)
- Injected into ACOS disk image via redoxfs mount
- Stored at `/usr/share/llm/{model_name}.{format}`
- If image size becomes an issue, increase `filesystem_size` in `redox_base/config/acos.toml`

## Success criteria
- [ ] An LLM inference engine cross-compiles for x86_64-unknown-redox
- [ ] A quantized model (< 1GB) loads successfully inside ACOS
- [ ] `mcp-query llm generate "Hello"` returns generated text
- [ ] `mcp-query llm info` returns model name, speed, RAM
- [ ] Inference speed > 5 tokens/sec in QEMU (CPU only)
- [ ] RAM usage < 1GB (model + runtime)
- [ ] Model load time < 10s
- [ ] All existing 44 mcp_scheme tests still pass
- [ ] Boot still succeeds (ACOS_BOOT_OK)
- [ ] evolution/memory/ has round entries for each AutoResearch iteration
- [ ] evolution/results/ has TSV tracking for engine eval + model eval + optim
- [ ] Git history shows every attempt (successes AND failures)

---PROMPT END---

## Notes pour la prochaine session

1. **Phase A est le gating factor** — si aucun moteur ne cross-compile, il faudra une approche alternative (WASM runtime, custom transformer minimal, ou host bridge)
2. Candle est le candidat le plus probable (Rust pur, pas de C deps)
3. Le plus petit modèle viable est SmolLM-135M (~80MB Q4) — commence par celui-là
4. La RAM QEMU est configurable : augmenter si nécessaire dans le Makefile QEMU
5. Le tokenizer HuggingFace (C++ deps) sera probablement le plus dur à cross-compiler — envisager un tokenizer BPE minimal en Rust pur
6. **Git discipline critique** : chaque tentative = un commit. Les échecs sont des données.
7. Si Phase A échoue totalement, l'alternative est WS5 Phase A (rule engine sans LLM)
