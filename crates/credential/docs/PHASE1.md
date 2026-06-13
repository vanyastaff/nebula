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
| **1b** | Route the resolver decision through `decide_refresh` | resolver consults `C::policy(&state).decide_refresh(...)` for the early-refresh/serve decision instead of the ad-hoc inline `state.expires_at()` + jitter test; `decide_refresh` is now a production consumer (closes "policy has zero consumers"); hot-path jitter dropped (scheduler-seam concern). **Reclassified:** deleting the `C::KEY != OAuth2Credential::KEY` compare (`resolver.rs:536`) is **NOT a Phase-1 item** — see note below. | credential | §17 Q8, Finding 1 |
| **1c** | Macro: synthesized `policy` reads state | `credential_attr.rs` `fn policy(state)` now surfaces `CredentialState::expires_at(state)` instead of a constant `None`, so a refreshable credential routes on real expiry | credential, macros | §17 F2 |
| **2** | `OwnerScopedKey` — close the confused-deputy (**priority #1**) | `CredentialStore::{get,delete,exists}` take a privately-constructed `OwnerScopedKey` (length-prefixed owner+id); no unscoped `get(&str)`; resolver receives a **validated binding**, not a raw id | credential, storage, tenancy, engine, tests | §10 rule 9, §17 |
| **3** | `ensure_local_source` into the resolver tail + tombstone reject | move the source check from the 4 facade sites into `resolve_for_slot`'s tail (`External` → `Unsupported`); binding-validation rejects a tombstoned id with typed `CredentialTombstoned` before a guard exists; **no `references()` port** | credential, engine | §17 Q9/Q10 |
| **4** | Framework lease handle + constructor-enforced staleness ceiling | resolver returns a lease handle, never a raw `&Secret`; ceiling is a constructor-validated bound (`Duration::MAX` unconstructible on the lease path); arch-test: no `&Secret` reachable except via the handle | credential | §17 F2 |
| **5** | `Scheme` sealed trait + per-protocol marker + `Slot<S: Scheme>` (F3) | binding axis becomes nominal; Stripe→Twilio bind = compile error; no `Box<dyn>`/catch-all; registration-time family-soundness check | credential, resource, macros | §17 F3 |
| **6** | OAuth2 grant discriminant (Q2 rider) | `OAuth2State` carries a grant discriminant; `client_credentials` re-acquires non-interactively, `device_code` reauths interactively — not a shared `ReauthRequired` | credential | §17 Q2 |
| **7** | Observability + scale DoD | read/material-access fail-closed audit; provider-returned-string redaction (§10 rules 18/19); generic store/transport/`ExternalProvider` contract-suite; contender blocks on claim watch/notify (`claims_exhausted == 0`, 7s IdP + 30 contenders) | credential | §17 DoD |

Phases 2–5 (DESIGN §17) follow after these land.

### Reclassification: the `C::KEY` compare is Phase 3, not Phase 1

DESIGN §17's Phase-1 DoD listed "delete the `C::KEY != OAuth2Credential::KEY`
hardcode". Reading the as-built showed that compare is **not** a routing decision —
it lives in `perform_refresh::try_oauth2_refresh` (`resolver.rs:536`, rotation-gated)
and is the *refresh mechanism*: it is currently the **only** path that actually
refreshes an OAuth2 credential, by calling `refresh_oauth2_state(state, transport)`
with the resolver-injected transport. `OAuth2Credential::refresh` itself is
deliberately disabled (`oauth2.rs:585` returns `oauth2_http_transport_disabled()`;
HTTP moved to the engine per ADR-0031). So deleting the compare with no replacement
would break OAuth2 refresh outright (`api/tests/e2e_oauth2_flow.rs:302`).

Removing it cleanly requires the OAuth2 transport-injection redesign — `OAuth2Credential`
refreshing through its own `Refreshable::refresh` (grant-discriminant-driven, Q2) with
a transport reachable from the trait, so the resolver no longer special-cases by key.
That is **Phase 3** (protocol model + OAuth2). Tracked there; the Phase-1 routing fix
(1b) does not touch it. The `runtime/`-wide "no `*Credential::KEY` compare" arch-test
becomes a Phase-3 gate, not a Phase-1 one.

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
  clean. Additive — no consumer yet.
- 2026-06-13: **increments 1b + 1c landed.** 1c: macro synthesized `policy` reads
  `CredentialState::expires_at(state)` (was a constant `None`). 1b: `resolve_with_refresh`
  (and `scheme_factory`, facade `scheme_factory`) bound on `Refreshable + CredentialLifecycle`,
  routes the serve/refresh decision through `C::policy(&state).decide_refresh(...)` —
  `decide_refresh` is now a production consumer. Hot-path jitter dropped. 261 tests green
  (316 with rotation), clippy clean (incl. `--all-features`), `nebula-api` tests compile.
  `C::KEY` compare reclassified to Phase 3 (see note above). Next: **increment 2** —
  `OwnerScopedKey` / confused-deputy (`CredentialStore::get(&str)` at `store.rs:176`).
