# APEX WS10: ACOS-MUX — Terminal Multiplexer Port

Port emux → ACOS-MUX via 4 AutoResearch Labs

## Objectif

Porter emux (multiplexeur terminal Rust) vers ACOS en adaptant les 4 crates OS-specific.
Le coeur pur-Rust (emux-vt, emux-term, emux-mux, emux-config = 11,921 LOC) reste intact.

## Source

- **Repo emux cloné** : `projects/agent_centric_os/emux_base/`
- **Spec** : `architecture/ACOS_MUX_SPEC.md`
- **Licence** : MIT — fork libre

## Architecture des Labs

```
APEX Orchestrator
  │
  ├── Lab 1: acos-mux-pty        ← PTY via ACOS scheme
  │   Scope: crates/emux-pty/src/acos.rs (nouveau) + lib.rs
  │   Métrique: cargo check + tests unitaires PTY
  │
  ├── Lab 2: acos-mux-render     ← crossterm → termion
  │   Scope: crates/emux-render/src/*.rs
  │   Métrique: cargo check + golden snapshot tests
  │
  ├── Lab 3: acos-mux-ipc        ← Transport ACOS + MCP bridge
  │   Scope: crates/emux-ipc/src/transport.rs + nouveau acos.rs
  │   Métrique: cargo check + codec roundtrip tests
  │
  └── Lab 4: acos-mux-daemon     ← Event loop ACOS
      Scope: crates/emux-daemon/src/server.rs + client.rs
      Dépend de: Labs 1, 2, 3
      Métrique: cargo check + session lifecycle tests
```

## Dépendances inter-labs

```
Lab 1 (PTY) ──┐
Lab 2 (Render)─┤──→ Lab 4 (Daemon) ──→ Integration Test
Lab 3 (IPC) ──┘
```

Labs 1, 2, 3 sont **indépendants** → parallélisables.
Lab 4 dépend des 3 premiers → séquentiel après.

## Exécution

### Phase 0: Fork & Rename
- Copier `emux_base/` → `components/acos-mux/`
- Renommer tous les crates : `emux-*` → `acos-mux-*`
- Renommer le workspace
- Valider : `cargo check` passe avant toute modification

### Phase 1: Labs parallèles (1, 2, 3)
```bash
/autoresearch_labs evolution/labs/acos-mux-pty.yaml
/autoresearch_labs evolution/labs/acos-mux-render.yaml
/autoresearch_labs evolution/labs/acos-mux-ipc.yaml
```

### Phase 2: Lab séquentiel (4)
```bash
/autoresearch_labs evolution/labs/acos-mux-daemon.yaml
```

### Phase 3: Integration
- Build complet du workspace
- Test end-to-end : daemon + client + split + PTY
- Branding ACOS (thème, status bar, config defaults)

### Phase 4: QEMU Test
- Inject dans l'image ACOS
- Boot QEMU, lancer acos-mux
- Valider : split, PTY, input routing

## Contraintes

- **ACOS, pas Redox** — cfg(target_os = "acos"), modules nommés acos.rs
- **Rust pur** — pas de nouvelle dépendance C
- **termion** comme backend render (maintenu par l'équipe ACOS/Redox)
- **Tests existants** comme filet de sécurité (1,473 tests emux)
