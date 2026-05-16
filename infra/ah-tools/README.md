# AH tool integration — `acos_builder_call`

> The Hermes-agent (AH) lives in a separate repo (`MKheru/ACOS-HERMES`,
> mirror under `~/Documents/Projects/ACOS-HERMES/`). The snippet in this
> directory is the **reference copy** of the Python tool that lets AH
> drive `acos-builder.service` over HTTP. The canonical source lives in
> `tools/local_runners.py` of `MKheru/ACOS-HERMES` — keep them in sync.

## File

- `acos_builder_call.py.snippet` — append-only block to add at the end
  of `hermes-agent/tools/local_runners.py`. Reuses the `_http_post_json`
  and `_http_get_json` helpers that already exist in that file alongside
  the `acos_runner_call` tool. The schema is registered under the
  `terminal` toolset (already whitelisted in `hermes_cli/tools_config.py`).

## Install on the VPS (acos-hermes-01)

```bash
# As root (the file is owned by hermes inside the agent venv repo)
sudo -u hermes bash -c "cat \
  /home/kheru/Projects/ACOS/infra/ah-tools/acos_builder_call.py.snippet \
  >> /home/hermes/hermes-agent/tools/local_runners.py"

# Restart hermes-agent so AH picks up the new tool
sudo systemctl restart hermes-agent.service
```

## Smoke test (from hermes user shell)

```bash
sudo -u hermes python3 -c "
import sys
sys.path.insert(0, '/home/hermes/hermes-agent')
from tools.local_runners import acos_builder_check, acos_builder_call
print('check =', acos_builder_check())
print(acos_builder_call({'endpoint': 'health'}))
"
```

Expected output: `check = True` and a JSON body with
`"status": "ok"` and `"service": "acos-builder"`.

## What this tool exposes to AH

| Endpoint | Effect | Body example |
|---|---|---|
| `health` | Liveness + tool inventory | `{}` |
| `build_image` | `make all CONFIG_NAME=acos-bare CI=1` — async | `{"timeout_s": 5400}` |
| `build_recipe` | `make {r,cr,ucr}.<recipe> CI=1` — async | `{"recipe": "mcpd", "clean": "r"}` |
| `sync` | `git fetch + checkout + pull + setup-acos.sh` | `{"ref": "main"}` |
| `list_jobs` | Every job in the in-memory ring | `{}` |
| `job_status` | One job's full state | `{"id": "<32hex>"}` |
| `job_log` | Tail of build log | `{"id": "<32hex>", "bytes": 32000}` |
| `cancel` | SIGTERM the job's process tree | `{"id": "<32hex>"}` |

POST endpoints return immediately with `{id, kind, state: "pending"}`;
poll `job_status` until `state ∈ {completed, failed, cancelled, timeout}`.

## When to update the canonical (ACOS-HERMES repo) vs this snippet

- **Add an endpoint or change schema** → edit `tools/local_runners.py` in
  `MKheru/ACOS-HERMES`, then update this snippet from the same diff.
- **Add a tool unrelated to acos-builder** → goes in ACOS-HERMES only, not
  here.
- **Change the server contract** (port, endpoint shape, error format) →
  update `infra/acos-builder/server.py` here AND the snippet AND the
  canonical hermes-agent tool, in one PR pair.
