---
name: Q4 post-freeze — TriggerAction `type Input` asymmetry (3-vs-1)
status: SINGLE-ROUND DECISION 2026-04-25 (post-freeze refinement; no §2.2 signature change)
date: 2026-04-25
authors: [architect (drafting); user (challenger)]
scope: 14th post-freeze pushback on §2.9 — distinct framing: not consolidation, but adding `type Input` to TriggerAction only, keeping all 4 traits separate
related:
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md §2.2 lines 158-237 + §2.9 lines 484-682
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/13-third-reanalysis-n8n-consumer.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/final_shape_v2.rs lines 209-262
---

# Q4 post-freeze — TriggerAction `type Input` asymmetry analysis

## Verification — Tech Spec §2.2 current state for 4 primaries

Read directly from `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`:

| Primary | Section | Line | `type Input` declared at trait level? |
|---|---|---|---|
| `StatelessAction` | §2.2.1 | 158-159 | **YES** — `type Input: HasSchema + DeserializeOwned + Send + 'static` |
| `StatefulAction` | §2.2.2 | 176-177 | **YES** — `type Input: HasSchema + DeserializeOwned + Send + 'static` |
| `TriggerAction` | §2.2.3 | 202-204 | **NO** — only `type Source: TriggerSource` + `type Error` |
| `ResourceAction` | §2.2.4 | 225-227 | **YES** — `type Input: HasSchema + DeserializeOwned + Send + 'static` |

**Asymmetry is real and 3-vs-1, exactly as user framed.** Three primaries declare `type Input` with the same bound chain at trait level; one (TriggerAction) does not.

## Verification — spike `final_shape_v2.rs` lines 209-262

| Primary | Spike line | `type Input`? |
|---|---|---|
| `StatelessAction` | 209-218 (declared 210) | YES |
| `StatefulAction` | 220-235 (declared 221) | YES |
| `ResourceAction` | 237-252 (declared 239) | YES |
| `TriggerAction` | 254-262 | **NO** — only `type Source` + `type Error` |

Spike confirms 3-vs-1 asymmetry as the shape that compiled end-to-end at commit `c8aef6a0`. Spike-locked since CP1.

## Asymmetry analysis — what user's proposal actually is

**The user's Q4 is a NEW option not analyzed in §2.9.3 (which only covered consolidation Options A/B/C).** This is **Option D — add `type Input` to TriggerAction at trait level, decoupled from method-input, as configuration carrier**:

```rust
pub trait TriggerAction: ActionSlots + Send + Sync + 'static {
    type Source: TriggerSource;
    type Input: HasSchema + DeserializeOwned + Send + 'static;  // NEW — configuration, not method-input
    type Error: ...;

    fn handle<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        event: <Self::Source as TriggerSource>::Event,           // unchanged — method-input is still event
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}
```

**Critical semantic point.** The user's proposal **inverts the meaning of `type Input`** between traits:

| Primary | What `type Input` means | Where it appears in method signature |
|---|---|---|
| `StatelessAction` (existing) | per-dispatch user-supplied data | `execute(&self, ctx, input: Self::Input)` — IS method parameter |
| `StatefulAction` (existing) | per-dispatch user-supplied data | `execute(&self, ctx, &mut state, input: Self::Input)` — IS method parameter |
| `ResourceAction` (existing) | per-dispatch user-supplied data | `execute(&self, ctx, &resource, input: Self::Input)` — IS method parameter |
| `TriggerAction` (proposed Option D) | **per-instance configuration** (set once at registration, read from `&self`) | `handle(&self, ctx, event: <Source as TriggerSource>::Event)` — **NOT method parameter** |

Three primaries: `type Input` IS the method parameter. Proposed for TriggerAction: `type Input` is a configuration carrier with NO appearance in any method signature. **Same syntactic surface, opposite semantics.**

## §2.9 axis re-examination under Option D

§2.9 distinguished four axes through Iter 1-3:
1. trait-method-input axis (where consolidation actually lives)
2. trigger-purpose-input axis (lifecycle layer, not trait method)
3. configuration axis (per-instance, in `&self` fields, declared via `ActionMetadata::parameters`)
4. schema-as-data vs schema-as-trait-type carrier axis (n8n consumers live on data axis)

**Option D introduces a fifth axis: trait-declared configuration carrier (compile-time, type-system-mediated).** The user's framing is that `TelegramTriggerInput { allowed_updates, restrict_to_chat_ids, download_files, image_size }` should be declared at trait level via `type Input = TelegramTriggerInput`, identically (in syntax) to how `StatelessAction::Input` is declared, even though the runtime role differs.

**Schema flow comparison.** User claims schema flows identically:

- StatelessAction: `<A::Input as HasSchema>::schema()` projection produces `ActionMetadata.base.schema`. This works because `A::Input` IS the per-dispatch input the engine deserializes from JSON.
- TriggerAction proposed: `<A::Input as HasSchema>::schema()` would produce `ActionMetadata.base.schema`. This works because the schema reflection mechanism (`HasSchema::schema()`) is type-driven, not lifecycle-driven — it doesn't care whether the Input is per-dispatch or per-instance.

**The schema-flow claim is mechanically correct.** `<T as HasSchema>::schema()` projects regardless of when T is materialized at runtime. So the schema-as-trait-type axis WOULD work for triggers under Option D.

## Why §2.9 prior iterations did NOT cover Option D explicitly

- Iter 1 (CP1): user asked "consolidate Input/Output to base Action" → §2.9.3 Options A/B/C all analyzed CONSOLIDATION. Option D (per-trait, asymmetric, decoupled-from-method) not on the table.
- Iter 2 (Q2 post-freeze): user introduced Configuration vs Runtime Input distinction → §2.9.1b named the configuration axis but framed configuration as living in `&self` fields + `ActionMetadata::parameters`, NOT as a trait associated type.
- Iter 3 (Q3 post-freeze): user cited n8n consumers (UI generation, port-typing, filter validation) → §2.9.1c named the schema-as-data carrier axis. The schema-as-trait-type axis was acknowledged as having no current consumer.

**Option D is genuinely new.** The user's Q4 framing — "type Input as trait-declared configuration carrier with same bounds as Stateless's Input but different runtime role" — is not what Iter 1-3 analyzed.

## Outcome — **(I) REJECT** (refined four times)

## Reason (one sentence)

**Option D forces TriggerAction's `type Input` to mean per-instance configuration while the other three primaries' `type Input` means per-dispatch method-parameter; the syntactic symmetry is cosmetic and the semantic asymmetry would be silent — readers would assume `TriggerAction::Input` threads through `handle()` like Stateless's does, when it does not.**

## Supporting analysis

Four converging blockers:

**B1 — Silent semantic divergence is worse than honest syntactic asymmetry.** Per `docs/STYLE.md` §0 universal mindset: trait surfaces should read as what they are. Three traits' `type Input` is the method parameter (visible in `execute(&self, ctx, input: Self::Input)`). Adding `type Input` to TriggerAction without a corresponding `handle(.., input: Self::Input)` parameter creates a trait surface where `type Input` means something different in the same trait family. A new contributor reading `TelegramTrigger::Input = TelegramTriggerInput` would reasonably assume `handle(.., input: TelegramTriggerInput)` — exactly the trap the existing 3-vs-1 asymmetry prevents by NOT declaring `type Input`. This violates `feedback_active_dev_mode.md` ("more-ideal over more-expedient"): the more-ideal shape is to let the trait read as what it actually does.

**B2 — Configuration is already a universal carrier via `ActionMetadata::parameters` + `&self` fields.** Per §2.9.1a (CP1 lock) and §2.9.6 point 1: configuration lives in `&self` struct fields with schema declared through `ActionMetadata::parameters` via `with_schema(<TelegramTriggerInput as HasSchema>::schema())` — universal across all 4 variants per `crates/action/src/metadata.rs:292`. The user's example structs (`TelegramTriggerInput { allowed_updates, ... }`, `RSSTriggerInput { url, interval }`, `GitHubTriggerInput { repository, events }`) ALL work today as `&self`-field-zone declarations + `parameters = Type` macro zone. Adding `type Input` at trait level adds a parallel declaration site without removing the existing one — the configuration is now declared in two places (associated type + `&self` field), and they must be kept in sync manually. This is signature-doubling, not signature-unification.

**B3 — User's claim "method signatures unchanged" exposes the asymmetry.** The user explicitly says: "Method signatures unchanged — handle() takes event, not Input." This is the load-bearing admission. In the other 3 primaries, declaring `type Input` is justified BECAUSE it appears in the method signature — the trait surface and the method surface are coherent. For TriggerAction, declaring `type Input` while the method signature does NOT carry it means the trait-level declaration is decorative — it exists for schema reflection and nothing else. But schema reflection is already universal via `with_schema(<T as HasSchema>::schema())` per B2. So the trait-level declaration serves no consumer that the schema-as-data path doesn't already serve, while introducing the silent-divergence trap from B1.

**B4 — ADR-0038 `accepted` status binds the four trait shapes verbatim.** Per Tech Spec §0.1 line 35 + ADR-0038 §Decision item 4 + spike `final_shape_v2.rs:209-262` (signature-locking source per §2.9.7 line 674). The four trait shapes that compiled end-to-end at spike commit `c8aef6a0` (Probe 1-6 PASS, Iter-2 §2.2 compose PASS, Iter-2 §2.4 cancellation PASS) are non-consolidated AND asymmetric on `type Input`. Re-validation of Option D would require new spike work to confirm the type system accepts the decoupled-trait-Input pattern AND the macro emission contract per ADR-0039 §1 absorbs it without per-trait branching. This is achievable in principle, but per `feedback_active_dev_mode.md` the cost-benefit demands a current consumer that the existing schema-as-data path does not satisfy. None has surfaced through four iterations.

**Critical: this is a rationale refinement, not a verdict change.** §2.9 verdict (REJECT) stands across all four iterations because the cumulative analysis surfaces five axes (method-input, trigger-purpose, configuration, schema-as-data-vs-schema-as-trait-type carrier, AND now trait-declared-configuration-carrier from Option D), and none enables consolidation OR per-trait `type Input` addition without silent semantic divergence (B1) or signature-doubling (B2).

## What §2.9.6 / §2.9.7 amendment-in-place captures

A minimal rationale amendment to §2.9 names Option D explicitly as the fifth axis (trait-declared-configuration-carrier) and the four blockers above. **No §2.2 signature change.** No ADR amendment (ADR-0038 §Decision item 4 still binds the verbatim shapes from `final_shape_v2.rs:209-262`). Status qualifier on §0.1 line 33 already cites "Q2 §2.9.1b axis-naming refinement per ADR-0035 amended-in-place precedent" — extends naturally to "Q4 §2.9.1d axis-naming refinement" with same precedent.

## Amendment trail (REJECT — rationale refinement only)

Per §15.9 amendment-in-place precedent (Q1 + Q2 + Q3 already established), this is a rationale-only refinement — no signature change, no ADR flip. Steps:

1. **Tech Spec §2.9** — append §2.9.1d subsection naming Option D + the trait-declared-configuration-carrier fifth axis + the four blockers (B1-B4) above.
2. **Tech Spec §2.9.5** — append "post-freeze 2026-04-25 Q4 per §2.9.1d — five-axis distinction adds trait-declared-configuration-carrier axis (Option D rejected on silent-semantic-divergence + signature-doubling)" to the rationale chain.
3. **Tech Spec §2.9.6** — append a sixth rationale point referencing §2.9.1d.
4. **Tech Spec §2.9.7** — append "Q4 post-freeze refinement (§2.9.1d) named the fifth axis — trait-declared-configuration-carrier — and rejected Option D on B1 (silent semantic divergence) + B2 (signature-doubling) + B3 (admitted method-signature unchanged exposes the asymmetry) + B4 (ADR-0038 binds verbatim spike shapes)."
5. **Tech Spec §0.1 line 33** — extend status line: "amended-in-place 2026-04-25 post-freeze for Q1 `*Handler` shape per §15.9 + Q2 §2.9.1b axis-naming refinement + Q3 §2.9.1c schema-carrier-axis refinement + Q4 §2.9.1d configuration-carrier-axis refinement per ADR-0035 amended-in-place precedent."
6. **§17 CHANGELOG** — append "Q4 post-freeze 2026-04-25: §2.9 amended-in-place — Option D (trait-declared `type Input` on TriggerAction only, decoupled from method parameter) rejected on silent-semantic-divergence + signature-doubling + ADR-0038 binding. §2.9.1d added; §2.9.5 / §2.9.6 / §2.9.7 rationale extended; verdict unchanged."
7. **No ADR amendment.** ADR-0038 §Decision item 4 binds the four trait shapes verbatim from `final_shape_v2.rs:209-262`; Option D would be a Tech Spec §2.2 signature change which would invalidate the freeze per §0.2 item 2 + item 4. REJECT preserves both.
8. **No spike re-run.** Spike `final_shape_v2.rs:209-262` remains the signature-locking source unchanged.

## Honest answer to user's asymmetry challenge (one paragraph)

The user identified a real syntactic asymmetry: 3 traits declare `type Input`, 1 does not. Prior iterations addressed CONSOLIDATION (hoisting to base trait) but did not analyze Option D (per-trait `type Input` on TriggerAction only, decoupled from method parameter). Option D is a genuinely new framing. **However, the asymmetry is honest reflection of the underlying semantics, not stylistic noise.** In the three primaries, `type Input` IS the method parameter the engine threads per-dispatch. In TriggerAction, the proposed `type Input` would be configuration set once at registration and read from `&self`, never threaded through `handle()`. Adding `type Input` to TriggerAction would create a trait family where the SAME associated type name carries OPPOSITE semantics (per-dispatch vs per-instance) — a silent divergence trap worse than the visible asymmetry it removes. The schema-reflection consumer the user names (`<A::Input as HasSchema>::schema()` projection for UI generation) is already universal via `with_schema(<T as HasSchema>::schema())` per `crates/action/src/metadata.rs:292` without trait-level declaration. The asymmetry stands as deliberate honest reflection of the lifecycle divergence; Option D removes the syntactic asymmetry only by introducing a deeper semantic asymmetry.

## Summary

**Verdict: I REJECT** (refined four times — Iter 4: Option D analyzed for the first time).

**Single-sentence reason:** Option D forces TriggerAction's `type Input` to mean per-instance configuration while the other three primaries' `type Input` means per-dispatch method-parameter; the syntactic symmetry is cosmetic and the semantic asymmetry would be silent — a worse trap than the visible 3-vs-1 asymmetry it removes.

**Tech Spec amendment:** §2.9.1d subsection naming Option D + four blockers (B1-B4) + extending §2.9.5 / §2.9.6 / §2.9.7 / §0.1 / §17 CHANGELOG per §15.9 precedent. No §2.2 signature change. No ADR amendment.

**Handoff:** if user accepts this rejection, architect can enact §2.9 amendment-in-place per §15.9 precedent. If user contests, single-round budget is exhausted — escalate to tech-lead for ratification.

---

## Q5 Option E — `type Config` (rename to escape semantic divergence)

**User's verbatim Q5 (post-Q4):** «может быть тогда `type Config`?» — proposing the per-instance configuration carrier be named `type Config` to avoid the `type Input` semantic-collision trap from Q4 B1.

### Engagement — does `type Config` resolve Q4 B1?

**Yes. Naming collision is resolved.** A new contributor reading `type Config = TelegramTriggerConfig` does NOT assume `handle(.., config: TelegramTriggerConfig)` because `Config` carries clear "configuration" semantics (per-instance, registration-time) at name-level. The cosmetic-vs-semantic mismatch from Q4 B1 dissolves. **User found the sharpest framing on iteration five.**

### Scope question — E1 (TriggerAction-only) vs E2 (universal across 4 traits)

User's proposal is ambiguous on scope. Two variants:

- **(E1) `type Config` only on TriggerAction.** Resolves visible 3-vs-1 asymmetry (Trigger gets a typed surface). Introduces NEW asymmetry in opposite direction: only Trigger declares Config; other three do not.
- **(E2) `type Config` universally across all 4 traits.** Symmetric outcome. Each trait gets `type Config` (per-instance) AND existing `type Input` where applicable (per-dispatch). Follows logically from §2.9.1a "Configuration is universal across all 4 trait variants."

### Q5 analysis — which Q4 blockers survive `type Config`?

| Blocker | Survives under E1? | Survives under E2? |
|---|---|---|
| **B1 — silent semantic divergence** (same name, opposite meaning) | NO. `type Config` ≠ `type Input` at name level → no collision | NO. Same — different name, different meaning |
| **B2 — signature-doubling** (parallel to `&self` + `parameters = T`) | YES (mitigatable via macro-emission link) | YES (universal: every action declares Config in macro zone AND trait surface) |
| **B3 — decorative (no method-signature carry)** | YES — `type Config` would not appear in `handle()`; only consumer is schema-reflection, already universal via schema-as-data axis | YES — same; consumer (compile-time bounded generic over configuration types) is speculative per §2.9.1c |
| **B4 — ADR-0038 binds verbatim spike shapes** | YES — spike `final_shape_v2.rs:254-262` has no `type Config` on TriggerAction | YES — spike has no `type Config` on any trait |
| **NEW B5 — §2.9.1a paradigm contradiction** | YES — E1 makes Trigger's universal-config carrier the trait associated type while other 3 keep `&self`+macro zone | YES — E2 inverts the universal-config paradigm choice (§2.9.1a Resolution point 1: «Configuration lives in `&self` fields, populated at registration» — chose this over `type Config` deliberately) |

**B1 dissolved. B2-B5 persist.** Naming collision was the load-bearing blocker against Option D; renaming resolves it. The remaining blockers (B2/B3/B4/B5) are weaker but cumulatively still binding.

### B5 in detail — §2.9.1a paradigm contradiction (the new finding under Option E)

§2.9.1a Resolution point 1 (CP2 ratified, line 501) states:

> «Configuration lives in `&self` fields, populated at registration. Per §4.2 ("Fields outside the zones pass through unchanged"), an action struct may declare ordinary fields — `pub url: String`, `pub interval: Duration`, `pub channel: KafkaChannel` — and the `#[action]` macro emits the struct verbatim with credentials/resources zone-injection composed in.»

§2.9.1a Resolution point 2 names the universal mechanism (line 502):

> «Configuration schema flows through `ActionMetadata::parameters` (`ValidSchema`) — universally, across all 4 variants.»

And §2.9.1a closing (line 507) records the **deliberate negative choice**:

> «`final_shape_v2.rs:209-262` does not have a `type Config` on any of the four traits; the spike's PASS is consistent with this resolution. Configuration carrier is `&self`; configuration schema carrier is `ActionMetadata::parameters` via `with_schema`. **No new associated type, no signature edit.**»

**E1 (TriggerAction-only) contradicts this:** Trigger gets `type Config` while the universality of the `&self`+macro-zone carrier is preserved for the other three — splitting the universal carrier across two paradigms.

**E2 (universal) contradicts this MORE STRONGLY:** every trait declares `type Config` AT TRAIT LEVEL while ALSO populating `&self` fields via macro zone — every action now has parallel declaration. The §2.9.1a "no new associated type" choice was deliberate; E2 inverts it.

### B3 in detail — what `type Config` adds beyond `with_schema`

User's framing implicitly asks: what consumer does `type Config` enable that `with_schema(<TelegramTriggerConfig as HasSchema>::schema())` does not? Four candidates evaluated:

- **(a) Engine-side typed deserialization.** Engine erases through JSON by design (§2.9.6 point 2 + §3 / §2.5 `ActionHandler` enum). `type Config` does not pierce JSON erasure; deserialization happens through `serde_json::from_value::<T>(...)` which already works with the `&self` field type. Not a new consumer.
- **(b) Compile-time bounded generic code: `fn validate<T: TriggerAction<Config = SomeBound>>(...)`.** This is a schema-as-trait-type axis consumer per §2.9.1c. Currently zero such consumers in Tech Spec scope. Speculative DX surface per `feedback_active_dev_mode.md`.
- **(c) Schema-as-trait-type axis carrier symmetric with schema-as-data axis.** Symmetric in form, but schema-as-data is already universal and goal-aligned with `docs/COMPETITIVE.md` line 41 (typed-Rust-contracts bet). Adding the parallel trait-type axis surface costs without enabling a new consumer.
- **(d) Compile-time bound `Config: HasSchema + DeserializeOwned`.** Today the `&self` field's type is the locus of these bounds (the `parameters = T` macro zone declares them at the macro layer; the field's type carries them). Hoisting to trait-associated-type adds the bound twice (trait + field), not once.

**No candidate reaches "current consumer that schema-as-data does not satisfy" threshold.** B3 carries.

### Outcome — **E.REJECT** (refined fifth time)

**E.REJECT** for both E1 and E2.

**Single-sentence reason:** Option E (renaming to `type Config`) resolves the Q4 B1 naming-collision blocker — user found the sharpest framing on iteration five — but the remaining blockers (B2 signature-doubling, B3 no compile-time consumer that schema-as-data does not satisfy, B4 ADR-0038 spike-shape binding, **B5 NEW — §2.9.1a "configuration carrier is `&self`+macro-zone, no new associated type" was a deliberate paradigm choice**) cumulatively still preclude ACCEPT under both E1 and E2.

### Honest acknowledgment to user

The user's `type Config` proposal is materially better than Option D. Q4 B1 was the load-bearing blocker — the silent-divergence trap from name-collision. `type Config` dissolves that. **Three of four Q4 blockers (B2/B3/B4) remain at lower force**, and a NEW blocker (B5 — paradigm contradiction with §2.9.1a Resolution point 1's deliberate "no `type Config` on any trait" closing) surfaces under direct re-examination.

This is the FIFTH iteration of essentially the same question with progressively sharper framing. The cumulative analysis now distinguishes **six axes**:

1. trait-method-input axis (where consolidation actually lives — §2.9.2)
2. trigger-purpose-input axis (lifecycle layer, not trait method — §2.9.1b)
3. configuration axis (per-instance, in `&self` fields — §2.9.1a Resolution)
4. schema-as-data vs schema-as-trait-type carrier axis (§2.9.1c)
5. trait-declared-configuration-carrier axis with name-collision (Option D — §2.9.1d)
6. **trait-declared-configuration-carrier axis with rename (Option E — §2.9.1e)**

Under axis 6, the rename resolves the silent-divergence concern but the §2.9.1a paradigm-choice (B5) and ADR-0038 spike-binding (B4) hold.

### Tech Spec amendment-in-place trail (REJECT — rationale refinement only)

Per §15.9 amendment-in-place precedent (Q1 + Q2 + Q3 + Q4 already established):

1. **§2.9** — append §2.9.1e subsection naming Option E (rename to `type Config`) + B1-resolved-but-B2-B3-B4-B5-still-bind cumulative analysis + scope question (E1 vs E2) + the new B5 finding.
2. **§2.9.5** — extend rationale chain: "post-freeze 2026-04-25 Q5 per §2.9.1e — six-axis distinction adds trait-declared-configuration-carrier-with-rename axis (Option E rejected on B5 paradigm contradiction with §2.9.1a + B4 ADR-0038 binding; B1 naming-collision dissolved by rename)."
3. **§2.9.6** — append seventh rationale point referencing §2.9.1e.
4. **§2.9.7** — append "Q5 post-freeze refinement (§2.9.1e) acknowledged user's sharpest-framing-yet `type Config` rename as resolving Q4 B1, but rejected on B5 (NEW — §2.9.1a Resolution point 1's deliberate 'no `type Config` on any trait, configuration carrier is `&self`+macro-zone' paradigm choice) + B4 (ADR-0038 binds verbatim spike shapes from `final_shape_v2.rs:209-262`)."
5. **§0.1 line 33** — extend status line: "...amended-in-place 2026-04-25 post-freeze for Q1 + Q2 §2.9.1b + Q3 §2.9.1c + Q4 §2.9.1d + **Q5 §2.9.1e configuration-carrier-rename refinement** per ADR-0035 amended-in-place precedent."
6. **§17 CHANGELOG** — append "Q5 post-freeze 2026-04-25: §2.9 amended-in-place — Option E (rename to `type Config` to resolve Q4 B1) rejected on B5 (NEW — paradigm contradiction with §2.9.1a Resolution point 1 deliberate 'no `type Config` on any trait' choice) + B4 (ADR-0038 spike binding). §2.9.1e added; §2.9.5 / §2.9.6 / §2.9.7 rationale extended; verdict unchanged. Both E1 (TriggerAction-only) and E2 (universal) rejected on the same blockers."
7. **No ADR amendment.** ADR-0038 binding preserved. ADR-0040 not flipped.
8. **No spike re-run.** Spike `final_shape_v2.rs:209-262` remains the signature-locking source unchanged.

### Summary

**Verdict: E.REJECT** (both E1 and E2 — refined fifth time).

**Honest acknowledgment:** user found the right framing on iteration five — `type Config` resolves Q4 B1 (silent semantic divergence from `type Input` name-collision). The remaining blockers (B2 signature-doubling, B3 no compile-time consumer beyond schema-as-data, B4 ADR-0038 binding) are weaker than B1 was, but a NEW blocker (B5) surfaced: §2.9.1a Resolution point 1 / line 507 closing made an explicit deliberate paradigm choice ("Configuration carrier is `&self`; ... No new associated type, no signature edit"). Adding `type Config` — under either E1 or E2 — inverts that paradigm choice.

**If user contests B5:** that is a §2.9.1a re-litigation, not a §2.9 re-litigation; cycle escalates to tech-lead for ratification of the B5 paradigm-choice question (whether `&self`+macro-zone universality should be inverted to a trait-associated-type universality). Single-round budget for this iteration is exhausted.
