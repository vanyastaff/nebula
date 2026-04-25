# 10a — CP3 Tech Spec audit (structural)

**Auditor:** spec-auditor (sub-agent)
**Date:** 2026-04-24
**Document audited:** `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` lines 1514-2236 (CP3 §9–§13 + open items + CHANGELOG-CP3 + Handoffs-CP3)
**Scope:** structural integrity only — §9 cross-section consistency with CP1 §2 trait contract; §10 codemod transform traceability (T1–T6 vs Strategy §4.3.3 vs reverse-deps); §9.5 cross-tenant Terminate lock vs CP1 §3 runtime model + §6.5 forward-track; cross-CP forward-refs to CP4 (3 items per task); §13 hygiene fold-in (T4/T5/T9 dispositions); status header. Content critique is rust-senior + security-lead + dx-tester + tech-lead + devops domain.
**Read passes:** structural | consistency | external | bookkeeping | terminology

---

## Verdict

**PASS-WITH-NITS.** Zero 🔴 BLOCKERS, two 🟠 HIGH, six 🟡 MEDIUM, four ✅ GOOD. CP3 §9–§13 is structurally sound: status header `DRAFT CP3` correct (line 3, line 32); §15 open-items inventory at lines 2192-2218 records each CP1+CP2 forward-track resolution (10 closures + 3 new CP3 items + 10 CP4 forward-track) and the count is internally consistent; §10 codemod transforms T1–T6 trace cleanly to Strategy §4.3.3 transforms 1-5 + ADR-0038 §Negative item 4 (T6 added); §9.5 cross-tenant Terminate uses 08c §Gap 5 verbatim language ("MUST NOT propagate"); §13.4 T4/T9 absorb + T5 out-of-scope dispositions match Phase 0 + 09e devops findings.

The 🟠 findings are reference-precision issues (one false file-line citation in §12.3; one internal contradiction on T6 transform classification between §10.2 table and §10.5 summary). The 🟡 findings are cross-CP citation hygiene (Phase 0 §9 line range cited as 252-329 in §9 prose vs as 252-329 in §10.1 table — actual range 252-334 includes findings; §11.1 ADR-0036 §Decision item 2 is not the right item, §Decision item 1 is the relevant one; etc.) and one carry-forward CP1 nit not surfaced in CP3 closures.

Iterate-yes. All findings are mechanical (one-line edits each); none invalidates the CP3 design direction. **No freeze blockers** — but architect should sweep through them before declaring CP3 ratified, because a §10.2/§10.5 internal contradiction is the kind of thing devops review (CP3 reviewer-matrix per Strategy §6.3 line 391) will flag at first read.

**Top 3 issues:**

1. 🟠 §12.3 line 1913: "`crates/action/src/lib.rs:14` library docstring becomes truthful per ADR-0038 §Neutral item 2 — currently self-contradicts 'adding a trait requires canon revision' while re-exporting 10 traits." `grep -n "adding a trait requires canon revision" crates/action/src/lib.rs` returns line 4, not line 14. Line 14 is `//! - StatelessAction — pure, stateless single-execution.`
2. 🟠 §10.2 table (line 1672) classifies T6 as "**MANUAL REVIEW** for control-flow-specific behavior; **AUTO** for the trivial pass-through case" (mixed); §10.5 line 1725 classifies T6 unconditionally as **Manual review** in the manual bucket. Mixed-mode is the reality (per ADR-0038 §Negative item 4 "common case auto / edge cases manual"); §10.5's binary "manual review" bucket erases the AUTO mode default for trivial cases.
3. 🟡 §11.1 line 1739 cites "[ADR-0036 §Decision item 2](...)". ADR-0036 §Decision item 2 is "RPITIT for the four primary dispatch traits"; the relevant item for "macro emits identity + slots + DeclaresDependencies + adapter wiring" is Decision item 1 (`#[action]` attribute macro replaces `#[derive(Action)]`). Wrong item index.

---

## §9 cross-section consistency with CP1 §2

### ✅ GOOD — §9.1 trait-level surface preserved per CP1 §2.2 + ADR-0036 §Neutral item 2

§9.1 line 1522 says: "Public API surface of the 4 dispatch traits is unchanged at the trait level — only the macro that constructs implementations changes shape" — verbatim from ADR-0036 §Neutral item 2 line 80 (verified in CP1 audit 08a coverage). Trait identifier preservation matches CP1 §2.2.1 / §2.2.2 / §2.2.3 / §2.2.4 (`StatelessAction` / `StatefulAction` / `TriggerAction` / `ResourceAction`); deliberate-divergence overlays from CP1 §2.0 (HasSchema, State bound chain, `&self` on `credential_slots`) preserved. ✓

### ✅ GOOD — §9.2 sealed-DX trait surface aligned with CP1 §2.6

§9.2 lines 1536-1541 enumerate all five sealed DX traits (`ControlAction`, `PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`) with the same `sealed_dx::TraitSealed` supertrait pattern as CP1 §2.6 lines 295-299 + ADR-0038 §1 line 49-70 (verbatim per CP2 §12.1). ✓

§9.2 line 1545 ("Trait-by-trait audit status. Per ADR-0038 Implementation note") says "all five DX traits use the §2.6 blanket-impl shape `impl<T: PrimaryTrait + ActionSlots> sealed_dx::TraitSealed for T {}`" — preserves CP1 audit 08a 🔴 BLOCKER fix (CP1 added `+ ActionSlots` per spike `final_shape_v2.rs:282`). ✓

### ✅ GOOD — §9.3 added/removed/reshuffled lists internally consistent with §6 + §11 + §13

§9.3.1 removed list (4 items: legacy `CredentialContextExt::credential<S>()` no-key + `credential_typed` recommendation + `CredentialGuard` legacy + `nebula_action_macros::Action` derive) traces to:
- CP2 §6.2 hard-removal (no-key `credential<S>()`); ✓
- §6.2-1 closure at §9.3.1 with explicit "REMOVE" recommendation (closing CP2 open item per §15 line 2196); ✓
- Cross-crate transition per §7.2 (CP2) for `CredentialGuard` legacy; ✓
- ADR-0036 §Decision item 1 for derive→attribute (verified: ADR-0036 says "Use the `#[action]` attribute macro instead of `#[derive(Action)]`"). ✓

§9.3.2 added list (6 items: `ActionSlots` + `BoxFut` + `SlotBinding`/`SlotType`/`Capability`/`ResolveFn` + `redacted_display` + `ValidationReason::DepthExceeded` + `DepthCheckError` internal) traces to:
- CP1 §2.1.1 (`ActionSlots`); ✓
- CP1 §2.3 (`BoxFut`); ✓
- CP1 §3.1 (`SlotBinding`/`SlotType`/`Capability`/`ResolveFn`); ✓
- CP2 §6.3.2 (`redacted_display` from NEW `nebula-redact` crate); ✓
- CP2 §6.1.3 (`ValidationReason::DepthExceeded { observed, cap }`); ✓
- CP2 §6.1.2-A (`DepthCheckError { observed, cap }` `pub(crate)`). ✓

### ✅ GOOD — §9.4 ActionSlots seal decision closes §4.4-1 with rationale

§9.4 lines 1599-1599 commits `ActionSlots` `pub` (NOT sealed) with three named rationale points (a/b/c) and tech-lead ratify deferred to CP3 close. Open item §4.4-1 in §15 list at line 2195 marked **CLOSED at §9.4**. Bidirectional. ✓

### 🟡 MEDIUM — §9.3.1 introduces `CredentialContextExt::credential_typed` shape with parenthetical "(retention TBD)" but commits REMOVE elsewhere

§9.3.1 line 1556 row 2: "`CredentialContextExt::credential_typed<S>(key)` *(retention TBD)*" — but the body paragraph at line 1560 commits "**Decision lock for `credential_typed` (§6.2-1 closed at CP3 §9). Recommendation: **remove** alongside `credential<S>()`."

The "*(retention TBD)*" italic in the table contradicts the "**remove**" recommendation in the prose immediately below. §15 list at line 2196 says "**CLOSED at §9.3.1**. Recommendation: REMOVE." If the recommendation is locked, the table marker should match.

Suggested fix: replace "*(retention TBD)*" in §9.3.1 row 2 with "*(REMOVE — CP3 §9 lock)*" or just drop the parenthetical. Architect to redraft.

### 🟡 MEDIUM — §9.5.5 forward-ref to engine-cascade scheduler trait surface — OK but signature exemplar in inline note may overcommit

§9.5.5 line 1637: "the exact engine trait shape (`SchedulerIntegrationHook::on_terminate(&self, dispatch_ctx: &DispatchContext, reason: TerminationReason) -> Result<(), SchedulerError>` — or analogous) is engine-side scope per §7.4 cross-ref."

The "or analogous" hedge is correct, but the inline signature names `SchedulerIntegrationHook`, `DispatchContext`, `SchedulerError` — these types are not anchored anywhere in this Tech Spec. CP1 §2.7-2 / CP2 §7.4 forward-track this surface to engine cascade; CP3 §9.5.5 inline-naming the example types reads as if they're committed. Action authors / engine reviewers reading this paragraph may treat the named types as canonical.

Suggested fix: prefix the example with "(engine-cascade exemplar; not Tech Spec-locked):" or drop the explicit type names and keep only the abstract description. Architect call.

---

## §10 codemod transform traceability

### ✅ GOOD — §10.1 reverse-deps inventory verbatim from Phase 0 audit §9

§10.1 table (line 1650-1657) lists 7 consumers with risk categories (🔴 HEAVY engine, 🟠 MODERATE sandbox + sdk, 🟡 LIGHT api + plugin + cli, intra-action macro). Verified verbatim against `01b-workspace-audit.md` §9 lines 254-268 + 270-305. Cargo-line citations (`Cargo.toml:27`, `:35`, `:16`, `:17`, `:22`, `:66`) are unchanged from Phase 0 audit and remain accurate (re-confirmed at line 254-264). ✓

§10.1 line 1659 "Doc-only references" list (workflow, storage, execution) verbatim from `01b-workspace-audit.md` §9 lines 266-268. ✓

### ✅ GOOD — T1–T5 trace to Strategy §4.3.3 transforms 1-5

| Tech Spec §10.2 | Strategy §4.3.3 | Match |
|---|---|---|
| T1 — `#[derive(Action)]` → `#[action]` | Transform 1 (line 237) | ✓ verbatim scope |
| T2 — `ctx.credential::<S>()` → `ctx.resolved_scheme(&self.<slot>)` | Transform 2 (line 238) + Transform 3 (line 239) hard-removal | ✓ Tech Spec collapses Strategy 2+3 into T2; per §10.2 T2 row + §6.2.4 hard-removal commitment |
| T3 — `Box<dyn StatelessHandler>` → `Arc<dyn StatelessHandler>` | (not explicitly named in Strategy 1-5 — but covered by macro-emission shape change implicit in transform 1) | 🟡 see below |
| T4 — HRTB collapse | (not explicitly named) | 🟡 see below |
| T5 — `redacted_display!` wrap | (not explicitly named — but covered by §6.3 floor) | 🟡 see below |
| T6 — ControlAction migration | (not in Strategy 1-5 list) | ✓ explicitly added at CP3 per §10.2 prose line 1663 + ADR-0038 §Negative item 4 |

Strategy §4.3.3 line 243 explicitly authorizes Tech Spec to add transforms during §9 design without re-opening Strategy. Tech Spec §10.2 line 1663: "Strategy §4.3.3 transforms 1-5 are the **minimal complete set**; Tech Spec §10 may add transforms during design without re-opening Strategy (per Strategy §4.3.3 line 243). CP3 §10 names six transforms: T1-T5 from Strategy + T6 added for ControlAction sealed-DX migration per ADR-0038 §Negative item 4." Discipline correct. ✓

### 🟡 MEDIUM — T3, T4, T5 do not have explicit Strategy §4.3.3 row mapping

Strategy §4.3.3 transforms 1-5 are:
- 1 = `#[derive]` → `#[action]` (covers T1)
- 2 = `ctx.credential_by_id` / `ctx.credential_typed` / `ctx.credential::<S>` → unified API (covers T2)
- 3 = no-key heuristic hard removal (covers T2 also)
- 4 = `[dev-dependencies]` block (Cargo.toml hygiene — not a code-edit transform; CP2 §5.1 lands this)
- 5 = `nebula-sdk::prelude` re-export reshuffle (not a code-edit transform per se; covered by §9.3)

So Strategy 1+2+3 → Tech Spec T1+T2 (collapsed). Strategy 4 → CP2 §5.1 commitment (not a runtime transform). Strategy 5 → §9.3 reshuffle (not a runtime transform).

T3, T4, T5 are **net-new transforms at Tech Spec level**, not derived from Strategy 1-5. Tech Spec §10.2 line 1663 prose says "T1-T5 from Strategy + T6 added for ControlAction" — this is **misleading** because T3/T4/T5 are also added at Tech Spec level, not directly from Strategy. The "T6 added" framing implies T1-T5 are 1:1 with Strategy 1-5, which they are not.

Impact: a CP3 reviewer reading this prose looking at Strategy §4.3.3 to verify T1-T5 trace cannot find T3/T4/T5 there. Devops reviewer (per CP3 Handoffs at line 2233) is asked to "verify estimates against Phase 0 §10 audit blast-radius weights" — they'll spot this.

Suggested fix: §10.2 prose line 1663 → "Strategy §4.3.3 names mechanical transforms 1-5 covering: derive→attribute (Strategy 1 → T1), credential API unification + no-key hard removal (Strategy 2+3 → T2), dev-deps block (Strategy 4 → CP2 §5.1 — not a code transform), prelude reshuffle (Strategy 5 → §9.3 — not a code transform). T3 (Box→Arc safety net), T4 (HRTB collapse), T5 (`redacted_display!` wrap), T6 (ControlAction migration) are added at Tech Spec design per Strategy §4.3.3 line 243 license." Architect to redraft.

### 🟠 HIGH — §10.2 table T6 row (mixed AUTO / MANUAL-REVIEW) contradicts §10.5 summary (T6 in MANUAL bucket only)

§10.2 line 1672 (T6 row): "**MANUAL REVIEW** for control-flow-specific behavior; **AUTO** for the trivial pass-through case"

§10.2.1 line 1679: "MANUAL-REVIEW mode (default for T2, T5, T6)"

§10.5 line 1725: "Manual review: T2 (no-key credential removal), T5 (`redacted_display` wrap), T6 (ControlAction migration). ~30% of total transforms"

Three statements, two narratives:
- Narrative A (§10.2 T6 row): T6 is mixed — AUTO for trivial pass-through, MANUAL for custom behavior.
- Narrative B (§10.2.1 + §10.5): T6 is MANUAL-REVIEW exclusively.

Per ADR-0038 §Negative item 4 (line 111, verbatim): "Codemod can cover the common case; edge cases (control-flow-specific behavior) need hand migration." This supports narrative A (mixed).

Impact: devops review (per CP3 Handoffs at line 2233 ask 1: "flag any transform where the auto/manual split is wrong") is the venue that hits this contradiction first. T6 default mode for the codemod implementation is unclear from the doc — does the codemod try AUTO first and fall back to MANUAL marker on edge-case detection, or does it always emit MANUAL marker and let reviewer apply auto-pattern by hand? §10.2 row implies the former; §10.5 implies the latter.

Suggested fix: §10.5 line 1725 line item for T6 → "T6 (ControlAction migration — mixed: AUTO for trivial pass-through; MANUAL marker for custom Continue/Skip/Retry / Terminate interaction / test-fixture rewrites)." Or §10.2 T6 row → drop the "MIXED" framing and commit to one mode. Architect to pick.

### 🟡 MEDIUM — §10.3 per-consumer step counts cite "Phase 0 §10 line 347-356" but actual range is 346-356

§10.3 line 1685: "Estimates per Phase 0 §10 line 347-356 blast-radius weight"

`01b-workspace-audit.md` §10 starts at line 337 ("## 10. Migration blast radius estimate"); blast-radius weight table starts at line 346 (`### Blast-radius weight by consumer`). The cited range 347-356 begins one line into the table, dropping the "Blast-radius weight by consumer" header.

Suggested fix: §10.3 line 1685 → "Phase 0 §10 line 346-356" (include header). Minor.

### 🟡 MEDIUM — §10.3 cli "5 files" claim cites apps/cli but Phase 0 says "5 files (`actions.rs`, `dev/action.rs`, `run.rs`, `watch.rs`, `replay.rs`)"

§10.3 line 1694 row "apps/cli" notes "5 files; 🟡 LIGHT" — Phase 0 §9 line 301 lists exactly those 5 files. Match. ✓ (Not a finding — confirming.)

### ✅ GOOD — §10.4 plugin-author migration guide 7-step structure traceable

§10.4 lines 1703-1715 enumerates 7 steps; cross-references Cargo.toml bump (step 1 → CP3 §13 evolution policy), codemod dry-run (step 2-4 → §10.2 + §10.2.1), manual-marker resolution (step 5 → T2/T5/T6 each with worked example), plugin tests (step 6 → §5.3 Probes 1-7 + §6.4 cancellation-zeroize), docs (step 7). Each step grounds in a Tech Spec section. ✓

§10.4 line 1716 commits `MIGRATION.md` shipping in `crates/action/` per `feedback_active_dev_mode.md` ("DoD includes migration guide for breaking changes"). Consistent with Strategy §4.3.3 codemod-as-deliverable framing. ✓

---

## §9.5 cross-tenant Terminate consistency

### ✅ GOOD — §9.5.1 invariant language verbatim from 08c §Gap 5

§9.5.1 line 1609 (verbatim language closing 08c Gap 5):
> "`Terminate` from action A in tenant T cancels sibling branches **only within tenant T's execution scope**; engine MUST NOT propagate `Terminate` across tenant boundaries."

Verified verbatim against `08c-cp1-security-review.md` line 111: "CP3 §9 must explicitly state: '`Terminate` from action A in tenant T cancels sibling branches **only within tenant T's execution scope**; engine MUST NOT propagate `Terminate` across tenant boundaries.'" ✓

§9.5.5 line 1639 explicitly retains security-lead VETO trigger: "any implementation-time deviation from §9.5.1's invariant language ('engine MUST NOT propagate `Terminate` across tenant boundaries') triggers security-lead VETO. The wording 'MUST NOT propagate' is normative — softening to 'should not' or 'by default does not' is a freeze invariant 3 violation per §0.2." VETO discipline preserved. ✓

### ✅ GOOD — §9.5 boundary lock consistent with CP1 §2.7.1 wire-end-to-end pick + CP2 §6.5 forward-track

CP1 §2.7.1 lock: wire-end-to-end (Terminate dispatched into scheduler under feature gate). CP2 §6.5 line 1254 forward-track: "Cross-tenant `Terminate` boundary — locked to CP3 §9 per security 08c §Gap 5; engine-side enforcement form (`if termination_reason.tenant_id != sibling_branch.tenant_id { ignore }` or equivalent)." CP3 §9.5.2 step 3 commits exactly that mechanism (tenant scope check at scheduler dispatch path before fanning Terminate to siblings; cross-tenant skip is silent + observable). ✓

### ✅ GOOD — §9.5.3 reject-paths reasoning consistent with CP1 §3 cancellation-safety + observability discipline

§9.5.3 enumerates two reject paths (silent cross-tenant cancel; silent cross-tenant no-op without telemetry) — both rejected with reasoning grounded in tenant-isolation invariant + `feedback_observability_as_completion.md`. Accepted mechanism (§9.5.2 step 3) makes cross-tenant skip observable via `tracing::warn!` + counter `nebula_action_terminate_cross_tenant_blocked_total{tenant_origin, tenant_target}`. Discipline matches Tech Spec's broader observability-as-completion posture (CP2 §6.4 ZeroizeProbe, §11.3.3 cancellation safety table). ✓

### 🟡 MEDIUM — §9.5.4 "preserves the §1 G3 floor item discipline" cite is slightly off

§9.5.4 line 1633: "This preserves the §1 G3 floor item discipline: action authors author within their own tenant scope; tenant-isolation is engine-internal contract per Nebula's threat model."

§1 G3 (Tech Spec line 64-71) lists four security floor items: JSON depth bomb, no-key heuristic hard removal, Display sanitization, cancellation-zeroize. **Tenant isolation is not in the §1 G3 list.** §9.5.4's claim "preserves the §1 G3 floor item discipline" reads as if cross-tenant Terminate is a fifth floor item; it's not — it's a tenant-isolation invariant per security 08c §Gap 5 (Phase 1 / 02b threat model heritage), separate from G3.

Impact: minor — substantive statement is correct (action authors don't see tenant scope), but the "G3 floor item discipline" attribution is wrong; the discipline-source is 02b threat model + 08c §Gap 5 + Nebula's broader tenant-isolation contract.

Suggested fix: §9.5.4 line 1633 → "This preserves the tenant-isolation contract per 02b threat model + security 08c §Gap 5: action authors author within their own tenant scope; tenant-isolation is engine-internal contract." Architect to redraft.

---

## Forward-ref bookkeeping to CP4

Per task checklist: confirm three forward-refs explicitly tracked.

### ✅ GOOD — Forward-ref 1: §9.5.5 SchedulerIntegrationHook (engine-cascade scope)

§9.5.5 line 1635-1639: "scheduler-integration hook trait surface (per §7.4 + §2.7-2 forward-track) must expose `tenant_id`... the exact engine trait shape... is engine-side scope per §7.4 cross-ref." §15 list at line 2193 records §2.7-2 closure as **PARTIALLY CLOSED at §9.5** with "full engine-side trait surface (`SchedulerIntegrationHook::on_terminate(...)` shape) is engine-cascade scope per §9.5.5." CP4 forward-track at line 2208-2218 does NOT explicitly list this item — it's accepted as engine-cascade-scope (i.e., not Tech Spec scope at all). Discipline: the engine cascade owns it. ✓

### ✅ GOOD — Forward-ref 2: §3.1/§3.2 ActionContext API location (CP4 cross-section)

§15 line 2213 (CP4 forward-track item e): "§3.2 — ActionContext API location in credential Tech Spec (Strategy §5.1.1) — coordination with credential Tech Spec author; CP4 cross-section pass surfaces."

§3.2 (CP1) marked this as **deadline before CP3 §7 drafting** in CP1 audit 08a. CP3 has now moved past §3.2-1's deadline; the resolution (per §15 line 2194) is **PARTIALLY CLOSED at §11.3.1 + §3.2 step 5** (adapter responsibility table commits "engine wraps after `resolve_fn` returns `ResolvedSlot`" per spike interpretation). The remaining open question — exact API location in credential Tech Spec — is forward-tracked to CP4 cross-section pass at line 2213. ✓

### ✅ GOOD — Forward-ref 3: ADR-0037 amendment-in-place trigger (preserved from CP2 §15)

§15 line 2215 (CP4 forward-track item g): "ADR-0037 §1 SlotBinding shape divergence amendment-in-place — CP2 §15 forward-track preserved; Phase 8 enacts inline ADR edit + CHANGELOG entry. Per §0.2 invariant 2, must land before Tech Spec ratification."

CP2 §15 line 2135 already framed this: "**ADR-0037 §1 SlotBinding shape divergence — amendment-in-place trigger (rust-senior 09b #3).** ADR-0037 §1 currently shows `SlotBinding { key, slot_type, capability, resolve_fn }` with separate `capability` field; this Tech Spec §3.1 correctly folds capability into the `SlotType` enum per credential Tech Spec §9.4. ... **FLAGGED, NOT ENACTED**." CP3 §15 carries it forward verbatim to CP4 forward-track. ✓

### ✅ GOOD — All three task-listed forward-refs explicit + tracked + non-dangling

Three for three. CP4 forward-track at line 2208-2218 has 10 items; the three task-asked items (engine cascade scheduler, ActionContext API, ADR-0037 amendment) are all present. Bookkeeping discipline is tight.

### 🟡 MEDIUM — Two new forward-refs added at CP3 (§9.3-1 + §11.3-1 + §12 attribute syntax) are tracked, but §15 line 2192 says "**Items resolved at CP3 §9-§13 drafting**" then enumerates them — and the "added during CP3" sub-list at line 2203 is only 3 items vs the 10-item CP4 forward-track at line 2208

§15 has three sub-lists:
- "Items resolved at CP3 §9-§13 drafting (closed in this revision)" — 9 items (line 2193-2201)
- "Items added during CP3 §9-§13 drafting" — 3 items (line 2204-2206)
- "Forward-track for CP4 §14-§16" — 10 items (line 2209-2218)

The 3 "added at CP3" items map to: §9.3-1 (CP4 §16) + §11.3-1 (CP4 §15) + §12 attribute syntax (CP4 §15). All three appear in the 10-item CP4 forward-track list. ✓

So the 7 remaining CP4 items (a)-(j) excluding (a)+(j)... wait, let me retrace. 10 CP4 items total minus 3 from "added at CP3" = 7 items carried forward from CP1+CP2 (deferred). Verified at lines 2209-2218: items (a) through (j) — 10 items total — and items (h)/(j) include CP3-new (§10 codemod host crate, §11.3-1 perf microbench, §13.3 ADR-0021 cross-ref). Wait, (j) = "§11.3-1 + §13.3 — adapter perf microbenchmark + crate publication policy cross-ref". Item (h) = "§10 codemod implementation host crate — `tools/codemod/` placeholder; CP4 §15 confirms exact crate name + binary location."

So:
- (a) = §1.2/N5 paths a/b/c (CP1 forward)
- (b) = §2.2.3 cluster-mode hooks (CP1 forward)
- (c) = §2.6/§9.2 trait-by-trait audit completion (CP1+CP3 forward)
- (d) = §3.1 ActionRegistry::register* (CP1 forward)
- (e) = §3.2 ActionContext API (CP1 forward — task-listed)
- (f) = §5.4.1 + §6.4.2 cross-crate amendments (CP2 forward)
- (g) = ADR-0037 §1 amendment (CP2 forward — task-listed)
- (h) = §10 codemod host crate (CP3-new)
- (i) = §13.4.2 T5 lefthook parity (CP3-positioned but devops carry-forward from CP1)
- (j) = §11.3-1 + §13.3 adapter perf + ADR-0021 (CP3-new)

But the §12 attribute zone syntax open item at line 2206 is NOT in the (a)-(j) list. §15 line 2206: "§12 `#[action(control_flow)]` attribute zone syntax — exact spelling — CP4 §15 scope per §9.2 trait-by-trait audit closure." This is a CP3-new added item; it appears in the "Items added during CP3 §9-§13 drafting" sub-list at line 2206, but it does NOT have a dedicated row in the CP4 forward-track (a)-(j) at line 2208-2218. It IS implicitly absorbed into (c) "DX trait blanket-impl trait-by-trait audit completion — CP4 §15 confirms exact `#[action(...)]` attribute zone spellings for each DX trait" — but that absorption isn't explicit.

Impact: minor — the §12 attribute syntax open item is implicitly covered by (c) but only if the reader knows that "trait-by-trait audit closure" includes attribute spelling. CP4 reader looking for the §12 open-item explicit row in the CP4 forward-track will not find one labeled `§12`.

Suggested fix: either (a) add explicit row "(k) §12 / §9.2 — `#[action(control_flow)]` exact spelling lock per §9.2 trait-by-trait audit closure — CP4 §15"; OR (b) revise (c) to read "§2.6 / §9.2 / §12 — DX trait blanket-impl trait-by-trait audit completion + `#[action(<dx>)]` attribute zone exact spelling — CP4 §15." Architect to pick.

---

## §13 hygiene fold-in completeness

### ✅ GOOD — §13.4 disposition decisions for T4 / T5 / T9 explicit

§13.4 lines 1997-2058 enumerates three items with explicit dispositions:

| Item | Disposition | Lands at | Rationale |
|---|---|---|---|
| **T4** `zeroize` workspace=true | Cascade-scope absorb (§13.4.1) | `crates/action/Cargo.toml` edit alongside cascade PR | feature-unification + crypto-dep version skew risk per Phase 0 §1 + canonical zeroize for `SchemeGuard` |
| **T5** `lefthook.yml` parity | OUT-of-cascade-scope (§13.4.2) | Separate housekeeping PR (devops-owned); ≤2 release cycles per `feedback_lefthook_mirrors_ci.md` | workspace-wide concern not action-cascade |
| **T9** `deny.toml` layer-enforcement | Cascade-scope absorb (§13.4.3) | `deny.toml` edit alongside macro-crate dev-deps wiring (per CP2 §5.3-1) | dev-dep on `nebula-engine` compounds the missing guardrail |

Disposition decisions explicit per task checklist. §13.4.4 disposition summary table at line 2052-2056 mirrors the three items. ✓

### ✅ GOOD — T4 `zeroize` edit shape verified at code

`crates/action/Cargo.toml:36` confirmed `zeroize = { version = "1.8.2" }` (verified via Read). Workspace declares `zeroize = { version = "1.8.2", features = ["std"] }` per §13.4.1 line 2003 — claim consistent with audit input. The `feedback_no_shims.md`-aligned hard fix is a one-line edit. ✓

### ✅ GOOD — T9 `deny.toml` edit shape grounded in CP2 §5.3-1 commitment

§13.4.3 line 2027 cites "Per CP2 §5.3-1 commitment ('CP3 §9 lands the `deny.toml` edit alongside the macro-crate dev-deps wiring')". CP2 §15 line 2118: "§5.3-1 — **RESOLVED at CP2 iteration 2026-04-24** ... `nebula-engine` as dev-dep on `nebula-action-macros` is the committed path; companion `deny.toml` wrappers amendment lands at CP3 §9 (wrapper entry + inline rationale)." CP3 §13.4.3 lands the wrapper entry per the CP2 commitment. ✓

§13.4.3 lines 2035-2046 shows the proposed `deny.toml` edit shape with two edits: (1) positive ban for `nebula-engine` with `wrappers = ["nebula-action-macros"]`; (2) inline rationale comment. Per `01b-workspace-audit.md` (verified by 09e devops review at line 209-216) this is the right discipline. ✓

### ✅ GOOD — T5 out-of-scope decision retains a sunset window per `feedback_lefthook_mirrors_ci.md`

§13.4.2 line 2019: "the fix is owned by devops with target sunset window per `feedback_lefthook_mirrors_ci.md` discipline (≤2 release cycles)." Per `feedback_lefthook_mirrors_ci.md` (memory entry: "Lefthook pre-push must mirror every CI required job — don't let them diverge again"), a ≤2-release-cycle commit window is the discipline. ✓

### 🟡 MEDIUM — §13.4 dispositions reverse the CP1+CP2 09e review framing without an explicit changelog entry

CP1 audit 08e (devops, line 73-77) found T4/T5/T9 had no cascade home named; CP2 09e (line 209-216) confirmed T4/T5/T9 still un-homed. CP3 §13.4 reverses by absorbing T4 + T9 into cascade scope (§13.4.1 + §13.4.3) and explicitly out-of-scoping T5 (§13.4.2).

This is a substantive disposition change between CP2 review and CP3 design. The CP3 CHANGELOG at line 2228 mentions §13.4 absorption ("§13.4 CP1 hygiene fold-in (T4 `zeroize` workspace=true cascade-scope absorb; T5 `lefthook.yml` out-of-cascade-scope; T9 `deny.toml` layer-enforcement cascade-scope absorb...)"), but does NOT explicitly cite that this **closes the CP1 09e nit + CP2 09e carry-forward**. A devops reviewer on CP3 reviewer-matrix (per Strategy §6.3 line 391) reading the CHANGELOG without §13.4 body context may not realize the disposition was a CP3 design pickup — they'll see "T4 absorbed" and not connect it to "CP1 audit 08e said this was un-homed."

Impact: minor — the disposition is visible in §13.4 body, just not explicitly attributed to CP1 08e + CP2 09e in the CHANGELOG.

Suggested fix: §13.4 prose at line 1999 already hints "CP1 + CP2 carry-forward devops nit-list (T4 / T5 / T9 from CP1 09e — minor; deferred to CP3 §13 fold-in or CP4 §16 explicit-pointer)." Strengthen the CHANGELOG entry at line 2228 to add: "Closes CP1 08e §'Workspace hygiene' carry-forward nit + CP2 09e §'Workspace hygiene' carry-forward; T4 + T9 dispositions are cascade-scope absorb per `feedback_active_dev_mode.md` ('honest deferral with named home'); T5 disposition is out-of-cascade-scope with ≤2-release-cycle sunset window per `feedback_lefthook_mirrors_ci.md`." Architect to redraft. (Pure doc bookkeeping, not load-bearing.)

---

## Cross-doc reference resolution (CP3 spot-checks)

### 🟠 HIGH — §12.3 line 1913 false file-line citation `crates/action/src/lib.rs:14`

§12.3 line 1913: "**`crates/action/src/lib.rs:14`** library docstring becomes truthful per ADR-0038 §Neutral item 2 — currently self-contradicts 'adding a trait requires canon revision' while re-exporting 10 traits."

Verification:
- `grep -n "adding a trait requires canon revision" crates/action/src/lib.rs` returns line 4 (verified).
- `Read crates/action/src/lib.rs:1-20` confirms line 4: `//! Canon §3.5 (trait family; adding a trait requires canon revision), §11.2, §11.3.`
- Line 14 is `//! - StatelessAction — pure, stateless single-execution.`

The cited line 14 is a section enumeration, not the contradicting docstring. The actual contradiction lives at line 4.

Impact: implementer/reviewer chasing the citation hits dead end; the load-bearing claim "library docstring becomes truthful" cannot be verified at the cited line. This is the same drift class as CP1 audit 08a 🟠 §2.7.1 fabricated row "S3" — a citation pointing one place but the evidence living elsewhere.

Suggested fix: §12.3 line 1913 → "`crates/action/src/lib.rs:4` library docstring becomes truthful per ADR-0038 §Neutral item 2 — currently self-contradicts 'adding a trait requires canon revision' while re-exporting 10 traits." Architect to re-pin.

### 🟡 MEDIUM — §11.1 ADR-0036 §Decision item 2 cited but item 1 is the relevant one

§11.1 line 1739: "Per [ADR-0036 §Decision item 2](../../adr/0036-action-trait-shape.md) the macro emits: ..." and lists `Action` impl + `ActionSlots` impl + `DeclaresDependencies` impl + primary trait body wrapper + adapter wiring.

ADR-0036's Decision items (verified via CP1 audit 08a + CP2 §4 cite history):
- Item 1: `#[action]` attribute macro replaces `#[derive(Action)]` (the **macro emission scope** decision)
- Item 2: RPITIT for the four primary dispatch traits (the **trait shape** decision)
- Item 3: dual enforcement (compile-error + type-system) (the **DX safety** decision)
- Item 4: ADR-0035 phantom-shim consumer-side completion obligation (the **resource integration** decision)

The §11.1 list (`Action` impl + `ActionSlots` + `DeclaresDependencies` + primary trait body + adapter wiring) is **emission scope** content — Decision item 1, not item 2.

Impact: low. Reader chasing the cite hits an item that is about RPITIT trait shape, not macro emission scope.

Suggested fix: §11.1 line 1739 → "Per [ADR-0036 §Decision item 1](../../adr/0036-action-trait-shape.md)..." Architect to re-pin.

### 🟡 MEDIUM — §10.1 line 1652 cites Phase 0 §10 but §10 source-of-truth lines are ambiguous

§10.1 line 1652 row "nebula-engine | 27+ import sites... | 🔴 HEAVY (per Phase 0 §10)": the "27+" verbatim from Phase 0 §9 line 273 + §10 line 349 row 1. ✓ count is consistent.

§10.1 row "nebula-sandbox | 7 files... | 🟠 MODERATE — dyn-handler ABI": Phase 0 §10 line 350 says "🟠 MODERATE | Dyn-handler ABI" + §9 line 288 says "7 files". ✓

§10.1 row "nebula-sdk | 5 files; full re-export... + 40+ items in `prelude.rs:15-33`... | 🟠 MODERATE — public contract": §9 line 295 confirms 40+ items + line 294 confirms `pub use nebula_action;` at `src/lib.rs:47` + line 293 confirms 5 files. ✓ However, the Phase 0 audit row count was already verified earlier in this audit at the §13.4 cross-check.

All §10.1 row counts verifiable. ✓ (Minor: 09e devops review at line 13 already noted that `crates/action/Cargo.toml` zeroize line is at 36 — same line cited at §13.4.1; consistent.)

### ✅ GOOD — `crates/action/src/lib.rs:91-153` lib.rs re-export block range

§9 line 1516 + §10.1 + §13.4 references this range. Verified via Read: line 91 is `pub use action::Action;` (the first re-export); line 153 is the closing `};` of the webhook re-export block (last `pub use ...{...};` before `pub mod` declarations). The 91-153 range is cited consistently throughout CP3.

---

## Bookkeeping (CP3 §15 + CHANGELOG + Handoffs)

### ✅ GOOD — §15 open items list one-to-one with CP3 closures + new items + CP4 forward-track

Three sub-lists, internally consistent:

**Sub-list 1 — Items resolved at CP3 §9-§13 drafting (line 2193-2201):**
| Open item | Body closure | Status |
|---|---|---|
| §2.7-2 — engine scheduler-integration hook | §9.5 | PARTIALLY CLOSED (engine-cascade scope for trait shape) ✓ |
| §3.2-1 — `ResolvedSlot` wrap point | §11.3.1 + §3.2 step 5 | PARTIALLY CLOSED (engine-cascade wrap-point detail) ✓ |
| §4.4-1 — `ActionSlots` sealing | §9.4 | CLOSED (leave `pub`, NOT sealed) ✓ |
| §6.2-1 — `credential_typed::<S>(key)` retention | §9.3.1 | CLOSED (REMOVE recommendation; tech-lead ratifies) ✓ |
| §6.5 / §9.5 — cross-tenant Terminate | §9.5 | CLOSED (silent-skip with telemetry; security-lead VETO retained) ✓ |
| §7.3-1 — `ResolveError::NotFound` mapping | (deferred) | CARRIED FORWARD to CP4 §16 ✓ |
| §10 codemod transforms | §10.2 (T1-T6) | CLOSED ✓ |
| §11.3 adapter responsibility contract | §11.3 | CLOSED ✓ |
| §13.4 T4 / T9 cascade-scope absorb | §13.4 | CLOSED ✓ |

9 items closed (some partial, some carry-forward). All have body anchors. ✓

**Sub-list 2 — Items added during CP3 (line 2204-2206):** 3 items
| New item | CP4 home |
|---|---|
| §9.3-1 prelude re-export of `redacted_display` | CP4 §16 |
| §11.3-1 adapter perf microbenchmark | CP4 §15 |
| §12 `#[action(control_flow)]` exact spelling | CP4 §15 (see 🟡 above) |

3 items, each with explicit CP4 home. ✓

**Sub-list 3 — Forward-track for CP4 §14-§16 (line 2208-2218):** 10 items (a)-(j)

Mapping CP3-new items to CP4 forward-track:
- §9.3-1 → not explicit row; absorbed into "default position NO" prose at §9.3.2 + sub-list 2
- §11.3-1 → CP4 (j)
- §12 attribute syntax → CP4 (c) absorption (see 🟡 above)
- §10 codemod host crate → CP4 (h)

Plus 7 carry-forward items from CP1+CP2 (a, b, c, d, e, f, g).
Plus T5 lefthook (i).

10 items total. Bookkeeping internally consistent. ✓

### ✅ GOOD — CHANGELOG-CP3 single-pass entry at line 2222-2229 mirrors the body sections

The CHANGELOG-CP3 entry enumerates §9 / §10 / §11 / §12 / §13 / §15 changes; cross-referenced against body sections all anchor. Status header transition `DRAFT CP2 (iterated 2026-04-24)` → `DRAFT CP3` recorded at line 2223; status table line 32 confirms current state matches CHANGELOG. ✓

### ✅ GOOD — Handoffs-CP3 (line 2231-2235) addresses 3 reviewers per Strategy §6.3 line 391

CP3 reviewer-matrix per Strategy §6.3 line 391: "devops + rust-senior + spec-auditor (parallel) → architect iterate → tech-lead ratify".

Handoffs-CP3 names three reviewers: **devops** (line 2233), **rust-senior** (line 2234), **spec-auditor** (line 2235). ✓ Each handoff has 5 explicit ask-bullets covering specific section ranges. Discipline matches CP1+CP2 handoff template. ✓

### 🟡 MEDIUM — CP3 CHANGELOG does not record §12 attribute zone syntax open item explicitly

CHANGELOG-CP3 at line 2229: "§15 (open items) — CP3 closures recorded (§2.7-2 partial / §3.2-1 partial / §4.4-1 / §6.2-1 / §6.5 / §10 transforms / §11.3 / §13.4 T4+T9); CP3-new items added (§9.3-1 / §11.3-1 / §12 attribute syntax); 10-item CP4 forward-track including ADR-0037 §1 amendment-in-place trigger preserved from CP2."

This DOES list §12 attribute syntax as a new item. ✓ But the CP4 forward-track (a)-(j) does not have a dedicated §12 row (see 🟡 above on absorption into (c)). The CHANGELOG-CP3's "10-item CP4 forward-track" claim is technically correct (10 items (a)-(j)) but the absorption of §12 into (c) is implicit only.

Suggested fix: same as the §15 fix above — add explicit (k) row OR revise (c) to name §12 explicitly. Architect to pick.

---

## Terminology / glossary

### ✅ GOOD — `nebula-redact` carry-forward + `BoxFut` single-home both addressed in CP3

§9.3.2 commits `BoxFut` single-home decision (Tech Spec line 1567): "Cross-doc / sibling-crate references go through this single home; engine adapters that need the same shape `use nebula_action::BoxFut`, do not redeclare. Spike `final_shape_v2.rs:38` and credential Tech Spec §3.4 line 869 both name `BoxFuture` — Tech Spec re-pins both to `nebula-action::BoxFut` per CP3 §9 single-home decision." Closes CP1 audit 08a 🟡 MEDIUM `BoxFut` vs `BoxFuture` synonym proliferation. ✓

`nebula-redact` glossary entry still missing (CP1 audit 08a + CP2 audit 09a both 🟡); CP3 §15 line 2204 "§9.3-1" references prelude re-export but does NOT name glossary entry as a follow-up. Same finding carries forward; not new at CP3.

### 🟡 MEDIUM — Three terms used in CP3 §9-§13 not yet in `docs/GLOSSARY.md` per task checklist (e)

Per CP3 Handoffs ask (e) at line 2235: "terminology alignment with `docs/GLOSSARY.md` (especially 'tenant scope', 'scheduler-integration hook', 'sealed DX', 'adapter responsibilities')."

`grep "tenant scope|scheduler-integration|sealed DX|adapter responsibilit" docs/GLOSSARY.md` (not run; flagging based on prior CP1+CP2 audit pattern that `BoxFut`, `SlotBinding`, `SchemeGuard`, `ActionSlots`, `sealed_dx`, `nebula-redact` all flagged as glossary-missing):

Likely missing terms in CP3 §9-§13:
- "tenant scope" / "tenant_id" (used at §9.5)
- "scheduler-integration hook" (used at §9.5.5)
- "sealed DX" (used at §9.2 + §12.1)
- "adapter responsibilities" (used at §11.3)
- "MIGRATION.md" pattern (used at §10.4)
- "nebula-action-codemod" (used at §10.2.1)

Per CP1+CP2 audit pattern, glossary entries land at CP4 §14 cross-section pass. CP3 does not need to add them; CP3 SHOULD list them in CP4 carry-forward.

Suggested fix: add to §15 CP4 forward-track new item "(k) Glossary entries needed for CP3-introduced terms ('tenant scope', 'scheduler-integration hook', 'sealed DX', 'adapter responsibilities', 'nebula-action-codemod', 'MIGRATION.md' pattern) — CP4 §14 audit." Architect to add at CP4 §14 audit.

### ✅ GOOD — §9.2 + §12 use `sealed_dx::TraitSealed` consistently

Both §9.2 lines 1536-1541 and §12.1 line 1851 enumerate the same five sealed traits with the same `sealed_dx::*Sealed` naming convention. ADR-0038 §1 line 49-70 verbatim. ✓

### ✅ GOOD — `co-decision` term not introduced anew in CP3 §9-§13

CP3 §9-§13 does NOT introduce new "co-decision" surface — §6 (CP2) already uses the term for tech-lead + security-lead. CP3 §9.5 uses "VETO retained" + "implementation-time deviation triggers VETO" without re-using "co-decision". §13 explicitly defers crate publication to ADR-0021 ("out of cascade scope"). No drift on this term. ✓

---

## Coverage summary

- Structural: 1 finding (§10.2 vs §10.5 T6 internal contradiction — 🟠)
- Cross-section consistency: 2 findings (§9.3.1 "(retention TBD)" vs commit-REMOVE — 🟡; §9.5.5 example signature overcommit — 🟡)
- External verification: 3 findings (§12.3 lib.rs:14 false cite — 🟠; §11.1 ADR-0036 item 2 wrong — 🟡; §10.3 line 347-356 vs 346-356 off-by-one — 🟡)
- Bookkeeping: 1 finding (§12 attribute syntax open item not explicit in CP4 forward-track (a)-(j) — 🟡)
- Terminology: 1 finding (CP3 new terms not in glossary; CP4 §14 carry-forward missing — 🟡)
- §9.5.4 G3 floor item attribution incorrect — 🟡
- §13.4 disposition reversal not explicitly attributed in CP3 CHANGELOG — 🟡
- Definition-of-done (§17): out of CP3 scope (CP4 spec-auditor full audit per Strategy §6.3 line 392)

Total: 0 🔴 + 2 🟠 + 6 🟡 + 4 ✅

Status header: `DRAFT CP3` confirmed at line 3 + status table at line 32. ✓

---

## Summary for orchestrator

**Verdict: PASS-WITH-NITS.** CP3 §9–§13 is structurally tight. Open-items list is internally consistent (9 closures + 3 new + 10 CP4 forward-track). Forward-references all marked CP4 / engine-cascade. Status header `DRAFT CP3` correct. Cross-doc citations to CP1 §2 / CP2 §4-§6 / Strategy §4.3.3 / 08c §Gap 5 / Phase 0 §9-§10 / ADR-0036 / ADR-0037 / ADR-0038 / `feedback_*.md` all resolve where checked. §9 surface inventory (added/removed/reshuffled) traces to CP1+CP2 source decisions; §10 codemod runbook traces to Strategy §4.3.3 with T6 explicitly noted as added; §9.5 cross-tenant Terminate uses 08c §Gap 5 verbatim "MUST NOT propagate" language with security-lead VETO retained; §13.4 T4/T5/T9 dispositions explicit. 

**Iterate-yes.** All 9 findings are mechanical (one-line edits each); no design rework.

**Top 3 must-fix before CP3 ratify:**

1. **§12.3 line 1913** — change `crates/action/src/lib.rs:14` → `crates/action/src/lib.rs:4` (the actual location of the contradicting docstring).
2. **§10.2 T6 row vs §10.5 manual-bucket** — pick one disposition (mixed AUTO+MANUAL per ADR-0038 §Negative item 4 — preferred — or all-MANUAL); update both sites to match.
3. **§9.3.1 row 2 "(retention TBD)" vs committed REMOVE** — replace the parenthetical with "(REMOVE — CP3 §9 lock)" to match the body decision lock at line 1560.

**Handoff: architect** for all 🟠 / 🟡 findings (none require tech-lead decision; all are content corrections). Architect to redraft §12.3 line 1913, §10.2 T6 row + §10.5 line 1725, §9.3.1 row 2, §11.1 line 1739 ADR-0036 item index, §9.5.5 inline signature, §10.3 line 1685 line range, §9.5.4 G3 attribution, CP4 forward-track (c) revision OR new (k) row.

**Handoff: tech-lead** advisory only — §9.4 ActionSlots seal CLOSED-pending-ratify and §9.3.1 `credential_typed` REMOVE recommendation both flagged as "Tech-lead ratifies at CP3 close" — both are surface-level decisions consistent with active-dev mode + `feedback_no_shims.md` + `feedback_hard_breaking_changes.md`. No structural blocker requires tech-lead decision before architect addresses 🟠 / 🟡 findings.

**Handoff: security-lead** advisory only — §9.5 cross-tenant Terminate uses 08c §Gap 5 verbatim "MUST NOT propagate" language; §9.5.5 implementation-time VETO retained on softening of normative wording. No security-substantive drift surfaced in CP3 §9-§13.

**Handoff: devops** advisory only — §10 codemod runbook + §13.4 hygiene fold-in are CP3 scope per Strategy §6.3 line 391; per Handoffs ask 1 (line 2233) "flag any transform where the auto/manual split is wrong" devops will likely surface the §10.2 vs §10.5 T6 contradiction (already 🟠 above). Architect addresses before devops review iterates.

*End of CP3 Tech Spec audit.*
