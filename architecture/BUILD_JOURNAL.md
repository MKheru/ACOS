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

---

## WS2 : MCP Bus — Native Scheme Registration (2026-03-22)

### Objectif
Transformer mcpd d'un daemon stdin/stdout en un vrai scheme Redox natif (`mcp:`).

### Pattern utilisé : randd (SchemeSync + SchemeDaemon)
- `ipcd` utilise `SchemeBlock` + `EventQueue` (complexe, async)
- `randd` utilise `SchemeSync` + `SchemeDaemon` (simple, blocking) ← **choisi**
- Boucle principale : `next_request` → `handle_sync` → `write_response`

### Erreurs cross-compilation rencontrées

**1. Crate `daemon` — chemin relatif incorrect**
```
error: Unable to update /mnt/redox/recipes/other/redox_base/recipes/core/base/source/daemon
```
- `inject_mcpd.sh` copie `components/mcpd/` → `recipes/other/mcpd/source/`
- Le chemin `../../redox_base/...` est relatif à `components/mcpd/`, pas à la destination
- **Fix :** `path = "../../../core/base/source/daemon"` (relatif depuis `recipes/other/mcpd/source/`)

**2. Crate `redox_syscall` — double import**
```
error: crate depends on `redox_syscall v0.7.3` multiple times with different names
```
- `redox_syscall` était à la fois en `[dependencies]` (optional) ET `[target.'cfg(target_os = "redox")'.dependencies]`
- **Fix :** Retirer la section `[target...]` — le feature `redox` gate tout via optional deps

**3. Crate `redox_syscall` — nom d'import**
- Le crate `redox_syscall` a `[lib] name = "syscall"` → accessible comme `use syscall::` en Rust
- Mais le dep key dans Cargo.toml doit correspondre : `syscall = { package = "redox_syscall", version = "0.7" }`
- Sinon Cargo crée un extern name `redox_syscall` qui ne match pas `use syscall::`

**4. `Error::new()` — signature i32 en v0.7**
```
error[E0308]: mismatched types — expected `i32`, found `usize`
```
- En v0.5, `Error::new()` prenait `usize`. En v0.7, c'est `i32`.
- **Fix :** `let errno = if e < 0 { -e } else { e };` (garder i32)

**5. `SchemeSync` trait — import manquant dans mcpd**
```
error[E0599]: no method named `on_close` found for struct `McpSchemeBridge`
```
- `call.handle_sync()` marche car il fait partie de l'API `Call`, pas du trait
- Mais `bridge.on_close(id)` est une méthode du trait `SchemeSync` → il faut `use redox_scheme::scheme::SchemeSync`

### qemu_runner.sh — faux positif kernel panic
Le script détecte "panic" dans la sortie série. `smolnetd` (réseau) panic avec "No network adapter" → le script conclut "KERNEL PANIC" alors que ACOS boot correctement (`ACOS_BOOT_OK` présent).

### Résultat final
- Cross-compile : **15.5s** (vs 10s en WS1, dû aux nouvelles deps redox_scheme+daemon+syscall)
- Binary : **792K** static ELF (vs 727K en WS1)
- 24 tests passent (vs 9 en WS1)
- MCP conformité : 100% (9/9 méthodes)
- Echo roundtrip latency : 436ns (host bench)

---

## WS3 : System Services — 8 services MCP fonctionnels (2026-03-22)

### Objectif
Remplacer les daemons Redox natifs par des services MCP dans mcpd. Chaque interaction système passe par `mcp://`.

### Phase A : Implémentation parallèle (APEX v2.1, 3 agents)

**3 fichiers handler créés en parallèle :**

| Fichier | Services | Handlers |
|---|---|---|
| `system_handlers.rs` (205 lignes) | system/info, process/list, memory/stats | SystemInfoHandler, ProcessHandler, MemoryHandler |
| `file_handlers.rs` (332 lignes) | file/read, file/write, file/search | FileReadHandler, FileWriteHandler, FileSearchHandler |
| `support_handlers.rs` (358 lignes) | log, config | LogHandler, ConfigHandler |

**Chaque handler implémente le trait `ServiceHandler` :**
```rust
pub trait ServiceHandler: Send {
    fn handle(&self, path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse;
    fn list_methods(&self) -> Vec<&str>;
}
```

**Dual-mode :** `#[cfg(not(target_os = "redox"))]` (host mock) et `#[cfg(target_os = "redox")]` (real data).

### Phase B : Intégration + Tests

- 3 `mod` declarations + 8 `router.register()` dans `McpScheme::new()`
- `SystemHandler` supprimé de handler.rs (remplacé par SystemInfoHandler)
- **44 tests passent** (24 existants + 20 nouveaux)
- Tests couvrent : roundtrip pour chaque service, path traversal rejection, config CRUD, log write/read

### Phase C : Review adversariale

**12 findings identifiés par 2 reviewers (security + logic) :**

| Sévérité | Findings | Exemples |
|---|---|---|
| HIGH (3) | Path traversal (abs paths), symlink-loop DoS, unconstrained write | Fixé : validate_path par components, ALLOWED_ROOT, MAX_SEARCH_DEPTH |
| MEDIUM (3) | OOM file read, Mutex unwrap panic, substring check | Fixé : 10 MiB limit, poison-safe lock, component-based check |
| LOW (6) | Error codes, info disclosure, connection ID wrap | Fixé : generic errors, correct JSON-RPC codes, free-slot search |

**Tous les 12 corrigés, 44 tests passent après fixes.**

### Phase D : Benchmarks (criterion)

| Service | Latence | < 10μs |
|---|---|---|
| memory/stats | 628 ns | ✅ |
| system/info | 939 ns | ✅ |
| process/list | 1061 ns | ✅ |
| config/set+get | 1848 ns | ✅ |
| log/write | 2473 ns | ✅ |
| file/read | 7482 ns | ✅ |
| file/write | 8578 ns | ✅ |

### Phase E : Cross-compilation + Boot

- Cross-compile : `--no-default-features --features redox` (résout le conflit `host-test` + `redox`)
- Binary : **876K** static ELF (vs 792K en WS2)
- Boot QEMU : `ACOS_BOOT_OK` en 4s

### Phase F : Bugs runtime découverts et fixés

**Bug 1 : mcpd ne démarrait pas — `notify mcpd` dans acos.toml**
- `notify` utilise le protocole `Daemon::spawn` (byte readiness)
- mcpd utilise `SchemeDaemon::new` (capability fd)
- **Fix :** `scheme mcp mcpd` dans acos.toml → utilise `SchemeDaemon::spawn`

**Bug 2 : `cat mcp:system` ne renvoyait rien**
- `cat` fait `open() → read()` mais jamais `write()`
- Le protocole scheme MCP nécessite `open() → write(request) → read(response) → close()`
- **Fix :** Créé `mcp-query` (545K), outil CLI dédié qui fait open+write+read sur le même fd

**Bug 3 : Données système à zéro (kernel, memory, processes)**
- mcpd appelle `setrens(0,0)` après `ready_sync_scheme()` → null namespace
- Les handlers ne peuvent plus lire `/scheme/sys/` au runtime
- **Fix :** Cache les données au moment de la construction des handlers (AVANT setrens)

### Phase G : AutoResearch — Formats /scheme/sys/ (Round 1)

**Méthode :** Ajout de prints diagnostiques dans mcpd AVANT setrens, capture via serial output QEMU.

**Formats réels découverts :**

| Fichier | Format | Notes |
|---|---|---|
| `/scheme/sys/uname` | 4 lignes : `OS\nversion\narch\nhash\n` | Parser : `lines()`, concat `OS-version-arch` |
| `/scheme/sys/context` | Table TSV : `PID EUID EGID STAT CPU AFFINITY TIME MEM NAME` | Parser : skip header, field[0]=PID, last=NAME |
| `/scheme/sys/memory` | **N'existe pas** | Kernel Redox n'implémente pas ce fichier |
| `/scheme/sys/uptime` | **N'existe pas** | Idem |
| `/etc/hostname` | `acos` (pas de newline) | Accessible avant setrens |

**Décision setrens :** Retiré `setrens(0,0)` pour permettre l'accès filesystem (file/read, file/write). La sécurité sera ré-adressée dans un futur workstream dédié.

### Résultat final WS3

```
mcp-query system info      → {"hostname":"acos","kernel":"ACOS-0.5.12-x86_64",...}
mcp-query process list     → [44 processus réels avec noms : init, mcpd, ion, ...]
mcp-query memory stats     → {note: "unavailable on this kernel build"}
mcp-query file read /etc/hostname → {"content":"acos","size":4}
mcp-query config set k v   → {"ok":true}
mcp-query config get k     → {"value":"v"}
mcp-query log write info "msg" src → {"ok":true,"index":0}
mcp-query mcp '{"jsonrpc":"2.0","method":"initialize","id":1}' → {capabilities, serverInfo: "acos-mcp"}
```

### Métriques finales WS3

| Métrique | Valeur |
|---|---|
| Services actifs | 10 (echo, mcp, system, process, memory, file, file_write, file_search, log, config) |
| Tests unitaires | 44 passing |
| Benchmarks | Tous < 10μs (628ns – 8578ns) |
| Binary mcpd | 876K |
| Binary mcp-query | 545K |
| Boot time | 4s |
| Security findings fixed | 12/12 |
| AutoResearch rounds | 1 (format discovery) |
| Commits | 6 (impl → review fixes → boot fix → mcp-query → cfg fix → runtime fix) |

### Outils créés durant WS3

| Outil | Rôle |
|---|---|
| `mcp-query` | CLI pour interroger les services MCP depuis ion shell |
| `mcp-diag` | Diagnostic tool (temporaire) pour dumper /scheme/sys/ |

### Leçons apprises

1. **`default = ["host-test"]` dans Cargo.toml** est dangereux : quand on ajoute `--features redox`, les deux features sont actives → conflits de symboles. Toujours utiliser `--no-default-features --features X` pour le cross-compile.
2. **Le protocole init Redox a 3 types de services** : `notify` (byte readiness), `scheme` (capability fd), `nowait` (fire-and-forget). Utiliser le mauvais type → le scheme n'est jamais enregistré.
3. **`setrens(0,0)`** coupe TOUT accès filesystem, pas seulement le réseau. Si un daemon doit lire des fichiers, soit on cache avant setrens, soit on ne l'utilise pas.
4. **Les fichiers `/scheme/sys/`** n'ont pas tous le même format et certains n'existent pas sur ce build kernel. Toujours valider le format réel avant de coder un parser.
5. **`cat` ne peut pas interroger un scheme Redox** car il ne fait pas write+read sur le même fd. Un outil dédié (mcp-query) est nécessaire.
