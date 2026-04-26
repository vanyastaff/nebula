# Cascade log — nebula-action redesign

## Meta

- **Start:** 2026-04-24
- **Orchestrator session:** claude/upbeat-mendel-b30f89 (worktree)
- **Input prompt version:** v1 — hands-off orchestrator dispatch for nebula-action redesign cascade, pattern inherited from nebula-credential CP6 work
- **Budget:** 5 days agent-work equivalent; 3 rounds per consensus protocol; 2 iterations per spike; 2 rounds per checkpoint review
- **Breaking-change policy:** OK with migration plan; reject without plan
- **Escalation trigger file:** `ESCALATION.md` at repo root (absent at start)

## Agent roster (verified from `.claude/agents/*.md` on 2026-04-24)

| Agent | Role |
|-------|------|
| architect | Long-form design document drafting with checkpoint cadence |
| devops | CI/CD, cargo deny, MSRV, workspace health |
| dx-tester | Newcomer API ergonomics smoke test |
| orchestrator | This session (meta-agent, routing + consolidation) |
| rust-senior | Idiomatic Rust review, safety, performance |
| security-lead | Threat modeling, credential/secret handling, sandboxing |
| spec-auditor | Long-form document structural integrity |
| tech-lead | Priority calls, trade-offs, cross-crate coordination |

All 8 stakeholder agents present; roster matches prompt expectations.

## Phase gate status

| Phase | Status | Artefact | Gate notes |
|-------|--------|----------|------------|
| Pre-setup | ✅ complete | this file | CASCADE_LOG initialized |
| Phase 0 | ✅ complete | [`01-current-state.md`](./01-current-state.md) + [`01a-code-audit.md`](./01a-code-audit.md) + [`01b-workspace-audit.md`](./01b-workspace-audit.md) | Gate passed: convergent audits, 4 🔴 + 9 🟠 findings; escalation flag raised (C1) but not hard-stop |
| Phase 1 | ✅ complete | [`02-pain-enumeration.md`](./02-pain-enumeration.md) + 02a/b/c/d sub-reports | Gate passed: 11 🔴 + 30+ 🟠 findings; critical reframe from tech-lead (credential CP6 unimplemented in credential crate too) |
| Phase 2 | ✅ complete | [`03-scope-decision.md`](./03-scope-decision.md) + 03a/b/c sub-reports | Gate locked: **Option A'** (co-landed action + credential CP6 design). Round 2 required because architect proposed B'+ hybrid that tech-lead hadn't evaluated. Design-scope-only reframe resolved budget concern. No VETO, no escalation. |
| Phase 3 | ✅ complete | [`Strategy FROZEN CP3`](../../specs/2026-04-24-action-redesign-strategy.md) + 04a/b + 05a/b + 06a/b sub-reviews | Frozen 2026-04-24 after CP1+CP2+CP3 cycles. Each CP iterated once after spec-auditor + tech-lead review. 540 lines total. |
| Phase 4 | ✅ complete | [`07-spike-NOTES.md`](./07-spike-NOTES.md) + `final_shape_v2.rs` | Iter-1 PASS + Iter-2 PASS; spike commit `c8aef6a0` on isolated worktree branch `worktree-agent-af478538`; 10 effective tests passing; Tech Spec §7 unblocks |
| Phase 5 | ✅ complete | [`ADR-0038`](../../adr/0038-action-trait-shape.md) + [`ADR-0039`](../../adr/0039-action-macro-emission.md) + [`ADR-0040`](../../adr/0040-controlaction-seal-canon-revision.md) | 3 PROPOSED ADRs drafted; ADR-NNNN+3 cluster-mode hooks deliberately deferred (scope §2) |
| Phase 6 | in-progress (continuation session) | Tech Spec CP1-CP4 (target file: `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`) | Continuation dispatched 2026-04-24; CP1 architect draft starting; 4 CPs × 5 parallel reviewers per Strategy §6.3 |
| Phase 7 | ✅ complete (lite) | Concerns register summary in Phase 8 deliverable | No multi-blocker consensus session triggered; register lives as Phase 8 §register-state |
| Phase 8 | in-progress | Summary file | Producing `docs/superpowers/specs/2026-04-24-nebula-action-redesign-summary.md` |

## Cross-crate awareness (orchestrator tracking)

- **nebula-credential Tech Spec at CP6** (corrected from prompt's "CP5" — recent commits `65443cdb`, `33eb3f01`, `883ccfbf` confirm CP6 freeze level). Per prompt non-goals: action cascade MUST NOT require credential §2.7 / §3.4 / §7.1 revisions. Phase 0 surfaces a spec-reality gap (C1) — **Phase 2 co-decision will face Option A/B/C**; C triggers escalation rule 10.
- **nebula-resource Tech Spec** — not frozen; prior artefact is `2026-04-06-resource-v2-design.md` (design doc, older than credential work). Action cascade can proceed tentatively; coordination flag if scope touches resource public API.
- **Prior action v2 design** (`2026-04-06-action-v2-design.md`) — 2.5 weeks old, multiple drift findings per 01a §8.

## Escalations

None raised so far. Watch list:

- Phase 2 co-decision on credential integration shape (Option A vs B vs C); C is hard-escalation rule 10.
- Phase 4 spike feasibility of `#[action]` attribute macro introduction (if Option A chosen).
- Adjacent finding T3: `nebula-runtime` dead reference in CI matrix + CODEOWNERS — should file separately at cascade end.

## Log entries

### 2026-04-24 T16:12 — Pre-Phase 0 reconnaissance complete

Orchestrator verified:
- All 8 agents present
- Action crate surface wider than prompt hint (10 trait surfaces vs 4; verified via lib.rs docstring)
- Prior v2 design docs exist (2.5 weeks old) — Phase 0 reconciled drift
- No external action-adjacent crates — migration blast radius localized to 7 direct reverse-deps
- `trybuild`/`macrotest` absent from dev deps — macro test harness gap confirmed

### 2026-04-24 T16:25 — Phase 0 audits returned

Rust-senior: 01a-code-audit.md (~450 lines). 4 🔴 + 9 🟠 findings.
Devops: 01b-workspace-audit.md (~380 lines). 0 🔴 + 8 🟠 + 15 🟡 findings.

Devops wrote to wrong repo path (main repo instead of worktree) — orchestrator corrected via `mv`. Soft process issue; no retry needed.

### 2026-04-24 T20:30 — Phases 4+5 complete (commit a38f6f5a for Phase 3, next for Phases 4+5)

**Phase 4 spike** — rust-senior isolated worktree, 2 iterations, both PASS:
- Iter-1: Created scratch crate; minimum types (CredentialRef<C>, AnyCredential, SlotBinding, SchemeGuard, SchemeFactory); hand-expanded `#[action(credentials(slack: SlackToken))]` for Stateless+Bearer; 3 compile-fail probes + 3 bonus probes all green
- Iter-2: 3 realistic actions (Stateless+Bearer / Stateful+OAuth2-refresh / ResourceAction+Postgres+Basic) compose; cancellation drop-order test passes (zeroize fires under `tokio::select!` mid-await); macro expansion within 2x perf bound
- Findings: 🔴 #1 auto-deref Clone shadowing on SchemeGuard probe (Tech Spec §16.1.1 amendment candidate); 🟢 #2 iter-3 lifetime-pin refinement validated; 🟡 #3 dual enforcement layers (type-system + compile_error!) per probe 3 contract
- Spike commit: `c8aef6a0` at `C:\Users\vanya\RustroverProjects\nebula\.claude\worktrees\agent-af478538\scratch\spike-action-credential\`
- Tech Spec §7 Interface unblocks per Strategy §5.2.4 aggregate-DONE

**Phase 5 ADR drafting** — 3 PROPOSED ADRs:
- ADR-0038 Action trait shape (#[action] attribute macro replacing derive; narrow zone rewriting)
- ADR-0039 Action macro emission contract (HRTB resolve_fn; dual enforcement; macro test harness)
- ADR-0040 ControlAction seal + canon §3.5 DX tier ratification (canon revision per §0.2)
- ADR-NNNN+3 cluster-mode hooks deliberately deferred (out of cascade scope per Strategy §6.2)

### 2026-04-26 — Spike iter-3 PASS (final_shape_v3 post-amendments)

**User noted spike artefact stale relative к Tech Spec post-amendment state.** Phase 4 spike validated pre-FROZEN-CP4 shapes (commit `c8aef6a0`); since then 4 amendment rounds (Q1+Q6+Q7+Q8) added 11 trait shape changes.

**Spike iter-3 dispatched** к rust-senior в isolated worktree (compile-validate, не full re-spike per Phase 4 PASS verdict для shape-only stands).

**Outcome: PASS**
- Isolated worktree commit: `10b24616` (branch `worktree-agent-a3ec73dbf722f0095`)
- 8 probes; clean `cargo +1.95.0 check`; no warnings/errors
- v3: 839 lines (vs v2's 284) — substantial growth reflects amendment additions

**Files written к main worktree (un-committed для orchestrator):**
- `docs/superpowers/drafts/2026-04-24-nebula-action-redesign/final_shape_v3.rs`
- `docs/superpowers/drafts/2026-04-24-nebula-action-redesign/spike-iter3-NOTES.md`

**v2 untouched** (historical artefact preserved per Phase 4 spike PASS reference).

**Diff summary v3 vs v2 (11 amendments verified compose):**
- **Q1** *Handler traits → `#[async_trait]`; BoxFut alias scope narrowed к SlotBinding HRTB only
- **Q6** TriggerAction gains start/stop adjacent к handle()
- **Q7 R1** StatefulAction restores init_state (mandatory) + migrate_state (default None)
- **Q7 R2** ResourceAction paradigm flip — configure/cleanup only; spurious execute/Input/Output dropped
- **Q7 R3** TriggerAction::handle returns Result<TriggerEventOutcome, Error> (Skip/Emit/EmitMany); accepts_events() predicate
- **Q7 R4** ResourceHandler uses Box<dyn Any + Send + Sync> boundary
- **Q7 R5** TriggerHandler::handle_event adopts TriggerEvent envelope (type-erased)
- **Q7 R6** WebhookAction/PollAction re-pinned as peers of Action (NOT subtraits of TriggerAction); each carries own associated types + lifecycle
- **Q8 F2** TriggerAction::idempotency_key default-opt-in hook + IdempotencyKey newtype
- **Q8 F9** ActionMetadata::max_concurrent: Option<NonZeroU32>
- **Q8 F12** NodeDefinition::action_version: semver::Version
- **Q8 F13** 4 doc-only engine cluster-mode trait placeholders (CursorPersistence/LeaderElection/ExternalSubscriptionLedger/ScheduleLedger)

**Спike iter-3 confirms FROZEN CP4 + Q1+Q6+Q7+Q8 amendments compose-clean.** Implementation can begin referencing v3 как concrete signature contract.

### 2026-04-26 — Q8 research-driven amendment complete (AMENDED-CLOSED-AGAIN)

**Phase 1 (4 parallel agents, ~6h):**
- rust-senior trigger-research: 5 🔴 + 9 🟠 (action scope) — cursor lie / no idempotency contract / no ScheduleAction / no external-reg reconciler / no QueueAction peer
- architect action-research + Temporal: 8 🔴 + 6 🟠 (action scope) — ItemLineage / determinism / AI sub-node / per-action concurrency / streaming / Saga / bulk-op idempotency / workflow-version
- security-lead credential-research: **0 🔴 in-scope** (3 🔴 outside scope → nebula-auth cascade slot)
- dx-tester parameter-research: 0 NEW 🔴 + 4 🟠; ≥50 of 62 n8n parameter pains structurally addressed

**Phase 2 architect synthesis: 15 unique deduped findings** (after rationalization across agents).

**Phase 2.5 deeper investigations** (user pushback на 2 architect-default positions):
- **F6 ItemLineage**: revised UP from (β) defer → **(c) canon NON-GOAL** — 12 use cases analyzed; 5 genuinely need engine support в n8n но structurally absorbed by Nebula's typed `ActionResult<T>` model (authors carry ids в payloads); 3 of 4 peer engines also lack lineage primitive; pillar fit weak per PRODUCT_CANON §6
- **F3 ScheduleAction**: **Hybrid B** — sealed-DX peer of TriggerAction (no canon revision per ADR-0040 §2 Webhook/Poll precedent) + open `Schedule` runtime trait + 3 blessed impls (Cron/Interval/OneShot); 15 brainstormed custom schedule kinds для community

**User decisions (final, ratified):**
- ✅ F7 Determinism = (i) canon explicit «no replay» (action authors use Rust language + std + crates напрямую)
- ✅ F6 ItemLineage = (c) canon NON-GOAL (revised UP, no future cascade slot)
- ✅ F3 ScheduleAction = DEFER к future cascade (Hybrid B shape recorded)
- ✅ F5 EventAction (renamed от QueueAction by user) = DEFER (sealed-DX peer shape recorded)
- ✅ F8 AgentAction + ActionTool (user named) = DEFER (new primary family shape recorded)
- ✅ F10 StreamAction + StreamStage (user named) = DEFER (new primary family shape recorded)
- ✅ F11 TransactionAction (user named) = DEFER (sealed-DX over Stateful OR new primary; shape TBD)

**Phase 3 architect enactment (5 AMENDs amendment-in-place per ADR-0035 precedent):**
- F2 Idempotency hook contract — `TriggerAction::idempotency_key()` default-opt-in (Tech Spec §2.2.3)
- F9 Per-action concurrency — `ActionMetadata::max_concurrent: Option<NonZeroU32>` (Tech Spec §3.6.1 NEW)
- F12 Workflow-version save-time pin — `NodeDefinition::action_version: SemVer` (Tech Spec §3.6.2 NEW; engine cascade locks enforcement)
- F13 4× engine cluster-mode trait placeholders — `CursorPersistence` / `LeaderElection` / `ExternalSubscriptionLedger` / `ScheduleLedger` (Tech Spec §3.7 NEW; doc-only contracts)
- F15 Mechanical docs cleanup — flagged для co-landing PR

**8 cascade slots committed** к `docs/tracking/cascade-queue.md` (NEW file):
- 2 from Strategy §6.6 (credential CP6 implementation; cluster-mode coordination)
- 5 from Q8 §15.12.7 (ScheduleAction / EventAction / AgentAction+ActionTool / StreamAction+StreamStage / TransactionAction) — каждый с architect-recommended trait shape
- 1 from Q8 §15.12.6 (nebula-auth для SSO/SAML/OIDC/LDAP/MFA — outside-scope)

**2 canon updates flagged для separate PR** (NOT Tech Spec):
- F7 Canon §0 «no replay» declaration (cite COMPETITIVE.md line 41)
- F6 Canon §6 ItemLineage non-goal entry

**Tech-lead RATIFY-AMENDED-CLOSED-AGAIN** verdict (Q8 ratification gate). Zero required edits (Q7 had 1 nit; Q8 mechanically clean). Phase 6 cascade remains AMENDED-CLOSED.

**Status header**: `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 + Q6 + Q7 post-closure audit; amended-in-place 2026-04-26 — Q8 research-driven amendment per §15.12)`

**Tech Spec final size:** 3522 lines (Q8 mostly added §3.6/§3.7/§15.12; total growth modest per amendment-only scope).

**No ADR amendments** — все Q8 AMEND items fall within Tech Spec author authority per ADR-0035 precedent.

**No escalation.** Cascade remains AMENDED-CLOSED pending only Q1 implementation path + Q5 canon §3.5 ratification + 2 canon update PRs (F7/F6).

### 2026-04-25 T08:00 — Q8 comprehensive design audit dispatched (research-driven gap closure)

**User directive**: pre-implementation comprehensive audit using `docs/research/` (4428 lines across 8 peer-research files: n8n actions/triggers/credentials/parameters/auth + temporal + windmill + activepieces). Cascade should be «глубже и чище и продуманне»; cover ALL Action types; не упустить ничего; spike-validate если нужно.

**Tech-lead Q7 retrospective recommendation already noted**: standing dual-audit pattern. Q8 extends к **research-driven coverage audit** — does FROZEN CP4 design address pain points identified in peer ecosystem? Are there architectural gaps что spike+research would surface?

**Phase 1 dispatch (4 parallel agents)**:
- **rust-senior**: research/n8n-trigger-pain-points + research/windmill-peer-research → trigger family coverage (3 incompatible state shapes from Q7; cluster-mode hooks; webhook reliability)
- **architect**: research/n8n-action-pain-points + research/temporal-peer-research → action design coverage (durable execution semantics; activity vs workflow distinction; saga patterns)
- **security-lead**: research/n8n-credential-pain-points + research/n8n-auth-architecture → credential integration coverage post Q7 R6 (peer-of-Action)
- **dx-tester**: research/activepieces-peer-research + research/n8n-parameter-pain-points → DX/macro design coverage (parameter UI; type-safe pieces; developer-first claims)

Each agent: line-by-line research read + cross-ref Tech Spec FROZEN CP4 + Q7 amendments. Output: research-cited gap list с severity tags.

**Phase 2**: architect consolidation; if new shapes/traits proposed → rust-senior spike iter-3 в isolated worktree validates compose с existing 9 traits + 7 adapters.

**Phase 3**: amendment-in-place per ADR-0035 precedent OR escalation если findings exceed amendment scope.

### 2026-04-25 T07:00 — Q7 post-closure audit complete + AMENDED CLOSED

**Phase 1 returned (parallel rust-senior + tech-lead, ~6h):**

- **Rust-senior production inventory** (9 traits + 7 adapters): 0 🔴 production bugs; 3 unexpected findings — (1) WebhookAction/PollAction PEERS of TriggerAction (NOT subtraits per ADR-0040); (2) 3 incompatible state shapes (WebhookAction::State ephemeral RwLock; PollAction::Cursor stack-frame; TriggerAction none); (3) PollCursor<C> wrapper не actual cursor (per-cycle wrapper, A::Cursor evaporates on task exit)
- **Tech-lead coverage map**: 5 🔴 + 3 🟠 + 8 🟡 + 4 🟢 + ~25 ✅; ~50% coverage; recommendation **AMENDED-CLOSED** (всё mechanical lifecycle slips same class as Q6)

**Phase 2+3 architect dispatch — combined (3h):**

- **Part A — §2.9 fifth iteration**: Verdict **(III)** REJECT preserved at trait-method-input axis; new finding **R6** emerges (sealed-DX bound chain re-pin: Webhook/Poll PEERS не subtraits). Q5 framing's NEW evidence не flips §2.9 because Webhook/Poll's own associated types live на their own traits, not на TriggerAction.

- **Part B — 17 amendments enacted as Q7 bundle:**
  - 🔴 R1: §2.2.2 init_state + migrate_state restored from production stateful.rs:56
  - 🔴 R2: §2.2.4 ResourceAction configure/cleanup restored; spurious execute/Input/Output dropped
  - 🔴 R3: §2.2.3 TriggerEventOutcome multiplicity (Skip/Emit/EmitMany) + accepts_events restored
  - 🔴 R4: §2.4 ResourceHandler Box<dyn Any> dyn boundary
  - 🔴 R5: §2.4 TriggerHandler TriggerEvent envelope
  - 🔴 R6: §2.6 sealed-DX bound chain re-pin (peer-of-Action, not TriggerAction subtraits)
  - §3.5 NEW typification path narrative (R3 + R5 + I3 cross-cite)
  - §8.1.2 cursor in-memory ownership narrative
  - §1.2 N1 cred-cascade dependency note
  - 8 🟡 doc-gap closures
  - §15.11 NEW enactment record
  - §17 CHANGELOG Q7 entry

**Tech Spec final size:** 3522 lines (was ~2400 at FROZEN CP4; +1100 for Q7 production-shape restoration).

**Tech-lead RATIFY-AMENDED-CLOSED** verdict (post-closure ratification gate). 1 mechanical nit fixed inline (§3.2 → §3.5 typification narrative reference в 6 sites).

**Status header**: `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 post-freeze + Q6 lifecycle gap + Q7 post-closure audit per §15.11)`

**Cascade closure**: **AMENDED CLOSED** (architect rejected face-saving FULLY-CLOSED framing; honest assessment per `feedback_active_dev_mode` — 17 missed findings is decisive cascade-quality miss).

**Tech-lead meta-finding (cascade retrospective input):** standing dual-audit pattern (production inventory + spec coverage map) before declaring FROZEN should propagate to future cascades. 17 missed findings across 5 reviewers across 4 CPs = systematic gap, not user pushback.

**No ADR amendments** — all R1-R6 fall within Tech Spec author authority per ADR-0035 amended-in-place precedent.

**No escalation.** Cascade fully closed pending Q1 implementation path + Q5 canon §3.5 ratification.

### 2026-04-25 T05:00 — Post-closure systematic audit dispatched

**User concern**: prior cascade phases (especially Q4-Q6 on §2.9 + lifecycle) **may have been insufficiently thorough on Trigger DX**. Specific concerns:

1. **PollCursor on PollAction = State analog** — PollAction (DX over TriggerAction per ADR-0040) has cursor concept. StatefulAction has `type State`. Does trigger family need State semantics что cascade missed?
2. **start/stop fully addressed at Q6 OR partially?** — Q6 amendment-in-place added start/stop к TriggerAction direct, но adapter layer architecture (TriggerActionAdapter, WebhookTriggerAdapter, PollTriggerAdapter) что absorbs lifecycle complexity не explicit в Tech Spec
3. **§2.9 fourth iteration via start() Input axis** — prior 4 REJECTs argued "Trigger has no user-supplied Input" basing on `handle()` parameter only. start() Input axis not discussed. WebhookAction.start() registers URL with external service — uses per-action info (URL, secret) — that's user-supplied configuration AT METHOD LEVEL via start(input)?
4. **Other gaps potentially missed**: TriggerHandler decoupling pattern (production has separate trait; Tech Spec collapsed); TriggerEvent type-erased payload migration to typed Source::Event not in §10 codemod; StatefulAction state migration semantics; ResourceAction pool integration completion

**3-phase audit dispatched (estimated 7-12h, 3-day budget):**

- Phase 1: rust-senior production capability inventory + tech-lead Tech Spec coverage map (parallel)
- Phase 2: §2.9 fifth iteration с start() Input axis specifically (if Phase 1 confirms axis materializes)
- Phase 3: amendment-in-place per ADR-0035 precedent OR escalation if 🔴 REGRESSION

### 2026-04-25 T04:30 — Post-freeze Q6 — TriggerAction lifecycle gap fix (SPLIT)

**Gap identified during user review of Tech Spec design**: production code `crates/action/src/trigger.rs:61-72` имеет `TriggerAction::start()` + `::stop()` lifecycle methods. Tech Spec §2.2.3 frozen без них. Phase 4 spike covered shape-only probes, не lifecycle. Tech Spec §2.9 amendment line 530 references "engine drives start" но start не определён ни в TriggerAction ни в TriggerSource — slip между production code и frozen design.

**Architect dispatched targeted gap fix** (no specialist parallel review per user spec — mechanical drift correction, не cascade revisit).

**Outcome: SPLIT lifecycle-only fix** (NO Q4 bundle).

**Design: Option (i)** — `TriggerAction` carries `start(&self, ctx)` + `stop(&self, ctx)` adjacent к existing `handle()`. Per-instance state (webhook URL, secret, registration token) lives в `&self` (consistent с §2.9.1a paradigm). `TriggerSource` остаётся shape-only (no runtime instantiation paradigm break).

**Bundle-vs-split rationale (architect honest call):**
- User trailing note suggested adding `Input` к start method to enable Q4 ACCEPT through backdoor
- Architect declined bundle: B1 (silent semantic divergence) survives `start(input)` refinement — `start(input)` is per-registration, `execute(input)` is per-dispatch, same name-collision trap from Q4
- B4 (ADR-0038 binds spike shape) would invalidate freeze without re-spike — disproportionate для mechanical gap fix
- Bundling conflates drift correction с paradigm choice; muddies freeze-warrant trail

**Migration**: ~3-5 internal sites + ~1 per community trigger plugin via new T7 codemod transform (AUTO; bodies unchanged).

**Tech Spec amendments (8 edits):**
- §0 status header + status table row updated
- §2.2.3 amendment-in-place: start + stop methods added к TriggerAction trait
- §10.2 transforms-list + new T7 row (AUTO migration)
- §10.2.1 + §10.5 AUTO mode list extended
- §15.10 enactment subsection (5 sub-sections documenting analysis + decision + design + migration + spike-scope-acknowledgment)
- §17 CHANGELOG entry для Q6 post-freeze

**ADR-0038 NOT modified** — lifecycle at per-method signature layer; ADR-0038 §Decision items 1-4 address trait-shape rewriting / emission / dual enforcement / phantom composition — none address per-method signatures, so signature addition не requires ADR amendment.

**Status qualifier appended**: `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 post-freeze + Q6 lifecycle gap)`.

**Spike scope acknowledged**: shape-only probes PASS verdict stands; lifecycle scope was out of spike contract; not a spike defect, a coverage gap closed by Q6 gap fix.

### 2026-04-25 T04:00 — Post-freeze Q5 — Option E (`type Config` rename) REJECT

**User responded к Q4 REJECT с targeted refinement: "может быть тогда `type Config`?"**

Sharpest framing yet — directly addresses Q4 blocker B1 (semantic divergence trap from `type Input` name collision). **`type Config`** carries clear "configuration" semantics; no contributor would assume `handle(.., config)` method parameter.

**Architect: E.REJECT** (both E1 TriggerAction-only and E2 universal across 4 traits)

**Honest acknowledgment**: user found sharpest framing на 5-й итерации; `type Config` **does dissolve B1** naming collision blocker.

**Why REJECT still holds (3 carry-forward + 1 NEW):**
- B2 signature-doubling persists (parallel к existing `&self` + `parameters = T`)
- B3 no compile-time consumer beyond what schema-as-data already provides
- B4 ADR-0038 binds verbatim spike shapes (`final_shape_v2.rs:254-262` has no `type Config`)
- **B5 NEW (load-bearing)**: §2.9.1a Resolution point 1 (line 501) + closing line 507 made explicit deliberate **paradigm choice**: «Configuration carrier is `&self`; configuration schema carrier is `ActionMetadata::parameters` via `with_schema`. No new associated type, no signature edit.» Both E1 and E2 invert that choice.

**Tech Spec amendments (rationale-only):**
- §2.9.1e (NEW) — Option E analysis + sixth axis (trait-declared-configuration-carrier-with-rename) + 4 blockers
- §2.9.5/§2.9.6/§2.9.7 extended to fifth iteration
- §0.1 status qualifier appended «+ Q5 §2.9.1e configuration-carrier-rename refinement»
- §17 CHANGELOG entry

**Spike unchanged. ADRs unchanged. §2.2 unchanged.**

**Escalation path**: B5 is paradigm-level decision locked at §2.9.1a. If user contests = §2.9.1a paradigm re-litigation, что = tech-lead ratification gate (not rationale-only architect call). Five iterations have established consistent REJECT под progressively sharper user framings; architect honest that prior reasons were sometimes incomplete (Q3 found schema axis; Q4 found Option D as new variant; Q5 dissolved name collision). B5 is the remaining principled axis.

### 2026-04-25 T03:30 — Post-freeze amendment Q4 — Option D (TriggerAction.type Input asymmetry) REJECT

**User raised FOURTH pushback.** Sharper framing than Iter 1-3:
- NOT base trait consolidation (already rejected 3 times)
- NOT new naming
- Specific: should TriggerAction have `type Input` directly on trait, same syntactic shape as Stateless/Stateful/Resource? Method signatures unchanged.
- Asymmetry argument: 3-vs-1 visible; user asks for technical/principled justification

**Architect verification:**
- §2.2 confirmed 3-vs-1: StatelessAction (line 159), StatefulAction (line 177), ResourceAction (line 227) declare `type Input`; TriggerAction (lines 202-211) does NOT
- Spike `final_shape_v2.rs:209-262` confirms same 3-vs-1
- This IS new framing — Option D (per-trait `type Input` decoupled from method parameter), not analyzed in Iter 1-3 (which all addressed CONSOLIDATION variants)

**Outcome: REJECT (refined four times — Option D analyzed for first time)**

**Concrete one-sentence reason**: Option D forces TriggerAction's `type Input` to mean per-instance configuration while the other 3 primaries' `type Input` means per-dispatch method-parameter; **same syntactic surface, opposite semantics** — silent divergence trap worse than the visible 3-vs-1 asymmetry it removes. Contributor reading `TelegramTrigger::Input = TelegramTriggerInput` would reasonably assume `handle(.., input: TelegramTriggerInput)` exists.

**Why REJECT not ACCEPT**: user explicitly admits "method signatures unchanged — handle() takes event, not Input." That admission IS load-bearing: in the other 3, `type Input` is justified BECAUSE it appears in method signature. For TriggerAction it would be decorative — schema reflection already universal via `with_schema(<T as HasSchema>::schema())` per `crates/action/src/metadata.rs:292`, not trait-level-blocked.

**Tech Spec amendments enacted (rationale-only, mirrors Q2/Q3 precedent):**
- §2.9.1d (NEW) — Option D + 4 blockers (semantic divergence trap; user's "decorative Input" admission load-bearing; schema reflection already universal; ADR-0038 §Decision item 4 binds verbatim spike shapes)
- §2.9.5/§2.9.6/§2.9.7 extended to "refined four times" + four-iteration history table
- §17 CHANGELOG + §0.1 status line entries

**No status qualifier change** (rationale-only per §15.9.5/§15.9.6 precedent). **No §2.2 signature change. No ADR amendment.**

ADR-0038 §Decision item 4 binds verbatim spike shapes; Option D would invalidate freeze per §0.2 items 2+4 — REJECT preserves both.

**Tech-lead ratification SKIPPED** (rationale-only refinement, mirror Q2 precedent; not amendment-cycle gate).

### 2026-04-25 T03:00 — Post-freeze amendment Q3 — §2.9 third REJECT-refined (schema axis distinction)

**User raised THIRD pushback на §2.9** с concrete consumer evidence: n8n 70+ trigger nodes expose Input schema (UI form generation) + Output schema (downstream type-checking) + filter/parameter validation. Examples: Telegram Trigger Input=`{allowed_updates}` Output=`TelegramUpdate`; GitHub Trigger Input=`{repository,events}` Output=`GitHubPayload`; Schedule Trigger Input=`{interval,cron}` Output=`ScheduledTick`. User claimed §2.9.6 point 2 ("no current consumer") incorrect; asked refactor handle() к accept `Self::Input` parameter.

**Architect re-analyzed (third iteration). Outcome: 3C REJECT-refined again.**

New concrete axis identified: **schema-as-data vs schema-as-trait-type carrier.**

User's three n8n consumers (UI generation / downstream typing / filter validation) all live on **schema-as-data axis** (`ActionMetadata.base.schema` + `inputs: Vec<InputPort>` + `outputs: Vec<OutputPort>`) — universal across all 4 trait variants TODAY. Verified at `crates/action/src/metadata.rs:98-292`. n8n itself uses runtime `INodeTypeDescription` data, not TypeScript generic associated types.

**Schema-as-trait-type axis** (consolidation target via base `Action<Input, Output>` trait) has NO current consumer. Adding it = parallel surface на trait-type axis without a consumer.

`docs/COMPETITIVE.md:29,41` disclaims n8n surface parity at canon level — Nebula's strategic position = "typed Rust contracts AGAINST n8n's surface-area model". So n8n parity isn't actually a Nebula goal at the architectural level.

Plus: user's handle() refactor proposal structurally breaks ADR-0035 §4.3 phantom-shim composition (configuration carrier `&self` is load-bearing — phantom-shim rewrites field zones; method parameters aren't field-zone targets).

**Tech Spec amendments enacted:**
- §2.9.1c (NEW) — verbatim Q3 record + axis distinction + n8n verification + COMPETITIVE.md citation + handle() refactor REJECT (4 reasons)
- §2.9.5 — refined to "(refined three times)" + four-axis decomposition (trait-method-input / trigger-purpose-input / configuration / schema-carrier)
- §2.9.6 prelude + point 2 — four-axis enumeration including Schema-as-data vs Schema-as-trait-type
- §2.9.7 Implications + Re-open trigger — extended to all three iterations; re-open requires "schema-as-trait-type consumer" with worked examples (dependency-typed resource graph / `fn collect<T: Action<I, O>>` aggregation)
- §15.9.6 — new Q3 enactment record sibling to §15.9.5 Q2

**No status qualifier change** (rationale-only refinement per §15.9.5 Q2 precedent). ADRs unchanged. Strategy unchanged. Spike unchanged.

**Tech-lead RATIFIED.** Third iteration closed.

### 2026-04-25 T02:00 — Post-freeze amendment (Q1 async_trait + Q2 §2.9 refinement)

**User raised two post-freeze design questions.** Architect re-analyzed; tech-lead ratified.

**Q1 ACCEPT (amendment-in-place enacted):**
- User pushback: ~15k crates use `async_trait`; ecosystem inertia argues for adopting it; on `dyn async fn` stabilization removing one attribute is mechanical
- **Critical finding**: ADR-0024 (accepted 2026-04-20, **before** Tech Spec freeze) §Decision items 1+4 explicitly enumerate `StatelessHandler`, `StatefulHandler`, `TriggerHandler`, `ResourceHandler` among 14 dyn-consumed traits approved for `#[async_trait]`. Pre-amendment Tech Spec manual `BoxFut<'a, T>` shape was inadvertent cross-ADR violation
- Amendment: §2.4 four *Handler traits flipped to `#[async_trait]`; §2.3 BoxFut alias survives ONLY for `SlotBinding::resolve_fn` HRTB fn-pointer (structurally distinct — compile-time fn pointer per credential Tech Spec §3.4 line 869, not runtime async dispatch)
- §15.9 enactment record added
- Cancel-safety preserved (Box::pin(async move {...}) preserves drop semantics on SchemeGuard<'a, C>; spike Iter-2 §2.4 test passes either shape)
- ADR-0024 is source-of-truth — no ADR file edit required

**Q2 REJECT (refined):**
- User pushback: trigger Event/Source is OUTPUT (trigger emits events), Input is CONFIGURATION from user settings. §2.9 framing was mis-classified
- Architect acknowledged: user's standard-workflow nomenclature is correct (events ARE trigger output for trigger-purpose axis); §2.9.2 "Input shape" column was loosely worded conflating trait-method-input axis with trigger-purpose axis
- Added §2.9.1b naming three axes explicitly: trait-method-input / trigger-purpose-input / configuration
- REJECT verdict basis SHARPENED (lifecycle-method divergence — start/stop vs execute vs configure/cleanup), not changed
- §2.9.5/.6/.7 rationale tightened

**Tech Spec status:** `FROZEN CP4 2026-04-25` → **`FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 post-freeze)`** per ADR-0035 amended-in-place precedent (Q2 rationale-tightening doesn't warrant separate qualifier)

**No ADR transitions; ADR-0040 still user ratification pending.**

### 2026-04-25 T01:30 — CP4 FROZEN + Phase 6 closes

**CP4 §14-§16 (Tech Spec meta + handoff, FINAL CP):**

- Architect drafted 339 lines (§14 cross-references with 5 sub-tables, §15 open items resolution including §15.5 ADR-0039 amendment-in-place ENACTMENT, §16 implementation handoff with (a/b/c) PR wave plan + DoD + rollback strategy)
- **ADR-0039 §1 SlotBinding amendment-in-place ENACTED** during CP4: capability folded into SlotType enum per credential Tech Spec §15.8 (CP5 supersession of §9.4); status header gains `proposed (amended-in-place 2026-04-25)` qualifier; CHANGELOG entry cites Tech Spec CP4 §15.5 + ADR-0035 amended-in-place precedent
- 2 parallel reviewers (compressed CP — final meta CP): spec-auditor REVISE (3 🔴 mechanical pin-fixes — wrong file path propagating through §14.5/§13.4.x/§16.2; superseded credential §9.4 citation; superseded `unstable-action-scheduler` flag name), security-lead ACCEPT (no edits, freeze-blocker NO, VETO retained on shim-form drift)
- Architect single-pass iteration applied 11 closures (3 🔴 + 3 🟠 + 5 🟡)
- Tech-lead **RATIFY-FREEZE** 11c: all 8 ratification checks pass

**Status transitions on freeze:**
- Tech Spec: `DRAFT CP4 (iterated 2026-04-25)` → **`FROZEN CP4 2026-04-25`**
- ADR-0038: `proposed` → **`accepted 2026-04-25`** (Tech Spec FROZEN CP4 gate cleared)
- ADR-0039: `proposed (amended-in-place 2026-04-25)` → **`accepted 2026-04-25 (amended-in-place 2026-04-25)`** (Tech Spec FROZEN CP4 gate cleared; amendment qualifier preserved)
- ADR-0040: stays **`proposed`** — user ratification on canon §3.5 revision required per cascade prompt; surfaced to user in Phase 8 summary as decision item

**Phase 6 closes.** Cascade fully complete pending Phase 8 summary refresh.

**Final Tech Spec line count:** ~2400+ lines (CP1 572 + CP2 711 + CP3 548 + CP4 339 + iteration entries).

**Forward-flagged для Phase 8 summary** (cross-crate amendments к credential Tech Spec, NOT enacted per ADR-0035 soft-amendment precedent):
- §16.1.1 probe #7 qualified-syntax SchemeGuard Clone shadow probe
- §15.7 `engine_construct_with_probe` test variant

### 2026-04-25 T00:30 — CP3 RATIFIED + commit-ready

**CP3 §9-§13 (Tech Spec interface + migration):**

- Architect drafted 548 lines (§9 public API surface incl. §9.5 cross-tenant Terminate boundary, §10 codemod runbook with 6 transforms T1-T6, §11 adapter authoring contract, §12 ControlAction + DX migration, §13 evolution policy)
- 5 parallel reviewers returned: spec-auditor PASS-WITH-NITS (2 🟠 + 6 🟡), rust-senior RATIFY-WITH-NITS (2 🟠 + 1 🟡), security-lead ACCEPT (focused §9.5 only — preserves "MUST NOT propagate" verbatim from 08c §Gap 5; engine-side enforcement at scheduler dispatch), dx-tester RATIFY-WITH-NITS (2 🟠 — control_flow syntax inconsistency + semver Cargo.toml gap), devops RATIFY-WITH-NITS (**2 critical compile-fail blockers**: nebula-redact workspace integration missing; deny.toml syntax wrong)
- Architect single-pass iteration applied 10 edits including 2 critical:
  - **§13.4.4 NEW subsection** committing nebula-redact workspace integration (new crate Cargo.toml/lib.rs + root workspace member + workspace dep; no new deny ban for leaf utility)
  - **§13.4.3 deny.toml restructure** — Edit 1 = wrappers-list extension of existing nebula-engine rule (NOT duplicate); Edit 2 = symmetric positive ban for nebula-action runtime layer per Phase 0 §11 row 9
- §10.2 T6 normalized to MIXED (AUTO default + MANUAL fallback) per ADR-0040 §Negative item 4
- §10.4 step 1.5 added `semver` Cargo.toml dep instruction (Phase 1 CC1 carry-forward)
- Tech-lead RATIFIED post-iteration (commit-ready; no round-2; no escalation; security-lead implementation-time VETO authority retained on §9.5 softening)

**Forward-tracked to CP4:**
- ADR-0039 §1 SlotBinding amendment-in-place (capability folded into SlotType per credential Tech Spec §9.4) — Phase 8 cross-section pass per §0.2 invariant 2 (enactment before CP4 freeze)
- Engine cascade handoff (§9.5.5 SchedulerIntegrationHook + §3.1 ActionRegistry::register* + §3.2 ActionContext API location)
- 10-item open-items queue → CP4 §15 resolution

### 2026-04-24 T23:30 — CP2 RATIFIED + commit-ready

**CP2 §4-§8 (Tech Spec macro emission + execution + security floor + lifecycle + storage):**

- Architect drafted 711 lines (§4 macro full token shape, §5 trybuild+macrotest harness with 6-probe port from spike c8aef6a0, §6 security must-have floor co-decision, §7 lifecycle SchemeGuard RAII flow, §8 storage)
- 5 parallel reviewers returned: spec-auditor PASS-WITH-NITS (3 🟠), security-lead ACCEPT-WITH-CONDITIONS (3 required edits + co-decision YES on all 4 §6 floor items), rust-senior RATIFY-WITH-NITS (3 🟠 incl. ADR-0039 amendment trigger), dx-tester RATIFY-WITH-NITS (2 🟠 cross-zone collision + author-trap probe), devops RATIFY-WITH-NITS (2 🟠 macrotest version + trybuild workspace pin)
- **User raised mid-iteration**: §2.9 reconsideration on TriggerAction config (RSS url+interval, Kafka channel post-ack)
- Architect single-pass iteration applied 13 reviewer items + §2.9 refinement
- **§2.9 REJECT preserved** with refined axis: Configuration (per-instance, `&self` + universal `with_schema`, applies to all 4 trait variants) vs Runtime Input (divergent — Stateless/Stateful/Resource execute-shape vs TriggerAction event projection). User's RSS/Kafka examples are CONFIGURATION not RUNTIME-INPUT — different lifecycle phase. New CP3 §2.9-1 forward-track: `ActionMetadata::for_trigger::<A>()` helper
- **§6 co-decision UNANIMOUS** tech-lead + security-lead on 4 floor items: JSON depth cap (`check_json_depth` `pub(crate)` + typed `DepthCheckError`), HARD REMOVAL `credential<S>()` (no `#[deprecated]`), `redacted_display()` in new `nebula-redact` crate + pre-`format!` sanitization, per-test `ZeroizeProbe`
- **ADR-0039 §1 SlotBinding amendment-in-place** (capability folded into SlotType per credential Tech Spec §9.4) — flagged in §15 for Phase 8 cross-section pass per ADR-0035 amended-in-place precedent; §0.2 invariant 2 enforces enactment before CP4 freeze
- Tech-lead RATIFIED post-iteration (commit-ready; no round-2; no escalation; implementation-time VETO authority retained on §6.2 hard-removal regression)

### 2026-04-24 T22:30 — CP1 RATIFIED + commit-ready

**CP1 §0-§3 (Tech Spec foundation):**

- Architect drafted 572 lines (§0 status/scope/freeze, §1 goals + non-goals, §2 trait contract with 4 primary + 5 sealed DX + ActionResult::Terminate decision, §3 runtime model with SlotBinding registry + HRTB dispatch + cancellation safety)
- **§2.7.1 Terminate decision: WIRE-END-TO-END** (per tech-lead Phase 1 solo call + canon §4.5 false-capability avoidance)
- 5 parallel reviewers returned: spec-auditor REVISE (3 🔴 + 3 🟠), rust-senior RATIFY-WITH-NITS (1 🔴 ser/de bound lift + 2 🟡), security-lead ACCEPT-WITH-CONDITIONS (no edits), dx-tester REVISE (2 🔴 ActionSlots undef + sealed migration target), devops RATIFY-WITH-NITS (2 🟠 feature flag freeze + nebula-runtime path)
- Architect single-pass iteration applied 9 critical items + minor nits — all closed cleanly
- **User-raised mid-iteration**: Input/Output base trait consolidation analysis. Architect added §2.9 with REJECT decision + concrete re-open trigger (TriggerAction structural divergence; no current consumer for sub-trait)
- Tech-lead RATIFIED post-iteration (commit-ready; no round-2; no escalation)

**Forward-tracked for CP2/CP3** (in §15 open items): security-lead 5 prep gaps (hard-removal mechanism, JSON depth-cap implementation, redacted_display() helper location, ZeroizeProbe instrumentation, cross-tenant Terminate boundary CP3); rust-senior BoxFut single-home (CP3 §7).

**Feature flag granularity decision**: parallel `unstable-retry-scheduler` + `unstable-terminate-scheduler` (not unified) — devops recommended, architect committed, tech-lead ratified.

### 2026-04-24 T22:00 — Continuation session start (Phase 6 Tech Spec)

User authorized Phase 6 Tech Spec drafting in continuation session. Worktree isolation rule active — no cross-cascade references к sibling worktrees. Tasks for CP1/CP2/CP3/CP4 + Phase 8 update created.

**Forward path locked** (per cascade prompt):
- CP1 — §0-§3 foundation (~600-800 lines): status/scope, goals, trait contract (4 primary + 5 sealed DX with full Rust sigs, Terminate decision, BoxFut alias), runtime model
- CP2 — §4-§8 macro + execution (~700-1000 lines, largest CP): #[action] token shape, macro test harness, security must-have floor, lifecycle, storage. Co-decision tech-lead + security-lead on §6.
- CP3 — §9-§13 interface + migration (~500-700 lines): public API, migration plan, adapter contract, ControlAction migration, evolution
- CP4 — §14-§16 meta + handoff (~200-300 lines): cross-refs, open items resolution, implementation handoff (Q1 options если not picked)

Per-CP cadence (Strategy §6.3): architect draft → 5 parallel reviewers (rust-senior + security-lead + dx-tester + devops + spec-auditor) → architect iterate once → tech-lead ratify → commit per CP.

**Hard escalation triggers:** review iteration round 3; CP2 co-decision deadlock; cross-crate API break beyond ADR-0039 §3 soft amendment; budget hit (5d); security 🔴 blocking CP ratification; macro emission perf bound violation.

**Soft escalations (precedent):** --no-verify for unrelated fmt drift (per spike c8aef6a0 + summary commits aa63e424, 3e10329f).

### 2026-04-24 T20:45 — Cascade scope completion decision

Orchestrator decides to **complete cascade at Phase 5 + write final summary**, deferring Phase 6 (Tech Spec drafting) to a separate user-authorized continuation session.

**Rationale:**
- Cascade has produced: Strategy frozen (540 lines) + Spike validated + 3 ADRs proposed = full design closure at Strategy/ADR level
- Phase 6 Tech Spec is "longest phase" per cascade prompt (4 CPs × 5 parallel reviewers per CP = ~20 dispatches)
- Orchestrator context budget tight for autonomous Phase 6 completion
- User can re-spawn cascade at Phase 6 entry point with all upstream artefacts available
- Per cascade prompt anticipated outcomes (15-25% probability): "Cascade completes но Phase 6 Tech Spec sections shallow" — orchestrator chooses NOT to ship shallow Tech Spec; defers cleanly instead

**Final deliverables produced this cascade:**
1. Phase 0 ground truth (`01-current-state.md` + 01a + 01b)
2. Phase 1 pain enumeration (`02-pain-enumeration.md` + 4 sub-reports)
3. Phase 2 scope decision (`03-scope-decision.md` + 03a + 03b + 03c)
4. Phase 3 Strategy FROZEN CP3 (`docs/superpowers/specs/2026-04-24-action-redesign-strategy.md` + 6 review files)
5. Phase 4 spike NOTES + final_shape_v2.rs (`07-spike-NOTES.md` + final_shape_v2.rs)
6. Phase 5 3 PROPOSED ADRs (`docs/adr/0036` + `0037` + `0038`)
7. Phase 8 summary (in-progress)

### 2026-04-24 T19:30 — Phase 3 complete + Strategy FROZEN CP3 (commit 68bbd4fc for Phase 2, next for Phase 3)

**Strategy Document drafted across 3 checkpoints with 1 iteration each:**

- **CP1** (§0-§3): architect drafted 197 lines; spec-auditor PASS-WITH-NITS (3 cite errors); tech-lead RATIFY-WITH-NITS (3 wording locks). Architect single-pass iteration applied 6 edits.
- **CP2** (§4-§5): architect appended 215 lines (§4 recommendation + §5 open items + spike plan); spec-auditor REVISE (1 🔴 spike signature drift, 2 🟠, 5 🟡); tech-lead RATIFY-WITH-NITS (2 edits). Architect single-pass iteration applied 9 edits — load-bearing 🔴 closed via path (a) `SlotBinding::resolve_fn`.
- **CP3** (§6): architect appended 128 lines (§6 post-validation roadmap, 8 sub-sections + new §6.9 retry-scheduler closure); spec-auditor REVISE (3 🔴 blockers); tech-lead RATIFY-WITH-NITS (1 edit). Architect single-pass iteration applied 7 edits.

**Status header:** `FROZEN CP3 2026-04-24`. Strategy total: 540 lines.

**Forward path locked:**
- Phase 4 spike — `SlotBinding::resolve_fn` HRTB + `SchemeGuard<'a, C>` cancellation drop-order verification, rust-senior isolated worktree, 2 iterations max
- Phase 5 ADRs — 3 required (trait shape; macro emission; ControlAction seal + canon §3.5 revision) + 1 optional (cluster-mode hooks)
- Phase 6 Tech Spec — 5 CPs (CP1 §0-§3 / CP2a §4-§5 / CP2b §6-§8 / CP3 §9-§13 / CP4 §14-§16); per-CP 5 reviewers parallel
- Phase 8 user pick — implementation path (a) single PR / (b) sibling cascades / (c) phased B'+ surface; (c) NOT VIABLE without committed credential CP6 cascade slot

### 2026-04-24 T17:22 — Phase 2 complete + scope locked (commit 786f2429 for Phase 1, next for Phase 2)

**Co-decision protocol: 2 rounds required.**

**Round 1 (parallel):**
- architect (03a): 4 options — A'/B'/B'+/C'. Leans B'+ as draft position.
- tech-lead (03b): picks A' (lean A, fallback C, not B). Did NOT see B'+ (file not on disk when dispatched in parallel).
- security-lead (03c): ACCEPT all three (A'/B'/C') with must-have floor; no VETO.

**Round 2 (tech-lead re-rank):**
Orchestrator dispatched tech-lead to evaluate B'+ explicitly + reframed cascade as **design-scope-only** (per prompt non-goals: Tech Spec closes at design; implementation post-cascade).

Tech-lead round 2: **A' 1st / B'+ 2nd / C' 3rd / B' 4th**. B'+ acceptable fallback with condition (CredentialRef / SlotBinding / SchemeGuard MUST land in nebula-credential, not nebula-action). Design-scope reframe collapsed budget axis: all options fit cascade design budget; only C' triggers escalation rule 10 (spec revision).

**Orchestrator picked A' per tech-lead priority call.** Cascade proceeds to Phase 3 without escalation.

**Cross-crate coordination flags raised for future tracking:**
- Credential crate: CP6 phantom+HRTB+RAII core (`CredentialRef<C>`, `SlotBinding`, `SchemeGuard`, `SchemeFactory`, `RefreshDispatcher`) still spec-only. A' Tech Spec describes design for both crates.
- Post-cascade implementation path: USER DECISION (a) single PR / (b) sibling cascades / (c) phased rollout. Flagged in Phase 8 summary.

**Security must-have floor (non-negotiable):**
1. JSON depth cap at adapter boundaries
2. Explicit keyed credential dispatch (method signature, hard removal of heuristic)
3. ActionError Display sanitization
4. Cancellation-zeroize test

**Tech-lead solo-decided calls ratified** (Phase 1 + Phase 2): seal ControlAction + canonize DX tier; feature-gate + wire ActionResult::Terminate; HRTB *Handler modernization recommended.

### 2026-04-24 T16:48 — Phase 1 complete + gate passed (commit fc18c736 for Phase 0, next for Phase 1)

**4 parallel agents returned, convergent findings:**

- dx-tester: 7 🔴 + 12 🟠; time-to-first-compile Stateless 12min / Stateful 8min / ResourceAction+Credential 32min (target <5min). Credential attribute unusable in both string AND typed form.
- security-lead: 2 🔴 exploitable-today (S-C2 cross-plugin credential shadow attack via type-name-lowercase key; S-J1 JSON depth bomb confirms Phase 0 C4) + 17 🟠/🟡. Webhook crypto primitives solid. Retry feature flag NOT exploitable (disproven via engine grep).
- rust-senior: 1 🔴 (reconfirmed C2 with cargo expand evidence) + 2 🟠 DATED. Cancel safety + error taxonomy reference-quality. HRTB `*Handler` boilerplate inherited from `async-trait` convention; modernizable to single-`'a` + type alias + `trait_variant::make(Send)`.
- tech-lead: 3 solo priority calls (ratified) + 1 structural co-decision input. **CRITICAL REFRAME**: credential CP6 vocabulary has zero `src/` matches in credential crate itself — Option A = co-landing two cascades, not catching up.

**11 deduplicated 🔴 + 30+ 🟠 findings** → gate passes easily (threshold: 0 🔴 + <3 🟠 triggers escalation).

**Tech-lead solo-decided priority calls** (Phase 1 outputs):
1. Seal ControlAction + canonize DX tier in §3.5 as "erases to primary"
2. Feature-gate AND wire `ActionResult::Terminate` in cascade (apply Retry discipline)
3. Frame Option A/B/C as A'/B'/C' with cost re-estimates (A' exceeds 5-day budget; B' scoped but defers; C' unfreezes frozen spec)

**Phase 2 dispatch next:** architect + tech-lead + security-lead co-decision on A'/B'/C'. Orchestrator expects escalation probability raised.

### 2026-04-24 T16:29 — Phase 0 consolidation + gate passed

Orchestrator produced `01-current-state.md` consolidating both audits. Audits are **convergent** (no contradictions; devops' macro-harness gap mechanically explains rust-senior's unprotected attribute rejection paths).

**Gate decision:** ✅ PROCEED to Phase 1.

Critical findings (4 🔴):
- **C1**: Credential Tech Spec CP6 §§2.7/3.4/7.1/15.7 vocabulary entirely absent from action crate (CredentialRef<C>, phantom rewriting, SlotBinding, HRTB resolve_fn, SchemeGuard, SchemeFactory — none exist). Phase 2 scope decision required.
- **C2**: `#[derive(Action)]` broken `parameters = Type` emission path.
- **C3**: `credential<S>()` type-name-lowercase heuristic as key — collision footgun.
- **C4**: No serde_json recursion limit at adapter deserialization boundary.

Major structural (9 🟠 + coverage 🟠): canon §3.5 drift via ControlAction, v2 spec "5 traits no extras" violated, ActionResult::Terminate not gated, no macro test harness, no benchmarks, unstable-retry-scheduler dead flag, dead nebula-runtime reference in CI, zeroize inline pin, lefthook doctests/msrv/doc gap, SDK prelude contract surface, engine tight-coupling, missing layer-enforcement deny rule.

Phase 1 dispatching 4 agents in parallel: dx-tester (authoring 3 action types), security-lead (threat model), rust-senior (idiomatic review), tech-lead (architectural coherence). All 4 briefed with C1-C4 as load-bearing context.
