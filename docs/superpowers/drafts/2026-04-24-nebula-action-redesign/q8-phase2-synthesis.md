# Q8 Phase 2 — research-driven gap synthesis + amendment plan

**Phase:** 2 of 3 (Phase 1 = 4 parallel research catalogs; Phase 2 = this synthesis; Phase 3 = amendment enactment OR escalation surfaces to user).
**Author:** architect (synthesizer; not decider).
**Inputs read line-by-line:**
- `q8-rust-senior-trigger-research.md` (507 lines) — trigger family + cluster-mode
- `q8-architect-action-research.md` (305 lines) — action core + Temporal axes
- `q8-security-credential-research.md` (516 lines) — credential + auth
- `q8-dx-tester` (no file emitted; orchestrator framing: 0 NEW 🔴 + 4 🟠 + 3 hold-line)
- Tech Spec FROZEN CP4 + Q1 + Q6 + Q7 amendments (`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` lines 1-100, status header)
- Strategy FROZEN CP3 (`docs/superpowers/specs/2026-04-24-action-redesign-strategy.md`)
- ADR-0035 phantom-shim + amend-in-place precedent
- ADR-0038 / ADR-0039 / ADR-0040
- `CASCADE_LOG.md` for cascade context

**Posture:** Phase 2 synthesis. NO amendment enacted in this document. Recommendations sit in front of tech-lead.

---

## §0 Synthesis posture

Phase 1 catalogs are **deliberately wide** (research files have 502 + 305 + 1582 lines worth of pain attribution; ~150+ underlying issues). Phase 2 must:

1. **Deduplicate** — convergent themes named once, cross-attributed.
2. **Categorize each finding** as AMEND / SPIKE+AMEND / DEFER / ESCALATE.
3. **Honestly mark scope** — Tech Spec FROZEN CP4 cannot absorb everything; some findings genuinely exceed amendment authority.
4. **Name a home for every DEFER** per `feedback_active_dev_mode.md` ("before saying 'defer X', confirm the follow-up has a home"). Vague punts are not allowed.
5. **Frame escalations cleanly** — user-decision items get framing, not architect verdicts.

Phase 1 finding total: rust-senior **5 🔴 + 9 🟠 + 9 🟡** (action-cascade-relevant); architect **8 🔴 + 6 🟠 + 9 🟡 + 3 🟢**; security-lead **0 🔴 + 0 🟠 in-scope** (4 🔴 + 8 🟠 in **separate nebula-auth cascade scope**); dx-tester **0 NEW 🔴 + 4 🟠 + 3 hold-line**. Deduped to **15 unique findings** in §1 below.

---

## §1 Gap consolidation table

Each finding cited once. Sources column shows convergent attribution. Recommendation column commits to one of: **AMEND** (Tech Spec amendment-in-place per ADR-0035 precedent — same pattern as Q6/Q7) / **SPIKE+AMEND** (validate new shape in iter-3 isolated worktree first, then amend) / **DEFER** (note in §15 with named cascade slot per `feedback_active_dev_mode`) / **ESCALATE** (exceeds amendment scope; user decision needed).

| ID | Sources | Description | Severity | Recommendation |
|----|---------|-------------|----------|----------------|
| **F1** | rust-senior #1 + architect §7.1 (poll cursor); §4.2 state shape | **Poll cursor in-memory unsafe for high-value integrations.** `PollAction::Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync` bound *suggests* persistence; runtime stores cursor as stack-frame local. Production doc (`poll.rs:744-754`) explicitly says "NOT acceptable for payments / audit / CRM." Trait surface lies about its persistence contract; data loss with no telemetry signal. | 🔴 | **AMEND** — Tech Spec §8.1.2 already documents this honestly (Q7 I1 amendment). What's missing is a **cursor-honesty signal at the trait surface**. Two viable amendments: (a) add `PollDurability::{Ephemeral, Durable}` declaration on `PollAction` requiring authors to opt into one; (b) lock placeholder `pub trait CursorStore { /* TBD */ }` in §8.3 boundary so future cluster-mode cascade has a forward target. (b) is mechanical (no spike); (a) requires SPIKE because new associated-const interacts with `*Sealed` blanket-impl. Recommend (b) at this CP and DEFER (a) until cluster-mode cascade picks. |
| **F2** | rust-senior #2 + architect §7.1.7 (bulk-op idempotency) + Temporal §2 idempotency-key axis | **No idempotency-key contract on inbound surfaces.** `WebhookAction::handle_request` returns `TriggerEventOutcome::{Skip, Emit, EmitMany}` with no idempotency key. Stripe/GitHub retry on 5xx → duplicate workflow executions. `BatchAction::process_item` similarly has no per-item key. Single most-cited n8n pain class. | 🔴 | **AMEND** — single `IdempotencyKey` typed contract in cascade scope. Recommend: lock `IdempotencyKey` newtype in §2.6 + extend `WebhookAction` and `BatchAction` (and `PollAction::DeduplicatingCursor` already has it) with optional `fn idempotency_key(&self, payload: &Self::Event) -> Option<IdempotencyKey>` hook. Engine-side dedup-store is cluster-mode cascade scope (§1.2 N4 already names home); the **action-side hook** is amendment scope per the same Q6 lifecycle-on-trait pattern. **SPIKE candidate** because hook interacts with `*Sealed` peer-trait shape per ADR-0040. |
| **F3** | rust-senior #3 + architect §3.1 + Temporal Schedule | **No ScheduleAction / cron trigger family.** n8n missed-cron-fires is hot data-loss class (`#23906`, `#25057`, `#27103`, `#27238`, `#23943`). Windmill has Schedule as separate kind. Nebula has TriggerAction.start/stop and PollAction.poll_config (interval, not cron syntax) but no schedule-ledger semantics, no missed-fire replay, no catch-up policy. | 🔴 | **DEFER** with named home — engine cluster-mode cascade per Strategy §3.4 row 3 (line 172). Schedule-ledger is engine concern; ScheduleAction trait surface is a future cascade scope (recommend: name "Trigger family expansion cascade" alongside cluster-mode cascade in CASCADE_LOG.md). Adding ScheduleAction now would: (1) violate ADR-0040 §0 sealed-DX 5-trait enumeration without canon §3.5 re-revision; (2) require schedule-ledger + missed-fire + catch-up runtime which is engine-cascade scope. **ESCALATE if user wants this in current cascade** — would exceed amendment authority per ADR-0040 §0.2 invariant 2 (no ADR mid-amendment) AND would force re-spike. |
| **F4** | rust-senior #4 (external-reg idempotency) | **External-registration idempotency not in activation contract.** `WebhookAction::on_activate` returns opaque state (`Send + Sync + Clone`). On crash mid-activation, next start re-registers → user has 2 GitHub webhooks pointing at same path. n8n `#24056`/`#24433`/`#23893` are persistent multi-year bug class. | 🟠 | **DEFER** with named home — engine cluster-mode cascade per Strategy §3.1 component 7 (`on_leader_*` hooks already reserved). Recommend: lock placeholder `pub trait ExternalSubscriptionLedger { /* TBD */ }` in §8.3 NOW (same discipline as F1 (b)) so cluster-mode cascade has forward target. Mechanical, no spike. |
| **F5** | rust-senior #5 (QueueAction) | **No QueueAction / broker trigger family** (Kafka, NATS, SQS, MQTT, RabbitMQ). TriggerAction shape-2 accommodates but no offset-management, no consumer-group identity, no ack/nack semantics. Windmill ships these as separate trigger kinds. | 🔴 (rust-senior); 🟠 (architect synthesis — same shape as F3) | **DEFER** — same home as F3. Trigger family expansion cascade. n8n broker-trigger pain class is real (`#14979`/`#19877`/`#28605`) but engine-side queue infrastructure is its own scope. **ESCALATE if user wants in cascade** — same blockers as F3. |
| **F6** | architect §7.1.1 (ItemLineage) | **`ItemLineage` typed surface is absent.** n8n's `pairedItem` per-item provenance class is one of most-filed forum-error classes (5+ forum threads cited). `MultiOutput` / `Branch` variants in `ActionResult` (§2.7.2) carry no lineage metadata back to input items. Nebula has structurally inherited the n8n pain class. | 🔴 | **ESCALATE** — scope expansion. Adding `ItemLineage` requires: (a) net-new associated type or context-extension on `ActionContext` OR (b) extension to `ActionResult` enum (which would re-pin §2.7.2 — ADR-0038 §Decision item 4 binds spike-locked variants). Either path violates §0.2 invariants 1 (Strategy revision — Strategy §3.4 doesn't enumerate ItemLineage as deferred row) AND 4 (spike-shape divergence). User decision needed: (i) add as cascade scope (re-spike + Strategy §3.4 row); (ii) defer to dedicated future cascade; (iii) canon-state Nebula does NOT commit to per-item lineage in v1. |
| **F7** | architect §7.1.2 (determinism) + Temporal §2 ctx.now/random/uuid | **Determinism contract is undeclared.** Temporal NDE is #1 user-facing failure even at 5+ years SDK maturity; SDKs force `ctx.now()` / `ctx.random()` / `ctx.uuid()`. Nebula `ActionContext` (spike `final_shape_v2.rs:205-207`) carries only `creds: &'a CredentialContext<'a>`. Author can `SystemTime::now()` and `rand::random()` freely. | 🔴 | **ESCALATE** — canon-level posture. Either: (i) canon-state "Nebula does not commit to replay-based durable execution; graph-edge persistence is the durability story" — no amendment needed except a §1.2 N-row + COMPETITIVE.md cross-cite; (ii) lock determinism contract NOW — `ctx.now()` / `ctx.random()` / `ctx.uuid()` on ActionContext + clippy-ban on `SystemTime::now()` + `rand::random()` in action bodies — and accept that Nebula moves toward replay-future. Path (i) is the architect's reading of `docs/COMPETITIVE.md` line 41 ("typed Rust integration contracts + honest durability"). User must pick. |
| **F8** | architect §7.1.3 (AI sub-node traits) | **Sub-node typed traits (ChatModel, Memory, Tool, Retriever) absent.** AI Agent ecosystem is fastest-churn class in n8n (`#24397`/`#21740`/`#28215`/`#26202`/`#27805`). Tech Spec has no surface for sub-node wiring. | 🔴 | **DEFER** with named home — **future AI cascade** (does not exist yet — recommend creating placeholder slot in CASCADE_LOG.md "AI / sub-node typed traits cascade"). Adding NOW would: (1) require canon §3.5 revision (5th primary OR new DX tier); (2) require Strategy revision (§3.4 doesn't enumerate); (3) re-spike. **ESCALATE if user wants in cascade scope.** |
| **F9** | architect §7.1.4 (per-action concurrency) | **Per-action-type concurrency knob deferred to engine cascade with no action-layer hook.** n8n `#21376`/`#21817`/`#28488`/`#26569` (WooCommerce 20× amplification) is unaddressed at action surface. Temporal `temporal#7666` 14 upvotes — Temporal still doesn't have it. | 🔴 (architect) → 🟠 after dedup with F2 | **AMEND** — add `fn concurrency_limit(&self) -> Option<usize>` hook on `ActionMetadata` per Q6 lifecycle-on-trait pattern. Engine consumes at scheduler dispatch (cluster-mode cascade owns enforcement). Mechanical addition; no spike (parallel to existing `with_schema`). |
| **F10** | architect §7.1.5 (streaming pipeline) | **Streaming pipeline primitive absent.** Memory leaks with large datasets (`#20124`/`#16862`/`#15269`) trace back to lack of bounded-channel backpressure. PollAction is event-by-event for triggers, not mid-graph stream operators. | 🔴 | **DEFER** — future cascade. Same scope as F8 — net-new primary trait OR new DX tier requires canon §3.5 revision + Strategy revision + spike. **ESCALATE if user wants in cascade.** Recommend: name slot "Streaming / processor cascade" in CASCADE_LOG.md. |
| **F11** | architect §7.1.6 (Saga / compensation) | **Saga / compensation surface absent.** ResourceAction `cleanup` is per-resource scope-exit, not workflow-level rollback. ActionResult has no `Compensate` / `Rollback` variant. Critical for financial / bulk-op workflows. | 🔴 | **DEFER** — future cascade. Adding NOW requires `ActionResult` enum re-pin (ADR-0038 binds variants). Recommend: name slot "Saga / compensation cascade" in CASCADE_LOG.md. **ESCALATE if user wants in cascade.** |
| **F12** | architect §7.1.8 (workflow-version migration) | **Workflow-version save-time pin migration absent.** n8n authors hand-roll `if (typeVersion < N)` branches. Nebula §13.1 trait-level deprecation; no per-action workflow-pin migration. All running executions must finish on old version before deploy (acceptable for batch-deploy). | 🟠 | **AMEND** — add §16.5 cascade-final precondition row: "executions-in-flight migration story documented OR Q1 path commits to batch-deploy + soak". Mechanical doc closure. |
| **F13** | rust-senior #6 + architect §4 (engine persistence boundary) | **Engine-side persistence boundary not locked.** Cluster-mode cascade activates with 3 named hooks (`IdempotencyKey`, `on_leader_*`, `dedup_window`) but no engine trait for cursor persistence, leader-state store, external-subscription ledger. Cascade gets a blank check. n8n persistent multi-year bugs (`#15878`/`#27416`/`#27103`) trace back to piecemeal engine design. | 🟠 | **AMEND** — lock placeholder engine traits NOW in §8.3 boundary: `pub trait CursorStore { /* TBD — cluster-mode cascade fills */ }` (closes F1), `pub trait ExternalSubscriptionLedger { /* TBD */ }` (closes F4), `pub trait LeaderStateStore { /* TBD */ }`. Mechanical (placeholder shapes only); no spike. Names the home so cascade isn't a blank check. |
| **F14** | rust-senior #7 (long-lived reconnect framework) | **Long-lived trigger reconnect framework absent.** TriggerAction shape-2 (run-until-cancelled) is canonical home for WebSocket / MQTT / Postgres LISTEN / IMAP IDLE / MCP. All such triggers need: connect, watchdog, exponential-backoff reconnect, circuit-breaker, lifecycle-trace correlation. n8n `#26812`/`#27867`/`#27071` are silent-stop class. | 🟠 | **DEFER** — future cascade. nebula-resilience already exists; cross-cascade integration is its own scope. Name slot "Trigger resilience integration cascade" in CASCADE_LOG.md. Mechanical addition would be premature. |
| **F15** | rust-senior #8 + #9 + #10 + #14 + dx-tester (per orchestrator framing — 4 🟠) | **Mechanical doc / pitfalls.md additions** (URL stability invariant; cross-process draining; activation atomicity; DX-peer authoring meta-pattern; pitfalls candidates per rust-senior #10). Plus dx-tester's 4 🟠 (parameter UI scope; activepieces type-safe pieces; developer-first claims — assumed mechanical per orchestrator framing). | 🟠 (aggregated) | **AMEND** — Tech Spec §2.6 add DX-peer authoring meta-pattern subsection; §11 adapter authoring contract extended; pitfalls candidates promoted to `docs/pitfalls.md` (separate file edit). All mechanical doc closure; no spike. |

**Distribution summary (15 findings):**

| Bucket | Count | IDs |
|--------|-------|-----|
| AMEND | **5** | F2, F9, F12, F13, F15 |
| SPIKE+AMEND | **2** (F1+F2 candidates if user wants stronger contracts; otherwise AMEND only) | F1 (path b only — placeholder trait); F2 |
| DEFER (with named home) | **5** | F3, F5, F8, F10, F11, F14, F4 — 7 actually |
| ESCALATE | **3** (canon-level / scope expansion) | F6 (ItemLineage), F7 (determinism), and conditional ESCALATE on F3/F5/F8/F10/F11 if user wants any in current cascade |

Note: F4 is DEFER with placeholder lock, so it's split across F4 (DEFER) + F13 (AMEND placeholder); same for F1 path (b).

Recounted: **AMEND 5** (F2 hook only / F9 / F12 / F13 / F15) + **SPIKE+AMEND 1** (F2 if user wants typed `IdempotencyKey` newtype validated against sealed peer chain) + **DEFER 7** (F1 path-a, F3, F4, F5, F8, F10, F11, F14) + **ESCALATE 2** (F6, F7) = **15 total**. Item totals match.

---

## §2 Cross-cutting design decisions

For each convergent theme, identify the SINGLE design decision that resolves multiple findings.

### §2.1 Idempotency contract design (closes F2 + F4 + F13)

**Theme:** three angles — webhook handler retry (Stripe HMAC), bulk-op per-item dedup (BatchAction), Temporal idempotency-key plumbing — all need a typed `IdempotencyKey` contract.

**Recommendation:** **Single trait-level newtype** + **per-action hook** + **engine-side dedup-store deferred**.

```rust
// Lock in §2.3 scratch types (parallel to BoxFut alias):
pub struct IdempotencyKey(pub Cow<'static, [u8]>);

// Per-action hook — opt-in (returns Option):
trait WebhookAction {
    // ... existing ...
    fn idempotency_key(&self, payload: &Self::Event) -> Option<IdempotencyKey> { None }
}

trait BatchAction {
    // ... existing ...
    fn item_idempotency_key(&self, item: &Self::Item) -> Option<IdempotencyKey> { None }
}

// PollAction's DeduplicatingCursor<K, C> is already idempotency-keyed via K — no change.

// Engine-side dedup-store: §8.3 placeholder
pub trait DedupStore { /* TBD — cluster-mode cascade fills */ }
```

**Single contract per surface; engine reconciles via cluster-mode cascade DedupStore.** SPIKE+AMEND if user wants the newtype validated against sealed-DX peer-chain compose (per ADR-0040 §0.2 invariant 2).

**Trade-off:** simpler than per-trait IdempotencyKey associated type (rust-senior + architect Phase 1 readings converge on `IdempotencyKey` as data, not type-system axis — research evidence does not justify trait-type-axis carrier per `feedback_third_pushback_carrier_axis.md`).

### §2.2 Determinism contract decision (closes F7)

**Theme:** Temporal NDE pain × Nebula ActionContext silence.

**Recommendation:** **Phase 8 framing presents both paths; tech-lead picks.**

- **Path (i) — canon "no replay":** Nebula commits to graph-edge persistence; durability story is engine-cascade ExecutionRepo per canon §11.3. Action authors retain `SystemTime::now()` / `rand::random()` freedom. Add §1.2 N-row "Determinism contract for replay-based durable execution" → DEFER to **never** (canon position) + COMPETITIVE.md line 41 cross-cite.
- **Path (ii) — canon "replay-future":** lock determinism contract NOW. Add `ctx.now()` / `ctx.random()` / `ctx.uuid()` on ActionContext + clippy-ban on direct calls in action bodies. Re-spike to validate primitive composition.

**Recommendation: path (i)** per `docs/COMPETITIVE.md` line 41 ("typed Rust integration contracts + honest durability") and architect Phase 1 §2.1 verdict ("Nebula and Temporal occupy structurally different positions — alignment is honest"). **But this is canon-level** — tech-lead must ratify, not architect.

### §2.3 State persistence — cursor honesty + cluster traits placeholder lock (closes F1 + F4 + F13)

**Theme:** PollAction::Cursor bound suggests persistence; runtime ephemeral; cluster-mode cascade gets blank check on engine traits.

**Recommendation:** **Two-pronged amendment.**

1. **Cursor honesty:** §2.6 PollAction trait doc gains explicit warning + §16.5 cascade-final precondition gains row "PollAction author who needs cross-restart durability MUST EITHER document acceptance OR target cluster-mode cascade ETA before shipping production integration." Mechanical doc.
2. **Engine trait placeholder lock:** §8.3 boundary section gains:
   ```rust
   // Forward declaration — cluster-mode cascade implements.
   // Action-cascade locks the surface so the future cascade has a stable target.
   pub trait CursorStore { /* TBD — cluster-mode cascade per Strategy §6.6 */ }
   pub trait ExternalSubscriptionLedger { /* TBD */ }
   pub trait LeaderStateStore { /* TBD */ }
   pub trait DedupStore { /* TBD */ }
   ```

**Trade-off:** placeholders look weird but they prevent the "blank check" failure mode rust-senior §7 explicitly names (n8n's persistent multi-year bugs trace to piecemeal engine design). Per `feedback_active_dev_mode.md` ("never settle for deferred without named home") — placeholder NAMES the home.

### §2.4 AI sub-node traits (closes F8)

**Theme:** ChatModel / Memory / Tool / Retriever trait family absent; n8n fastest-churn class.

**Recommendation:** **DEFER** to dedicated AI cascade — name slot in CASCADE_LOG.md NOW. Phase 8 framing notes this as a known major scope item that does NOT block action cascade closure. **ESCALATE only if user wants in current cascade scope** (which would force canon §3.5 re-revision + Strategy revision + spike — disproportionate).

### §2.5 ItemLineage (closes F6)

**Theme:** per-item provenance; n8n most-filed forum-error class.

**Recommendation:** **ESCALATE** — user-decision item. Three viable paths:

- **(α)** Add `ItemLineage` to `ActionContext` extension (`ctx.lineage()`) — context-extension scope, requires Strategy §3.4 row + spike of context-borrow-shape under cancellation. AMENDABLE in current cascade if user authorizes.
- **(β)** Defer to dedicated cascade — name slot "Item lineage / data-flow tracking cascade" in CASCADE_LOG.md.
- **(γ)** Canon-state Nebula does NOT commit to per-item lineage in v1 (analogous to F7 path (i)).

**Architect framing:** (β) is honest per `feedback_active_dev_mode.md` — Nebula has not yet committed to per-item-tracking semantics; declaring v1 absence preserves freedom for future design. (α) is amendable but premature.

### §2.6 ScheduleAction / QueueAction / WebSocketAction primaries (closes F3 + F5)

**Theme:** rust-senior recommends adding these as new sealed-DX peers. Architect synthesis: each requires canon §3.5 re-revision (currently 5 sealed-DX traits per ADR-0040) + spike.

**Recommendation:** **DEFER** to dedicated trigger-family-expansion cascade. Name slot in CASCADE_LOG.md. Tech Spec §2.6 gains DX-peer authoring meta-pattern subsection (closes rust-senior §3 finding) so future peers have a documented pattern to follow.

**ESCALATE** if user wants any in current cascade — forces canon revision + ADR amendment per ADR-0040 §0.2.

---

## §3 Phase 3 amendment scope

Two sub-buckets per the prompt structure.

### §3.1 AMEND-IN-CASCADE bucket (Tech Spec amendment-in-place per ADR-0035 precedent)

Following the Q6 / Q7 precedent (status header qualifier chain; §15.X enactment record; CHANGELOG entry; CHANGELOG appendix close). **5 amendments + 1 conditional spike+amend:**

| # | Finding | Section affected | Form |
|---|---------|------------------|------|
| **A1** | F2 idempotency hook | §2.6 WebhookAction + §2.6 BatchAction + §2.3 IdempotencyKey newtype | Add `fn idempotency_key()` opt-in hook + new `IdempotencyKey(Cow<[u8]>)` type. Mechanical addition; default-impl returns None. **SPIKE candidate** if user wants typed validation against sealed-DX peer-chain compose. |
| **A2** | F9 per-action concurrency | `ActionMetadata` builder | Add `fn concurrency_limit(&self) -> Option<usize>` accessor; `with_concurrency_limit(usize)` builder method. Engine consumes at scheduler dispatch (cluster-mode cascade enforces). |
| **A3** | F12 workflow-version migration | §16.5 precondition + §13.1 narrative | Add §16.5 row "executions-in-flight migration story documented OR Q1 path (a) batch-deploy + soak commitment". §13.1 narrative gains paragraph on workflow-pin migration framing for post-1.0 work. |
| **A4** | F13 engine trait placeholder lock | §8.3 boundary | Add 4 placeholder `pub trait` declarations: `CursorStore`, `ExternalSubscriptionLedger`, `LeaderStateStore`, `DedupStore`. Bodies are `/* TBD — cluster-mode cascade per Strategy §6.6 */`. Closes "blank check" failure mode. |
| **A5** | F15 mechanical docs | §2.6 DX-peer authoring meta-pattern + §11 adapter contract + new pitfalls.md entries | (a) §2.6 subsection on Schedule/Queue/WebSocket peer-authoring pattern; (b) §11 cross-process draining vs in-process draining note + URL stability invariant; (c) `docs/pitfalls.md` 6 entries per rust-senior §6 finding #10 (PollAction `deny_unknown_fields`; `initial_cursor` seed-from-now; DeduplicatingCursor `max_seen` sizing; signature-before-parse; long-lived reconnect requirement; `tokio_unstable` trap). |

**Total: 5 amendments + 1 conditional spike (A1).**

**Status header qualifier expected:** `(amended-in-place 2026-04-25 — Q8 research-driven gap closure per §15.12 — idempotency hook + per-action concurrency + executions-in-flight migration + engine trait placeholders + DX-peer authoring + 6 pitfalls entries)`

### §3.2 DEFER WITH HOME bucket

Each row gets a committed cascade slot per `feedback_active_dev_mode.md` discipline.

| # | Finding | Cascade slot | Status in CASCADE_LOG.md |
|---|---------|--------------|--------------------------|
| **D1** | F1 path-(a) cursor durability declaration | Engine cluster-mode coordination cascade (already named at Strategy §6.6) | Already named; F13 placeholder ensures forward target |
| **D2** | F3 ScheduleAction primary | NEW: "Trigger family expansion cascade" | Recommend: add row to CASCADE_LOG.md `### Cross-cascade awareness` section + CASCADE_LOG.md cross-link from action cascade summary |
| **D3** | F4 external-registration ledger | Engine cluster-mode coordination cascade | F13 placeholder closes |
| **D4** | F5 QueueAction primary (Kafka/NATS/SQS/MQTT) | Same as D2 | Same |
| **D5** | F8 AI sub-node traits (ChatModel/Memory/Tool/Retriever) | NEW: "AI / sub-node typed traits cascade" | Recommend: name slot. Coordinator with future LLM/MCP work. |
| **D6** | F10 streaming pipeline primitive | NEW: "Streaming / processor cascade" | Recommend: name slot. Coordinated with `nebula-eventbus` graduation. |
| **D7** | F11 Saga / compensation | NEW: "Saga / workflow-rollback cascade" | Recommend: name slot. |
| **D8** | F14 long-lived reconnect framework | NEW: "Trigger resilience integration cascade" (cross-cuts nebula-resilience) | Recommend: name slot. |

**Total: 8 deferrals** (D1 + D3 already named; D2 + D4-D8 require new CASCADE_LOG.md slot rows). The CASCADE_LOG.md edit itself is **out of action-cascade scope** (it's an orchestrator-managed file); architect surfaces these as Phase 8 summary rows for orchestrator/user to absorb.

### §3.3 Cross-bucket totals

- **5 AMEND** + 1 conditional spike (= 5-6 amendments enacted in Phase 3)
- **8 DEFER WITH HOME** (cascade slots named; engine-trait placeholders close 2 of them)
- **2 ESCALATE** (F6, F7) — user-decision items framed in §6 below
- **0 unresolved** — every finding has a recommendation.

---

## §4 Spike iter-3 dispatch decision

### §4.1 New shapes proposed

Three candidate new shapes from §3.1:

1. **`IdempotencyKey(Cow<[u8]>)` newtype + opt-in hooks on WebhookAction + BatchAction** (A1).
2. **`concurrency_limit()` ActionMetadata builder method** (A2). NOT a new trait shape — additive accessor on existing struct.
3. **4 placeholder engine traits** (`CursorStore`, `ExternalSubscriptionLedger`, `LeaderStateStore`, `DedupStore`) in §8.3 (A4). Bodies are `/* TBD */` — no shape to validate.

### §4.2 Recommendation

**SPIKE iter-3 NOT REQUIRED for A2/A3/A4/A5. CONDITIONAL SPIKE for A1.**

- **A2** is mechanical accessor on existing ActionMetadata struct — same shape as `with_schema(...)`; no new compose risk.
- **A3** is doc + precondition row.
- **A4** is `pub trait Foo {}` placeholders — empty bodies cannot violate compose.
- **A5** is doc + pitfalls.md entries.
- **A1** introduces a new newtype + per-trait hook; default-impl returns None. The hook is opt-in so `*Sealed` blanket-impl chain is unaffected. **However**, if user wants the hook to be **required** (no default-impl) for production correctness, then SPIKE iter-3 is recommended:

**Conditional SPIKE iter-3 scope (if user requires non-opt-in idempotency contract):**
- Worktree-isolated branch
- Hand-expand `#[action(...)]` macro for: WebhookAction with IdempotencyKey hook + BatchAction with item_idempotency_key hook + StatelessAction WITHOUT (control case)
- Verify sealed-DX peer-chain compose (3 actions parallel)
- 1 compile-fail probe: WebhookAction missing idempotency_key impl when default-impl removed → expected `error[E0046]`
- 1 functional test: hook returns None → engine treats as no-dedup; hook returns Some(key) → dedup-store call observed

**Architect default recommendation:** A1 with **default-impl returns None** (opt-in) — no spike needed; matches Q6 lifecycle-on-trait pattern (start/stop with engine-driven default semantics).

**Tech-lead picks:** opt-in (default-impl) → no spike; required (no default-impl) → spike iter-3.

---

## §5 §2.9 sixth iteration check

§2.9 has been REJECTED 5 times across:
- CP1 user pushback (initial Trigger-asymmetry)
- CP2 user pushback (per-instance config — Q2)
- post-freeze Q3 (n8n consumer evidence — schema-as-data axis)
- post-freeze Q4 (Option D `type Input` directly on TriggerAction — 4-axis)
- post-freeze Q5 (Option E `type Config` rename — paradigm choice §2.9.1a)

**Q8 surfaced new evidence:** Temporal axes (workflow.input ≠ activity.input — different lifecycle phases) + n8n action class catalog (~57 distinct classes; ~150 underlying issues).

### §5.1 Does Temporal evidence justify §2.9 re-open?

Architect Phase 1 §6.2 reading: "**Temporal recognizes input shapes diverge across activity vs workflow vs signal vs query lifecycle phases. Temporal does NOT consolidate them into a base trait.**" This is **precedent for §2.9 REJECT**, not against it.

§2.9.1c (Q3) already names the four-axis decomposition: Method-Input/Output, Configuration, Trigger-purpose, Schema-as-data. Temporal evidence reinforces all four:
- Method-Input axis: Temporal Activity input ≠ Workflow input ≠ Signal input ≠ Query input → mirrors Nebula's StatelessAction::execute(input) ≠ TriggerAction::handle(event) divergence.
- Schema-as-data: Activepieces' Zod schemas are runtime data, not generic types → mirrors §2.9.1c "schema-as-data axis is universal across 4 traits TODAY."

### §5.2 Does n8n evidence justify §2.9 re-open?

n8n consumer evidence already cited at Q3 (`feedback_third_pushback_carrier_axis.md`). Q8 surfaced no NEW consumer category beyond what §2.9.1c records.

### §5.3 Verdict on §2.9

**ZERO re-open triggers from Q8.** Verdict closure preserved. Architect Phase 1 §6.4 explicitly states: "No new axis surfaces from n8n / Temporal research that would unblock §2.9 consolidation. Verdict: §2.9 REJECT (refined three times) closure preserved." Phase 8 should NOT re-open §2.9.

**No amendment proposed for §2.9 in Phase 3.**

---

## §6 Escalation candidates

Findings that exceed amendment authority + need user decision. Each framed as decision item with options.

### §6.1 Escalation E1 — F7 determinism posture

**Question:** Does Nebula commit to replay-based durable execution (Temporal-style) or to graph-edge persistence (current canon §11.3)?

**Why escalate:** canon-level posture decision. Either answer requires canon §11.3 / `docs/COMPETITIVE.md` cross-cite OR new ActionContext primitives + clippy bans + spike.

**Options:**

| Option | What it locks | Trade-off |
|--------|---------------|-----------|
| **(i) Canon "no replay"** | Add §1.2 N-row + COMPETITIVE.md line 41 cross-cite + closure note | Honest about Nebula's actual durability story; closes Temporal-comparison without future obligation |
| **(ii) Canon "replay-future"** | Lock `ctx.now()` / `ctx.random()` / `ctx.uuid()` on ActionContext + clippy-ban on `SystemTime::now()` + `rand::random()` direct use + spike iter-3 to validate primitive compose | Opens replay-future at cost of: more ActionContext surface; clippy-ban migration cost on existing actions; spike work; potential interaction with cancellation-zeroize floor item |
| **(iii) Canon "deferred to future cascade"** | Add §1.2 N-row + name "Durable execution / replay cascade" slot in CASCADE_LOG.md | Punts decision; preserves both options |

**Architect framing:** **(i)** is the honest reading of canon `docs/COMPETITIVE.md` line 41 ("typed Rust integration contracts + honest durability") and matches Nebula's graph-edge state model. **(ii)** is the maximalist position that opens the largest design space but has highest cost. **(iii)** is the punt.

**Architect recommendation: (i)** — honest now, future cascade remains free to revisit.

**Tech-lead/user picks.**

### §6.2 Escalation E2 — F6 ItemLineage scope

**Question:** Does Nebula commit to per-item provenance tracking in v1?

**Why escalate:** scope expansion + paradigm decision. Either path expands action cascade or names a future cascade.

**Options:**

| Option | What it locks | Trade-off |
|--------|---------------|-----------|
| **(α) Add ItemLineage to ActionContext** | `ctx.lineage()` extension + Strategy §3.4 row + spike to validate borrow-shape under cancellation | Closes n8n's most-filed forum-error class structurally; cost = re-spike + Strategy revision (violates §0.2 invariant 1 unless ratified) |
| **(β) Defer to dedicated cascade** | Name "Item lineage / data-flow tracking cascade" slot in CASCADE_LOG.md | Honest deferral; cascade stays scoped; but n8n pain class structurally inherited until cascade lands |
| **(γ) Canon "no per-item lineage"** | Add §1.2 N-row + canon statement | Closes design freedom permanently; risk if Nebula targets data-pipeline workloads |

**Architect framing:** **(β)** is honest per `feedback_active_dev_mode.md` — name the home, defer the work. **(α)** is technically amendable but premature without consumer evidence beyond n8n parity (which COMPETITIVE.md disclaims as non-goal). **(γ)** is too aggressive for v1.

**Architect recommendation: (β)** — name future cascade slot.

**Tech-lead/user picks.**

### §6.3 Conditional escalations — F3 / F5 / F8 / F10 / F11 (trigger-family + AI + streaming + Saga)

**Question:** Should ANY of these be in current cascade scope vs deferred to future cascades?

**Architect framing:** **all five default to DEFER per §3.2 with named cascade slots.** Bringing any into current scope forces:
- ADR-0040 §0.2 invariant violation (canon §3.5 re-revision)
- Strategy §3.4 revision (no current row)
- Spike iter-3 validation
- Tech Spec freeze invalidation per §0.2 invariant 1

**Architect recommendation: ALL DEFER.** User MAY pull any individually into scope, in which case escalation cost is per item. Architect surfaces each as conditional escalation in Phase 8 summary.

**Tech-lead/user picks.**

---

## §7 Cascade closure recommendation

Three frames consistent with prior closures (`feedback_post_closure_audit_bundle_pattern.md`).

### §7.1 Frame analysis

| Frame | When applicable | Honest? |
|-------|-----------------|---------|
| **AMENDED-CLOSED-AGAIN** (lighter touch; most findings DEFER WITH HOME) | If user accepts §3.1 5-amendment bundle + §3.2 8-deferrals + §6.1 (i) determinism + §6.2 (β) ItemLineage defer | YES — all findings categorized, cascade-final criteria intact |
| **AMENDED-WITH-NEW-SCOPE** (cascade scope expands к include AI / ItemLineage / determinism) | If user picks §6.1 (ii) determinism OR §6.2 (α) ItemLineage OR pulls F3/F5/F8/F10/F11 into scope | Only honest if user explicitly authorizes scope expansion + new spike iter-3 |
| **ESCALATED** (multiple findings exceed amendment authority) | If user defers all decisions to cascade-summary phase | Honest punt; orchestrator surfaces to user; cascade stays in "Phase 8 user-pick required" status |

### §7.2 Architect recommendation

**AMENDED-CLOSED-AGAIN** with the architect-default §6.1 (i) + §6.2 (β) escalation positions accepted by user.

**Rationale:** Q8 was post-FROZEN deep audit. 15 unique findings. Of those:
- **5 mechanical amendments** (§3.1) — fits Q6/Q7 precedent.
- **8 named deferrals** (§3.2) — every cascade slot named; no blank check.
- **2 escalations** (§6.1 + §6.2) — architect-default positions are honest-now-defer-future.
- **§2.9 zero re-open triggers** — closure preserved.

If user accepts the architect-defaults, Phase 3 enacts 5 amendments-in-place per ADR-0035 precedent (status header gains Q8 qualifier; §15.12 enactment record; CHANGELOG entry). If user picks (ii) on determinism or (α) on ItemLineage or pulls in any DEFER finding, cascade re-enters scope expansion mode and requires new spike iter-3 + Strategy revision.

**Honest framing per `feedback_active_dev_mode.md`:** post-FROZEN Q8 surfaces a wider gap landscape than prior Q1/Q6/Q7 — **but the gaps concentrate in scope-expansion territory (ItemLineage, AI, Saga, Streaming, Schedule/Queue), NOT in spec-vs-production drift.** Q7 closed the spec-vs-production drift class. Q8's 15 findings are: 5 amendable now + 8 future cascades + 2 user-canon-decisions. Cascade can close cleanly at Phase 3 amend-bundle landing IF user accepts architect-default positions on F6 + F7.

---

## §8 Phase 3 dispatch readiness

If tech-lead ratifies this synthesis:

1. **§3.1 amendments enacted** as single CP "Q8" bundle per Q7 precedent — status header qualifier; §15.12 enactment record; CHANGELOG entry.
2. **§6.1 + §6.2 user picks** → architect updates Tech Spec §1.2 N-rows + CASCADE_LOG.md cross-cascade-awareness rows accordingly.
3. **§3.2 cascade slots** → orchestrator updates CASCADE_LOG.md with named slots (out of action-cascade-author scope; Phase 8 summary surfaces).
4. **A1 spike conditional dispatch** — if user wants required idempotency contract, dispatch rust-senior iter-3 to worktree-isolated scratch crate.

Phase 3 enactment is **architect-authorable** for §3.1 + §6.1 + §6.2 user-choice landings; **orchestrator-authorable** for §3.2 CASCADE_LOG.md updates.

---

## §9 Sources

| Source | Used for |
|--------|----------|
| `q8-rust-senior-trigger-research.md` (507 lines) | F1, F2, F3, F4, F5, F13, F14, F15 attribution; §1.6 cluster-mode coverage; §3 trigger family completeness |
| `q8-architect-action-research.md` (305 lines) | F2 (idempotency), F6 (ItemLineage), F7 (determinism), F8 (AI sub-node), F9 (concurrency), F10 (streaming), F11 (Saga), F12 (workflow-version); §2 Temporal comparison; §6 §2.9 sixth iteration check |
| `q8-security-credential-research.md` (516 lines) | §0 confirms 0 🔴 in-scope (4 🔴 + 8 🟠 in separate nebula-auth cascade — out of action cascade scope); §6 cross-cascade severity rollup |
| dx-tester (orchestrator framing) | F15 mechanical doc bucket (4 🟠) |
| Tech Spec FROZEN CP4 status header (line 33) | §0.2 invariant boundaries; ADR-0035 amend-in-place precedent |
| Strategy FROZEN CP3 §3.4 + §6.6 | DEFER cascade-home naming; engine cluster-mode cascade existence |
| ADR-0035 / 0036 / 0037 / 0038 | Compose-rule constraints; spike-shape binding |
| `feedback_post_closure_audit_bundle_pattern.md` | AMENDED-CLOSED-AGAIN framing precedent |
| `feedback_active_dev_mode.md` | "Defer with named home" discipline |
| `feedback_no_shims.md` | F1 path (b) placeholder lock — NOT a shim, a forward-declaration with TBD body |

---

## §10 Open items raised this synthesis

- **§6.1** — F7 determinism posture: tech-lead/user picks (i) / (ii) / (iii). Architect-default (i).
- **§6.2** — F6 ItemLineage scope: tech-lead/user picks (α) / (β) / (γ). Architect-default (β).
- **§4.2** — A1 idempotency hook required vs opt-in: tech-lead picks. Architect-default opt-in (default-impl returns None) → no spike.
- **§3.2** — 6 new cascade slots named (D2 / D4 / D5 / D6 / D7 / D8): orchestrator-managed CASCADE_LOG.md edit; out of action-cascade-author scope. Phase 8 summary surfaces.

---

## §11 Handoffs requested

- **tech-lead** — ratify synthesis (§1 categorization + §2.2/§2.4/§2.5/§2.6 cross-cutting decisions + §6 escalation framings + §7 closure recommendation). Pick architect-defaults or re-frame.
- **rust-senior** — conditional iter-3 spike dispatch ONLY IF user requires non-opt-in idempotency contract (A1 conditional spike).
- **security-lead** — out of scope for this synthesis (security-credential research returned 0 🔴 in-scope; nebula-auth cascade is separate slot).
- **orchestrator** — Phase 8 summary updates: 5 amendments enacted (§3.1) + 8 cascade slots named (§3.2) + 2 escalations framed (§6) + AMENDED-CLOSED-AGAIN closure verdict (§7.2).

**Architect does NOT enact amendments in this Phase 2 document. Phase 3 dispatched separately.**
