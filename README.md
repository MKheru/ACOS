# ACOS

An operating system where **everything is MCP**, and **AI Guardian is the brain**.

ACOS is an AI-first OS built on a Rust micro-kernel where every system interface -- network, files, processes, display -- is an MCP service. An AI Guardian supervises the entire system and makes decisions via local LLM. No cloud, no API keys, everything runs on your machine.

This is Phase 1 of a larger project: build an OS that AI can fully operate, so that eventually AI can determine what hardware architecture is optimal for itself. Current computing hardware hasn't fundamentally changed since its inception -- ACOS is a step toward rethinking the stack from the OS up.

Built in 23 days, solo, with AI development tools. 54 commits. From first kernel compile to 19 live MCP services with local LLM tool calling.

```
        Guardian (brain)
           |
    +------+------+
    |      |      |
   net    ai    talk
    |      |      |
  Ollama  tools  conversation
    |
  19 MCP services — all discovered at runtime, zero hardcoded
```

## status

ACOS currently runs as a **virtual OS in QEMU**. All 19 MCP services work, the AI can call tools, Guardian consults the LLM for decisions, and the full system boots and operates.

Once the feature set stabilizes, the next phase is porting ACOS to boot on real hardware.

## what it looks like

Log into ACOS, and every system resource is an MCP service you can query:

```
root:~# mcp list
19 services [live]: system, process, memory, file, file_write, file_search,
config, log, echo, net, llm, ai, guardian, konsole, display, talk, command, service, net_status

root:~# mcp-query system info
{"hostname":"acos","kernel":"ACOS-0.5.12-x86_64","memory_total":2147483648,"uptime":0}

root:~# mcp-query echo ping
{"result":"pong"}
```

The AI can use tools. Ask it something and it calls MCP services to find the answer:

```
root:~# mcp-query ai ask "What is the hostname?"
{"text":"The hostname is acos.","tool_calls_made":1}

root:~# mcp-query ai ask "How much free memory?"
{"text":"1.93 GB free out of 2.0 GB total.","tool_calls_made":1}
```

LLM runs locally via Ollama (phi4-mini for generation, qwen2.5 for tool calling):

```
root:~# mcp-query llm generate "Say hello"
{"text":"Hello! How can I assist you today?","model":"phi4-mini","finish_reason":"stop"}
```

Guardian watches the system and consults the LLM for decisions:

```
root:~# mcp-query guardian consult '{"description":"High CPU usage"}'
{"action":"monitor","ai_consulted":true,"reasoning":"CPU usage alone is not critical..."}
```

Or just chat with `mcp-talk`, the conversational interface with full tool access:

```
root:~# mcp-talk
ACOS Terminal — AI-Native Interface
Type naturally. The AI is your shell.

acos> what processes are running?

  Here is the list of running processes on ACOS:
  | PID | Name                         | Memory  |
  |-----|------------------------------|---------|
  | 21  | /scheme/initfs/bin/redoxfs   | 65 MB   |
  | 32  | /usr/bin/mcpd                | 3 MB    |
  | 38  | /usr/bin/ion                 | 4 MB    |
  ...
```

## the bigger picture

ACOS follows a simple principle, inspired by Unix's "everything is a file":

> **Everything is MCP.**

Every system interface (network, display, filesystem, processes, AI) is an MCP service accessible through the `mcp:` kernel scheme. Guardian is not a monitoring app you launch -- it's a kernel-level service that supervises all other services, always active.

This design exists for a reason: if an AI can fully control an OS through a uniform protocol, it can eventually reason about what the optimal system architecture looks like. ACOS is the software layer that makes this possible.

**Phase 1** (current): ACOS -- an OS where AI is a first-class citizen, not a guest application.
**Phase 2** (future): AI uses ACOS to reason about and design hardware architectures optimized for AI workloads.

The project also aims to be compatible with frameworks like [OpenClaw](https://github.com/openclaw/openclaw) -- giving AI assistants a native OS to operate, instead of running as apps on top of a human-centric OS.

## components

| Component | What it does | Lines |
|-----------|-------------|-------|
| **mcpd** | MCP daemon -- registers the `mcp://` kernel scheme, routes JSON-RPC to service handlers | ~400 |
| **mcp_scheme** | 19 service handlers (system, process, memory, file, net, llm, ai, guardian, konsole...) | ~8000 |
| **mcp_query** | CLI tool: `mcp-query <service> <method> [params]` | ~340 |
| **mcp_talk** | Conversational AI with tool calling and multi-turn context | ~500 |
| **acos_guardian** | Autonomous health monitor -- anomaly detection + LLM consultation | ~720 |
| **acos_mux** | Terminal multiplexer | ~4000 |
| **llm_engine** | On-device GGUF inference (SmolLM-135M) | ~600 |
| **ion builtins** | `mcp` and `guardian` shell commands for the Ion shell | ~2300 |

## quick start

You need: Linux with KVM, Podman, Ollama with `phi4-mini` + `qwen2.5:7b-instruct-q4_K_M`, ~8 GB disk, ~4 GB RAM.

```bash
# 1. Clone
git clone https://github.com/MKheru/ACOS.git && cd ACOS

# 2. Set up ACOS (downloads base kernel, applies patches, links components)
./scripts/setup-acos.sh

# 3. Build the disk image
cd base
CI=1 PODMAN_BUILD=1 REPO_BINARY=1 make all CONFIG_NAME=acos-bare

# 4. Cross-compile and inject ACOS binaries
cd .. && ./scripts/build-inject-all.sh --rebuild

# 5. Boot
qemu-system-x86_64 \
  -machine q35 -cpu host -enable-kvm -smp 4 -m 2048 \
  -vga std -serial mon:stdio \
  -drive file=base/build/x86_64/acos-bare/harddrive.img,format=raw,if=none,id=drv0 \
  -device nvme,drive=drv0,serial=ACOS \
  -nic user,model=e1000 \
  -bios /usr/share/edk2/ovmf/OVMF_CODE.fd \
  -no-reboot
```

Login: `root` / `password`. Then type `mcp list` and you're in.

## how it works

ACOS registers a kernel-level `mcp:` URL scheme. When any process opens `mcp:system`, the kernel routes the file descriptor to `mcpd`, which parses JSON-RPC and dispatches to the appropriate service handler.

```
process                    kernel                     mcpd
  |                          |                          |
  |-- open("mcp:system") --> |                          |
  |                          |-- fd to mcpd ----------> |
  |-- write(json-rpc) -----> |                          |
  |                          |-- route to handler ----> |
  |                          |                          |-- SystemHandler
  |                          |                          |   .handle()
  |<- read(response) ------- | <-- response ---------- |
```

The AI service (`mcp:ai`) sends prompts to Ollama via the net service, receives tool call requests, executes them by dispatching to other MCP services, and returns the final answer. The LLM can call `system_info`, `process_list`, `file_read`, `net_dns_resolve`, etc. -- all through the same MCP bus.

Guardian runs as a kernel-level service. It polls system metrics, detects anomalies, and consults the LLM to decide whether to `ALLOW`, `MONITOR`, or `BLOCK`.

## project structure

```
ACOS/
  mcpd/                 Core daemon workspace (Rust)
    src/main.rs           Daemon entry point — registers mcp: scheme
    mcp_scheme/           19 service handlers + router + protocol
    mcp_query/            CLI query tool
    mcp_talk/             Conversational AI interface
    acos_guardian/         System health monitor
    acos_mux/             Terminal multiplexer
    llm_engine/           On-device GGUF inference
  ion-builtins/          MCP + Guardian shell builtins for Ion
  config/                ACOS image config + build recipe
  patches/               6 kernel patches + 2 Ion patches
  docs/                  Architecture docs + session journals
  scripts/               Build, test, QEMU automation
  harness/               Evaluation harness
```

## where this stands and where it needs to go

ACOS started as a solo project, built in 23 days with the help of AI development tools. I'm not an OS developer by trade -- but in 2026, tools like Claude Code finally made it possible for someone with motivation and a clear vision to build a working prototype of something that would have been out of reach before.

The foundation is there: 19 MCP services, LLM tool calling, Guardian intelligence, a working virtual OS. But this is just a prototype, and there's a long road ahead -- real hardware boot, security hardening, multi-process Guardian, proper driver support, OpenClaw integration, and eventually the hardware design questions that motivated this whole project.

**I need help.** If the vision resonates with you -- an OS built for AI, where everything is MCP -- I'd love for you to get involved. Whether you're an OS developer, a Rust engineer, an AI researcher, or just someone curious about what computing looks like when AI is a first-class citizen: contributions, feedback, ideas, and criticism are all welcome.

The `docs/` folder contains the full build journal showing every step, every decision, and every mistake. Nothing is hidden.

Build journal highlights:

- **WS1**: Kernel + bootloader rebranding for ACOS
- **WS2-3**: MCP bus with 12 system services
- **WS4**: On-device LLM inference (SmolLM-135M GGUF)
- **WS5**: AI supervisor with tool calling
- **WS7**: Virtual console system (konsole)
- **WS9**: Guardian autonomous monitor + LLM consultation
- **WS10**: Terminal multiplexer
- **Final**: Ollama integration, 19 live services, full network stack

## requirements

- **Host**: Linux with KVM (tested on Fedora 43)
- **Build**: Podman, GNU Make, ~8 GB disk
- **LLM**: [Ollama](https://ollama.com/) with `phi4-mini` and `qwen2.5:7b-instruct-q4_K_M`
- **Runtime**: QEMU with OVMF (UEFI), ~4 GB RAM
- **Network**: QEMU user-mode networking (`-nic user,model=e1000`) routes to Ollama on host

## license

Apache 2.0. See [LICENSE](LICENSE).

Built on a micro-kernel foundation from [Redox OS](https://www.redox-os.org/) (MIT license).
