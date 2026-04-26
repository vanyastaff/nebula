# Q8 Phase 2.5 sub-investigation — F6 ItemLineage analysis

**Date:** 2026-04-25
**Context:** User pushback on F6 ItemLineage in Q8 Phase 2 synthesis ("ESCALATE — architect-default β defer to dedicated cascade"). User asks: «depends on requirements + how we could solve it + whether needed + in which cases». This document supplies the analytical foundation for that decision. **Not a cascade revisit; not enacting amendments.**

**Sources:**
- `docs/research/n8n-action-pain-points.md` (lines 25–290 — pairedItem class)
- `docs/research/temporal-peer-research.md` (full — checked for lineage equivalent)
- `docs/research/windmill-peer-research.md` (full — checked for lineage equivalent)
- `docs/research/activepieces-peer-research.md` (full — checked for lineage equivalent)
- `docs/PRODUCT_CANON.md` §4 (pillars), §6 (architecture-pillar map), §8 (what Nebula is not)
- `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` §2.7.2 (`ActionResult` shape — lines 738–795)
- Q8 Phase 2 synthesis §2.5 + §6.2 (escalation framing)

---

## §1 What pairedItem actually solves in n8n

### §1.1 The structural problem

n8n's runtime model is **arrays-flowing-through-nodes**: `execute()` receives `INodeExecutionData[]` (array of items) and returns `INodeExecutionData[][]` (outer = output branch, inner = items in that branch) (`docs/research/n8n-action-pain-points.md:114-117`). Each output `INodeExecutionData` carries a `pairedItem: { item: number } | number[]` field (`:119-121`). The field links each output back to its source input index.

`WorkflowExecute.assignPairedItems` auto-fills only **trivial** cases — 1:1 mapping or a single input. **Anything non-trivial** (Code-node custom JS, aggregation, splitting, sub-workflow calls) is **author-responsibility** (`:120-121`).

When pairedItem is missing or wrong, downstream operations that need provenance fail with: **«Paired item data for item from node X is unavailable»** — one of the most-filed forum errors (`:242-249`, 5 distinct forum threads cited).

### §1.2 What downstream operations need pairedItem

Reading the `pairedItem` issue cluster — what actually breaks when lineage is lost:

1. **`$('NodeName').item` expression resolution** — this is the canonical n8n expression for "give me the item from another node that corresponds to *this* current item." It traverses `pairedItem` chains backward to find the upstream item. Without lineage: `#15981` Merge `$('NodeName').item` fails for non-first inputs; `#24534` Set node loses `.item` context after pin-data; `#27767` AI Agent v3.1 throws `$getPairedItem` error on If-node false branch.

2. **Sub-workflow item correlation** — `#14568` Execute Sub-workflow loses pairedItem.

3. **Merge node item-by-item joining** — Merge "by index" / "by key" mode requires knowing which output items correspond to which input.

4. **Continue-on-error attribution** — `#12558` Continue-on-error loses reference: when item N fails and downstream wants to attribute the failure back to its source.

5. **DB query mode item-tracking** — `#4507` Postgres default query mode breaks pairedItems (when one query produces N result rows from M input rows, lineage between input and result rows is lost).

### §1.3 Why n8n has this problem at all

The root: n8n's data-flow contract is **untyped JS arrays + free-form transformations + per-node author responsibility**. When the author writes Code-node JavaScript that maps/filters/joins items, the runtime cannot infer lineage automatically — only the author knows which output came from which input. So n8n requires the author to set `pairedItem` correctly. Most authors don't. Hence the forum-error class.

---

## §2 Use case enumeration (12 distinct cases)

For each use case: scenario → why lineage matters → criticality.

| # | Scenario | What needs lineage | Criticality |
|---|----------|--------------------|-------------|
| **U1** | **Map (1:1 transformation)** — "for each customer, fetch enriched profile" | Trivial — output[i] obviously came from input[i]. No lineage tracking needed because the engine can infer it from position. | Auto-trivial; n8n already auto-fills this. |
| **U2** | **Filter (N:≤N)** — "for each order, drop if amount < $10" | Output is a subset; user wants `$('orders').item` from a downstream node to find the original order. Without lineage: position[i] in filtered output ≠ position[i] in input. | Trivial if engine emits `original_index` per surviving item. Common case. |
| **U3** | **Aggregation (N:1)** — "count orders per customer" | Output references which inputs were aggregated (e.g., "this row aggregates inputs [0, 3, 7]"). Downstream "drill-into-aggregate" requires this. | Useful for BI / drill-down workflows; not load-bearing for most. |
| **U4** | **Fan-out (1:N)** — "for each invoice, split into line items" | Each output line-item should remember its source invoice. Downstream "attribute back to invoice" or "report which invoice failed" requires it. | **Common in n8n** — `SplitInBatches`, `Item Lists` Split-Out node. Forum errors cluster here. |
| **U5** | **Merge/Join (M+N→K)** — "merge orders with customers by customer_id" | Each merged output references both source items. Two distinct lineage chains per output. | Critical for Merge-node correctness; n8n's #1 churn area (`docs/research/n8n-action-pain-points.md:394-399` — 13+ Merge issues). |
| **U6** | **Sub-workflow item correlation** — "for each input, run sub-workflow X, then join results back" | Result item must reference the input item that triggered the sub-call. | Common; `#14568` is exactly this. |
| **U7** | **Selective retry on partial failure** — "retry only the items that failed" | Engine must know which items failed (lineage from failure → input). | Critical for batch-style actions. Currently solved in n8n by `continueOnFail` + per-item error markers — *but lineage is the substrate that makes retry-N-of-M coherent*. |
| **U8** | **DB-query result row attribution** — "for each input row, run UPDATE; report which UPDATE failed" | Output row → input row lineage. | Common; `#4507` Postgres class. |
| **U9** | **Workflow trace / audit** — "show execution path of item X end-to-end across 12 nodes" | Per-item trace through DAG. | Operator-side debugging; observability concern, not action-author concern. |
| **U10** | **Deduplication across nodes** — "drop duplicate items based on `original_id`" | Item identity preserved across transformations. | Solvable without engine lineage — just preserve `id` field in payload. |
| **U11** | **Cancellation propagation per item** — "if user cancels a batch sub-execution, cancel only items that haven't finished" | Per-item cancellation token, item identity. | Possible but not currently a Nebula semantic. Engine cancels at action-execution granularity, not item-level. |
| **U12** | **Idempotency-per-item** — "this batch has 100 items; if we retry, don't reprocess items 1–47 already completed" | Per-item idempotency key + position-tracking checkpoint. | Currently ADR-0038 §15.12 idempotency hook applies to the *whole action invocation*, not per-item. Per-item idempotency would be a substantial scope expansion. |

**Triage by criticality:**

- **Auto-trivial / zero engine support needed**: U1, U10
- **Genuinely needs engine support to be ergonomic**: U2, U4, U5, U6, U8 (5 cases — all of n8n's forum-error class)
- **Useful but workaroundable**: U3, U9, U7
- **Substantial scope expansion if engine-supported**: U11, U12
- **Pure observability concern (not action-author concern)**: U9

---

## §3 Nebula structural avoidance analysis

Does Nebula's design **avoid the lineage problem** by virtue of its execution model? Or does it inherit it?

### §3.1 Nebula's execution model vs n8n's

n8n: arrays-flowing-through-nodes. Each node's `execute` sees `INodeExecutionData[]` and emits `INodeExecutionData[][]`. **Items are first-class.**

Nebula (per Tech Spec §2.7.2 lines 742–790): `ActionResult<T>` carries an `ActionOutput<T>` (single typed output, not an array). The variants are:
- `Success { output: ActionOutput<T> }` — one output
- `MultiOutput { outputs: HashMap<PortKey, ActionOutput<T>>, main_output: Option<ActionOutput<T>> }` — one output per port (still each is a single typed value)
- `Branch { selected: BranchKey, output: ActionOutput<T>, alternatives: HashMap<BranchKey, ActionOutput<T>> }` — branch routing
- `Route { port: PortKey, data: ActionOutput<T> }` — single port routing

**Items as a first-class concept do not exist in `ActionResult<T>`.** `T` is whatever the action's `Output` associated type is — could be `Vec<Order>`, could be `Order`, could be `()`. The engine does not see "items"; it sees typed payloads.

### §3.2 Per-axis analysis

| Axis | n8n model | Nebula model | Lineage-need in Nebula? |
|------|-----------|--------------|-------------------------|
| Map (1:1) | Author writes loop in `execute`; output array [i] should pair with input array [i] | Action's `Input` is whatever shape (often a single item OR a `Vec<Item>` via the action's typed input). Output is whatever shape. **The "1:1 between items" abstraction is not at the engine level.** | **No engine support needed.** If the action takes `Vec<T>` and returns `Vec<U>`, the *action author* preserves whatever id field they want — the engine has no opinion. |
| Filter (N:≤N) | Same — array-in, smaller-array-out | Same — action takes `Vec<T>`, returns `Vec<T>` (filtered). Author preserves id. | **No engine support needed.** Action-internal concern. |
| Aggregation (N:1) | Code-node aggregates; pairedItem = `[0, 3, 7]` indices | Action takes `Vec<T>`, returns `U` (single aggregate). | **No engine support needed.** If the aggregate wants to reference source items, the result type carries that data. |
| Fan-out (1:N) | One node receives 1 input item, emits N output items, each pointing back via `pairedItem` | If author wants 1-input-N-output behavior at the engine level: today they'd use `MultiOutput` (port-based, not item-based) or return a `Vec<T>` (which downstream is a single typed payload). **The "split one input into N items" pattern doesn't have an engine primitive.** | **No engine support today.** If the author's downstream needs to "process each emitted item independently with its own retry, observability, and cancellation," **today's Nebula doesn't have an item-loop primitive at all.** |
| Merge (M+N→K) | Merge-node receives 2 inputs, emits joined; pairedItem = `[i, j]` | An action takes a custom `Input` that holds `(Vec<A>, Vec<B>)`, returns a `Vec<Joined>`. Engine sees one action, one output. | **No engine support needed if authoring is the boundary.** Nebula doesn't have a "merge by lineage" engine primitive — the author writes the merge logic inside the action. |

### §3.3 The structural insight

**n8n needs `pairedItem` because items are first-class engine objects.** The engine routes items, the engine indexes items, expressions like `$('NodeName').item` resolve through the engine's item store. So when an item moves through 12 nodes, the engine has to know "this output item came from that input item" — otherwise `$('NodeName').item` cannot resolve.

**Nebula does not have first-class items.** The engine routes typed payloads (`ActionOutput<T>`). What `T` contains is opaque to the engine. There is no engine-level expression like `$('NodeName').item` that requires per-item provenance — Nebula's expression system (per ADR-0024 / `nebula-expression`) operates on the typed payload, not on per-item lineage chains.

**Therefore:** in Nebula's current model, **lineage is not a structural need at the engine level.** It would only become an engine concern if Nebula adopts an n8n-style "items flow through nodes" semantic — which would be a profound scope expansion (and not aligned with any current PRODUCT_CANON pillar — see §4 below).

### §3.4 What about the use cases that *do* need lineage (U2, U4, U5, U6, U8)?

In Nebula's typed model:

- **U2 (Filter)**: action takes `Vec<Order>`, returns `Vec<Order>` filtered. If downstream wants `original_index`, the author preserves an `id` field in `Order`. Engine doesn't need to know.
- **U4 (Fan-out)**: today, this is **not an engine pattern** in Nebula. If the user wants per-item independent execution (each item retried independently, each item observed independently), they need an **item-loop primitive** — which Nebula does NOT currently have. The closest equivalent is `BatchAction` (per ADR-0038), which executes items as one batch, not per-item.
- **U5 (Merge)**: author writes merge inside the action.
- **U6 (Sub-workflow correlation)**: today this is an engine primitive (cross-action invocation). The engine could pass a per-call `correlation_id`; that's not the same as `pairedItem` (item-level) — it's invocation-level. **Lighter scope.**
- **U8 (DB row attribution)**: author preserves input row id in the result row.

**Conclusion:** Nebula's typed-payload model **eliminates** the n8n per-item lineage need for U1, U2, U3, U10. For U4 (fan-out per-item independent execution), **Nebula doesn't have the upstream primitive** (item-loop), so lineage is moot until that primitive lands. For U5/U6/U8, lineage is internal to the action body, not engine concern. U7/U11/U12 are scope-expansion territory that also need per-item primitives the engine doesn't currently have.

---

## §4 Implementation options with trade-offs

### §4.1 Mapping to PRODUCT_CANON pillars (gate test)

Per `docs/PRODUCT_CANON.md` §6 line 178: **"Major choices should map to a pillar; if a feature maps to none, it is probably out of scope."**

Pillars are: Throughput, Safety, Keep-alive, DX (§4.1–§4.4). 

- **ItemLineage as engine primitive → Throughput?** No — adds per-item allocation overhead.
- **→ Safety?** Indirectly — closes a forum-error class for *consumers porting from n8n*. Not a Nebula-internal safety class.
- **→ Keep-alive?** No.
- **→ DX?** Yes, but only for users porting from n8n who expect `$('NodeName').item` semantics. **Not a DX win for Nebula-native authoring** (Nebula authors write typed payloads and preserve their own ids).

**Gate result:** lineage maps to "DX for n8n migration" — a real but narrow pillar fit. It does NOT map naturally to the four canonical pillars unless Nebula intentionally targets "n8n migration ergonomics" as a competitive bet.

### §4.2 Options

**Option (a) — Full automatic lineage tracking**

Engine threads per-item lineage automatically through every action. `ActionResult` extended with `lineage: ItemLineage<InputId>` field. Every output item carries provenance back to inputs.

- **Cost:** Per-item allocation; `HashMap` per-action-call growth; serialization cost in execution journal; engine code complexity (lineage-aware adapter for every primary trait).
- **Coverage:** 100% of use cases U2/U4/U5/U6/U8 closed structurally.
- **DX:** invisible to authors *if* engine auto-fills trivial cases AND provides typed helpers for non-trivial cases (the n8n analogue of `assignPairedItems` + helpers).
- **Risk:** Adopts n8n's "items as first-class" model — profound semantic shift. Forces *every* action to be expressible as item-array-in / item-array-out (or opt-out). Forces engine to have an "item identity" concept it currently doesn't have.
- **Pillar fit:** None of canonical four; introduces a fifth pillar implicitly ("data-flow / pipeline").

**Option (b) — Opt-in lineage hook per action**

Action declares it cares about lineage via metadata (e.g., `fn lineage_mode() -> LineageMode { LineageMode::Tracked }` accessor on `ActionMetadata`). Engine threads lineage only for opted-in actions. Actions that don't opt-in see no lineage overhead.

- **Cost:** Lower than (a) — pay only for opted-in surface. Still requires engine-level item-identity primitive.
- **Coverage:** Per-action explicit. Authors must reason about whether their action is in a lineage-tracked subgraph.
- **DX:** Mixed — explicit is good, but "is my action lineage-tracked or not" becomes a workflow-author question (which depends on what its consumers are).
- **Risk:** Same fundamental structural shift as (a) — engine still needs item-identity concept; opt-in just narrows the cost surface.
- **Pillar fit:** Same as (a).

**Option (c) — No engine lineage support; canon-state v1 deferral**

Nebula does not address lineage at the engine level. If users need cross-action item correlation, they preserve their own id fields in typed payloads. Document this as an explicit non-goal (analogous to `docs/PRODUCT_CANON.md:397` "WASM is an explicit non-goal for plugin isolation" pattern).

- **Cost:** Zero engine.
- **Coverage:** Zero engine. Authors solve U2/U5/U6/U8 by carrying ids in their typed payloads (already trivially possible). U4 (fan-out per-item independent execution) is not addressable until a fan-out primitive lands (orthogonal to lineage).
- **DX:** No regression for Nebula-native authors. n8n migrants must restructure (preserve ids in typed payloads instead of relying on `$('NodeName').item`).
- **Risk:** Closes design freedom permanently. If Nebula later targets data-pipeline workloads (analogous to dbt / Airflow / Dagster), lineage may resurface as a pain class — but at that point a dedicated cascade designs it from first principles, not as a tacked-on n8n parity feature.
- **Pillar fit:** Honest deferral; aligns with §6 "if it maps to no pillar, it is probably out of scope."

**Option (d) — Hybrid: minimal lineage primitive, no auto-enrichment**

Nebula provides a `CorrelationId` type that actions can opt into carrying through their typed payloads (a thin newtype + serialization helpers). Engine does NOT auto-thread; engine does NOT enrich `ActionResult`. The crate `nebula-action` ships authoring helpers (`fn carry_correlation<I, O>(input: I, output: O) -> O` etc.).

- **Cost:** Tiny — a newtype + a few helpers. No engine changes; no `ActionResult` extension.
- **Coverage:** Author-driven; engine-agnostic. Closes U2/U5/U6/U8 ergonomically (helper makes preserving ids one-line). Does NOT close U4/U7/U11/U12 (those need primitives lineage-tracking is not the right substrate for).
- **DX:** Author-explicit — they know they're carrying lineage; they know when to use the helper.
- **Risk:** Helper-as-discoverability — authors who don't know the helper exists won't use it. Mitigation: include in scaffolding template + `dx-tester` validates first-day-author finds it.
- **Pillar fit:** **DX pillar** — fits cleanly. No new engine concept; just an authoring helper for a common pattern.

### §4.3 Peer comparison

- **Temporal:** No `pairedItem`-equivalent. Activity is the call-unit; activity attempts have history but there is no per-item lineage primitive (`docs/research/temporal-peer-research.md:81-89,287-289` confirms — Search Attributes are workflow-level, not item-level). Temporal's data-flow is "activity returns a typed value; downstream uses it." This is what option (c) / (d) match.
- **Windmill:** Zero hits for "lineage" / "pairedItem" / "provenance" in `docs/research/windmill-peer-research.md`. Windmill scripts return values; there is no per-item lineage primitive. Matches option (c) / (d).
- **Activepieces:** Zero hits for lineage in `docs/research/activepieces-peer-research.md`. Same shape.

**Only n8n has a per-item lineage primitive — and it's their #2 pain class.** This is a strong signal that lineage as engine primitive is expensive and brittle, not a competitive must-have.

---

## §5 Recommendation + rationale

### §5.1 Recommendation

**Pick option (c) — Canon-state Nebula v1 does NOT commit to per-item lineage at the engine level.**

Optionally enrich with **option (d) light layer** in a future cascade (NOT current cascade): ship `CorrelationId` newtype + authoring helpers in `nebula-action` if and when authors surface explicit pain. **Do not include in current cascade scope** (would require Strategy §3.4 row + spike + ADR amendment, all for a feature that fits no canonical pillar today).

### §5.2 Rationale

1. **Structural avoidance is real.** Nebula's typed-payload `ActionResult<T>` model **eliminates** the n8n-class lineage problem for U1/U2/U3/U10 (4 of 12 use cases). Authors carry their own ids in typed payloads — same pattern as Temporal, Windmill, Activepieces. The forum-error class is n8n-specific because *items* are first-class engine objects in n8n; in Nebula they are not.

2. **Pillar fit is weak.** Per `docs/PRODUCT_CANON.md` §6: "if a feature maps to no pillar, it is probably out of scope." Lineage maps to "DX for n8n migration" — a narrow secondary concern, not a canonical pillar. Ratifying ItemLineage now would set the precedent that "n8n parity" is a Nebula pillar — and Nebula's competitive position is explicitly *not* "n8n with Rust" (it's typed-durability vs script-glue per `docs/PRODUCT_CANON.md:55, 505`).

3. **n8n's pain is self-inflicted.** Their lineage class exists because untyped JS arrays + author-responsibility model + free-form transformations make automatic lineage impossible. Nebula's typed Rust model makes the *original problem* not exist — lineage is solved by `Order { id: u64, ... }` + author preservation, not by an engine primitive.

4. **Use cases that genuinely need engine support (U4 fan-out, U7 selective retry, U11 per-item cancel, U12 per-item idempotency) require an item-loop primitive Nebula doesn't have.** Lineage is a *consequence* of having item-loops, not an independently useful primitive. Designing lineage before the item-loop primitive is putting the cart before the horse — when (if) Nebula adds an item-loop primitive, lineage falls out naturally as part of that design.

5. **Peer evidence is unambiguous.** Three of four peer engines (Temporal, Windmill, Activepieces) have no lineage primitive and no equivalent forum-error class. The fourth (n8n) has it and it's a top-5 pain class. This is the signal.

6. **Active-dev mode discipline (`feedback_active_dev_mode.md`).** Architect-default position per Q8 §6.2 was (β) defer to dedicated cascade. After this analysis, the more-honest position is (γ) canon-state explicit non-goal: **Nebula v1 has no per-item lineage primitive at the engine level; if a future cascade introduces fan-out / streaming / item-loop primitives, lineage is designed as part of THAT cascade, not retrofitted.** Naming the absence is more honest than promising a future cascade slot for a feature that may never need to exist.

### §5.3 When option (c) makes sense vs when it doesn't

**Option (c) holds IF:**
- Nebula's competitive position remains "typed durability vs script-glue" (per PRODUCT_CANON §2.5 / §6).
- Nebula does not add an item-loop / fan-out / streaming primitive in v1.
- Nebula does not target n8n-migration as a strategic axis.
- Authors are expected to carry ids in their typed payloads (one-line discipline).

**Option (c) breaks DOWN IF:**
- Nebula adds fan-out (1:N) with per-item independent observability, retry, and cancellation. At that point, item identity becomes an engine concept, and lineage is part of that primitive's design.
- Nebula adds an n8n-style expression `$('NodeName').item` semantic. This would force per-item tracking. **Currently Nebula does not have this** — `nebula-expression` operates on typed payloads.
- Nebula explicitly targets n8n-migration ergonomics as a pillar (would require canon revision).

### §5.4 Concrete delineation — where Nebula draws the line

**Nebula commits to (canon-stable, option c):**
- Typed `ActionResult<T>` — the engine routes opaque typed payloads.
- Authors preserve any cross-action correlation via fields in their typed payloads (`Order { id, ... }`).
- No engine-level item identity, item store, or `$('NodeName').item` resolution.

**Nebula explicitly does NOT commit to (option c gate):**
- Per-item lineage tracking through actions.
- Per-item retry / cancellation / idempotency at the engine level.
- n8n-style expression semantics that resolve through item-position chains.

**Future cascade slot (named but unscheduled):** "Item-loop / fan-out primitive cascade" — IF Nebula later decides to add fan-out (1:N independent-item execution), lineage is designed as part of that cascade from first principles. This is NOT the same as "ItemLineage cascade slot" — it is broader (item-loop is the primitive; lineage is a consequence).

---

## §6 What this means for current cascade

**Current cascade scope (architect framing, NOT decision):**

| Path | What it requires | Honest? |
|------|------------------|---------|
| **Architect-default REVISED to (c)** — canon-state explicit non-goal | Tech Spec gains §15.13 row "ItemLineage explicitly out of scope per canon §4.5 / §6 pillar gate; option (c) per Q8 Phase 2.5"; CASCADE_LOG.md gains "Item-loop / fan-out primitive" cascade slot (not "ItemLineage cascade slot") | **Most honest.** Closes the design question; documents the rationale; preserves freedom for future fan-out primitive cascade. |
| **(β) Defer to dedicated cascade** (Q8 Phase 2 §6.2 architect-default) | CASCADE_LOG.md gains "Item lineage / data-flow tracking cascade" slot | Honest, but commits Nebula to revisiting lineage as a *standalone* concern, when in fact lineage only matters as a consequence of fan-out / item-loop primitives. **Risks framing the future problem incorrectly.** |
| **(α) Add to cascade** (Q8 Phase 2 §6.2 path α) | Strategy §3.4 row + spike + Tech Spec amendment | **Not recommended** — see §5.2 reasons 2/4/5. Premature; spends spike budget on a feature with no canonical pillar fit. |

**Architect updated default after this analysis: option (c) per §5.4.** The Q8 Phase 2 §6.2 (β) framing was conservative; the deeper analysis surfaces that (β) implicitly accepts the n8n framing of the problem (lineage as standalone concern). (c) names the structural insight: Nebula avoids the class by virtue of its typed-payload model; lineage only resurfaces if an item-loop primitive is added; designing lineage before item-loop is premature.

---

## §7 Summary table — answers to user's four questions

| User question | Answer |
|---------------|--------|
| **What does pairedItem solve?** | n8n's per-item provenance for `$('NodeName').item` expressions, sub-workflow correlation, Merge-by-index/key, continue-on-error attribution, DB-query row attribution. **5 use cases (U2/U4/U5/U6/U8) are load-bearing in n8n.** |
| **How could we solve it?** | Four options: (a) full auto, (b) opt-in, (c) canon non-goal, (d) lightweight `CorrelationId` newtype helper. Recommend (c) primary, (d) future-optional. |
| **Do we need it?** | **No, not at the engine level.** Nebula's typed `ActionResult<T>` model structurally avoids n8n's class. 3 of 4 peer engines (Temporal, Windmill, Activepieces) have no lineage primitive and no equivalent pain class. |
| **In which cases?** | If Nebula later adds an item-loop / fan-out (1:N independent execution) primitive, lineage is designed as part of THAT cascade — not as a standalone retrofit. Until then, authors preserve ids in their typed payloads (one-line discipline; pattern matches Temporal / Windmill / Activepieces). |

---

## §8 Open items raised by this analysis

- **§8.1** — If user accepts (c) revised default, CASCADE_LOG.md gets "Item-loop / fan-out primitive cascade" slot (broader than "ItemLineage" — captures the actual primitive whose absence matters). Decision: user.
- **§8.2** — Tech Spec §15.13 row form: "ItemLineage explicitly out of scope per canon pillar gate" — this would be amendment-in-place per ADR-0035 precedent. Whether to enact in current cascade or defer to next post-closure cycle: user decides.
- **§8.3** — `CorrelationId` newtype helper (option d light layer) — not recommended for current cascade; framed as future-optional. If ever enacted, it's a nebula-action authoring helper, not an engine concept.
- **§8.4** — Q8 Phase 2 §6.2 architect-default (β) is **superseded by this analysis to (c)**. If user accepts, the §6.2 framing in q8-phase2-synthesis.md needs an addendum noting the supersession.

---

**Architect summary (≤200 words):**

Recommendation: **option (c)** — Nebula v1 has no per-item lineage primitive at the engine level. Optionally augment with option (d) light `CorrelationId` newtype helper in a future cascade only if author pain surfaces.

Use case count: **12 enumerated** (§2). Of these, **5 (U2/U4/U5/U6/U8) genuinely need engine support in n8n** but are **structurally absorbed by Nebula's typed `ActionResult<T>` model** (authors carry ids in their typed payloads — same pattern as Temporal / Windmill / Activepieces, three of four peer engines).

1-line rationale: **n8n's lineage class exists because items are first-class engine objects in n8n; in Nebula they are not — the engine routes opaque typed payloads, so lineage is solved by author-side `id` fields, not by an engine primitive.**

Does Nebula need this? **No** — at the engine level. Pillar fit is weak (no canonical pillar match per `docs/PRODUCT_CANON.md` §6). Three of four peer engines have neither lineage primitive nor equivalent pain class — the signal is unambiguous.

If Nebula ever adds a fan-out / item-loop primitive (post-v1), lineage falls out as part of THAT cascade's design from first principles. Naming the absence (canon-state explicit non-goal) is more honest than promising a future "ItemLineage cascade" for a feature that may never need to exist independent of an item-loop primitive.

**Architect-default position revised from Q8 Phase 2 §6.2 (β defer) to (c canon non-goal)** based on this deeper analysis. User decides whether to ratify the revised default and how to enact (current cascade amendment vs post-closure cycle vs decline).
