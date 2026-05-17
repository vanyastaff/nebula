# Credential Subsystem Completion — `nebula-credential-runtime`

- **Date:** 2026-05-15
- **Status:** validated design (input to implementation plan)
- **Scope decision (user):** full subsystem A+B+C+D; ADRs revisable (only L1
  PRODUCT_CANON invariants fixed); `nebula-credential-builtin` = contract + 2–3
  production-grade reference impls.
- **Authoritative canon:** `RustroverProjects/docs/` PRODUCT_CANON §3.5, §12.5,
  §13.2; INTEGRATION_MODEL; ADR-0028/0030/0031/0032/0033/0034/0041/0051.

## 1. Problem

The `nebula-credential` *contract crate is internally complete* — zero
`todo!()`/`unimplemented!()`, ADR-0051 Phases A–D closed, 12 compile-fail
probes. The gaps are in its wiring, not the crate:

- **A — API is non-functional.** All 12 services in
  `crates/api/src/services/credential.rs` return `503 ServiceUnavailable`
  (CRUD, test/refresh/revoke, resolve/continue, type-discovery). `AppState`
  (`crates/api/src/state.rs:108-115`) holds only ad-hoc OAuth in-memory maps;
  no `CredentialStore`, no `CredentialTypeRegistry`, no general
  `PendingStateStore`, no encryption layer, no capability gating.
- **B — canon §12.5/§3.5 violation (observability is DoD).**
  `CredentialResolver::with_event_bus()` is never called → `CredentialEvent`
  (`Refreshed/Revoked/ReauthRequired`) never emitted. `CredentialMetrics`
  constants never incremented (only engine lease metrics wired). No
  `#[instrument]` on resolve/refresh. No resilience/backoff on the main
  refresh path; the CAS retry loop is a tight loop without jitter.
- **C — `nebula-credential-builtin` is an empty П1 scaffold.** Only the
  dormant `sealed_caps` module (`BearerSealed`/`BasicSealed`/`SigningSealed`/
  `TlsIdentitySealed`, `#[allow(dead_code)]` "becomes load-bearing in П3");
  zero concrete types. The П3 reference types it exists to ship were never
  written. (The contract crate also ships 3 types in
  `crates/credential/src/credentials/` — see §9 for why these legitimately
  stay there.)
- **D — `ExternalProvider::resolve` not wired into resolution.** Vault /
  external secrets never reach actual credential resolution (explicit
  deferred non-goal of ADR-0051 Phase D).

## 2. Layer-law facts (verified in `deny.toml`)

`deny.toml [bans].wrappers` is the enforced gate (not AGENTS prose):

- `nebula-engine` wrappers `{nebula-cli, nebula-api(dev)}` → only api/cli may
  depend on engine.
- `nebula-storage` wrappers `{nebula-engine, nebula-api,
  nebula-credential-vault(dev)}`.
- `nebula-credential-builtin` wrappers `{self}` — comment explicitly
  anticipates: "П3+ extends this allowlist with composition-root crates that
  wire concrete types".
- `engine` Cargo.toml already depends on `storage` (Exec→Exec, curated) →
  curated Exec-sibling deps are the codebase's actual pattern.

**Constraint:** the management facade needs both `nebula-storage` (Exec) and
`nebula-engine` (Exec). The only crates that may depend on both today are
`nebula-api` and `nebula-cli`. Therefore the facade **must be Exec-tier or
higher** — it cannot be a Business-tier crate. The user's "business" lands as
the `nebula-credential-builtin` concrete types (item C); the facade is Exec.

## 3. Panel deliberation & consensus

Panel: systems architect, Rust 1.95 / edition 2024 engineer, security/
credential engineer, API/observability engineer, radical deconstruction
critic. Output is a **proposal**, scope-checked against the user's words.

Three homes for the facade were evaluated:

- **A — facade module inside `nebula-engine`.** Lowest churn, ADR-0030
  untouched. Rejected as primary: a *management application service* inside
  the *workflow execution engine* conflates bounded contexts; the
  facade/mechanism boundary is only a module convention, easily eroded —
  engine already demonstrates this erosion (`Option<EventBus>=None` never
  wired; `InMemoryStore` reachable directly).
- **B — full extract** (relocate engine's resolver/lease/rotation/
  RefreshCoordinator/claim-repo into a new Exec crate). Cleanest domain
  decomposition; **rejected unanimously** for this effort — relocating
  chaos-tested ADR-0041 claim-repo against a "finalize to stable" goal is
  unacceptable risk. Recorded as the *deferred ideal*.
- **B-lite — new Exec crate owning ONLY the new surface.** Consensus.

**Deciding argument for B-lite over A (security, not aesthetics):** a crate
boundary is an *enforceable invariant boundary*. The 8 abuse-case invariants
must be forced by the constructor's type. A module inside engine sits next to
`nebula-storage{credential-in-memory}` and a public `CredentialResolver`;
"secure composition" then rests on discipline (the exact anti-pattern engine
already exhibits). A dedicated crate with a single `pub` constructor and
crate-private internals makes the secure path the only path. This maps onto
the user's standing principles (type-enforce-not-discipline; boundary-erosion;
adversarial-security) and the literal ask ("separate crates", "properly
organize structure").

**Refinements folded in (non-blocking):**
1. ADR-0052 must record full extraction (engine de-god, variant B) as the
   deferred ideal so the goal is not lost.
2. The C relocation grep-gate **fired and falsified the relocation premise**
   (2026-05-15). Importer enumeration showed `OAuth2Credential` + the
   `oauth2` submodule are production contract for the engine resolver
   (`crates/engine/src/credential/resolver.rs:13,435,439`,
   `rotation/token_refresh.rs:25,72`; ADR-0030), the api OAuth ceremony
   (`crates/api/src/handlers/credential_oauth.rs:11,378`,
   `services/oauth/flow.rs:7`; ADR-0031), and the public SDK prelude
   (`crates/sdk/src/prelude.rs:37-57`). `nebula-credential-builtin` is a
   charter-bound plugin-facing leaf upper tiers must not depend on
   (`deny.toml` wrappers `{self}`). Conclusion: **no relocation**; the
   contract crate legitimately owns ApiKey/Basic/OAuth2. §9 rewritten
   accordingly. This is the grep-gate working as designed, not validation
   theatre.

## 4. Decision: `nebula-credential-runtime` (Exec)

Single owner of the **credential management bounded context**. Sole `pub`
entry: `CredentialService`. All invariant-bearing composition is
crate-private. Acyclic: `engine` does **not** depend on runtime; runtime
depends on engine.

**Dependencies:** `nebula-credential` (contract, ↓), `nebula-credential-builtin`
(↓), `nebula-storage` (Exec-sibling, curated), `nebula-engine` (Exec-sibling,
curated — runtime calls resolver/RefreshCoordinator/LeaseLifecycle),
cross-cutting `nebula-eventbus`/`nebula-metrics`/`nebula-error`/
`nebula-resilience`.

**`deny.toml` changes (explicit, the documented mechanism):**
- add `nebula-credential-runtime` to wrappers of `nebula-engine`,
  `nebula-storage`, `nebula-credential-builtin`.
- new ban entry: `nebula-credential-runtime` wrappers
  `{nebula-api, nebula-cli, self}`.
- `crates/api/Cargo.toml`: add `nebula-credential-runtime`; the existing
  `nebula-engine` dep stays (workflow execution, unrelated to credentials).

## 5. Public surface — typestate builder

A missing mandatory collaborator is a **compile error**, not a runtime panic.

**Panel refinement (2026-05-15, user-empowered panel, supersedes the
12-phantom typestate):** the phantom-typestate builder is replaced by the
simpler idiomatic form — `CredentialServiceBuilder::new(<all mandatory
by value>)` + chained optional setters + an **infallible `build(self)`**.
Same compile-time guarantee (you cannot call `new` without every
mandatory argument), but **zero `unwrap`/`Option` by construction**
(mandatory fields are owned values, not `Option`s unwrapped at build).
Drastically simpler/maintainable ("великолепно", not baroque). Tenant
isolation is enforced at the **facade operation level**: `create` writes
`StoredCredential.metadata["owner_id"] = scope.owner_id()`; `get/list/
update/delete` load then reject `owner_id != scope.owner_id()` with
`CredentialServiceError::NotFound` (no cross-tenant existence leak). The
crate-private store stack is `AuditLayer(CacheLayer(EncryptionLayer(B)))`
composed once at `build()` (storage `ScopeLayer`'s per-call resolver
conflicts with build-once; facade-level check is equivalent, build-once
safe, and directly testable).

**Recon-driven refinement (2026-05-15, verified against engine/storage):**
`CredentialStore` and `PendingStateStore` use RPITIT (`-> impl Future`) +
generic methods → **not object-safe**; `Arc<dyn CredentialStore>` is not a
valid type and no blanket impl bridges it. `CredentialResolver<S>` and the
storage layers (`EncryptionLayer<S>` …) are generic over the store. There­
fore the facade is **generic over the raw backend and pending store**:

```
pub struct CredentialService<B: CredentialStore, PS: PendingStateStore> { /* private */ }
impl<B: CredentialStore, PS: PendingStateStore> CredentialService<B, PS> {
    pub fn builder() -> CredentialServiceBuilder</* typestate */>;
}
```

`B`/`PS` appear **only on the struct**, never in operation signatures
(operations are `&self` async methods) — this is not the over-generic
anti-pattern; it is forced by RPITIT non-object-safety. The API
composition root fixes `B = InMemoryStore`, `PS = InMemoryPendingStore`
(feature `credential-in-memory`) now; real backends substitute later
without touching operation call sites.

Builder (typestate; `.build()` callable only when all mandatory set). The
facade **constructs** the resolver + lease lifecycle internally inside
`build()` — they are NOT injected (injecting a pre-built resolver would
let a caller supply a mis-composed/unencrypted one; internal construction
*is* the security boundary):
- mandatory: `raw_store(B)`, `key_provider(Arc<dyn KeyProvider>)`,
  `audit_sink(Arc<dyn AuditSink>)`, `scope_resolver(Arc<dyn ScopeResolver>)`,
  `cache_config(CacheConfig)`, `pending_store(PS)`,
  `registry(Arc<CredentialRegistry>)`, `dispatch(Arc<CredentialDispatch>)`
  (see §5a), `observer(Arc<dyn CredentialObserver>)`,
  `lease_config(LeaseLifecycleConfig)`, `shutdown(CancellationToken)`.
- optional: `refresh_coordinator(Arc<RefreshCoordinator>)` (default
  `RefreshCoordinator::new()`), `external_providers(ExternalProviderChain)`
  → `StateSource::External`.
- `build()` composes `ScopeLayer::new(AuditLayer::new(CacheLayer::new(
  EncryptionLayer::new(raw, key_provider), cache_config), audit_sink),
  scope_resolver)`, then
  `CredentialResolver::new(Arc::new(store)).with_refresh_coordinator(rc)
  .with_event_bus(observer.event_bus())`, then
  `LeaseLifecycle::spawn(lease_config, observer.lease_bus(),
  observer.metrics(), shutdown)`. Raw backend never escapes.
- `CredentialObserver` is **deliberately designed object-safe** (no
  RPITIT, no generic methods) so it can be `Arc<dyn CredentialObserver>`;
  it exposes `event_bus() -> Arc<EventBus<CredentialEvent>>`,
  `lease_bus() -> Option<Arc<EventBus<LeaseEvent>>>`,
  `metrics() -> Option<Arc<dyn MetricsEmitter>>`, and `on_resolve/
  on_refresh/on_revoke(...)` non-async hook methods.

## 5a. Dispatch mechanism (the dyn→typed crux)

Recon verdict: there is **no runtime string-keyed dispatch table**.
`execute_resolve::<C,_>`, `resolve_with_refresh::<C: Refreshable>`,
`dispatch_test::<C: Testable>`, `dispatch_revoke::<C: Revocable>` are all
generic over a concrete `C`; the registry only does
`resolve::<C>(key) -> Option<&C>` (downcast). Engine already re-exports
`StateProjectionRegistry` / `StateProjectionError` — the existing
type-erasure-by-key precedent for `C::State`.

**Decision (no invention):** mirror/extend the `StateProjectionRegistry`
pattern as `CredentialDispatch`: a `HashMap<&'static str, DispatchEntry>`
populated **alongside** `register_builtins` via `register_dispatch::<C>()`
(and capability-bounded `register_dispatch_refreshable::<C: Refreshable>`
etc., so a closure exists *iff* the type has the capability — structural,
mirrors `plugin_capability_report`). `DispatchEntry` holds boxed
type-erased fns that capture the monomorphized generic call
(`resolve_fn`, `refresh_fn: Option`, `test_fn: Option`,
`revoke_fn: Option`, `project_fn`). Plan 2 **must read
`crates/engine/src/credential/registry.rs` (`StateProjectionRegistry`)
first** and follow its erasure shape verbatim; do not design a novel
table. Capability-gating is by closure presence (`Option::None` ⇒
`CredentialServiceError::CapabilityUnsupported`), not runtime reflection.

Operations (replace the 12 stubs). `TenantScope { org, ws }` is a mandatory
newtype argument (not `Option`):

- CRUD: `create`, `get`, `list`, `update` (CAS via `expected_version`),
  `delete`.
- Lifecycle (structurally capability-gated): `test` (Testable), `refresh`
  (Refreshable; via engine RefreshCoordinator), `revoke` (Revocable + lease
  `revoke_for_credential`).
- Acquisition: `resolve`, `continue_resolve` → `Acquisition::{Complete,
  Pending{token, interaction}}`.
- Discovery: `list_types`, `get_type`.

Errors: `CredentialServiceError` (thiserror + `nebula_error::Classify`),
mapped to HTTP by api. No stringly-typed "requires X pending".

## 6. Crate-private composition = security boundary

8 abuse cases (adversarial review run *before* design) → structural fixes:

| # | Abuse | Structural fix |
|---|-------|----------------|
| 1 | Confused deputy (cross-tenant `GET /…/{cred}`) | store key is a composite derived from mandatory `TenantScope`; no op callable without scope |
| 2 | Schema-bypass / `$expr` injection | `validate_props`: `registry.get(key).schema().validate()` + `serde_json::from_value()`; `{"$expr":…}` envelope refused (canon §12.5/Phase 9); reuse `properties_pipeline.rs` invariant |
| 3 | Secret echo in responses | API response built only from `CredentialSnapshot`; `State`/`Scheme` never serialized (`SecretWire`/ADR-0034) |
| 4 | SSRF via test/refresh | dispatch only when capability present + URL allowlist (parity with OAuth ADR-0031) |
| 5 | Cross-tenant lease replay | `revoke_for_credential` scans namespaced ids (ties to #1) |
| 6 | Pending-token hijack | general `PendingStateStore` inherits OAuth guarantees: unguessable + single-use + TTL ≤ 10 min + bound to principal |
| 7 | Plaintext-at-rest | `build()` wraps the raw backend in nesting order (outermost→innermost) `Scope(Audit(Cache(Encryption(raw))))` — `Encryption` is adjacent to the backend so persisted bytes are always ciphertext; raw never escapes the crate; compile-fail probe: raw store unusable without layers |
| 8 | Audit not fail-closed | sink refusal → `StoreError::AuditFailure` (ADR-0028 inv. 4), never log-and-continue |

**Refinement #3 (lead-reviewed, implemented `3d6c2aa5`):** the row-#3
proof is *stronger* than "response serialized redacted". `CredentialSnapshot`
is deliberately **not `Serialize`** (it holds a type-erased
`Box<dyn Any>`), so it can never reach the wire via serde at all;
serializing the *projected* scheme would in fact expose the secret
because the scheme's `#[serde(with = serde_secret)]` is the
*encrypted-at-rest* path that must preserve the value. The facade proof
is therefore: (a) `Debug` redaction to `[REDACTED]` on freshly-created
**and** re-fetched snapshots (`tests/adversarial.rs::abuse3`), and (b)
the `!Serialize` property pinned structurally by
`tests/compile_fail/snapshot_not_serialize.rs` (E0277). The API layer
(Plan 3) must build its response DTO from the secret-free
`CredentialSnapshot` accessors (`kind`/`scheme_pattern`/`record`), never
by serializing the snapshot or scheme.

## 7. B — observability closed structurally

Non-`Option` `CredentialObserver` injected at the constructor. Default impl:
emits `CredentialEvent`/`LeaseEvent` to `nebula-eventbus`, increments the
existing `CredentialMetrics`, opens `#[instrument]` spans on resolve/refresh/
revoke. `NoopObserver` is chosen explicitly (tests). Because emission sits on
the single code path through the facade, "never wired" (`None`) is
unrepresentable. Refresh path wrapped in `nebula_resilience::retry_with`
(exponential backoff + jitter) over the existing `RefreshCoordinator`
claim-repo; the tight CAS retry loop is replaced by the resilience policy.

## 8. D — `StateSource` (replace, not bridge)

No adapter/bridge (project rule: replace the wrong thing directly). The
resolver's hardcoded "state always from `CredentialStore`" is **replaced** by
a polymorphic source:

```
enum StateSource { LocalEncrypted, External(Arc<dyn ExternalProvider>) }
```

`External` → `provider.resolve()` → if `ProviderResolution.lease` present →
`lease_lifecycle.track(...)`. ADR-0051 Phase-D non-goal is *fulfilled* here,
not worked around; ADR-0051 itself is untouched.

## 9. C — `nebula-credential-builtin` populated with net-new П3 types

**No relocation** (grep-gate verdict, §3 refinement 2). `ApiKeyCredential`,
`BasicAuthCredential`, `OAuth2Credential` **stay in `nebula-credential`**:
they are the SDK prelude's public credential surface
(`crates/sdk/src/prelude.rs:37-57`) and `OAuth2Credential`/`oauth2::*` is
production contract for the engine resolver + token refresh (ADR-0030) and
the api OAuth ceremony (ADR-0031). Moving them would force engine/api/sdk to
depend on a charter-bound plugin-facing leaf — architecturally wrong, not a
junior wrapper widen.

The radical critic's "incoherent split" smell was a misread: there is no
split to fix by relocation. The real gap is that the **П3 reference types
`nebula-credential-builtin` exists to ship were never written** — its
`sealed_caps` module has sat dormant since П1.

Fix: write **3 net-new production-grade static reference credentials** in
`nebula-credential-builtin`, each mirroring the proven `BasicAuthCredential`
shape (`crates/credential/src/credentials/basic_auth.rs` — `impl Credential`
+ `#[derive(Schema, Deserialize, Default)]` Properties + five
`plugin_capability_report::Is*` = `false`). Zero upper-tier dependencies
(charter intact, `deny.toml` wrappers stay `{self}`), zero importer churn.
Delivers the user's "contract + 2–3 эталона" (2–3 reference impls in
builtin).

**`sealed_caps` / `#[capability]` activation is explicitly NOT in scope
here.** The phantom-shim П3 work needs scheme-acceptance traits
(`Accepts*`) and `dyn`-capability consumers that do not exist yet — that is
a separate research-bearing concern, not credential-subsystem completion.
`nebula-credential-builtin/src/lib.rs:30`'s `#[allow(dead_code)]`
"`becomes load-bearing in П3`" stays accurate and dormant; this effort does
not claim to reach phantom-shim П3.

**Concrete reference set** (distinct verified-static schemes; final field
names pinned in the plan):
- `BearerTokenCredential` — `Scheme = State = SecretToken`
  (`identity_state!` confirmed `scheme/secret_token.rs:45`), KEY
  `"bearer_token"`, `AuthPattern::SecretToken`.
- `SharedKeyCredential` — `Scheme = State = SharedKey`, KEY
  `"shared_key"`, `AuthPattern::SharedSecret`. Plan adds
  `identity_state!(SharedKey, "shared_key", 1);` to
  `scheme/shared_key.rs` (additive one-liner mirroring
  `secret_token.rs:45`; grep-verified absent first).
- `SigningKeyCredential` — `Scheme = State = SigningKey`, KEY
  `"signing_key"`, `AuthPattern::RequestSigning`. Plan adds
  `identity_state!(SigningKey, "signing_key", 1);` to
  `scheme/signing_key.rs` (same additive pattern, grep-verified first).

`nebula_credential_builtin::register_builtins(&mut CredentialRegistry)`
registers these 3 via `CredentialRegistry::register(_, env!("CARGO_CRATE_NAME"))`;
`nebula-credential-runtime` calls it (alongside the contract crate's own
ApiKey/Basic/OAuth2 and plugin-discovered types) when constructing the
`Arc<CredentialRegistry>` passed to the service builder.

## 10. A — API wiring

- `crates/api/src/services/credential.rs`: delete all 12 stub bodies; each
  becomes a thin call into `state.credential: Arc<CredentialService>`.
- `crates/api/src/state.rs` + composition root (`bin/nebula-server.rs`):
  build the service via the typestate builder with the real layered store +
  `KeyProvider` + populated registry + eventbus + metrics observer.
- Fold `oauth_pending_store` / `oauth_credential_store` /
  `oauth_state_tokens` into the service's pending/store. OAuth HTTP ceremony
  remains in api per ADR-0031 but drives the service's `resolve`/
  `continue_resolve` (closes abuse #6 — no weaker parallel in-memory map).
- Response projection from `CredentialSnapshot` only (abuse #3).
- OpenAPI (ADR-0047): handlers cease to be 503; utoipa annotations updated;
  stub-endpoint policy no longer applies (they are implemented).

## 11. ADR-0052

`docs/adr/0052-credential-runtime-crate.md` — narrowly supersedes the
facade-ownership slice of ADR-0030: "engine retains the low-level resolver /
RefreshCoordinator / lease *mechanism*; `nebula-credential-runtime` owns the
*management facade + type registry + StateSource + observability*; api/cli
depend on runtime for credential management." ADR-0041 untouched. ADR-0051
untouched (its Phase-D non-goal is fulfilled, not superseded). Records
variant B (full engine de-god) as the deferred ideal. Includes the ADR-0028
8-invariant canon-audit checklist.

## 12. Verification bar (non-negotiable)

- `task dev:check` green (fmt + clippy `-D warnings` + nextest + doctests +
  deny).
- Integration tests: tenant-scoped API CRUD round-trip
  (create→get→list→update-CAS→delete); lifecycle capability-gated;
  interactive resolve/continue; `StateSource::External` via wiremock-Vault;
  observability assertions (`tracing-test` for spans, metrics counters,
  emitted events).
- One adversarial test per abuse case (incl. compile-fail probe: raw store
  without layers does not compile; cross-tenant get denied; `$expr`
  rejected; secret never in response body; SSRF allowlist; audit
  fail-closed).
- ADR-0028 8-invariant canon-audit checklist completed in ADR-0052.

## 13. Constraints

- No plan/task IDs or "Phase A/B/C/D" labels in committed *code* or code
  comments — comments must read correctly after the plan is deleted (spec
  and ADR may use phase labels; code may not).
- No adapters/bridges/shims; replace the wrong thing directly.
- Conventional Commits validated by `convco`; scope = crate name without
  `nebula-` prefix or top-level area; squash-merge to `main`, no
  force-push.
- `Cargo.lock` discipline: on any dep add, stage the root `Cargo.lock`;
  rebase conflicts resolved with `--theirs`, never `cargo update -p`.
- No `unwrap()`/`expect()`/`panic!()` in library code; typed `thiserror`
  errors only.
- `#![forbid(unsafe_code)]` in the new crate.

## 14. Non-goals

- Cross-replica lease coordination (remains ADR-0041's deferred separate-ADR
  item; runtime is single-replica, gates multi-replica through the existing
  L2 claim repo).
- Full engine de-god / variant B relocation (deferred ideal, recorded in
  ADR-0052).
- Broad first-party provider catalogue (GitHub/Slack/AWS SigV4/…): out of
  scope per the user's "contract + 2–3 reference impls" decision.
- Plane A (platform auth to Nebula): remains separate `nebula-auth` future
  work per ADR-0033.

## 15. Phased decomposition outline

Detailed task graph is produced by `writing-plans`. High-level phases:

1. **Crate scaffold + layer wiring:** create `nebula-credential-runtime`,
   `deny.toml` wrappers, ADR-0052 draft.
2. **C — 3 net-new static reference credentials** in
   `nebula-credential-builtin` (mirror `BasicAuthCredential`) +
   `register_builtins`; no relocation, no importer churn, no
   `#[capability]`/phantom-shim (grep-gate verdict §3.2, §9).
3. **Facade core:** typestate builder, crate-private layered-store
   composition, `CredentialServiceError`, validation pipeline.
4. **B — `CredentialObserver`** (events + metrics + spans + resilience
   refresh).
5. **D — `StateSource`** integration with `LeaseLifecycle`.
6. **A — API wiring:** services, `AppState`, composition root, OpenAPI.
7. **Verification:** integration + adversarial tests, canon audit, full
   `task dev:check`.
