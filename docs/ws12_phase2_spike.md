# WS12.M9 — Phase 2 prerequisite tracker

## Tracker

Phase 2 is gated by GitHub issue [MKheru/ACOS#2](https://github.com/MKheru/ACOS/issues/2).

This document is the WS12 handoff note for the first Rust-native LLM runtime spike. It does not implement mistral.rs yet; it records the exact condition that unlocks the spike and the commands/examples to run once the tracker is closed.

## Unlock condition

Start the Phase 2 spike only after `MKheru/ACOS#2` is closed.

Rationale: issue #2 tracks deterministic image rebuild / artifact versioning. WS12 Phase 2 needs a reproducible build path before adding a Rust-native inference runtime to the image, otherwise failures cannot be attributed cleanly to either image drift or LLM runtime integration.

## First validation after #2 closes

From the repository root:

```bash
cargo check --manifest-path mcpd/Cargo.toml --target x86_64-unknown-redox -p llm-runtime-api
```

Expected outcome: `llm-runtime-api` cross-checks for Redox without pulling host-only runtime dependencies into the API crate.

If this fails, fix the API crate boundary before adding any backend implementation.

## Minimal mistral.rs example for the spike

The first backend spike should live outside the production handler path until the Redox cross-check above is green. The target is a minimal example proving that the WS12 API can wrap mistral.rs without leaking backend-specific types through `LlmBackend`.

Sketch:

```rust
use llm_runtime_api::{InferenceBudget, InferenceRequest, LlmBackend};

async fn smoke<B: LlmBackend>(backend: &B) {
    let request = InferenceRequest::text("Explain ACOS in one sentence")
        .with_budget(InferenceBudget::default_dev());

    let response = backend.infer(request).await.expect("mistral.rs smoke infer");
    assert!(!response.text.trim().is_empty());
}
```

Acceptance for the later spike:

- model loading is isolated behind a backend module;
- `llm-runtime-api` remains backend-agnostic;
- no hardcoded model name is introduced in `mcp_scheme` handlers;
- prompt and tool-output redaction hooks remain mandatory before observability export.

## Non-goals for WS12.M9

- no mistral.rs dependency is added;
- no runtime backend is implemented;
- no MCP handler behavior changes;
- no image recipe is modified.
