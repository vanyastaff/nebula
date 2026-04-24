# ADR-0035: Phantom-shim capability pattern

## Status

**Proposed.** 2026-04-24.

Amends portions of [Strategy §3.2 / §3.3](../superpowers/specs/2026-04-24-credential-redesign-strategy.md) (Checkpoint 1, frozen at commit `d5045774`):

- §3.2 "On `dyn` semantics" paragraph — superseded (dyn-safety framing conflated well-formedness and runtime dispatch).
- §3.3 pseudo-Rust example — superseded (supertrait-chain form not well-formed as `dyn`).
- §3.3 closing paragraph — updated reference to this ADR.

Preserves without modification: Strategy §2 Foundational decisions, §3.1 (Credential shape), §3.4 (H1/H2/H3 hypotheses), §3.5 (macro cross-check), §3.6 (trait-heaviness discipline), §3.7 (fallbacks).

Post-validation: spike iteration 1 (commit `acfec719` on worktree branch `worktree-agent-a23a1d2c`) confirms the pattern prescribed below compiles + enforces Strategy §3.3 semantics on the Bitbucket triad, with clean `E0277` diagnostic on the negative case.

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
// Module-private sealing — prevents cross-crate manual phantom impls.
mod sealed {
    pub trait Sealed {}
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

// Sealed blanket — only types satisfying BitbucketBearer become sealed.
// External crates cannot impl sealed::Sealed for their own types (sealed
// is crate-private), therefore external crates cannot manually impl the
// phantom trait — they have to go through the blanket, which requires
// BitbucketBearer membership.
impl<T: BitbucketBearer> sealed::Sealed for T {}

// "Phantom" capability trait — dyn-safe marker for dyn positions.
// Supertrait is sealed::Sealed + auto-traits. NO Credential supertrait
// → no unspecified-assoc-type closure → `dyn BitbucketBearerPhantom` is
// well-formed as a type.
pub trait BitbucketBearerPhantom: sealed::Sealed + Send + Sync + 'static {}

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

### §2. Scope

The phantom-shim pattern applies **only** to:

- **Pattern 2** (service-bound capability): `CredentialRef<dyn ServiceXBearerPhantom>` positions.
- **Pattern 3** (capability-only utility): `CredentialRef<dyn AcceptsBearerPhantom>` positions.

**Pattern 1** (concrete per-credential-type, e.g. `CredentialRef<SlackOAuth2Credential>`) does NOT use phantom — the type parameter is a concrete `Credential`, no `dyn` projection, no well-formedness gap. Pattern 1 ergonomics are unchanged by this ADR.

### §3. Sealed module placement convention

Each capability-trait-defining crate declares a **locally-scoped** `sealed` module. Two equivalent forms:

- `mod sealed { pub trait Sealed {} }` — `mod` is non-public (no `pub` prefix).
- `pub(crate) trait Sealed` within any pub module.

Either form achieves **crate-private** `Sealed`. The Sealed trait is NOT re-exported from the defining crate. External crates cannot reference `crate_x::sealed::Sealed` at import time and cannot impl it for their own types — no cross-crate forging of capability membership.

**Plugin authors** declaring their own capability traits (`CustomServiceBearerPhantom`) follow the same convention: declare a local `mod sealed` in the plugin crate; the plugin's phantom chain seals against the plugin's own `sealed::Sealed`. No cross-crate Sealed sharing. Each crate protects only its own phantom traits. This decouples plugin's phantom-correctness from the builtin crate's.

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
3. A single `mod sealed { pub trait Sealed {} }` at crate root — idempotent across `#[capability]` invocations (the macro emits this block only once per crate; subsequent invocations skip it).
4. Blanket `impl<T: BitbucketBearer> sealed::Sealed for T {}`.
5. The phantom trait `pub trait BitbucketBearerPhantom: sealed::Sealed + Send + Sync + 'static {}`.
6. Blanket `impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}`.

The macro ensures emitted phantom references `crate::sealed::Sealed` (same-crate path). Plugin crates get their own emission; cross-crate `sealed` sharing is neither requested nor permitted.

Action-side macro (`#[action]`) accepts `CredentialRef<dyn BitbucketBearer>` in user-facing syntax and rewrites it to `CredentialRef<dyn BitbucketBearerPhantom>` in generated code — or rejects the non-phantom form with a guidance diagnostic. Decision between rewrite-silently vs reject-with-guidance is Tech-Spec-material.

### §5. Minimum bounds verification — iter-2 obligation

Spike iteration-2 shall verify empirically whether the phantom trait's bounds `Send + Sync + 'static` are all required at every real use site. Candidate tightenings:

- If no `Send/Sync` requirement surfaces through `CredentialRef<C: ?Sized>` bounds or executor spawn boundaries, tighten to `sealed::Sealed + 'static`.
- If `Send + Sync` required at any real call site, bounds remain.

Decision pinned empirically in iter-2 `NOTES.md`. If it differs from the starting form above, this ADR gains a "bounds-verification addendum" (an amendment to the existing ADR, not a new ADR).

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
