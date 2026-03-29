# ACOS - Agent-Centric Operating System

> An operating system where everything is MCP, and AI Guardian is the brain.

ACOS is a fork of [Redox OS](https://www.redox-os.org/) (Rust micro-kernel) designed for AI agents. Every system interface -- network, display, files, processes -- is an MCP service. AI Guardian supervises all services and makes intelligent decisions via local LLM (Ollama).

## Architecture

```
         Guardian (brain)
            |
    +-------+-------+
    |       |       |
   net     ai     talk
    |       |       |
  Ollama  tools  conversation
```

**19 MCP services** -- all probed at runtime, zero hardcoded.

### Components

| Component | Description |
|-----------|-------------|
| `mcpd` | MCP daemon -- kernel-level `mcp://` scheme handler with service router |
| `mcp_scheme` | Service implementations: system, process, memory, file, config, log, echo, net, llm, ai, guardian, konsole, display, talk, command, service |
| `mcp_query` | CLI tool for querying MCP services |
| `mcp_talk` | CLI conversational AI interface with tool calling |
| `acos_guardian` | Autonomous system health monitor with LLM consultation |
| `acos_mux` | AI-aware terminal multiplexer |
| `llm_engine` | On-device LLM inference (SmolLM-135M via GGUF) |
| `ion-builtins` | MCP and Guardian shell builtins for Ion (Redox shell) |

## Quick Start

### Prerequisites

- Linux host with KVM support
- Podman (for cross-compilation)
- Ollama with `phi4-mini` and `qwen2.5:7b-instruct-q4_K_M`
- ~8 GB disk, ~4 GB RAM

### Build

```bash
# 1. Clone Redox OS and apply ACOS patches
./scripts/setup-redox.sh

# 2. Build the base image
cd redox_base
CI=1 PODMAN_BUILD=1 REPO_BINARY=1 make all CONFIG_NAME=acos-bare

# 3. Cross-compile ACOS components
./scripts/build-inject-all.sh --rebuild

# 4. Boot ACOS in QEMU
qemu-system-x86_64 \
  -machine q35 -cpu host -enable-kvm -smp 4 -m 2048 \
  -vga std -serial mon:stdio \
  -drive file=redox_base/build/x86_64/acos-bare/harddrive.img,format=raw,if=none,id=drv0 \
  -device nvme,drive=drv0,serial=ACOS \
  -nic user,model=e1000 \
  -bios /usr/share/edk2/ovmf/OVMF_CODE.fd \
  -no-reboot
```

## Demo

```bash
# List all MCP services
mcp list                              # 19 services [live]

# Query services
mcp-query echo ping                   # {"result":"pong"}
mcp-query system info                 # hostname, kernel, memory, uptime
mcp-query net dns resolve example.com # 93.184.215.14

# AI with tool calling
mcp-query ai ask "What is the hostname?"
# {"text":"The hostname is acos.","tool_calls_made":1}

# LLM direct
mcp-query llm generate "Say hello"
# {"text":"Hello! How can I assist you today?","model":"phi4-mini"}

# Guardian consultation
mcp-query guardian consult '{"description":"High CPU usage"}'
# {"action":"monitor","ai_consulted":true,"reasoning":"..."}

# Conversational AI
mcp-talk
# acos> What processes are running?
# (AI calls process_list tool, formats results)
```

## Project Structure

```
ACOS/
  mcpd/              Core daemon workspace
    mcp_scheme/        MCP bus + 19 service handlers
    mcp_query/         CLI query tool
    mcp_talk/          Conversational AI tool
    acos_guardian/     Health monitor daemon
    acos_mux/          Terminal multiplexer
    llm_engine/        On-device inference
  ion-builtins/       MCP shell integration for Ion
  config/             Redox image config + recipe
  patches/            Patches for Redox OS + Ion shell
  architecture/       Design docs and session journals
  scripts/            Build, test, and QEMU automation
  harness/            Evaluation harness
```

## Development History

23 days of development, from first Redox compile to 19-service MCP bus with local LLM integration.
See `architecture/` for detailed session journals.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.

ACOS is built on [Redox OS](https://www.redox-os.org/) which is licensed under the MIT license.
