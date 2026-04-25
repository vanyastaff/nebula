---
id: 0038
title: controlaction-seal-canon-revision
status: proposed
date: 2026-04-24
supersedes: []
superseded_by: []
tags: [action, governance, canon-revision, sealed-trait, canon-3.5, canon-0.2, cascade-action-redesign]
related:
  - docs/PRODUCT_CANON.md#35-integration-model-one-pattern-five-concepts
  - docs/PRODUCT_CANON.md#02-when-canon-is-wrong-revision-triggers
  - docs/superpowers/specs/2026-04-24-action-redesign-strategy.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
linear: []
---

# 0038. ControlAction seal + canon §3.5 DX tier ratification

## Status

**Proposed** — drafted 2026-04-24 as the third of the 3-ADR set for the nebula-action redesign cascade ([Strategy §6.2](../superpowers/specs/2026-04-24-action-redesign-strategy.md#62-adr-drafting-roadmap)). Drafted after ADR-0036 (trait shape) and ADR-0037 (emission contract) so the canon revision rationale is grounded in the prior two locks. Tech-lead solo-decided this call at Phase 1 ([pain enumeration §7](../superpowers/drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md)); user ratification at Phase 8 cascade summary. Status moves to `accepted` upon user ratification.

## Context

[`PRODUCT_CANON.md` §3.5 line 82](../PRODUCT_CANON.md#35-integration-model-one-pattern-five-concepts) enumerates the action trait family:

> Action — what a step does. Dispatch via action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`). Adding a trait requires canon revision (§0.2).

`PRODUCT_CANON.md` §0.2 (line 27) specifies canon revision triggers — "capability lag" + "uncovered case" both fire on the current state.

The implementation reality diverges:

- `crates/action/src/lib.rs:13-20` re-exports **10 trait surfaces**, including `ControlAction` (public, **non-sealed**, fifth dispatch-time-shaped trait) plus 5 DX specialization traits (`PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`, `ControlAction`).
- The library docstring at `crates/action/src/lib.rs:11` self-contradicts: it re-states the canon-revision rule while re-exporting 10 traits that exceed the canon enumeration.
- Engine dispatch (verified by tech-lead grep) is preserved as a clean 4-variant `ActionHandler` enum — `ControlAction` and DX traits "erase to primary" at dispatch, so this is **documentation drift, not structural drift**.
- Unfixed, the public-non-sealed shape of `ControlAction` is a literal canon §3.5 violation by §0.2 wording: external plugin crates can `impl ControlAction for LocalType` and produce a fifth dispatch-time trait without canon revision.

Two governance forces converge:

- **Strategy §1(c) — canon §3.5 governance drift via ControlAction + DX tier.** Documentation drift today; structural risk under any plugin-author who treats public-non-sealed as license to extend.
- **`feedback_active_dev_mode`** — "don't half-seal." Either close the seal cleanly (sealed trait pattern, no public impl) or remove the trait. Half-measures (e.g., `#[doc(hidden)]` while public) preserve the violation.

The action redesign cascade is the right place to settle this — A' touches the `lib.rs` re-export block, the macro layer, and the canon section anyway. Carrying the violation forward without resolution is a `feedback_active_dev_mode` cost compounding into the next cascade.

A canon §3.5 revision (per §0.2) is the correct mechanism — the canon enumerates the dispatch core, and the DX tier is structurally a wrapper layer. The canon as currently written has a silent gap (it does not anticipate sealed-DX-sugar over primary traits). The revision makes the gap explicit + closes the trait-family-extension door.

## Decision

### §1. Seal `ControlAction` (sealed-trait pattern)

`ControlAction` becomes a sealed trait — community plugin crates may NOT implement it directly. Sealing follows the per-capability inner-sealed-trait pattern from [ADR-0035 §3](./0035-phantom-shim-capability-pattern.md#3-sealed-module-placement-convention):

```rust
// In crates/action/src/lib.rs (or a dedicated sealed module):
mod sealed_dx {
    pub trait ControlActionSealed {}
    pub trait PaginatedActionSealed {}
    pub trait BatchActionSealed {}
    pub trait WebhookActionSealed {}
    pub trait PollActionSealed {}
}

pub trait ControlAction: sealed_dx::ControlActionSealed { /* ... */ }
```

The blanket `impl<T: StatelessAction> sealed_dx::ControlActionSealed for T {}` (or analogous, depending on the wrap shape) ensures only `StatelessAction` implementors gain `ControlAction` membership via the **adapter pattern** — community plugins use `StatelessAction` as the primary dispatch trait + adapter to gain `ControlAction` semantics. Internal Nebula crates may continue to author `ControlAction`-using actions through the sealed adapter.

Same per-capability sealed convention for the four DX specialization traits (`PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`). All five DX traits become sealed.

### §2. Revise canon §3.5 to enumerate the DX tier explicitly

Replace canon §3.5 line 82 wording:

**Before:**
> Action — what a step does. Dispatch via action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`). Adding a trait requires canon revision (§0.2).

**After:**
> Action — what a step does. Dispatch via 4 primary trait variants (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`). Authoring DX wraps these via sealed sugar traits (`ControlAction`, `PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`) — adding a primary variant requires canon revision (§0.2); adding a sealed DX trait is a non-canon-revision act.

The revision distinguishes two structurally different acts:

- **Adding a primary dispatch trait** — affects engine dispatch shape, requires `ActionHandler` enum extension, requires canon revision per §0.2.
- **Adding a sealed DX sugar trait** — wraps existing primary, erases to primary at dispatch, does NOT extend `ActionHandler`. No canon revision required, but each DX trait must be sealed (per §1) to prevent governance drift back to today's state.

This is a **canon revision per §0.2** — "capability lag" trigger fires (the canon as written has a silent gap; reality reveals the gap; canon revision closes it).

### §3. CR3 fix — hard removal, not deprecated shim

Per [scope decision §3 must-have floor](../superpowers/drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) + Strategy §2.11 `feedback_no_shims` citation, the related CR3 cross-plugin shadow attack fix is **hard removal** of `CredentialContextExt::credential<S>()` no-key heuristic, not `#[deprecated]` keeping the heuristic compilable. Security-lead 03c §1 retains implementation-time VETO on this point; this ADR does not relax that requirement.

CR3 fix is part of A' implementation per scope decision §1.4; this ADR notes the hard-removal discipline as ratified, not opens it for negotiation.

## Consequences

### Positive

1. Closes Strategy §1(c) governance drift — `ControlAction` and 4 DX traits become sealed; community plugin crates cannot extend the dispatch surface; canon §3.5 reads honestly.
2. Canon §3.5 revision is structural (recognizes a tier the canon was silent on), not retroactive (the 4-primary enumeration is preserved). Aligns canon with `crates/action/src/lib.rs` re-export honestly.
3. Engine dispatch shape unchanged — `ActionHandler` enum stays 4-variant; sealing the DX tier formalizes the "erases to primary" property already true at runtime. No engine-layer surgery required.
4. Sealed pattern composes with [ADR-0035 §3](./0035-phantom-shim-capability-pattern.md) per-capability inner-sealed-trait convention — same crate-private `mod sealed_dx { pub trait XSealed {} ... }` shape as ADR-0035's `mod sealed_caps`. Consistency across crate-internal sealing patterns.
5. `feedback_active_dev_mode` discipline applied — no half-seal, no `#[doc(hidden)]`-while-public, no deferred-canon-revision. Resolution lands in cascade.
6. Plugin authors gain a clear contract: primary dispatch via 4 primary traits, DX via sealed sugar (must use adapter pattern via `StatelessAction` + sealed wrap). No surprise canon-revision cost when they think they're "just implementing a trait."

### Negative

1. **Existing ControlAction implementors in Nebula-internal crates must migrate to the sealed shape.** Likely small surface (no tracked external implementors per Strategy §1(c)). Migration cost is one-time; in-cascade per scope §1.5.
2. **Canon revision PR lands alongside ADR.** Adds coordination — canon §3.5 wording change requires explicit user ratification at Phase 8 (per [scope §1.5 tech-lead solo-decided calls ratified in cascade scope](../superpowers/drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md)). User can reject this ADR + canon revision as a unit; tech-lead solo-decision at Phase 1 is presented for user-level review.
3. **Plugin authors lose the freedom to define a 5th DX trait themselves.** Sealing the DX tier means the 5 sealed traits ship with `nebula-action`; authors who want a 6th must propose it for inclusion in `nebula-action` (or build it as a non-sealed convenience layer outside the action vocabulary). This is the desired governance posture per §1.
4. **Two-step user-facing migration:** code that today does `impl ControlAction for X` must move to `impl StatelessAction for X` + sealed adapter. Codemod can cover the common case; edge cases (control-flow-specific behavior) need hand migration. In-cascade per scope §1.6.

### Neutral

- Public API surface of the 4 primary traits (`StatelessAction` / `StatefulAction` / `TriggerAction` / `ResourceAction`) is unchanged. The DX tier seal is additive over an existing pattern, not a structural reshuffle.
- `crates/action/src/lib.rs:11` library docstring becomes truthful — currently self-contradicting (re-states canon while re-exporting 10 traits); post-cascade it states the actual shape.

## Alternatives considered

### Alternative A — Demote `ControlAction` to free helper functions

Remove `ControlAction` trait entirely; replace with free functions `apply_control_flow(...)` users call from `StatelessAction::execute`.

**Rejected.** DX harm. The trait provides typed dispatch over control-flow result variants (`Continue` / `Skip` / `Retry`); free functions lose the trait-based ergonomic. The trait is structurally good — the violation is the *non-sealed* shape, not the trait itself. Sealing is the targeted fix; deletion is over-correction.

### Alternative B — Keep `ControlAction` public-non-sealed; add canon §3.5 revision recognizing a "wider DX tier"

Accept the governance drift; canon revision codifies the de-facto state without adding a seal.

**Rejected.** `feedback_active_dev_mode` violation — accepting the drift means each future DX trait can be added without sealing. Plugin authors who add their own sealed-but-public 6th trait become structurally indistinguishable from Nebula-internal DX. Canon governance signal weakens. The seal is the load-bearing part; canon revision without seal is just paperwork.

### Alternative C — Defer canon revision to a later cascade; ship the seal alone

Land the seal in this cascade; canon revision lands in a follow-up.

**Rejected.** `feedback_active_dev_mode` "before saying defer X, confirm the follow-up has a home." A follow-up canon-revision cascade has no scheduled home — the canon revision IS the gating governance act for sealing. Deferring it means the canon §3.5 line 82 wording stays "adding a trait requires canon revision" while we just sealed 5 traits — the canon and the code disagree visibly. Carry-forward governance debt is exactly what `feedback_active_dev_mode` flags.

## Implementation notes

### Changes to canon

Inline edit at [`docs/PRODUCT_CANON.md` §3.5 line 82](../PRODUCT_CANON.md#35-integration-model-one-pattern-five-concepts) with the wording change in §2 above. The canon edit lands as a PR alongside this ADR per [Strategy §2.2](../superpowers/specs/2026-04-24-action-redesign-strategy.md#2-constraints).

### Changes to `crates/action/src/lib.rs`

- Add `mod sealed_dx { ... }` with 5 inner sealed traits (one per DX trait).
- Add blanket impls sealing each DX trait against its eligible primary (likely all on `StatelessAction`; trait-by-trait audit at Tech Spec §7 design time).
- Update library docstring at line 11 to match the canon §3.5 revised wording (drop self-contradiction).

### Phase 8 ratification

User ratification gates ADR `accepted` status. Tech-lead solo-decided this call at Phase 1; cascade summary surfaces both the seal + canon revision wording change for user review.

## References

- [`PRODUCT_CANON.md` §3.5 line 82](../PRODUCT_CANON.md#35-integration-model-one-pattern-five-concepts) — current 4-trait enumeration; revised by this ADR.
- [`PRODUCT_CANON.md` §0.2 line 27](../PRODUCT_CANON.md#02-when-canon-is-wrong-revision-triggers) — canon revision triggers ("capability lag" fires).
- [Strategy Document](../superpowers/specs/2026-04-24-action-redesign-strategy.md) — §1(c) governance drift; §2.1 / §2.2 canon constraints; §6.2 ADR roadmap.
- [Phase 1 pain enumeration](../superpowers/drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md) — §7 tech-lead Phase 1 solo decisions, including the ControlAction seal call (`decision_controlaction_seal`).
- [Phase 2 scope decision](../superpowers/drafts/2026-04-24-nebula-action-redesign/03-scope-decision.md) — §1.5 ratifies tech-lead Phase 1 calls in cascade scope; §3 must-have floor (CR3 hard-removal discipline cited in §3 above).
- [ADR-0035 phantom-shim capability pattern](./0035-phantom-shim-capability-pattern.md) — §3 sealed module placement convention (per-capability inner sealed traits); composition reference for §1 sealed-DX shape.
- [ADR-0036 action trait shape](./0036-action-trait-shape.md) + [ADR-0037 macro emission](./0037-action-macro-emission.md) — companion ADRs in the 3-ADR set; macro emission contract assumes the sealed DX tier from this ADR.

---

*Proposed by: architect (nebula-action redesign cascade Phase 5), 2026-04-24. Tech-lead Phase 1 solo decision; user ratification at Phase 8 cascade summary. Composed with ADR-0036 (trait shape) and ADR-0037 (emission contract).*
