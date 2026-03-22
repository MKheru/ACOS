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

Le développement est organisé en **8 workstreams** indépendants, chacun avec ses branches AutoResearch.

```
ACOS Development Tree
│
├── WS1: Kernel Identity (rebrand + isolation du build)
├── WS2: MCP Bus (scheme natif, routing, protocol)
├── WS3: System Services (process, file, net via MCP)
├── WS4: LLM Runtime (moteur d'inférence local)
├── WS5: AI Supervisor (orchestration, tool calls, mémoire)
├── WS6: Developer Experience (SDK, docs, tooling)
├── WS7: Konsole — Multi-Console Natif & Display Manager
└── WS8: Human Interface (terminal IA, Servo/WASM, voix)
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

---

## Séquencement & Dépendances

```
Trimestre 1 (Phase Fondation)
════════════════════════════
WS1 ████████████████░░░░░░░░  Kernel Identity + build offline
WS2 ████████████████████████  MCP Bus complet
WS6 ████████░░░░░░░░░░░░░░░░  README + CI + quickstart

Trimestre 2 (Phase Services + Konsole)
══════════════════════════════════════
WS3 ████████████████████████  Tous les services system/file/net
WS4 ████████████████░░░░░░░░  LLM Runtime (évaluation + intégration)
WS7 ████████████████████████  Konsole multi-console natif ← PRIORITAIRE
WS6 ░░░░████████████████████  SDK + docs + tutoriels

Trimestre 3 (Phase Intelligence)
════════════════════════════════
WS5 ████████████████████████  AI Supervisor (utilise Root Konsole)
WS4 ░░░░░░░░████████████████  Optimisation LLM (GPU, hot-swap)
WS7 ░░░░░░░░░░░░████████████  Multi-écran, recording, perf
WS8 ████████████░░░░░░░░░░░░  Terminal IA conversationnel

Trimestre 4 (Phase Interface)
═════════════════════════════
WS8 ░░░░░░░░░░░░████████████  Servo/WASM, Dashboard, Voix
WS5 ░░░░░░░░░░░░░░░░████████  Auto-diagnostic, multi-agents
WS6 ░░░░░░░░░░░░████████████  Plugins, benchmarks publics
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
                                                          WS8 (UI dans Konsole)
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
| **v0.1 — First Light** | Boot + MCP scheme + echo service | ✅ Fait |
| **v0.2 — Services** | system + file + net via MCP | T1 |
| **v0.3 — Konsole** | Multi-console natif + Root IA + display manager | T2 |
| **v0.4 — Intelligence** | LLM runtime + superviseur dans Root Konsole | T2-T3 |
| **v0.5 — Conversation** | Shell IA conversationnel dans Konsole utilisateur | T3 |
| **v0.6 — Self-Aware** | Auto-diagnostic + self-healing + multi-agents | T4 |
| **v1.0 — First Contact** | OS complet utilisable en daily, multi-écran | T5+ |

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
