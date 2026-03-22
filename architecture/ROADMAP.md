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

Le développement est organisé en **7 workstreams** indépendants, chacun avec ses branches AutoResearch.

```
ACOS Development Tree
│
├── WS1: Kernel Identity (rebrand + isolation du build)
├── WS2: MCP Bus (scheme natif, routing, protocol)
├── WS3: System Services (process, file, net via MCP)
├── WS4: LLM Runtime (moteur d'inférence local)
├── WS5: AI Supervisor (orchestration, tool calls, mémoire)
├── WS6: Developer Experience (SDK, docs, tooling)
└── WS7: Human Interface (terminal IA, Servo/WASM, voix)
```

---

## WS1: Kernel Identity & Build Independence

**Objectif :** ACOS est un projet autonome, pas un "mod Redox". Build reproductible, zéro dépendance réseau.

### Tâches

| # | Tâche | Complexité | Mode | Branche |
|---|---|---|---|---|
| 1.1 | Remplacer "Redox" par "ACOS" dans tous les messages kernel (boot, panic, logs) | Facile | Dev | `beta/ws1-branding` |
| 1.2 | Modifier `os-release`, hostname, login banner | Facile | Dev | `beta/ws1-branding` |
| 1.3 | Forker le repo kernel Redox en local (`recipes/core/kernel/source/`) | Moyen | Dev | `beta/ws1-kernel-fork` |
| 1.4 | Forker relibc en local | Moyen | Dev | `beta/ws1-relibc-fork` |
| 1.5 | Créer `build_offline.sh` — compilation 100% locale sans REPO_BINARY | Moyen | Dev | `beta/ws1-offline-build` |
| 1.6 | Remplacer le registry Redox par un registry local (dossier `packages/`) | Moyen | Dev | `beta/ws1-local-registry` |
| 1.7 | Automatiser le build CI (GitHub Actions + cache Podman) | Moyen | Dev | `beta/ws1-ci` |
| 1.8 | Publier les images ACOS (ISO, QEMU img) en release GitHub | Facile | Dev | `beta/ws1-releases` |

**Critère de merge :** Le build complet fonctionne sans aucune connexion réseau après le clone initial.

---

## WS2: MCP Bus — Le Cœur d'ACOS

**Objectif :** Le scheme `mcp:` est un citoyen de première classe dans le kernel. Chaque service s'enregistre et communique via MCP.

### Tâches

| # | Tâche | Complexité | Mode | Branche |
|---|---|---|---|---|
| 2.1 | Enregistrer `mcp:` via `Socket::create("mcp")` dans mcpd (utiliser `redox_scheme` crate) | Moyen | Dev | `beta/ws2-scheme-register` |
| 2.2 | Implémenter `open("mcp://service/resource")` → dispatche vers le handler | Moyen | Dev | `beta/ws2-open` |
| 2.3 | Implémenter `write()` → envoi JSON-RPC request | Moyen | Dev | `beta/ws2-write` |
| 2.4 | Implémenter `read()` → réception JSON-RPC response | Moyen | Dev | `beta/ws2-read` |
| 2.5 | Tester depuis ion : `cat mcp://echo <<< '{"jsonrpc":"2.0","method":"ping","id":1}'` | Moyen | Dev | `beta/ws2-integration-test` |
| 2.6 | Conformité MCP spec : `initialize`, `tools/list`, `resources/list`, `prompts/list` | Dur | **AutoResearch** | `beta/ws2-mcp-conformity` |
| 2.7 | Optimiser la latence IPC du scheme MCP (< 10μs round-trip) | Dur | **AutoResearch** | `beta/ws2-latency` |
| 2.8 | Optimiser le throughput (> 100K msg/s) | Dur | **AutoResearch** | `beta/ws2-throughput` |
| 2.9 | Support multi-clients simultanés (100+ connexions MCP parallèles) | Moyen | **AutoResearch** | `beta/ws2-concurrency` |
| 2.10 | Registre de services dynamique (un service peut s'enregistrer/se désenregistrer à chaud) | Moyen | Dev | `beta/ws2-service-registry` |
| 2.11 | Protocole binaire optionnel (MessagePack au lieu de JSON pour les chemins chauds) | Dur | **AutoResearch** | `beta/ws2-binary-proto` |

**Critère de merge :** Un processus userspace peut ouvrir `mcp://echo`, envoyer un ping, recevoir un pong, avec une latence < 50μs.

**Métriques AutoResearch :**
- Latence round-trip (μs) — target < 10
- Throughput (messages/seconde) — target > 100K
- Conformité MCP spec (% des méthodes standard implémentées) — target 100%

---

## WS3: System Services — Remplacer le Userspace Redox

**Objectif :** Chaque daemon Redox est remplacé par un service MCP dans mcpd. L'utilisateur (humain ou IA) interagit avec le système exclusivement via `mcp://`.

### Tâches

| # | Tâche | Complexité | Mode | Branche |
|---|---|---|---|---|
| 3.1 | **Service `system/info`** — hostname, uptime, kernel version, memory | Facile | Dev | `beta/ws3-system-info` |
| 3.2 | **Service `system/processes`** — list, kill, priority, resource usage | Moyen | Dev | `beta/ws3-processes` |
| 3.3 | **Service `system/memory`** — allocation, usage, pressure | Moyen | Dev | `beta/ws3-memory` |
| 3.4 | **Service `file/read`** — lire un fichier via `mcp://file/path/to/file` | Moyen | Dev | `beta/ws3-file-read` |
| 3.5 | **Service `file/write`** — écrire un fichier | Moyen | Dev | `beta/ws3-file-write` |
| 3.6 | **Service `file/search`** — recherche par contenu/métadonnées | Dur | **AutoResearch** | `beta/ws3-file-search` |
| 3.7 | **Service `net/http`** — requêtes HTTP sortantes | Moyen | Dev | `beta/ws3-net-http` |
| 3.8 | **Service `net/dns`** — résolution DNS | Facile | Dev | `beta/ws3-net-dns` |
| 3.9 | **Service `console`** — terminal interactif via MCP (remplace getty+ptyd) | Dur | Dev | `beta/ws3-console` |
| 3.10 | **Service `log`** — logging structuré centralisé | Moyen | Dev | `beta/ws3-logging` |
| 3.11 | **Service `config`** — configuration système key-value | Facile | Dev | `beta/ws3-config` |
| 3.12 | **Service `package`** — installer/mettre à jour des composants | Dur | Dev | `beta/ws3-package` |
| 3.13 | Retirer `ipcd` de l'image — mcpd gère tout l'IPC | Moyen | Dev | `beta/ws3-remove-ipcd` |
| 3.14 | Retirer `smolnetd` — mcpd gère le réseau | Dur | Dev | `beta/ws3-remove-smolnetd` |
| 3.15 | Benchmark : latence de chaque service vs équivalent Redox natif | Dur | **AutoResearch** | `beta/ws3-benchmarks` |

**Critère de merge :** Tous les services passent leurs tests d'intégration dans QEMU. Chaque service est accessible via `mcp://` depuis n'importe quel processus.

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

## WS7: Human Interface — Le Pont Humain-IA

**Objectif :** L'utilisateur interagit avec ACOS via un terminal intelligent et/ou une interface web.

### Tâches

| # | Tâche | Complexité | Mode | Branche |
|---|---|---|---|---|
| 7.1 | Terminal MCP : affichage riche (markdown, tableaux, code coloré) | Moyen | Dev | `beta/ws7-terminal` |
| 7.2 | Autocomplétion IA dans le terminal | Moyen | **AutoResearch** | `beta/ws7-autocomplete` |
| 7.3 | Historique conversationnel persistant | Facile | Dev | `beta/ws7-history` |
| 7.4 | Intégrer Servo (moteur web Rust) pour les WebApps | Très dur | Dev | `beta/ws7-servo` |
| 7.5 | Exposer le DOM de Servo via `mcp://ui/dom` | Très dur | Dev | `beta/ws7-dom-mcp` |
| 7.6 | Notifications système via MCP | Facile | Dev | `beta/ws7-notifications` |
| 7.7 | Interface vocale (STT → MCP → LLM → TTS) | Très dur | Dev | `beta/ws7-voice` |
| 7.8 | Dashboard système web (CPU, RAM, services, logs) accessible via Servo | Dur | Dev | `beta/ws7-dashboard` |
| 7.9 | Thèmes et personnalisation visuelle | Facile | Dev | `beta/ws7-themes` |

---

## Séquencement & Dépendances

```
Trimestre 1 (Phase Fondation)
════════════════════════════
WS1 ████████████████░░░░░░░░  Kernel Identity + build offline
WS2 ████████████████████████  MCP Bus complet
WS6 ████████░░░░░░░░░░░░░░░░  README + CI + quickstart

Trimestre 2 (Phase Services)
════════════════════════════
WS3 ████████████████████████  Tous les services system/file/net
WS4 ████████████████░░░░░░░░  LLM Runtime (évaluation + intégration)
WS6 ░░░░████████████████████  SDK + docs + tutoriels

Trimestre 3 (Phase Intelligence)
════════════════════════════════
WS5 ████████████████████████  AI Supervisor complet
WS4 ░░░░░░░░████████████████  Optimisation LLM (GPU, hot-swap)
WS7 ████████████░░░░░░░░░░░░  Terminal IA

Trimestre 4 (Phase Interface)
═════════════════════════════
WS7 ░░░░░░░░░░░░████████████  Servo/WASM, Dashboard, Voix
WS5 ░░░░░░░░░░░░░░░░████████  Auto-diagnostic, multi-agents
WS6 ░░░░░░░░░░░░████████████  Plugins, benchmarks publics
```

### Graphe de dépendances

```
WS1 (Identity) ──→ WS6 (DX) ──→ Community launch
     │
     ▼
WS2 (MCP Bus) ──→ WS3 (Services) ──→ WS5 (Supervisor) ──→ WS7 (UI)
                       │                    ▲
                       ▼                    │
                  WS4 (LLM Runtime) ────────┘
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
| **v0.3 — Intelligence** | LLM runtime + superviseur basique | T2 |
| **v0.4 — Conversation** | Shell IA conversationnel | T3 |
| **v0.5 — Self-Aware** | Auto-diagnostic + self-healing | T4 |
| **v1.0 — First Contact** | OS complet utilisable en daily | T5+ |

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
