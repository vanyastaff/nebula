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
| **2** | `OwnerScopedKey` — close the confused-deputy on the slot path (**priority #1**) | `OwnerScopedKey` (privately constructed, obtainable only from a `ValidatedCredentialBinding`); `resolve_for_slot` resolves via `resolver.resolve_scoped(&key)`, which **re-verifies the stored row's `owner_id` at load** (cross-tenant id → `NotFound`, existence-hiding). Confused-deputy closed by construction on the exploit path. | credential | §10 rule 9, §17 |
| **2b** | Store-port sealing (follow-up) | Make `CredentialStore::{get,delete,exists}` themselves take `OwnerScopedKey` so the unscoped `get(&str)` primitive cannot be expressed by *any* caller (not just the slot path). Wide cascade: 5 storage decorators + tenancy `ScopeLayer` + erased wrappers + facade `load_owned` + tests. | credential, storage, tenancy, engine, tests | §17 |
| **3** | Tombstone reject (Q9) — **landed** | `revoke` writes a tombstone epoch over the row (no delete-then-upsert) so a revoked id cannot be resurrected; `validate_credential_binding` rejects a tombstoned id with the typed `CredentialTombstoned` before a guard exists; `load_owned`/`list` treat it as gone; `resolve_scoped` fails closed on the validate-then-revoke race. **No `references()` port.** | credential | §17 Q9 |
| **3b** | Facade E2E harness + remaining Q10 | Back the deferred end-to-end tests: `validate_credential_binding`-rejects-tombstoned and the `External`-source regression. **Home = api integration tests** (`crates/api/tests`), not in-credential unit tests: `from_secure_parts` needs the engine resolver + lease lifecycle + storage layer stack, which `nebula-credential` cannot depend on (a dev-dep would be a `credential→storage` cycle). **Blocker found:** the tombstone E2E needs a credential that is **non-interactive *and* Revocable**, which no first-party builtin is (`api_key`/`basic_auth` aren't Revocable; `oauth2` is interactive, so it can't be created without the OAuth handshake). So 3b also adds a custom-registry factory variant (`with_parts`/`with_registry`) + a non-interactive Revocable test credential + its revoke ops. Plus the *structural* Q10 source-awareness (resolver-tail by construction) replacing the per-call `ensure_local_source` gate landed in increment 2. | credential, api | §17 Q9/Q10 |
| **4** | Constructor-enforced lease staleness ceiling (F2) — **landed** | `StalenessCeiling` newtype whose constructor rejects zero / above-`HARD_CAP`, so `Duration::MAX` is unconstructible on the lease path; the scheduler clamps every renewal interval to it (an unbounded provider TTL still re-validates by the ceiling). "No raw `&Secret` reachable except via the guard" was **already** enforced by the `CredentialGuard` / `SchemeGuard` compile-fail tests (`compile_fail_*_guard_clone`/`_retention`/`_sensitivity`) — no new work. | credential | §17 F2 |
| **4b** | Herd-breaking renewal jitter (W2) | Apply structurally-non-zero jitter at the scheduler renewal seam so N replicas computing the same state-derived deadline don't renew in lockstep. **Not a drop-in:** it makes `renewal_interval` non-deterministic, which breaks the existing precise-timing lease tests (`tokio::time::pause` + `advance(69s/71s)`); and F2 forbids an author-`ZERO`-able jitter knob, so the rotation subsystem's `enable_jitter` bool is the wrong pattern — needs a determinism-injection seam (test RNG/clock) that keeps jitter always-on in prod. | credential | §17 F2 / W2 |
| **5** | F3 mechanics axis = open `SchemeFamily` trait (not enum) — **DECIDED (Option F, planёrka), design in DESIGN §17** | Mechanics = an **open trait sealed at the obligation** (`SchemeFamily { const EGRESS: EgressShape; fn refresh_classes(); fn pattern(); }`) + `AuthScheme::type Family`, checked at registration by a containment law — NOT a sealed enum (a sealed enum forces a breaking release on the first unshipped protocol, e.g. RFC 9421). `AuthPattern` untouched; `RefreshStrategy` relocates to core; `CredentialCategory` deleted (zero behavioural readers). Slices: **5a** core contract (`EgressShape`+`SchemeFamily`+`AuthScheme::Family`+relocate `RefreshStrategy`, transitional default) · **5b** macro emits `Family`, migrate impls, drop default · **5c** `Slot<S: AuthScheme>` + trybuild + arch-test + fix stale `type Auth` doc · **5d** registration containment-law check + typed error · **5e** *(subtractive, owner-visible)* delete `CredentialCategory` · **5f** plugin open-world example + DESIGN finalise. | core, credential, resource, macros | §17 F3 |
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
  `C::KEY` compare reclassified to Phase 3 (see note above).
- 2026-06-13: **increment 2 landed.** `OwnerScopedKey` (privately constructed, store.rs) +
  `ValidatedCredentialBinding::owner_scoped_key()` + resolver `resolve_scoped` with a
  fail-closed `verify_owner` load-time gate; `resolve_for_slot` routes through it. The
  `owner_id` metadata key is now a single shared const (`store::OWNER_ID_METADATA_KEY`,
  facade aliases it). Confused-deputy closed by construction on the slot path. 3 regression
  tests (matching / cross-tenant→NotFound / unstamped→foreign). 264 lib tests green (319 w/
  rotation), clippy clean incl `--all-features`, nebula-api + nebula-engine compile. Next:
  **increment 2b** (store-port sealing — wide cascade) or **increment 3** (ensure_local_source
  into resolver tail + tombstone reject, Q9/Q10).
- 2026-06-13: **Q10 latent defect closed** — `resolve_for_slot` now calls
  `ensure_local_source()` (it was the one secret-resolving entry point missing the gate the
  planёрka flagged); an `External`-source service fails with `ExternalSourceNotWired` instead
  of reading local bytes. Guard mirrors the 3 sibling gates (create/update/delete). Remaining
  increment-3 work: the *structural* version (source-awareness in the resolver tail rather
  than a per-call gate), the dedicated External-source regression test (needs a facade test
  harness — none exists yet), and the tombstone-reject in binding-validation (Q9).
- 2026-06-13: **increment 3 (Q9 tombstone reject) landed** (`4ea98488`; rustdoc pre-fix
  `67c57c3f`). `revoke` CAS-overwrites the row with a `revoked_at` epoch + empty secret bytes
  instead of deleting it (no resurrection, no delete-then-upsert). `StoredCredential::is_tombstoned`
  is the fail-closed liveness check (present-but-unparseable epoch still reads tombstoned).
  `validate_credential_binding` rejects a tombstoned id with the typed
  `ValidatedCredentialBindingError::CredentialTombstoned` before a binding (and thus a guard)
  exists — no `references()` port. `load_owned` maps a tombstoned row to `NotFound` (so
  get/update/test/refresh + a repeat revoke see it as gone) and `list` skips it; `resolve_scoped`
  fails closed on the validate-then-revoke race. 269 lib tests green (3 store-predicate + 2
  resolver), clippy clean incl `--all-features`, nebula-api + nebula-engine compile, rustdoc
  `-D warnings` clean. The same rustdoc run surfaced a **pre-existing** private-intra-doc-link
  on `CredentialHandle` (from the hot-swap-handles change, which lefthook pre-push does not
  gate) — fixed in `67c57c3f`. Deferred to **3b**: the facade test harness and the end-to-end
  tests it unblocks (binding-rejects-tombstoned, External-source), plus the structural Q10
  source-awareness. Next: **2b** (store-port sealing — wide cascade) or **4** (framework lease
  handle + staleness ceiling).
- 2026-06-13: **increment 4 (constructor-enforced lease staleness ceiling, F2) landed**
  (`da9fb14b`). `StalenessCeiling` newtype in `runtime/lease/policy.rs`: `new` rejects zero and
  any bound above `HARD_CAP` (7 days), so `Duration::MAX` is unconstructible; `Default` is 24h
  (matches the resolver's static re-validation floor). `LeaseLifecycleConfig` carries it; the
  scheduler clamps every renewal interval (track + renew-success) to it through one
  `renewal_interval` helper, so a provider reporting an unbounded TTL still re-validates by the
  ceiling. Re-exported from `runtime::` and `runtime::lease`. 273 lib tests green (4 ceiling),
  clippy `--all-features` clean, nebula-api + nebula-engine compile, rustdoc `-D warnings` clean.
  The F2 "resolver returns a lease handle, never a raw `&Secret`" item needed **no new work** —
  it is already enforced by the existing `CredentialGuard`/`SchemeGuard` compile-fail tests
  (`compile_fail_*_guard_clone` / `_retention` / `_sensitivity`). Deferred to **4b**: the W2
  herd-breaking renewal jitter — it makes `renewal_interval` non-deterministic (breaks the
  precise-timing lease tests that `tokio::time::advance` to 69s/71s boundaries) and F2 forbids
  an author-`ZERO`-able jitter knob, so it needs a determinism-injection seam, not the rotation
  subsystem's `enable_jitter` bool. Next: **4b** (jitter), **2b** (store-port seal), or **5**
  (`Scheme` markers / `Slot<S>`).
- 2026-06-13: **increment 5 type design drafted (F3), code blocked on an owner fork.** Scouting
  found `AuthScheme` is an **open** trait in `nebula-core::auth` (not credential), and the core
  already carries `AuthPattern` (cosmetic, has `Custom`) + `SensitiveScheme`/`PublicScheme`
  (sensitivity axis) — so F3's sealed-family + marker model must *reconcile* with them, not bolt
  on. The binding-axis compile-error largely already holds via generics (`SlotCell<CredentialGuard<S>>`);
  the real F3 work is the **sealed `SchemeFamily` mechanics enum** (`AuthScheme::FAMILY`, no
  `Custom`/`Box<dyn>`) + the registration-time soundness check + the `Slot<S: AuthScheme>` bound +
  the trybuild/arch-tests. Concrete design + 4 slices (5a–5d) recorded in DESIGN §17. **Code is
  gated on the owner fork — `SchemeFamily` vs the existing `AuthPattern`: (A) two enums / (B) fold
  out `Custom` / (C) family subsumes pattern** (recommend A). No core code written (spec-first;
  this reseals a foundational core trait + the plugin open-world model).
- 2026-06-13: **F3 DECIDED via adversarial planёrka (Option F) — supersedes the A/B/C fork above.**
  Owner corrected the bare-fork ask: run the analysis + convene a planёrka + design for the
  un-breakable contract (not least-ripple); "no consumer yet" is YAGNI to reject. The 7-agent panel
  (market evidence + blast-radius + future-contract war-game → 3 radical critics → chair) rejected
  A/B/C: a **sealed `SchemeFamily` enum forces a breaking framework release** on the first protocol
  the 1.0 variant set didn't ship (RFC 9421 HTTP Message Signatures is the live disqualifier).
  **Option F: mechanics axis = an OPEN `SchemeFamily` trait sealed at the *obligation*** (`const
  EGRESS: EgressShape` + `fn refresh_classes()` + `fn pattern()`), `AuthScheme::type Family`, a
  containment-law registration check; the only closed set is a small `#[non_exhaustive] EgressShape`
  (irreducible wire-primitive residue). `AuthPattern` untouched; `RefreshStrategy` relocates to
  core; `CredentialCategory` deleted (verified zero behavioural readers); `Sensitive/PublicScheme`
  kept. Full design + slices 5a–5f + overridden dissent in DESIGN §17. Panel verdict is a proposal;
  spot-verified its 3 load-bearing claims against source (stale `type Auth` doc; no
  `CredentialCategory::` behavioural reads; `RefreshStrategy` relocatable). Next: implement **5a**
  (additive core contract — reversible); the subtractive **5e** (`CredentialCategory` delete) stays
  last + owner-visible (destructive-step gating).
