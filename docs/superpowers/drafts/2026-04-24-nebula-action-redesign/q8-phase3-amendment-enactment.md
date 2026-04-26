---
name: Q8 Phase 3 amendment-in-place enactment
description: Architect's enactment record for Q8 post-closure research-driven amendment — 5 AMEND items in-place + 5 deferred cascade slots committed + 2 canon updates flagged + 3 outside-scope auth findings spawned.
phase: 3 (enactment)
status: enacted
date: 2026-04-26
related:
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md
  - docs/tracking/cascade-queue.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/q8-phase2-synthesis.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/q8-phase2.5-itemlineage-analysis.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/q8-phase2.5-scheduleaction-analysis.md
---

# Q8 Phase 3 amendment-in-place enactment

## Part A — 5 AMEND enactments (file paths + line refs)

All 5 amendments landed inline in the FROZEN CP4 Tech Spec per ADR-0035 amended-in-place precedent. Tech Spec status header updated (`status: ... amended-in-place 2026-04-26 — Q8 ...`); §0.1 status table CP4 row qualifier appended; §15.12 NEW enactment record inserted between §15.11 and §16; §17 CHANGELOG Q8 entry appended.

| Q8 amendment | File path | Section | Insertion type |
|---|---|---|---|
| **F2 — `TriggerAction::idempotency_key()` hook + `IdempotencyKey` type** | `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` | §2.2.3 (TriggerAction trait shape) | Method added to trait shape between `accepts_events()` and `handle()`; `IdempotencyKey` value type added adjacent to `TriggerEventOutcome`; second callout box (Q8 F2) appended at §2.2.3 top |
| **F9 — `ActionMetadata::max_concurrent: Option<NonZeroU32>` field** | `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` | §3.6.1 (NEW) | New §3.6 subsection added between §3.5 and §4; §3.6.1 shows the metadata field add per `crates/action/src/metadata.rs:96-117` `#[non_exhaustive]`-safe shape |
| **F12 — `NodeDefinition::action_version: SemVer` surface contract** | `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` | §3.6.2 (NEW) | Sibling subsection in §3.6; documents Tech Spec records the surface contract (`ActionMetadata::base.version` is the source the engine reads); engine cascade locks dispatch enforcement |
| **F13 — 4× engine cluster-mode trait placeholders** | `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` | §3.7 (NEW) | New subsection added after §3.6; declares `CursorPersistence`, `LeaderElection`, `ExternalSubscriptionLedger`, `ScheduleLedger` as doc-only trait shapes in `nebula-engine`; engine cascade implements bodies |
| **F15 — Mechanical docs cleanup (6 pitfalls.md entries + minor doc cross-refs)** | `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` | §17 CHANGELOG (this entry) + cascade-final precondition reference | Flagged in this CP for cleanup PR co-landing with cascade implementation; pitfalls.md edit not enacted in this CP per `docs/AGENT_PROTOCOL.md` doc-edit discipline |

**Sections amended inline with §15.12 callout/reference:**

- Tech Spec status header (line 3) — Q8 qualifier appended.
- §0.1 status table (line 33) — CP4 row qualifier appended per spec wording.
- §2.2.3 callout box (post-Q7 narrative) — Q8 F2 callout added.
- §2.2.3 trait shape — `idempotency_key()` method added; `IdempotencyKey` type defined adjacent to `TriggerEventOutcome`.
- §3.6 (NEW subsection) — F9 + F12 surface contracts.
- §3.7 (NEW subsection) — F13 doc-only trait placeholders.
- §15.12 (NEW subsection) — Q8 enactment record (§15.12.1 enactment table + §15.12.2 amend-in-place rationale + §15.12.3 cross-cascade impact + §15.12.4 cascade-final precondition extensions + §15.12.5 canon updates flagged + §15.12.6 outside-scope auth findings + §15.12.7 5 deferred cascade slots + §15.12.8 §15.1 closure non-update).
- §17 CHANGELOG — Q8 entry appended at file end.

## Part B — 5 deferred cascade slots committed (cascade-queue.md additions)

Created [`docs/tracking/cascade-queue.md`](../../../tracking/cascade-queue.md) per Strategy §6.6 cross-crate coordination tracking discipline. The file did not exist on disk at draft time per audit verification (Tech Spec §16.5 cascade-final precondition row noted "the file does not exist on disk at draft time"). Created with 8 slots:

| Slot # | Cascade name | Source |
|---|---|---|
| 1 | Credential CP6 implementation | Strategy §6.6 (line 416-426) |
| 2 | Cluster-mode coordination | Strategy §6.6 (line 426 — last paragraph) |
| 3 | ScheduleAction cascade | Q8 §15.12.7 (per Phase 2.5 deeper analysis Hybrid B) |
| 4 | EventAction cascade (renamed by user from QueueAction) | Q8 §15.12.7 |
| 5 | AgentAction + ActionTool cascade | Q8 §15.12.7 |
| 6 | StreamAction + StreamStage cascade | Q8 §15.12.7 |
| 7 | TransactionAction cascade | Q8 §15.12.7 |
| 8 | `nebula-auth` Tech Spec cascade | Q8 §15.12.6 |

Each slot has architect-recommended shape recorded; Owner / Scheduled date / Queue position fields are TBD placeholders (user fills at next planning cycle). Slot governance rules + adding-new-slot procedure documented inline in `cascade-queue.md`.

## Part C — 2 canon updates flagged for separate PR

Per cascade prompt Part C: 2 canon updates touch `docs/PRODUCT_CANON.md`, NOT this Tech Spec. Mention in §15.12.5 enactment record + §17 CHANGELOG; actual canon edit lives in housekeeping PR after cascade closes.

| Canon update | Target section | Rationale | Closes |
|---|---|---|---|
| **F7 — «no replay» declaration** | `docs/PRODUCT_CANON.md` §0 | Explicit position that Nebula is NOT a durable-execution engine in the Temporal sense — workflows are durable per state machine; action authors use Rust language + std + crates directly without replay-determinism constraint. Cite `docs/COMPETITIVE.md` line 41 ("Our bet: Typed Rust integration contracts + honest durability beat a large but soft ecosystem"). | Q8 Phase 2 §3 F7 architect-default position |
| **F6 — ItemLineage non-goal entry** | `docs/PRODUCT_CANON.md` §6 | Explicit non-goal entry: ItemLineage primitive (n8n's `pairedItem` lineage tracking) is NOT a Nebula goal. Per Phase 2.5 ItemLineage analysis — typed payload model absorbs n8n lineage class; 3 of 4 peer engines (Temporal, Argo, Prefect) also lack lineage primitive — unambiguous structural-avoidance signal. NO future cascade slot for ItemLineage (rejected as scope-creep at canon level, revised UP from β defer per Phase 2.5 deeper analysis). | Q8 Phase 2 §3 F6 amended-default position |

These two canon edits are explicitly out-of-scope for this Phase 3 enactment per cascade prompt Part C constraint ("do NOT touch PRODUCT_CANON.md в this Phase 3"). Architect flags only.

## Part D — 3 outside-scope auth findings (`nebula-auth` cascade slot)

Per cascade prompt Part D: 3 🔴 SSO/SAML/OIDC/LDAP/MFA gaps identified by security-lead Q8 research live outside action-cascade scope. Architect-recommended shape: spawn separate `nebula-auth` Tech Spec future cascade.

Cascade slot 8 in `docs/tracking/cascade-queue.md` records the architect recommendation. Orchestrator commits the slot at next planning cycle per cascade prompt Part D framing ("Add к cascade_queue.md as slot N+5 if not already there. Architect recommendation only; orchestrator commits slot.").

Source: [`q8-security-credential-research.md`](q8-security-credential-research.md) — security-lead Q8 research stream identified the 3 🔴 gaps as SSO/SAML/OIDC/LDAP/MFA (the 5 sub-classes); Q8 Phase 2 synthesis bucketed all 3 as outside action-cascade scope per pillar-fit gate.

## §15.12 enactment record summary

§15.12 enactment record captures:

- **§15.12.1 Enactment** — 5 AMEND items table with Tech Spec section + class + spike risk per item; per-ADR composition analysis (no ADR file edits required); picked-rationale per amendment.
- **§15.12.2 Why amend-in-place vs supersede** — research-driven gap-closure with default-opt-in surfaces and doc-only contracts (no paradigm shift); ADR-0035 §Status block "canonical-form correction" criterion satisfied; bundle landing in single CP per §15.5/§15.9/§15.10/§15.11 precedent.
- **§15.12.3 Cross-cascade and downstream impact** — ADR statuses preserved (incl. ADR-0038 NOT auto-flipped); production code impact (zero immediate change required for community plugins); reverse-dep impact (~3-5 internal sites + 0 community plugin migration); codemod additions (T10 added).
- **§15.12.4 §16.5 cascade-final precondition update** — two new preconditions added (Q8 5 AMEND items absorbed; cascade-queue slots 3-8 committed).
- **§15.12.5 Canon updates flagged for separate PR** — F7 + F6 canon edits flagged; explicitly NOT touched in this Phase 3.
- **§15.12.6 Outside-scope auth findings** — `nebula-auth` cascade slot 8 spawn record.
- **§15.12.7 Deferred cascade slots** — 5 slots committed (3: ScheduleAction; 4: EventAction; 5: AgentAction + ActionTool; 6: StreamAction + StreamStage; 7: TransactionAction).
- **§15.12.8 §15.1 closure entries updated** — no new row needed (Q8 was research-driven post-closure audit, not pre-CP review); deferred items committed to `cascade-queue.md`, parallel to §15.8 deferred-with-trigger registry.

**Status qualifier confirmed.** Per §15.10/§15.11 precedent for amendments with structural ripple (5 AMEND items across 3 sections; 2 NEW subsections; cascade-queue.md created), Q8 warrants full status qualifier per cascade prompt requirement. Status qualifier applied:

```
status: FROZEN CP4 2026-04-25 (... + Q7 ...; amended-in-place 2026-04-26 — Q8 research-driven amendment per §15.12 — 5 AMEND items: F2 idempotency hook + F9 per-action concurrency + F12 workflow-version pin + F13 4× engine cluster-mode trait placeholders + F15 mechanical docs cleanup, plus 5 deferred cascade slots committed to docs/tracking/cascade-queue.md + 2 canon updates flagged for separate PR + 3 outside-scope auth findings spawned to nebula-auth cascade slot)
```

§0.1 status table CP4 row qualifier appended exactly per cascade prompt wording:
```
+ Q8 post-closure research-driven amendment per §15.12 — 5 AMEND items + 5 deferred cascade slots + 2 canon updates flagged + 3 outside-scope auth findings spawned
```

Header status string + §0.1 table qualifier both updated. Cross-references between §2.2.3 / §3.6 / §3.7 / §15.12 / §17 CHANGELOG / cascade-queue.md slots 3-8 are all `grep`-able.

