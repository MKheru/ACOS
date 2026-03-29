# ACOS

An operating system where **everything is MCP**, and **AI Guardian is the brain**.

ACOS is a Rust micro-kernel OS (fork of [Redox OS](https://www.redox-os.org/)) where every system interface -- network, files, processes, display -- is an MCP service. An AI Guardian watches over the system and makes decisions via local LLM. No cloud, no API keys, everything runs on your machine.

Built in 23 days. 54 commits. From first kernel compile to 19 live MCP services with local LLM tool calling.

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
  | PID | Name                  | Memory  |
  |-----|-----------------------|---------|
  | 21  | /scheme/initfs/bin/redoxfs | 65 MB |
  | 32  | /usr/bin/mcpd         | 3 MB    |
  | 38  | /usr/bin/ion          | 4 MB    |
  ...
```

## components

| Component | What it does | Lines |
|-----------|-------------|-------|
| **mcpd** | MCP daemon — registers the `mcp://` kernel scheme, routes JSON-RPC to service handlers | ~400 |
| **mcp_scheme** | 19 service handlers (system, process, memory, file, net, llm, ai, guardian, konsole...) | ~8000 |
| **mcp_query** | CLI tool: `mcp-query <service> <method> [params]` | ~340 |
| **mcp_talk** | Conversational AI with tool calling and multi-turn context | ~500 |
| **acos_guardian** | Autonomous health monitor — anomaly detection + LLM consultation | ~720 |
| **acos_mux** | Terminal multiplexer (forked from emux) | ~4000 |
| **llm_engine** | On-device GGUF inference (SmolLM-135M) | ~600 |
| **ion builtins** | `mcp` and `guardian` shell commands for the Ion shell | ~2300 |

## quick start

You need: Linux with KVM, Podman, Ollama with `phi4-mini` + `qwen2.5:7b-instruct-q4_K_M`, ~8 GB disk, ~4 GB RAM.

```bash
# 1. Clone
git clone https://github.com/MKheru/ACOS.git && cd ACOS

# 2. Setup Redox OS base + apply ACOS patches
./scripts/setup-redox.sh

# 3. Build the disk image
cd redox_base
CI=1 PODMAN_BUILD=1 REPO_BINARY=1 make all CONFIG_NAME=acos-bare

# 4. Cross-compile and inject ACOS binaries
cd .. && ./scripts/build-inject-all.sh --rebuild

# 5. Boot
qemu-system-x86_64 \
  -machine q35 -cpu host -enable-kvm -smp 4 -m 2048 \
  -vga std -serial mon:stdio \
  -drive file=redox_base/build/x86_64/acos-bare/harddrive.img,format=raw,if=none,id=drv0 \
  -device nvme,drive=drv0,serial=ACOS \
  -nic user,model=e1000 \
  -bios /usr/share/edk2/ovmf/OVMF_CODE.fd \
  -no-reboot
```

Login: `root` / `password`. Then type `mcp list` and you're in.

## how it works

ACOS registers a kernel-level `mcp:` URL scheme via Redox's scheme system. When any process opens `mcp:system`, the kernel routes the file descriptor to `mcpd`, which parses JSON-RPC and dispatches to the appropriate service handler.

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

The AI service (`mcp:ai`) sends prompts to Ollama via the net service, receives tool call requests, executes them by dispatching to other MCP services, and returns the final answer. This means the LLM can call `system_info`, `process_list`, `file_read`, `net_dns_resolve`, etc. — all through the same MCP bus.

Guardian runs as a background monitor. It polls system metrics, detects anomalies, and optionally consults the LLM to decide whether to `ALLOW`, `MONITOR`, or `BLOCK`.

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
  config/                acos-bare.toml + mcpd recipe
  patches/               6 Redox patches + 2 Ion patches
  docs/                  Architecture docs + session journals
  scripts/               Build, test, QEMU automation
  harness/               Evaluation harness
```

## development history

See `docs/` for the full journal. Highlights:

- **WS1**: Rebranded Redox → ACOS (kernel, bootloader, login)
- **WS2-3**: Built MCP bus with 12 system services
- **WS4**: On-device LLM inference (SmolLM-135M GGUF)
- **WS5**: AI supervisor with tool calling
- **WS7**: Virtual console system (konsole)
- **WS9**: Guardian autonomous monitor + LLM consultation
- **WS10**: Terminal multiplexer (acos-mux)
- **Final**: Ollama integration (phi4-mini + qwen2.5), 19 live services, network via QEMU user-mode

## requirements

- **Host**: Linux with KVM (tested on Fedora 43)
- **Build**: Podman, GNU Make, ~8 GB disk
- **LLM**: [Ollama](https://ollama.com/) with `phi4-mini` and `qwen2.5:7b-instruct-q4_K_M`
- **Runtime**: QEMU with OVMF (UEFI), ~4 GB RAM
- **Network**: QEMU user-mode networking (`-nic user,model=e1000`) routes to Ollama on host

## license

Apache 2.0. See [LICENSE](LICENSE).

Built on [Redox OS](https://www.redox-os.org/) (MIT license).
