# Phase 1 — single runtime pipeline (implementation plan + progress)

Approved 2026-06-12 (DESIGN §20). This is the **implementation-order** companion to
DESIGN §17 (design-level) + §19.1 (the six decisions) + the §17 "Phase-1 spec deltas".
Execution discipline: **expand-contract, whole-workspace-green per commit** — add the new
total function/type first (green), migrate consumers, delete the old branch **last**; no
shims. Commit each green increment.

## Ordered increments

Each row is one (or few) green commits. "Blast radius" = crates the change cascades into.

| # | Increment | Core change | Blast radius | DESIGN ref |
|---|-----------|-------------|--------------|------------|
| **1a** | `Decision` + `decide_refresh` foundation | Add a total, **pure**, time-free `Decision { Usable, Refresh, Reacquire, Revalidate, Dead }` + `CredentialPolicy::decide_refresh(last_validated, now, floor, early_refresh)`; mandatory re-validation floor (owner ruling — even static past `floor` → `Revalidate`). Additive only. | credential | §17 F2/Q8, §19.1 |
| **1b** | Route the resolver through `decide_refresh`; **delete `C::KEY != OAuth2Credential::KEY`** | resolver consults `decide_refresh` for every credential; the hardcoded key compare (`resolver.rs:536`) and the ad-hoc `state.expires_at()` routing (`:209/:236/:351`) go away; jitter stays at the scheduler seam (pure decision, non-zero jitter applied once outside) | credential | §17 Q8, Finding 1+2 |
| **1c** | Macro: synthesized `policy` must read state | `credential_attr.rs:435` `fn policy(_state)` stops ignoring `_state`; arch-test greps for any `*Credential::KEY` compare in `runtime/` | credential, macros | §17 F2 |
| **2** | `OwnerScopedKey` — close the confused-deputy (**priority #1**) | `CredentialStore::{get,delete,exists}` take a privately-constructed `OwnerScopedKey` (length-prefixed owner+id); no unscoped `get(&str)`; resolver receives a **validated binding**, not a raw id | credential, storage, tenancy, engine, tests | §10 rule 9, §17 |
| **3** | `ensure_local_source` into the resolver tail + tombstone reject | move the source check from the 4 facade sites into `resolve_for_slot`'s tail (`External` → `Unsupported`); binding-validation rejects a tombstoned id with typed `CredentialTombstoned` before a guard exists; **no `references()` port** | credential, engine | §17 Q9/Q10 |
| **4** | Framework lease handle + constructor-enforced staleness ceiling | resolver returns a lease handle, never a raw `&Secret`; ceiling is a constructor-validated bound (`Duration::MAX` unconstructible on the lease path); arch-test: no `&Secret` reachable except via the handle | credential | §17 F2 |
| **5** | `Scheme` sealed trait + per-protocol marker + `Slot<S: Scheme>` (F3) | binding axis becomes nominal; Stripe→Twilio bind = compile error; no `Box<dyn>`/catch-all; registration-time family-soundness check | credential, resource, macros | §17 F3 |
| **6** | OAuth2 grant discriminant (Q2 rider) | `OAuth2State` carries a grant discriminant; `client_credentials` re-acquires non-interactively, `device_code` reauths interactively — not a shared `ReauthRequired` | credential | §17 Q2 |
| **7** | Observability + scale DoD | read/material-access fail-closed audit; provider-returned-string redaction (§10 rules 18/19); generic store/transport/`ExternalProvider` contract-suite; contender blocks on claim watch/notify (`claims_exhausted == 0`, 7s IdP + 30 contenders) | credential | §17 DoD |

Phases 2–5 (DESIGN §17) follow after these land.

## Decision semantics (1a)

`decide_refresh` is **pure** (no clock read, no `rand` — `now` and `floor` are inputs;
jitter is applied once at the scheduler seam, never here — §24 invariant):

- inline expiry passed (`expires_at <= now`) → `Refresh` if auto-renewable, else `Reacquire`
- within the early-refresh window (`expires_at - now <= early_refresh`) → `Refresh` if
  auto-renewable, else `Usable` (let it ride until expiry; nothing to renew)
- renewable lease near/at TTL → `Refresh`; non-renewable lease expired → `Reacquire`
- no expiry, no lease, past the mandatory floor (`now - last_validated > floor`):
  - auto-renewable → `Refresh`
  - static / non-renewable → `Revalidate` (owner ruling: no "valid forever"; probe via `Testable`)
- otherwise → `Usable`
- (`Dead` is set by the caller from a revoked tombstone / terminal `invalid_grant`, not by
  `decide_refresh` — it has no access to that signal.)

## Progress log (fact only)

- 2026-06-12: baseline `cargo check -p nebula-credential --all-targets` green (35s).
- 2026-06-12: **increment 1a landed** (`7ae102f7`). `Decision` enum + pure
  `CredentialPolicy::decide_refresh` in `lifecycle.rs`; 7 unit tests green; crate clippy
  clean. Additive — no consumer yet. Next: **1b** — migrate `runtime/resolver.rs`
  (`resolve_with_refresh`, `:209/:236/:351`) onto `decide_refresh` and delete the
  `C::KEY != OAuth2Credential::KEY` branch at `:536`.
