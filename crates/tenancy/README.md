# nebula-tenancy

The multi-tenancy **security boundary** for Nebula. Business tier.

`nebula-tenancy` owns the *policy* of tenant isolation — it never owns the
`Scope` type itself (that is the Core-tier `nebula-storage-port` plain-data
value type, so port signatures can require it without an upward dependency).

## What it provides

- **`ScopeResolver`** — resolves an authenticated `Principal` (actor + the
  org/workspace binding it authenticated against) into the port's
  `Scope { workspace_id, org_id }`. Generalised from the credential-specific
  `ScopeLayer`. The default `BindingScopeResolver` is a *fail-closed*
  projection: an absent workspace binding is rejected, never silently
  widened to org-only.
- **Scoping decorators** — one per port store trait, wrapping
  `Arc<dyn …Store>` and injecting the resolved `Scope` into every call.
  The engine/api receive only the decorated handle; the raw adapter
  constructor is crate-private to wiring, so a confused-deputy caller is
  *structurally* unable to forge another tenant's scope.

## Threat model (normative — spec §6.1)

| Abuse case | Mitigation |
|---|---|
| Confused deputy / cross-tenant row access | Every scoped read/transition is `WHERE id = ? AND workspace_id = ? AND org_id = ?`; an id↔scope mismatch returns `NotFound` — never the row, never a distinct "denied" that leaks existence. |
| Idempotency replay-oracle | `IdempotencyStore`/`IdempotencyGuard` keys are tenant-namespaced (`{scope}:{key}`) so tenant A cannot probe or poison tenant B's dedup entry. |
| Control-queue confused deputy | `ControlQueue::enqueue` is scoped; the engine consumer re-verifies scope on `claim_pending` before dispatch. |
| Credential scope-layer regression | The re-home preserves ADR-0029 fail-closed audit + zeroize-on-drop + pending single-use/TTL/session-binding; conformance covers cross-tenant pending-replay denial. |

## Dependencies

`nebula-storage-port` (the port + `Scope` type) and `nebula-core` (ID
newtypes + the actor `Principal`). No sqlx, no adapter, no upward deps.

## Layer position

Business tier. Only composition roots depend on it (`nebula-api`
`AppState`, the engine wiring, and the scoped-conformance dev-dep). See the
`deny.toml` `[wrappers]` allowlist and ADR-0066.
