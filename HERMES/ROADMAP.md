# ROADMAP ACOS-HERMES

> Plan global de durcissement et évolution de Hermes Agent (NousResearch) en agent dédié au projet ACOS, sécurisé et fork-able.
> Document maître : `/home/ankheru/Documents/Projects/ACOS/HERMES/ROADMAP.md`
> Documents associés :
>  - `AUDIT.md` — cartographie des risques (2026-04-27, 23 modules, 5 HAUTS / 8 MOYENS / 10 BAS)
>  - `HARDENING_PLAN.md` — détail tactique des patches avec code
>  - `LAB_MCP_SECURITY.md` — brief AutoResearchClaw (à venir)

---

## Vision

Hermes Agent (`NousResearch/hermes-agent`) est un agent IA opensource qui parle aux LLM via OpenRouter et opère via Discord/Slack/Telegram avec des tools (terminal, file, MCP, etc.). On l'utilise comme **AH (ACOS Hermes Agent)** — assistant de développement pour le projet ACOS.

**Constat de l'audit (2026-04-27)** : Hermes upstream a une couche de sécurité de base (redact, tirith, allowlist Discord IN) mais **ne traite pas l'injection MCP** ni le **filtrage outbound**. Avec les modèles chinois opaques (MiniMax/DeepSeek/GLM/Qwen) et un contexte ACOS sensible (tokens GitHub, Discord, Hetzner), c'est insuffisant.

**Décision architecturale** : on fork `NousResearch/hermes-agent` en **`MKheru/ACOS-HERMES`**, on patch en commits atomiques, on cherry-pick l'upstream sélectivement.

**Triple objectif** :
1. Court terme — Hardener AH pour qu'il reste un assistant fiable, sans risque de leak ou self-modif
2. Moyen terme — Produire un **MCP Security Framework** réutilisable (proxy MCP-guard) via un lab AutoResearchClaw
3. Long terme — Capitaliser ces apprentissages dans la conception de **Guardian (WS9)** d'ACOS, où le bus MCP kernel devient le policy enforcement point central

---

## Phase 1 — Audit ✅ TERMINÉE (2026-04-27)

### Livrables
- ✅ Code source Hermes rapatrié localement (`HERMES/source/`, 49 MB Python)
- ✅ Audit méthodique 23 modules → `HERMES/AUDIT.md`
- ✅ 5 risques HAUT identifiés, 8 MOYEN, 10 BAS, top 5 patches priorisés

### Findings clés
- 🔴 **MCP output injection** non filtré (`tools/mcp_tool.py:1948-1950`)
- 🔴 **Discord outbound** non filtré (`gateway/platforms/discord.py:669`)
- 🟡 MCP sampling non capé, file write non protégé sur `~/.hermes/`, env var leak via MCP outputs

---

## Phase 2 — Création du fork + plan tactique 🟡 EN COURS

### Tâches
| # | Tâche | Effort | Dépendances |
|---|---|---|---|
| 2.1 | Documenter le plan de hardening (`HARDENING_PLAN.md`) | 0.5 jour | Audit ✅ |
| 2.2 | Validation Khéri sur prio + scope | 30 min | 2.1 |
| 2.3 | Fork `NousResearch/hermes-agent` → `MKheru/ACOS-HERMES` (commit pin 755a2804) | 30 min | 2.2 |
| 2.4 | Setup branches : `acos-base-755a2804` (frozen), `acos-main` (dev), tag releases | 30 min | 2.3 |
| 2.5 | README + CONTRIBUTING + LICENSE override (rester MIT, créditer NousResearch) | 1h | 2.3 |

### Critère de fin de phase
- Repo `MKheru/ACOS-HERMES` créé, fork visible, README clair
- `HARDENING_PLAN.md` validé par Khéri

---

## Phase 3 — Patches de hardening 📋 PLANIFIÉE

### Sous-phases parallèles

#### Phase 3A — Patches courts (1 jour)
Risques 🟡 MOYENS, chirurgicaux, peu de risque de régression :

| Patch | Fichier | Effort |
|---|---|---|
| 3.1 | Étendre `file_tools.py` blocklist : `~/.hermes/`, `*.env*`, `~/.ssh/*` | 0.25j |
| 3.2 | MCP sampling rate limit + audit (10 req/min, log par appel) | 0.5j |
| 3.3 | MCP env var validation (refuser si nom contient TOKEN/KEY/SECRET) | 0.25j |

#### Phase 3B — Patches profonds (1.5-2 jours)
Risques 🔴 HAUTS, demandent design soigné :

| Patch | Fichier | Effort |
|---|---|---|
| 3.4 | **MCP Output Injection Scanner** — wrapper sanitize sur tous les outputs MCP avant injection au LLM | 1j |
| 3.5 | **Discord Outbound Filter** — pre-send hook qui passe redact + scan injection + canal whitelist | 0.5j |

#### Phase 3C — Tests + intégration (0.5 jour)
- Tests unitaires sur les 5 patches
- Test d'intégration : AH boot, prompt simple, MCP Jina round-trip, Discord echo
- Régression sur les 300+ tests Hermes existants

### Critère de fin de phase
- 5 commits atomiques mergés sur `acos-main`
- Tests verts (existants + nouveaux)
- AH déployé sur acos-hermes-01 avec le fork ACOS-HERMES

### Coût estimé Phase 3
- ~3 jours dev (Claude + Khéri review)
- ~1 € OpenRouter pour les tests itératifs

---

## Phase 4 — Lab AutoResearchClaw "MCP Security Framework" 🔬 À DESIGN

### Objectif

Détourner ARC (conçu pour idée → paper) en mode **adversarial security research** :
- **Generator** code un proxy MCP-guard générique (Python, ~500 LOC)
- **Adversary** code 50+ scénarios d'attaque (output injection, tool description injection, sampling abuse, unicode obfuscation, hidden chars, etc.)
- **Evaluator** mesure detection_rate, false_positive_rate, latency_overhead, sur des MCP servers réels (Jina + 2-3 autres en sandbox)
- **PIVOT/REFINE** : si detection_rate < 95%, ARC pivote vers une approche différente

### Pré-requis
- Phase 1 + 2 ✅ (threat model AUDIT.md)
- OAuth Gemini fait (pour tourner ARC en gratuit) OU budget OpenRouter validé (~10-15€)
- `LAB_MCP_SECURITY.md` rédigé (brief précis pour ARC)

### Livrables
1. **Code Python** : `acos-mcp-guard.py` — proxy générique, drop-in pour n'importe quel MCP client
2. **Threat model document** : ~30 pages structurées, format académique
3. **Test suite** : 50+ scenarios d'attaque réplicables
4. **Benchmark report** : detection_rate, FPR, latency par catégorie d'attaque
5. **Paper draft** : potentiellement publiable arxiv preprint

### Coût estimé
- Sur Gemini CLI subscription : **0€** marginal (limites quota), 4-8h de tourner
- Sur OpenRouter MiniMax : **8-15€** pour un lab 23 stages complets

### Critère de fin de phase
- Proxy MCP-guard fonctionnel, intégré à ACOS-HERMES (Phase 3 patch consolidé)
- Detection rate ≥ 95% sur la suite d'attaques
- Document threat model lisible par Khéri en 30 min
- Décision : **publier le paper** sur arxiv (visibilité ACOS) ou **garder privé** (avantage compétitif)

---

## Phase 5 — Capitalisation pour ACOS Guardian (WS9) 🌱 LONG TERME

### Pourquoi cette phase

ACOS = OS où **tout est MCP**. Le bus MCP kernel devient un vecteur d'attaque potentiel équivalent à ce qu'on aura traité côté Hermes. **Les apprentissages du proxy MCP-guard nourrissent directement la conception de Guardian** (WS9, prochain workstream ACOS) :

- Patterns d'injection MCP identifiés en Phase 4 → règles Guardian
- Architecture proxy/sanitizer → architecture Guardian au niveau kernel
- Threat model générique → threat model spécifique kernel-level

### Tâches
| # | Tâche | Lien |
|---|---|---|
| 5.1 | Extraire les patterns universels du proxy MCP-guard | code Phase 4 |
| 5.2 | Adapter pour le bus MCP kernel d'ACOS (Rust, no_std-compatible) | WS9 Guardian |
| 5.3 | Spec Guardian dans `docs/APEX_WS9_GUARDIAN.md` enrichie de ces patterns | ACOS roadmap |
| 5.4 | Lab AutoResearch supplémentaire : `Guardian latency optimization` | WS9.F |

### Livrables
- WS9 Guardian spec finale (à merger dans `docs/ROADMAP.md`)
- Implémentation Guardian opérationnelle dans ACOS (latence < 100ms, decision policy de base)

---

## Calendrier indicatif

```
Semaine 1                 │  Semaine 2          │  Semaine 3+
──────────────────────────┼─────────────────────┼────────────────────────
J1  ✅ Audit              │  J6-J9 Lab ARC      │  J11+ Phase 5
J2  Phase 2 (fork+plan)   │  (Phase 4)          │  WS9 Guardian
J3  Phase 3A patches      │  J10 Intégration    │  intégré à ACOS
J4  Phase 3B patches      │       proxy MCP-guard│
J5  Phase 3C tests        │       dans ACOS-     │
                          │       HERMES         │
                          │
                          │  Lab tourne en       │
                          │  arrière-plan        │
                          │  pendant Phase 3     │
                          │  si OAuth Gemini OK  │
```

---

## Indicateurs de réussite globaux

| Métrique | Cible J+30 |
|---|---|
| Risques 🔴 HAUT résolus | 5/5 |
| Risques 🟡 MOYEN traités | ≥ 6/8 |
| Tests existants Hermes (300+) qui passent toujours | 100% |
| Couverture des 5 vecteurs d'injection MCP | ≥ 95% detection rate |
| Coût LLM cumulé (OpenRouter + Gemini) | < 50 € |
| Patches ACOS-HERMES vs upstream | ≤ 15 commits (audit-able en 1h) |
| Document publishable | 1 (paper arxiv ou blog post détaillé) |

---

## Décisions ouvertes (à trancher avec Khéri)

| # | Décision | Options | Deadline |
|---|---|---|---|
| D1 | Voie de travail | A (patch d'abord) / B (lab d'abord) / C (parallèle) | Avant Phase 2.3 |
| D2 | Backend ARC | Gemini CLI gratuit / OpenRouter MiniMax payant | Avant Phase 4 |
| D3 | Repo public ou privé pour ACOS-HERMES | github.com/MKheru/ACOS-HERMES public / privé | Avant Phase 2.3 |
| D4 | Paper Phase 4 | Publish arxiv / garder privé | Fin Phase 4 |

**Ma reco synthétique** :
- D1 → **C** (parallèle), patches courts pendant que ARC tourne en background
- D2 → **Gemini CLI** gratuit (OAuth à faire par Khéri)
- D3 → **public** (cohérent avec ACOS public, fork transparent)
- D4 → **publish arxiv** si qualité au RV (visibilité ACOS, contribution open source)

---

**Document maintenu par** : Claude (assistant de Khéri)
**Dernière révision** : 2026-04-27
**Version** : 1.0
