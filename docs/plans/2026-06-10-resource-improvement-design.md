# nebula-resource — Improvement Design (final)

Repo root: `C:/Users/vanya/RustroverProjects/nebula/.claude/worktrees/dreamy-kare-8698d4`. All paths below are relative to that root. Inputs: synthesis report (`docs/plans/2026-06-10-resource-improvement-synthesis.md`), three critiques, and direct source verification of `crates/resource/src/{resource.rs,error.rs,ext.rs,registry.rs,manager/options.rs,manager/shutdown.rs,lib.rs}`, `crates/resource/macros/src/{resource.rs,lib.rs}`, `crates/engine/src/resource_accessor.rs`, `crates/sdk/src/lib.rs`.

---

## 1. Target API (end-state)

### 1.1 Authoring — one trait, two assoc types, two derives, no panicking macro

```rust
// ---- config: schema + fingerprint derived, B1 dead ----
#[derive(ResourceConfig, serde::Deserialize, Clone)]
struct PgConfig {
    url: String,
    max_conns: u32,
}
// #[derive(ResourceConfig)] emits:
//   impl ResourceConfig: fingerprint() = deterministic structural hash folded over
//     every field (fields must impl Hash; #[config(skip_fingerprint)] per-field opt-out),
//     validate() hook via optional #[config(validate = path)].
//   impl HasSchema: empty schema by default; #[config(schema = external)] suppresses
//     emission so a real #[derive(Schema)] can coexist.
// ResourceConfig::fingerprint() loses its `default 0` body and becomes REQUIRED —
// a hand impl must consciously write it; the silent equal-zero hot-reload no-op
// (B1) is unrepresentable. The doc inversion is fixed at the same time.

// ---- slots: epoch fold + accessors derived, B5 dead ----
pub type CredentialSlot<C> = SlotCell<CredentialGuard<C>>;   // kills the noisiest field type

#[derive(ResourceSlots, Clone)]            // legal on slot-less structs (fold = 0, explicit)
struct Postgres {
    #[credential("db")]
    iam: CredentialSlot<IamToken>,
}
// #[derive(ResourceSlots)] emits: DeclaresDependencies, fn iam_slot(&self) -> Option<Arc<…>>,
// and `impl HasCredentialSlots { fn credential_slot_epoch(&self) -> u64 { /* positional fold */ } }`.
// It NEVER emits a Resource method (no todo!(), no coherence trap). Invalid key literal /
// wrong attr shape = expansion-time compile_error! with the span on the offending token (B22).

// ---- the trait: Lease and Error assoc types are GONE ----
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;

    fn key() -> ResourceKey;

    fn create(&self, config: &Self::Config, ctx: &ResourceContext)
        -> impl Future<Output = Result<Self::Runtime, Error>> + Send;        // crate::Error direct

    // defaulted lifecycle, all -> Result<_, Error>:
    // check, shutdown, destroy, on_credential_refresh, on_credential_revoke
    // credential_slot_epoch is NOT here any more — it lives on HasCredentialSlots (derived).
    fn metadata() -> ResourceMetadata where Self: Sized { /* default from key + Config schema */ }
}

// ---- capability traits stay opt-in (topology hooks live where they are real) ----
pub trait Resident: Resource { fn is_alive_sync(&self, _: &Self::Runtime) -> bool { true } }
pub trait Pooled: Resource where Self::Runtime: Clone {
    fn is_broken(&self, _: &Self::Runtime) -> BrokenCheck { BrokenCheck::Healthy }
    fn recycle(&self, _: &Self::Runtime, _: &InstanceMetrics)
        -> impl Future<Output = Result<RecycleDecision, Error>> + Send { async { Ok(RecycleDecision::Keep) } }
    fn prepare(&self, _: &Self::Runtime, _: &ResourceContext)
        -> impl Future<Output = Result<(), Error>> + Send { async { Ok(()) } }
}
// The Bounded topology (Capped<N> / Exclusive / Unbounded, CapMarker, BoundedRelease,
// BoundedRuntime) is DELETED — zero consumers (B9), config-unrepresentable cap (B18),
// pre-built-runtime contradiction. Sessions return later as an opt-in `Sessioned`
// capability trait if and only if a real session resource appears.

// ---- author error enums stay optional and typed; `?` works ----
#[derive(Debug, thiserror::Error, ClassifyError)]
enum PgError {
    #[error("connect")]
    #[classify(transient)]
    Connect(#[from] std::io::Error),                  // source chain PRESERVED (.with_source)
    #[error("rate limited")]
    #[classify(exhausted, retry_after = .0)]          // field-valued, runtime Retry-After OK
    RateLimited(Duration),
}
// ClassifyError emits `From<PgError> for nebula_resource::Error` classifying by reference,
// then attaching the whole value as source — no more Display-flattening (B22/DX#9).
```

A minimal resident resource (the reqwest case) is now: 1-line config derive + 3-line config struct + 2-line resource struct + ~8-line `impl Resource` + 1-line registration ≈ **15 lines** (was ~52; ceremony target <20 met).

### 1.2 Registration — constructors are the proof, B4 unrepresentable

```rust
impl Manager {
    /// The only registration entry point.
    pub fn register<R: Resource>(&self, reg: Registration<R>) -> Result<RowHandle, Error>;
}

/// All fields private. RegistrationSpec (7 public fields), RegisterOptions, and the
/// exported ErasedAcquireFn alias are deleted. The erased acquire hook, the TopologyRuntime
/// variant, and the config fingerprint are derived INSIDE the constructor — the
/// topology/acquire-fn mismatch class and the hand-passed fingerprint cannot be written.
pub struct Registration<R: Resource> { /* private */ }

impl<R: Resource + HasCredentialSlots> Registration<R> {
    pub fn resident(resource: R, config: R::Config) -> Self where R: Resident;
    pub fn pooled(resource: R, config: R::Config, pool: PoolConfig) -> Self where R: Pooled;
    // PoolConfig.max_size etc. are runtime values — JSON/operator-configurable caps for free.

    #[must_use] pub fn scope(self, s: ScopeLevel) -> Self;                       // default Global
    #[must_use] pub fn slot_bindings(self, b: &[(&str, &str)]) -> Self;          // sets SlotIdentity structurally
    #[must_use] pub fn recovery(self, p: RecoveryGateConfig) -> Self;            // policy value, not Option<Arc<Gate>>
}
// The `R: HasCredentialSlots` bound makes "forgot #[derive(ResourceSlots)]" a COMPILE error —
// the silent epoch-0 staleness hole (B5's residue) is type-closed, not documented.

/// Returned from register; verbs grow additively (no second signature break later).
pub struct RowHandle { /* key, scope, identity pinned */ }
impl RowHandle {
    pub fn status(&self) -> RowStatus;                      // includes `tainted` once B11 lands
    // follow-ups add: deregister() (per-row drain+destroy, B6), reload() (B2), reset_gate() (B10)
}
```

`Registry`, `AnyManagedResource`, `ManagedResource`, `TopologyRuntime`, `PoolRuntime`, `ResidentRuntime`, `ReleaseQueue`, `DedupKey` all go `pub(crate)`; `Registry::register` derives `type_id` from the managed value so the `type_index` desync (B14) is unconstructible.

### 1.3 Acquire + the erased seam — classification survives end-to-end (B3 dead)

```rust
// one typed front door (replaces the per-topology acquire family for consumers)
let guard: ResourceGuard<Postgres> = manager.acquire::<Postgres>(&ctx, AcquireOptions::default()).await?;
guard.query("select 1").await?;                     // Deref to R::Runtime; Drop → ReleaseQueue
let ctx = ResourceContext::test();                  // feature = "test-util": default scope + fresh cancel token

// engine seam keeps its shape (no engine rewiring) but stops lying:
//   Manager::acquire_erased_for(...) -> Result<Box<dyn Any + Send + Sync>, resource::Error>   (unchanged)
//   resource::Error::to_core_error():
//       NotFound  -> CoreError::ResourceNotFound { key }      (new core variant; no more CredentialNotFound mislabel, L2)
//       all kinds -> retryable + retry_after preserved        (already true — verified error.rs:254-279)
//   NEW: impl From<CoreError> for resource::Error reconstructs ErrorKind from CoreError's
//        Classify metadata (category + retryable + retry_hint) — ext.rs / resource_ref.rs use it
//        through ONE shared downcast helper instead of two copy-pasted Permanent-flattening bodies.
//   crates/action context maps CoreError classification -> ActionError::retryable(retry_after) / fatal,
//        so Transient / Exhausted{retry_after} / Backpressure finally reach workflow retry policies.
```

`ErrorKind` (8 variants) and `Error` (kind + scope + key + source) survive unchanged — the taxonomy was never the problem; the seam was.

### 1.4 Adopt / reject ledger

**Radical architect:** ADOPT — delete `type Lease` (Lease ≅ Runtime on both live topologies, kills B15's round-trip); delete `type Error` (typed enums survive via `ClassifyError` + `?`, minus one assoc type and one `From` ceremony); delete Bounded wholesale; private-field `Registration` with hook derived inside; `RowHandle` row lifecycle; structured erased seam with classification preserved; pub(crate) amputation; one-home-per-invariant docs; keep the pool fork conditional on loom (PR3). REJECT — topology-as-value enum (Alt A): capability traits + typed constructors kill B4 equally while keeping compile-time pairing proof and the existing pool/resident runtimes — less migration, same guarantee. REJECT — pool hooks merged into the base trait: pollutes the minimal-resource floor that the DX case is built on. REJECT — tower::Service shape: guard Drop is a return channel, tower has no slot for it (the architect concedes this). REJECT — `Sessioned` now: no consumer; add on first real session resource. DEFER — pool extraction to a credential-agnostic module/crate: decided inside PR3 with loom as the gate.

**DX author:** ADOPT — #1 derive split (as `ResourceSlots`, composing with hand-written `impl Resource`); #2 typed registration constructors; #3 `ResourceConfig` derive with structural fingerprint; #4's error-threading half; #6 assoc-type collapse; #8 single acquire front door + `ResourceContext::test()`; #9 ClassifyError source-chain + runtime retry_after; #10 per-row verbs (PR2). REJECT — the `#[derive(Resource)]` + `#[create]` method-attribute style: fights trait coherence, hides the trait from IDEs/docs, magic over explicitness; the hand-written impl is now small enough not to need it. REJECT — #5 runtime-valued Bounded caps and #7 framework-built Bounded runtimes: moot, Bounded is deleted. REJECT — first-class rate-limiting (RPM token bucket): QoS composition belongs to `nebula-resilience`, not the lifecycle crate. DEFER — #4's `Res<Postgres>` typed action parameter with activation-time validation: engine/action-side (D4 follow-up). DEFER — `Manager::test().with_fake::<R>()`: post-redesign a fake is `Registration::resident(Fake, cfg)` — one line; revisit only if that proves insufficient.

**Security critic:** ADOPT now — custom `Debug` for `SlotIdentity`/`DedupKey` redacting credential ids (L1); `ResourceNotFound` mislabel fix (L2); `Registry` demotion + derived type_id (B14); `release()` must never `spawn` into a missing runtime (B13). ADOPT as follow-ups — tenant-scoped `(scope, ResourceKey)` identity resolution + credential-ownership verification before SlotIdentity derivation (PR6, the gating M12.4 requirement); #714 generation-fenced revoke + atomic register-then-publish replacing the quiesce prose (PR6); per-tenant RecoveryGate + reset surface + cancel-vs-failure (PR4); bounded rescue population + honest drop accounting (PR4); loom on drain/epoch primitives (PR3).

Constraint compliance: RPITIT throughout (no async_trait), `#![forbid(unsafe_code)]` untouched, thiserror-typed errors at every boundary, no shims (old surfaces are deleted, not bridged), all enforcement structural (bounds/constructors/private fields, not docs), resource stays Business-layer (talks to engine only via the existing accessor seam and `nebula-eventbus`), observability addressed as DoD (spans + typed errors on every new path now; events/metrics overhaul is PR4 and is on the critical path, not optional).

---

## 2. This-PR slice (one branch, implementable today)

Scope: nebula-resource + its macros, with mechanical fallout in `engine` (tests/registrar constructor swap only — `register_resolved` and `acquire_erased_for` signatures stay stable), `action` (one error-mapping site), `core` (one additive error variant), `sdk` (re-export curation), `examples` (two files shrink). Excluded per instruction: B2, B6, B7/B8/B15-pool-surgery, D2.

Order is dependency order; each item must end crate-green (per-crate commit points per workspace convention).

**0. Inventory sweep (prereq, no code).** `rg` the workspace for `Bounded|Capped|CapMarker|BoundedRelease|BoundedRuntime|RegistrationSpec|RegisterOptions|ErasedAcquireFn|GuardInner::Shared|type Lease|credential_slot_epoch` to fix the exact blast list before editing. Acceptance: list pasted into the PR description.

**1. Delete the Bounded topology (B9, B18, E3-partial).**
Delete `crates/resource/src/topology/bounded.rs`, `crates/resource/src/runtime/bounded.rs`; remove `TopologyRuntime::Bounded` arm (`crates/resource/src/runtime/mod.rs`), `acquire_bounded`/`erased_acquire_bounded_for` (`crates/resource/src/manager/acquire.rs`, `manager/mod.rs`), lib.rs:117-129 exports, `topology = "bounded"` in `crates/resource/macros/src/resource_attrs.rs`, bounded trybuild fixtures, `docs/topology-reference.md` sections.
Acceptance: `cargo check -p nebula-resource -p nebula-resource-macros` green; workspace rg shows zero references.

**2. Trait re-foundation (B5-root, B15-contract, B3-prep).**
`crates/resource/src/resource.rs`: drop `type Lease`, drop `type Error`; `create/check/shutdown/destroy/on_credential_refresh/on_credential_revoke` return `Result<_, crate::Error>`; remove `credential_slot_epoch` from `Resource`; add `pub trait HasCredentialSlots { fn credential_slot_epoch(&self) -> u64; }` (new `crates/resource/src/slots.rs` or in `resource.rs`); add `pub type CredentialSlot<C>`. `crates/resource/src/topology/pooled.rs`: delete the bidirectional `Into` bounds, require `Runtime: Clone`; `topology/resident.rs` likewise. Mechanical sweep through `runtime/pool.rs` (drop `.into()` conversions at lease mint/release — the pool entry is now the canonical instance), `runtime/resident.rs`, `manager/*` (drop `.map_err(Into::into)` shims), `guard.rs` (delete `GuardInner::Shared` + `ResourceGuard::shared` and its 9 match arms while in the file — B9).
Acceptance: `cargo nextest run -p nebula-resource` green; existing pooled/resident lifecycle + rotation tests pass unmodified in intent (mechanical edits only).

**3. Derive split + macro hygiene (B5, B22).**
`crates/resource/macros/src/resource.rs` → `slots.rs`: `#[derive(ResourceSlots)]` emitting `DeclaresDependencies` + slot accessors + `HasCredentialSlots` fold; never any `Resource` method; expansion-time `compile_error!` for invalid key literal and rejected attr shapes, diagnostics spanned to the offending token; delete the `topology` attr (informational lie). Fix `macros/src/lib.rs` rustdoc (examples must match the parser; drop the Option/Lazy table); fix `purpose` attr plumbing or delete it; `ClassifyError`: classify by reference, attach the moved value via `.with_source(err)`, accept `retry_after = .N` / named field. Fix `crates/sdk/macros-support/src/attrs.rs:61-66` wrong-type-vs-missing misdiagnosis.
Acceptance: trybuild suite green on warm cache (`cargo test -p nebula-resource` plain, per `reference_trybuild_agent_timeout`); new compile-fail fixtures: derive-on-enum, bad key literal, slot field of wrong type; doctest showing the two-derive + hand-impl pattern.

**4. `#[derive(ResourceConfig)]` + required fingerprint (B1).**
New `crates/resource/macros/src/config.rs`: fingerprint = deterministic hash fold over fields (fields: `Hash`; `#[config(skip_fingerprint)]` opt-out; `#[config(schema = external)]` suppresses the default empty-`HasSchema` emission). `crates/resource/src/resource.rs:64-70`: delete the `fingerprint() -> 0` default — method becomes required; fix the inverted doc; provide in-crate impls for `()`/primitive stub configs. Fix the propagated misreading in `examples/examples/resource_resident_http.rs:202-206` and the hand-passed fingerprint in `examples/examples/resource_postgres_pool.rs:370-371` (fingerprint now computed inside registration — item 5).
Acceptance: unit tests — two configs differing in one field ⇒ different fingerprints, identical ⇒ equal; hot-reload `NoChange` test asserts the corrected semantics; `cargo doc -p nebula-resource` clean.

**5. Typed registration + RowHandle + single front door (B4, B14, B9, E2-partial).**
`crates/resource/src/manager/options.rs`: delete `RegistrationSpec` and `RegisterOptions`. New `Registration<R>` (private fields, `resident`/`pooled` constructors deriving hook + `TopologyRuntime` + fingerprint internally, `scope`/`slot_bindings`/`recovery` builders — `recovery` takes `RecoveryGateConfig`, constructing the gate inside). `Manager::register` returns `RowHandle { key, scope, identity }` with `status()`. `manager/acquire.rs`: add `Manager::acquire::<R>(&ctx, opts)` dispatching on the row's topology; demote the per-topology methods and `ErasedAcquireFn` to `pub(crate)`. `registry.rs`: `Registry`/`AnyManagedResource` → `pub(crate)`; `register` derives `type_id` from `managed.managed_type_id()`. `context.rs`: `ResourceContext::test()` under `test-util`. Keep `register_resolved` and `acquire_erased_for` signatures byte-stable (internal re-route only). Mechanical: `crates/engine/src/resource/registrar.rs` + `crates/engine/src/resource_accessor.rs` tests swap to constructors.
Acceptance: compile-fail fixture `Registration::pooled` on a non-`Pooled` type and on a type missing `ResourceSlots`; round-trip test register→acquire via the front door per topology; `unexpected_topology` error variant deleted; `cargo nextest run -p nebula-engine` green.

**6. Error seam (B3, L2).**
`crates/core` error module: add `CoreError::ResourceNotFound { key }` (additive, `#[non_exhaustive]`). `crates/resource/src/error.rs`: `to_core_error` NotFound → the new variant; add `impl From<CoreError> for Error` reconstructing `ErrorKind` from `Classify` metadata. New `crates/resource/src/downcast.rs` helper used by both `ext.rs` and `resource_ref.rs` (deletes the two Permanent-flattening copies; type-mismatch becomes a typed error carrying expected/actual type names). `crates/action/src/context.rs:755-766`: map by classification — retryable+retry_after → `ActionError::retryable`, else fatal.
Acceptance: exhaustive round-trip test — all 8 `ErrorKind`s through `to_core_error` → `From<CoreError>` preserve retryability and `retry_after`; action-side test: a `Transient` acquire failure surfaces as a retryable `ActionError` with hint.

**7. Robustness/security no-brainers (B13, L1, #589, B26 subset).**
`crates/resource/src/dedup.rs`: hand-written `Debug` for `SlotIdentity`/`DedupKey` printing arity + redaction tag, never credential ids; assert via test. `guard.rs:531-570`: `tokio::runtime::Handle::try_current()` — fall back to the ReleaseQueue submit path instead of panicking outside a runtime. `manager/shutdown.rs:272` (#589): replace the `tokio::Mutex` around `release_queue_handle` with `std::sync::Mutex` (take under lock, await outside). B26 subset: `impl From<Infallible> for Error`; dedup the `retry_at` predicate (`manager/gate.rs:88` / `recovery/gate.rs:427`); NotFound Display key duplication; `ReleaseFuture` private alias out of pub signatures; drop `ResourceRef<R: ?Sized>` relaxation; delete `WarmupStrategy::Parallel` (silent lie, zero consumers); log the discarded `BrokenCheck::Broken` reason at `pool.rs:631, 1220` (tracing only — not pool surgery).
Acceptance: targeted unit tests for Debug redaction and no-runtime release; clippy clean.

**8. Surface amputation + doc truth (E2, B9, B23, B25).**
`crates/resource/src/lib.rs`: re-export only {Resource, Resident, Pooled, ResourceConfig, HasCredentialSlots, CredentialSlot/SlotCell, derives, Registration, RowHandle, Manager, ManagerConfig, AcquireOptions, ResourceContext, ResourceGuard, Error/ErrorKind, ResourceEvent, SlotIdentity, ScopeLevel, configs, rotation pair types, shutdown types}; everything else `pub(crate)` (`RecoveryWaiter`, `DedupKey`, `Registry`, `ReleaseQueue`, `ManagedResource`, `TopologyRuntime`, runtimes, `LookupOutcome`). `crates/sdk/src/lib.rs:51`: replace wholesale `pub use nebula_resource` with a curated `pub mod resource { pub use … }`. Docs: fix README flagship example (compile-checked as a lib.rs doctest), fan-out "follow-up" staleness in `crates/resource/README.md` and `crates/credential-runtime/README.md`, phantom retry/`execute`-module claims in `manager/mod.rs:26` + `acquire.rs:357,415,583,697`, stale deleted-method names (`mod.rs:493-495`), `ScopeFind` link, "Fourteen"→actual count, plan-id scrub (`slot.rs:218`, `dedup.rs:135`, `registry.rs:1298,1503`, `acquire.rs:81,413,501`); update `crates/resource/CLAUDE.md` (it still documents 4 assoc types and Bounded).
Acceptance: `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-resource --no-deps`; rg sweep zero plan-ids; README snippet is a passing doctest; `cargo nextest run -p nebula-resource -p nebula-engine -p nebula-action`, examples build.

**Net LoC estimate:** ≈ −3.2k deleted (Bounded ~2.3k incl. tests, Spec/Options/Shared/dead exports ~0.5k, doc trims ~0.4k) + ≈ +1.5k added (two derives, Registration/RowHandle, error seam, tests) ⇒ **net ≈ −1.7k**, with the authoring floor dropping ~52→~15 lines.

---

## 3. Follow-up PR sequence

1. **Per-row lifecycle:** `RowHandle::deregister` (taint→drain→destroy reusing revoke machinery), same-triple re-register → `AlreadyRegistered` unless `.replacing()`, one scope-resolution policy across the lookup matrix, taint in `RowStatus`/health — closes B6, B11, B14-residual, half of D5.
2. **Pool hardening, loom or fall back:** loom probes for the `(AtomicU64, Notify)` drain trackers + epoch fence + taint TOCTOU (workspace `storage-loom-probe` precedent), one absolute deadline end-to-end with phase-tagged timeouts, idle-lock snapshot for rotation hooks, prepare-doc truth, lifetime jitter — closes B7, B8, B16, B15-residual, converts E1; deadpool-wrap is the explicit fallback if loom fails the fork.
3. **Observability as data:** event payloads as enums carrying scope/slot-identity/execution-id, record the declared `nebula_metrics::naming` metrics with per-key labels, RecoveryGate half-open + cancel-vs-failure + Manager/`RowHandle::reset_gate`, bounded rescue population + honest drop accounting — closes B20, B21, B10, B12, #716, E6.
4. **Honest hot reload:** drain-then-rebuild per topology with truthful `ReloadOutcome` variants on `RowHandle::reload`, `register_resolved` stops taking `&ExpressionEngine` (engine pre-resolves) — closes B2, B19, #712.
5. **Tenant-true engine wiring (M12.4 enabler):** `(tenant-scope, ResourceKey)`-keyed slot-identity resolution replacing the global map at `crates/engine/src/engine.rs:214`, credential-ownership verification before `SlotIdentity` derivation, generation-fenced revoke-vs-recreate (#714), atomic register-then-publish replacing the quiesce prose — closes the security critic's req 1+2, makes D5's claim true.
6. **D4/D2 differentiators:** typed action-parameter injection validated at activation (`Res<Postgres>`), then suspension hooks (`on_suspend`/`on_resume`) over the existing release/acquire pipeline — closes the remaining injection gap and D2.

---

## 4. Rejected ideas

- **Topology-as-value enum replacing capability traits (radical Alt A core):** typed constructors already make B4 unrepresentable while keeping the compile-time pairing proof and the working runtimes — same kill, less migration, no hooks-on-base-trait pollution.
- **Wrap deadpool instead of keeping the pool:** the revoke-epoch fence threads through every resurrect path (idle/checkout/release/warmup/maintenance); a wrapper reopens TOCTOU windows and deadpool has no maintenance loop to hook — the fence is the D1 moat. Loom in PR3 is the survival condition.
- **`tower::Service` as the resource trait:** guard Drop is a pool return channel; tower's request→response shape has no slot for it.
- **`#[derive(Resource)]` with `#[create]` method attributes (DX ideal):** fights coherence, hides the trait from rustdoc/IDE navigation; the redesigned hand-impl is ~8 lines.
- **Keeping Bounded with runtime-valued caps (DX#5/#7):** rehabilitating a zero-consumer topology is sunk-cost; delete and re-add as `Sessioned` on first real demand.
- **`Sessioned` trait now / rate-limit (RPM) primitive now:** no consumer; QoS composition is `nebula-resilience`'s job.
- **Renaming `type Runtime` → `Handle`:** cosmetic churn across every impl for a LOW-severity naming gripe (B24).
- **`Manager::test().with_fake::<R>()` dedicated fake API:** post-redesign a fake is one `Registration::resident` line; add only if real test friction survives.
- **Pool extraction into a separate crate in this pass:** boundary decision belongs with the loom/timeout work (PR3), not the API PR.
- **Touching engine identity-map / bind-population now:** highest-stakes security change (PR5 here = security req 1); must not ride an API-redesign PR.
- **Hash-digest SlotIdentity or any relaxation of structural equality:** the digest-free exact row key is the one verified-correct security property — untouchable.
- **Re-adding per-crate examples or a `plans/` dir:** repo rules (root `examples/` member; no plan-ids in code).

---

## 5. Risks of this plan

- **Compile blast radius (engine/api/sdk/action/examples).** Every `impl Resource` in workspace tests/fixtures breaks on the assoc-type removal; `crates/engine/src/resource/registrar.rs` and `resource_accessor.rs` tests construct `RegistrationSpec` directly. Mitigation: item-0 rg inventory first; `register_resolved` + `acquire_erased_for` signatures frozen so engine *wiring* is untouched; per-crate-green commit points (lefthook is per-staged-crate); execute in one pass per the bold-refactor convention rather than gating every edit.
- **`CoreError::ResourceNotFound` ripple.** api/engine sites matching `CredentialNotFound` for resource paths (e.g. the engine accessor test at `crates/engine/src/resource_accessor.rs:253`) change meaning; api HTTP mapping may need one new match arm. Mitigation: variant is additive on a `#[non_exhaustive]` enum; rg `CredentialNotFound` and re-point resource-origin assertions deliberately.
- **Behavioral change in action retry semantics (B3 fix).** Previously-fatal acquire failures become retryable; a workflow with an aggressive retry policy may now retry into a down backend. This is the *intended* contract, but tests encoding the old fatal behavior will fail and retry-storm risk shifts to policy defaults. Mitigation: call it out as the PR's one behavior change; verify Layer-2 retry caps bound the storm; update action tests to assert classification, not fatality.
- **Derive rewrite churn under trybuild.** Compile-fail fixtures false-TIMEOUT on cold nextest cache (known: `reference_trybuild_agent_timeout`); never `TRYBUILD=overwrite` a timeout. Mitigation: warm cache + plain `cargo test` for the macros crate.
- **Fingerprint-required breaks stub configs.** Every `impl ResourceConfig` in tests must now write `fingerprint` or switch to the derive; risk of mechanical `0` copy-paste recreating B1 locally. Mitigation: migrate stubs to `#[derive(ResourceConfig)]`; lint the diff for literal `fn fingerprint(&self) -> u64 { 0 }` outside the `()` impl.
- **SDK re-export curation under-shoots.** Curating `crates/sdk/src/lib.rs:51` may drop a path `api`/`server` or doctests import. Mitigation: rg `nebula_sdk::nebula_resource` / `sdk::resource` before finalizing; workspace `cargo check` is the gate.
- **Pool mechanical edits drift into surgery.** Item 2's `Into`-removal touches `runtime/pool.rs` lease-mint/release sites adjacent to B15's hazard; temptation to "fix the pool while here" blows the PR. Mitigation: hard rule — type-level edits only in pool.rs this PR; correctness work is PR3 (loom-gated).
- **Windows worktree toolchain quirks.** `cargo fmt --all` fails with os error 206 in deep worktree paths; verify fmt per-crate; backgrounded pushes look hung under lefthook (~100s). Known, documented in workspace references.
- **Size risk.** Eight ordered items is a large single PR; if review pressure demands, the natural split seam is items 1–4 (trait + derives) vs 5–8 (registration + seam + surface) — both halves are independently green, and nothing in the plan depends on them landing together.
