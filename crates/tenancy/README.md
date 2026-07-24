# nebula-tenancy

The multi-tenancy **security boundary** for Nebula. Business tier.

`nebula-tenancy` owns the *policy* of tenant isolation — it never owns the
`Scope` type itself (that is the Core-tier `nebula-storage-port` plain-data
value type, so port signatures can require it without an upward dependency).

## What it provides

- **`ScopeResolver`** — resolves an authenticated `Principal` (actor + the
  org/workspace binding it authenticated against) into the port's
  `Scope { workspace_id, org_id }`. The default `BindingScopeResolver` is a *fail-closed*
  projection: an absent workspace binding is rejected, never silently
  widened to org-only.
- **Scoping decorators** — wrappers for the enumerated general Scope-taking port traits, wrapping
  `Arc<dyn …Store>` and injecting the resolved `Scope` into every call.
  Deployment wiring gives consumers only decorated handles, so a
  confused-deputy caller cannot substitute another tenant's scope. This does
  not include credential persistence, which uses mandatory owner-bound
  selectors and a separate authority above the port.

## Threat model (normative — spec §6.1)

| Abuse case | Mitigation |
|---|---|
| Confused deputy / cross-tenant row access | Every scoped read/transition is `WHERE id = ? AND workspace_id = ? AND org_id = ?`; an id↔scope mismatch returns `NotFound` — never the row, never a distinct "denied" that leaks existence. |
| Idempotency replay-oracle | `IdempotencyStore`/`IdempotencyGuard` keys are tenant-namespaced (`{scope}:{key}`) so tenant A cannot probe or poison tenant B's dedup entry. |
| Control-queue confused deputy | `ControlQueue::enqueue` is scoped; the engine consumer re-verifies scope on `claim_pending` before dispatch. |
| Credential authority regression | The supported authenticated HTTP management path uses its own one-decision authority/controller and mandatory `CredentialSelector`; tenancy exposes no credential metadata decorator and no `None == admin` bypass. K3 still owns sole-semantic-writer closure for technical service paths. |

## Dependencies

`nebula-storage-port` (the port + `Scope` type) and `nebula-core` (ID
newtypes + the actor `Principal`). No sqlx, no adapter, no upward deps.

## Layer position

Business tier. First-party deployment wiring in `apps/server`, API/engine technical consumers,
and scoped-conformance tests depend on it only through the permitted layer direction. See the
`deny.toml` `[wrappers]` allowlist and ADR-0072.
