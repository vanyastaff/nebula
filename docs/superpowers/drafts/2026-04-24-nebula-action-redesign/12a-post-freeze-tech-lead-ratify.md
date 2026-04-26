---
name: tech-lead ratification — post-freeze amendment-in-place (Q1 + Q2)
status: complete
date: 2026-04-25
authors: [tech-lead]
scope: Ratify architect's amendment-in-place enacted in `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` per `12-post-freeze-reanalysis.md`
related:
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/12-post-freeze-reanalysis.md
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md
  - docs/adr/0024-defer-dynosaur-migration.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
---

## Post-freeze amendment ratification verdict (RATIFY / RE-ITERATE / ESCALATE)

**RATIFY.** Commit-ready: yes. No escalation.

The amendment-in-place is a canonical-form correction realigning Tech Spec §2.3 + §2.4 with already-ratified ADR-0024. Per ADR-0035 amended-in-place precedent, post-freeze correction of cross-ADR consistency violation is the proportionate mechanism (no supersede ADR; no fresh CP gate). Architect's honest correction record (12-post-freeze-reanalysis.md §Q1) names the freeze-time miss precisely.

## ADR-0024 cross-reference verification

Verified at `docs/adr/0024-defer-dynosaur-migration.md` lines 67-99:

- **§Decision item 1 (lines 67-71):** "`#[async_trait]` is the approved mechanism for the 14 remaining `dyn`-consumed async traits in Nebula."
- **§Decision item 1 enumeration (lines 73-81):** explicitly lists `TriggerHandler`, `StatelessHandler`, `ResourceHandler`, `StatefulHandler` among the 14 dyn-consumed traits.
- **§Decision item 4 (lines 93-99):** "`#[async_trait]` is not forbidden for new code when the trait is `dyn`-consumed from day one."

The pre-amendment Tech Spec §2.4 mandating manual `BoxFut<'a, T>` per-method on these exact 4 traits was a cross-ADR consistency violation. ADR-0024 (accepted 2026-04-20) predates Tech Spec FROZEN CP4 (2026-04-25) by 5 days; the freeze ratification protocol did not include cross-ADR check against workspace-wide ADRs (only cascade-internal four were in `related:`). Architect's amendment realigns to source-of-truth ADR. **Correct fix.**

## §2.3 BoxFut survival scope check

Verified at Tech Spec line 244-256 + line 2466. `BoxFut<'a, T>` alias survives **only** for `SlotBinding::resolve_fn` HRTB fn-pointer per credential Tech Spec §3.4 line 869 + ADR-0039 §1. Structurally distinct from `*Handler` per-method async return:

- HRTB fn-pointer with `for<'ctx>` quantification — compile-time monomorphized at slot registration
- Not a method on a `dyn`-consumable trait — `#[async_trait]` does not rewrite HRTB fn-pointer signatures

Survives correctly. Single use site keeps the alias load-bearing without spillover into `*Handler` shape.

## §2.4 async_trait cancel-safety preservation

Verified at Tech Spec line 312 (equivalence note). `#[async_trait]` macro-expands to `Pin<Box<dyn Future<Output = T> + Send + 'async_trait>>` per method — structurally equivalent to pre-amendment manual `BoxFut<'a, T>`. Heap allocation per call unchanged; drop semantics on `SchemeGuard<'a, C>` mid-`.await` unchanged (spike Iter-2 §2.4 cancellation drop test passes under either shape per architect's re-analysis table). Bytecode delta non-existent.

CP2 §6.4 cancel-safety invariants (cancel-on-drop discipline; `SchemeGuard` zeroize-on-drop on cancellation; `tokio::select!` body discipline) are method-body-level invariants — `#[async_trait]`'s `Box::pin(async move { body })` wrapper preserves them. **No regression.**

## §2.9.1b three-axes acceptance

Verified at Tech Spec line 511-534. Three-axis distinction named explicitly:

1. **Trait-method-input axis** — `handle(&self, ctx, event)` parameter at type-system level; engine sources events from `Source: TriggerSource` and dispatches each into `handle`.
2. **Trigger-purpose-input axis** — workflow-nomenclature framing where configuration (RSS url + interval, Kafka channel) is "input" and events are conceptually "output."
3. **Configuration axis** — `&self` fields + `parameters = T` schema, universal across all 4 traits.

User's verbatim Russian pushback recorded; nomenclature acknowledged correct for workflow-purpose axis. REJECT verdict preserved on lifecycle-method divergence basis: consolidation breaks under all three axes (trait-method-input forces redundant projection or `type Input = ()` lie; trigger-purpose-input forces every action to declare `type Config`, paradigm-breaking universal `&self`-fields pattern; output framing not a method-return value). **Refinement is rationale-tightening only — verdict basis sharpened, not changed.** Q2 ratification stands.

## Status header amendment qualifier

Verified at Tech Spec line 3 + line 33 (§0.1 status table CP4 row). Qualifier reads:

> `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 post-freeze)`

Status table row carries full attribution: "amended-in-place 2026-04-25 post-freeze for Q1 `*Handler` shape per §15.9 + Q2 §2.9.1b axis-naming refinement per ADR-0035 amended-in-place precedent."

Per ADR-0035 amended-in-place precedent (referenced throughout Tech Spec §2037 and adjacent §15.5 ADR-0039 amendment), post-freeze canonical-form correction is the proportionate mechanism. Q1 (structural §2.3 + §2.4 signature change) is the amendment qualifier; Q2 (rationale-tightening only, verdict unchanged) is appropriately recorded in §15.9.5 without separate header qualifier. **Header form correct.**

## Regressions check

Verified across CP1/CP2/CP3 design surfaces:

- **CP2 §6.4 cancel-safety guarantees** — preserved (see §2.4 check above; `Box::pin` wrapper preserves drop semantics).
- **Spike NOTES finding #1 (SchemeGuard auto-deref Clone shadow probe)** — orthogonal to `*Handler` shape (probe lives at `SchemeGuard` `impl<'a, C: Capability>` level, not in handler trait method signatures). No interaction.
- **§2.2 RPITIT signatures** (4 primary traits Stateless/Stateful/Trigger/Resource) — preserved per ADR-0024 §Decision item 3 ("Native AFIT remains the default for new traits that are not `dyn`-consumed"). The 4 primary traits stay native AFIT; only the 4 `*Handler` companions flip to `#[async_trait]`. Type-state separation between primary and companion preserved.
- **CP3 §9-§13 floor items** (4 floor items including §9.5 secret-propagation `nebula-redact` + §13.4.3 deny.toml wrappers-list) — none sensitive to `*Handler` method signature shape. Untouched.
- **ADR-0038 / ADR-0039 / ADR-0040 status invariants** — unchanged (verified at line 2168-2169). ADR-0038 locks trait shape at the action trait family level (not `*Handler` per-method); ADR-0039 amended-in-place per §15.5 (orthogonal to this Q1 amendment); ADR-0040 still proposed pending user ratification on canon §3.5 revision (correctly excluded from this scope).
- **§16.5 cascade-final precondition** — verified at line 2484-2486. No new precondition; implementation absorbs `#[async_trait]` adoption mechanically (Cargo.toml dep already in `[workspace.dependencies]` per ADR-0024).

**No regressions.**

## Summary

Cross-ADR violation correctly identified by user pushback; architect's amendment-in-place is the proportionate mechanism; all six ratification checks pass; no contested calls. Tech Spec FROZEN CP4 stands with the post-freeze amendment-in-place qualifier.

**Verdict: RATIFY. Commit-ready: yes. No escalation. ADR-0040 not auto-flipped (canon §3.5 revision still pending user ratification — correctly out-of-scope).**
