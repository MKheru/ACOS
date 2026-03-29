# WS10: Rich Interface — Architecture Overview

Transform ACOS from a text-based OS to one with rich UI: integrated web browser, voice interface, graphical system dashboard, and visual customization.

## Prérequis

- **WS7** (Konsole) — le display manager gère les Konsoles
- **WS8** (mcp-talk) — le terminal conversationnel est l'interface principale
- **WS9** (Guardian) — le monitoring autonome est en place

---

## Architecture Cible

```
┌─────────────────────────────────────────────────────────────────────┐
│  ACOS avec Rich Interface (WS10)                                      │
│                                                                       │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │                    DisplayHandler                             │    │
│  │  ┌─────────────┐  ┌──────────────┐  ┌────────────────────┐  │    │
│  │  │ Konsole 0   │  │ Konsole 1    │  │ Konsole 2          │  │    │
│  │  │ Guardian    │  │ mcp-talk     │  │ Servo Browser      │  │    │
│  │  │ (text)      │  │ (text)       │  │ (HTML/CSS/JS)      │  │    │
│  │  └─────────────┘  └──────────────┘  └────────────────────┘  │    │
│  └──────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  ┌────────────────────┐  ┌───────────────────┐                      │
│  │ Voice Pipeline     │  │ Theme Engine       │                      │
│  │ STT → MCP → TTS    │  │ mcp:ui/theme       │                      │
│  └────────────────────┘  └───────────────────┘                      │
│                                                                       │
│  New MCP Services:                                                    │
│  mcp:ui/dom        — Servo DOM access                                │
│  mcp:ui/render     — Servo page rendering                            │
│  mcp:ui/theme      — Visual customization                            │
│  mcp:voice/listen  — Speech-to-text                                  │
│  mcp:voice/speak   — Text-to-speech                                  │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Composants

### 1. Servo Browser Engine Integration

Intégrer [Servo](https://servo.org/) (moteur web en Rust) comme un type de Konsole. Une "Servo Konsole" affiche du HTML/CSS/JS au lieu de texte ANSI.

**Pourquoi Servo :**
- Écrit en Rust (même écosystème qu'ACOS)
- Embeddable (servo-embedding API)
- GPU-capable via WebGPU/WebGL
- Léger comparé à Chromium/WebKit

**MCP API :**
```
mcp://ui/dom/query {selector: "div.stats"}     → DOM nodes
mcp://ui/dom/modify {selector, attribute, value} → Modify DOM
mcp://ui/render {url: "acos://dashboard"}        → Render page
mcp://ui/screenshot                               → Capture framebuffer
```

### 2. DOM Exposure via mcp://ui/dom

L'IA peut lire et modifier le DOM de toute page affichée dans une Servo Konsole. Cela permet à l'IA de :
- Lire le contenu des pages web
- Modifier l'interface utilisateur dynamiquement
- Créer des interfaces riches à la demande
- Scraper/interagir avec des apps web

### 3. Voice Interface (STT → MCP → LLM → TTS)

Pipeline vocal complet :
```
Microphone → mcp:voice/listen (STT) → texte → mcp:talk/ask → réponse IA → mcp:voice/speak (TTS) → Haut-parleur
```

**Options STT :** Whisper.cpp (local) ou API externe
**Options TTS :** Piper (local, Rust bindings) ou API externe

### 4. System Dashboard

Un dashboard HTML/CSS/JS rendu dans une Servo Konsole :
- Métriques système en temps réel (process, memory, CPU)
- Historique des anomalies du Guardian
- Logs système avec filtrage
- Graph de l'architecture MCP services
- Accessible via `acos://dashboard`

### 5. Themes & Visual Customization

```
mcp://ui/theme/set {name: "cyberpunk"}
mcp://ui/theme/list
mcp://ui/theme/custom {colors: {bg: "#0a0a0a", fg: "#00ff41", accent: "#ff0055"}}
```

Affecte les Konsoles texte (couleurs ANSI) ET les Konsoles Servo (CSS variables).

---

## Phases & Tâches

### Phase A: Servo Integration (Fondation)

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 10.A1 | Cross-compiler Servo embedding pour ACOS (x86_64-unknown-redox) | Très dur | Dev |
| 10.A2 | Nouveau type de Konsole : ServoKonsole (framebuffer GPU) | Dur | Dev |
| 10.A3 | Renderer : Servo → framebuffer virtio-gpu | Dur | Dev |
| 10.A4 | Créer `mcp:ui/render` — charger et afficher une page HTML | Dur | Dev |
| 10.A5 | Input routing : clavier/souris vers Servo Konsole | Moyen | Dev |

### Phase B: DOM Exposure

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 10.B1 | Créer `mcp:ui/dom/query` — CSS selector → DOM nodes | Dur | Dev |
| 10.B2 | Créer `mcp:ui/dom/modify` — modifier attributs/contenu | Dur | Dev |
| 10.B3 | DOM event streaming — l'IA reçoit les events DOM | Dur | Dev |
| 10.B4 | Security : sandboxing des modifications DOM | Moyen | Dev |

### Phase C: Voice Interface

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 10.C1 | Intégrer Whisper.cpp pour STT (ou proxy vers API externe) | Dur | Dev |
| 10.C2 | Intégrer Piper TTS (ou proxy vers API externe) | Dur | Dev |
| 10.C3 | Créer `mcp:voice/listen` et `mcp:voice/speak` | Moyen | Dev |
| 10.C4 | Pipeline complet : voix → texte → IA → voix | Moyen | Dev |
| 10.C5 | Wake word detection ("Hey ACOS") | Dur | **AutoResearch** |

### Phase D: System Dashboard

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 10.D1 | Page HTML/CSS/JS du dashboard (`acos://dashboard`) | Moyen | Dev |
| 10.D2 | WebSocket bridge : données système en temps réel vers le dashboard | Moyen | Dev |
| 10.D3 | Graphiques : process timeline, memory usage, anomaly history | Moyen | Dev |
| 10.D4 | Auto-launch dashboard dans Servo Konsole au boot | Facile | Dev |

### Phase E: Themes

| # | Tâche | Complexité | Mode |
|---|---|---|---|
| 10.E1 | Theme engine : ANSI palette + CSS variables | Moyen | Dev |
| 10.E2 | Thèmes pré-définis : default, dark, cyberpunk, minimal, high-contrast | Facile | Dev |
| 10.E3 | `mcp:ui/theme` service avec set/list/custom | Facile | Dev |
| 10.E4 | Persistance des préférences via mcp:config | Facile | Dev |

---

## Risques & Décisions ouvertes

| Risque | Impact | Mitigation |
|---|---|---|
| Cross-compilation Servo pour ACOS | Bloquant | Évaluer l'effort, potentiellement commencer par un renderer HTML minimal |
| GPU dans QEMU (virtio-gpu) | Performance | Fallback software rendering |
| Taille du binaire Servo | Espace disque | Servo minimal (pas de media, pas de WebRTC) |
| Latence STT/TTS locale | UX | Commencer avec proxy API, optimiser local ensuite |

---

## Critères de succès

- [ ] Une page HTML s'affiche dans une Servo Konsole
- [ ] L'IA peut lire et modifier le DOM via MCP
- [ ] L'utilisateur peut parler à ACOS et recevoir une réponse vocale
- [ ] Le dashboard affiche les métriques système en temps réel
- [ ] Les thèmes changent l'apparence de toutes les Konsoles

---

## Dépendance WS10 dans le graphe

```
WS9 (Guardian) ──→ WS10 (Rich Interface)
     │                  ├── Phase A: Servo (indépendant, long)
     │                  ├── Phase B: DOM (dépend de A)
WS8 (mcp-talk) ────────├── Phase C: Voice (indépendant)
     │                  ├── Phase D: Dashboard (dépend de A)
WS7 (Konsole) ─────────└── Phase E: Themes (indépendant)
```
