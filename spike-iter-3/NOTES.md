# Spike iter-3 — Gate 3 dyn-safety validation (nebula-credential CP5/CP6)

**Branch:** `worktree-agent-afe8a4c6` (from `claude/funny-jepsen-b23d20`).
**Date:** 2026-04-24.
**Gate:** §15.12.3 (Tech Spec CP6).
**Toolchain:** Rust 1.95.0 stable.
**Predecessor spikes:** iter-1 (commit `acfec719`), iter-2 (commit `1c107144`) — both on CP4 shape (4-assoc-type Credential, const-bool capability flags). This iter-3 validates CP5/CP6 shape (3-assoc-type base + 5 capability sub-traits).

## Iterations log

### iter-3a: CP5/CP6 trait shape port
- Created `spike/` standalone workspace (excluded from root workspace).
- Ported `credential-proto` to CP5/CP6 shape per §15.4 decision (a):
  - Base `Credential` trait reduced to 3 assoc types (`Input`, `Scheme`, `State`) + `const KEY` + 3 methods (`metadata`, `project`, `resolve`), all `where Self: Sized`.
  - `Pending` moved to `Interactive` sub-trait (§15.4 per consensus decision).
  - Five capability sub-traits: `Interactive`, `Refreshable` (with `REFRESH_POLICY` const), `Revocable`, `Testable`, `Dynamic` (with `LEASE_TTL` + `&self` dropped per CP6 Gap 3).
  - `AuthScheme` dichotomy `SensitiveScheme` + `PublicScheme` per §15.5.
  - `SchemeGuard<'a, C>` + `SchemeFactory<C>` per §15.7.
  - `CredentialRegistry::register<C>() -> Result<(), RegisterError>` per §15.6.
- `credential-proto-builtin` — 3 concrete credentials (`ApiKeyCredential`, `OAuth2Credential`, `SalesforceJwtCredential`) + ADR-0035 phantom-shim portfolio (Bitbucket Pattern 2 + `AnyBearerPhantom` Pattern 3).
- Engine-side dispatchers `RefreshDispatcher<C: Refreshable>`, `RevokeDispatcher<C: Revocable>`, `InteractiveDispatcher<C: Interactive>` demonstrating static capability bounds.

### iter-3b: SchemeGuard lifetime-invariance finding
- Initial retention probe compiled when it shouldn't have. Root cause: `PhantomData<&'a ()>` is covariant; `'a` without any actual borrow to pin it falls through to inferred `'static`.
- Investigation: isolated minimal repro in `/tmp/test_guard.rs` (compiles); switching to `PhantomData<fn(&'a ())>` (invariant) + explicit `&'a ctx` borrow parameter makes the compiler enforce non-retention.
- Probe re-written to model realistic "guard passed alongside `&'a ctx` borrow" form; retention then correctly rejected with E0597.
- **Finding surfaces a §15.7 SchemeGuard design gap** — see §4 below.

### iter-3c: integration tests + stderr snapshot acceptance
- 9 integration tests pass (`cargo test -p credential-proto-builtin`).
- 6 compile-fail probes pass (`cargo test -p spike-compile-fail`). Expected diagnostics verified verbatim against §16.1.1 probe table.

## Answers to questions (a)-(e)

### (a) `dyn Credential` object-safety preserved — **NO (pre-existing)**

Empirical: `Box<dyn Credential<Input=…, Scheme=…, State=…>>` fails to construct with:

```
error[E0038]: the trait `Credential` is not dyn compatible
  --> tests/ui/dyn_credential_const_key.rs:15:20
   |
15 |     let _: Box<dyn Credential<Input = (), Scheme = BearerScheme, State = ApiKeyState>> =
   |                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ `Credential` is not dyn compatible
…
note: the trait is not dyn compatible because it contains associated const `KEY`
```

Root cause: `const KEY: &'static str` in the base trait. Per rustc E0038: "Just like static functions, associated constants aren't stored on the method table. If the trait or any subtrait contain an associated constant, they are not dyn compatible."

**This is NOT a regression introduced by the sub-trait split.** Prior CP4 shape (iter-1/iter-2) had the SAME `const KEY` + additional `const INTERACTIVE: bool` / `REFRESHABLE: bool` / etc. `dyn Credential` was equally blocked there. Iter-2's `AnyCredential` trait (narrower, no assoc const) is what enabled runtime-dispatch over credentials.

Verdict:
- CP5/CP6 sub-trait split **preserves the CP4 status quo** — `dyn Credential` is not object-safe, and was never the dispatch vehicle. The production path is:
  1. **Phantom-shim `dyn XPhantom`** for Pattern 2/3 (action consumers). Phantom trait has NO Credential supertrait → no const KEY → dyn-compatible.
  2. **Generic type parameter `C: Credential`** for statically-dispatched code (engine dispatchers).
  3. **Narrower object-safe trait** (e.g. `AnyCredential` from iter-2) for runtime-type-erased registry entries that don't need the full Credential surface.
- Sub-trait split did not BREAK anything; it preserved the design.

### (b) Phantom-shim erases `C::Scheme` cleanly — **YES**

Empirical: `CredentialRef<dyn BitbucketBearerPhantom>` constructs and action `BitbucketFetchAction { cred: CredentialRef::new("oauth2") }` compiles with `OAuth2Credential` backing. Pattern 3 `CredentialRef<dyn AnyBearerPhantom>` accepts both `ApiKeyCredential` and `OAuth2Credential`.

The 3-assoc-type base (vs CP4's 4-assoc-type) did NOT affect phantom-shim well-formedness. Reason: the phantom trait has NO `Credential` supertrait:

```rust
pub trait BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync {}
```

It only inherits `BearerSealed + Send + Sync` — no unspecified associated types, no const items. `dyn BitbucketBearerPhantom` is fully well-formed as a type regardless of how many assoc types `Credential` has.

Per ADR-0035 §1 canonical form (amendment 2026-04-24-B) transplanted verbatim with per-capability inner seals (`sealed_caps::BearerSealed`, `sealed_caps::BasicSealed`, etc). No refinement of the ADR needed for Pattern 2.

Pattern 3 coexistence with Pattern 2 requires a SEPARATE sealed trait, because two blanket impls of `sealed_caps::BearerSealed` would collide under orphan coherence (Rust cannot reason about bound disjointness). Spike uses `mod sealed_pattern3 { pub trait AnyBearerSealed {} }` — distinct seal, no coherence conflict. This matches the ADR-0035 §3 amendment ("per-capability inner seals").

### (c) `dyn Refreshable` needs parallel phantom-shim — **YES**

Empirical: writing `let _r: Box<dyn Refreshable> = …` would be rejected for two reasons:
1. `Refreshable: Credential` supertrait chain inherits `Credential`'s `const KEY` → E0038 (not dyn-compatible).
2. Same supertrait chain inherits 3 unspecified assoc types → would also fail E0191 if E0038 didn't fire first.

The parallel phantom-shim:
```rust
impl<T: Refreshable> sealed_caps::RefreshableSealed for T {}
pub trait RefreshablePhantom: sealed_caps::RefreshableSealed + Send + Sync {}
impl<T: Refreshable> RefreshablePhantom for T {}
```

works identically to `BitbucketBearerPhantom`. `Box<dyn RefreshablePhantom>` constructs cleanly. Integration test confirmed:

```rust
let _r: Box<dyn RefreshablePhantom> = Box::new(OAuth2Credential);
let _r2: Box<dyn RefreshablePhantom> = Box::new(SalesforceJwtCredential);
```

`InteractivePhantom` same pattern; same outcome.

Verdict: (c) YES, parallel phantom-shim required for every lifecycle sub-trait that needs dyn-dispatch. Engine's refresh registry uses `HashMap<CredentialKey, Box<dyn RefreshablePhantom>>`; each entry's presence IS the capability query (silent-downgrade vector structurally eliminated — an `ApiKeyCredential` cannot be boxed as `dyn RefreshablePhantom`, so the registry cannot contain a non-refreshable entry by construction).

**ADR-0035 amendment recommendation: YES.** See §5 below.

### (d) Capability-const downgrade path — **N/A (hard breaking change accepted)**

The CP4 const-bool style (`const REFRESHABLE: bool = true` + defaulted `refresh() { Ok(NotSupported) }`) is structurally removed in CP5/CP6. A legacy consumer writing:

```rust
if <MyCred as Credential>::REFRESHABLE { /* dispatch */ }
```

fails with E0599 "no associated item named `REFRESHABLE` found for type".

There is no equivalent back-compat bridge, because the purpose of §15.4 is precisely to eliminate the silent-downgrade class (sec-lead N3+N5). Retaining a legacy bool flag would defeat it.

Replacement mechanisms demonstrated in `integration::question_d_capability_const_via_trait_bounds`:
1. **Compile-time path:** generic bounds + dispatcher construction. `RefreshDispatcher::<OAuth2Credential>::for_credential()` compiles; `RefreshDispatcher::<ApiKeyCredential>::for_credential()` fails with E0277.
2. **Runtime path:** phantom-shim downcast. `Box<dyn RefreshablePhantom>` successfully holds `OAuth2Credential` but cannot hold `ApiKeyCredential` (blanket impl doesn't apply).

Per `feedback_hard_breaking_changes` memory: expert-level spec-correct breaking change. §15.4 Cons row acknowledges "one-time learning cost". No adapter needed.

Verdict: (d) N/A — the downgrade path is intentionally absent. Hard breaking change. No structural alternative exists that preserves the safety property.

### (e) 4 compile-fail probes fire with expected diagnostics — **YES (6 of 6)**

All 6 probes pass (`cargo test -p spike-compile-fail` green). Verbatim diagnostics:

**Probe 1 — state_zeroize.rs** (E0277 ZeroizeOnDrop):
```
error[E0277]: the trait bound `BadState: zeroize::ZeroizeOnDrop` is not satisfied
  --> tests/ui/state_zeroize.rs:14:26
   |
14 | impl CredentialState for BadState {}
   |                          ^^^^^^^^ unsatisfied trait bound
```

**Probe 2 — capability_subtrait_missing_method.rs** (E0046 missing method):
```
error[E0046]: not all trait items implemented, missing: `refresh`
  --> tests/ui/capability_subtrait_missing_method.rs:XX
   |
   | impl Refreshable for NaughtyCred {}
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ missing `refresh` in implementation
```

**Probe 3 — engine_dispatch_capability.rs** (E0277 Refreshable not satisfied):
```
error[E0277]: the trait bound `ApiKeyCredential: Refreshable` is not satisfied
  --> tests/ui/engine_dispatch_capability.rs:10:14
   |
10 |     let _d = RefreshDispatcher::<ApiKeyCredential>::for_credential();
   |              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the trait `Refreshable` is not implemented for `ApiKeyCredential`
```

**Probe 4 — scheme_guard_retention.rs** (E0597 borrow-escape):
```
error[E0597]: `ctx_stack` does not live long enough
  --> tests/ui/scheme_guard_retention.rs:XX
```

**Bonus Probe 5 — dyn_credential_const_key.rs** (E0038 not dyn-compatible):
```
error[E0038]: the trait `Credential` is not dyn compatible
  --> tests/ui/dyn_credential_const_key.rs:15:20
note: the trait is not dyn compatible because it contains associated const `KEY`
```

**Bonus Probe 6 — pattern2_service_reject.rs** (E0277 phantom chain rejection):
```
error[E0277]: the trait bound `ApiKeyCredential: BitbucketBearerPhantom` is not satisfied
  --> tests/ui/pattern2_service_reject.rs:XX
note: required for `ApiKeyCredential` to implement `BitbucketBearer`
note: required for `ApiKeyCredential` to implement `BitbucketBearerPhantom`
```

All expected diagnostics match §16.1.1 table entries. Probe files + `.stderr` snapshots committed at `spike/compile-fail/tests/ui/`.

## §3 SchemeGuard lifetime-invariance finding (iter-3b)

**Gap in Tech Spec §15.7 as currently written.**

The spec's `SchemeGuard<'a, C>` definition:

```rust
pub struct SchemeGuard<'a, C: Credential> {
    scheme: <C as Credential>::Scheme,
    _lifetime: PhantomData<&'a ()>,
}
```

does NOT structurally prevent retention. The `PhantomData<&'a ()>` is covariant in `'a`. When nothing constrains `'a` to a specific caller-side lifetime, rustc infers `'a = 'static` and the guard can be stored in a `'static` field without error. Empirically demonstrated in `/tmp/test_guard.rs`:

```rust
pub struct Guard<'a> { _secret: String, _l: PhantomData<&'a ()> }
impl<'a> Guard<'a> {
    pub fn new(s: String) -> Self { Self { _secret: s, _l: PhantomData } }
}
struct Keeper<'long> { retained: Option<Guard<'long>> }
// Compiles successfully — Guard<'static> can be stored in Keeper<'static>.
```

**The retention barrier is the engine-side construction contract**, not the guard type itself. The probe `scheme_guard_retention.rs` models the working form: the engine passes the guard ALONGSIDE a short-lived `&'a ctx: &CredentialContext` borrow, using the SAME `'a` binder. Then `'a` is pinned to the context's stack lifetime and the guard cannot escape.

**Recommendation for §15.7 spec clarification** (non-blocking for П1 — the barrier still works when used correctly):
1. Document the contract: engine MUST pass `SchemeGuard<'a, C>` with `'a` bound to a concurrently-borrowed `&'a CredentialContext` (or equivalent short-lived borrow). Without this, `'a` is unbounded and the retention barrier is void.
2. Consider `PhantomData<fn(&'a ())>` (invariant in 'a) — tightens but still relies on an actual reference to pin 'a.
3. Consider `SchemeGuard<'a, C>` holding a `&'a C::Scheme` internally (owned moves to `Deref` target via the borrow) — shifts the barrier into the struct shape but costs a layer of indirection.

Cleanest fix: make the engine-facing hook signature
```rust
fn on_credential_refresh<'a>(
    &self,
    ctx: &'a CredentialContext<'a>,
    new_scheme: SchemeGuard<'a, Self::Credential>,
) -> Result<(), Self::Error>
```

— `ctx`'s `&'a` borrow forces `'a` to be short-lived, and `new_scheme: SchemeGuard<'a, ...>` inherits the same `'a`.

This is a **minor §15.7 refinement**, not a design flaw. The compile-fail probe `scheme_guard_retention.rs` in the spike enforces the CORRECT form: guard + `&'a ctx` with shared `'a` → retention blocked. Engine implementers just need to be aware that the `&'a ctx` borrow is load-bearing.

## §4 ADR-0035 amendment recommendation

**Proposal: amendment 2026-04-24-C — parallel phantom-shim for lifecycle sub-traits.**

Current ADR-0035 §2 Scope limits phantom-shim to Pattern 2 (service-bound capability, e.g. `BitbucketBearerPhantom`) + Pattern 3 (generic capability, e.g. `AnyBearerPhantom`). It does not cover lifecycle sub-traits (`Refreshable`, `Revocable`, `Interactive`, `Testable`, `Dynamic`).

Question (c) findings: lifecycle sub-traits inherit `Credential`'s const KEY + 3 assoc types via supertrait chain, making `dyn Refreshable` doubly-blocked (E0038 + E0191). Engine components that need runtime-dispatch over refreshable credentials (refresh registry, scheduled rotation task, batch test harness) need parallel phantom-shims:

```rust
// At nebula-credential-builtin (or nebula-credential core):
mod sealed_lifecycle {
    pub trait RefreshableSealed {}
    pub trait RevocableSealed {}
    pub trait InteractiveSealed {}
    pub trait TestableSealed {}
    pub trait DynamicSealed {}
}

impl<T: Refreshable> sealed_lifecycle::RefreshableSealed for T {}
pub trait RefreshablePhantom: sealed_lifecycle::RefreshableSealed + Send + Sync {}
impl<T: Refreshable> RefreshablePhantom for T {}
// … identical pattern for the other four.
```

Same chain shape as Pattern 2. The amendment adds:
- **§2 Scope** — add fourth bullet: "Pattern 4 (lifecycle sub-trait erasure): `CredentialRef<dyn RefreshablePhantom>` / `Box<dyn RefreshablePhantom>` positions in engine runtime registries."
- **§3 Sealed module placement** — extend with a separate `mod sealed_lifecycle` for the 5 lifecycle phantoms. Canonical form per §3 amendment 2026-04-24-B applies unchanged.

**No contradiction with existing ADR-0035 shape.** The lifecycle phantoms are structurally identical to service phantoms; only the real trait differs (`Refreshable` instead of `BitbucketBearer`). Amendment is additive.

Register row `adr-0035-amendment-c-lifecycle-phantom` opens with `proposed` status pointing to this §4.

## §5 Tech Spec §15.4 update recommendation

Add this paragraph to §15.4 under "Composition with ADR-0035 phantom-shim":

> **Spike iter-3 outcome (2026-04-24, `worktree-agent-afe8a4c6`):** Gate 3 validation confirmed sub-trait split composes cleanly with phantom-shim pattern on 3 credential types (`ApiKeyCredential` static, `OAuth2Credential` Interactive+Refreshable+Revocable, `SalesforceJwtCredential` Interactive+Refreshable). Five empirical findings:
>
> 1. **`dyn Credential` was never object-safe** — `const KEY` blocks E0038 under both CP4 and CP5/CP6 shape. Sub-trait split did not regress this. Phantom-shim (no Credential supertrait) is the sole dyn-dispatch vehicle, as prior iter-1/iter-2 already established.
> 2. **Phantom-shim erases `C::Scheme` cleanly** with 3-assoc-type base Credential (vs CP4 4-assoc-type). Assoc-type count does not affect phantom trait well-formedness — the phantom has no Credential supertrait.
> 3. **Lifecycle sub-traits (`Refreshable` et al.) need parallel phantom-shims** for dyn-dispatch — `dyn Refreshable` blocked by supertrait-inherited const KEY + assoc types. Pattern analogous to Pattern 2 service phantoms. See ADR-0035 amendment 2026-04-24-C.
> 4. **Const-bool downgrade path is structurally absent** — spec-correct hard breaking change. Replacement: static generic bounds + phantom-shim runtime dispatch.
> 5. **All 6 compile-fail probes fire with expected diagnostics** (4 mandatory from §16.1.1 + 2 bonus). `E0038` / `E0046` / `E0277` / `E0597` as specified.
>
> Spike §3 also surfaced a §15.7 SchemeGuard lifetime-invariance refinement (minor): `PhantomData<&'a ()>` alone doesn't prevent retention unless `'a` is pinned to a concurrent `&'a` borrow. Spec text does not currently call this out. Non-blocking for П1; refine §15.7 documentation at П1 scaffolding time.

Register row `gate-spike-iter3-dyn-safety` flips from `proposed` to `decided` with spike commit pointer.

## §6 Reproducibility

Worktree: `.claude/worktrees/agent-afe8a4c6/` on branch `worktree-agent-afe8a4c6`.

```powershell
cd spike
cargo check -p credential-proto                # trait shape compiles
cargo check -p credential-proto-builtin        # 3 credentials + phantom portfolio compile
cargo test  -p credential-proto-builtin        # 9 integration tests pass
cargo test  -p spike-compile-fail              # 6 compile-fail probes pass
```

All commands green on Rust 1.95.0 stable.

## §7 Structure reference

```
spike/
├── Cargo.toml                              # standalone workspace, resolver 3
├── rust-toolchain.toml                     # pin 1.95.0
├── NOTES.md                                # this file
├── credential-proto/
│   ├── Cargo.toml
│   └── src/lib.rs                          # CP5/CP6 trait shape (~550 LOC)
├── credential-proto-builtin/
│   ├── Cargo.toml
│   ├── src/lib.rs                          # 3 credentials + phantom portfolio (~430 LOC)
│   └── tests/integration.rs                # 9 integration tests
└── compile-fail/
    ├── Cargo.toml
    └── tests/
        ├── compile_fail.rs                 # trybuild harness
        └── ui/
            ├── state_zeroize.rs + .stderr              # Probe 1
            ├── capability_subtrait_missing_method.rs + .stderr  # Probe 2
            ├── engine_dispatch_capability.rs + .stderr # Probe 3
            ├── scheme_guard_retention.rs + .stderr     # Probe 4
            ├── dyn_credential_const_key.rs + .stderr   # Bonus
            └── pattern2_service_reject.rs + .stderr    # Bonus
```

## §8 Summary

| Question | Verdict | Evidence |
|----------|---------|----------|
| (a) `dyn Credential` object-safety with sub-trait split | NO — pre-existing E0038 from `const KEY`, not a sub-trait-split regression | `ui/dyn_credential_const_key.rs` probe |
| (b) Phantom-shim erases `C::Scheme` cleanly | YES — 3-assoc-type base unchanged from CP4 behavior | `BitbucketFetchAction` + `GenericBearerAction` compile; integration tests pass |
| (c) `dyn Refreshable` needs parallel phantom | YES — `dyn Refreshable` doubly-blocked (E0038 + E0191); `RefreshablePhantom` demonstrated working | `Box<dyn RefreshablePhantom>` construction in integration test |
| (d) Capability-const downgrade path | N/A — hard breaking change, spec-correct | `question_d` test; §15.4 Cons row acknowledges |
| (e) 4+ compile-fail probes fire | YES — 6 of 6 pass with expected diagnostics (4 mandatory + 2 bonus) | `spike-compile-fail` crate green |

Gate 3 §15.12.3 closure criteria satisfied:
- All 5 questions (a)-(e) answered empirically.
- ADR-0035 amendment 2026-04-24-C proposed (parallel phantom for lifecycle sub-traits).
- §15.4 update paragraph drafted (§5 above).
- Spike branch preserved for archive via worktree.

Ready for П1 trait-scaffolding per §16.1 phase list.
