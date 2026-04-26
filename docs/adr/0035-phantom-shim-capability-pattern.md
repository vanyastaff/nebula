# ADR-0035: Phantom-shim capability pattern

## Status

**Proposed** 2026-04-24, with amendments applied 2026-04-24-B post spike iter-2 validation (worktree branch `worktree-agent-a23a1d2c`, commit `1c107144`), 2026-04-24-C post spike iter-3 (worktree branch `worktree-agent-afe8a4c6`, commit `f36f3739`), 2026-04-26 post Stage 4 review (visibility-symmetry clarification on macro emission), and 2026-04-26-B post PR #582 review (rename of `#[action]` attribute macro to `#[action_phantom]` to avoid collision with the `#[derive(Action)]` helper attribute).

**Post iter-2 amendments applied** (canonical-form corrections, not stylistic):

- **§3 Sealed module placement** — canonical form corrected from single shared `Sealed` to per-capability inner `Sealed` traits. Rust coherence collision on shared `Sealed` when two capabilities share a service supertrait blocks the original form for any realistic multi-capability crate. Details in §3 amendment note.
- **§5 Minimum bounds verification** — outcome DECIDED: `'static` dropped as redundant under Rust 2021+ default-object-lifetime rules; `Send + Sync` kept as forward-compat stability promise.
- **§1 canonical form** updated to reflect both amendments (pseudo-Rust with `sealed_caps::BearerSealed`, `Phantom: XSealed + Send + Sync`).

**Post iter-3 amendment applied 2026-04-24-C** (additive, not a correction):

- **§2 Scope** extended with **Pattern 4 — lifecycle sub-trait erasure** for `dyn RefreshablePhantom` / `dyn InteractivePhantom` / `dyn RevocablePhantom` / `dyn TestablePhantom` / `dyn DynamicPhantom`. Engine-side runtime registries that need to iterate over all credentials satisfying a lifecycle capability (e.g., proactive refresh scheduler over all `Refreshable` credentials) use the same phantom-shim pattern as Pattern 2/3. Structurally identical; additive. Details in §2 Pattern 4 amendment note.
- **§3 Sealed module placement** extended convention — `mod sealed_lifecycle { pub trait RefreshableSealed {} pub trait InteractiveSealed {} ... }` per-capability seal analogous to `mod sealed_caps` for service capabilities. Same per-capability inner trait form per §3 amendment 2026-04-24-B rationale; same coherence correctness.

**Post Stage 4 amendment applied 2026-04-26** (visibility-symmetry clarification, not a correction):

- **§1 canonical form** clarified — the phantom trait inherits the visibility of the capability trait declared with `#[capability]`. `pub trait BitbucketBearer` produces `pub trait BitbucketBearerPhantom`; `pub(crate) trait` produces `pub(crate) trait`. This composes correctly with crate-internal capabilities, where forcing the phantom to `pub` would leak a private capability into the public surface. The `#[capability]` macro emits the phantom with `let vis = &trait_def.vis;` to preserve this symmetry. Details in §1 visibility-symmetry note. Cross-references the macro emission decision in §4.

Amends portions of [Strategy §3.2 / §3.3](../superpowers/specs/2026-04-24-credential-redesign-strategy.md) (Checkpoint 1, frozen at commit `d5045774`):

- §3.2 "On `dyn` semantics" paragraph — superseded (dyn-safety framing conflated well-formedness and runtime dispatch).
- §3.3 pseudo-Rust example — superseded (supertrait-chain form not well-formed as `dyn`).
- §3.3 closing paragraph — updated reference to this ADR.

Preserves without modification: Strategy §2 Foundational decisions, §3.1 (Credential shape), §3.4 (H1/H2/H3 hypotheses — iter-2 decided H1 with H3 inline form), §3.5 (macro cross-check — iter-2 validated mechanism (i)), §3.6 (trait-heaviness discipline), §3.7 (fallbacks — none triggered).

Post-validation:
- **Iter-1** (commit `acfec719`): confirmed the initial two-trait form compiles + enforces Strategy §3.3 semantics on the Bitbucket triad.
- **Iter-2** (commit `1c107144`): validated the amended per-capability sealed form (re-ran Bitbucket triad under new shape), all 11 integration tests pass, all 7 compile-fail probes fail with expected diagnostics (`E0277` × 3 / `E0271` × 2 / `E0599` × 2), 4 Criterion perf benches all ~150× under 1µs absolute budget.

## Context

Strategy Checkpoint 1 §3.2 stated that `dyn BitbucketBearer` in `CredentialRef<dyn BitbucketBearer>` is a "nominal bound for compile-time type-checking, not a classical vtable trait object", framing the runtime path as type-erased through `AnyCredential`. The framing conflated two independent compile-time concerns:

1. **Type well-formedness** — whether `dyn T` is a valid Rust type at the point of use.
2. **Runtime dispatch** — whether a vtable materializes at runtime.

Rust's well-formedness check is a compile-time constraint that applies independently of runtime dispatch. If trait `T` has any ancestor with unspecified associated types, `dyn T` is rejected with `E0191` at the point of use — the rejection happens before any consideration of runtime vtable materialization.

Strategy §3.3 prescribed:

```rust
pub trait BitbucketBearer: BitbucketCredential {}
impl<T> BitbucketBearer for T where T: BitbucketCredential, T::Scheme: AcceptsBearer {}
```

`Credential` has 4 associated types (`Input`, `State`, `Scheme`, `Pending`). Even with methods declared `where Self: Sized` (which keeps them out of the vtable), `dyn Credential` requires all four assoc types specified. `dyn BitbucketCredential` inherits the constraint through the supertrait chain, and `dyn BitbucketBearer` inherits it transitively. Therefore `CredentialRef<dyn BitbucketBearer>` — the form §3.3 prescribed for Pattern 2 action consumers — does not compile.

Spike iteration-1 confirmed the rejection:

```
error[E0191]: the value of the associated types `Input`, `Pending`,
              `Scheme` and `State` in `Credential` must be specified
  --> src/lib.rs:207:31
   |
207|     pub bb: CredentialRef<dyn BitbucketBearer>,
   |                               ^^^^^^^^^^^^^^^
```

The §3.2 "nominal bound" reassurance did not address the well-formedness check. Strategy §3.3 as written is not a compilable pattern.

## Decision

Adopt the **two-trait phantom-shim pattern** for Pattern 2 and Pattern 3 `CredentialRef<dyn …>` positions. Pattern 1 (concrete `CredentialRef<ConcreteCredential>`) is unaffected — no `dyn` projection, no well-formedness gap.

### §1. Canonical form

```rust
// Module-private sealing — per-capability inner traits (amendment 2026-04-24-B).
// Shared-`Sealed` form would cause Rust coherence collision when two
// capabilities sub-trait a common service trait (see §3 amendment note).
mod sealed_caps {
    pub trait BearerSealed {}
    // … one inner trait per capability exposed by this crate
    //   (BasicSealed, TlsIdentitySealed, SigningSealed, …)
}

// "Real" capability trait — supertrait-chained for compile-time constraint.
// Used only for blanket-impl eligibility. NOT usable in `dyn` positions
// (inherits Credential's 4 unspecified assoc types).
pub trait BitbucketBearer: BitbucketCredential {}

impl<T> BitbucketBearer for T
where
    T: BitbucketCredential,
    T::Scheme: AcceptsBearer,
{}

// Sealed blanket — only types satisfying BitbucketBearer gain
// BearerSealed membership. External crates cannot impl
// sealed_caps::BearerSealed for their own types (sealed_caps is
// crate-private), therefore external crates cannot manually impl the
// phantom trait — they must go through the blanket, which requires
// BitbucketBearer membership.
impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}

// "Phantom" capability trait — dyn-safe marker for dyn positions.
// Supertrait is sealed_caps::BearerSealed + Send + Sync. NO Credential
// supertrait → no unspecified-assoc-type closure → `dyn BitbucketBearerPhantom`
// is well-formed as a type. `'static` dropped per §5 verification
// (redundant under Rust 2021+ default-object-lifetime rules).
pub trait BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync {}

impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}

// Consumer (action) uses the phantom in the dyn position:
#[action(credential)]
pub bb: CredentialRef<dyn BitbucketBearerPhantom>,
```

Resolution walk at compile time for the Bitbucket triad:

- `BitbucketOAuth2` with `Scheme = BearerScheme` → `BearerScheme: AcceptsBearer` ✓ → satisfies `BitbucketBearer` (blanket) → satisfies `sealed::Sealed` (blanket) → satisfies `BitbucketBearerPhantom` (blanket) ✓
- `BitbucketPat` with `Scheme = BearerScheme` → same path ✓
- `BitbucketAppPassword` with `Scheme = BasicScheme` → `BasicScheme: AcceptsBearer` ✗ → does NOT satisfy `BitbucketBearer` → does NOT satisfy `sealed::Sealed` → does NOT satisfy `BitbucketBearerPhantom` → **compile error** at action declaration ✓

Spike iteration-1 reproducer (`spike/credential-proto-builtin/examples/compile_fail_app_password_via_phantom.rs`) confirms the diagnostic chain:

```
error[E0277]: the trait bound `BasicScheme: AcceptsBearer` is not satisfied
  → required for BitbucketAppPassword to implement BitbucketBearer
  → required for BitbucketAppPassword to implement BitbucketBearerPhantom
```

#### §1 Visibility-symmetry note (2026-04-26, post Stage 4 review)

The phantom trait inherits the visibility of the capability trait declared with `#[capability]`. Concretely:

| Capability declaration                              | Emitted phantom trait                                |
|-----------------------------------------------------|------------------------------------------------------|
| `pub trait BitbucketBearer: …`                      | `pub trait BitbucketBearerPhantom: …`                |
| `pub(crate) trait LocalCapability: …`               | `pub(crate) trait LocalCapabilityPhantom: …`         |
| `pub(super) trait ModScopedCap: …`                  | `pub(super) trait ModScopedCapPhantom: …`            |

**Rationale.** Forcing the phantom to a fixed visibility (e.g. always `pub`) would leak a `pub(crate)` capability through the phantom into the crate's public surface — defeating the visibility constraint the author wrote. Inheriting visibility from the capability trait keeps the public surface symmetric: a crate-internal capability composes only with crate-internal phantom-typed positions; a public capability admits public phantom-typed positions across crate boundaries. This is the only direction that composes correctly with module-private capability traits.

**Macro emission contract.** The `#[capability]` macro implementation reads `let vis = &trait_def.vis;` from the parsed `ItemTrait` and applies it verbatim to the emitted phantom trait declaration (see §4 macro emission contract and `crates/credential/macros/src/capability.rs`). Plugin authors who want a strictly-public phantom paired with a crate-internal capability can declare both manually following the §1 canonical form — the macro is the convenience path, not the only legal shape.

**Sealing remains independent of visibility.** The crate-private `mod sealed_caps` (§3) is a separate axis: it prevents *external* crates from forging capability membership regardless of whether the visible capability/phantom pair is `pub` or `pub(crate)`. A `pub(crate) trait LocalCapabilityPhantom` is still sealed against the same crate's other `mod`s (other modules cannot impl it for foreign types because they cannot reach `crate::sealed_caps::LocalCapabilitySealed`). Visibility-symmetry is about API exposure; sealing is about authorship.

### §2. Scope

The phantom-shim pattern applies to:

- **Pattern 2** (service-bound capability): `CredentialRef<dyn ServiceXBearerPhantom>` positions.
- **Pattern 3** (capability-only utility): `CredentialRef<dyn AcceptsBearerPhantom>` positions.
- **Pattern 4** (lifecycle sub-trait erasure, added 2026-04-24-C post spike iter-3): `Box<dyn RefreshablePhantom>` / `Box<dyn InteractivePhantom>` / `Box<dyn RevocablePhantom>` / `Box<dyn TestablePhantom>` / `Box<dyn DynamicPhantom>` positions — engine runtime registries iterating over all credentials satisfying a lifecycle capability.

**Pattern 1** (concrete per-credential-type, e.g. `CredentialRef<SlackOAuth2Credential>`) does NOT use phantom — the type parameter is a concrete `Credential`, no `dyn` projection, no well-formedness gap. Pattern 1 ergonomics are unchanged by this ADR.

#### §2 Pattern 4 amendment note (2026-04-24-C, post spike iter-3 validation at commit `f36f3739`)

Spike iter-3 (Tech Spec CP6 Gate 3 §15.12.3) validated that Tech Spec CP5/CP6 sub-trait capability split (`Interactive` / `Refreshable` / `Revocable` / `Testable` / `Dynamic` per Tech Spec §15.4) introduces a new dyn-dispatch axis distinct from the service-capability axis that Patterns 2/3 cover. Empirically:

- `Refreshable: Credential` inherits `Credential`'s `const KEY` + 3 assoc types (`Input`/`Scheme`/`State` under CP5/CP6 shape; `Pending` moved to `Interactive`). Both blockers fire: `E0038` (const KEY not object-safe) + `E0191` (unspecified assoc types). `dyn Refreshable` rejected.
- Parallel phantom chain resolves the block identically to Pattern 2:
  ```rust
  mod sealed_lifecycle {
      pub trait RefreshableSealed {}
      pub trait InteractiveSealed {}
      pub trait RevocableSealed {}
      pub trait TestableSealed {}
      pub trait DynamicSealed {}
  }

  impl<T: Refreshable> sealed_lifecycle::RefreshableSealed for T {}

  pub trait RefreshablePhantom: sealed_lifecycle::RefreshableSealed + Send + Sync {
      // Optional object-safe methods that project from Refreshable, e.g.:
      // fn kind(&self) -> &'static str;
      // fn refresh_policy(&self) -> RefreshPolicy;
  }

  impl<T: Refreshable> RefreshablePhantom for T {}
  ```
- `Box<dyn RefreshablePhantom>` is well-formed. Engine can store heterogeneous Refreshable credentials in `Vec<Box<dyn RefreshablePhantom>>` for proactive refresh iteration.
- Same pattern applies to the other 4 lifecycle sub-traits.

Spike iter-3 reproducer: `spike/credential-proto-builtin/src/lib.rs` §6-§8 (phantom portfolio for both service-capability + lifecycle axes). Integration test `question_c_dyn_refreshable_needs_phantom` demonstrates `Box<dyn RefreshablePhantom>` construction and the compile-fail case (`Box<dyn Refreshable>` rejected).

**Implication for macro emission contract (§4):** `#[plugin_credential]` macro must emit lifecycle phantom blankets per opt-in (`#[credential(refreshable, revocable)]` → emit `RefreshablePhantom` + `RevocablePhantom` blanket impls). Non-opt-in lifecycle phantom shims do not compile (no blanket → `impl RefreshablePhantom for X` not derivable), which is the desired compile-gate for capability-const downgrade (§15.4 decision — no legacy bool fallback).

### §3. Sealed module placement convention

#### §3 Amendment (2026-04-24-B, post spike iter-2 validation at commit `1c107144`)

Original §3 text prescribed a single `pub trait Sealed` per crate. That form compiles **only if** the crate has exactly one capability phantom, or if the capability traits are mutually disjoint by supertrait chain (rare in practice). For any crate declaring two or more capability phantoms whose "real" traits share a common supertrait (e.g. `BitbucketBearer: BitbucketCredential` + `BitbucketBasic: BitbucketCredential`), Rust's coherence check rejects the blanket `impl<T: CapX> Sealed for T` + `impl<T: CapY> Sealed for T` pair as overlapping — even when no concrete type satisfies both bounds. This is because Rust's coherence checker does not reason over trait-bound disjointness; two blanket impls on the same trait with different parameter bounds are declared overlapping whenever their bounds could theoretically intersect.

This is a **canonical-form correction after spike validation**, not stylistic preference — the original shape does not compile in any realistic multi-capability Nebula crate.

Per-capability inner Sealed traits are the canonical form:

```rust
// At src/lib.rs (crate root), once per crate:
mod sealed_caps {
    pub trait BearerSealed {}
    pub trait BasicSealed {}
    pub trait TlsIdentitySealed {}
    pub trait SigningSealed {}
    // … one inner trait per capability exposed by this crate.
}
```

Each phantom supertrait references its own `sealed_caps::XSealed`. Blanket impls now target different Sealed traits (`impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}` vs `impl<T: BitbucketBasic> sealed_caps::BasicSealed for T {}`) — no coherence overlap.

#### Module privacy structure (unchanged from original §3 rationale)

The outer `mod sealed_caps` has no `pub` prefix (crate-private module), but each inner `trait XSealed` IS `pub` within that module. This distinction is load-bearing:

- `mod sealed_caps` being crate-private means `crate_x::sealed_caps::*` is not reachable from outside — external crates cannot import the path.
- `pub trait XSealed` inside `mod sealed_caps` is visible-within-the-crate; the `pub` phantom trait (`pub trait BitbucketBearerPhantom: sealed_caps::BearerSealed + ...`) references a `pub`-within-scope supertrait, so `private_in_public` lint does not fire.

Alternative forms tried and rejected:

- `pub(crate) trait XSealed` within a `pub` module. The `pub` phantom trait then has a `pub(crate)` supertrait — triggers the `private_in_public` lint (warning in most editions, **hard error in Rust 2024**). Not viable.
- `pub trait XSealed` at crate root (no surrounding `mod`). External crates can then import `crate_x::XSealed` and impl it for their own types — defeats the sealing intent entirely.

The canonical `mod sealed_caps { pub trait XSealed {} … }` per-capability form is the **only supported shape** per this ADR (as amended). Tech Spec documents the convention; the `#[capability]` macro assumes this form exists and targets the capability-specific inner Sealed trait when emitting its blanket.

**Plugin authors** declaring their own capability traits follow the same convention: declare a local `mod sealed_caps` at the plugin's crate root with one inner Sealed trait per capability the plugin exposes; the plugin's phantom chain seals against the plugin's own `crate::sealed_caps::XSealed`. No cross-crate sealed sharing. Each crate protects only its own phantom traits.

### §4. Macro emission contract (`#[capability]`)

The `#[capability]` macro (post-spike, Tech-Spec-material) expands a single capability trait declaration into the full shape. This hides the two-trait verbosity from everyday use.

Input (user writes):

```rust
#[capability(scheme_bound = AcceptsBearer)]
pub trait BitbucketBearer: BitbucketCredential {}
```

Output (macro emits):

1. The "real" trait as written (with supertrait chain).
2. Blanket `impl<T: BitbucketCredential> BitbucketBearer for T where T::Scheme: AcceptsBearer {}`.
3. Blanket `impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}` — assumes `crate::sealed_caps::BearerSealed` already exists (see §4.1). The capability-specific inner Sealed trait name is specified via macro arg (`sealed = BearerSealed` below) or derived from the capability trait name.
4. The phantom trait `<vis> trait BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync {}`, where `<vis>` is the **visibility of the capability trait** (visibility-symmetry, per §1 visibility-symmetry note 2026-04-26). `'static` dropped per §5 amendment.
5. Blanket `impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}`.

#### §4.1 The macro does NOT emit the `sealed` module

Proc-macros in stable Rust cannot share state across invocations. An "emit `mod sealed` once per crate, skip thereafter" pattern is **not implementable** without external mechanisms (e.g. `inventory`, which this redesign rejects as cross-crate unreliable — see Strategy §2.1 discussion). Macros expand at the call site, which may be inside a nested module; they cannot synthesise a single shared crate-root module across multiple invocations.

Therefore the crate author declares the sealed module **manually, once, at crate root**, with one inner Sealed trait per capability the crate exposes:

```rust
// src/lib.rs
mod sealed_caps {
    pub trait BearerSealed {}
    pub trait BasicSealed {}
    pub trait TlsIdentitySealed {}
    // … add one per capability declared in this crate.
}

// Later in the same crate:
#[capability(scheme_bound = AcceptsBearer, sealed = BearerSealed)]
pub trait BitbucketBearer: BitbucketCredential {}

#[capability(scheme_bound = AcceptsBasic, sealed = BasicSealed)]
pub trait BitbucketBasic: BitbucketCredential {}
// … etc.
```

The `sealed = XSealed` macro argument tells `#[capability]` which inner Sealed trait to target when emitting the blanket. Plugin authors maintain their own `mod sealed_caps` with plugin-specific capability Sealed traits.

If the module or the target inner Sealed trait is absent when a `#[capability]` macro expansion runs, the emitted `impl sealed_caps::XSealed for T` fails to compile with `E0433` (`unresolved import crate::sealed_caps` or `unresolved import crate::sealed_caps::XSealed`). The error diagnostic points at the generated impl line; Tech Spec documents the crate-root `mod sealed_caps { ... }` onboarding step (one trait declaration per exposed capability) that resolves it.

#### §4.2 Alternatives considered for the idempotency problem

- **Uniquely-named per-capability sealed** (e.g. `BitbucketBearerSealed` as a separate trait per capability). Eliminates the manual `mod sealed` declaration but doubles the trait count per capability and pollutes the crate namespace. Trade-off rejected — one-line manual module is cheaper than N extra named sealed traits.
- **Macro-emitted `mod sealed` at call site**. Unimplementable — the macro expands at the call site (which may be inside a nested module), not at crate root, and cannot share a single crate-wide module across invocations.

#### §4.3 Action-side translation

Action-side macro (`#[action]`) accepts `CredentialRef<dyn BitbucketBearer>` in user-facing syntax and rewrites it to `CredentialRef<dyn BitbucketBearerPhantom>` in generated code — or rejects the non-phantom form with a guidance diagnostic. Decision between rewrite-silently vs reject-with-guidance is Tech-Spec-material.

### §5. Minimum bounds verification — DECIDED post iter-2

**Post iter-2 verification outcome** (spike commit `1c107144`):

- **`'static` DROPPED.** Empirically verified — all 11 integration tests pass and all 7 compile-fail probes still fail without the `'static` bound. Reason: Rust 2021+ default-object-lifetime rules make `dyn Phantom` in struct-field positions implicitly `+ 'static`, so the explicit bound is redundant.
- **`Send + Sync` KEPT.** Technically droppable (types in practice satisfy both via `PhantomData` auto-impls on `CredentialRef<C: ?Sized>`), but kept as a **forward-compat stability promise** for consumers using `&dyn Phantom` / `Box<dyn Phantom>` outside `CredentialRef` (registration transients, channel-passed handles). Removing these bounds later would be a breaking change; keeping them is cheaper than breaking.

Final canonical phantom bound: `pub trait XPhantom: sealed_caps::XSealed + Send + Sync {}`.

This verification outcome is the "bounds-verification addendum" originally anticipated in the iter-2-pending version of this section — recorded in place, not as a separate ADR.

## Consequences

### Positive

1. `dyn CapabilityPhantom` is well-formed as a Rust type — `E0191` does not fire at the point of use.
2. Strategy §3.3 semantic guarantee preserved end-to-end — blanket impl chain compile-rejects wrong-capability types; verified by spike iter-1 compile-fail test.
3. Per-crate sealed boundary prevents cross-crate forging of capability membership (see §3). This closes a coherence hole that would exist in a one-trait-with-where-clause alternative (see Alternative A).
4. Plugin extensibility preserved. Plugin crates declare their own sealed + phantom — no conflict with Strategy §2.1 sealed policy (which applies to the `Credential` trait itself at a different scope).
5. `#[capability]` macro (§4) hides the two-trait verbosity. User-facing code reads `#[capability] pub trait FooBearer: FooCredential` — the phantom is a compile-time artifact.
6. Runtime cost zero. Phantom trait has no methods, no vtable entries added. Dispatch continues through the `AnyCredential` + `TypeId` path per Strategy §3.2 type-erased runtime.

### Negative

1. Every Pattern 2 / Pattern 3 capability trait requires a paired Phantom. Without `#[capability]` macro, this is non-trivial boilerplate (~10 lines per capability). Macro is required for production ergonomics.
2. Action-side `#[action]` macro must translate `CredentialRef<dyn FooBearer>` → `CredentialRef<dyn FooBearerPhantom>` (or reject). New mechanism added to `#[action]` scope.
3. Two closely-named traits (`BitbucketBearer` and `BitbucketBearerPhantom`) — potential naming confusion for readers of generated code. Macro documentation must clarify. In user-written code with macro, the `Phantom` name never appears.
4. Compile-fail diagnostic chains through two levels (`Scheme not satisfied → Trait not satisfied → Phantom not satisfied`). Slightly more verbose for users to parse. Spike iter-1 confirmed the chain stays readable.

### Neutral

- Public API surface of capability traits is unchanged from Strategy §3.3 intent — the phantom is an implementation mechanism for dyn positions, not a new concept in the Credential mental model.
- `nebula-credential-builtin` crate gains one `mod sealed` block at crate root and one `*Phantom` trait per capability. Both macro-emitted.

## Alternatives considered

### Alternative A — One-trait form with where-clause

```rust
pub trait BitbucketBearer: Send + Sync + 'static {}
impl<T> BitbucketBearer for T
where T: BitbucketCredential, T::Scheme: AcceptsBearer {}
```

Single trait; supertrait relaxed to auto-traits only; constraint moved to the blanket's where-clause. `dyn BitbucketBearer` well-formed.

**Rejected.** Coherence hole: under Rust orphan rules (RFC 2451), plugin crates can `impl BitbucketBearer for LocalType` — because `LocalType` is local to the plugin crate, the impl is permitted even though `BitbucketBearer` is foreign. As long as `LocalType` is not already covered by the blanket (not `BitbucketCredential + AcceptsBearer`), no overlap, and the manual impl succeeds. External plugin could falsely claim membership in a builtin capability.

Sealing the one-trait form (adding `sealed::Sealed` supertrait to `BitbucketBearer`) closes the hole — but at that point the boilerplate is the same as two-trait, and the conceptual separation is worse (one trait serves as both constraint and dyn marker, sealed supertrait confuses "this trait is sealed from external impls" with "this trait is constraint-based"). Two-trait wins on clarity and macro ergonomics.

### Alternative B — Tag struct (not trait)

```rust
pub struct BitbucketBearerTag;
pub bb: CredentialRef<BitbucketBearerTag>,
```

**Rejected.** Loses the "bound IS the trait" semantic reading. `CredentialRef<C: ?Sized>` becomes misleading — the tag is a concrete type, not a trait constraint. Bound enforcement would have to shift into `CredentialRegistry` runtime logic, degrading the compile-time guarantee that Strategy §3.3 was chosen to preserve.

### Alternative C — Specify associated types in dyn

```rust
pub bb: CredentialRef<dyn Credential<Input = …, State = …, Scheme = …, Pending = …>>,
```

**Rejected.** Defeats the purpose of Pattern 2. The whole point of `dyn BitbucketBearerPhantom` is to bind at the service/capability level without committing to a concrete `Credential` impl. Specifying all four assoc types forces commitment to a single concrete credential type, collapsing Pattern 2 back to Pattern 1 and eliminating service-grouping compile-time checking.

## Implementation notes

### Changes to Strategy Document

Inline amendments in [`docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md`](../superpowers/specs/2026-04-24-credential-redesign-strategy.md):

- **§3.2** — "On `dyn` semantics — what the spike must validate" paragraph replaced; distinguishes type well-formedness from runtime dispatch; points at this ADR for phantom-shim pattern resolving the Pattern 2 / Pattern 3 well-formedness problem.
- **§3.3** — pseudo-Rust example replaced with two-trait form above; closing paragraph updated to reference `dyn BitbucketBearerPhantom` (not `dyn BitbucketBearer`) and cite this ADR.
- **§0** — freeze policy addendum clarifying that §3.2/§3.3 inline amendments via ADR carry forward pointer; Strategy remains the primary reader entry point with ADR references embedded.

### Register entry

Add row to [`docs/tracking/credential-concerns-register.md`](../tracking/credential-concerns-register.md) Type system section:

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| `arch-phantom-shim-convention` | Two-trait phantom-shim pattern with per-crate sealed placement for capability traits in `dyn` positions | tech-spec-material | decided | ADR-0035; spike iter-1 validated first instance (commit `acfec719`) |

### Spike iteration-2 obligations introduced by this ADR

- Add `mod sealed` + `sealed::Sealed` blanket + `Phantom` sealed-supertrait to the spike crate's capability sub-traits (iter-1 has the Phantom without sealing).
- Empirical minimum bounds verification per §5 above.
- `#[action]` macro hand-expansion (Q2) must demonstrate the translation from user-facing `dyn FooBearer` to generated `dyn FooBearerPhantom`.

## References

- [Strategy Document](../superpowers/specs/2026-04-24-credential-redesign-strategy.md) — §3.2 (dyn semantics), §3.3 (pattern), §3.7 (fallbacks not triggered by this ADR).
- [Credential Concerns Register](../tracking/credential-concerns-register.md) — row `arch-phantom-shim-convention`.
- Spike iteration-1 artefacts on worktree branch `worktree-agent-a23a1d2c`, commit `acfec719`:
  - `spike/NOTES.md` §2 (iterations 1a/1b/1c with rationale for Option C phantom), §3 (reproducibility commands).
  - `spike/final_trait_shape_v1.rs` (distilled snapshot).
  - `spike/credential-proto-builtin/src/lib.rs` (live implementation — note: does not yet apply sealing; iter-2 obligation).
  - `spike/credential-proto-builtin/examples/compile_fail_app_password_via_phantom.rs` (diagnostic chain proof).
- Rust Reference — [trait objects and dyn compatibility](https://doc.rust-lang.org/reference/types/trait-object.html).
- [Rust RFC 2451 — re-rebalancing-coherence](https://rust-lang.github.io/rfcs/2451-re-rebalancing-coherence.html) (orphan rule source).

---

*Proposed by: vanyastaff + Claude (credential redesign workstream), 2026-04-24. Blocks spike iteration-2 decisions on H2/H3 hand-expansion shape and sealed-placement convention.*
