---
name: Q8 post-closure amendment bundle — tech-lead ratification
description: Ratification verdict for architect's Phase 3 enactment of Q8 research-driven amendment (5 AMEND items + 5 deferred cascade slots + 2 canon flags + 3 outside-scope auth findings spawned).
phase: ratification (post-Phase-3)
status: RATIFY-AMENDED-CLOSED-AGAIN
date: 2026-04-25
related:
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/q8-phase3-amendment-enactment.md
  - docs/tracking/cascade-queue.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/q8-phase2-synthesis.md
---

## Q8 bundle ratification verdict

**RATIFY-AMENDED-CLOSED-AGAIN.** Commit-ready: **YES**. Cascade closure: **AMENDED-CLOSED-AGAIN** (second post-closure amendment per Q7 precedent — does NOT regress to FULLY-CLOSED). Escalation: **NONE**.

All 5 AMEND items enacted in-place per architect's Phase 3 report. All 8 cascade-queue slots committed (slots 1–2 from Strategy §6.6 + slots 3–7 from Q8 §15.12.7 + slot 8 from Q8 §15.12.6). 2 canon flags scoped correctly to separate PR. ADR-0040 status preserved (`proposed`) per cascade prompt — NOT auto-flipped.

## F2 idempotency hook check

PASS. `TriggerAction::idempotency_key<'a>(&'a self, _event: &'a <Self::Source as TriggerSource>::Event) -> Option<IdempotencyKey>` (default `None`) at lines 316–321 + `IdempotencyKey(String)` newtype at lines 379–390. Default-opt-in shape mirrors `accepts_events()` default `false` precedent (§15.11 R5). Lifetime parametrization `<'a>` correctly threads through `<Self::Source as TriggerSource>::Event` projection — composes with §2.2.3 `handle()` bound chain. Doc cites cluster-mode cascade slot 2 + Strategy §3.1 component 7 + §5.1.5 line 297 — all grep-able. Resolves rust-senior 🔴 #2 (webhook dup-retry pain class) at hook-surface level; engine consumption is engine-cascade scope as flagged.

## F9 per-action concurrency check

PASS. `ActionMetadata::max_concurrent: Option<core::num::NonZeroU32>` at line 1303 with `#[serde(default, skip_serializing_if = "Option::is_none")]` — non-breaking field add against `#[non_exhaustive]` per `metadata.rs:96` line-pin. Builder method `with_max_concurrent` documented (line 1307). Macro emission path threading documented per ADR-0039 §1. T10 codemod transform added at §10.2 for hand-built ActionMetadata literals. Production code with `ActionMetadata::new(...)` constructor unaffected. `NonZeroU32` (not `u32`) is the right choice — disallows nonsensical zero-concurrency declarations at the type level.

## F12 workflow-version check

PASS. `NodeDefinition::action_version: nebula_resource::Version` at line 1343 — surface contract reference only; engine-cascade scope explicitly named (cascade slot 2). Doc names `ExecutionError::ActionVersionDrift { expected, actual }` as the engine-side enforcement variant (engine-cascade scope locks exact shape). Migration policy recommendation (`#[serde(default = "ActionVersion::unknown")]` + one-time re-pin) documented at line 1351. Preserves user-saved workflows. Surface contract cleanly delegated.

## F13 4× engine placeholder check

PASS. All four traits declared at lines 1375–1411 with empty bodies + `Send + Sync + 'static` bounds: `CursorPersistence` (PollAction cross-restart durability), `LeaderElection` (multi-worker coordination consumed by `on_leader_*` hooks), `ExternalSubscriptionLedger` (webhook URL stability across rebalance), `ScheduleLedger` (missed-fire replay for ScheduleAction cascade slot 3). Doc-only contract framing correctly named — engine cascade implements bodies. Rust-senior recommendation honored: forward-reference rot avoided for F2 `IdempotencyKey` return type per `feedback_active_dev_mode.md` ("before saying 'defer X', confirm the follow-up has a home"). Vocabulary established now without blocking action cascade landing.

## F15 docs cleanup scope check

PASS. F15 is mechanical pitfalls.md cleanup + doc cross-refs — flagged in §17 CHANGELOG entry and §16.5 cascade-final precondition without enacting `pitfalls.md` edits in this CP. Per `docs/AGENT_PROTOCOL.md` doc-edit discipline + `feedback_no_shims.md` adjacent rationale, cleanup PR co-lands with cascade implementation. Scope correct — Tech Spec is not the right home for `pitfalls.md` line-edits.

## Cascade-queue slots check

PASS. `docs/tracking/cascade-queue.md` exists; carries 8 slots in the table:
- Slot 1: Credential CP6 implementation (Strategy §6.6 line 416-426)
- Slot 2: Cluster-mode coordination (Strategy §6.6 line 426 last paragraph)
- Slot 3: ScheduleAction cascade — Hybrid B (sealed-DX + open `Schedule` runtime trait) per Phase 2.5 ScheduleAction analysis
- Slot 4: EventAction cascade (renamed from QueueAction by user) — sealed-DX peer
- Slot 5: AgentAction + ActionTool cascade — NEW primary trait family; canon §3.5 revision likely
- Slot 6: StreamAction + StreamStage cascade — NEW primary trait family; canon §3.5 revision likely
- Slot 7: TransactionAction cascade — shape TBD (sealed-DX over StatefulAction OR new primary)
- Slot 8: `nebula-auth` Tech Spec cascade — SSO/SAML/OIDC/LDAP/MFA per security-lead Q8 research

Each slot has architect-recommended shape recorded; Owner / Scheduled date / Queue position fields are TBD placeholders (orchestrator/user fills at next planning cycle per cascade-queue.md slot governance). Strategy §6.6 silent-degradation guard satisfied for all 5 Q8 deferred items (F3/F5/F8/F10/F11) + 3 outside-scope nebula-auth findings.

Phase 2 synthesis cross-check: F3=ScheduleAction → slot 3; F5=QueueAction → slot 4 (renamed EventAction); F8=AI sub-node → slot 5; F10=streaming → slot 6; F11=Saga → slot 7. All five DEFER items have a named home.

## Canon updates flag check

PASS. Two canon updates correctly scoped to separate PR (NOT Tech Spec amendment):
- F7 «no replay» declaration → `docs/PRODUCT_CANON.md` §0 (user picked path (i) = architect-default; matches Phase 2 §6.1 framing)
- F6 ItemLineage non-goal → `docs/PRODUCT_CANON.md` §6 (user picked path (c) = revised UP from architect-default β per Phase 2.5 ItemLineage deeper analysis; 3-of-4 peer engines lack lineage primitive — unambiguous structural-avoidance signal)

§15.12.5 records the flagging with citation chains; `docs/COMPETITIVE.md` line 41 cited for F7 rationale. Per cascade prompt Part C constraint ("do NOT touch PRODUCT_CANON.md в this Phase 3"), architect correctly did not enact canon edits in this CP. Housekeeping PR after cascade closes is the right home.

## Status qualifier appropriateness

PASS. Status header at line 3 includes the full Q8 qualifier per cascade prompt verbatim wording; §0.1 status table CP4 row at line 33 includes the appended qualifier text exactly as cascade prompt specified. Qualifier scope (5 AMEND items + 5 deferred cascade slots + 2 canon flagged + 3 outside-scope auth findings) accurately describes enactment per §15.5/§15.9/§15.10/§15.11 precedent. Q8 warrants full qualifier per §15.10 / §15.11 precedent (structural ripple via 2 NEW subsections + cascade-queue.md created); architect's "full qualifier" judgment is correct.

## §15.12 enactment record check

PASS. §15.12 record at lines 3338–3447 covers all 8 sub-sub-sections per cascade prompt requirement: §15.12.1 enactment table + §15.12.2 amend-in-place rationale + §15.12.3 cross-cascade impact (incl. ~3-5 internal sites + 0 community plugin migration impact + T10 codemod) + §15.12.4 §16.5 cascade-final preconditions (declared) + §15.12.5 canon flags + §15.12.6 outside-scope auth findings + §15.12.7 5 deferred cascade slots + §15.12.8 §15.1 closure non-update. Per-ADR composition analysis covers ADR-0024 / ADR-0035 / ADR-0038 / ADR-0039 / ADR-0040 with explicit "ADR-0040 status preserved (proposed) — Q8 does NOT auto-flip" per cascade prompt. Cross-references between §2.2.3 / §3.6 / §3.7 / §15.12 / §17 / cascade-queue.md slots all grep-able.

## Cascade closure recommendation

**AMENDED-CLOSED-AGAIN.** Per Q7 §15.11 precedent: post-closure research-driven amendment that adds default-opt-in surfaces, surface contracts, doc-only trait shapes, and named-home commitments does NOT trigger §0.2 invariant 4 spike-shape divergence (final_shape_v2.rs:209-262 unchanged). Cascade remains in AMENDED-CLOSED state — second amendment-in-place since CP4 freeze (Q7 was first; Q8 is second). Both follow ADR-0035 amended-in-place precedent for canonical-form correction + research-driven gap-closure with named-home commitments.

No cascade revisit triggered. No 🔴 escalation surfaces — all 5 AMEND items are non-spike-divergent; 5 DEFER items have named homes; 2 canon flags scoped correctly; 3 outside-scope spawns scoped correctly. ADR-0040 status preservation honored; user ratification on canon §3.5 still pending per cascade prompt (separate workflow from this Q8 enactment).

## Required edits if any

**NONE.** Architect's Phase 3 enactment is mechanically clean. One observation — NOT a required edit:

- §16.5 cascade-final precondition list at lines 3517–3527 still shows the 6 original rows; the 2 new Q8 preconditions are declared in §15.12.4 only (lines 3409–3410), not appended to §16.5 itself. Architect explicitly notes this at line 3876: *"§16 unchanged except §16.5 precondition extension is implicit per §15.12.4 reference (table itself preserved verbatim — preconditions added via §15.12.4 record cross-ref)."* This matches the §15.9.4 / §15.10.4 / §15.11.4 precedent (each prior amendment also declared §16.5 precondition extensions in §15.X.4 records without modifying the §16.5 table). Pattern is intentional and consistent — accept.

The Q7 ratification flagged a single anchor-fix nit (§3.2→§3.5 in three sites). No analogous nit surfaces in Q8 — anchor citations to §15.12 / §3.6.1 / §3.6.2 / §3.7 are correctly pinned throughout.

## Summary

**Verdict.** RATIFY-AMENDED-CLOSED-AGAIN. **Commit-ready: YES.** **Cascade status: AMENDED-CLOSED-AGAIN** (second post-closure amendment; consistent with Q7 §15.11 precedent). **Escalation: NONE.**

All 10 ratification checks PASS:
1. F2 idempotency hook — default-opt-in shape correct, resolves rust-senior 🔴 #2
2. F9 per-action concurrency — `#[non_exhaustive]`-safe with NonZeroU32 type discipline
3. F12 workflow-version — surface contract correctly delegated to engine cascade
4. F13 4× engine placeholders — doc-only contracts; vocabulary establishment without engine blocking
5. F15 docs cleanup — correctly scoped to cleanup PR
6. 8 cascade slots committed — Strategy §6.6 silent-degradation guard satisfied for all 5 Q8 defers + 3 outside-scope spawns
7. 2 canon flags — F7 §0 + F6 §6 correctly scoped to separate PR
8. Status qualifier — verbatim matches cascade prompt wording
9. §15.12 record — all 8 sub-sub-sections complete
10. No escalation triggers — none of the 5 AMEND items change spike-locked shapes

Phase 6 cascade remains in **AMENDED-CLOSED** state. Phase 8 implementation handoff continues per §16; user pick on path (a)/(b)/(c) at Phase 8 cascade summary; ADR-0040 ratification on canon §3.5 revision still pending (separate workflow).

Tech-lead authorizes commit of Q8 enactment bundle: Tech Spec amendments + new `docs/tracking/cascade-queue.md`. F15 pitfalls.md cleanup + 2 canon updates land in subsequent housekeeping PRs after cascade closes.
