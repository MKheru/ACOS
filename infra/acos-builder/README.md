# acos-builder.service — HTTP build orchestrator for AH

> Async, job-based HTTP daemon listening on `127.0.0.1:8772`. Exists so
> AH (whose `hermes-agent.service` has a SECCOMP filter that blocks
> subprocess fork+exec of `make`, `git`, `podman`, etc.) can still drive
> ACOS image / recipe builds via `urllib.request` to a localhost endpoint.
>
> See `docs/BUILDING_ACOS_ON_VPS.md` §7 for the bigger picture.

## Files

- `server.py` — the HTTP daemon (single file, stdlib only, ~280 lines)
- `acos-builder.service` — the systemd unit (`User=hermes`, `NoNewPrivileges=no`)
- `install.sh` — idempotent installer; copies the server to `/opt/acos-builder/`
  and the unit to `/etc/systemd/system/`, reloads systemd, enables, restarts,
  smoke-tests `/health`.

## Install

```bash
# On the VPS, as root
cd /home/<user>/path/to/ACOS/infra/acos-builder
sudo ./install.sh
```

Defaults to user `hermes`. Override with `sudo ACOS_BUILDER_USER=foo ./install.sh`
if you want a different service user (also requires `/home/foo/acos/` to exist
with the cookbook clone).

## Verify

```bash
curl -sS http://127.0.0.1:8772/health | python3 -m json.tool
```

Expect `status: ok`, `service: acos-builder`, and a `tools` map listing
`make`, `git`, `podman`, `bash` paths.

## Endpoint summary

| Method+Path | Effect | Body |
|---|---|---|
| `GET /health` | liveness | — |
| `POST /build_image` | `make all CONFIG_NAME=acos-bare CI=1` | `{timeout_s?: 60..7200}` |
| `POST /build_recipe` | `make {r,cr,ucr}.<recipe> CI=1` | `{recipe, clean?, timeout_s?}` |
| `POST /sync` | git fetch + checkout + pull + `setup-acos.sh` | `{ref?, timeout_s?}` |
| `GET /jobs` | list all jobs | — |
| `GET /jobs/{id}` | one job's state | — |
| `GET /jobs/{id}/log?bytes=N` | tail of build log | — |
| `POST /jobs/{id}/cancel` | SIGTERM the job | — |

POST endpoints return `{id, kind, state: "pending"}` immediately. The
build runs in the background; poll `/jobs/{id}` until
`state ∈ {completed, failed, cancelled, timeout}`.

## Configuration knobs (edit `server.py`)

| Constant | Default | Meaning |
|---|---|---|
| `ACOS_ROOT` | `/home/hermes/acos` | Cookbook clone path |
| `BASE_DIR` | `/home/hermes/acos/base` | Where `make all` runs |
| `LISTEN_PORT` | `8772` | TCP port |
| `MAX_CONCURRENT_BUILDS` | `1` | Block second build_image/build_recipe while one runs |
| `DEFAULT_IMAGE_BUILD_TIMEOUT` | `5400` s (90 min) | |
| `DEFAULT_RECIPE_BUILD_TIMEOUT` | `1800` s (30 min) | |
| `DEFAULT_SYNC_TIMEOUT` | `300` s (5 min) | |

## Why `NoNewPrivileges=no`

Rootless podman uses `fusermount3` for FUSE mounts. `fusermount3` is
setuid-root, so with `NoNewPrivileges=yes` the kernel refuses the
escalation and the final `mk/disk.mk` stage fails with:

```
installer: failed to install: fusermount3: mount failed: Operation not permitted
```

The unit therefore sets `NoNewPrivileges=no`. The remaining sandboxing
(`User=hermes`, `ProtectSystem=full`, scoped `ReadWritePaths`) keeps
blast radius limited.

## Known gaps / follow-ups

- `/cancel` only SIGTERMs the bash parent; podman containers spawned by
  the build keep running. Workaround: `podman kill <container-id>` from
  a `kheru` SSH session. Real fix: track podman cidfile or use
  `--pid=container` so containers die with parent.
- No job-history persistence across service restart (jobs are in-memory).
  Persisted JSON meta in `/var/lib/acos-builder/jobs/` is best-effort
  diagnostic — not consulted on startup.
- No concurrency above 1 — by design, since the cookbook serialization
  is fragile anyway. Lift `MAX_CONCURRENT_BUILDS` once you confirm
  recipe-level isolation.
