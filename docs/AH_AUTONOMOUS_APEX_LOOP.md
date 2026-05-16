# AH autonomous APEX loop — design & operations

> How `acos-hermes` (AH, the Codex GPT-5.5-orchestrated Hermes agent) is
> expected to work through the 61-task APEX backlog with minimal human
> intervention, given the build/test infrastructure that landed in PR #3.
>
> Read alongside `BUILDING_ACOS_ON_VPS.md` — that doc covers the build
> chain; this one covers the *autonomous work loop on top of it*.

## 1. Capability inventory (as of 2026-05-16)

AH has, today, the technical means to do every step of the
"design → code → build → test → commit → PR" cycle without human
intervention:

| Capability | Endpoint / Mechanism |
|---|---|
| Read TASKLIST + brief | `~/ACOS_TASKLIST.md`, `~/ACOS_NEXT_BACKLOG.md`, `~/.workspace/acos-redesign/wsN-*/` |
| Read source code | `~/acos/` (the hermes-side cookbook clone) |
| Edit source code | `~/acos/` (`hermes` user owns it; `tools/file_operations.py` writes) |
| Run cargo test / clippy / fmt | `acos_runner_call(endpoint="cargo", ...)` → port 8771 |
| Build a recipe (e.g. mcpd) | `acos_builder_call(endpoint="build_recipe", body={"recipe": "mcpd"})` → port 8772 |
| Build the full image | `acos_builder_call(endpoint="build_image")` |
| Boot the image in QEMU | `acos_runner_call(endpoint="qemu", body={"action": "boot-test", "accel": "tcg"})` |
| Sync hermes clone with `main` | `acos_builder_call(endpoint="sync", body={"ref": "main"})` |
| Git status / diff / log | `acos_runner_call(endpoint="git_status"|"git_diff"|"git_log")` |
| Commit + push branch | `gh` CLI installed; `GITHUB_TOKEN` in `/etc/hermes/env.list` (see CLAUDE.md §2ter) |
| Open draft PR | `gh pr create --draft --base main --head ah/<task-id>` |
| Post Discord update | Discord bot token in env; `~/.hermes/` config |

The **only** non-AH-driven step is human PR review before merge (per
`HERMES.md` §9.1: "Branches `ah/<topic>` + draft PR sur ACOS / ACOS-HERMES
(jamais merge direct)").

## 2. The loop — 3-phase rollout

### Phase 0 — Backlog audit (one-shot, ~15 min)

Most of the P0 tasks in `~/ACOS_NEXT_BACKLOG.md` were authored on
2026-05-15. Between then and 2026-05-16, Claude Code on the laptop
landed PR #2 (v0.7.0-smcp) and PR #3 (cookbook drift fixes) which closed
many WS1 / WS2 / WS3 / WS11 / WS12 tasks. The backlog file does **not**
reflect that. Phase 0 reconciles it.

**Prompt template:**

```
You are AH. The backlog ~/ACOS_NEXT_BACKLOG.md was written 2026-05-15
based on the meta-audit ACOS_COMPLETION_MATRIX.md. Since then, the
following PRs landed: v0.7.0-smcp (commits 1ccf7a3..efd5580) and PR #3
(cookbook drift). For EACH P0 task in ACOS_NEXT_BACKLOG.md:

1. acos_runner_call git_log + git_diff to verify what's in main
2. Decide: done_already / available / blocked_by:<id>
3. If done_already: include the commit hash that closes it.

Produce ~/ACOS_NEXT_BACKLOG_FILTERED.md with the same task lines but
prefixed by `[done_already commit:X]` / `[available]` / `[blocked_by:Y]`,
plus a TL;DR at top: "N tasks closed, M tasks ready, K blocked".

Post a 4-line summary to Discord #acos-hermes when done.

Do NOT modify any source code. Do NOT open any PR. This phase is
audit-only. Stop after writing FILTERED.md and posting Discord.
```

Expected output: `~/ACOS_NEXT_BACKLOG_FILTERED.md` showing maybe
20-25 tasks closed already, 25-35 truly available, 5-10 blocked.

### Phase 1 — Pilot task (one-shot, ~30-60 min)

After Phase 0, AH picks the **simplest available P0 task** from
`FILTERED.md` and runs the full coding cycle end-to-end, stopping after
opening the draft PR. Stop here for human review of the workflow
quality. This is the "kick-off" gate Khéri opens with a single Discord
message; it's not pre-fired by cron.

**Prompt template:**

```
You are AH. From ~/ACOS_NEXT_BACKLOG_FILTERED.md, pick the FIRST task
marked [available] AND tagged [priority:P0]. Execute the full cycle:

1. Print the task line you picked.
2. acos_builder_call sync ref=main to get the latest cookbook clone.
3. Read the corresponding brief (~/.workspace/acos-redesign/wsN-*/
   03_l1_implementation.md for the WS the task belongs to).
4. Create branch ah/<task-id> via gh/git (e.g. ah/ws9-m1).
5. Implement the task. Write tests if the brief specifies them.
6. acos_runner_call cargo command=test (relevant package) — must pass.
7. If the task touches a recipe, acos_builder_call build_recipe — must complete.
8. git add + commit + push (use git-commit's HEREDOC pattern from HERMES.md).
9. gh pr create --draft --base main --head ah/<task-id> --title "<task-title>" --body "<from-brief>".
10. Post the PR URL to Discord #acos-hermes.
11. STOP. Do NOT pick the next task; await Khéri's "continue" message.

Quality requirements:
  - No unused imports / no dead code (cargo clippy --deny warnings).
  - No `unsafe` unless the brief explicitly authorizes it.
  - Commit message format from HERMES.md §5 (no emoji, no Co-authored-by lines).
  - PR description references the task ID and the brief path.
  - If you hit a blocker (missing tool, missing dep, ambiguous spec),
    DO NOT improvise — post the blocker to Discord and stop.
```

### Phase 2+ — Continuous loop (autonomous, cron-triggered)

After Khéri reviews & approves the Phase 1 pilot's pattern, the loop
runs unattended. AH picks the next `[available]` task in priority
order, repeats the Phase-1 cycle, and posts a daily digest to Discord.

**Cron entry (in AH's user crontab — runs as `hermes`):**

```
# AH APEX loop — fires every 4h, AH self-limits to 1 task per fire.
0 */4 * * *  /home/hermes/scripts/ah-apex-loop.sh
```

The script just shells out to `hermes -z "$(cat ~/ACOS_APEX_LOOP_PROMPT.md)"`.
The prompt itself enforces the 1-task-per-fire cap and emits Discord
on PR creation.

**Daily digest cron (08:00 local):**

```
0 8 * * *  /home/hermes/scripts/ah-apex-digest.sh
```

Posts to Discord: open PRs (with cargo-status), tasks closed last 24h,
remaining `[available]` count, any blockers AH hit.

## 3. Guardrails (HERMES.md §9.1)

- Branch name **always** `ah/<task-id>` — never push to `main`.
- PR **always** opened as `--draft`. Khéri or Claude (laptop) flips it
  to ready-for-review after a code reading pass.
- `cargo test` on impacted packages **must** be green before push. If
  the test infrastructure itself is broken, AH posts to Discord and stops.
- `acos_builder_call build_recipe` (for tasks touching a recipe) **must**
  return `state=completed`. Build failure = stop + Discord.
- AH **does not** rebase main or force-push.
- AH **does not** modify `patches/redox/0001-0008` (the pre-existing WS1
  patches). New patches numbered `0014+`.
- AH **does not** touch `infra/acos-builder/` (the file is owned by root
  on the VPS specifically to prevent this).

## 4. Failure modes & recovery

| Symptom | Likely cause | Recovery |
|---|---|---|
| `acos_builder_call` 409 conflict | Another build in flight | Wait 30s, retry. If still blocked after 5 min, post Discord. |
| `acos_runner_call cargo` exit !=0 | Test failure or compile error in AH's diff | AH reads stderr, decides fix-or-revert. If unsure, post Discord and stop. |
| `gh pr create` fails 401 | `GITHUB_TOKEN` expired | Post Discord; Khéri rotates (see memory `reference_github_token_rotation`). Token expires 2026-07-26. |
| Task brief contradicts current code | Backlog drift (audit not done) | Run Phase 0 again. |
| Two concurrent AH fires | Cron racing or human-triggered overlap | The `MAX_CONCURRENT_BUILDS=1` cap in acos-builder rejects the second; AH retries. |

## 5. Manual triggers (Khéri-side)

```bash
# Trigger Phase 0 audit
ssh acos-hermes "sudo systemd-run --uid=hermes --gid=hermes \
  -p EnvironmentFile=/etc/hermes/env.list \
  -p WorkingDirectory=/home/hermes \
  -p Environment=HERMES_HOME=/home/hermes/.hermes \
  --pipe --wait --collect --quiet \
  /home/hermes/hermes-agent/venv/bin/hermes -z \
  \"\$(cat /home/hermes/ACOS_APEX_PHASE0_AUDIT.md)\""

# Kick off Phase 1 pilot (after Phase 0 is reviewed)
ssh acos-hermes "sudo systemd-run --uid=hermes --gid=hermes \
  -p EnvironmentFile=/etc/hermes/env.list \
  -p WorkingDirectory=/home/hermes \
  -p Environment=HERMES_HOME=/home/hermes/.hermes \
  --pipe --wait --collect --quiet \
  /home/hermes/hermes-agent/venv/bin/hermes -z \
  \"\$(cat /home/hermes/ACOS_APEX_PHASE1_PILOT.md)\""

# Enable continuous loop (Phase 2+) after pilot is reviewed
ssh acos-hermes "sudo -u hermes crontab -e"
# Add the cron entries from §2 above
```

## 6. Reverting / pausing

Pausing the loop (between fires): `sudo -u hermes crontab -r` removes
the cron. AH won't auto-fire.

Cancelling a running task: `acos_builder_call cancel` works for builds;
for the AH process itself, `sudo systemctl stop hermes-agent` and the
current `hermes -z` call dies.

Reverting all in-flight AH PRs: close them in batch with
`gh pr list --author "github-actions[bot]" --label ah-auto --json number -q '.[].number' | xargs -I {} gh pr close {}`
(replace label if the convention changes).

## 7. Quality measurement (post-pilot)

After Phase 1 ships its first 3 PRs, Khéri measures:

| Metric | Target | Action if missed |
|---|---|---|
| `cargo clippy --deny warnings` clean | 100% | Improve prompt with explicit clippy step |
| PR description references brief | 100% | Tighten prompt §1 step 9 |
| Tests written for new code | per brief spec | Update prompt with "if brief says N tests, write N tests" |
| Time to draft PR | <60 min / P0 task | Investigate which step is slow (audit log) |
| Human-required revisions per PR | <2 | Track and iterate on prompt |

After 10 PRs at target, lift the "draft only" rule for low-risk WS
(docs / scripts). Code-touching tasks stay draft until further notice.
