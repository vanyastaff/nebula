---
name: post-freeze re-examination of FROZEN CP4 — Q1 + Q2
status: complete
date: 2026-04-25
authors: [architect]
scope: Re-examine two design decisions on Tech Spec FROZEN CP4 2026-04-25 commit `d24d318f` per ADR-0035 amended-in-place precedent
related:
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md
  - docs/adr/0024-defer-dynosaur-migration.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
  - docs/adr/0038-action-trait-shape.md
  - docs/adr/0039-action-macro-emission.md
---

# Post-freeze re-examination — Q1 + Q2 outcomes

## Q1 — `async_trait` vs manual `BoxFut<'a, T>` on `*Handler` traits

### Outcome: ACCEPT (amendment-in-place)

User's pushback identified a **load-bearing audit miss** in the Tech Spec freeze: ADR-0024 (accepted 2026-04-20, four days before Tech Spec FROZEN CP4) explicitly enumerates `StatelessHandler` / `StatefulHandler` / `TriggerHandler` / `ResourceHandler` (lines 73-81 of the ADR) among the 14 dyn-consumed traits approved for `#[async_trait]`. Per ADR-0024 §Decision items 1 + 4:

> "`#[async_trait]` is the approved mechanism for the 14 remaining `dyn`-consumed async traits in Nebula" — §Decision item 1 (line 67-71)
>
> "`#[async_trait]` is not forbidden for new code when the trait is `dyn`-consumed from day one" — §Decision item 4 (line 93-99)

The pre-amendment Tech Spec §2.3 + §2.4 locked manual `BoxFut<'a, T>` per `*Handler` method — a hand-rolled approximation of what `#[async_trait]` emits internally. This was a cross-ADR consistency violation: the Tech Spec freeze did NOT cite ADR-0024, and the architect (this agent) introduced a shape that contradicts already-ratified workspace policy.

### Re-analysis findings

| Axis | Manual `BoxFut<'a, T>` | `#[async_trait]` | Verdict |
|---|---|---|---|
| **Performance** | One `Pin<Box<dyn Future + Send>>` per call | One `Pin<Box<dyn Future + Send + 'async_trait>>` per call (macro emits) | **Equivalent** — bytecode delta non-existent |
| **Cancel safety** | Drop semantics on `SchemeGuard<'a, C>` mid-`.await` per spike Iter-2 §2.4 | Same — `Box::pin(async move { body })` preserves drop order | **Equivalent** — spike test passes either shape |
| **Idiom currency 1.95** | rust-senior 02c §6 marked `for<'life0, 'life1, 'a>` DATED; recommended single-`'a` + `BoxFut` | ADR-0024 §Decision item 1 + 4 — workspace-policy-aligned | `#[async_trait]` aligns with already-ratified ADR; rust-senior recommendation is honored at the source level (no `'life0` boilerplate visible) |
| **Migration on `async_fn_in_dyn_trait` stabilization** | Per-method-signature edit times 4 traits + drop `BoxFut<'a, T>` returns + lifetime parameters | One attribute deletion per trait | **`#[async_trait]` mechanically simpler** (user's pushback correct) |
| **DX (plugin authors)** | Authors don't implement `*Handler` — adapters emit it | Same | **Equivalent** — DX delta is rust-internal |
| **Ecosystem composition** | nebula-action would be the only crate hand-rolling HRTB without using `#[async_trait]` (verified: plugin-sdk + api use `#[async_trait]`; other dyn-consumed traits across workspace use it per ADR-0024 enumeration) | Composes uniformly with rest of workspace | `#[async_trait]` is the workspace-uniform path |

### Honest correction record

The architect's pre-freeze position ("explicit-over-magic") was wrong on two counts:
1. It did not consult ADR-0024 — the ADR explicitly governs this exact decision for these exact 4 traits.
2. The manual `BoxFut` form is not "explicit-over-magic" — it is a hand-rolled approximation of what `#[async_trait]` emits. There is no cleaner explicit form; both shapes produce identical runtime.

User's pushback is correct. The 15k-crates ecosystem usage figure reflects mature workspace adoption; ADR-0024's already-ratified policy is the workspace's response to that ecosystem reality.

### Amendment trail (Q1)

**Tech Spec inline edits** (`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`):
- **Status header** (line 3) — `FROZEN CP4 2026-04-25` → `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 post-freeze)`
- **§0.1 status table** CP4 row — gained "amended-in-place 2026-04-25 post-freeze for Q1 `*Handler` shape per §15.9" qualifier
- **§2.3** — amended-in-place callout added at section top; rewritten as `BoxFut<'a, T>` alias used at `SlotBinding::resolve_fn` HRTB only (single use site); narrative refined to name the survival rationale
- **§2.4** — amended-in-place callout added at section top; four `*Handler` traits flipped from manual `BoxFut<'a, T>` per-method to `#[async_trait::async_trait]` per ADR-0024 §Decision items 1 + 4; cancel-safety equivalence note added; migration trigger when `async_fn_in_dyn_trait` stabilizes named; three reasons for `#[async_trait]` over manual `BoxFut` enumerated
- **§15.9** (NEW subsection) — Q1 post-freeze amendment-in-place enactment record. Five sub-subsections: §15.9.1 enactment (no ADR file edit needed); §15.9.2 amend-in-place vs supersede rationale; §15.9.3 cross-cascade and downstream impact; §15.9.4 §16.5 cascade-final precondition update; §15.9.5 Q2 refinement record (no separate Q2 qualifier)
- **CHANGELOG — post-freeze amendment-in-place 2026-04-25** entry added at end of document

**No ADR file edits required:**
- ADR-0024 is source-of-truth and unchanged.
- ADR-0038 locks trait shape (the action trait family + `#[action]` macro emission for action structs) — does NOT lock `*Handler` per-method async return shape. No edit needed.
- ADR-0039 locks macro emission for the action struct (`ActionSlots::credential_slots()`, `SlotBinding`, qualified-syntax probe, test harness, perf bound) — none sensitive to `*Handler` shape. No edit needed beyond the §15.5 amendment already enacted.
- ADR-0035 phantom-shim composition preserved (field-shape-level, not method-signature-level).

**Production code impact (next implementation step):**
- `crates/action/Cargo.toml` — add `async-trait = { workspace = true }` to `[dependencies]` (already in `[workspace.dependencies]` per ADR-0024)
- `crates/action/src/{stateless.rs:300-322, stateful.rs:445-472, trigger.rs:300-381, resource.rs:65-106}` — replace hand-rolled `for<'life0, 'life1, 'a>` HRTB with `#[async_trait]` annotation; rewrite `*Handler` trait method signatures to `async fn` form per Tech Spec §2.4
- `crates/action/src/handler.rs:39-50` — `ActionHandler` enum unchanged (variants `Arc<dyn StatelessHandler>` etc. continue to compile)

---

## Q2 — TriggerAction Input/Output framing under user's standard-workflow nomenclature

### Outcome: REJECT (refined)

User's framing is **correct standard workflow nomenclature**: in n8n / Temporal / Camunda / Argo conventions, a trigger's *purpose* is "produce events" — events are conceptually trigger output; configuration (RSS url + interval, Kafka channel) is the user-supplied input at user-settings time.

The §2.9.2 table column "Input shape" was loosely worded — it conflated two distinct lifecycle phases:
1. **Trait-method-input axis** — the trait method `handle()`'s input parameter at the type-system level (where consolidation would actually consolidate as `type Input` on the trait)
2. **Trigger-purpose-input axis** — the user's standard-nomenclature framing where configuration is "input" and events are "output"

### Re-analysis findings

Both lenses are valid; they measure different things:

- **Trait-method-input axis (what §2.9.2 actually measures):** `handle(&self, ctx, event)` — the `event` parameter IS a method input at the type-system level. The engine sources events from `Source: TriggerSource` and dispatches each into `handle`.
- **Trigger-purpose axis (what the user names):** A trigger's reason-for-existence is "produce events for the engine's event channel." Events are conceptually trigger output; configuration is the trigger's input (universal across all 4 traits, lives in `&self` fields + `parameters = T` schema).

**Neither axis enables `Action<Input, Output>` consolidation:**

- **Trait-method-input axis:** Trigger's `event` projection is asymmetric to Stateless/Stateful/Resource's user-input. Consolidation would force `type Input = <Source as TriggerSource>::Event` (redundant projection) or `type Input = ()` (lying about the actual method input).
- **Trigger-purpose axis:** Configuration is universal across all 4 traits, lives in `&self` fields + `parameters = T`. Hoisting "configuration" to a `type Config` associated trait surface would force every action (including Stateless) to declare `type Config = SomeStruct` — paradigm-breaking the universal `&self`-fields pattern.
- **Output axis under user's framing:** "Trigger emits event stream" lives at the trigger lifecycle level (engine drives `start` → engine receives events on a channel → engine dispatches `handle(event)` per event). The trait method `handle()` returns `Result<(), Error>`; aggregate "trigger output = event stream" is not a method-return value.

**Verdict.** User's nomenclature is correct for the trigger-purpose axis; consolidation still cannot honestly resolve the trait-method-signature-level divergence under any of the three axes. REJECT preserved; rationale tightened.

### Where Tech Spec rationale was tightened (Q2)

**Tech Spec inline edits** (`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`):
- **§2.9.1b** (NEW sub-subsection) — user's verbatim Russian pushback recorded; three-axis distinction named explicitly (trait-method-input vs trigger-purpose-input vs configuration); user's nomenclature acknowledged as correct for the trigger-purpose axis; consolidation breakdown traced under each of the three axes; §2.9-1 carry-forward closure (§15.6) reaffirmed.
- **§2.9.5** — verdict annotation refined: "Rationale tightened across two iterations: CP2 2026-04-24 per §2.9.1a; post-freeze 2026-04-25 per §2.9.1b — three-axis distinction confirmed."
- **§2.9.6** — rationale prelude refined to acknowledge three-axis distinction; explicitly names trait-method-Input as the consolidation axis (not configuration; not trigger-purpose). The §2.9 axis at the trait-system layer is Method-Input/Output.
- **§2.9.7** — implications refined to note "REJECT (refined twice)" with reference to §2.9.1b three-axis distinction.

**No ADR file edits required.** Q2 is rationale-tightening only; verdict unchanged. Per §15.9.5, Q2 is not a separate amendment-in-place qualifier on the Status header — Q1 is the structural amendment (§2.3 + §2.4 shape change); Q2 refines the rejection rationale without changing the verdict.

---

## Cascade-state changes

| Item | Pre-amendment | Post-amendment | Notes |
|---|---|---|---|
| Tech Spec status | `FROZEN CP4 2026-04-25` | `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 post-freeze)` | Q2 is rationale only |
| ADR-0024 status | accepted 2026-04-20 | accepted 2026-04-20 (unchanged) | Source-of-truth ADR; this amendment realigns Tech Spec with it |
| ADR-0035 status | accepted (amended 2026-04-24-B / 2026-04-24-C) | unchanged | Phantom-shim composition preserved |
| ADR-0038 status | accepted 2026-04-25 | unchanged | Trait shape unchanged at `*Handler` shape level |
| ADR-0039 status | accepted 2026-04-25 (amended-in-place 2026-04-25) | unchanged | Macro emission shape unchanged |
| ADR-0040 status | proposed (pending user ratification on canon §3.5) | unchanged | Out-of-scope for this re-examination |
| Spike `final_shape_v2.rs` | unchanged | unchanged | Spike does not specify `*Handler` shape |
| §2.2 RPITIT signatures | unchanged | unchanged | RPITIT typed surface preserved per ADR-0024 §Decision item 3 ("Native AFIT remains the default for new traits that are not `dyn`-consumed") |
| §3 / §4 / §5 / §6 / §7 / §8 / §9-§13 | unchanged | unchanged | Cross-section impact is §2.3 + §2.4 only |

---

## Process retrospective (for memory update)

**What surfaced through user pushback that the freeze missed:**
- Cross-ADR consistency check was not part of the freeze ratification protocol. ADR-0024 (accepted 2026-04-20) was four days older than the Tech Spec FROZEN CP4 — the ADR set was searchable but not searched against the cascade-internal ADRs (ADR-0035 / ADR-0038 / ADR-0039 / ADR-0040). The Tech Spec's "related ADRs" frontmatter listed only the cascade-internal four; ADR-0024 was the policy-source for the pre-existing workspace decision on `#[async_trait]` for dyn-consumed traits.
- The §2.9.2 table column wording "Input shape" was loosely chosen — it conflated trait-method-input with trigger-purpose-input. CP2 §2.9.1a addressed configuration but not the Output framing the user named at post-freeze. Three-axis naming earlier would have prevented the Q2 re-raise.

**Pattern to encode (architect memory):**
- For Tech Spec freeze ratification on cross-cutting design (async-fn-in-trait, dyn-trait shape, error taxonomy, etc.), include cross-ADR consistency check against workspace-wide ADRs (not only cascade-internal). Specifically: `git log --all docs/adr/ | head -50` + read titles for any ADR touching the same surface as the freeze topic.
- For axis-naming in REJECT verdicts, name the three lifecycle axes explicitly (configuration / trait-method-input/output / lifecycle-purpose-input/output) when triggers / resources / event-driven shapes are in play. Single-axis "Input/Output" framing collapses too much.

---

## Audit hand-off

- **spec-auditor** — please verify (this file plus Tech Spec post-amendment):
  1. §2.3 + §2.4 callout boxes reference §15.9 and ADR-0024 verbatim
  2. §15.9 enactment record cites ADR-0024 §Decision items 1 + 4 with line-pinned references
  3. §2.9.1b three-axis distinction is internally consistent with §2.9.2 trait-method-input table column
  4. CHANGELOG — post-freeze amendment-in-place 2026-04-25 entry at document tail captures both Q1 + Q2 changes
  5. Status header + §0.1 status table CP4 row both carry "(amended-in-place 2026-04-25 — Q1 post-freeze)" qualifier
- **tech-lead** — please re-ratify the FROZEN CP4 with post-freeze Q1 + Q2 amendments. Q1 is a structural amendment (§2.3 + §2.4 signature change); Q2 is rationale-refinement only. Per §15.9.2, the amendment-in-place is proportionate (canonical-form correction across cross-ADR-authoritative source; not a paradigm shift). ADR-0024 is the source-of-truth ADR; this amendment realigns Tech Spec with already-ratified workspace policy.
