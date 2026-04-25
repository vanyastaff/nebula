---
id: 0037
title: action-macro-emission
status: accepted
date: 2026-04-24
accepted_date: 2026-04-25
amended_in_place: 2026-04-25
supersedes: []
superseded_by: []
tags: [action, macro, emission, hrtb, slot-binding, nebula-action, cascade-action-redesign]
related:
  - docs/superpowers/specs/2026-04-24-action-redesign-strategy.md
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
  - docs/adr/0036-action-trait-shape.md
linear: []
---

# 0037. Action `#[action]` macro emission contract

## Status

**Accepted 2026-04-25 (amended-in-place 2026-04-25)** — drafted 2026-04-24 as the second of the 3-ADR set for the nebula-action redesign cascade ([Strategy §6.2](../superpowers/specs/2026-04-24-action-redesign-strategy.md#62-adr-drafting-roadmap)). Drafted after ADR-0036 (trait shape) so the emission contract is grounded in the trait-shape decision. Status moved from `proposed` → `accepted` 2026-04-25 after Phase 6 Tech Spec FROZEN CP4 ratification (tech-lead 11c freeze ratification). Retains amendment-in-place qualifier from same date per Tech Spec CP4 §15.5 enactment.

**Amended-in-place 2026-04-25** per [Tech Spec CP4 §15.5](../superpowers/specs/2026-04-24-nebula-action-tech-spec.md) to fold `capability` into the `SlotType` enum, aligning with [credential Tech Spec §9.4 line 2452](../superpowers/specs/2026-04-24-credential-tech-spec.md) authoritative three-variant matching pipeline (`Concrete { type_id }`, `ServiceCapability { capability, service }`, `CapabilityOnly { capability }`). Per [ADR-0035 amended-in-place precedent](./0035-phantom-shim-capability-pattern.md) (status block records 2026-04-24-B and 2026-04-24-C amendments — same canonical-form-correction discipline applies here, not a paradigm shift). Pre-amendment §1 had separate `capability` field; post-amendment shape locates capability inside `SlotType` variants. Field renamed `key` → `field_name` (`&'static str`) for clarity (this is the *Rust struct field name*, not a credential `SlotKey`). §3 (qualified-syntax probe), §4 (test harness), §5 (emission perf bound) unaffected — `SlotBinding` shape is not load-bearing for those sections.

## Context

ADR-0036 locks the trait shape: `#[action]` attribute macro with narrow zone rewriting + dual enforcement layer. This ADR locks the **emission contract** — what tokens the macro produces, the HRTB function-pointer shape for `SlotBinding::resolve_fn`, and two emission-time obligations the spike surfaced.

Three artefacts ground this ADR:

- **Credential Tech Spec §3.4 line 869** — the load-bearing HRTB declaration: `for<'ctx> fn(&'ctx CredentialContext<'ctx>, &'ctx SlotKey) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>`. The same shape is used by `RefreshDispatcher::refresh_fn` (Tech Spec §7.1). `SlotBinding` includes capability marker + scheme expectation per §3.4 step 2.
- **Spike Iter-1 §1.1 module layout** ([NOTES](../superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)) — concrete reference module split: `slot.rs`, `scheme_guard.rs`, `action.rs`, `resolve.rs`. Hand-expansion `src/hand_expanded.rs` lines 84-155 = 71 LOC of macro-emitted region for one Stateless+Bearer action with one Bearer slot. `bin/iter2_compose.rs` validates the same emission shape across Stateful+OAuth2 and ResourceAction+Basic.
- **Spike findings #1 + #3** — auto-deref Clone shadow on `SchemeGuard<'a, C>` (silent probe-pass risk via `Scheme: Clone`); dual enforcement layer for declaration-zone discipline.

Two design questions converge at the emission contract:

1. **What is the `resolve_fn` token shape?** HRTB function pointer (compiles to one fn-pointer-sized vtable entry per slot, monomorphized at call site) vs `Box<dyn Fn>` (heap-allocated trait object, lifetime gymnastics for the `'ctx` parameter). The spike validates HRTB; the Tech Spec mandates HRTB.
2. **What constitutes regression-test discipline for emission?** Without `trybuild` + `macrotest`, three independent agents hit the same bugs (Strategy §1(b) CR2/CR8/CR9/CR11). Macro output drift is silent. Six probes exist as spike artefacts; production harness ports these.

Open question 5 from the spike (NOTES §4 question 5 — `ResolvedSlot` enum vs `SchemeGuard<'a, C>` direct return) is **not in this ADR's scope** — it's a credential-side resolve-fn return-type detail per Tech Spec §7. This ADR specifies the `resolve_fn` HRTB *shape* the action macro emits, not the wrap point of `SchemeGuard`.

## Decision

The `#[action]` macro emits the following, per credential slot declared in the `credentials(slot: Type)` zone:

### §1. `ActionSlots::credential_slots()` returns `&'static [SlotBinding]`

*(Shape amended-in-place 2026-04-25 per Tech Spec CP4 §15.5 — `capability` folded into `SlotType` enum; `key` renamed to `field_name`; receiver `&self` per credential Tech Spec §3.4 line 851 cardinality.)*

```rust
impl ActionSlots for SlackBearerAction {
    fn credential_slots(&self) -> &'static [SlotBinding] {
        const SLOTS: &[SlotBinding] = &[
            SlotBinding {
                field_name: "slack",
                slot_type: SlotType::CapabilityOnly { capability: Capability::Bearer },
                resolve_fn: resolve_as_bearer::<SlackToken>,
            },
        ];
        SLOTS
    }
}
```

`SlotBinding` shape — three fields after amendment (capability lives inside `SlotType`):

```rust
#[derive(Clone, Copy, Debug)]
pub struct SlotBinding {
    pub field_name: &'static str,
    pub slot_type: SlotType,
    pub resolve_fn: ResolveFn,
}
```

`SlotType` is the three-variant matching pipeline mirroring [credential Tech Spec §9.4 line 2452](../superpowers/specs/2026-04-24-credential-tech-spec.md) verbatim — engine-side `iter_compatible` (credential Tech Spec §9.4 line 2456-2470) dispatches on this enum:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SlotType {
    /// Pattern 1 — concrete `CredentialRef<C>` field; engine matches by type-id.
    Concrete { type_id: TypeId },
    /// Pattern 2 — `CredentialRef<dyn ServicePhantom>` with both service identity AND
    /// capability projection. Engine matches by `service_key` + the registry-computed
    /// capability set per credential Tech Spec §15.8 (`RegistryEntry::capabilities`,
    /// CP5 supersession of §9.4 — pre-CP5 plugin-metadata `capabilities_enabled` is
    /// REMOVED). Same matching axes; capability authority shifts plugin-metadata →
    /// type-system at `CredentialRegistry::register<C>` time.
    ServiceCapability { capability: Capability, service: ServiceKey },
    /// Pattern 3 — `CredentialRef<dyn AnyBearerPhantom>`, capability-only projection.
    CapabilityOnly { capability: Capability },
}
```

The `resolve_fn` field has type `for<'ctx> fn(&'ctx CredentialContext<'ctx>, &'ctx SlotKey) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>` per [credential Tech Spec §3.4 line 869](../superpowers/specs/2026-04-24-credential-tech-spec.md). `SlotBinding` is `Copy + 'static` (verified by spike `slot.rs` static assert) so `&'static [SlotBinding]` storage is well-formed. Capability dispatch (Pattern 2 ServiceCapability vs Pattern 3 CapabilityOnly) is encoded in the `slot_type` discriminant; the engine's runtime registry matches per credential Tech Spec §9.4 three-variant pipeline.

### §2. Dual enforcement layer for declaration-zone discipline (per spike finding #3)

Both enforcement mechanisms ship in the production macro per [ADR-0036 §3](./0036-action-trait-shape.md):

- **Type-system layer (always on, structural).** Bare `CredentialRef<C>` field outside `credentials(...)` zone has no emitted `ActionSlots` impl → struct cannot reach `Action` blanket marker → `error[E0277]: trait bound X: Action not satisfied`. Spike Probe 3 confirms ([NOTES §1.4](../superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)).
- **Proc-macro layer (DX diagnostic).** When the macro parses an `#[action]` invocation and detects a `CredentialRef<_>` field outside the declared `credentials(...)` zone, it emits `compile_error!("did you forget to declare this credential in `credentials(slot: Type)`?")` with span pointing at the offending field. Cleaner than letting the user hit `E0277` with no actionable hint.

Both layers cover the same violation at different stages. The type-system layer is the structural ground; the proc-macro layer is DX cleanup.

### §3. Auto-deref Clone shadow probe — qualified syntax mandated (per spike finding #1)

Naive `let g2 = guard.clone()` does NOT compile-fail for `SchemeGuard<'_, C>` because `SchemeGuard: Deref<Target = Scheme>` and canonical schemes (`BearerScheme`, `BasicScheme`, `OAuth2Scheme`) all derive `Clone` for ergonomic reasons. Auto-deref resolves `.clone()` against `Scheme`, producing a Scheme clone (a leak — Scheme contains `SecretString`, also `Clone`). The compile-fail probe **silently green-passes** while the `SchemeGuard: !Clone` invariant is violated by user code.

The macro emits a regression-test probe using **qualified syntax**:

```rust
// Emitted into a `compile_fail` test harness adjacent to the action:
let _ = <SchemeGuard<'_, SlackToken> as Clone>::clone(&guard);  // E0277 fires here
```

The qualified form bypasses auto-deref. This is a **Tech Spec §16.1.1 amendment candidate** for the credential crate (per [spike NOTES §3 finding #1](../superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)) — the credential `compile_fail_scheme_guard_clone.rs` probe must use the qualified form too. This ADR codifies the qualified-form rule for any `SchemeGuard` no-clone probe emitted by `#[action]`.

### §4. Macro test harness ships with implementation

Production `crates/action/macros/Cargo.toml` gains `[dev-dependencies]` block (currently absent per Strategy §1(b)) with `trybuild` + `macrotest`. The 6 probes from spike commit `c8aef6a0` `tests/compile_fail/probe_{1..6}_*.rs` port forward to `crates/action/macros/tests/`:

| Probe | Asserts | Expected diagnostic |
|---|---|---|
| 1 | `ResourceAction` impl missing `Resource` assoc type | `E0046` |
| 2 | `TriggerAction` impl missing `Source` assoc type | `E0046` |
| 3 | Bare `CredentialRef` outside `credentials(...)` zone | `E0277` (type layer) + `compile_error!` (proc-macro layer) |
| 4 | `SchemeGuard::clone()` via qualified syntax | `E0277` |
| 5 | `SchemeGuard` retention beyond `'a` lifetime | `E0597` |
| 6 | Wrong-Scheme `resolve_as_bearer::<BasicCred>` | `E0277` (subsumes `E0271` per Rust 1.95 diagnostic rendering) |

Strategy §4.3.1 absorbs `*Handler` HRTB modernization scope as a *separate* concern — that's about runtime trait surface, not macro emission. **This ADR is macro-emission only.** HRTB modernization for `*Handler` traits is locked at Tech Spec §7 design time per Strategy §4.3.1.

### §5. Emission perf bound

Per spike §2.5 ([NOTES](../superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)): emission for one Bearer slot is 71 LOC of macro-emitted region. Naive ratio vs old `#[derive(Action)]` is 3.2x; **adjusted ratio (net of user-code absorbed) is 1.6-1.8x**. Within Strategy §5.2.4 perf bound.

The bound is **per-slot, not per-action** (per spike §2.5 caveat): N credential slots emit ~10 additional LOC per slot beyond the first. Tech Spec §7 commits to "per-slot emission cost" as the gate, not "per-action emission cost."

## Consequences

### Positive

1. HRTB fn-pointer shape is monomorphized — one fn-pointer-sized field per slot, no heap allocation, lifetime parameters compose cleanly with action body's `&'a` borrow chain (spike Iter-2 confirmed across Stateless / Stateful / Resource).
2. Dual enforcement layer makes declaration-zone discipline structural at the type system AND DX-friendly at the macro layer. Bypass of one is caught by the other.
3. Auto-deref Clone shadow finding becomes a documented regression-test invariant rather than a latent silent-pass risk. The qualified-syntax probe catches the actual `!Clone` violation; the credential crate's adjacent probe gains the same protection (Tech Spec §16.1.1 amendment candidate).
4. Macro test harness closes Strategy §1(b) — three independent agents hitting the same `parameters = Type` emission bug becomes structurally impossible because regression coverage exists.
5. Emission shape composes with [ADR-0035 §4.3](./0035-phantom-shim-capability-pattern.md) action-side rewrite — the macro emits `dyn ServiceCapabilityPhantom` translation per ADR-0035 contract; phantom-shim end-to-end with credential CP6 + this emission contract.
6. Per-slot emission perf bound (1.6-1.8x adjusted) is structurally specified and verifiable via `cargo expand` measurement at any later point.

### Negative

1. **HRTB shape is rigid.** No `for<'ctx> async fn(...)` syntax exists on Rust 1.95 (per [spike NOTES §4 open question 1](../superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)). `BoxFuture<'ctx, ...>` return is load-bearing, not a wart. Future Rust release adding `async fn` pointers becomes a clean migration; until then, this is the only shape.
2. **Macro emits ~2x current `#[derive(Action)]` token count** (adjusted ratio 1.6-1.8x). Compile-time impact at `cargo expand` is measurable but within Strategy §5.2.4 perf bound. Larger crates with many actions will see incremental compile-time growth.
3. **Macro must be aware of `SchemeGuard` qualified-syntax** when emitting probes — the macro is no longer purely token-shape mechanical; it carries a Tech-Spec-specified probe form. Documentation cost in the macro source.
4. **Two enforcement layers can produce two diagnostics for the same violation** (the proc-macro `compile_error!` fires first; if user bypasses by hand-implementing `ActionSlots`, type-system `E0277` fires). Slightly more verbose for users to parse; spike confirmed both diagnostics stay readable.

### Neutral

- Engine-side dispatch is unchanged — the engine consumes `&'static [SlotBinding]` at registry time and dispatches via `resolve_fn` HRTB at execution. No engine-layer surgery required by this ADR; engine wiring design is Tech Spec §7 / §9.
- Codemod transforms (Strategy §4.3.3) cover the `#[derive(Action)]` → `#[action]` shape migration; per-slot emission cost growth is a one-time migration cost, not ongoing per-edit cost.

## Alternatives considered

### Alternative A — Emit `Box<dyn Fn>` instead of HRTB fn pointer

```rust
resolve_fn: Box<dyn for<'ctx> Fn(&'ctx CredentialContext<'ctx>, &'ctx SlotKey)
                                  -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>> + Send + Sync>,
```

**Rejected.** Performance and lifetime gymnastics. Heap allocation per slot at registration time vs zero-allocation HRTB fn pointer. The `Box<dyn Fn>` shape requires a specific lifetime erasure dance that the spike Iter-2 found awkward to monomorphize per-slot. HRTB fn pointer is the canonical Rust shape for "function-shaped value with quantified lifetime" — this is what the credential Tech Spec specifies.

### Alternative B — Skip proc-macro `compile_error!` enforcement; rely on type-system layer alone

`#[action]` does not parse-check `CredentialRef<_>` placement; bare-CredentialRef-outside-zone hits `E0277` from the missing `Action` blanket impl.

**Rejected.** DX regression. The `E0277` diagnostic is structurally correct but offers zero hint about the actual cause (forgotten `credentials(slot: Type)` declaration). Plugin authors waste cycles reverse-engineering the cause. Two-layer enforcement is cheap (proc-macro parse pass already runs); skipping the helpful diagnostic optimizes for purity over usability.

### Alternative C — Macro emits a separate `*Slots` struct alongside the action struct

Instead of `impl ActionSlots for SlackBearerAction`, macro emits `struct SlackBearerActionSlots { ... }` + `impl Bound for SlackBearerActionSlots`. Action struct refers to the slots struct by associated const.

**Rejected.** Doubles the type count per action (one `*Action` + one `*Slots` per declaration). Pollutes namespace and rustdoc. The trait-impl form keeps slot metadata associated with the action via the trait system, which is the natural Rust shape.

## References

- [Strategy Document](../superpowers/specs/2026-04-24-action-redesign-strategy.md) — §4.3.1 macro modernization scope; §5.2.4 perf bound; §6.2 ADR roadmap; §1(b) emission bug class.
- [Spike NOTES](../superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md) — §1.1 module layout; §1.4 Probe 3 result; §1.6 hand-expansion; §2.5 emission perf; §3 finding #1 (auto-deref Clone shadow); §3 finding #3 (dual enforcement layer); commit `c8aef6a0`.
- [Credential Tech Spec](../superpowers/specs/2026-04-24-credential-tech-spec.md) — §3.4 line 869 HRTB shape; §7.1 `RefreshDispatcher::refresh_fn` parallel pattern; §15.7 `SchemeGuard<'a, C>` lifecycle; §16.1.1 probe table (auto-deref Clone shadow amendment candidate).
- [ADR-0035 phantom-shim capability pattern](./0035-phantom-shim-capability-pattern.md) — §4.3 action-side rewrite obligation; ADR-0035 §4.1 macro-cannot-emit-shared-module rationale (informs §4 test harness location: macro-adjacent, not pre-emitted).
- [ADR-0036 action trait shape](./0036-action-trait-shape.md) — companion ADR locking trait shape; this ADR's emission contract assumes ADR-0036's narrow zone rewriting.

---

*Proposed by: architect (nebula-action redesign cascade Phase 5), 2026-04-24. Composed with ADR-0036 (trait shape) and ADR-0038 (ControlAction seal + canon revision).*
