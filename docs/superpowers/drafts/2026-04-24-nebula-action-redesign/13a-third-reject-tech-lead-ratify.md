---
name: Tech-lead ratification — Third post-freeze REJECT-refined (3C) on §2.9
description: Solo-decider ratification of architect's third re-examination of §2.9. Confirms schema-as-data vs schema-as-trait-type axis, n8n implementation evidence, COMPETITIVE.md citation, ADR-0035 phantom-shim composition preservation, amendment scope, re-open trigger refinement.
status: ratified
type: ratification
date: 2026-04-25
related: [docs/superpowers/drafts/2026-04-24-nebula-action-redesign/13-third-reanalysis-n8n-consumer.md, docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md §2.9.1c, §2.9.5-7, §15.9.6, docs/COMPETITIVE.md line 29 + 41]
---

## Ratification verdict (RATIFY / RE-ITERATE / ESCALATE)

**RATIFY.** Third iteration commit-ready. No round-2 needed.

## Schema-as-data axis distinction soundness

**Sound.** Verified at code: `crates/action/src/metadata.rs:98-292` carries `inputs: Vec<InputPort>` (line 105), `outputs: Vec<OutputPort>` (108), and universal `with_schema(schema: ValidSchema)` builder (292). All four `for_*` helpers (166-222) project `<A::Input as HasSchema>::schema()` into `with_schema` — schema-as-data is the universal mechanism that works across all 4 trait variants TODAY without trait-level Input/Output. The three n8n consumers (UI form gen via `base.schema`; downstream type-checking via `outputs: Vec<OutputPort>`; filter validation via `ValidSchema` constraints) map cleanly onto the data axis. Distinction is structurally correct, not rhetorical.

## n8n implementation evidence verification

**Verified.** Architect's claim that n8n's `INodeTypeDescription` is runtime data (not TypeScript GAT) matches public n8n source — `INodeTypeDescription.properties` / `.inputs` / `.outputs` are JS arrays, not generic type parameters. n8n is JS/TS at runtime; compile-time generic associated types are not part of its reflection surface. Architect's framing — "n8n's reflection is data, not types" — is true.

## COMPETITIVE.md citation verification

**Verified verbatim at lines 29 + 41.**
- Line 29: «Reliability and clarity of execution as a system, plus DX for integration authors — not feature parity with n8n/Make on day one, and **not** a surface-area race in v1.»
- Line 41: «**Our bet:** Typed Rust integration contracts + honest durability beat a large but soft ecosystem; a smaller library of reliable nodes wins over time.»

These are canon-level disclaim of n8n surface parity. Architect's citation is faithful.

## handle() refactor / ADR-0035 break check

**Architect's REJECT is correct on all four sub-grounds.** The structurally critical one is reason #3: ADR-0035 §4.3 phantom-shim composition rewrites `&self` field-zone for credentials/resources slots. If `handle()` accepted `Self::Input` carrying configuration, credential `CredentialRef<dyn KafkaSaslPhantom>` for trigger-time auth would have nowhere to live (method-parameter types are not field-zone targets). The `&self` configuration carrier is structurally load-bearing for ADR-0035 §4.3 — the refactor would break composition. Reasons #1 (per-event vs per-registration lifecycle), #2 (cross-trait symmetry breakage), and #4 (n8n's `INodeType.execute` reads via `this.getNodeParameter()` from instance, paralleling `&self`) are correct supporting axes.

## Amendment scope appropriateness

**Appropriate per §15.9.5 Q2 precedent.** Q3 is rationale-tightening only:
- §2.9.1c (new sub-subsection) records Q3 pushback verbatim + names schema-as-data vs schema-as-trait-type axis — sibling structure to §2.9.1a / §2.9.1b.
- §2.9.5 verdict gains "(refined three times)" annotation; verdict unchanged (REJECT consolidation, status quo Option C).
- §2.9.6 prelude moves three-axis → four-axis; point 2 refines "no current consumer" → "no current consumer on the schema-as-trait-type axis" with COMPETITIVE.md line 41 citation.
- §2.9.7 implications wording extended; re-open trigger second bullet refined.
- §15.9.6 new enactment record (sibling to §15.9.5).
- No §2.2 signature ripple. No status header qualifier change (per §15.9.5 precedent — rationale-tightening without signature ripple). No ADR-0038 / 0037 / 0038 amendment. ADR-0035 phantom-shim composition explicitly preserved.

This matches §15.9.5 Q2 precedent exactly: rationale refinement absorbed into §2.9 sub-subsection + §15.9.6 record + verdict annotation, no status qualifier change.

## Re-open trigger concreteness

**Concrete enough.** §2.9.7 second bullet now reads "schema-as-trait-type consumer (compile-time / type-system-mediated walking by `Action<I, O>` type identity)" with worked examples: dependency-typed resource graph walking by trait-level type identity, or compile-time `fn collect<T: Action<I, O>>` aggregation. The two example forms are concrete enough that a future reviewer can disambiguate "is THIS a re-open trigger?" against them. The COMPETITIVE.md disclaim explicitly excludes n8n parity from re-open territory — closes the most likely false-positive trigger surface.

Future cascade scenarios that WOULD trigger re-open: a `nebula-ui` crate walking `T: Action<I, O>` for visual schema rendering; a canon §3.5 revision committing to n8n surface parity. Both are forward-pointing acknowledgments per architect's section "Forward-pointing acknowledgement" — appropriate scope.

## Summary

Architect's third re-examination cleanly resolves the user's concrete-consumer pushback. The schema-as-data vs schema-as-trait-type axis is the missed framing prior REJECTs left implicit; naming it explicitly closes the rationale gap without changing the verdict. n8n's runtime-data reflection model is correctly identified as living on the data axis (where Nebula's surface is universal via `ActionMetadata::with_schema`). COMPETITIVE.md citation is faithful and decisive — n8n parity is canon-level non-goal. The `handle()` refactor REJECT is structurally grounded (ADR-0035 composition + lifecycle + cross-trait symmetry + n8n parallel). Amendment scope mirrors §15.9.5 Q2 precedent exactly: rationale-only, no signature ripple, no status qualifier change, no ADR amendment.

**Commit-ready: YES.** Ratify-and-commit on this iteration. Third iteration of same question is closed; further pushback on §2.9 from same axes would repeat ground already covered three times.
