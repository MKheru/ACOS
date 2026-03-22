# ACOS Build Journal — 2026-03-22

## Session complète : de zéro au premier boot avec mcpd

### Contexte initial
- AutoResearch de Karpathy contenait des copies de `train.py`/`prepare.py` (ML training) totalement inadaptées pour du dev OS
- Redox OS cloné dans `redox_base/` mais jamais compilé
- 13 rounds de recherche documentaire sans code fonctionnel

---

## Phase 1 : Restructuration du workspace

**Action :** Suppression des fichiers ML, création de la structure ACOS.

```
projects/agent_centric_os/
├── architecture/          ← Documents d'architecture
├── components/
│   ├── mcp_scheme/        ← Bibliothèque MCP (JSON-RPC 2.0)
│   └── mcpd/              ← Daemon MCP (binaire)
├── evolution/             ← Système AutoResearch
│   ├── loops/iterate.sh
│   ├── memory/round_*.md
│   └── results/*.tsv
├── harness/               ← Évaluation
│   ├── evaluate.py
│   └── qemu_runner.sh
├── scripts/               ← Utilitaires
│   ├── build_in_podman.sh
│   └── inject_mcpd.sh
└── redox_base/            ← Fork Redox OS
```

---

## Phase 2 : Composant MCP Scheme

**`components/mcp_scheme/`** — Bibliothèque Rust pure (pas de dépendances Redox).

### Erreurs rencontrées :
1. **`extern crate libc;`** → Erreur `rustc_private`. Fix : ajouter `libc = "0.2"` dans Cargo.toml
2. **`unused import: Value`** dans handler.rs. Fix : retirer l'import
3. **Bench manquant** → `benches/ipc_latency.rs` référencé dans Cargo.toml mais pas créé

### Résultat :
- 9 tests unitaires passent
- Score baseline harness : 399.57
- Benchmark Criterion configuré

### Harness d'évaluation (`harness/evaluate.py`) :
1. **`-Z unstable-options`** ne marche qu'en nightly → Retiré, parsing de la sortie standard
2. **Bug parsing** : `pass_count = int(...)` au lieu de `+=` → la 2e ligne "test result" (doc-tests, 0 passed) écrasait le compteur

---

## Phase 3 : Daemon mcpd

**`components/mcpd/`** — Binaire qui sert le scheme `mcp:`.

### Erreurs rencontrées :
1. **Dépendances Redox git** (`redox_scheme`, `libredox`) → Les repos GitLab Redox nécessitent une authentification. Fix : rendre les deps optionnelles via feature `redox`
2. **`scheme.close(handle)?`** → `i32` n'implémente pas `std::error::Error`. Fix : `.map_err(|e| format!("close error: {}", e))?`

### Résultat :
- Mode Linux : stdin/stdout JSON-RPC (ping → pong, echo → echo)
- Mode Redox : placeholder pour `Socket::create("mcp")`

---

## Phase 4 : Build du container Podman

**Commande :** `make build/container.tag`

### Processus :
1. Pull `docker.io/library/debian:trixie` (déjà en cache)
2. `apt-get install` ~70 paquets (GCC, QEMU, cmake, etc.) — **~10 min**
3. `podman/rustinstall.sh` — installe la toolchain Rust pour cross-compilation — **~5 min**
4. Télécharge `cbindgen`

### Erreur rencontrée :
- Aucune — le container s'est construit sans problème

---

## Phase 5 : Première compilation de l'image Redox

### Config : `acos-bare.toml`

**Première tentative — `.config` avec `PODMAN_BUILD=1` :**
```
Please unset PODMAN_BUILD=1 in .config!
make: podman: No such file or directory
```
**Erreur :** Le `.config` est monté via le volume dans le container Podman. Le Makefile dans le container re-lit `.config` et tente de relancer Podman depuis l'intérieur du container (inception).
**Fix :** Ne PAS mettre `PODMAN_BUILD=1` dans `.config`. Le Makefile host le gère via la variable d'env.

**Deuxième tentative — Config `acos-bare.toml` ultra-minimale (sans `base`) :**
Manquait `ipcd`, `ptyd`, `getty` — les daemons système de base sans lesquels Redox ne peut pas démarrer.
**Fix :** Baser sur `minimal.toml` (qui inclut `base.toml`) au lieu de tout recréer.

**Troisième tentative — Avec `mcpd` dans `[packages]` :**
```
TOML parse error at line 1, column 1
data did not match any variant of untagged enum SourceRecipe
```
**Erreur :** La recipe `[source]` était vide. Le cookbook Redox attend `git=`, `tar=`, `path=`, ou `same_as=`.
**Fix :** Utiliser `path = "recipes/other/mcpd/source"`.

**Quatrième tentative — Avec `path=` :**
```
thread 'main' (28) panicked at src/bin/repo.rs:1547:25:
slice index starts at 2 but ends at 1
```
**Erreur :** Bug dans le TUI (ratatui) du repo builder Redox. Quand `total_log_lines` ≤ 1, le calcul `total_log_lines - 1` donne 0 mais `start` peut être > 0.
**Fix :** Patché `src/bin/repo.rs` ligne 1545 pour borner `start ≤ end`.

**Cinquième tentative — Avec `git = "file:///mnt/redox/..."` :**
Le TUI capturait toute la sortie (escape codes) et bloquait dans le pipe non-TTY.
**Fix :** `CI=1` désactive le TUI (`config.rs:89: tui = Some(!env::var("CI").is_ok_and(...))`).

**Sixième tentative — `CI=1` + `REPO_BINARY=1` :**
```
cook mcpd - failed
failed to fetch: "Package mcpd does not exist in server repository"
```
**Erreur :** `REPO_BINARY=1` télécharge des packages pré-compilés depuis le serveur Redox. Notre package custom n'y existe pas.
**Fix :** Compiler séparément et injecter dans l'image.

---

## Phase 6 : Procédure finale (ce qui fonctionne)

### Étape 1 : Build de l'image de base (une seule fois)
```bash
cd redox_base
CI=1 PODMAN_BUILD=1 make all CONFIG_NAME=acos-bare
# → build/x86_64/acos-bare/harddrive.img (196 MB)
```

### Étape 2 : Injecter les sources mcpd
```bash
./scripts/inject_mcpd.sh
```

### Étape 3 : Cross-compiler mcpd pour Redox
```bash
cd redox_base
podman run --rm \
    --cap-add SYS_ADMIN --device /dev/fuse --network=host \
    --volume "$(pwd):/mnt/redox:Z" \
    --volume "$(pwd)/build/podman:/root:Z" \
    --workdir /mnt/redox/recipes/other/mcpd/source \
    redox-base \
    bash -c '
        export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
        export RUSTUP_TOOLCHAIN=redox
        cargo build --release --target x86_64-unknown-redox
    '
# → recipes/other/mcpd/source/target/x86_64-unknown-redox/release/mcpd (727K, static ELF)
```

### Étape 4 : Injecter le binaire dans l'image
```bash
MOUNT_DIR="/tmp/acos_mount"
mkdir -p "$MOUNT_DIR"
build/fstools/bin/redoxfs build/x86_64/acos-bare/harddrive.img "$MOUNT_DIR" &
sleep 2

# Copier le binaire
cp recipes/other/mcpd/source/target/x86_64-unknown-redox/release/mcpd "$MOUNT_DIR/usr/bin/mcpd"

# Init scripts
echo 'requires_weak 00_base
nowait mcpd' > "$MOUNT_DIR/usr/lib/init.d/15_mcp"

echo 'echo ACOS_BOOT_OK' > "$MOUNT_DIR/usr/lib/init.d/99_acos_ready"

# Démonter
fusermount3 -u "$MOUNT_DIR"
```

### Étape 5 : Tester
```bash
cd /path/to/agent_centric_os
./harness/qemu_runner.sh redox_base/build/x86_64/acos-bare/harddrive.img 60
# → "Boot SUCCESS at 4s"
```

### Résumé des temps :
| Étape | Durée |
|---|---|
| Container Podman (première fois) | ~15 min |
| Image Redox (première fois) | ~5 min |
| Cross-compilation mcpd | ~10 sec |
| Injection dans image | ~3 sec |
| Boot QEMU + test | ~4 sec |

### Itérations suivantes (cycle AutoResearch) :
Modifier le code → `inject_mcpd.sh` → cross-compile (10s) → inject (3s) → test (4s) = **~20 secondes par itération**
