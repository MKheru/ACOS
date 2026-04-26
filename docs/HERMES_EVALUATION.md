# Hermes Agent — Evaluation for ACOS

**Date:** 2026-04-26
**Subject:** [NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent) as a candidate component for ACOS Guardian / agent layer.
**Verdict:** **Do not embed. Adopt patterns. Optionally use as external client.**

---

## Context

Hermes Agent is a Python-based, self-improving AI agent built by Nous Research, positioned as a successor to OpenClaw (which appears in our roadmap as a Phase 2 reference). It is mature, MIT-licensed, and well-architected. The question this document answers: should ACOS embed Hermes as the user-facing or system-level agent layer, instead of building Guardian and the agent stack from scratch?

The short answer is no — Hermes lives in a different conceptual layer than Guardian, and in a different language than our kernel. But its architectural patterns are the best open-source blueprint we have found for several Guardian sub-systems, and we will adopt them deliberately.

---

## What Hermes is

| Aspect | Details |
|---|---|
| Language | Python 3.11 (uv-based env) |
| Size | ~26,000 lines of Python core (`run_agent.py` + `cli.py` + `hermes_cli/main.py`) |
| License | MIT |
| Conceptual layer | **User-facing personal assistant** — runs on top of an OS, not as one |
| Entry points | CLI, gateway (Telegram/Discord/Slack/etc.), ACP (VS Code/Zed), batch, API server |
| Memory | 4 layers: prompt-resident `MEMORY.md`+`USER.md` (3.5k chars max) / SQLite+FTS5 sessions / procedural skills (`agentskills.io` standard) / optional Honcho user model |
| Self-improvement | Periodic-nudge loop, autonomous skill creation after non-trivial tasks (5+ tool calls, error recovery, user correction), patch-over-edit for skill updates |
| Extensibility | Plugin hooks: `pre_llm_call`, `post_llm_call`, `on_session_start`, `on_session_end`. Plugin registries for memory providers and context engines. |
| MCP | Native MCP client (~2,200 LOC in `tools/mcp_tool.py`) — Process / HTTP / WebSocket transports |
| Deployment model | $5 VPS, GPU cluster, or serverless (Daytona/Modal/Singularity) — designed to hibernate when idle |

---

## Why Hermes cannot be the core of ACOS

Four structural reasons, in order of severity:

### 1. Wrong conceptual layer

ACOS Guardian is a **system supervisor** — its concerns are resource allocation, anomaly detection, service health, and policy enforcement. It is a kernel-adjacent component.

Hermes is a **user-facing assistant** — its concerns are conversational tasks, multi-platform messaging, scheduled automations, and skill accumulation. It is an end-user application that happens to be agentic.

These are not two implementations of the same thing. They are two adjacent layers in the agent stack. Substituting one for the other does not simplify the architecture; it conflates layers and creates downstream confusion.

### 2. Python in a Rust microkernel OS

Embedding ~26k lines of Python plus the CPython 3.11 runtime, plus uv, plus the entire pip dependency surface, into a Rust microkernel project means ACOS effectively becomes a minimal Linux distribution that hosts Hermes. The thesis "OS Rust micro-kernel, LLM as kernel" cannot survive that.

A Rust port is a complete rewrite (estimated 6+ months) with no benefit over building Guardian in Rust from the start.

### 3. Foreign deployment model

Hermes targets `$5 VPS / GPU cluster / serverless infrastructure` with cloud-side hibernation. ACOS targets bare-metal hardware as its endgame. Daytona, Modal, and Singularity backends have no meaning in an OS that boots on physical hardware.

### 4. Functional surface area is parasitic for a kernel

18 messaging adapters (Telegram, Discord, Slack, WhatsApp, Signal, Email, SMS, Matrix, Mattermost, DingTalk, Feishu, WeCom, WeChat, BlueBubbles, QQBot, Home Assistant, Webhook, API Server), 6 terminal backends, Atropos RL training, Hugging Face / OpenRouter / Nebius / Anthropic / OpenAI / etc. provider integrations, ACP for IDEs.

Rich and useful for a personal assistant. Pure noise for an OS kernel.

---

## What Hermes is genuinely worth stealing

The following patterns are the clearest open-source reference we have seen for problems Guardian must solve. They will be reproduced in Rust, in Guardian, with attribution.

| Pattern | Why it matters for ACOS |
|---|---|
| **4-layer memory architecture** (always-on / FTS5 sessions / procedural skills / optional user model) | Cleanest separation we have seen between *episodic* (what happened) and *procedural* (how to do things) memory — directly applicable to `mcp://guardian` |
| **Periodic nudge mechanism** | Curated memory does not grow unbounded; the agent itself decides what is worth keeping |
| **`agentskills.io` open standard** for procedural skills | Skills become portable across compatible agents — adopt as Guardian skill format |
| **Patch-over-edit preference** for skill updates | Token-efficient, correctness-preserving (full rewrites risk breaking working logic) |
| **Lossy summarization with lineage preserved in SQLite** | Long-context handling without losing the ability to trace earlier turns |
| **Plugin hooks** (`pre/post_llm_call`, `on_session_start/end`) | Clean extensibility surface without forking the agent core |
| **Self-registering tool registry** at import time | Validates the dynamic-discovery approach we already use in mcpd |
| **MCP client architecture** (Process / HTTP / WebSocket transports) | Reference implementation we will read, not embed |

---

## How Hermes can legitimately exist alongside ACOS

Three roles, by ascending integration cost:

### Role A — Hermes as external MCP client of ACOS (zero cost, immediate)

```
You on Telegram/CLI ── Hermes (host) ──MCP-over-WS──► ACOS mcpd (19 services)
```

Hermes runs on your laptop or VPS. It connects to ACOS the same way it connects to any MCP server. You get cross-platform conversation, persistent memory, autonomous skills — none of which ACOS has to implement itself. This is "user-facing agent" territory, where Guardian is not supposed to operate.

### Role B — Hermes as architectural reference for Guardian (high ROI, no coupling)

Read the relevant subsystems of Hermes (memory layers, nudge loop, skills, MCP integration), port the patterns to Rust inside Guardian. Adopt `agentskills.io` as the Guardian skill format. This work is captured as task **11.12** in WS11.

### Role C — Hermes as optional Python user-agent on ACOS (Phase 3+, hardware)

Once ACOS boots on real hardware with Python available (relibc-Python work on Redox is ongoing), running a Hermes instance *next to* the Rust Guardian is plausible. Hermes handles the user, Guardian handles the system, they communicate via MCP. Not before Phase 3 of the current roadmap.

---

## Decision

1. **Do not embed Hermes** in ACOS. Wrong layer, wrong language, wrong scope.
2. **Adopt Hermes patterns** as the architectural reference for Guardian (WS9 design ADR + WS11 task 11.12).
3. **Adopt `agentskills.io`** as the Guardian procedural skill format.
4. **Validate Role A** (Hermes as external MCP client) once `mcp://gui` exposes a WSS endpoint (WS13) — 30-minute manual test, no code commitment.
5. **Defer Role C** to Phase 3 roadmap.

---

## Open questions

- Will `agentskills.io` evolve in a way that is compatible with our Rust `LlmBackend` trait, or will we need a translation shim?
- If Hermes ports to native Rust (Nous Research has not announced this), should we revisit Role A → embedded?
- Does the periodic-nudge pattern map cleanly onto a kernel-level Guardian whose "session" is the lifetime of the OS, not a conversation? This needs design work.

---

## References

- [Hermes Agent GitHub](https://github.com/NousResearch/hermes-agent)
- [Hermes Agent Documentation](https://hermes-agent.nousresearch.com/docs)
- [Architecture page](https://hermes-agent.nousresearch.com/docs/developer-guide/architecture)
- [agentskills.io standard](https://agentskills.io)
- ACOS roadmap WS9 (Guardian), WS11 (LLM Runtime Rust-Natif), WS13 (Web GUI)
