# WS10b: ACOS-MUX QEMU Integration

> Résoudre les derniers blocages pour qu'acos-mux fonctionne dans ACOS/QEMU
> Prérequis : WS10 terminé (commit 32059ba, 1551 tests, 4 labs OK)

## Contexte

acos-mux (port complet d'emux, 8 crates Rust) compile pour `x86_64-unknown-redox`, se lance dans QEMU, entre en alternate screen... puis quitte immédiatement. Le child shell `ion` meurt après `execvp`.

**Cause identifiée** : le modèle PTY de Redox (`pty:` scheme) diffère de POSIX. Quand on fait `open("pty:N")` une seconde fois, on n'obtient pas un slave — il faut comprendre le vrai modèle du `ptyd` de Redox.

**Blocage secondaire** : pas de contrôleur QEMU fiable pour automatiser les tests (QMP via socat perd la session à chaque commande).

## Phase A : Contrôleur QEMU (faire en premier)

### Objectif
Créer un script/outil capable de, depuis le host :
1. Booter ACOS dans QEMU (headless ou avec fenêtre VGA)
2. Attendre le boot complet (`ACOS_BOOT_OK` dans le serial log)
3. Se loguer automatiquement (root/password)
4. Exécuter des commandes dans le shell
5. Lire la sortie (serial log ou screendump VGA)
6. Tout ça de manière fiable et scriptable

### Options à tester (par ordre de priorité)

**Option 1 : Python `qemu.qmp` (recommandée)**
```bash
pip install qemu.qmp
```
Bibliothèque officielle QEMU. Connexion persistante, async, pas de problème de session.
Combinée avec `-serial file:` pour lire la sortie et `send-key` pour l'input.

**Option 2 : `pexpect` sur la console série**
```bash
pip install pexpect
```
Lancer QEMU avec `-serial pty` (crée un pseudo-terminal sur le host).
Utiliser `pexpect.spawn()` pour interagir comme avec SSH — pattern matching sur les prompts.
C'est la solution la plus naturelle pour un shell interactif.

**Option 3 : MCP QEMU server (`Neanderthal/mcp-qemu-vm`)**
Nécessite SSH + X11 dans la VM. Trop lourd pour ACOS minimaliste, mais à considérer si les autres échouent.

### Critère de succès Phase A
Un script `scripts/qemu-test.py` qui fait :
```
boot → login → run "echo HELLO" → vérifier "HELLO" dans la sortie → exit
```
Le tout en < 30 secondes, reproductible.

### Fichiers existants utiles
- `harness/qemu_runner.sh` — boot headless existant (fonctionne, prouvé)
- `harness/qemu_inject.sh` — injection binaire dans l'image (fonctionne)
- `scripts/qemu-control.sh` — tentative QMP (ne fonctionne PAS, problème de session)
- Image : `redox_base/build/x86_64/acos-bare/harddrive.img`
- RedoxFS mount : `redox_base/build/fstools/bin/redoxfs`

## Phase B : Fix PTY Redox (lab autoresearch)

### Problème
Le scheme `pty:` de Redox alloue des PTY via des paths comme `pty:N`.
- `open("pty:")` → crée une nouvelle paire, retourne le master fd
- `fpath(master_fd)` → retourne `/scheme/pty/N`
- `open("pty:N")` dans le child → **ne retourne PAS un slave** (comportement inconnu)

Le child fait : `open("pty:N")` OK → `dup2` sur stdin/stdout/stderr → `execvp("/usr/bin/ion")` → ion meurt immédiatement.

### Investigation TERMINÉE — Résultats

Source ptyd trouvé à : `redox_base/recipes/core/base/source/ptyd/src/`

**Modèle PTY Redox confirmé :**
- `open("pty:")` (path vide) → crée `Pty::new(id)` + `PtyControlTerm` (master, tient `Rc<Pty>`)
- `open("pty:N")` → cherche handle N, prend sa `Weak<Pty>`, crée `PtySubTerm` (slave)
- Data flow : master.write→input→mosi→slave.read / slave.write→output→miso→master.read
- `dup(fd, "pgrp")` → accès au process group
- `dup(fd, "termios")` → accès termios
- `dup(fd, "winsize")` → accès taille fenêtre

**Hypothèses vérifiées :**
- ✅ Hypothèse 2 partielle : le slave s'obtient via `open("pty:N")` — CORRECT, N est le handle_id
- ❌ Hypothèse 1 : dup n'est PAS utilisé pour le slave (dup sert à pgrp/termios/winsize)
- ❌ Hypothèse 3 : il y a bien master/slave séparés (ControlTerm vs SubTerm)

**Causes probables du crash de ion :**
1. **Weak<Pty> EOF** : SubTerm tient un `Weak<Pty>`. Si le ControlTerm (parent) drop son `Rc<Pty>`,
   la Weak échoue → slave.read() retourne Ok(0) = EOF → ion lit EOF sur stdin et quitte.
   → Le master fd doit rester vivant dans le parent tout le temps.
2. **pgrp non configuré** : ion a besoin de job control, le pgrp n'est jamais set sur le PTY.
   → Il faut `dup(slave_fd, "pgrp")` pour configurer le process group.
3. **setsid() timing** : le child fait setsid() avant d'ouvrir le slave — peut causer des problèmes.
4. **TERM variable** : ion peut ne pas supporter TERM=xterm sur la console VGA Redox.

### Lab YAML (à créer après Phase A)

```yaml
lab_id: acos-mux-qemu-pty
description: |
  Fix PTY slave handling for Redox pty: scheme.
  The child process must get a working slave fd connected to ion shell.

metric:
  command: "python3 scripts/qemu-test.py boot-and-test-mux"
  regex: "SCORE=(\\d+)"
  direction: higher
  target: 3
  unit: checks

# Score: 0=crash, 1=mux starts, 2=mux stays alive 3s, 3=shell prompt visible in PTY

files:
  - crates/acos-mux-pty/src/acos_redox.rs
  - bins/acos-mux/src/redox_compat.rs

working_dir: projects/agent_centric_os/redox_base/recipes/other/mcpd/source/acos_mux

budget:
  waves: 3
  agents_per_wave: 4
  fine_tune_rounds: 5
```

### Autres problèmes à résoudre (après le PTY)

| Problème | Fichier | Description |
|----------|---------|-------------|
| Escape sequences | `redox_compat.rs` | Flèches/Ctrl-B pas reconnus sur VGA console |
| Status bar invisible | `redox_compat.rs` | Couleurs 16-ANSI implémentées mais non testées |
| Curseur figé | `redox_compat.rs` | MoveTo ANSI peut ne pas fonctionner sur VGA Redox |
| Input lent | `redox_compat.rs` | BufReader implémenté mais stdin Redox peut bloquer |

## Cycle de build/test

```bash
# 1. Cross-compiler
cd redox_base
podman run --rm --cap-add SYS_ADMIN --device /dev/fuse --network=host \
  --volume "$(pwd):/mnt/redox:Z" --volume "$(pwd)/build/podman:/root:Z" \
  --workdir /mnt/redox/recipes/other/mcpd/source/acos_mux redox-base bash -c '
    export PATH="/mnt/redox/prefix/x86_64-unknown-redox/sysroot/bin:$PATH"
    export RUSTUP_TOOLCHAIN=redox
    cargo build --release --target x86_64-unknown-redox -p acos-mux --features acos 2>&1
  '

# 2. Injecter dans l'image
mkdir -p /tmp/acos_mount
build/fstools/bin/redoxfs build/x86_64/acos-bare/harddrive.img /tmp/acos_mount &
sleep 3
cp recipes/other/mcpd/source/acos_mux/target/x86_64-unknown-redox/release/acos-mux \
   /tmp/acos_mount/usr/bin/acos-mux
fusermount3 -u /tmp/acos_mount

# 3. Tester (via le contrôleur de la Phase A)
python3 scripts/qemu-test.py boot-and-test-mux
```

Temps total par itération : ~30 secondes.

## Contraintes

- **ACOS, pas Redox** dans le code applicatif (cfg, noms de modules, branding)
- MAIS `cfg(target_os = "redox")` dans le code de compilation (le sysroot Redox utilise ce target)
- **Rust pur** — pas de nouvelle dépendance C
- **Ne PAS modifier** `components/acos-mux/` (référence) — uniquement `redox_base/recipes/.../acos_mux/`
- **Podman** obligatoire pour la cross-compilation (toolchain Redox dans le container `redox-base`)
- **ion** est le shell par défaut (`/usr/bin/ion`), pas `/bin/sh`

## Fichiers debug dans le guest ACOS

| Fichier | Contenu |
|---------|---------|
| `/tmp/acos-mux-pty-debug.txt` | fpath result + slave path dérivé |
| `/tmp/acos-mux-child-debug.txt` | État du child process (slave open, TERM, exec) |
| `/tmp/m.log` | EMUX_LOG (si `EMUX_LOG=/tmp/m.log` défini) |
| `/tmp/acos-mux.crash` | Panic hook output |
