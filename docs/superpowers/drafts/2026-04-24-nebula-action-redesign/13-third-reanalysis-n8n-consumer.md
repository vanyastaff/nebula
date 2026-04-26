---
name: Third post-freeze re-analysis — n8n trigger reflection consumer evidence
description: User's third pushback on §2.9 with concrete consumer evidence (70+ n8n trigger nodes) and trait-level vs method-level framing distinction. Outcome: 3C (REJECT-refined-again) with §2.9.7 rationale tightening enacted via amendment-in-place per ADR-0035.
status: closed
type: post-freeze re-analysis
date: 2026-04-25
related: [docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md §2.9, docs/COMPETITIVE.md, docs/adr/0038-action-trait-shape.md]
---

# Third post-freeze re-analysis — n8n consumer evidence + trait/method framing

User raised THIRD post-freeze pushback on §2.9 with two new arguments:
1. **Concrete consumer evidence**: n8n's 70+ trigger nodes (Telegram, GitHub, Schedule) provide concrete UI-generation + downstream type-checking consumer for typed Input/Output reflection.
2. **Trait-level vs method-level framing distinction**: at TRAIT level Input/Output is uniform across 4 variants (configuration schema → event/output payload schema). Method-level divergence is orthogonal.

User asks if `handle()` should be refactored to take `Self::Input` parameter explicitly.

## Q1: trait-level vs method-level distinction analysis

User's framing:
- TRAIT-level Input/Output = associated types for schema reflection / UI generation / downstream type-checking
- METHOD-level signatures = lifecycle-specific (start/stop vs execute vs configure/cleanup) — legitimately diverge

**The framing has surface validity but breaks under structural inspection.** The TRAIT level and METHOD level cannot be cleanly separated for `Input`/`Output` because:

1. **The trait-level associated type IS the method-level signature contributor.** When `StatelessAction::execute(&self, ctx, input: Self::Input) -> Result<Self::Output, _>` declares `type Input` / `type Output`, those associated types ARE referenced in the method signature. You cannot have "trait Input" that is divorced from "method Input" — the method binds the trait type. Per `final_shape_v2.rs:209-218`, `StatelessAction::execute` literally types `input: Self::Input` and returns `Self::Output`. The "trait-level reflection" the user names is a *projection* of the same associated type the method consumes.

2. **The user's proposed third axis collapses to two existing axes.**
   - "TRAIT-level Input as configuration schema" = §2.9.1b configuration axis (universal, lives on `&self` + `parameters = T`).
   - "TRAIT-level Output as event/handle type" = §2.9.1b trigger-purpose axis (lives on `<Self::Source as TriggerSource>::Event` for triggers, on `Self::Output` for the other three).
   - "METHOD-level signatures diverge" = §2.9.2 trait-method-input axis (already documented as the divergence reason).

The user's TRAIT/METHOD distinction is real linguistically, but the §2.9.1b three-axis decomposition already captures the same content with different naming. Rename, not new axis.

3. **The trait-level "uniform" framing only holds if you ignore the projection.** User's claim:
   - StatelessAction: trait Input = execute-input data; trait Output = execute-output data ✓
   - StatefulAction: trait Input = execute-input data; trait Output = execute-output data ✓
   - TriggerAction: trait Input = CONFIGURATION schema; trait Output = EVENT payload schema ✗ (asymmetric — configuration ≠ runtime input)
   - ResourceAction: trait Input = CONFIGURATION schema; trait Output = Resource handle type ✗ (asymmetric — Resource handle is borrowed input, not output)

Stateless/Stateful: Input = runtime parameter, Output = runtime return. TriggerAction in user's framing: Input = registration-time configuration, Output = runtime event. **These are not the same axis.** Stateless/Stateful Input is per-dispatch; Trigger Input (under user's framing) is per-registration. The "uniform at trait level" framing requires conflating two different lifecycles. ResourceAction's "Resource handle as Input" is a third meaning (long-lived borrowed handle, neither registration-config nor per-dispatch parameter).

**Verdict on Q1.** The trait-level vs method-level distinction is linguistically clean but does not produce a uniform "trait Input/Output" axis across the four primaries. It re-surfaces the same three lifecycle axes §2.9.1b already named (trait-method-input vs trigger-purpose vs configuration) under different labels.

## Q2: n8n parity concrete consumer evidence verification

User cites n8n's 70+ trigger nodes (Telegram Trigger: Input = `{allowed_updates, restrict_to_chat_ids}`, Output = `TelegramUpdate`; GitHub Trigger; Schedule Trigger) as concrete consumer for typed Input/Output reflection.

**Two structural problems with this evidence.**

### Q2.1: COMPETITIVE.md disclaims n8n parity as a day-one goal

`docs/COMPETITIVE.md:29` (extracted from PRODUCT_CANON.md §2 / §2.5):

> "Competitive dimension (do not dilute): Reliability and clarity of execution as a system, plus DX for integration authors — not feature parity with n8n/Make on day one, and **not** a surface-area race in v1."

`docs/COMPETITIVE.md:41` (n8n-specific bet):

> "**Our bet:** Typed Rust integration contracts + honest durability beat a large but soft ecosystem; a smaller library of reliable nodes wins over time."

**Nebula's stated competitive position is that n8n's surface-area approach is the LOSING model.** N8n parity for UI generation is not a goal Nebula commits to. The user's "concrete consumer" evidence is a goal Nebula has explicitly disclaimed at canon level.

### Q2.2: Even if the goal were adopted, n8n's reflection is data, not types

N8n is JS/TS at runtime. Its "Input schema" for UI generation is JSON schema as data — there is no compile-time type identity. When n8n's UI engine reads "Telegram Trigger has `allowed_updates` parameter," it reads from a JSON manifest, not from a TypeScript trait associated type.

Nebula's equivalent (data-as-schema):
- `ActionMetadata::with_schema(schema: ValidSchema)` — universal, works for all 4 trait variants.
- For Stateless/Stateful/Resource: `for_stateless::<A>()` derives schema from `A::Input as HasSchema` (compile-time projection through associated type, ergonomic shortcut).
- For TriggerAction: `with_schema(<RSSConfig as HasSchema>::schema())` — direct schema injection (no shortcut helper, but mechanism is identical).

**User's evidence supports schema availability**; it does not force trait associated types. Schema-as-data via `ActionMetadata` is the universal mechanism; trait associated types are the per-trait ergonomic shortcut for the three Input-bearing primaries.

### Q2.3: For the Output side — trigger event payload type-checking

User's "downstream type-checking" framing: connecting Telegram Trigger's output to a Stateless action's input requires type-compatibility check (TelegramUpdate flows into the next node's Input).

**This already exists for TriggerAction without consolidation.** Per `final_shape_v2.rs:255` + Tech Spec §2.2.3, TriggerAction declares `type Source: TriggerSource`, and `TriggerSource::Event` is the typed event payload. Compile-time downstream type-checking — if it were a goal — would project through `<T::Source as TriggerSource>::Event`, not through a hypothetical `T::Output`. The projection IS available; consolidation does not enable it (or block it).

**Verdict on Q2.** N8n parity is a non-goal at canon level (COMPETITIVE.md). Even if it were a goal, n8n's reflection is schema-as-data which Nebula already supports universally via `ActionMetadata::with_schema`. Trigger output type-checking, if pursued, projects through `Source: TriggerSource::Event` — not blocked by the absence of `type Output` on TriggerAction.

## Q3: handle() refactor specific proposal evaluation

User asks: refactor `handle(&self, ctx, event)` to `handle(&self, input: Self::Input, ctx)` with TriggerAction declaring `type Input` (configuration).

**This is structurally wrong for a trigger.** Three reasons:

1. **`handle()` is the per-event dispatch method.** The engine calls `handle()` once per event the source produces (RSS feed item, Kafka record, webhook delivery). The method is invoked many times per registered trigger; configuration is registered ONCE. Threading configuration through a per-event method parameter would force the engine to re-supply the same configuration on every dispatch — purposeless overhead and a semantic confusion (is this dispatched config or static config?).

2. **Configuration is already accessed via `&self`.** Per §2.9.1a + the `tests/execution_integration.rs:155` precedent (NoOpTrigger holds `meta: ActionMetadata`), trigger configuration lives in struct fields. `RSSTrigger { url: String, interval: Duration }` reads `self.url` / `self.interval` inside `handle`. Adding a `Self::Input` parameter that re-supplies the same struct fields would be redundant — the receiver `&self` already carries them.

3. **`handle()`'s parameter is the engine-projected event.** Per `final_shape_v2.rs:260` + Tech Spec §2.2.3 line 209, `event: <Self::Source as TriggerSource>::Event` IS the only meaningful per-dispatch input — what the source produced for THIS dispatch. Renaming `event` to `input: Self::Input` and assigning it to mean configuration would lose the projection (engine has no way to bind `Self::Input` to source events) and lie about what `handle` consumes.

**The current shape is correct.** `handle(&self, ctx, event)` = receiver-carries-config + engine-projected-event-per-dispatch. The user's proposed refactor would introduce semantic confusion without DX benefit.

**Verdict on Q3.** Reject the refactor. Current `handle()` shape per `final_shape_v2.rs:257-261` is structurally correct for trigger lifecycle.

## Q4: §2.9.6 / §2.9.7 point 2 wrongness check

The user targets "no current consumer for typed Input/Output reflection materializes" (this phrase appears in **§2.9.7 line 637**, NOT §2.9.6 — §2.9.6 contains the broader rationale). Re-reading §2.9.7 verbatim:

> "A concrete consumer for typed Input/Output reflection materializes (e.g., a future dependency-typed resource graph that needs to walk the action family by Input/Output type identity). Current §3 / §4 / §7 do not require this."

**Was the phrasing too narrow? Yes — partially.** The §2.9.7 wording binds "consumer" to "compile-time type-identity walking of the action family." User's evidence surfaces a different consumer class: schema-availability for UI generation. That class IS satisfied today (universally, for all 4 traits, via `ActionMetadata::with_schema`), so the §2.9.7 phrasing should distinguish:
- **Schema-as-data consumers** (UI generation, runtime validation, manifest export): EXIST today, satisfied universally by `ActionMetadata::with_schema` (data path) and `<A::Input as HasSchema>::schema()` projection where Input exists (ergonomic-shortcut path). Do not require trait-level Input/Output consolidation.
- **Schema-as-trait-type consumers** (compile-time walking by type identity, e.g., `fn collect<T: Action<I, O>>` aggregating actions by I/O type): SPECULATIVE; no current Tech Spec section requires them.

The §2.9.7 wording conflated these two. User's evidence shows the first class exists; the §2.9.7 rationale should refine to claim "no consumer requiring trait-level type-identity walking" rather than "no consumer for typed reflection."

**The verdict (REJECT) is unchanged** — schema-as-data consumers do not force consolidation; they are satisfied by the universal `with_schema` mechanism. But the §2.9.7 rationale wording should be tightened to acknowledge the data-class consumer exists and is satisfied without consolidation, and to restrict the "no current consumer" claim to the type-identity-class which remains speculative.

This is a rationale-tightening amendment, not a verdict reversal.

## Outcome: 3C (REJECT-refined-again) with §2.9.7 rationale tightening

**REJECT consolidation.** Status quo (Option C) preserved. Rationale tightened across **three** iterations:
- CP2 2026-04-24 per §2.9.1a — Configuration vs Runtime Input axis named.
- Post-freeze 2026-04-25 Q2 per §2.9.1b — three-axis distinction (trait-method-input vs trigger-purpose vs configuration).
- Post-freeze 2026-04-25 Q3 per §2.9.1c (new) — schema-as-data vs schema-as-trait-type consumer distinction.

**New axis named: schema-as-data vs schema-as-trait-type consumers.**

- Schema-as-data consumers (UI generation, validation, manifest export per n8n parity surface): EXIST. Satisfied universally by `ActionMetadata::with_schema` for any trait variant. The `for_stateless::<A>()` etc. helpers project `<A::Input as HasSchema>::schema()` as ergonomic shortcuts where Input exists; the underlying `with_schema` builder is the universal path that works without `type Input`. TriggerAction can supply schema via `ActionMetadata::new(...).with_schema(<RSSConfig as HasSchema>::schema())` — no helper today (§2.9-1 closed at §15.6), but the mechanism is identical.
- Schema-as-trait-type consumers (compile-time walking the action family by `Action<I, O>` type identity): SPECULATIVE. Current §3 / §4 / §7 do not require this. The TriggerAction `<Self::Source as TriggerSource>::Event` projection ALREADY provides compile-time event-type access for any future `T: TriggerAction` consumer; trigger-output type-checking, if pursued, projects through Source.

**Why n8n parity does not unblock consolidation.**

1. **N8n parity is a non-goal at canon level.** `docs/COMPETITIVE.md:29` explicitly disclaims "feature parity with n8n/Make on day one"; line 41 names typed Rust contracts as Nebula's bet AGAINST n8n's surface-area model. User's framing positions n8n parity as required; canon positions it as not-pursued.
2. **Even if n8n parity were a goal, n8n's reflection is data not types.** N8n's UI generation reads JSON schema as runtime data; it does not require compile-time type identity. Nebula's equivalent is `ActionMetadata::with_schema` (universal) and `HasSchema` projection (ergonomic). Both work without trait-level Input/Output consolidation.
3. **TriggerAction's `Source: TriggerSource::Event` projection already provides typed downstream-checking.** `<T::Source as TriggerSource>::Event` is a compile-time-projectable type identity for any `T: TriggerAction`. Adding `type Input` / `type Output` on TriggerAction would either duplicate this (`type Output = <Source as TriggerSource>::Event` redundant) or contradict it (`type Input = X` where X has no method-binding meaning).
4. **User's `handle()` refactor proposal is structurally wrong** (Q3) — configuration is per-registration, not per-dispatch; receiver `&self` is the correct carrier; method parameter `event` IS the per-dispatch input.

## Amendment trail

This re-analysis enacts ONE amendment-in-place to the Tech Spec per ADR-0035 §3:

### Amendment T3-A: §2.9.7 rationale tightening — schema-as-data vs schema-as-trait-type axis (Q3 post-freeze)

**File:** `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` §2.9.7 line 634-638.

**Pre-amendment wording (line 634-638):**

> **Re-open trigger.** This decision is reconsidered if either of the following fires:
> - A fifth primary dispatch trait is proposed that shares the Stateless/Stateful/Resource Input/Output shape (canon §3.5 revision per §0.2). At four of five sharing the shape, the cost-benefit shifts; consolidation may become principled.
> - A concrete consumer for typed Input/Output reflection materializes (e.g., a future dependency-typed resource graph that needs to walk the action family by Input/Output type identity). Current §3 / §4 / §7 do not require this.

**Post-amendment wording:**

> **Re-open trigger.** This decision is reconsidered if either of the following fires:
> - A fifth primary dispatch trait is proposed that shares the Stateless/Stateful/Resource Input/Output shape (canon §3.5 revision per §0.2). At four of five sharing the shape, the cost-benefit shifts; consolidation may become principled.
> - A concrete schema-as-trait-type consumer materializes (e.g., a future dependency-typed resource graph that needs to walk the action family by trait-level `Action<I, O>` type identity, or compile-time `fn collect<T: Action<I, O>>` aggregation). Schema-as-data consumers (UI generation per [`docs/COMPETITIVE.md`](../../COMPETITIVE.md) line 29-41 — explicitly NOT n8n-parity-on-day-one — runtime validation, manifest export) are satisfied today by `ActionMetadata::with_schema` (universal across all 4 traits) and `<A::Input as HasSchema>::schema()` projection where Input exists; they do NOT require trait-level Input/Output consolidation. TriggerAction's `<Self::Source as TriggerSource>::Event` projection already provides compile-time event-type identity for any `T: TriggerAction` consumer (§2.2.3); trigger output type-checking, if pursued, projects through `Source` rather than through hypothetical `type Output`. Current §3 / §4 / §7 do not require schema-as-trait-type walking.

**Plus:** add new §2.9.1c (sibling to §2.9.1a / §2.9.1b) that records this Q3 pushback verbatim and the schema-as-data / schema-as-trait-type axis.

**Plus:** §2.9.5 verdict prefix gains "(refined three times)" instead of "(refined twice)"; §2.9.6 first sentence changes "three-axis" to "four-axis" with the data-vs-type axis added; §2.9.7 Implications wording extended; §2.9.7 Re-open trigger second bullet refined from "concrete consumer for typed Input/Output reflection" → "concrete schema-as-trait-type consumer" with COMPETITIVE.md line 29-41 citation explicitly disclaiming n8n surface parity as a re-open trigger.

**No status header change.** Per §15.9.5 precedent (Q2 post-freeze refinement landed without status qualifier change because it was rationale-tightening only — no §2.2 signature ripple), Q3 follows the same pattern. Status header remains `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 post-freeze)` — Q1 is the only structural amendment (§2.3 + §2.4 `*Handler` shape change); Q2 + Q3 are rationale refinements absorbed into §2.9.1b / §2.9.1c sub-subsections + §15.9.5 / §15.9.6 enactment records.

**Plus:** §15.9.6 Q3 post-freeze refinement record added (sibling to §15.9.5 — same enactment-tracking pattern).

**No ADR-0038 amendment needed.** The decision (REJECT consolidation) is unchanged. ADR-0038 ratified the per-trait shape per `final_shape_v2.rs:209-262`; the Tech Spec §2.9 verdict is downstream-of and consistent-with ADR-0038.

## Cascade-state changes (as enacted 2026-04-25)

- **Tech Spec §2.9.5 / §2.9.6 / §2.9.7** — amendment-in-place (rationale tightening only; verdict unchanged). §2.9.5 decision header to "(refined three times)" + four-axis description. §2.9.6 prelude to four-axis + point 2 to "schema-as-trait-type axis" wording with COMPETITIVE.md citation. §2.9.7 Implications wording extended; Re-open trigger second bullet refined to "schema-as-trait-type consumer" framing.
- **New §2.9.1c** — verbatim Q3 pushback record + schema-as-data vs schema-as-trait-type axis distinction + n8n consumer verification (`INodeTypeDescription` is data not types) + COMPETITIVE.md line 29-41 canon-level disclaim of n8n parity + handle() refactor REJECT (4 reasons).
- **New §15.9.6** — Q3 post-freeze refinement record (sibling to §15.9.5 Q2 record). Documents enactment scope, no-ADR-amendment reasoning, no-signature-ripple confirmation, no-status-qualifier-change rationale per §15.9.5 precedent, amend-in-place vs supersede justification, Phase 8 cascade summary impact (none), §2.9-1 closure reaffirmation.
- **Status header** — UNCHANGED. Stays `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 post-freeze)` per §15.9.5 / §15.9.6 precedent: rationale-tightening amendments without signature ripple do not warrant status qualifier change.
- **ADR-0038** — UNCHANGED. Decision stable; rationale lives in Tech Spec.
- **ADR-0039** — UNCHANGED. §15.5 amendment-in-place preserved; Q3 does not touch macro emission.
- **ADR-0040** — UNCHANGED. Still PROPOSED awaiting user ratification.
- **Strategy** — UNCHANGED.
- **ADR-0035** — UNCHANGED. Phantom-shim composition preserved (the user's `handle(.., input, event)` refactor proposal that would have broken `&self` field-zone rewriting is REJECTED in §2.9.1c).
- **Spike `final_shape_v2.rs:209-262`** — UNCHANGED. Signature-locking source preserved.

## Honesty note

The user's pushback was substantive on TWO of three points:
- **Q1 (trait/method distinction)**: linguistically clean, but collapses to existing §2.9.1b axes. No new axis. PARTIAL credit — surfaced that §2.9.1b's terminology could be clearer.
- **Q2 (n8n consumer evidence)**: invalidated by COMPETITIVE.md canon-level disclaim of n8n parity. NO credit at the goal-validity level. PARTIAL credit at the §2.9.7 wording level — phrase "no current consumer" was too narrow; should distinguish data-class vs type-class consumers.
- **Q3 (handle() refactor)**: structurally wrong — confuses per-registration configuration with per-dispatch input. NO credit.

The §2.9.7 wording fix is genuine and overdue. Two prior re-analyses missed the data-class vs type-class distinction; this one surfaces it. The user's evidence accelerated discovery of a real (if minor) wording gap — credit where due.

The verdict (REJECT) is correct because consolidation cannot honestly accommodate trigger's lifecycle divergence (trait-method-input axis), and schema-as-data consumers are satisfied without consolidation. The rationale tightening is a real refinement of a previously-too-narrow phrase.

## Why not 3A (full ACCEPT)?

User's evidence does not establish the necessary precondition for 3A. ACCEPT consolidation would require either:
- A concrete schema-as-trait-type consumer in this Tech Spec's §3 / §4 / §7. None exists.
- A canon-level commitment to n8n surface-parity. COMPETITIVE.md disclaims it.
- A handle() refactor proposal that survives Q3 structural scrutiny. The proposal does not.

3A would be expedient (closes the user pushback) but not principled (consolidation breaks Trigger lifecycle honesty per §2.9.2 + §2.9.6 invariants the spike `final_shape_v2.rs` validated end-to-end at commit `c8aef6a0`).

## Why not 3B (partial accept — TriggerAction-only Input/Output)?

3B (add `type Input` / `type Output` to TriggerAction only) would:
- Force Trigger to declare `type Input = ()` (configuration is in `&self`, not a per-dispatch parameter) or `type Input = SomeConfig` (which conflates registration-time and runtime axes — same problem Q3 surfaced).
- Force `type Output = ()` (handle returns unit by §2.9.2 invariant) or `type Output = <Source as TriggerSource>::Event` (redundant projection — Source already provides this).
- Add asymmetry to ADR-0038's per-trait shape decision without functional gain.

The "asymmetry between TriggerAction and the other three" the user names is **structural**, not stylistic. Consolidating only Trigger's surface to match the others would require lying about what Trigger actually consumes/produces at the trait level. The honest shape is the current one — three-of-four share Input/Output because they share lifecycle; Trigger diverges because Trigger's lifecycle differs.

## Open items raised this re-analysis

None. All Q-paths resolve cleanly within the existing §2.9 framework with the §2.9.7 rationale tightening.

## Forward-pointing acknowledgement

If a future cascade introduces:
- A `nebula-ui` crate that walks `T: Action<I, O>` for visual schema rendering — that triggers schema-as-trait-type re-evaluation per §2.9.7's revised re-open trigger.
- An ADR proposing canon §3.5 revision to commit Nebula to n8n-style surface parity — that would override COMPETITIVE.md line 29 disclaim and force §2.9 reconsideration on different grounds.

Neither is anticipated in the active cascade. Per `feedback_active_dev_mode.md`, speculative surface area for unrealized consumers is technical debt; the current Tech Spec lands without it.
