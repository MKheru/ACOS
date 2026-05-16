# WS13 Phase 2 action gateway — blocked on WS1/WS2 authority-shim API

Status: blocked
Scope: WS13 Phase 2 action gateway
Owner dependency: WS1/WS2 authority-shim API

## Decision

WS13 Phase 2 action gateway must not be implemented until the WS1/WS2 authority-shim API is stable enough to authorize every UI action intent before dispatch.

The Phase 2 gateway is the first WS13 surface that can request state-changing actions. Shipping it before the authority shim is callable from the gateway would force one of three unsafe patterns:

- optimistic allow/deny stubs that do not reflect production policy;
- direct handler dispatch from HTTP/WebSocket code without capability checks;
- a second ad-hoc authorization layer outside the ACOS authority model.

All three options would create false completion signals for WS13 and weaken the guarantee that remote UI actions are mediated by the same capability system as MCP calls.

## Required unblock condition

WS13 Phase 2 action gateway can move from `blocked` to `available` only after all of the following are true:

1. WS1/WS2 expose a stable authority-shim API callable by the WS13 backend.
2. Each `UiAction` variant maps deterministically to a `CapabilityTarget` or equivalent authority request.
3. Authorization is revalidated per action/message; long-lived sessions must not cache allow decisions.
4. Denied actions return a typed denial result and emit an audit/trace event linked to the request trace.
5. A regression test or QEMU smoke marker proves an allowed action succeeds and a denied action fails closed in the image.

## Allowed before unblock

- documentation and ADRs;
- schema work for signed action envelopes;
- closed `UiAction` enum design;
- `/health`-only workspace plumbing that exposes no action endpoint.

## Explicitly not allowed before unblock

- `POST /api/actions/request` wired to handlers without authority-shim mediation;
- generic `execute`, `rpc`, `method`, shell, or free-form command endpoints;
- cached authorization decisions for a session after capability revocation;
- fallback allow behavior when authority lookup fails;
- demos that mark Phase 2 action execution as implemented while using stubs.

## Backlog link

This records the dependency for `WS13.6` from `~/ACOS_NEXT_BACKLOG_FILTERED.md`:

> Bloquer Phase 2 action gateway jusqu'à WS1/WS2 authority-shim API.
