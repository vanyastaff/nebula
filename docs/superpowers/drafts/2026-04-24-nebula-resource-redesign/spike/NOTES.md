# Phase 4 spike — iter-1 notes

Date: 2026-04-24. Branch: `worktree-agent-a6945235`. Validates Strategy
§3.6 trait shape (frozen at CP3) before Phase 6 Tech Spec elaborates it.

## Status — PASSED

All iter-1 exit criteria met:

- `cd spike && cargo check --all-targets` — clean
- `cd spike && cargo test --all-targets` — 6 integration tests pass
- `cd spike && cargo test --doc` — 3 compile-fail probes resolve as
  expected (the bad code does not compile)
- `cd spike && cargo clippy --all-targets` — clean (workspace `all =
  deny` + spike-local hygiene)
- `<Self::Credential as Credential>::Scheme` syntax works at the call
  site (and in `fn` signatures) without contortion
- `type Credential = NoCredential;` reads naturally; default no-op hook
  accepted with no `unwrap()` / footgun pattern
- Parallel `join_all` dispatch + per-resource timeout demonstrates
  isolation under both latency (one resource sleeping 3s, budget 250ms)
  and errors (one resource returning `Err`, siblings still get `Ok`)
- Reverse-index write path is real: `Manager::register::<R>` populates
  `by_credential` for credential-bearing R and skips it for
  `R::Credential = NoCredential`. The lookup site (`on_credential_
  refreshed` / `_revoked`) cannot panic the way today's
  `manager.rs:262/370` `todo!()` does.

## Iteration log

### What compiled on first attempt

- The `Resource` trait shape itself with `type Credential: Credential`,
  the `<Self::Credential as Credential>::Scheme` projection in `create`
  and `on_credential_refresh`, and default no-op futures on the
  rotation hooks.
- `NoCredential` impl of the `Credential` trait — including
  `metadata()` via `CredentialMetadataBuilder`, `project()` returning
  `NoScheme`, and `resolve()` returning `ResolveResult::Complete`.
- 5 topology sub-traits (Pool, Resident, Service, Transport,
  Exclusive) extending `Resource` with their topology-specific methods.
- Mock impls (`MockKvStore`, `MockHttpClient`, `MockPostgresPool`,
  `MockKafkaTransport`, `MockServiceResource`, `MockExclusiveResource`)
  declaring `type Credential` either as `NoCredential` (opt-out) or as
  the real `StaticTokenCredential` projecting `SecretToken`.
- Compile-fail doctests demonstrating: wrong-signature
  `on_credential_refresh` override is rejected; non-`Credential` type
  cannot satisfy `type Credential`; `NoScheme` cannot pretend to be
  `SecretToken`.
- All 6 async integration tests (parallel dispatch, error isolation,
  latency isolation, reverse-index population, register-without-id
  rejection, revocation dispatch).

### What needed an iteration

- **Send/Sync on `&dyn Any` scheme passing.** First attempt typed the
  scheme as `&dyn Any` and the `dispatch_refresh` future failed
  `Send`-ness because `dyn Any` is not `Sync`. Fix: type the scheme as
  `&(dyn Any + Send + Sync)`. Symptom showed up at compile time as
  "future created by async block is not `Send`". One-edit fix, but
  worth flagging — production dispatcher will face the same constraint
  any time it stores or passes erased-type schemes around.

- **Clippy: `manual_async_fn` on Resource impls.** The first cut wrote
  every impl method as `fn name(...) -> impl Future<Output=...> +
  Send { async { ... } }` to mirror the trait declaration. Clippy (1.95
  default `clippy::all`) prefers `async fn` on the impl side. Mirror
  rule does not apply between trait decls (where RPITIT uses `impl
  Future`) and impls (which can use `async fn` and desugar to the
  same). All Resource impls now use `async fn` — cleaner, idiomatic,
  and unblocks the pedantic clippy gate in main workspace.

- **Clippy: `clone_on_copy` for `CredentialId`.** `CredentialId =
  Ulid<CredentialIdDomain>` is `Copy`. Initial tests called
  `cred_id.clone()` reflexively. Trivial cleanup.

### "This almost didn't work" — flag for Phase 6

Nothing in iter-1 actually broke. The §3.6 shape is genuinely
workable. But two things were closer than they looked:

1. **Type erasure on the dispatcher boundary requires `Box<dyn
   ResourceDispatcher>`.** `Manager` holds heterogeneous resources
   keyed by `CredentialId`, and the only stable way to call
   `R::on_credential_refresh(&scheme)` for arbitrary `R` is a
   trampoline closure that downcasts a `&dyn Any` scheme. RPITIT in
   trait objects is not stable on 1.95, so the trampoline returns
   `Pin<Box<dyn Future<...> + Send>>`. **This is a `Box::pin` per
   refresh dispatch on every registered resource.** Tech Spec §5
   should be aware: the dispatcher hot path is *not* zero-allocation
   even though the resource-side hooks are. This cost is incurred ONLY
   on rotation, not on resource acquire — it should be acceptable, but
   it should be acknowledged explicitly rather than discovered.

2. **`NoCredential` requires a real `Credential` impl with real
   `metadata()`.** Because `Credential::metadata()` returns a
   `CredentialMetadata` constructed via a builder that requires a
   non-empty key / name / description / pattern, `NoCredential` can't
   just `unimplemented!()` — it has to provide real (if obviously
   tombstone-shaped) metadata. The spike does this with a
   `credential_key!("no_credential")` static. If `NoCredential` is
   ever registered with a credential store (which it shouldn't be —
   that's a nonsense operation), nothing crashes, but the `metadata()`
   payload is genuinely meaningless. Tech Spec §5 should decide
   whether to (a) special-case `NoCredential` at registration sites
   (reject), or (b) accept that the impl is structurally correct but
   semantically inert.

## Ergonomic findings

### `<Self::Credential as Credential>::Scheme` syntax

**Reads cleanly at call sites.** Examples from the spike:

```rust
fn create(
    &self,
    config: &Self::Config,
    scheme: &<Self::Credential as Credential>::Scheme,
    ctx: &ResourceContext,
) -> impl Future<Output = ...> { ... }
```

```rust
fn _takes_pg_scheme_v2(
    _scheme: &<<MockPostgresPool as Resource>::Credential as Credential>::Scheme,
) {}
```

The double-`as` in the second form is verbose, but it's only needed in
free functions reaching into a foreign resource's credential. Inside
the trait method the simpler `<Self::Credential as Credential>::Scheme`
form is the one users actually write. **Verdict: ergonomic enough,
no DX redesign needed.**

### `type Credential = NoCredential;`

**Maps 1:1 to today's `type Auth = ();`.** No `Option`-wrapping, no
`Self::Sized` extra bounds, no `unwrap()` / `expect()`. The default
`on_credential_refresh` no-op body fires automatically because the
impl never overrides it (because `NoCredential`-bound resources don't
care about rotation). At the manager site, `Manager::register::<R>`
inspects `TypeId::of::<R::Credential>() == TypeId::of::<NoCredential>()`
to skip the reverse-index write. **Verdict: idiomatic, no footgun.**

### Per-resource isolation in `Manager::on_credential_refreshed`

`tokio::time::timeout(per_resource_budget, dispatcher.dispatch_refresh(scheme))`
inside the `join_all` map closure — that's it. Each resource gets its
own bubble. The slow-resource test in `lib.rs` (3s sleep, 250ms
budget) completes in ~270ms wall-clock, demonstrating the isolation
empirically. **Verdict: parallel + per-resource timeout in 6 lines of
code; no contortion.**

## Open questions for Tech Spec CP1

These are not iter-1 blockers — spike PASSED — but they're decisions
CP1 should lock so iter-2 (if needed) and Phase 6 Tech Spec §3-§5 have
clear ground.

1. **Where does `NoCredential` live in production?** Spike defines it
   in `resource-shape` (because it's a resource-side opt-out). Two
   plausible homes in production:
   - `nebula-credential` (next to `Credential` trait): the type IS a
     `Credential` impl, so this is structurally honest.
   - `nebula-resource` (in a `mod no_credential`): the consumer is
     resource-side and `nebula-credential` doesn't otherwise care.
   Tech Spec §3 should pick. Spike found no compile-time difference;
   it's a layering call.

2. **Does `Manager::register::<R>` use `TypeId` to detect
   `NoCredential`, or does it use a sealed marker trait?** Spike uses
   `TypeId::of::<R::Credential>() == TypeId::of::<NoCredential>()`.
   This works but is slightly off-grid (most Rust code does
   sealed-trait or `const SOMETHING: bool`). A sealed trait would
   give the compiler more affordance for static dispatch — but it
   would also force `NoCredential` to live in the same crate as the
   marker.

3. **Box-allocation per dispatch.** Each `dispatch_refresh` call
   allocates a `Box::pin(async move { ... })` because RPITIT in trait
   objects isn't stable. This is a once-per-rotation cost (not
   per-acquire), which should be acceptable, but Tech Spec §6
   observability gate should pick a name for the latency histogram
   that surfaces it (`nebula_resource.credential_rotation_dispatch_
   latency_seconds` already in §4.9).

4. **Per-resource timeout configuration surface.** Spike hardcodes
   the timeout on `Manager::with_timeout`. Strategy §4.3 says "Phase 6
   Tech Spec §5 specifies the timeout configurable surface
   (per-resource budget; default value; surfacing through
   `RegisterOptions`)". Concrete questions: is the budget per-resource
   (i.e. set at register time, varies per R) or per-manager (one
   budget for all)? Spike's shape allows either.

5. **Compile-fail probe coverage.** Spike has 3 doctests covering:
   wrong override sig, type-bound enforcement, `NoScheme` inertness.
   Production should add: (a) `Manager::register::<R>` rejects double
   registration of the same `Arc<R>` against different credentials;
   (b) `register::<R: NoCredential resource>(r, Some(real_id))` — what
   should happen? Spike currently silently ignores the id; production
   may want to warn or reject.

## Cross-crate findings (NOT a blocker)

- **`nebula-credential::Credential::resolve` requires
  `nebula_schema::FieldValues` directly.** Not re-exported by
  `nebula-credential`; the spike had to add `nebula-schema` as a
  direct dep to write a `Credential` impl. Production resource crate
  doesn't need this (it consumes `<Cred as Credential>::Scheme`, never
  declares its own `Credential`), but this is a friction point for any
  third-party that wants to ship a `Credential` impl outside the
  workspace. Worth noting in `feedback_no_shims.md` discussions.

- **`SecretToken` re-exported under `nebula_credential::scheme`** —
  not at root. Used `use nebula_credential::scheme::SecretToken;`. No
  spike issue; just confirming the import path.

## Fitness for iter-2

**Recommendation: iter-2 NOT required for shape validation.** Iter-1
covers all 7 exit criteria from the task brief. Iter-2 would buy:

- Compat sketches against the existing 9 test resources in
  `crates/resource/src/{topology,manager}.rs` (i.e. write the actual
  migration patches and confirm they compile). Useful for Phase 6 §13
  per-consumer migration enumeration but not required for shape
  validation per se.
- Perf bench: how does `Box::pin` per dispatch compare with a
  monomorphized dispatch path? Probably negligible at the rotation
  cadence (rotation is rare), but a 5-line criterion bench would
  silence that question.
- A `final_shape_v2.rs` with full lifecycle methods (`check`,
  `shutdown`, `destroy`) the spike omitted. Would give Phase 6 Tech
  Spec §3 a near-final compile-able shape to copy.

If orchestrator wants to confirm migration mechanics with a Phase 1
real-resource compat sketch before Phase 6, iter-2 is worth ~1 hour.
Otherwise, the shape decision can land at Phase 6 directly.

## Files

- `spike/Cargo.toml` — workspace manifest
- `spike/crates/resource-shape/` — the trait shape library
  - `src/lib.rs` — re-exports
  - `src/resource.rs` — `Resource` trait (Strategy §3.6 shape)
  - `src/no_credential.rs` — `NoCredential` + `NoScheme` opt-out
  - `src/topology.rs` — Pool / Resident / Service / Transport /
    Exclusive sub-traits
  - `src/manager.rs` — `Manager` + `ResourceDispatcher` trampoline +
    parallel `join_all` dispatch with per-resource timeout
- `spike/crates/resource-shape-test/`
  - `src/lib.rs` — 4 mock Resource impls + 6 integration tests
  - `src/compile_fail.rs` — 3 compile-fail doctests
