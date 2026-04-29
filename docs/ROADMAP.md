# ACOS — Roadmap Stratégique & Plan de Développement

*"The last OS was designed for humans clicking icons. The next one will be designed for AI agents reasoning about the world."*

---

## Pourquoi ACOS existe

Chaque OS aujourd'hui — Linux, Windows, macOS — a été conçu **avant l'IA**. Ils traitent l'IA comme une application de plus, enfermée dans des conteneurs, communiquant via des pipes texte et des API HTTP bricolées par-dessus 50 ans de dette technique POSIX.

**ACOS renverse le paradigme :** l'IA n'est pas une app. L'IA EST le système d'exploitation. Le hardware, les fichiers, le réseau, l'interface — tout est exposé comme des ressources sémantiques que l'IA comprend et orchestre nativement, via un protocole standard (MCP) intégré au noyau.

### Ce qui existe déjà (et pourquoi c'est insuffisant)

| Projet | Approche | Limitation |
|---|---|---|
| **AIOS** (Rutgers, 5.4k★) | Framework Python, "LLM as kernel" | Pas un vrai OS — tourne SUR Linux. Overhead Python. Pas de sécurité kernel. |
| **LLMBasedOS** | Infrastructure middleware | Couche d'abstraction, pas un OS. Toujours Linux en dessous. |
| **MCP** (Anthropic → Linux Foundation) | Protocole app-level | Standard de communication, pas d'intégration kernel. |
| **Redox OS** | Micro-noyau Rust pur | OS généraliste, aucune orientation IA. |

### Ce qu'ACOS apporte de nouveau

**ACOS est le premier OS où MCP est un protocole kernel**, pas un middleware. Le bus IPC du noyau parle nativement JSON-RPC/MCP. Un agent IA n'a pas besoin de "lancer un shell et parser du texte" — il ouvre `mcp://system/processes` comme on ouvre un fichier, et reçoit une réponse structurée.

**Positionnement unique :**
```
                    Application-level         Kernel-level
                    ──────────────────       ──────────────
AI Framework    │   AIOS, LangChain    │                   │
                │   (Python sur Linux) │                   │
────────────────┼──────────────────────┼───────────────────┤
MCP Protocol    │   Claude, ChatGPT    │   ★ ACOS ★        │
                │   (API middleware)    │   (MCP natif      │
                │                      │    dans le noyau)  │
────────────────┼──────────────────────┼───────────────────┤
Micro-kernel    │                      │   Redox OS        │
Rust            │                      │   (généraliste)   │
```

---

## Structure des Workstreams

Le développement est organisé en **10 workstreams** indépendants, chacun avec ses branches AutoResearch.

```
ACOS Development Tree
│
├── WS1: Kernel Identity (rebrand + isolation du build)          ✅ COMPLETE
├── WS2: MCP Bus (scheme natif, routing, protocol)               ✅ COMPLETE
├── WS3: System Services (process, file, net via MCP)            ✅ COMPLETE
├── WS4: LLM Runtime (Gemini proxy + SmolLM backup)             ✅ COMPLETE
├── WS5: AI Supervisor (orchestration, tool calls)               ✅ COMPLETE
├── WS6: Developer Experience (SDK, docs, tooling)               ⏸ Deferred
├── WS7: Konsole — Multi-Console Natif & Display Manager         ✅ COMPLETE
├── WS8: Human Interface (mcp-talk, terminal IA conversationnel) ✅ COMPLETE
├── WS9: AI Guardian — Autonomous System Monitor                 ← NEXT
└── WS10: Rich Interface (Servo, voix, dashboard, thèmes)       📋 Planned
```

---

## WS1: Kernel Identity & Build Independence ✅ COMPLETE (2026-03-22)

**Objectif :** ACOS est un projet autonome, pas un "mod Redox". Build reproductible, zéro dépendance réseau.

**Résultat :** ACOS boot en 4s via QEMU. Zéro strings "Redox" visibles. Branding complet (kernel, bootloader, login, os-release).

### Tâches

| # | Tâche | Complexité | Mode | Statut |
|---|---|---|---|---|
| 1.1 | Remplacer "Redox" par "ACOS" dans tous les messages kernel (boot, panic, logs) | Facile | Dev | ✅ Done |
| 1.2 | Modifier `os-release`, hostname, login banner | Facile | Dev | ✅ Done |
| 1.3 | Forker le repo kernel Redox en local | Moyen | Dev | ⏸ Deferred |
| 1.4 | Forker relibc en local | Moyen | Dev | ⏸ Deferred |
| 1.5 | Créer `build_offline.sh` — compilation 100% locale sans REPO_BINARY | Moyen | Dev | ✅ Done (inject workflow) |
| 1.6 | Remplacer le registry Redox par un registry local | Moyen | Dev | ⏸ Deferred |
| 1.7 | Automatiser le build CI | Moyen | Dev | ⏸ Deferred |
| 1.8 | Publier les images ACOS en release GitHub | Facile | Dev | ⏸ Deferred |

**Critère de merge :** Le build complet fonctionne sans aucune connexion réseau après le clone initial.

---

## WS2: MCP Bus — Le Cœur d'ACOS ✅ COMPLETE (2026-03-22)

**Objectif :** Le scheme `mcp:` est un citoyen de première classe dans le kernel. Chaque service s'enregistre et communique via MCP.

**Résultat :** `mcp:` enregistré comme vrai scheme Redox natif. Conformité MCP 100% (9/9 méthodes). Latence 436ns. Binary 792K. Boot OK.

### Tâches

| # | Tâche | Complexité | Mode | Statut |
|---|---|---|---|---|
| 2.1 | Enregistrer `mcp:` via `Socket::create("mcp")` dans mcpd | Moyen | Dev | ✅ Done — SchemeSync + SchemeDaemon pattern |
| 2.2 | Implémenter `open("mcp://service/resource")` → dispatch | Moyen | Dev | ✅ Done — scheme_bridge.rs openat() |
| 2.3 | Implémenter `write()` → envoi JSON-RPC request | Moyen | Dev | ✅ Done — scheme_bridge.rs write() |
| 2.4 | Implémenter `read()` → réception JSON-RPC response | Moyen | Dev | ✅ Done — scheme_bridge.rs read() |
| 2.5 | Tester depuis ion : `cat mcp://echo` | Moyen | Dev | ✅ Done — boot QEMU + ACOS_BOOT_OK |
| 2.6 | Conformité MCP spec : 9/9 méthodes | Dur | **AutoResearch** | ✅ Done — 100% (8 rounds) |
| 2.7 | Optimiser la latence IPC (< 10μs) | Dur | **AutoResearch** | ✅ Done — 436ns (8 rounds, FxHashMap + zero-copy) |
| 2.8 | Optimiser le throughput (> 100K msg/s) | Dur | **AutoResearch** | ⏸ Deferred (host bench only, needs QEMU bench) |
| 2.9 | Support multi-clients (100+ connexions) | Moyen | **AutoResearch** | ✅ Done — max 1024 handles, hardened |
| 2.10 | Registre de services dynamique | Moyen | Dev | ✅ Done — Router::register/unregister API |
| 2.11 | Protocole binaire (MessagePack) | Dur | **AutoResearch** | ⏸ Deferred (JSON perf sufficient at 436ns) |

**Critère de merge :** ✅ Atteint — latence 436ns << 50μs target.

**Métriques finales :**
- Latence round-trip : **436ns** (target < 10μs ✅)
- MCP conformité : **100%** (9/9 méthodes ✅)
- Sécurité : max buffer 1MiB, max 1024 handles, graceful error recovery
- Binary : 792K static ELF, cross-compiled x86_64-unknown-redox
- 24 tests unitaires passent

### Build issues résolus durant WS2
- `redox-scheme` (pas `redox_scheme`) est le nom crate correct sur crates.io
- `redox_syscall` crate a `[lib] name = "syscall"` → utiliser `syscall = { package = "redox_syscall" }` dans Cargo.toml
- `daemon` crate path: relatif depuis injected location (`../../../core/base/source/daemon`)
- `SchemeSync` trait import nécessaire pour `on_close` dans mcpd
- `Error::new()` attend `i32` en v0.7 (pas `usize`)

---

## WS3: System Services — Remplacer le Userspace Redox ✅ COMPLETE (2026-03-22)

**Objectif :** Chaque daemon Redox est remplacé par un service MCP dans mcpd. L'utilisateur (humain ou IA) interagit avec le système exclusivement via `mcp://`.

**Résultat :** 8 services implémentés, 44 tests, latence < 10μs, boot OK en 4s. `mcp-query` CLI pour interroger depuis ion shell.

### Tâches

| # | Tâche | Complexité | Mode | Statut |
|---|---|---|---|---|
| 3.1 | **Service `system/info`** — hostname, kernel, uptime, memory | Facile | Dev | ✅ Done — cached before setrens, kernel parsed from uname |
| 3.2 | **Service `system/processes`** — list processes | Moyen | Dev | ✅ Done — reads /scheme/sys/context, 44 procs visible |
| 3.3 | **Service `system/memory`** — allocation stats | Moyen | Dev | ✅ Done — placeholder (file absent on kernel) |
| 3.4 | **Service `file/read`** — lire un fichier via MCP | Moyen | Dev | ✅ Done — path validation, 10 MiB limit |
| 3.5 | **Service `file/write`** — écrire un fichier | Moyen | Dev | ✅ Done — path validation, traversal protection |
| 3.6 | **Service `file/search`** — recherche contenu | Dur | **AutoResearch** | ✅ Done — recursive walk, symlink-safe, depth-limited |
| 3.7 | **Service `net/http`** — requêtes HTTP sortantes | Moyen | Dev | ⏸ Deferred (requires network stack) |
| 3.8 | **Service `net/dns`** — résolution DNS | Facile | Dev | ⏸ Deferred (requires network stack) |
| 3.9 | **Service `console`** — terminal via MCP | Dur | Dev | ⏸ Deferred (requires ptyd integration) |
| 3.10 | **Service `log`** — logging structuré centralisé | Moyen | Dev | ✅ Done — ring buffer 1000 entries, timestamps |
| 3.11 | **Service `config`** — configuration key-value | Facile | Dev | ✅ Done — in-memory FxHashMap, CRUD methods |
| 3.12 | **Service `package`** — installer/MAJ composants | Dur | Dev | ⏸ Deferred |
| 3.13 | Retirer `ipcd` — mcpd gère l'IPC | Moyen | Dev | ⏸ Deferred (requires network) |
| 3.14 | Retirer `smolnetd` — mcpd gère le réseau | Dur | Dev | ⏸ Deferred (requires network) |
| 3.15 | Benchmark : latence service vs natif | Dur | **AutoResearch** | ✅ Done — all < 10μs (628ns–8578ns) |

**Critère de merge :** ✅ Atteint — 8 services testés en QEMU, accessibles via `mcp-query`.

**Métriques finales :**
- Services : **10 actifs** (echo, mcp, system, process, memory, file, file_write, file_search, log, config)
- Tests : **44 passing** (20 nouveaux)
- Latence : **628ns–8578ns** (tout < 10μs)
- Binary mcpd : **876K** | mcp-query : **545K**
- Security findings : **12/12 fixed**

### Outils créés
- `mcp-query` — CLI pour open+write+read sur scheme MCP
- `mcp-diag` — Diagnostic des formats /scheme/sys/ (temporaire)

---

## WS4: LLM Runtime — Le Moteur d'Inférence

**Objectif :** Un moteur d'inférence LLM tourne nativement dans ACOS, capable de générer des tokens à partir d'un prompt.

### Évaluation des options (AutoResearch multi-branches)

```
beta/ws4-llm-engine
├── beta/ws4-llamacpp       ← llama.cpp (C++, mature, GGUF)
│   ├── try-cpu-only        ← Sans GPU, GGUF Q4_K_M
│   └── try-vulkan          ← Avec GPU via Vulkan
├── beta/ws4-candle          ← Candle (Rust pur, HF)
│   ├── try-phi3-mini       ← Microsoft Phi-3 Mini (3.8B)
│   └── try-smollm          ← HF SmolLM (1.7B)
└── beta/ws4-burn            ← Burn (Rust, backends multiples)
```

### Tâches

| # | Tâche | Complexité | Mode | Branche |
|---|---|---|---|---|
| 4.1 | Cross-compiler llama.cpp pour x86_64-unknown-redox | Dur | Dev | `beta/ws4-llamacpp` |
| 4.2 | Cross-compiler Candle pour Redox | Dur | Dev | `beta/ws4-candle` |
| 4.3 | Benchmark : tokens/seconde sur CPU (sans GPU) | Moyen | **AutoResearch** | `beta/ws4-bench-cpu` |
| 4.4 | Benchmark : consommation RAM par modèle | Moyen | **AutoResearch** | `beta/ws4-bench-ram` |
| 4.5 | Évaluer les modèles candidats (Phi-3 Mini, SmolLM, Qwen2-1.5B, Gemma 2B) | Dur | **AutoResearch** | `beta/ws4-model-eval` |
| 4.6 | Intégrer le moteur choisi comme daemon `llmd` dans ACOS | Moyen | Dev | `beta/ws4-llmd-daemon` |
| 4.7 | Exposer l'inférence via `mcp://llm/generate` | Moyen | Dev | `beta/ws4-mcp-llm` |
| 4.8 | Streaming de tokens via `mcp://llm/stream` | Moyen | Dev | `beta/ws4-streaming` |
| 4.9 | Support GPU (Vulkan/compute shader si disponible) | Très dur | Dev | `beta/ws4-gpu` |
| 4.10 | Hot-swap de modèle (charger/décharger sans reboot) | Moyen | Dev | `beta/ws4-hotswap` |

**Critère de merge :** Un modèle 1-3B génère des tokens à > 5 tok/s sur CPU dans QEMU, accessible via `mcp://llm/generate`.

**Métriques AutoResearch :**
- Tokens/seconde (target > 10 sur CPU natif, > 5 dans QEMU)
- RAM utilisée (target < 2GB pour un modèle 1-3B quantisé)
- Temps de chargement du modèle (target < 5s)

---

## WS5: AI Supervisor — Le Cerveau d'ACOS

**Objectif :** Un agent IA permanent (`acosd`) écoute le bus MCP, comprend les intentions, orchestre les services, et apprend.

### Tâches

| # | Tâche | Complexité | Mode | Branche |
|---|---|---|---|---|
| 5.1 | Créer le daemon `acosd` — boucle principale (listen MCP → LLM → action) | Dur | Dev | `beta/ws5-acosd` |
| 5.2 | Système de prompts système pour les tool calls MCP | Dur | **AutoResearch** | `beta/ws5-system-prompts` |
| 5.3 | Tool calling : le LLM génère des appels `mcp://` valides | Dur | **AutoResearch** | `beta/ws5-tool-calling` |
| 5.4 | Mémoire conversationnelle (historique des échanges utilisateur) | Moyen | Dev | `beta/ws5-memory` |
| 5.5 | Mémoire long-terme (préférences utilisateur, patterns d'usage) | Dur | **AutoResearch** | `beta/ws5-long-memory` |
| 5.6 | Auto-diagnostic : le superviseur détecte et corrige les erreurs système | Très dur | **AutoResearch** | `beta/ws5-self-heal` |
| 5.7 | Apprentissage : fine-tuning incrémental sur les interactions utilisateur | Très dur | **AutoResearch** | `beta/ws5-learning` |
| 5.8 | Multi-agents : le superviseur peut spawner des sous-agents spécialisés | Dur | Dev | `beta/ws5-multi-agent` |
| 5.9 | Shell conversationnel : remplacer ion par un prompt IA | Moyen | Dev | `beta/ws5-ai-shell` |
| 5.10 | Sécurité : le superviseur ne peut jamais outrepasser les permissions MCP | Dur | Dev | `beta/ws5-security` |

**Critère de merge :** L'utilisateur peut taper en langage naturel *"quels processus consomment le plus de mémoire ?"* et recevoir une réponse structurée via le superviseur IA.

**Métriques AutoResearch :**
- Précision des tool calls (% de commandes MCP valides générées) — target > 95%
- Latence réponse end-to-end (prompt → réponse) — target < 3s
- Taux de succès des actions système (% d'actions qui aboutissent) — target > 90%

---

## WS6: Developer Experience — Écosystème Open Source

**Objectif :** Rendre ACOS accessible aux contributeurs. Documentation, SDK, exemples, CI/CD.

### Tâches

| # | Tâche | Complexité | Mode | Branche |
|---|---|---|---|---|
| 6.1 | README.md projet (vision, quickstart, architecture) | Moyen | Dev | `beta/ws6-readme` |
| 6.2 | CONTRIBUTING.md (comment contribuer, style guide, workflow git) | Moyen | Dev | `beta/ws6-contributing` |
| 6.3 | SDK Rust pour créer des services MCP (crate `acos-sdk`) | Dur | Dev | `beta/ws6-sdk` |
| 6.4 | Template "Hello World" service MCP | Facile | Dev | `beta/ws6-template` |
| 6.5 | Documentation MCP API (OpenAPI/AsyncAPI spec) | Moyen | Dev | `beta/ws6-api-docs` |
| 6.6 | GitHub Actions CI (build + test + boot QEMU sur chaque PR) | Moyen | Dev | `beta/ws6-ci` |
| 6.7 | Images pré-compilées en release (ISO, QEMU img, Podman) | Moyen | Dev | `beta/ws6-releases` |
| 6.8 | Tutoriel : "Créer votre premier service ACOS en 10 minutes" | Moyen | Dev | `beta/ws6-tutorial` |
| 6.9 | Site web projet (GitHub Pages, landing page) | Moyen | Dev | `beta/ws6-website` |
| 6.10 | Benchmark public reproductible (comparaison ACOS vs Linux+Docker pour agents IA) | Dur | Dev | `beta/ws6-benchmarks` |
| 6.11 | Système de plugins : les services MCP sont des crates publiables sur crates.io | Dur | Dev | `beta/ws6-plugins` |

**Critère de merge :** Un développeur externe peut cloner le repo, lancer `make` et avoir un ACOS bootable en < 15 minutes.

---

## WS7: Konsole — Multi-Console Natif & Display Manager

**Objectif :** ACOS possède un multiplexeur de terminaux **natif au noyau**, pas une app userspace comme tmux. Chaque console est un scheme MCP. L'IA dispose de ses propres consoles dédiées en permanence.

### Pourquoi c'est fondamental (pas cosmétique)

Sur Linux, tmux est un hack brillant : un processus userspace qui émule plusieurs terminaux dans un seul PTY. Mais c'est une couche d'abstraction au-dessus d'une couche d'abstraction (PTY → terminal → shell). L'IA qui utilise Claude Code/Codex doit lancer tmux, créer des panes, parser du texte ANSI — c'est absurde.

**Dans ACOS, le multiplexage est un scheme kernel.** Chaque console est un `mcp://konsole/N` que n'importe quel processus (humain ou IA) peut ouvrir, lire, écrire, redimensionner, et monitorer via MCP.

### Architecture Konsole

```
┌─────────────────────────────────────────────────────────────┐
│                    ÉCRAN PHYSIQUE / QEMU                     │
│                                                              │
│  ┌─────────────────────┐  ┌────────────────────────────┐   │
│  │ Konsole 0 (Root IA) │  │ Konsole 1 (Utilisateur)    │   │
│  │ ═══════════════════  │  │ ═══════════════════════    │   │
│  │ [acosd] Monitoring:  │  │ user@acos:~$               │   │
│  │ CPU: 23% MEM: 1.2G   │  │ > Montre-moi les logs      │   │
│  │ Services: 5 actifs   │  │ [IA] Voici les 10 derniers │   │
│  │ Dernière action:     │  │ logs système...             │   │
│  │ "optimisé scheduler" │  │                             │   │
│  │                      │  │                             │   │
│  │ [Alertes]            │  │                             │   │
│  │ ⚠ RAM > 80%          │  │                             │   │
│  ├──────────────────────┤  ├─────────────────────────────┤   │
│  │ Konsole 2 (Agent #1) │  │ Konsole 3 (Agent #2)       │   │
│  │ ═══════════════════  │  │ ═══════════════════════    │   │
│  │ [claude] Building... │  │ [agent] Scanning network   │   │
│  │ cargo build --rel    │  │ Found 3 devices on LAN     │   │
│  │ Compiling mcp_sch... │  │ 192.168.1.1 - router       │   │
│  │ ████████████░░ 78%   │  │ 192.168.1.42 - NAS         │   │
│  └──────────────────────┘  └─────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### Les 4 types de consoles ACOS

| Type | Propriétaire | Persistance | Usage |
|---|---|---|---|
| **Konsole Root IA** | `acosd` (superviseur) | Permanente, toujours visible | Monitoring, alertes, actions autonomes |
| **Konsole Utilisateur** | L'humain | Permanente | Shell conversationnel, interaction avec l'IA |
| **Konsole Agent** | Un agent IA (Claude, etc.) | Éphémère (durée de la tâche) | Build, tests, exploration, développement |
| **Konsole Service** | Un daemon MCP | Éphémère | Logs, debug, monitoring d'un service spécifique |

### La Konsole Root IA — Le différenciateur

C'est la console que **l'IA occupe en permanence**. Elle est toujours affichée (en sidebar ou en split). L'utilisateur peut la lire à tout moment pour voir ce que l'IA fait, pense, et planifie. L'IA peut aussi y **poser des questions à l'utilisateur** :

```
[acosd @ Konsole 0]
──────────────────────────
11:42:03 Optimisation du scheduler terminée. Gain: +12% throughput.
11:42:15 Détecté: 3 fichiers non utilisés depuis 90 jours dans /home/user/Downloads
         → Voulez-vous les archiver ? [y/n/later]
11:43:01 Agent Claude a demandé l'accès à mcp://net/http (permission réseau)
         → Autoriser pour cette session ? [y/n/always]
11:44:30 Mise à jour disponible pour le service mcp://file (v0.2.1 → v0.3.0)
         → Changelog: +recherche sémantique, +compression. Installer ? [y/n]
```

L'utilisateur répond dans SA console (Konsole 1) ou directement dans la Konsole Root.

### MCP API des Konsoles

Chaque console est un scheme MCP :

```
mcp://konsole/list                    → Liste toutes les consoles actives
mcp://konsole/0                       → La Root IA (lecture seule pour l'utilisateur)
mcp://konsole/1                       → Console utilisateur
mcp://konsole/create                  → Créer une nouvelle console
mcp://konsole/0/resize {cols:80,rows:24}  → Redimensionner
mcp://konsole/0/read                  → Lire le contenu actuel
mcp://konsole/0/write {data:"..."}    → Écrire (si autorisé)
mcp://konsole/layout                  → Gérer le layout (split, tabs, etc.)
```

### Display Manager natif

Le layout des consoles est géré par un **display manager MCP** intégré :

```
mcp://display/layout {
  "type": "split",
  "direction": "horizontal",
  "ratio": [30, 70],
  "left": {"konsole": 0},          // Root IA (30% gauche)
  "right": {
    "type": "split",
    "direction": "vertical",
    "ratio": [60, 40],
    "top": {"konsole": 1},         // Utilisateur (haut droite)
    "bottom": {"konsole": 2}        // Agent (bas droite)
  }
}
```

Le display manager gère aussi le **multi-écran** :
```
mcp://display/screens                  → Liste les écrans physiques
mcp://display/screens/0/assign {konsoles: [0, 1]}  → Écran principal
mcp://display/screens/1/assign {konsoles: [2, 3]}  → Écran secondaire
mcp://display/screens/1/fullscreen {konsole: 2}     → Agent en plein écran
```

### Tâches

| # | Tâche | Complexité | Mode | Branche |
|---|---|---|---|---|
| 7.1 | **Scheme `konsole:`** — chaque console est un scheme MCP dans mcpd | Dur | Dev | `beta/ws7-konsole-scheme` |
| 7.2 | **Konsole manager** — créer/détruire/lister des consoles | Moyen | Dev | `beta/ws7-konsole-mgr` |
| 7.3 | **Framebuffer renderer** — rendu texte sur framebuffer VGA/virtio | Dur | Dev | `beta/ws7-framebuffer` |
| 7.4 | **Layout engine** — split horizontal/vertical, tabs, resize | Dur | **AutoResearch** | `beta/ws7-layout` |
| 7.5 | **Konsole Root IA** — console dédiée acosd, toujours visible | Moyen | Dev | `beta/ws7-root-konsole` |
| 7.6 | **Input routing** — les touches clavier vont à la console focusée | Moyen | Dev | `beta/ws7-input` |
| 7.7 | **Raccourcis clavier** — switch console, split, resize (à la tmux) | Moyen | Dev | `beta/ws7-keybindings` |
| 7.8 | **Scrollback buffer** — historique scrollable par console | Moyen | Dev | `beta/ws7-scrollback` |
| 7.9 | **Multi-écran** — détecter et gérer N écrans physiques | Dur | Dev | `beta/ws7-multiscreen` |
| 7.10 | **Assignation écran** — assigner des consoles à des écrans via MCP | Moyen | Dev | `beta/ws7-screen-assign` |
| 7.11 | **Détachement/Rattachement** — comme tmux attach/detach mais natif | Moyen | Dev | `beta/ws7-detach` |
| 7.12 | **Rendu riche** — markdown, couleurs, tableaux, barres de progression | Dur | **AutoResearch** | `beta/ws7-rich-render` |
| 7.13 | **Notification cross-console** — un service peut notifier une autre console | Facile | Dev | `beta/ws7-notifications` |
| 7.14 | **Session recording** — enregistrer/rejouer une session console | Moyen | Dev | `beta/ws7-recording` |
| 7.15 | **Performance** — latence input-to-display < 16ms (60fps) | Dur | **AutoResearch** | `beta/ws7-perf` |

**Critère de merge :** 4 consoles simultanées (Root IA, Utilisateur, 2 Agents) affichées en split, avec input routing correct, latence < 16ms.

**Métriques AutoResearch :**
- Latence input-to-display (ms) — target < 16 (60fps)
- RAM par console (KB) — target < 256
- Layout recalculation time (μs) — target < 1000
- Scrollback search time pour 10K lignes (ms) — target < 50

### Pourquoi avant WS8 (Interface humaine) ?

Le multi-console est une **dépendance** de WS8 (Servo/WASM) et de WS5 (AI Supervisor). L'IA ne peut pas opérer efficacement sans ses propres consoles. Les agents comme Claude Code ne peuvent pas travailler sans multi-pane. Le display manager est nécessaire avant toute interface graphique.

```
WS7 (Konsole) ──→ WS5 (Supervisor utilise Root Konsole)
     │
     └──→ WS8 (Servo s'affiche dans une Konsole)
```

---

## WS8: Human Interface — Le Pont Humain-IA

**Objectif :** L'utilisateur interagit avec ACOS via un terminal intelligent et/ou une interface web, le tout rendu dans des Konsoles.

### Tâches

| # | Tâche | Complexité | Mode | Branche |
|---|---|---|---|---|
| 8.1 | Terminal MCP conversationnel (prompt IA, pas shell classique) | Moyen | Dev | `beta/ws8-terminal` |
| 8.2 | Autocomplétion IA dans le terminal | Moyen | **AutoResearch** | `beta/ws8-autocomplete` |
| 8.3 | Historique conversationnel persistant | Facile | Dev | `beta/ws8-history` |
| 8.4 | Intégrer Servo (moteur web Rust) comme Konsole spéciale | Très dur | Dev | `beta/ws8-servo` |
| 8.5 | Exposer le DOM de Servo via `mcp://ui/dom` | Très dur | Dev | `beta/ws8-dom-mcp` |
| 8.6 | Interface vocale (STT → MCP → LLM → TTS) | Très dur | Dev | `beta/ws8-voice` |
| 8.7 | Dashboard système web dans une Konsole Servo | Dur | Dev | `beta/ws8-dashboard` |
| 8.8 | Thèmes et personnalisation visuelle | Facile | Dev | `beta/ws8-themes` |

**Statut WS8 : ✅ COMPLETE (2026-03-24)**

**Résultat :** mcp-talk (terminal IA conversationnel) avec TalkHandler (6 méthodes MCP), line editor raw-mode, 7 commandes spéciales, colorisation ANSI, security hardening. 300 tests. QEMU validé — l'IA répond, les tool calls s'exécutent.

---

## WS9: AI Guardian — Autonomous System Monitor

**Objectif :** Un agent IA autonome (`acos-guardian`) surveille le système en permanence. Il tourne dans une boucle de monitoring, affiche l'état sur la console droite, et alerte l'utilisateur via des prompts interactifs quand il détecte une anomalie.

**C'est le premier processus IA autonome d'ACOS** — il opère sans intervention humaine, faisant d'ACOS un OS véritablement agent-centric.

### Architecture boot WS9
```
Boot → split console 50/50 vertical
  ├── LEFT  (Konsole 1): mcp-talk     ← Terminal utilisateur interactif
  └── RIGHT (Konsole 0): acos-guardian ← Moniteur autonome
```

### Composants
- **acos-guardian** — Binaire autonome, boucle de monitoring (poll 30s)
- **GuardianHandler** — Service MCP `mcp:guardian` (state, anomalies, respond, config, history)
- **Anomaly Detection Engine** — 5 détecteurs : ProcessCrash, MemoryThreshold, LogErrors, FileChanges, ServiceDown
- **Interactive Prompt System** — Prompts de choix (fix/ignore/instruct) envoyés à la console utilisateur
- **Boot Integration** — Split console natif, mcp-talk comme shell par défaut
- **`mcp:metrics`** — Service CPU/RAM temps réel (WS14, dépendance directe pour les seuils MemoryThreshold)

### Tâches

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 9.1 | GuardianHandler + data model (state, anomalies, config) | Moyen | Dev |
| 9.2 | acos-guardian binary + monitoring loop (poll → detect → display) | Moyen | Dev |
| 9.3 | ProcessCrash detector | Moyen | Dev |
| 9.4 | MemoryThreshold detector | Facile | Dev |
| 9.5 | LogError detector | Moyen | Dev |
| 9.6 | FileChange detector | Moyen | Dev |
| 9.7 | ServiceDown detector | Moyen | Dev |
| 9.8 | Interactive prompt system (choice display + user response) | Dur | Dev |
| 9.9 | mcp-talk integration (detect & render guardian prompts) | Dur | Dev |
| 9.10 | Boot integration (50/50 split, auto-launch, mcp-talk as default shell) | Moyen | Dev |
| 9.11 | Anomaly detection accuracy optimization | Dur | **AutoResearch** |
| 9.12 | Poll interval optimization (responsiveness vs CPU) | Moyen | **AutoResearch** |

**Critère de merge :** À boot, console split 50/50 avec mcp-talk à gauche et guardian à droite. Le guardian détecte un crash de processus simulé et envoie un prompt interactif à l'utilisateur.

**Métriques AutoResearch :**
- Detection rate (% anomalies détectées) — target > 95%
- False positive rate — target < 5%
- Detection latency (secondes) — target < 35s
- CPU overhead during monitoring — target < 2%

> Voir `APEX_WS9_GUARDIAN.md` pour le prompt APEX complet avec phases détaillées, agents et labs.

---

## WS10: Rich Interface — Servo, Voix, Dashboard, Thèmes

**Objectif :** Transformer ACOS d'un OS textuel en un OS à interface riche avec navigateur web intégré, interface vocale, dashboard système graphique, et personnalisation visuelle.

### Composants
- **Servo Browser Integration** — Moteur web Rust comme type de Konsole (HTML/CSS/JS)
- **DOM Exposure** — `mcp://ui/dom` permet à l'IA de lire/modifier le DOM
- **Voice Interface** — STT (Whisper.cpp) → MCP → LLM → TTS (Piper)
- **System Dashboard** — Page web temps réel dans une Servo Konsole
- **Themes** — `mcp:ui/theme` avec palettes ANSI + CSS variables

### Tâches

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 10.1 | Cross-compiler Servo embedding pour ACOS | Très dur | Dev |
| 10.2 | ServoKonsole type (framebuffer GPU) | Dur | Dev |
| 10.3 | `mcp:ui/dom` — query + modify DOM | Dur | Dev |
| 10.4 | `mcp:ui/render` — charger et afficher HTML | Dur | Dev |
| 10.5 | Whisper.cpp STT integration | Dur | Dev |
| 10.6 | Piper TTS integration | Dur | Dev |
| 10.7 | `mcp:voice/listen` et `mcp:voice/speak` | Moyen | Dev |
| 10.8 | System dashboard HTML (acos://dashboard) | Moyen | Dev |
| 10.9 | Theme engine + thèmes pré-définis | Moyen | Dev |
| 10.10 | Wake word detection ("Hey ACOS") | Dur | **AutoResearch** |

**Critère de merge :** Une page HTML s'affiche dans une Servo Konsole, l'IA peut lire le DOM, et l'utilisateur peut parler à ACOS.

> Voir `APEX_WS10_RICH_INTERFACE.md` pour l'architecture détaillée.

---

## WS11: LLM Runtime Rust-Natif — mistral.rs + Gemma 4 + Agentic Loop

**Objectif :** Remplacer Ollama/phi4-mini par [mistral.rs](https://github.com/EricLBuehler/mistral.rs) (Rust-natif, MCP client intégré) avec Gemma 4 E4B (Apache 2.0, function calling entraîné nativement) comme modèle par défaut. Exposer les 19 services MCP comme **tool definitions** consommables par l'agentic loop. Préparer l'architecture `LlmBackend` pour portabilité Phase 2.

**Phase A — host-side uniquement.** Le port natif Redox = WS12.

### Pourquoi mistral.rs + Gemma 4

| Critère | Ollama (actuel) | mistral.rs (cible) |
|---|---|---|
| Langage runtime | Go | **Rust** (cohérence noyau) |
| Client MCP natif | ❌ | ✅ Process / HTTP / WebSocket |
| Agentic loop server-side | ❌ | ✅ |
| Tool dispatch HTTP custom | ❌ | ✅ POST vers endpoint |
| Gemma 4 day-0 | partiel | ✅ multimodal complet |
| Cross-compile Redox | hors scope | candidat WS12 |

Gemma 4 (sortie 2 avril 2026, Apache 2.0) remplace **phi4-mini** qui n'a pas de function calling natif (vérifié au benchmark : 0% tool accuracy). Gemma 4 E4B tient en 6 GB RAM (4-bit), 128K contexte, multimodal texte+image+audio.

### Tâches

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 11.1 | ADR `LlmBackend` trait — interface abstraite (`generate`, `tool_call`, `embed`, `tokenize`), 2 impls (mistralrs, ollama fallback) | Facile | Dev |
| 11.2 | Installer mistral.rs avec features SIMD (`mkl` ou `cuda`), valider `mistralrs doctor` | Facile | Dev |
| 11.3 | Benchmark : tok/s + tool-call latency Gemma 4 E4B vs phi4-mini + qwen2.5 | Moyen | **AutoResearch** |
| 11.4 | Feasibility cross-compile : `cargo check --target x86_64-unknown-redox -p mistralrs --no-default-features` — go/no-go WS12 | Moyen | Dev |
| 11.5 | **Prérequis bloquant** — exposer `$service/tools/list` sur les 19 services mcpd (JSON MCP tool defs) | Moyen | Dev |
| 11.6 | Refactor `net.llm_request` : route Ollama → mistralrs-server OpenAI-compat | Moyen | Dev |
| 11.7 | mistralrs HTTP tool dispatch → endpoint mcpd `/scheme/mcp/tools/call` | Moyen | Dev |
| 11.8 | Refactor `guardian.respond` : passage en mode agentic loop (LLM → tool call → exec MCP → response) | Moyen | Dev |
| 11.9 | Refactor `ai.tool_call` : déléguer la boucle à mistralrs au lieu de parser manuellement | Moyen | Dev |
| 11.10 | Strict schema mode : valider chaque tool call contre le JSON schema du service avant exécution | Moyen | Dev |
| 11.11 | Sandbox mistralrs-server (systemd unit dédiée, user non-root, AF_UNIX socket) | Moyen | Dev |
| 11.12 | Adopter le standard [agentskills.io](https://agentskills.io) pour les skills Guardian (portabilité agents) | Facile | Dev |
| 11.13 | Tests QEMU : `guardian respond` utilise agentic loop sur cas réels | Dur | Dev |
| 11.14 | Pinning strict : `mistralrs = "=X.Y.Z"`, hash SHA256 GGUF figé | Facile | Dev |
| 11.15 | Fallback model switch : config `ACOS_LLM_MODEL=gemma-4-E4B \| qwen-3.5-4B` | Facile | Dev |
| 11.16 | Documentation `docs/WS11_LLM_RUNTIME.md` + retrait Ollama du quickstart README | Facile | Dev |

**Critère de merge :**
- ✅ `guardian respond` utilise agentic loop sans orchestration côté Rust (LLM gère la boucle)
- ✅ Toutes les tool calls validées par JSON schema strict avant exec
- ✅ Tool call round-trip p95 < 500 ms (local, prompt simple)
- ✅ Gemma 4 E4B Q4 tourne en < 8 GB RAM host
- ✅ `LlmBackend` trait permet de swap mistralrs ↔ ollama via config
- ✅ 19 services exposent `tools/list` au format MCP standard

**Métriques cibles :**
- Latence tool-call p95 : **< 500 ms**
- RAM host : **< 8 GB** (Gemma 4 E4B Q4)
- Accuracy tool selection : **> 90%** (suite 20 prompts représentatifs)
- Throughput génération : **≥ 30 tok/s** CPU avec MKL ou **≥ 60 tok/s** CUDA
- Couverture agentic : **19/19 services** exposent `tools/list`

**Prérequis bloquants :**
- 11.4 (cross-compile feasibility) → go/no-go pour WS12
- 11.3 (bench favorable) → go/no-go pour WS11 lui-même
- 11.5 (tool definitions) → bloque 11.6-11.9, **doit être fait en premier**

> Voir `docs/HERMES_EVALUATION.md` pour l'évaluation comparative avec hermes-agent (NousResearch).

---

## WS12: LLM Runtime Redox-Native — Gemma 4 dans le noyau

**Objectif :** Porter mistral.rs (CPU pur, no_std-compatible) sur la cible `x86_64-unknown-redox`. Exposer le moteur d'inférence comme service `mcp://llm` interne à mcpd, supprimant la dépendance LLM-host. Gemma 4 E2B (4 GB Q4) tourne *dans* la VM ACOS — premier pas vers la promesse "LLM as kernel".

**Phase B — exploratoire, R&D, dépend de WS11.4 favorable.**

### Défis identifiés

1. **Pas de BLAS sur Redox** — pas de MKL, pas de OpenBLAS officiellement portés. Sans accélération SIMD, inférence CPU pure-Rust = 0.2 tok/s (vérifié au benchmark WS11). Solutions possibles :
   - Porter OpenBLAS / BLIS sur Redox (gros effort)
   - Écrire des kernels matmul AVX2 inline en Rust (effort moyen, gain ciblé)
   - Attendre support CUDA Redox (jamais — trop spécialisé)
2. **Std target maturity** — `x86_64-unknown-redox` est tier 3. Tokio, memmap, fs ABI à valider.
3. **Memory mapping** — Gemma 4 utilise mmap pour les poids GGUF. Redox a un mmap mais pas testé sur 4-15 GB de fichiers.
4. **GPU drivers** — pour Phase 3 hardware réel, pas de drivers GPU dans Redox. Décision : Phase B reste **CPU-only obligatoire**.

### Tâches

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 12.1 | Fork minimal mistral.rs, strip features GPU/Metal/Accelerate | Moyen | Dev |
| 12.2 | Cross-compile workflow Podman pour `x86_64-unknown-redox` | Dur | Dev |
| 12.3 | Patcher tokio dependencies (remplacer si syscalls non-Redox) | Dur | Dev |
| 12.4 | Valider mmap GGUF 4 GB sur Redox | Moyen | Dev |
| 12.5 | Kernels matmul AVX2 inline (Rust + asm!) | Très dur | **AutoResearch** |
| 12.6 | Service `mcp://llm` natif dans mcpd (refactor du llm_handler) | Moyen | Dev |
| 12.7 | Bench tok/s in-VM avec Gemma 4 E2B Q4 | Moyen | **AutoResearch** |
| 12.8 | Suppression de `mcp:net` → Ollama dépendance | Facile | Dev |
| 12.9 | RAM budget : valider VM ACOS avec 6-8 GB pour modèle + OS | Facile | Dev |
| 12.10 | Boot test : ACOS boot avec LLM préchargé < 30s | Dur | **AutoResearch** |

**Critère de merge :**
- ✅ `mcp-query llm generate "hello"` répond en < 5s avec Gemma 4 E2B *dans* la VM, sans Ollama host
- ✅ Tok/s ≥ 5 sur Gemma 4 E2B CPU pur dans QEMU
- ✅ Boot ACOS + LLM ready en < 30s
- ✅ RAM totale VM (OS + modèle + KV cache) < 8 GB

**Métriques cibles :**
- Tok/s in-VM : **≥ 5** (E2B Q4)
- Cold start LLM : **< 30s** depuis boot
- RAM totale : **< 8 GB** (OS ~2 GB + modèle ~4 GB + cache ~2 GB)
- Latence inférence (10 tokens) : **< 5s**

**Risques de no-go :**
- Si WS11.4 montre que cross-compile est cassé sans patch upstream majeur → on diffère WS12 jusqu'à upstream PR
- Si tok/s < 1 même avec kernels AVX2 → on revoit la stratégie (modèle plus petit ? quantification 2-bit ? renoncer LLM-in-kernel pour Phase 1 hardware ?)

> Cette work représente la promesse fondatrice d'ACOS : "LLM as kernel". Échec acceptable, abandon non-acceptable.

---

## WS13: Web GUI Remote-First — Interface universelle MCP-driven

**Objectif :** Construire l'interface graphique d'ACOS comme une **SPA web hébergée par mcpd**, accessible depuis n'importe quel navigateur (laptop / phone / tablette). Local et distant utilisent **le même bundle**, le même protocole MCP-over-WebSocket, les mêmes tool definitions. Phase A : remote browser → ACOS. Phase B (= WS10 reformulé) : Servo embedded rend la même SPA en local.

**Pourquoi maintenant** : la GUI n'est plus un binaire compilé qu'on doit cross-compiler — c'est un **agent client** comme un autre, qui parle MCP. L'humain ↔ l'OS et l'IA ↔ l'OS passent par le **même protocole**, mêmes 19 services, même format JSON-RPC.

### Architecture cible

```
[laptop / phone / TV]                  [ACOS / QEMU / hardware]
  Browser                                  
    └─ HTML + JS bundle                    mcpd
       └─ HTTPS/WSS ─────────────────────► ├── mcp://gui (nouveau)
                                           ├── 19 services existants
                                           └── exposed via wss://acos.local/mcp
```

### Stack proposée (recommandation)

| Couche | Choix | Raison |
|---|---|---|
| Framework UI | **SolidJS** (ou Svelte) | Bundle <50 kB, pas de VDOM, perf proche DOM natif (critique sur Servo embedded sans GPU) |
| Build | **Vite** (dev) + bundle statique (prod) | Single HTML+JS+CSS, pas de Node runtime côté ACOS |
| Transport | **MCP-over-WebSocket** | Spec MCP officielle, mistralrs supporte côté Rust |
| Auth | **mTLS + token MCP** | Modèle agent-to-agent, pas de session/cookie |
| Rendering local (Phase B) | **Servo** (déjà au plan WS10) | Cohérence Rust |
| À éviter | React, Electron, Next.js | Lourds, supply chain massive (npm), hostiles aux contraintes OS |

### Tâches

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 13.1 | ADR architecture `mcp://gui` + scheme `acos://` (URL custom pour ressources GUI) | Facile | Dev |
| 13.2 | Service `mcp://gui` : `tools/list`, `state`, `notify`, `dialog.show`, `panel.open`, etc. | Moyen | Dev |
| 13.3 | HTTP/WSS server intégré à mcpd (serve bundle + WebSocket MCP transport) | Moyen | Dev |
| 13.4 | Bootstrap SPA : SolidJS + Vite + connection WSS au serveur ACOS | Moyen | Dev |
| 13.5 | UI : dashboard système (CPU/RAM/services), 19 services explorables, log live | Moyen | Dev |
| 13.6 | mTLS — émission cert serveur ACOS + client (pairing) | Moyen | Dev |
| 13.7 | Service `mcp://identity` pour pairing nouveaux clients (QR code) | Moyen | Dev |
| 13.8 | LLM driving — Gemma 4 reçoit le state visuel sérialisé, peut appeler les tools UI | Moyen | Dev |
| 13.9 | Offline-first : bundle 100% local (pas de CDN, pas de Google Fonts) | Facile | Dev |
| 13.10 | Mobile-friendly responsive layout | Moyen | Dev |
| 13.11 | Tests : pilotage de la GUI par Guardian via tool calls | Dur | **AutoResearch** |

**Critère de merge :**
- ✅ Browser laptop se connecte à `https://<acos-host>:8443/`, voit dashboard temps réel des 19 services
- ✅ Toute action UI = appel MCP à un service mcpd (pas d'API REST custom)
- ✅ Guardian peut envoyer une notification visible dans la GUI
- ✅ Gemma 4 (via mistralrs) peut piloter la GUI en appelant `mcp://gui/tools/list`
- ✅ Bundle JS+CSS+HTML < 200 kB, zéro requête externe au runtime

**Métriques cibles :**
- Bundle size : **< 200 kB** (gzipped)
- Time to first interactive : **< 1s** sur laptop, **< 3s** sur phone
- Latence interaction MCP (click → action) : **< 100 ms** local, **< 300 ms** remote
- Cold start ACOS GUI server : **< 500 ms**

**Position vs WS10 :** WS13 absorbe la couche "rendering web" de WS10. WS10 devient strictement "Servo embedded + voix + thèmes" — la GUI elle-même (HTML/JS/state) appartient à WS13 et est partagée entre remote et embedded.

---

## WS14: Metrics Service — CPU% et RAM Temps Réel

**Objectif :** Ajouter un service `mcp:metrics` dans mcpd qui expose CPU% (usage CPU global via sampling des processus) et RAM (used/total/free) en temps réel, interrogable via `mcp-query` depuis ion.

### Pourquoi un service Metrics

WS3 fournit `system/info` (hostname, uptime), `system/processes` (liste) et `system/memory` (RAM statique au boot). `mcp:metrics` est complémentaire — il expose CPU% et RAM **en temps réel** avec cache TTL 5s, consolidées dans un seul service pour Guardian et les agents IA. `mcp:memory` (WS3.3) = snapshot statique au boot ; `mcp:metrics/ram` = mise à jour temps réel via `/scheme/sys/meminfo`. Les deux co-existent.

### Cache TTL 5s

Lecture directe de `/scheme/sys/context` (CPU) et `/scheme/sys/meminfo` (RAM) a un coût (~50-100μs cumulés). Un cache TTL avec `Arc<Mutex<CacheEntry>>` (5s) amortit ce coût pour les appels rapprochés — Guardian polling toutes les 30s est unaffected.

### Tâches

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 14.1 | `MetricsHandler` — structure + methods `snapshot`, `history`, `reset` | Facile | Dev |
| 14.2 | Lecture CPU% — lire `/scheme/sys/context` (colonne TIME par processus), calculer Δtime/Δwall entre deux snapshots. Limitation : kernel Redox n'expose pas de stats CPU globales (idle, iowait). Pour CPU% global précis, prérequis : patch kernel WS14.0 pour exposer `/scheme/sys/cpu_stat`. En attendant : somme des ΔTIME visibles / wall_time = approximation (rate limit pour Guardian足够了) | Moyen | Dev |
| 14.3 | Lecture RAM — lire `/scheme/sys/meminfo` (prefered) ou estimer via `/scheme/sys/context` — ne duplique pas system/memory, utilise les mêmes sources | Facile | Dev |
| 14.4 | Cache TTL 5s — `Arc<Mutex<Option<CacheEntry>>>` avec `Instant` interne. `Option` permet invalidate/reload sans re-create the cell. Check TTL sur chaque accès snapshot(), reload si expiré | Moyen | Dev |
| 14.5 | `metrics snapshot` — retourne `{cpu_percent: f32, ram_used: u64, ram_total: u64, ram_free: u64, timestamp_us: u64}` | Facile | Dev |
| 14.6 | `metrics history` — ring buffer 60 snapshots (échantillonnage configurable, default 1/min). **Indépendant du cache TTL 5s** — history a son propre sampler qui ne passe PAS par `snapshot()` pour éviter les cache miss garantis. Le cache 5s sert uniquement aux appels rapprochés type Guardian polling toutes les 30s | Moyen | Dev |
| 14.7 | Tests : `mcp-query metrics snapshot` → JSON vérifiable, latence mesurée | Facile | Dev |

### Critère de merge

- ✅ `mcp-query metrics snapshot` retourne `{cpu_percent, ram_used, ram_total, ram_free, timestamp_us}` — tous non-nuls
- ✅ Latence round-trip mesurée **< 10 μs** (cache hit, même machine de ref WS3)
- ✅ Cache TTL 5s valide — appels < 5s apart ne re-parsent pas les schemes kernel
- ✅ Tests passent dans QEMU boot, `mcp list` affiche `metrics`
- ✅ **Hooks d'abonnement exposés** pour que WS9 Guardian puisse se brancher (`metrics/subscribe(Callback, thresholds)` côté metrics, intégration Guardian = WS9.X future)

### Métriques cibles

- Latence cache hit : **< 10 μs**
- Latence cache miss (première lecture) : **< 100 μs** (parse schemes kernel)
- RAM overhead : **< 20 KB** (ring buffer 60 × 64 bytes + cache metadata)
- CPU% accuracy : **± 5%** vs comptage ticks
- History depth : **60 snapshots** (ring buffer circulaire)

### Prérequis bloquants

- WS3 (`system/info`, `system/processes`, `system/memory`) merged et boot-validé — on réutilise les mêmes lectures de schemes kernel
- **WS14.0 (hors scope si kernel patch necessaire)** — Si CPU% global précis requis (vs approximation via ΔTIME visibles) : patch kernel Redox pour exposer `/scheme/sys/cpu_stat` avec champs {idle_ticks, total_ticks}. Sans ce patch, approximation somme(ΔTIME)/wall_time suffit pour seuils Guardian

---

## Séquencement & Dépendances

```
Trimestre 1 (Phase Fondation) ✅ DONE
════════════════════════════════════
WS1 ████████████████████████  Kernel Identity + build offline          ✅
WS2 ████████████████████████  MCP Bus complet                          ✅
WS3 ████████████████████████  System Services (10 services)            ✅

Trimestre 2 (Phase Intelligence + Konsole) ✅ DONE
══════════════════════════════════════════════════
WS4 ████████████████████████  LLM Runtime (Gemini proxy + SmolLM)     ✅
WS5 ████████████████████████  AI Supervisor (function calling)         ✅
WS7 ████████████████████████  Konsole (14 services, 318 tests)        ✅

Trimestre 3 (Phase Human Interface) — EN COURS
═══════════════════════════════════════════════
WS8 ████████████████████████  mcp-talk (300 tests, QEMU validé)       ✅
WS9 ████████████████░░░░░░░░  AI Guardian (autonomous monitor)        ← NEXT
WS14 █░░░░░░░░░░░░░░░░░░░░░░░  mcp:metrics (CPU/RAM temps réel)         ← avec WS9
WS6 ░░░░████████████░░░░░░░░  SDK + docs + tutoriels                  ⏸

Trimestre 4 (Phase LLM Rust-Natif + Web GUI)
═════════════════════════════════════════════
WS11 ████████████████████████  mistral.rs + Gemma 4 + tool definitions
WS13 ░░░░████████████████████  Web GUI Remote-First (SolidJS + WSS)

Trimestre 5+ (Phase Embedded + Hardware)
═════════════════════════════════════════
WS12 ░░░░░░░░░░██████████████  LLM natif Redox (mistral.rs CPU pur)
WS10 ░░░░░░░░░░░░██████████░░  Servo embedded, voix, thèmes
WS6  ░░░░░░░░████████████████  Plugins, benchmarks publics
```

### Graphe de dépendances

```
WS1 (Identity) ──→ WS6 (DX) ──→ Community launch
     │
     ▼
WS2 (MCP Bus) ──→ WS3 (Services) ──→ WS7 (Konsole) ──→ WS5 (Supervisor)
                       │                    │                   │
                       ▼                    ▼                   ▼
                  WS4 (LLM Runtime) ──→ WS5 (utilise Konsole Root)
                                                                │
                                                                ▼
                                                          WS8 (mcp-talk)
                                                                │
                                                                ▼
                                                          WS9 (Guardian)
                                                                │
                                                                ▼
                                                          WS10 (Rich Interface)
```

---

## Stratégie AutoResearch par Phase

### Phase 1 — Composants isolés (maintenant)
Chaque composant est itéré individuellement dans sa branche beta.
```
Modifier src/ → inject (3s) → cross-compile (10s) → inject image (3s) → boot test (4s)
= 20 secondes par itération
= 180 itérations/heure
= ~1500 itérations/nuit
```

### Phase 2 — Intégration (T2)
Les branches beta validées sont mergées dans alpha. Tests d'intégration croisés.
```
Merge beta/ws2 + beta/ws3 → test intégration complète
Si régression → bisect automatique → identifier le conflit
```

### Phase 3 — Meta-optimisation (T3)
L'agent optimise ses propres instructions et métriques.
```
Analyser les patterns de succès → modifier program.md → relancer les boucles
Le superviseur IA aide à optimiser son propre développement (bootstrap)
```

### Phase 4 — Auto-évolution (T4)
Le superviseur IA d'ACOS participe à son propre développement (comme MiniMax M2.7).
```
acosd génère des hypothèses d'amélioration → crée des branches beta → teste → merge
L'OS s'améliore lui-même.
```

---

## Indicateurs de Succès pour l'Adoption Open Source

### Technique
- [ ] Build en < 15 min depuis un clone frais
- [ ] Boot en < 5 secondes dans QEMU
- [ ] > 95% des méthodes MCP standard implémentées
- [ ] LLM local fonctionnel avec > 10 tok/s
- [ ] Latence MCP IPC < 10μs

### Communauté
- [ ] README clair avec vision, quickstart, screenshots/démo
- [ ] CONTRIBUTING.md avec workflow git et style guide
- [ ] 10+ issues labellisées "good first issue"
- [ ] SDK documenté pour créer des services MCP
- [ ] Tutoriel "Mon premier service ACOS"
- [ ] CI/CD fonctionnel (chaque PR testée automatiquement)
- [ ] Releases régulières avec images bootables
- [ ] Benchmark public comparant ACOS à Linux pour les workloads IA

### Milestones GitHub
| Milestone | Contenu | Cible |
|---|---|---|
| **v0.1 — First Light** | Boot + MCP scheme + echo service | ✅ Fait (WS1-WS2) |
| **v0.2 — Services** | 10 system services via MCP | ✅ Fait (WS3) |
| **v0.3 — Intelligence** | LLM runtime + AI supervisor + function calling | ✅ Fait (WS4-WS5) |
| **v0.4 — Konsole** | Multi-console natif + Root IA + display manager | ✅ Fait (WS7) |
| **v0.5 — Conversation** | mcp-talk terminal IA conversationnel | ✅ Fait (WS8) |
| **v0.6 — Guardian + Metrics** | AI Guardian autonome, split console, mcp:metrics (CPU/RAM temps réel) | ← NEXT (WS9 + WS14) |
| **v0.7 — LLM Rust** | mistral.rs + Gemma 4 + tool definitions + agentic loop | Planned (WS11) |
| **v0.8 — Web GUI** | Remote-first SPA, MCP-over-WSS, accessible navigateur | Planned (WS13) |
| **v0.9 — Native LLM** | mistral.rs CPU pur dans Redox, fin dépendance Ollama | Phase 2 (WS12) |
| **v0.10 — Rich Interface** | Servo embedded, voix, dashboard graphique, thèmes | Phase 2 (WS10) |
| **v1.0 — First Light Hardware** | Boot bare metal, plus QEMU only | Phase 3 |
| **v1.5 — Self-Aware** | Auto-diagnostic + self-healing + multi-agents | Phase 4 |

---

## Appel à contribution (draft pour le README GitHub)

> **ACOS** is the first operating system where AI is not an app — it IS the system.
>
> Built on a Rust micro-kernel (Redox OS fork), ACOS replaces every userspace daemon with MCP-native services. A local LLM supervisor orchestrates the entire system via semantic IPC. No shell scripts. No config files. You talk to your computer, and it understands.
>
> **Why contribute?**
> - Every OS today was designed before AI. This one is designed for it.
> - MCP is becoming the universal protocol for AI agents (Anthropic → Linux Foundation). ACOS makes it a first-class kernel citizen.
> - Pure Rust from kernel to userspace. Memory-safe by design.
> - AutoResearch-driven: the development process itself is AI-assisted and measurable.
>
> **Get started in 5 minutes:**
> ```bash
> git clone https://github.com/ankheru/acos
> cd acos && make boot  # Builds and boots in QEMU
> ```
