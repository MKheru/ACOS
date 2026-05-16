# WS13 Phase 1 dashboard — blocked on WS11 observability MCP

Status: blocked
Scope: WS13 Phase 1 read-only dashboard
Owner dependency: WS11 observability MCP surface

## Decision

WS13 Phase 1 dashboard must not be implemented beyond doc-only/plumbing until WS11 exposes a stable `observability.recent` MCP method.

The Phase 1 dashboard depends on live trace data for its core read-only view. Shipping a dashboard before `observability.recent` exists would force one of two bad options:

- mock data or duplicated storage outside the observability service;
- a private/non-MCP read path that bypasses the ACOS capability and trace model.

Both options create false completion signals for WS13 and make later authority/observability integration harder to verify.

## Required unblock condition

WS13 Phase 1 dashboard can move from `blocked` to `available` only after all of the following are true:

1. WS11 exposes `observability.recent` through MCP.
2. `observability.recent` returns bounded recent trace/event data suitable for a read-only dashboard.
3. Access to the endpoint is read-only and compatible with the authority/capability model.
4. A regression test or QEMU smoke marker proves the endpoint is present in the image.

## Allowed before unblock

- documentation and ADRs;
- schema design that does not claim runtime availability;
- workspace plumbing that does not expose a fake dashboard data source.

## Explicitly not allowed before unblock

- `GET /api/observability/recent` backed by mock data;
- a dashboard status of `done` or `designed` implying implementation completion;
- direct reads of internal observability storage from WS13 code bypassing MCP;
- Phase 1 UI demos that present synthetic trace data as real system state.

## Backlog link

This records the dependency for `WS13.5` from `~/ACOS_NEXT_BACKLOG_FILTERED.md`:

> Bloquer Phase 1 dashboard jusqu'à WS11 observability MCP expose `observability.recent`.
