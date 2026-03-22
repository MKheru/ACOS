# ACOS AutoResearch Program — Agent-Centric Operating System

## Vision
ACOS est un OS "AI-First" basé sur un fork de Redox OS (micro-noyau Rust), où toute communication entre l'IA, le matériel et l'humain passe par un bus sémantique MCP natif au noyau (`mcp:` scheme).

## Méthode : Boucle Hybride (Dev Classique + AutoResearch)

Ce projet utilise deux modes de travail complémentaires :

### Mode 1 : Développement Classique (APEX)
Pour l'architecture globale, les décisions structurelles, et l'intégration des composants.
- Piloté par l'humain avec l'agent comme assistant
- Commits manuels après validation

### Mode 2 : Boucle AutoResearch (Itération Autonome)
Pour optimiser des composants isolés avec une métrique mesurable.
- L'agent modifie le fichier cible (ex: `components/mcp_scheme/src/lib.rs`)
- Le harnais d'évaluation (`harness/evaluate.py`) compile, teste dans QEMU, mesure
- Décision automatique : garder / rollback
- Mémoire cumulative dans `evolution/memory/`

## Structure du Projet

```
projects/agent_centric_os/
├── program.md                  ← CE FICHIER (instructions agent)
├── architecture/               ← Documents d'architecture
│   └── ACOS_ARCHITECTURE.md    ← Architecture v4 (référence)
├── components/                 ← Composants développés pour ACOS
│   └── mcp_scheme/             ← Premier composant : le scheme MCP
│       ├── src/                ← Code Rust (modifiable par l'agent en mode AutoResearch)
│       ├── tests/              ← Tests unitaires et d'intégration
│       └── Cargo.toml
├── redox_base/                 ← Fork de Redox OS (base, modif prudentes)
├── evolution/                  ← Système d'auto-évolution
│   ├── loops/                  ← Scripts de boucle d'itération
│   ├── memory/                 ← Mémoire cumulative (à la MiniMax)
│   └── results/                ← Historique TSV des itérations
├── harness/                    ← Harnais d'évaluation
│   ├── evaluate.py             ← Script d'éval (compile + QEMU + score)
│   └── qemu_runner.sh          ← Lanceur QEMU headless
├── scripts/                    ← Utilitaires
│   └── build_in_podman.sh      ← Build via Podman
└── MEMORY.md                   ← Journal d'évolution (legacy, migré vers evolution/memory/)
```

## Composant Actuel : MCP Scheme (`mcp:`)

### Objectif
Implémenter un scheme handler Redox natif qui permet aux processus d'ouvrir des ressources via `mcp://service/resource`. C'est le composant fondamental d'ACOS.

### Métriques d'évaluation (pour la boucle AutoResearch)
1. **Compilation** : Le composant compile sans erreur → binaire (pass/fail)
2. **Tests unitaires** : Tous les tests passent (pass/fail)
3. **Latence IPC** : Temps de round-trip d'un message MCP en microsecondes (lower is better)
4. **Throughput** : Messages MCP par seconde (higher is better)
5. **Score composite** : `score = (1000 / latency_us) * throughput * test_pass_rate`

### Contraintes
- Tout le code du composant est en Rust
- Doit s'intégrer dans le système de schemes de Redox
- Doit supporter le format JSON-RPC du protocole MCP standard
- Les tests doivent pouvoir tourner hors-QEMU (unit tests) ET dans QEMU (integration)

## Workflow Agent

### Phase 1 : Fondation (actuelle)
1. Examiner le système de schemes de Redox OS (`redox_base/`)
2. Créer le squelette du scheme `mcp:` dans `components/mcp_scheme/`
3. Écrire le harnais d'évaluation (`harness/evaluate.py`)
4. Faire compiler le premier build minimal dans Podman
5. Booter dans QEMU headless et valider le boot

### Phase 2 : Boucle AutoResearch sur le MCP Scheme
- L'agent itère sur `components/mcp_scheme/src/lib.rs`
- Chaque itération : modifier → compiler → tester → mesurer → décider
- Mémoire cumulative dans `evolution/memory/round_N.md`

### Phase 3 : Intégration et Meta-boucle
- Intégrer le scheme dans Redox
- L'agent optimise aussi ses propres instructions (ce fichier)
