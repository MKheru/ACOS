# ACOS-MUX — Cahier des charges v1.0

> Multiplexeur de terminal pour ACOS (Agent-Centric OS)
> Date: 2026-03-25

## Contexte

ACOS-MUX est le multiplexeur de terminal natif d'ACOS. La v1 (WS9B) était un prototype
rudimentaire : split 50/50, I/O pipé (pas de PTY), doublons d'affichage. Le port de tmux
a échoué (ncurses cross-compile bloqué sur wint_t). On repart de zéro en s'appuyant sur
un projet Rust existant compatible Redox OS.

## Contrainte technique clé

**Rust pur — zéro dépendance C.** C'est la contrainte n°1. Ncurses (C) a tué le port tmux.
Tout doit compiler pour `x86_64-unknown-redox` sans binding C.

## Must-have (P0)

- Split vertical et horizontal (au moins 2 panneaux)
- Switch de panneau actif (raccourci clavier, style tmux `Ctrl-B`)
- Gestion PTY réelle pour chaque panneau (pas de pipe I/O)
- Rendu terminal correct (séquences ANSI, couleurs, curseur)
- Redimensionnement dynamique des panneaux
- Compile pour `x86_64-unknown-redox` — Rust pur, zéro dépendance C

## Should-have (P1)

- Panneaux multiples (>2), créer/fermer dynamiquement
- Scrollback par panneau
- Barre de statut (nom panneau, heure, infos système)
- Raccourcis configurables

## Nice-to-have (P2)

- Copier/coller entre panneaux
- Recherche dans le scrollback
- Intégration MCP (panneau guardian dédié, notifications)
- Sauvegarde/restauration de layout

## Hors scope

- Auto-lancement au boot
- Compatibilité tmux (pas besoin de reprendre son protocole)

## Critères de sélection d'un projet de base

1. Écrit en Rust (pur ou quasi-pur)
2. Pas de dépendance ncurses/terminfo C
3. Gestion PTY (ou abstraction portable)
4. Activité du projet (commits récents, communauté)
5. Licence compatible (MIT/Apache/BSD)
6. Proximité avec nos P0 — moins on a à modifier, mieux c'est
