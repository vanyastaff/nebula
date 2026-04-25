---
id: 0036
title: action-trait-shape
status: accepted
date: 2026-04-24
accepted_date: 2026-04-25
supersedes: []
superseded_by: []
tags: [action, macro, trait-shape, nebula-action, canon-3.5, cascade-action-redesign]
related:
  - docs/superpowers/specs/2026-04-24-action-redesign-strategy.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md
linear: []
---

# 0036. Action trait shape — `#[action]` attribute macro replacing `#[derive(Action)]`

## Status

**Accepted 2026-04-25** — drafted 2026-04-24 alongside ADR-0037 / ADR-0038 as the 3-ADR set for the nebula-action redesign cascade ([Strategy §6.2](../superpowers/specs/2026-04-24-action-redesign-strategy.md#62-adr-drafting-roadmap)). Status moved from `proposed` → `accepted` 2026-04-25 after Phase 6 Tech Spec FROZEN CP4 ratification (tech-lead 11c freeze ratification).

## Context

The current `nebula-action` integration surface uses `#[derive(Action)]` (`crates/action/macros/src/derive.rs`) which cannot perform field-type rewriting — derives observe fields by name and type but emit code adjacent to the struct, not over it. The credential CP6 spec ([credential Tech Spec §2.7 line 486-528](../superpowers/specs/2026-04-24-credential-tech-spec.md), [§3.4 line 807-939](../superpowers/specs/2026-04-24-credential-tech-spec.md)) specifies a typed credential surface — `CredentialRef<C>` field-level handles, dyn-position translation `CredentialRef<dyn ServiceCapability>` → `CredentialRef<dyn ServiceCapabilityPhantom>` per ADR-0035 — that requires field-zone rewriting, not just adjacent emission.

Phase 4 spike (commit `c8aef6a0`, [spike NOTES](../superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md) §1.4 + §3 finding #3) confirmed two structurally-distinct enforcement mechanisms for the "bare `CredentialRef<C>` outside `credentials(...)` zone" contract:

- **Option (a) — proc-macro `compile_error!`** at parse-time when a `CredentialRef<_>` field appears outside the `credentials(slot: Type)` zone. DX-friendly diagnostic ("did you forget `credentials(slot: Type)`?").
- **Option (b) — type-system enforcement.** Without `#[action]`, no `ActionSlots` impl is emitted; the struct cannot satisfy the `Action` blanket marker. Spike Probe 3 result: `error[E0277]: trait bound BareUserStruct: Action not satisfied`.

The spike validates option (b) is type-system-enforceable today; option (a) is layered DX. Both are compatible — they catch the same violation at different stages.

Three forces converge at this trait-shape decision:

- **Strategy §3.2 placement lock** — `CredentialRef<C>` / `SlotBinding` / `SchemeGuard<'a, C>` live in `nebula-credential`, not `nebula-action`. The action-side surface is field-shape rewriting + slot-binding emission, not vocabulary ownership.
- **`feedback_boundary_erosion`** — "convenience helper in the wrong crate compounds." Treating cross-crate placement as a boundary decision rules out moving credential vocabulary into action; the macro must work *across* the boundary.
- **`feedback_hard_breaking_changes`** — `#[derive(Action)]` removal is acceptable as a single-cut break. Plugin authors migrate via codemod; semver-checks are advisory-only during alpha.

A narrow rewriting contract is required: rewriting confined to attribute-tagged zones (`credentials(slot: Type)` / `resources(slot: Type)`), NOT arbitrary field rewriting (which would harm LSP/grep — fields visible in source must mean what they say).

## Decision

Adopt the `#[action]` **attribute macro** in `nebula-action`, replacing `#[derive(Action)]`. The macro has a **narrow declarative rewriting contract**:

1. **Rewriting scope.** Rewriting is confined to fields declared inside the `#[action(credentials(slot: Type), resources(slot: Type))]` attribute zones. Fields outside these zones are NOT rewritten — they pass through to the generated struct unchanged.
2. **Emission contract.** The macro emits, in addition to the rewritten struct:
   - `Action` impl (blanket-marker satisfaction; preserves current `Action` semantics)
   - `ActionSlots` impl with `credential_slots() -> &'static [SlotBinding]` and `resource_slots() -> &'static [ResourceBinding]`
   - `DeclaresDependencies` impl (replaces hand-written `impl DeclaresDependencies for X` referencing `CredentialRef` fields)
3. **Dual enforcement layer for declaration-zone discipline** (per spike finding #3):
   - **Type-system layer (always on, structural)** — bare `CredentialRef<C>` field outside `credentials(...)` zone fails the `Action` blanket marker because no `ActionSlots` impl is emitted. Probe 3 confirmed: `error[E0277]: trait bound BareUserStruct: Action not satisfied`.
   - **Proc-macro layer (DX, opt-in helpful diagnostic)** — when the macro detects a `CredentialRef<_>` field outside the declared `credentials(...)` zone, it emits a `compile_error!` with a fix-it message. Cleaner than letting the user hit `E0277` with no actionable hint.
4. **Pattern composition with ADR-0035.** The `#[action]` macro emits `dyn ServiceCapabilityPhantom` translation per ADR-0035 §4.3 action-side rewrite obligation. User-facing syntax `CredentialRef<dyn BitbucketBearer>` rewrites to generated code `CredentialRef<dyn BitbucketBearerPhantom>` (ADR-0035 §1 canonical form). The action macro IS the consumer-side completion of ADR-0035's two-trait phantom-shim contract.

The macro test harness — `trybuild` for compile-fail probes + `macrotest` for emission stability — ships with the implementation per Strategy §4.3.1. Six probes already exist as spike artefacts (commit `c8aef6a0` `tests/compile_fail/probe_{1..6}_*.rs`); production harness ports these forward.

The macro emission contract details (token shape, `ActionSlots::credential_slots()` HRTB shape, the auto-deref Clone shadow probe, etc.) are scoped to **ADR-0037** (this ADR locks trait-shape, ADR-0037 locks emission).

## Consequences

### Positive

1. Aligns nebula-action with the credential CP6 typed surface ([credential Tech Spec §2.7 / §3.4](../superpowers/specs/2026-04-24-credential-tech-spec.md)) — `CredentialRef<C>` field handles, `SlotBinding` registration, `SchemeGuard` RAII flow through one cohesive vocabulary.
2. Closes Strategy §1(a) credential paradigm mismatch — `time-to-first-successful credential-bearing action` target <5 minutes (vs current 32) becomes structurally achievable; `ctx.credential::<S>(key)` API surface materializes through macro-emitted slot bindings.
3. Composes with ADR-0035 phantom-shim: action macro is the consumer-side rewrite obligation ADR-0035 §4.3 leaves unbound. After this ADR + ADR-0037, ADR-0035's contract is end-to-end.
4. Narrow rewriting zone preserves LSP/grep semantics — fields outside `credentials(...)`/`resources(...)` are visible, name-meaningful, and IDE-navigable. Rewriting is opt-in via attribute zone, not pervasive struct-level surgery.
5. Macro test harness lands as a structural artefact — current `crates/action/macros/Cargo.toml` has no `[dev-dependencies]` (Strategy §1(b)). The 6-probe spike harness ports forward; emission-bug class (CR2 / CR8 / CR9 / CR11) becomes regression-covered.

### Negative

1. **Hard break on `#[derive(Action)]` for plugin authors.** The 7 reverse-deps (Strategy §6.1) must migrate; codemod ships in cascade per [scope decision §1.6](../superpowers/drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md). Acceptable per `feedback_hard_breaking_changes`.
2. **Two enforcement layers add some apparent redundancy** (type-system + proc-macro both catch bare `CredentialRef`). Justified per spike finding #3: type-system layer is structural truth; proc-macro layer is DX cleanup. Removing either weakens the contract — type-system-only loses helpful diagnostic; proc-macro-only loses the structural guarantee for users who bypass the macro (impossible in normal flow but a real regression-test invariant).
3. **Macro complexity grows** vs `#[derive(Action)]`. Token emission ~2x current (per spike §2.5; adjusted ratio 1.6-1.8x net of user-code absorbed). Within Strategy §5.2.4 perf bound. ADR-0037 specifies the emission shape.
4. **`#[action]` attribute zones become canonical syntax** users learn. Documentation cost — README examples + migration guide must teach the zone discipline. Codemod and migration guide ship in cascade.

### Neutral

- Action's runtime dispatch shape (4-variant `ActionHandler` enum, RPITIT body `impl Future + Send + 'a`) is unchanged. Trait-family enumeration (canon §3.5) is preserved at runtime. Governance / DX-tier seal questions are scoped to ADR-0038.
- Public API surface of the 4 dispatch traits (`StatelessAction` / `StatefulAction` / `TriggerAction` / `ResourceAction`) is unchanged at the trait level — only the macro that constructs implementations changes shape.

## Alternatives considered

### Alternative A — Keep `#[derive(Action)]`, layer field-rewriting via wrapper types

Plugin author writes `#[derive(Action)] struct X { #[credential] slack: CredentialSlot<SlackToken> }`. The wrapper type `CredentialSlot<C>` performs the typed-handle work; the derive emits adjacent `ActionSlots` referencing the field name.

**Rejected.** Two debts: (a) `CredentialSlot<C>` becomes a parallel-vocabulary to `CredentialRef<C>` from credential CP6 — two-vocabulary maintenance violates Strategy §3.2 placement lock; (b) the derive cannot rewrite `CredentialRef<dyn BitbucketBearer>` → `CredentialRef<dyn BitbucketBearerPhantom>` per ADR-0035 §4.3, so the action-side phantom-shim rewrite obligation cannot be discharged. ADR-0035's phantom contract becomes structurally unenforceable. This is the explicit B' fallback path Strategy §3.2 / §6.8 reserves for the case where attribute-macro adoption fails — used here as fallback only.

### Alternative B — Attribute macro with arbitrary field rewriting

`#[action]` rewrites every field type per a global mapping table (e.g., `String` → `&str` reborrow, `Credential = X` → `CredentialRef<X>`). User writes near-plain Rust; macro performs ergonomic type munging.

**Rejected.** DX harm > phantom safety win. Fields visible in source no longer mean what they say at compile time — IDE hover, grep, and rustdoc diverge from the actual generated type. Plugin authors hit "why does this `String` field act like `&str`?" mysteries. Narrow attribute-zone rewriting per the chosen decision is the principled middle: rewriting is opt-in, zone-bounded, and visible in the attribute syntax.

### Alternative C — Procedural macro that consumes `impl Action for X { ... }` blocks

Instead of an attribute on the struct, macro consumes the impl block. Field shape stays plain; macro injects synthesized methods.

**Rejected.** Cannot rewrite struct fields (impl blocks come after struct declaration). The credential-handle field-type rewrite is structurally absent. Same root cause as Alternative A's derive limitation.

## References

- [Strategy Document](../superpowers/specs/2026-04-24-action-redesign-strategy.md) — §3.2 placement lock; §4.3.1 macro modernization; §6.2 ADR roadmap; §6.8 B'+ contingency activation criteria (fallback path for this decision).
- [Scope decision](../superpowers/drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) — §1 chosen scope (Option A'); §1.6 plugin ecosystem migration design.
- [Spike NOTES](../superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md) — §1.4 Probe 3 result (option (b) type-system-enforceable); §3 finding #3 (dual enforcement layer rationale); commit `c8aef6a0`.
- [ADR-0035 phantom-shim capability pattern](./0035-phantom-shim-capability-pattern.md) — §4.3 action-side rewrite obligation discharged by this ADR + ADR-0037.
- [Credential Tech Spec](../superpowers/specs/2026-04-24-credential-tech-spec.md) — §2.7 line 486-528 macro translation; §3.4 line 807-939 dispatch narrative.

---

*Proposed by: architect (nebula-action redesign cascade Phase 5), 2026-04-24. Composed with ADR-0037 (emission contract) and ADR-0038 (ControlAction seal + canon revision).*
