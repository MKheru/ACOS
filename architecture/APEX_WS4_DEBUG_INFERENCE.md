# APEX Prompt — WS4 Debug: Fix LLM Inference Output

**Usage:** Copier tout le contenu entre les balises `---PROMPT START---` et `---PROMPT END---` et le coller après `/apex` dans Claude Code.

---PROMPT START---

Debug and fix the SmolLM-135M inference output in ACOS at /var/home/ankheru/Documents/Projects/Karpathy_AutoResearch/projects/agent_centric_os/

## Context
ACOS is an AI-native OS built on a Rust micro-kernel. WS4 LLM Runtime is implemented:
- Pure Rust LLM inference engine (from scratch, zero external deps)
- GGUF v3 parser, Q4_K_M dequantization, full LLaMA transformer forward pass
- SmolLM-135M Q4_K_M loaded successfully (101MB, 30 layers, 576 hidden, 9 heads, 3 KV heads)
- Inference runs at **4.2 tok/s** in QEMU — forward pass executes through all 30 layers
- MCP integration works: `mcp-query llm generate "Hello"` returns JSON-RPC response

## The Problem
The model generates `<endoftext>` tokens in a loop instead of coherent text:
```
mcp-query llm generate Hello I am ACOS
→ {"text":"<endoftext>|<endoftext>|<endoftext>|...", "tokens_generated":64, "tokens_per_sec":4.2}
```

Key observations:
- 64 tokens generated WITHOUT EOS break → the generated token is NOT the eos_id
- But every token decodes to `<endoftext>` → argmax returns same bad token every time
- `mcp-query llm info` reports `"quantization":"Q8_0"` but model is Q4_K_M → possible quant type mismatch
- 4.2 tok/s proves the forward pass runs (30 layers × matmul × attention) — it's not crashing

## Root Cause Hypotheses (ranked by likelihood)

### H1: Q4_K_M dequantization is incorrect (HIGH probability)
The GGUF file is Q4_K_M quantized but `info` reports Q8_0. If tensors are dequantized with the wrong method, all matmul outputs are garbage → logits are noise → argmax returns arbitrary but consistent token.
- **Test:** Dequant a known tensor, compare values against llama.cpp or Python reference
- **Files:** `components/llm_engine/src/generate.rs` (dot_q4k, dequant_q4k_block)

### H2: GGUF tensor data offset/alignment is wrong (HIGH probability)
The GGUF data section starts at an aligned offset after metadata. If the alignment calculation is off by even 1 byte, ALL tensor data is shifted → garbage weights.
- **Test:** Compare first 16 bytes of `token_embd.weight` tensor data with a reference tool
- **Files:** `components/llm_engine/src/model.rs` (data section offset calculation)

### H3: Tokenizer vocab not loaded correctly from GGUF (MEDIUM probability)
If `tokenizer.ggml.tokens` array isn't parsed correctly, the vocab is wrong → prompt encodes to wrong IDs → model sees garbage input.
- **Test:** Encode "Hello" and compare token IDs with HuggingFace tokenizer output
- **Files:** `components/llm_engine/src/tokenizer.rs`, `model.rs` (vocab extraction)

### H4: matmul dimensions mismatch (MEDIUM probability)
If a weight tensor has shape [out_dim, in_dim] but matmul_q assumes [in_dim, out_dim] (row-major vs column-major), the dot product is computed on wrong elements.
- **Test:** Check tensor shapes from GGUF vs what matmul_q expects
- **Files:** `components/llm_engine/src/generate.rs` (matmul_q, transformer_forward)

### H5: RoPE was fixed but K heads still get double-rotated (LOW probability — already fixed)
The original RoPE bug applied rotation 3x per KV head. Fix was applied but needs verification.
- **Test:** Add assertion that K heads are only rotated once

### H6: BOS token not prepended to prompt (LOW probability)
Some models require BOS token at start. If missing, first-token attention is wrong.
- **Test:** Check if SmolLM-135M requires BOS, prepend if needed

## Debug Strategy — AutoResearch Lab

### Phase 1: Instrumentation (add debug output to host tests)
1.1 Add a host integration test that loads the REAL model file and dumps:
    - First 10 token IDs from encoding "Hello world"
    - Top-5 logits after first forward pass
    - Tensor shapes and quant types for key tensors
    - First 8 float values from dequantized `token_embd.weight` row 0
1.2 Compare these against reference values from Python/llama.cpp

### Phase 2: Reference comparison
2.1 Run SmolLM-135M Q4_K_M through llama.cpp on Linux host and capture:
    - Token IDs for "Hello world"
    - Logits after first token
    - Generated text
2.2 Run through our engine on Linux host (same model file) and compare

### Phase 3: Fix loop (AutoResearch iterations)
FOR each hypothesis (H1-H6):
    3.1 Add diagnostic code to test the hypothesis
    3.2 If confirmed: implement fix
    3.3 Test on host first (cargo test with real model)
    3.4 Cross-compile and inject into ACOS image
    3.5 Boot QEMU, test `mcp-query llm generate "Hello"`
    3.6 git commit with result
    3.7 Log to evolution/results/ws4_debug_inference.tsv

### Phase 4: Validation
4.1 Generate coherent text from at least 3 different prompts
4.2 Verify tokens_per_sec > 5 in QEMU
4.3 Verify `mcp-query llm info` shows correct quant type and model name

## Current state of files

### Core inference engine
- `components/llm_engine/src/model.rs` — GGUF v3 parser (15 tests, loads SmolLM OK)
- `components/llm_engine/src/generate.rs` — Dequant + forward pass (17 tests)
- `components/llm_engine/src/tokenizer.rs` — BPE tokenizer (loads from GGUF vocab)
- `components/llm_engine/src/lib.rs` — Public API, cfg-gated mock/real

### MCP integration
- `components/mcp_scheme/src/llm_handler.rs` — ServiceHandler for mcp://llm/*
- `components/mcp_query/src/main.rs` — CLI with llm generate/info/stream

### Build system
- Cross-compile: `--no-default-features --features redox` (disables host-test mock)
- inject_mcpd.sh syncs sources to recipe
- Podman container: redox-base with Rust redox toolchain
- Model: `/usr/share/llm/smollm.gguf` (101MB Q4_K_M) in ACOS image

### Key architecture constants (SmolLM-135M)
- 30 layers, 576 hidden_dim, 9 heads, 3 kv_heads, head_dim=64
- FFN intermediate: 1536, vocab: 49152, max_ctx: 2048
- RoPE theta: 10000, RMSNorm eps: 1e-5
- Activation: SwiGLU, Attention: GQA (3:1 head sharing)

### Reference model file
- `models/SmolLM-135M.Q4_K_M.gguf` (101MB) — downloaded from QuantFactory/SmolLM-135M-GGUF
- Also available as `smollm.gguf` inside the ACOS image

## Tools available
- `cargo test` on host with real model file (set MODEL_PATH env var)
- llama.cpp can be installed on host for reference comparison
- Python transformers + gguf libraries for reference values
- QEMU for end-to-end testing

## Success criteria
- [ ] `mcp-query llm generate "Hello"` produces coherent English text (not <endoftext>)
- [ ] `mcp-query llm info` shows correct model_name and quantization
- [ ] At least 3 different prompts produce different, meaningful output
- [ ] Inference speed > 3 tok/s in QEMU
- [ ] All existing tests still pass
- [ ] evolution/results/ has TSV tracking for each debug iteration
- [ ] Git history shows every attempt with descriptive messages

---PROMPT END---

## Notes pour la session de debug

1. **Commencer par H1/H2** — vérifier la dequantization et l'alignement des tenseurs en premier
2. **Utiliser llama.cpp comme référence** — installer sur l'hôte, faire tourner le même modèle, comparer les valeurs
3. **Le test sur l'hôte est plus rapide** — pas besoin de cross-compiler/QEMU pour chaque iteration
4. **Le modèle est à** `models/SmolLM-135M.Q4_K_M.gguf` sur l'hôte
5. **L'info retourne Q8_0 au lieu de Q4_K_M** — c'est un indice important, vérifier comment le quant type est reporté
6. **4.2 tok/s prouve que le forward pass tourne** — c'est un problème de précision/données, pas de structure
