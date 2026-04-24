# Credential redesign spike — NOTES

Iteration-2 pause artifact. Orchestrator review required before
proceeding to iteration 3 (if any — iter-2 clears the prompt's DONE
threshold of ≥4 of 5 scope questions; actually 5-of-5 this time).

**Strategy authority (iter-2 baseline):**
- `docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md`
  — Checkpoint 2 at commit `0100c80a` (after ADR-0035 amendments).
  §3.2 "On `dyn` semantics" paragraph + §3.3 pseudo-Rust example
  SUPERSEDED by ADR-0035.
- `docs/adr/0035-phantom-shim-capability-pattern.md` — two-trait +
  per-crate sealed canonical form. §4.1 mandates manual `mod sealed
  { pub trait Sealed {} }` at crate root.
- `docs/tracking/credential-concerns-register.md` — 130 rows; row
  `arch-authscheme-clone-zeroize` not an iter-2 blocker (Tech Spec
  decides).

Read via `git show` (spike worktree must not alter docs):
- Strategy amendments: `git show 0100c80a:docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md`
- ADR-0035: `git show 0100c80a:docs/adr/0035-phantom-shim-capability-pattern.md`

## Status summary — iteration 2

| Item | Iter-1 | Iter-2 |
|---|---|---|
| Q1 §3.3 — TYPE-LEVEL | PASS | PASS (re-verified under sealed form) |
| Q1 §3.3 — SEMANTIC (compile-time rejection of AppPassword) | PASS | PASS |
| Q2 `#[action]` 0/2+-slot shorthand ambiguity | deferred | **PASS** |
| Q3 H1 PhantomData+TypeId registry — compile | PASS | PASS (indexed, ahash) |
| Q3 H1 — runtime resolve | PASS (5 tests) | PASS (unchanged) |
| Q3 H2 proc-macro binding table — compile | deferred | **PASS** |
| Q3 H3 typed accessor methods — compile | deferred | **PASS** |
| Q3 baseline + h1/h2/h3 Criterion benches | **none** | **4/4 DONE; all << 1µs** |
| Q3a §3.5 mechanism (i) trait-resolution cross-check | deferred | **PASS** |
| Q3a §3.5 mechanism (ii) compile-time capability registry | deferred | not attempted — (i) suffices |
| Q4 DualAuth (mTLS + Bearer) — compile | deferred | **PASS** |
| Q4 DualAuth — runtime + wrong-scheme rejection | deferred | **PASS** (test + compile-fail) |
| Q7 two-crate split | PASS | PASS |
| ADR-0035 §1 canonical form applied | n/a | **APPLIED with refinement** |
| ADR-0035 §5 minimum bounds verified | n/a | **DONE** |
| External-forge compile-rejection (sealed-trait hint) | n/a | **VERIFIED** |

### Scope questions resolved (prompt success criteria)

**5 of 5** scope questions (Q1, Q2, Q3, Q4, Q7) resolved with concrete
code + tests. Prompt DONE threshold is ≥4 of 5 — iter-2 exceeds it.

### Fallback selection: **NONE**

§3.3 passes ⇒ Fallback A off the table. §3.5 mechanism (i) passes ⇒
Fallback B off the table. Pattern 2 retained + macro cross-check
compile-enforced.

### Perf decision

All three hypotheses p95 well under 1µs (≈8ns worst at upper CI).
**H1 is the pick** (with H3 inline form as macro sugar). Per iter-2
§3, H2 binding table is over-engineered for this workload — the
fn-pointer indirection solves a non-problem. Rationale in §3.

## §1 What changed iter-1 → iter-2

### Files modified

Both crates:
- `credential-proto/Cargo.toml` — +ahash 0.8 (perf-grade hasher).
- `credential-proto/src/lib.rs` — CredentialKey now Arc<str>;
  CredentialRegistry indexed on Arc<str> via ahash; `resolve_concrete`
  and `resolve_any` take `&str` (no CredentialKey::clone on hot path);
  registry.new() uses fixed-seed ahash (deterministic benches).

- `credential-proto-builtin/Cargo.toml` — +criterion dev-dep, +4
  bench harnesses, +3 more compile-fail examples.
- `credential-proto-builtin/src/lib.rs` — ADR-0035 canonical form
  applied. `mod sealed_caps` (per-capability Sealed) added. Phantom
  traits re-bounded on sealed supertrait, `'static` dropped. Dual-
  auth: TlsIdentityScheme + MtlsClientCredential + TlsIdentityPhantom;
  service-agnostic BearerPhantom; Resource trait + MtlsHttpResource
  + MtlsHttpAction; resolve_mtls_pair.

Files added:
- `benches/bench_baseline.rs`, `bench_h1.rs`, `bench_h2.rs`, `bench_h3.rs`.
- `tests/pattern2_dispatch.rs`, `tests/dualauth.rs`, `tests/action_macro_q2.rs`.
- `examples/compile_fail_external_forge.rs` — sealed-trait rejection proof.
- `examples/compile_fail_app_password_to_bearer_projection.rs` — §3.5 (i) proof.
- `examples/compile_fail_dualauth_wrong_scheme.rs` — Q4 §3.5 cross-check.
- `examples/compile_fail_zero_slot_credential_shorthand.rs` — Q2 0-slot.
- `examples/compile_fail_two_slot_credential_shorthand.rs` — Q2 2+-slot.
- `spike/final_trait_shape_v2.rs` — distilled iter-2 snapshot.

Total compile-fail examples: 7 (2 from iter-1, 5 new). All fail as required.
Total runtime tests: 11 (5 from iter-1, 6 new). All pass.

## §2 ADR-0035 canonical form — iter-2 application + REFINEMENT

### Applied verbatim

- `mod sealed_caps` at crate root (crate-private module, inner `pub trait …Sealed` visible within crate).
- Two-trait chain per capability:
  1. **Real trait** — `BitbucketBearer: BitbucketCredential`, blanket-
     impl'd for `T: BitbucketCredential, T::Scheme: AcceptsBearer`.
  2. **Sealed blanket** — `impl<T: BitbucketBearer> sealed_caps::BearerSealed for T`.
  3. **Phantom trait** — `BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync`
     (dropped `'static` per §5 verification — see §6 below).
- Blanket `impl<T: BitbucketBearer> BitbucketBearerPhantom for T`.

### Refinement beyond ADR-0035 §3 text — per-capability inner Sealed traits

**Problem.** ADR-0035 §3 prescribes `mod sealed { pub trait Sealed {} }`
with a **single** `Sealed` trait shared by every phantom. The spike
attempted this form and hit a Rust orphan-coherence wall:

```rust
// Naive form — two blanket Sealed impls for different capability bounds:
impl<T: BitbucketBearer> sealed::Sealed for T {}
impl<T: BitbucketBasic>  sealed::Sealed for T {}  // COHERENCE REJECTED
```

The compiler sees "T could conceivably satisfy both `BitbucketBearer`
AND `BitbucketBasic`" (even though no concrete type does — orphan rules
don't look at that), declares the two blankets overlapping, and rejects.
This is Rust's trait-coherence machinery working as intended — same root
rule that rejects:
```
impl<T: TraitA> MyTrait for T {}
impl<T: TraitB> MyTrait for T {}
```
for any non-disjoint `TraitA`/`TraitB`.

**Resolution — one Sealed trait per capability.** The spike uses:
```rust
mod sealed_caps {
    pub trait BearerSealed {}
    pub trait BasicSealed {}
    pub trait TlsIdentitySealed {}
    pub trait GenericBearerSealed {}
}
impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}
impl<T: BitbucketBasic>  sealed_caps::BasicSealed  for T {}
// …
```
Each phantom has its own sealed supertrait. No coherence collision.
Same ADR-0035 §3 crate-private module structure; just one `pub trait
*Sealed` per capability instead of a single shared `Sealed`.

**Impact for ADR-0035 text**:
- Current §3 wording "mod sealed { pub trait Sealed {} }" implies a
  single Sealed trait. Works only if the crate has exactly one phantom,
  or if the capability traits are mutually disjoint by supertrait chain
  (rare — our service-trait `BitbucketCredential` is supertrait to
  BOTH `BitbucketBearer` and `BitbucketBasic`).
- **Recommended ADR-0035 addendum**: name the inner sealed trait per
  capability. `mod sealed { pub trait BearerSealed {} pub trait BasicSealed {} … }`.
  The `#[capability]` macro (ADR-0035 §4) emits the per-capability
  Sealed **declaration** (inside `mod sealed`) as part of expansion,
  with a name derived from the capability. The user still manually
  declares `mod sealed` once at crate root; the macro fills it with
  capability-specific entries.
- This does NOT invalidate ADR-0035's core claim (the two-trait sealed
  pattern works). It sharpens the §3 form.

### External-forge compile rejection — VERIFIED

`examples/compile_fail_external_forge.rs` attempts to impl
`BitbucketBearerPhantom` for an external `RogueCredential` struct.
Compilation fails with rustc's built-in sealed-trait hint:

```
error[E0277]: the trait bound `RogueCredential: sealed_caps::BearerSealed` is not satisfied
  = note: required for `RogueCredential` to implement `BitbucketBearer`
  = note: required for `RogueCredential` to implement `sealed_caps::BearerSealed`
note: required by a bound in `BitbucketBearerPhantom`
  …
  = note: `BitbucketBearerPhantom` is a "sealed trait", because to
          implement it you also need to implement
          `credential_proto_builtin::sealed_caps::BearerSealed`, which is
          not accessible; this is usually done to force you to use one
          of the provided types that already implement it
```

rustc's own sealed-trait diagnostic fires — confirms the pattern is
recognized by the compiler as the idiomatic sealed-trait shape.

## §3 Pattern 2 end-to-end dispatch — KEY FINDING

### Setup

Orchestrator flagged: "Iter-1 validated only `dyn position compiles +
registry lookup returns Box<dyn AnyCredential>`. Did NOT validate the
realistic action code path." Iter-2 validated it via `tests/pattern2_dispatch.rs`.

### Finding — `Credential::project` on a dyn is impossible; engine-side projection wins

The action body DOES NOT receive `&dyn BitbucketBearerPhantom`. It
receives a projected scheme reference. Why:

- `Credential::project(state: &Self::State) -> Self::Scheme` is `where
  Self: Sized`. Not callable on `&dyn AnyCredential`.
- The action-struct's phantom-typed field is **purely a compile-time
  binding signal** — it says "this slot expects a bearer-capable
  credential" and nothing more.
- At invocation, the engine already has the typed `Credential::State`
  (it just decrypted it from storage using the credential's typed
  State — that's how `nebula-storage` works). With typed state in hand,
  the engine calls `C::project(&state)` at full type-knowledge and
  hands the action a `&Scheme`.

Consequence: H1's concern that "action code must enumerate
`downcast_ref::<SlackOAuth2>() else downcast_ref::<SlackPAT>() else …`"
**does not apply** — enumeration happens only at macro-generated resolve
sites, where the type set is fixed by the macro's compile-time knowledge
of the action's declared capability bound. The action body never touches
Credential.

### Mapping to Strategy §3.5 mechanism (i)

The engine-side resolve fn looks like:
```rust
fn resolve_as_bearer<C>(
    reg: &CredentialRegistry,
    key: &str,
    state: &C::State,
) -> Option<ResolvedBearer>
where
    C: Credential<Scheme = BearerScheme>,
{
    let _ = reg.resolve_concrete::<C>(key)?;
    Some(ResolvedBearer(C::project(state)))
}
```

The `where C: Credential<Scheme = BearerScheme>` is **Strategy §3.5
mechanism (i) trait-resolution cross-check** in concrete form — the
compiler enforces at the resolve-site that the concrete C projects to
the right scheme. This is independent of (and complementary to) the
phantom bound at the action struct declaration:
- **Declaration-site check** (phantom trait on `CredentialRef<dyn …>`):
  the ACTION author cannot write code that references a non-conforming
  credential type in their struct.
- **Resolve-site check** (where-clause on macro-generated resolve fn):
  the ENGINE cannot instantiate a resolve path that projects to the
  wrong scheme.

Both mechanisms are compile-enforced. Both are required for the whole
system to type-check end-to-end.

### Compile-fail proof

`examples/compile_fail_app_password_to_bearer_projection.rs`:
```
error[E0271]: type mismatch resolving `<BitbucketAppPassword as Credential>::Scheme == BearerScheme`
  |
  |     let _ = resolve_as_bearer::<BitbucketAppPassword>(…);
  |                                 ^^^^^^^^^^^^^^^^^^^^ expected `BearerScheme`, found `BasicScheme`
```

Readable and direct — no macro-generated noise. This is the diagnostic
Tech Spec can rely on for user-facing errors.

### H2/H3 decision follow-through

H2's binding table adds fn-pointer indirection for a problem that
doesn't exist: the macro already knows the concrete type per slot at
expansion time, so H1/H3 inline the whole chain. H2 is useful only
if the slot set is dynamic at runtime (e.g. reflective credential
loading) — which this subsystem explicitly doesn't do. **H2 rejected
on ergonomics grounds**, not perf grounds (perf is fine). H3 is
H1 with macro-generated typed accessor method names — semantic
equivalent.

## §4 Perf benches — ALL under budget

All four benches ran under Criterion 0.5 with `--measurement-time 2
--warm-up-time 1 --sample-size 100`. Fixed-seed ahash; 64-entry
registry; release profile.

| Bench | Mean (ns) | Upper CI (p95 proxy) | Budget (1000ns) | Verdict |
|---|---|---|---|---|
| baseline | 5.54 | 5.56 | 0.6% | **eligible** |
| h1 | 6.44 | 6.48 | 0.6% | **eligible** |
| h2 | 8.34 | 8.35 | 0.8% | **eligible** |
| h3 | 6.25 | 6.28 | 0.6% | **eligible** |

Deltas:
- h1 over baseline: +0.9 ns (AnyCredential vtable indirection + as_any).
- h2 over h1: +1.9 ns (fn-pointer indirect + TypeId check).
- h3 vs h1: -0.2 ns (inlined accessor; within noise).

All four are ~150x under the absolute 1µs budget. **Perf is NOT the
decision-driving factor.** Ergonomics + dyn-safety + macro-expansion
simplicity all point the same direction: H1 shape with H3 macro-generated
accessor sugar.

### Caveats (flagged for Tech Spec)

1. **64-entry registry.** Production registries will be larger.
   HashMap lookup is O(1) amortized with load-factor-bounded collision
   chains — 10x registry size ≈ 1.1–1.5x lookup cost, still far
   below 1µs. If Tech Spec wants conservative numbers, re-bench
   with 10k entries; I'd expect h1 ~8ns, not 60ns.
2. **Fixed-seed ahash.** Production would use `ahash::RandomState::new()`
   (runtime-rng feature). Seeding adds ~1 atomic load per map
   construction; doesn't change per-lookup cost.
3. **Warm-cache, no contention.** The benches are single-threaded
   hot-loop. Real workflow executions spawn across a thread pool;
   first-access cost includes cache miss (+10–20ns). Still well
   under 1µs.
4. **`block_on` / async wrapping.** Production credential resolve is
   async-wrapped. The async overhead (maybe 50–100ns for tokio's
   Poll machinery on a trivial future) dwarfs the sync path benches
   here. But that overhead is shared by ALL three hypotheses — doesn't
   affect the cross-hypothesis decision. Tech Spec should re-bench
   with the async wrapping once the `impl Future` shape for `resolve`
   is stable.

## §5 §3.5 mechanism (ii) — not attempted

Candidate mechanism (ii) was: "compile-time capability registry via
`inventory`-style or explicit `register_resource_auth::<R, C>()` at
plugin init; macro performs lookup of tag pairs at expansion."

Not attempted in iter-2 because mechanism (i) (trait-resolution
cross-check) lands cleanly and covers the §3.5 obligation. Per Strategy
§3.7, if EITHER mechanism works, §3.5 passes and Fallback B does not
activate. `inventory` is separately rejected by Strategy §2.1 on
cross-crate unreliability grounds, so (ii) was never the preferred
path anyway.

Flag for Tech Spec: if (i) turns out inadequate at macro-expansion
time in the actual proc-macro implementation (e.g. the macro can't
see the Resource's AcceptedAuth assoc type without trait-solver tricks
that don't work in stable 1.95 proc-macros), (ii) via explicit
`register_resource_auth!` declarations is the fallback. Not an iter-2
concern.

## §6 Minimum bounds verification (ADR-0035 §5 obligation)

Starting form (ADR-0035 §1): `Phantom: sealed::Sealed + Send + Sync + 'static`.

### Empirical test sequence

1. **Dropped `'static`.** `cargo build` + `cargo test --tests` pass.
   All 7 compile-fail examples continue to fail. Reason: `CredentialRef
   <dyn Phantom>` is declared as a struct field (no lifetime param on
   the struct); Rust 2021+ default-object-lifetime rules make `dyn
   Phantom` within that struct field default to `dyn Phantom + 'static`.
   The explicit `'static` on the phantom trait is therefore redundant.
   **Decision: DROPPED.**
2. **Dropped `Send + Sync`.** `cargo build` + `cargo test --tests`
   also pass. Reason: `CredentialRef<C: ?Sized>` uses `PhantomData<fn()
   -> C>` which is Send+Sync regardless of C. The auto-derive works
   through the PhantomData shape.
   **Decision: KEPT anyway.** Rationale: forward-compat. A consumer
   might use `&dyn Phantom` or `Box<dyn Phantom>` OUTSIDE a
   `CredentialRef` (e.g. during registration transients, channel
   sends across threads, etc). Dropping Send+Sync on the phantom
   trait forward-leaks the constraint to every such consumer. The
   cost of keeping Send+Sync in the bound is zero at runtime; the
   cost of re-adding it later is a breaking change. Stability wins.

### Final bounds

```rust
pub trait BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync {}
```

vs. ADR-0035 §1 starting form:
```rust
pub trait BitbucketBearerPhantom: sealed::Sealed + Send + Sync + 'static {}
```

`'static` dropped. `Send + Sync` kept. Sealed supertrait renamed per §2
refinement (per-capability Sealed).

**Recommendation for ADR-0035 addendum**: strike `'static` from the
phantom supertrait; add note that `'static` is redundant under 2021+
default-object-lifetime rules for struct-field-typed `dyn Phantom`
positions.

## §7 Compat sketches — iter-1 carried forward

All 5 iter-1 compat sketches (trigger binding, multi-step persistent
flow, mid-refresh race, provider URL templating, external-provider
typed resolve) remain COMPAT with the iter-2 shape. The ADR-0035
canonical form and the indexed registry do not affect any of them.

One iter-2 addendum to sketch #2 (multi-step persistent flow):
`continue_resolve`'s signature was not touched in iter-2; the Pending
enum approach (Shape A) remains the recommended accommodation.
`continue_resolve` still has its iter-1 compat risk if dynamic-N
flows become material — Tech Spec decides.

## §8 Iteration-2 reproducibility

All commands from `spike/credential-proto-builtin/`.

**Build + tests** (must all succeed):
```bash
cargo build
cargo test --tests    # 11 tests pass
```

**Compile-fail examples** (must all FAIL):
```bash
for ex in compile_fail_app_password_is_not_bearer \
          compile_fail_app_password_via_phantom \
          compile_fail_external_forge \
          compile_fail_app_password_to_bearer_projection \
          compile_fail_dualauth_wrong_scheme \
          compile_fail_zero_slot_credential_shorthand \
          compile_fail_two_slot_credential_shorthand; do
    cargo build --example "$ex" 2>&1 | grep -E "^error\[" | head -1
done
```

**Benches** (all four; each runs ~4 seconds):
```bash
cargo bench --bench bench_baseline -- --measurement-time 2 --warm-up-time 1 --sample-size 100
cargo bench --bench bench_h1       -- --measurement-time 2 --warm-up-time 1 --sample-size 100
cargo bench --bench bench_h2       -- --measurement-time 2 --warm-up-time 1 --sample-size 100
cargo bench --bench bench_h3       -- --measurement-time 2 --warm-up-time 1 --sample-size 100
```

## §9 Blockers / questions for orchestrator

Before iter-3 (if any):

1. **ADR-0035 §3 refinement — per-capability Sealed traits.** The
   coherence-collision issue described in §2 is real; ADR-0035's
   current §3 text implies a single Sealed trait and will not compile
   on a real crate with ≥2 capability sub-traits of the same service.
   Recommended: amend ADR-0035 §3 to name the per-capability Sealed
   (e.g. `sealed::BearerSealed`) and update §4 macro contract so
   `#[capability(...)]` emits its own `pub trait XSealed {}`
   declaration into `mod sealed` at expansion.
2. **ADR-0035 §5 `'static` bound — drop.** Per §6 this addendum
   is concrete. Strike `'static` from ADR-0035 §1 phantom form.
3. **§3.5 mechanism (ii) — do we need it?** (i) alone passes.
   Recommended: explicitly mark (ii) as optional/fallback in the
   Tech Spec; don't require both. This closes a source of scope
   creep.
4. **Pattern-2 dispatch path declaration.** The §3 finding (engine-
   side projection via where-clause, action receives `&Scheme`)
   is important for Tech Spec — it tells readers "the phantom trait
   is for type-checking at struct declaration; the runtime path is
   where-clause-constrained resolve." This narrative should land in
   Tech Spec §resolve-flow (or equivalent). Not blocking for iter-3,
   just a writing handoff.
5. **Remaining credential types (Slack, Anthropic, AwsSigV4+Sts,
   Postgres, SalesforceJwt) + 2 actions (GenericSlackAction,
   GenericHttpBearerAction).** Orchestrator call: do these land in
   iter-3, or is the iter-2 set (Bitbucket triad + mTLS + dual-auth
   action) sufficient for the spike's purpose? The trait shape is
   proven; more credential types just re-verify the same pattern.
   I lean "skip iter-3; proceed to Tech Spec" unless orchestrator
   wants broader stress-test.

## §10 Process note

Per prompt §Inter-iteration checkpoint:
- This NOTES.md + `final_trait_shape_v2.rs` + the iter-2 changes
  committed to the worktree branch (`worktree-agent-a23a1d2c`).
- Commit message prefix: `chore(spike): iteration 2 complete — PAUSE
  for orchestrator review` (project convco hook rejects plain `spike:`
  prefix; same mitigation as iter-1).
- After commit + final message, spike STOPS making tool calls.
  Orchestrator SendMessage signals whether to continue to iter-3 (or
  finish at iter-2 if ≥4-of-5 threshold is met — which it is).
