# Prochaine Session — Repo Public ACOS

## Objectif

Créer le repo public `ACOS` sur GitHub (MKheru/ACOS) à partir de `projects/agent_centric_os/`.

## Étape 1 : Extraire l'historique ACOS

Extract ACOS history from the main repository

Alternative si filter-repo ne garde pas assez d'historique :
```bash
git subtree split -P projects/agent_centric_os/ -b acos-public
```

## Étape 2 : Nettoyer le contenu

### À garder
```
architecture/           ← Toute la doc (BUILD_JOURNAL, SESSION_*, ROADMAP, etc.)
components/             ← mcpd source, mcp_query, mcp_talk, llm_engine
scripts/                ← acos_qemu.py, build-inject-all.sh, qemu-verify-ion.py
harness/                ← qemu_runner.sh, qemu_inject.sh
redox_base/config/      ← acos-bare.toml uniquement
redox_base/recipes/other/mcpd/  ← notre code mcpd
redox_base/recipes/core/ion/source/src/lib/builtins/{mcp,guardian,mod}.rs
```

### À exclure
```
.claude/                ← Config Claude Code (privé)
evolution/              ← Labs AutoResearch (privé — process interne)
redox_base/build/       ← Artefacts de build (4 GB)
redox_base/prefix/      ← Toolchain cross-compile
redox_base/target/      ← Cargo cache
**/target/              ← Tous les build artifacts
*.bak*                  ← Backups
**/.claude/             ← Configs Claude dans les sous-projets
```

### .gitignore pour le repo public
```
redox_base/build/
redox_base/prefix/
**/target/
*.bak*
.claude/
.env
```

## Étape 3 : README.md

Structure du README :

```markdown
# ACOS — Agent-Centric Operating System

> An operating system where everything is MCP, and AI Guardian is the brain.

## Vision

ACOS is a fork of Redox OS designed for AI agents. Every system interface
(network, display, files, processes) is an MCP service. AI Guardian supervises
all services and makes intelligent decisions via local LLM (Ollama).

## Architecture

        Guardian (brain)
           ↓
    ┌──────┼──────┐
    net    ai    talk
    ↓      ↓      ↓
  Ollama  tools  conversation

19 MCP services — all probed at runtime, zero hardcoded.

## Quick Start (5 steps)

1. Clone: git clone + submodule init
2. Build image: CI=1 PODMAN_BUILD=1 REPO_BINARY=1 make all CONFIG_NAME=acos-bare
3. Cross-compile: podman run ... cargo build --release --target x86_64-unknown-redox
4. Inject: redoxfs mount → cp binaries → unmount
5. Boot: qemu-system-x86_64 ... -nic user,model=e1000

## Demo

mcp list                         → 19 services [live]
mcp call echo.ping               → {"result":"pong"}
mcp net dns resolve example.com  → 104.18.26.120
guardian state                   → {"status":"nominal",...}
mcp-talk                         → AI chat with tool calling

## Requirements

- Linux host with KVM
- Podman (for cross-compilation)
- Ollama with phi4-mini + qwen2.5 (for LLM)
- ~8 GB disk, ~4 GB RAM

## Project History

23 days of development, 141 commits, from first Redox compile to
19-service MCP bus with local LLM integration.

See architecture/ for detailed session journals.
```

## Étape 4 : Créer le repo GitHub

```bash
gh repo create MKheru/ACOS --public --description "Agent-Centric Operating System — Everything is MCP, Guardian is the Brain"
git remote add github git@github.com:MKheru/ACOS.git
git push github main
```

## Étape 5 : Vérifier

- [ ] Historique visible (141 commits ou filtré ACOS only)
- [ ] Pas de secrets/API keys
- [ ] Pas de .claude/ ni evolution/labs
- [ ] README clair avec Quick Start
- [ ] redox_base/build/ exclu (.gitignore)
- [ ] Taille repo < 100 MB (sans build artifacts)

## Questions à résoudre

1. **Submodule Redox** : garder comme submodule (propre mais complexe pour les users) ou copier les fichiers modifiés uniquement ?
2. **Nom du repo** : `ACOS` ou `agent-centric-os` ?
3. **License** : MIT comme Redox ? Custom ?
4. **ion source** : c'est un fork du repo Redox ion — comment le gérer ? Submodule vers notre fork ou copie inline ?
