# Building ACOS on `acos-hermes-01` — canonical guide

> Single source of truth for full and incremental ACOS image builds on the
> VPS. Written so AH (the in-process Hermes Agent) and any human operator
> can run a build without ambiguity. Read this end-to-end before invoking
> `make all` or `make r.<recipe>` on the VPS.

**Audience**: AH (via `acos-runner.service` HTTP endpoint), Khéri (over SSH),
and any future contributor.

**Repo-of-truth**: <https://github.com/MKheru/ACOS> (`main` branch).
The fixes that unblock the build live in `patches/redox/` — they are NOT in
upstream Redox.

---

## 1. Inventory — what lives where

### 1.1 Host facts

| Item | Value |
|---|---|
| Host | `acos-hermes-01` (Hetzner CCX23, AMD EPYC-Milan, 4 vCPU / 16 GB RAM / 150 GB disk) |
| OS | Debian 12 bookworm, kernel 6.1 |
| Container runtime | Podman 4.3.1 (the `redox-base` image is ~2.15 GB) |
| QEMU | 7.2.22 (`qemu-system-x86_64`) — **TCG only** |
| `/dev/kvm` | **Not present** — Hetzner Cloud disables nested virt |
| UEFI firmware | `/usr/share/OVMF/OVMF_CODE.fd` |
| `sccache` | Not installed (build is fast enough without it; the relibc cookbook caches Rust crates) |

### 1.2 Two cookbook clones (this trips people up)

There are **two separate clones** of the ACOS repo on the VPS, owned by
different users. They are NOT symlinked.

| Path | Owner | Used by | State |
|---|---|---|---|
| `/home/kheru/Projects/ACOS/` | `kheru` | Khéri over SSH for full image builds | Has the cookbook drift fixes applied |
| `/home/hermes/acos/` | `hermes` | `acos-runner.service` for AH-driven calls | Older snapshot — needs `git pull` + setup-acos.sh re-run after each PR merge |

The two clones MUST be re-synced after every cookbook-drift-affecting merge
to `main` (see §8 "Syncing the hermes-side clone").

### 1.3 Active systemd services involved in a build

| Unit | Purpose | Endpoint |
|---|---|---|
| `acos-runner.service` | AH's HTTP runner for build / cargo / QEMU / git ops | `127.0.0.1:8771` |
| `hermes-agent.service` | AH itself (sandboxed: SECCOMP + `ReadWritePaths`) | (no HTTP, MCP only) |
| `gemini.service`, `tts.service`, `whisper.service` | Unrelated AH side-services | — |

`acos-runner.service` runs as user `hermes`, working dir `/home/hermes`, and
calls scripts inside `/home/hermes/acos/`. Anything outside `/home/hermes`
is out of its reach unless re-routed through SSH.

---

## 2. Initial setup from zero (laptop or fresh VPS)

This is the "freshly-cloned ACOS" procedure. Run once per new clone.

```bash
# 1. Clone the repo at main
git clone https://github.com/MKheru/ACOS.git ~/Projects/ACOS
cd ~/Projects/ACOS

# 2. Run the setup script. It will:
#    - clone redox-os/redox.git into base/ at BASE_COMMIT
#    - apply every patch in patches/redox/*.patch via `git am`
#    - copy config/acos-bare.toml into base/config/
#    - clone redox-os/ion.git into base/recipes/core/ion/source/ at ION_COMMIT
#    - apply patches/ion/*.patch
#    - clone redox-os/{bootloader,kernel,userutils}.git into base/recipes/core/<name>/source/
#    - symlink mcpd/ into base/recipes/other/mcpd/source
#    - copy config/recipe.toml into base/recipes/other/mcpd/
./scripts/setup-acos.sh
```

Verify the result:

```bash
cd ~/Projects/ACOS/base
git log --oneline -5     # last commits should include WS1 + cookbook drift fixes
ls recipes/core/{bootloader,kernel,userutils,ion}/source/.git   # all four must exist
```

---

## 3. Full image build (acos-bare config)

The canonical "I want a fresh harddrive.img" build. This takes 30-60 minutes
on the VPS (first run, no cache), 5-15 minutes on subsequent runs.

```bash
cd ~/Projects/ACOS/base
make all CONFIG_NAME=acos-bare CI=1
```

- `CI=1` is mandatory on a headless VPS. It disables the cookbook TUI which
  freezes on a TTY-less stdin and looks like a hang.
- The build runs through podman by default (`PODMAN_BUILD=1`). All compile
  work happens inside `localhost/redox-base:latest`.
- On success the image lands at `build/x86_64/acos-bare/harddrive.img`
  (536 MB, ISO 9660 + MBR boot).

### 3.1 Recipe-by-recipe targets

Targets useful during iteration (don't trigger a full rebuild):

| Target | Effect |
|---|---|
| `make r.<recipe>` | Cook one recipe (e.g. `make r.base-initfs`) |
| `make c.<recipe>` | Clean one recipe's `target/` |
| `make cr.<recipe>` | clean + cook the recipe |
| `make ucr.<recipe>` | unfetch + clean + cook (forces source re-clone too) |
| `make r.relibc` | Cook relibc only (special target keeps prefix sysroot) |
| `make c.relibc` | Clean relibc AND wipe prefix sysroot (heavy reset) |

### 3.2 What "CI=1" actually changes

Inside `mk/repo.mk` and the cookbook's `cook.rs`, `CI=1` flips the runner
from a `script(1)` TTY-recording wrapper to direct stdout. This:

1. Prevents the freeze on TTY-less stdin (the original symptom that
   blocked the build for 30+ minutes silently)
2. Drops the colorized progress bars (logs become greppable)
3. Has no effect on the compiled artifacts

If you don't set `CI=1`, the build can appear to hang while it's actually
waiting on a non-existent TTY.

---

## 4. The 5 cookbook drift fixes (history-of-PR-#3)

These patches live in `patches/redox/` and are applied automatically by
`setup-acos.sh`. They are what makes a fresh build work. The "relibc ABI
skew" in the meta-audit was a misdiagnosis — the real blockers were:

| Patch | Recipe | Why it was needed |
|---|---|---|
| `0009` | `core/base-initfs` | Upstream commit `c0f548f2` converted the `bootstrap` crate from `staticlib` to `#![no_main]` `[[bin]]`. Old recipe still built it as a static lib + manual `ld`. New recipe: cargo direct + `-Clink-arg=-nostartfiles` so gcc skips `crt0` (bootstrap has its own `_start`). |
| `0010` | `core/base` | `fbbootlogd` called the new one-arg `console_draw::TextScreen::new()` with zero args (E0061 at C build). Patch passes `None` via cookbook's `patches=[...]` source-patches mechanism. |
| `0011` | `core/bootloader` + `core/kernel` | `rev = "2b57738"` (bootloader) and `rev = "77e12fd"` (kernel) were rewritten away on upstream master. Drop the rev pins, track HEAD. |
| `0012` | `core/userutils` | Same rev-pin problem (`5a2674c`). |
| `0013` | `libs/ncurses` | relibc has no `wchar.h` / `wctype.h` / `wint_t`. Pass `--disable-widec` + bump the upstream tarball blake3. |

Plus a `config/acos-bare.toml` change: **drop `libevent` + `tmux`** — tmux
needs `resolv.h` (relibc gap), and a terminal multiplexer is not essential
on a dev OS.

If a fresh `setup-acos.sh` fails to apply one of these patches, upstream
moved further and the patch needs rebasing. Don't manually patch the
source tree — fix the patch file and re-run setup.

---

## 5. Inject-only flow (faster than full rebuild)

`scripts/build-inject-all.sh` does NOT rebuild the image. It assumes
`harddrive.img` already exists and overlays freshly-compiled artifacts
on top:

- Rebuilds `mcpd` (via cookbook recipe → produces an MCP daemon binary)
- Injects `mcpd` + any updated init scripts + any updated config into the
  existing `harddrive.img` using `redoxfs`

Use it when:
- You changed `mcpd/` source but nothing in the kernel / bootloader / core
- You changed `config/acos-bare.toml` `[[files]]` blocks
- You want to test a fix in < 60 seconds instead of 15+ minutes

```bash
cd ~/Projects/ACOS
./scripts/build-inject-all.sh           # injects in place
./scripts/build-inject-all.sh --rebuild # forces mcpd rebuild before inject
```

This is what AH's `POST /build` endpoint calls. So AH can quickly iterate
on mcpd / config but **cannot rebuild the kernel** without an SSH session
to `kheru@acos-hermes-01`.

---

## 6. Booting ACOS in QEMU (on the VPS, TCG)

Hetzner Cloud does not expose `/dev/kvm`. **TCG only**, which is roughly
10-30× slower than KVM. Acceptable for boot verification, not for
day-to-day iteration. Local laptop boots are KVM-accelerated.

### 6.1 Quick smoke boot (~60s)

```bash
qemu-system-x86_64 \
  -machine q35 -accel tcg -smp 2 -m 2048 \
  -vga std -display none \
  -serial file:/tmp/acos-serial.log \
  -bios /usr/share/OVMF/OVMF_CODE.fd \
  -drive file=$HOME/Projects/ACOS/base/build/x86_64/acos-bare/harddrive.img,format=raw,if=none,id=drv0 \
  -device nvme,drive=drv0,serial=ACOS \
  -net none -no-reboot &
QEMU_PID=$!
sleep 60
kill $QEMU_PID
grep -E "Redox|RedoxFS|bootloader|Hardware" /tmp/acos-serial.log
```

You should see:

```
Redox OS Bootloader 1.0.0 on x86_64/UEFI
Hardware descriptor: Acpi(...)
Looking for RedoxFS:
RedoxFS <uuid>: 509 MiB
```

Past that point, the bootloader shows a VGA-only resolution selector that
waits for `Enter`. To proceed in headless mode, use `scripts/qemu-test.py`
or `acos-runner.service`'s `/qemu` endpoint with `action=boot-test` which
both know how to send `Enter` via QMP.

### 6.2 Patching the existing qemu-test.py for TCG

`scripts/qemu-test.py` is hardcoded with `-cpu host -enable-kvm`. On the
VPS, replace those two flags with `-cpu max -accel tcg`. The QMP +
serial-PTY logic that follows works identically.

### 6.3 Boot via acos-runner (the AH path)

```bash
curl -s -X POST http://127.0.0.1:8771/qemu \
  -H 'content-type: application/json' \
  -d '{"action": "boot-test", "accel": "tcg", "timeout_s": 300}'
```

Response shape: `{"stdout": "...", "stderr": "...", "duration_s": N, "exit_code": 0|1}`.
The script handles the resolution selector + login automatically (`accel:
tcg` is honored even if the script's default was KVM).

---

## 7. AH (Hermes Agent) build access — what works today

AH cannot directly `subprocess.run("make")`. The sandbox forbids it:

- `hermes-agent.service` has `ProtectSystem=strict`, `ReadWritePaths=/home/hermes /tmp /var/lib/hermes /var/log/hermes`
- `SystemCallFilter=@system-service` is permissive enough for most syscalls,
  but binaries that touch privileged areas trigger SIGSYS
- The cookbook tree under `/home/kheru/Projects/ACOS/base/` is outside
  every `ReadWritePaths` entry — AH can neither read nor write it directly

AH talks to `acos-runner.service` (running as user `hermes`, no sandbox)
over HTTP `127.0.0.1:8771`. The runner does have shell + cargo + podman +
qemu in PATH and operates on `/home/hermes/acos/`.

### 7.1 Endpoints AH can call today

| Method+Path | What it does | Body shape |
|---|---|---|
| `GET /health` | Liveness probe + tool inventory | (none) |
| `POST /build` | Run `./scripts/build-inject-all.sh` (inject only, no fresh image) | `{"timeout_s": 1800}` |
| `POST /cargo` | `cargo {check,test,build,bench,fmt,clippy} -p <pkg>` in `mcpd/` | `{"command": "test", "package": "mcp-scheme", "args": []}` |
| `POST /preflight` | Run `scripts/test_qemu_preflight.py` | `{}` |
| `POST /qemu` | `boot-test` / `screenshot` / `vga-login` / `eval` / `preflight` | see §6.3 |
| `POST /git_status` | `git status --short` on `/home/hermes/acos` | `{}` |
| `POST /git_diff` | `git diff <args>` | `{"args": ["HEAD~1"]}` |
| `POST /git_log` | `git log --oneline -20 <args>` | `{"args": []}` |

### 7.2 What AH can do via `acos-builder.service` (added 2026-05-16)

Full image builds and single-recipe rebuilds were missing from
`acos-runner`. They now live in a **second** localhost daemon,
`acos-builder.service` (port `8772`), with an async job model — POST
returns a `job_id` immediately, AH polls `job_status` until completion.

| Method+Path | What it does | Body shape |
|---|---|---|
| `GET /health` | Liveness | (none) |
| `POST /build_image` | `make all CONFIG_NAME=acos-bare CI=1` (~30-60 min cold, ~3-5 min warm) | `{"timeout_s": 5400}` |
| `POST /build_recipe` | `make {r,cr,ucr}.<recipe> CI=1` (single recipe) | `{"recipe": "mcpd", "clean": "r", "timeout_s": 1800}` |
| `POST /sync` | `git fetch + checkout + pull + setup-acos.sh` | `{"ref": "main"}` |
| `GET /jobs` | List all jobs | (none) |
| `GET /jobs/{id}` | One job's state | (none) |
| `GET /jobs/{id}/log?bytes=N` | Tail of build log | (none) |
| `POST /jobs/{id}/cancel` | SIGTERM the job | (none) |

AH calls this from Python via `tools.local_runners.acos_builder_call`
(snippet at `infra/ah-tools/`). Workflow:

```python
# 1. Submit
r = acos_builder_call({"endpoint": "build_image", "body": {"timeout_s": 5400}})
job_id = json.loads(r)["id"]

# 2. Poll
while True:
    s = json.loads(acos_builder_call({"endpoint": "job_status", "body": {"id": job_id}}))
    if s["state"] in ("completed", "failed", "cancelled", "timeout"):
        break
    time.sleep(30)

# 3. On failure, fetch log tail
if s["state"] != "completed":
    log = json.loads(acos_builder_call({"endpoint": "job_log", "body": {"id": job_id, "bytes": 50000}}))
```

**Install on the VPS:**

```bash
cd ACOS/infra/acos-builder && sudo ./install.sh
```

Idempotent. See `infra/acos-builder/README.md` for the full reference
including the `NoNewPrivileges=no` caveat (required for FUSE/rootless podman).

**Install the AH-side tool:**

```bash
sudo -u hermes bash -c "cat ACOS/infra/ah-tools/acos_builder_call.py.snippet >> /home/hermes/hermes-agent/tools/local_runners.py"
sudo systemctl restart hermes-agent.service
```

### 7.3 Adding endpoints in the future

When AH proposes new endpoints, edit `/opt/acos-builder/server.py`,
`sudo systemctl restart acos-builder`, smoke-test with `curl`. The
service is intentionally small and `server.py` is owned by root (so AH
cannot self-modify it — a guardrail). Then push the updated `server.py`
back to `infra/acos-builder/` and PR.

---

## 8. Syncing the hermes-side clone after a PR merge

When something lands in `MKheru/ACOS` `main` that affects the cookbook —
new patch in `patches/redox/`, change to `setup-acos.sh`, change to
`config/acos-bare.toml` — the hermes-side clone must be updated:

```bash
# As root, run inside the hermes user shell
sudo -u hermes -i
cd ~/acos
git pull --ff-only origin main

# Re-run setup-acos.sh. It's idempotent — it skips already-cloned trees
# and only applies new patches.
./scripts/setup-acos.sh

# Verify
cd base
git log --oneline -5
```

If a patch in `patches/redox/` was rebased, `setup-acos.sh` will print
"Skipped (already applied)" or error out depending on whether the new
patch overlaps with what's already applied. When in doubt, re-clone:

```bash
# DESTRUCTIVE — only if you're sure
sudo -u hermes -i
mv ~/acos ~/acos.stale.$(date +%s)
git clone https://github.com/MKheru/ACOS.git ~/acos
cd ~/acos
./scripts/setup-acos.sh
```

---

## 9. Known limitations (not bugs)

| Limitation | Workaround / Future fix |
|---|---|
| No `/dev/kvm` on Hetzner Cloud → TCG only → slow boot | Local laptop boots are fast; VPS is for build + headless verify |
| No `wchar.h` / `wctype.h` / `resolv.h` in relibc | Tmux + wide-char ncurses disabled; will need a relibc patch chantier to bring them back |
| Boot message still says "Redox OS Bootloader" | Patches `0001`+`0002` rebrand was lost when source dirs were wiped; a follow-up needs `patches/kernel/` + `patches/bootloader/` trees |
| Hermes clone diverges from main on every merge | Manual `git pull && setup-acos.sh` on hermes side; an `acos-runner` `POST /sync` endpoint would automate it |
| Cookbook recipes pin upstream commit revs that get rewritten | Either drop the rev pin (track HEAD) or mirror the upstream source somewhere stable. We did the first for bootloader / kernel / userutils. |

---

## 10. Troubleshooting recipes (when it breaks again)

### 10.1 `cook X - failed: failed to fetch: ... did not appear to be a git repository`

A `file://` recipe's local source clone is missing. Either:
- The recipe-managed source dir was wiped (e.g. `recipes/core/userutils/source/`)
- `setup-acos.sh` didn't clone it (we updated it in PR #3 to clone bootloader / kernel / userutils — but a NEW file:// recipe added later will hit this)

Fix: clone the source manually OR add the repo name to `setup-acos.sh`'s loop.

### 10.2 `failed to fetch: The downloaded tar blake3 'X' is not equal to blake3 in recipe.toml`

The upstream maintainer replaced or re-published the tarball without
notice. Compare sha256 against the canonical URL first; if identical
(file unchanged, only blake3 in recipe is stale), update the recipe's
`blake3 = "..."` line.

### 10.3 Error E0061: function takes N args but 0 were supplied (inside an upstream Rust source)

Upstream API drift — a public function signature changed and we link
against the source via cookbook. Fix in a `recipes/.../source-patches/`
patch file and reference it from `recipe.toml`'s `[source].patches`
array (see `recipes/core/base/recipe.toml` + `source-patches/0001-…`
for the canonical pattern).

### 10.4 `multiple definition of '_start'` + `undefined reference to 'relibc_start_v1'`

A `#![no_main]` binary is pulling in `crt0.o` because the build line
passes `-nodefaultlibs` but not `-nostartfiles`. Both are needed. See
patch `0009` for the canonical fix in a cookbook recipe.

### 10.5 Build hangs forever with no output

99% chance you forgot `CI=1`. The cookbook TUI is waiting on a TTY that
doesn't exist on the VPS. Kill the build, re-run with `CI=1`.

### 10.6 `cook X - failed: 'X is not a git repository'`

The recipe's `[source].git` points to `file:///mnt/redox/...` but the
local clone is empty (or was reset, including `.git/`). Check
`recipes/<cat>/<recipe>/source/.git/` exists; if not, the cookbook will
keep failing until you reclone.

---

## 11. Suggested build commands cheat sheet (kheri@acos-hermes)

```bash
# Fresh full image (30-60 min)
cd ~/Projects/ACOS/base && make all CONFIG_NAME=acos-bare CI=1

# Quick mcpd-only iteration (~60s)
cd ~/Projects/ACOS && ./scripts/build-inject-all.sh

# Rebuild one recipe + inject
cd ~/Projects/ACOS/base && make cr.mcpd
cd ~/Projects/ACOS && ./scripts/inject_mcpd.sh

# Smoke-boot the image (60s)
qemu-system-x86_64 -machine q35 -accel tcg -smp 2 -m 2048 \
  -vga std -display none -serial file:/tmp/acos.log \
  -bios /usr/share/OVMF/OVMF_CODE.fd \
  -drive file=~/Projects/ACOS/base/build/x86_64/acos-bare/harddrive.img,format=raw,if=none,id=drv0 \
  -device nvme,drive=drv0,serial=ACOS -net none -no-reboot &
sleep 60; kill %1
grep -i redox /tmp/acos.log
```

## 12. AH cheat sheet (HTTP to acos-runner)

```bash
# Liveness
curl -s http://127.0.0.1:8771/health | jq

# Inject-only build
curl -s -X POST http://127.0.0.1:8771/build \
  -H 'content-type: application/json' -d '{}' | jq '{exit_code, duration_s, stdout: (.stdout | split("\n") | .[-15:])}'

# Cargo test the mcp-scheme crate
curl -s -X POST http://127.0.0.1:8771/cargo \
  -H 'content-type: application/json' \
  -d '{"command": "test", "package": "mcp-scheme", "timeout_s": 300}' | jq '{exit_code, duration_s}'

# Boot test (TCG)
curl -s -X POST http://127.0.0.1:8771/qemu \
  -H 'content-type: application/json' \
  -d '{"action": "boot-test", "accel": "tcg", "timeout_s": 300}' | jq

# Git status of hermes-side clone
curl -s -X POST http://127.0.0.1:8771/git_status -d '{}' | jq -r .stdout
```

---

## 13. When this doc is wrong

Bring up the discrepancy in chat (`#acos-hermes` on Discord, or a session
with Claude Code on the laptop), describe the symptom, paste the actual
output, and update §10 troubleshooting. Don't let drift accumulate
silently.

Last verified end-to-end: **2026-05-16** — full `make all CONFIG_NAME=acos-bare CI=1`
produced a 536 MB `harddrive.img`; UEFI bootloader confirmed running
under TCG; AH's `acos-runner.service` endpoints all return 200 OK to
`GET /health`.
