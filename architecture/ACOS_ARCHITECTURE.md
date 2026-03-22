# Architecture ACOS (Agent-Centric OS)

*Version 5.0 — Post-Phase 1 : Réalité technique établie (2026-03-22)*

---

## 1. Vision

ACOS est un système d'exploitation **AI-LLM-centric** construit sur un micro-noyau Rust (fork de Redox OS). Chaque composant userspace est remplacé par un équivalent natif MCP (Model Context Protocol). Un LLM local spécialisé sert de superviseur système, capable de gérer le kernel et l'espace utilisateur via le bus sémantique `mcp:`.

**Principe fondateur :** L'humain ne manipule plus des fichiers, des processus ou des commandes. Il converse avec l'OS. L'OS comprend, agit, et rend compte via MCP.

---

## 2. Anatomie de Redox OS — Ce qu'on utilise réellement

### 2.1. Redox est un micro-noyau, PAS un exokernel

```
EXOKERNEL (MIT Aegis)              MICRO-NOYAU (Redox/ACOS)
┌─────────────────────┐           ┌─────────────────────┐
│ App + libOS          │           │ App (userspace)      │
│ (gère sa propre      │           │                      │
│  mémoire, FS, etc.)  │           ├──────────────────────┤
├─────────────────────┤           │ Services userspace   │
│ Exokernel            │           │ (drivers, FS,        │
│ (~1000 lignes)       │           │  réseau, IPC, LLM)   │
│ Multiplex HW brut    │           ├──────────────────────┤
└─────────────────────┘           │ Micro-noyau Rust     │
                                   │ (~30K lignes)        │
                                   │ Scheduling, IPC,     │
                                   │ mémoire virtuelle    │
                                   └──────────────────────┘
```

Un exokernel expose le hardware brut et laisse chaque app gérer ses propres abstractions.
Redox (et donc ACOS) fournit un noyau minimal mais complet : scheduler, mémoire virtuelle, IPC, syscalls.
Tout le reste vit en userspace comme des daemons qui s'enregistrent via des "schemes" (URIs).

### 2.2. Architecture interne du kernel Redox

```
kernel/ (~30 000 lignes Rust, zéro C)
├── src/
│   ├── main.rs              ← Point d'entrée du noyau
│   ├── scheme/              ← Root scheme — le mécanisme d'enregistrement
│   │   ├── root.rs          ← File::create(":nom") → enregistre un scheme
│   │   └── ...
│   ├── context/             ← Processus et context switching
│   │   ├── switch.rs        ← Changement de contexte CPU
│   │   ├── signal.rs        ← Signaux POSIX
│   │   └── ...
│   ├── memory/              ← Mémoire virtuelle, pagination
│   ├── syscall/             ← ~30 appels système
│   │   ├── fs.rs            ← open, read, write, close (tout passe par les schemes)
│   │   ├── process.rs       ← fork, exec, exit, wait
│   │   └── ...
│   └── arch/x86_64/         ← Code spécifique à l'architecture
│       ├── interrupt/       ← IDT, handlers d'interruptions
│       ├── paging/          ← Tables de pages
│       └── ...
└── Cargo.toml
```

**Schemes kernel** (dans le noyau, pas remplaçables) :
- `pipe:` — communication inter-processus basique
- `event:` — notification d'événements
- `memory:` — allocation mémoire
- `time:` — horloge système
- `sys:` — informations système
- `proc:` — gestion des processus
- `:` (root) — enregistrement des schemes userspace

### 2.3. Ce qui est en userspace (remplaçable)

| Daemon Redox | Scheme | Rôle | Remplacé par ACOS |
|---|---|---|---|
| `ipcd` | `chan:`, `shm:` | IPC channels, shared memory | **mcpd** (IPC via MCP) |
| `ptyd` | `pty:` | Pseudo-terminaux | **mcpd** terminal service |
| `smolnetd` | `tcp:`, `udp:`, `ip:` | Stack réseau | **mcpd** net service |
| `nvmed` | `disk/nvme:` | Driver NVMe | Gardé tel quel |
| `ahcid` | `disk/ahci:` | Driver AHCI/SATA | Gardé tel quel |
| `redoxfs` | `file:` | Système de fichiers | À terme : FS sémantique |
| `getty` | — | Login console | **mcpd** console service |
| `init` | — | Démarrage | Notre propre séquence |

### 2.4. Le build system et les serveurs Redox

**Clarification importante :** Le fait que `REPO_BINARY=1` télécharge depuis les serveurs Redox n'est PAS une dépendance architecturale. C'est un **cache de compilation** (comme un registry npm/cargo). On peut :

1. **`REPO_BINARY=1`** : télécharger des `.pkgar` pré-compilés (rapide, ~2 min)
2. **`REPO_BINARY=0`** : tout compiler depuis les sources (lent, ~30 min)
3. **Notre approche** : utiliser les binaires pré-compilés pour les composants non modifiés (kernel, bootloader, relibc) et compiler uniquement nos composants (mcpd, etc.) via cross-compilation directe

Le marketplace Redox ne sert que pendant le build. L'image finale est 100% autonome et n'a aucune connexion aux serveurs Redox au runtime.

---

## 3. Stratégie ACOS : Kernel vierge + Services MCP

### 3.1. Les trois couches ACOS

```
┌─────────────────────────────────────────────────────────┐
│                    COUCHE 3 : INTELLIGENCE               │
│                                                          │
│  ┌──────────────────────────────────────────┐           │
│  │         LLM Superviseur ACOS              │           │
│  │  (petit modèle local spécialisé OS)       │           │
│  │  - Comprend les commandes en langage nat.  │           │
│  │  - Orchestre les services via MCP          │           │
│  │  - Apprend du comportement utilisateur     │           │
│  └────────────────────┬─────────────────────┘           │
│                       │ JSON-RPC / MCP                   │
├───────────────────────┼─────────────────────────────────┤
│                    COUCHE 2 : SERVICES MCP               │
│                       │                                  │
│  ┌────────┐ ┌────────┤ ┌────────┐ ┌────────┐           │
│  │system: │ │file:   │ │net:    │ │ui:     │           │
│  │process │ │sémant. │ │tcp/http│ │term/web│           │
│  └───┬────┘ └───┬────┘ └───┬────┘ └───┬────┘           │
│      │          │          │          │                  │
│  ┌───┴──────────┴──────────┴──────────┴───┐             │
│  │         mcpd — Daemon MCP unifié        │             │
│  │   (routeur JSON-RPC, registry services) │             │
│  └──────────────────┬─────────────────────┘             │
│                     │ Scheme IPC (mcp:)                  │
├─────────────────────┼───────────────────────────────────┤
│                  COUCHE 1 : NOYAU                        │
│                     │                                    │
│  ┌──────────────────┴──────────────────┐                │
│  │  Kernel Redox (fork ACOS)            │                │
│  │  ├── Root scheme (`:`)               │                │
│  │  ├── Scheduler (→ agent-aware)       │                │
│  │  ├── Memory manager                  │                │
│  │  ├── Syscalls                        │                │
│  │  └── Schemes kernel (pipe, event...) │                │
│  └─────────────────────────────────────┘                │
│                                                          │
│  ┌──────────────┐ ┌──────────┐ ┌──────────┐            │
│  │ Bootloader   │ │ relibc   │ │ Drivers  │            │
│  └──────────────┘ └──────────┘ └──────────┘            │
└─────────────────────────────────────────────────────────┘
```

### 3.2. Ce qu'on garde, modifie et remplace

**GARDE (tel quel) :**
- Kernel Redox (scheduling, mémoire, syscalls)
- Bootloader
- relibc (libc Rust pour Redox)
- Drivers hardware (nvmed, ahcid)
- base-initfs (init minimal, logd, randd)

**MODIFIE (dans le kernel) :**
- **Scheduler** → prioriser les tâches IA (inference bursts, tensor ops)
- **Root scheme** → optimiser le routage MCP (fast-path pour `mcp:`)
- **Branding** → ACOS au lieu de Redox dans os-release, boot messages

**REMPLACE (tout le userspace) :**
- `ipcd` → **mcpd** (IPC natif MCP)
- `ptyd` → **mcpd** terminal service
- `smolnetd` → **mcpd** network service
- `getty` → **mcpd** console/auth service
- `ion` → gardé temporairement pour debug, remplacé à terme par un shell MCP

**AJOUTE (nouveau) :**
- **LLM Superviseur** — le cerveau IA local
- **Service `file` sémantique** — graphe de connaissances au lieu d'arborescence
- **Service `ui`** — interface web via Servo/WASM (Phase 2+)

---

## 4. Le LLM Superviseur — Le cerveau d'ACOS

### 4.1. Le problème à résoudre

Un OS AI-centric sans IA intégrée n'est qu'un micro-noyau de plus. La pièce manquante critique est un LLM qui tourne **nativement dans l'OS** et qui :
- Écoute le bus MCP en permanence
- Comprend les intentions utilisateur en langage naturel
- Orchestre les services système (fichiers, réseau, processus)
- Apprend et s'adapte au comportement de l'utilisateur

### 4.2. Options pour le LLM local

| Option | Taille | Runtime | Avantages | Inconvénients |
|---|---|---|---|---|
| **llama.cpp** (GGUF) | 1-7B params | C++, CPU/GPU | Mature, portable, petit | Pas Rust natif |
| **Candle** (HF) | 1-7B params | Rust pur | 100% Rust, s'intègre parfaitement | Moins mature |
| **burn** | 1-7B params | Rust pur | Backend flexible (CPU/GPU/WASM) | Plus jeune |
| **API distante** | illimité | HTTP | Puissant | Nécessite réseau |

**Recommandation :** Commencer par **llama.cpp** cross-compilé pour Redox (C++ avec relibc devrait fonctionner via le toolchain existant), puis migrer vers **Candle** (Rust pur) quand on aura validé l'architecture.

### 4.3. Architecture du superviseur IA

```
┌─────────────────────────────────────────────┐
│              acosd (AI Supervisor)            │
│                                              │
│  ┌──────────────────┐  ┌────────────────┐   │
│  │  LLM Engine       │  │  MCP Client    │   │
│  │  (llama.cpp ou    │  │  (écoute le    │   │
│  │   Candle)         │  │   bus mcp:)    │   │
│  └────────┬─────────┘  └───────┬────────┘   │
│           │                    │             │
│  ┌────────┴────────────────────┴────────┐   │
│  │         Contexte & Mémoire            │   │
│  │  - Historique des conversations       │   │
│  │  - État du système (via mcp://system) │   │
│  │  - Préférences utilisateur            │   │
│  │  - Graphe de connaissances local      │   │
│  └──────────────────────────────────────┘   │
└──────────────────────┬──────────────────────┘
                       │ JSON-RPC
                       ▼
              mcp://ai/supervisor
```

**Flux typique :**
1. L'utilisateur tape ou dit : *"Montre-moi les fichiers modifiés aujourd'hui"*
2. Le message arrive sur `mcp://ai/query`
3. Le LLM parse l'intention → tool call `mcp://system/files?modified=today`
4. Le service `file` répond avec la liste
5. Le LLM formate la réponse et l'envoie sur `mcp://ui/display`

### 4.4. Spécialisation du LLM pour ACOS

Le LLM ne doit pas être un chatbot généraliste. Il doit être **spécialisé** pour :
- Comprendre les schemes MCP et les syscalls Redox
- Générer des commandes MCP valides (JSON-RPC)
- Diagnostiquer les erreurs kernel/userspace
- Optimiser l'allocation de ressources

**Méthode de spécialisation :** Fine-tuning sur un dataset de :
- Commandes MCP ↔ actions système
- Logs kernel ↔ diagnostics
- Questions utilisateur ↔ séquences de tool calls

Ce dataset sera **généré par AutoResearch** : l'agent itère sur les prompts système du LLM, mesure la précision des tool calls, et optimise.

---

## 5. Stratégie AutoResearch Multi-Branches

### 5.1. Le principe : exploration arborescente

```
                    main (alpha)
                        │
            ┌───────────┼───────────┐
            │           │           │
        beta/mcp    beta/llm    beta/sched
        (IPC perf)  (LLM integ) (scheduler)
            │           │           │
        ┌───┴───┐   ┌───┴───┐      │
        │       │   │       │      │
     b/mcp   b/mcp  b/llm   b/llm  ...
     /json   /bin   /candle /llama
     (format) (proto)(engine)(engine)
```

**Règles :**
- `main` (alpha) = la version stable, bootable, testée
- `beta/*` = branches d'exploration pour chaque composant
- Sous-branches = hypothèses concurrentes testées en parallèle
- **Merge vers alpha** seulement si le score AutoResearch s'améliore
- **Abandon** si le score régresse après N itérations

### 5.2. Métriques par branche

| Branche | Métrique primaire | Métrique secondaire | Budget temps |
|---|---|---|---|
| `beta/mcp-ipc` | Latence IPC (μs) | Throughput (msg/s) | 20s/itération |
| `beta/mcp-protocol` | Conformité MCP spec (%) | Taille binaire | 20s/itération |
| `beta/llm-engine` | Tokens/seconde | RAM utilisée (MB) | 5min/itération |
| `beta/llm-prompts` | Précision tool calls (%) | Latence réponse | 2min/itération |
| `beta/scheduler` | Latence p99 sous charge (ms) | Throughput global | 30s/itération |
| `beta/fs-semantic` | Latence query (ms) | Pertinence résultats | 1min/itération |

### 5.3. Boucle d'itération (par branche)

```
┌─────────────────────────────────────────────────┐
│              BOUCLE AUTORESEARCH                 │
│                                                  │
│  1. git checkout -b beta/component-hypothesis    │
│  2. Agent modifie le code du composant           │
│  3. inject_mcpd.sh → cross-compile (10s)         │
│  4. Injecter dans image (3s)                     │
│  5. Boot QEMU + mesurer métriques (4-30s)        │
│  6. Comparer au score précédent                  │
│  7. Si mieux → commit + continuer                │
│     Si pire → revert + essayer autre chose       │
│  8. Écrire mémoire (ce qui a marché/échoué)      │
│  9. Après N rounds → merge dans alpha ou abandon │
│                                                  │
│  Temps par itération : 20s à 5min selon le test  │
│  Itérations par heure : 12 à 180                 │
│  Itérations par nuit : 100 à 1500                │
└─────────────────────────────────────────────────┘
```

### 5.4. Meta-boucle (à la MiniMax M2.7)

Au-dessus des boucles par composant, une **meta-boucle** optimise le processus lui-même :

```
┌──────────────────────────────────────────────────┐
│              META-BOUCLE                          │
│                                                   │
│  1. Analyser les résultats de toutes les branches │
│  2. Identifier les patterns de succès/échec       │
│  3. Modifier le program.md (instructions agent)   │
│  4. Modifier le harness d'évaluation              │
│  5. Réajuster les métriques et priorités          │
│  6. Lancer un nouveau cycle de branches           │
│                                                   │
│  Fréquence : après chaque batch de 10-20 rounds   │
└──────────────────────────────────────────────────┘
```

---

## 6. Roadmap par phases

### Phase 1 — Fondation ✅ (FAIT)
- [x] Workspace structuré
- [x] Composant mcp_scheme (lib Rust, 9 tests)
- [x] Daemon mcpd (Linux + cross-compilé Redox)
- [x] Image ACOS bootable (QEMU, 4s)
- [x] Harness d'évaluation fonctionnel
- [x] Procédure de build documentée

### Phase 2 — Services MCP natifs (EN COURS)
- [ ] Enregistrer le scheme `mcp:` dans le kernel Redox (via `redox_scheme` crate)
- [ ] Implémenter les méthodes MCP standard (tools/list, resources/read)
- [ ] Service `system` (processes, memory, uptime via MCP)
- [ ] Service `file` (accès FS via MCP)
- [ ] Supprimer la dépendance à `ipcd`/`ptyd` (remplacer par mcpd)
- [ ] Branding complet ACOS (kernel messages, boot screen)
- [ ] Couper toute connexion aux serveurs Redox dans le build

### Phase 3 — Intelligence (LLM local)
- [ ] Évaluer llama.cpp vs Candle pour Redox
- [ ] Cross-compiler le runtime LLM pour Redox
- [ ] Intégrer un petit modèle (1-3B params) comme daemon `acosd`
- [ ] Spécialiser le modèle pour les commandes MCP/système
- [ ] Boucle AutoResearch sur la précision des tool calls
- [ ] Shell conversationnel (remplacer ion par un prompt IA)

### Phase 4 — Autonomie
- [ ] Scheduler agent-aware (priorisation des tâches IA)
- [ ] Filesystem sémantique (graphe de connaissances au lieu de hiérarchie)
- [ ] Le superviseur IA gère les mises à jour de l'OS lui-même
- [ ] Auto-diagnostic et auto-réparation via le LLM

### Phase 5 — Interface humaine
- [ ] Servo/WASM pour l'interface web
- [ ] Toutes les apps sont des WebApps dans le sandbox Servo
- [ ] Interaction vocale via le superviseur IA
- [ ] Le DOM de Servo exposé via `mcp://ui/`

---

## 7. Configuration du build autonome (sans serveurs externes)

### Objectif : zéro dépendance réseau au runtime ET au build

**Au runtime (déjà fait) :** L'image ACOS bootée n'a aucune connexion réseau. Tout est self-contained.

**Au build (à faire) :**
1. Compiler le kernel depuis les sources locales (pas de `REPO_BINARY`)
2. Remplacer `relibc` par une version forkée localement si nécessaire
3. Tous les composants ACOS compilés localement via Podman
4. Le marketplace/registry local remplace le serveur Redox :
   ```
   recipes/
   ├── core/          ← kernel, bootloader, relibc (sources git locales)
   └── acos/          ← nos composants (mcpd, acosd, etc.)
   ```
5. Un script `build_offline.sh` qui fait tout sans réseau

---

## 8. Glossaire

| Terme | Définition dans ACOS |
|---|---|
| **Scheme** | Un URI qui identifie un service (ex: `mcp:`, `file:`, `tcp:`) |
| **Root scheme** (`:`) | Le mécanisme kernel qui enregistre les schemes userspace |
| **mcpd** | Le daemon central ACOS qui sert le scheme `mcp:` |
| **acosd** | Le superviseur IA (LLM local) — Phase 3 |
| **MCP** | Model Context Protocol — le protocole de communication agent IA ↔ système |
| **JSON-RPC 2.0** | Le format de messages utilisé par MCP |
| **AutoResearch** | Boucle autonome : modifier → compiler → tester → garder/rollback |
| **Alpha/Beta** | Alpha = branche stable, Beta = branches d'exploration |
