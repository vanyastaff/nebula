# Q8 — Trigger family research-driven coverage audit

**Phase:** 1 of 3 (research catalog; architect synthesizes Phase 2; tech-lead ratifies Phase 3)
**Reviewer:** rust-senior
**Slice:** Trigger family — `TriggerAction` / `WebhookAction` / `PollAction` peers + cluster-mode hooks + cursor persistence
**Date:** 2026-04-25

**Sources read line-by-line (no skim):**

- `docs/research/n8n-trigger-pain-points.md` (502 lines)
- `docs/research/windmill-peer-research.md` (475 lines, trigger-relevant sections)
- `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` (§§2.2.3, 2.4, 2.6, 3.5, 8.1.2, 1.2 N4) — FROZEN CP4 + Q7 amendments
- `docs/superpowers/specs/2026-04-24-action-redesign-strategy.md` §3.1 component 7 + §3.4 row 3 + §5.1.5 (cluster-mode hooks)
- `crates/action/src/trigger.rs` (full, 648 lines)
- `crates/action/src/webhook.rs` (lines 1-200 + 560-665 + 1000-1330)
- `crates/action/src/poll.rs` (lines 1-300 + 700-870 + 1300-1467)

**Verdict severities:**

- 🔴 STRUCTURAL — Tech Spec design cannot address; new trait/shape required
- 🟠 INCOMPLETE — Tech Spec partially addresses; amendment needed
- 🟡 OK-WITH-DOC — Tech Spec addresses but needs explicit doc cross-ref or pitfalls.md promotion
- 🟢 INTENTIONAL — Tech Spec deliberately scopes out with rationale (§1.2 N1-N4 / §3.4)

---

## §1 Pain points cataloged (n8n-trigger-pain-points.md)

48 distinct pain points from n8n research (`n8n-trigger-pain-points.md`). Source line is the line number in the research file. "Tech Spec coverage" cites Tech Spec sections (or Strategy §3.4 / §1.2 N for explicit deferrals).

### §1.1 Webhook URL identity / activation lifecycle (15 points)

| Source line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| `33-38` | Test-URL vs Production-URL split — top forum complaint | Not addressed; Tech Spec §2.6 + `webhook.rs` carry single URL space, no test/prod bifurcation | 🟢 (Nebula doesn't replicate n8n's split — n8n problem doesn't exist by design) |
| `38, 152-158` | Test URL listens once then dies (single-shot pattern) | N/A — Tech Spec has no single-shot listener concept | 🟢 (intentional) |
| `155-160` | Production URL reverts to test mode on n8n Cloud | N/A | 🟢 |
| `160-161` | WhatsApp Trigger Production URL changes after hours active (path-rewriting) | Tech Spec §2.6 webhook URL is engine-resolved at adapter boundary; URL identity stability is engine-cascade scope; not surfaced on `WebhookAction` trait | 🟠 — Tech Spec doesn't specify URL stability invariant; should add ENGINE_GUARANTEES cross-ref note in §2.6 |
| `64-69, 173` | External webhook registration not rolled back on deactivation (`#24056` ClickUp, `#24433` Jira) | Tech Spec §2.6 `WebhookAction::on_deactivate(state, ctx)` exists; **but no engine-side reconciler exists in design** for desired-vs-actual external state diff | 🟠 — `on_deactivate` is action-author best-effort; n8n Quick Win 5 (external_subscriptions reconciler table) has no Tech Spec analogue. Engine-cascade scope candidate. |
| `64, 170` | `#27416` multi-main: activation failure permanently deactivates without audit trail | §1.2 N4 cluster-mode coordination defers leader election; activation atomicity is engine-cascade | 🟢 (deferred with home) |
| `171` | `#24850` "Webhook Workflows Canceled on Pod Redeploy Despite Graceful Shutdown" | Tech Spec §3.4 cancel-safety per-action; **no draining contract on adapter shutdown** — `WebhookTriggerAdapter::stop()` waits for in-flight via `idle_notify` (good) but pod-level redeploy needs engine-side draining | 🟠 — adapter-level draining works; pod-level draining is engine concern but Tech Spec should cross-ref |
| `175-176` | `#21614` "Deployment + Activation via API does not register webhook path" — two-stage non-atomic | Tech Spec §2.6 `on_activate` returns state; engine wiring of HTTP listener registration is not specified at trait boundary | 🟠 — n8n Quick Win 7 (atomic activation with post-registration health probe) absent from Tech Spec |
| `292-298` | Webhook URL stability bugs (`#26333` silent overwrite of shared path; `#23908` duplicate URL) | Tech Spec §2.6 doesn't specify slot-uniqueness invariant for webhook URL paths | 🟠 — engine concern but action-side `WebhookAction::config()` has no path-uniqueness preflight |
| `302-306` | Active vs Inactive state inconsistency (`#23046`, `#22472`, `#21824`) | `WebhookTriggerAdapter::state: RwLock<Option<Arc<A::State>>>` + `started: AtomicBool` for poll cover the in-process race; cross-process / multi-main is N4 deferred | 🟢 within process boundary; 🟠 cross-process |
| `64-69` | Activation = distributed state, desync is norm | §1.2 N4 → cluster-mode cascade | 🟢 deferred-with-home |
| `170` | `#27416` multi-main silent permanent deactivation (open) | §1.2 N4 | 🟢 |
| `253` | `#26812` MCP Server trigger dies silently after hours (likely hot-reload class `#19882`) | Tech Spec §3.4 cancel-safety + `WebhookTriggerAdapter` reconnect not codified; long-lived connection reconnect loop is action-author responsibility | 🟠 — generic "long-lived trigger needs reconnect" pattern not surfaced as DX concern |
| `259-260` | Form trigger 404 race (`#24627`) — user clicks before webhook registered | Tech Spec §2.6 doesn't specify activation-then-readiness barrier | 🟠 — n8n Quick Win 7 (post-registration health probe) — engine-side concern but Tech Spec should call it out as engine obligation |
| `258-260` | Chat trigger path stale after deactivation (`#26724`) + mobile UI broken | UI scope, OUT | 🟢 |

### §1.2 Polling state persistence & cursor (8 points)

| Source line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| `54-60, 179-189` | Polling state stored as arbitrary JSON in `staticData` (no schema) | Tech Spec §2.6 `PollAction::Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync` typed; `#[serde(deny_unknown_fields)]` is action-author choice (not enforced by Tech Spec) | 🟡 OK-with-doc — typed cursor is the answer; should cite n8n `#28445` as anti-pattern in pitfalls.md |
| `184-185` | `#28445` NotionTrigger stored `moment.js` object → activation fails after deserialize | Typed cursor prevents this class | 🟢 by-design |
| `186` | `#24539` Gmail hangs on future timestamps | Action-author concern (cursor advancement validation) | 🟡 — could land as DX guidance |
| `187` | `#17795` Postgres Trigger leaks DB connections across reactivations | Resource lifecycle (`ResourceAction::cleanup`) addresses; PollAction itself doesn't open long-lived connections | 🟡 |
| `188-189` | `#25122` IMAP same email fires multiple times (no idempotency keys) | `DeduplicatingCursor<K, C>` (per `poll.rs:553-719`) provides bounded-FIFO dedup | 🟡 OK with doc — Tech Spec §8.1.2 mentions but doesn't surface as primary DX pattern |
| `9` (executive summary) | "Cursor in-memory only, restart resets" | Tech Spec §8.1.2 (Q7 I1 amendment) **explicitly documents this** with the 1948-1963 narrative | 🟢 documented; n8n catastrophe-class is **NOT addressed** for high-value integrations (payments) |
| `744-754` (`poll.rs` doc) | "On process restart, `initial_cursor` is called again. Either re-flood or silent skip — no error, no warning" | Production doc states this is **NOT acceptable for payments/audit/CRM**. Tech Spec §8.1.2 names this as cluster-mode forward-track. | 🔴 STRUCTURAL — see §6 finding #1 |
| `188` | Webhook idempotency-key handling not in core | Tech Spec §2.6 `WebhookAction::handle_request` doesn't enforce idempotency key on transport; n8n Quick Win 8 (`(workflow_id, trigger_id, idempotency_key) UNIQUE` schema) has no Tech Spec analogue | 🔴 STRUCTURAL — see §6 finding #2 |

### §1.3 Cron / Schedule (5 points)

| Source line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| `40-46, 268-275` | **No catch-up for missed cron fires** ("If n8n is down, scheduled runs missed. No automatic catch-up.") | **No ScheduleAction or ScheduleTrigger trait in nebula-action.** TriggerAction has `start()` / `stop()` only — no schedule-ledger surface. PollAction's `PollConfig::base_interval` is interval-based, not cron-syntax. | 🔴 STRUCTURAL — see §6 finding #3 |
| `192-201` | `#27103` randomized cron registers duplicates on save | No Schedule trait; deferred | 🔴 (no ScheduleAction at all) |
| `198-201` | `#27238` custom cron with ranged DOM + DOW intersection bug | Cron parsing not in scope | 🔴 |
| `199-200` | `#23943` "Hours Between Triggers" interval mode fails | PollAction::PollConfig::with_backoff matches; not a cron equivalent | 🟡 |
| `202` | `#20945` "polling interval too short" rejected | `POLL_INTERVAL_FLOOR = 100ms` (poll.rs:36) clamps similarly | 🟢 |

### §1.4 Duplicate executions & deduplication (8 points)

| Source line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| `49-53, 277-289` | **Topology bug: Queue Mode workers + main both start active workflows → 3× exec** (`#15878`) | §1.2 N4 → cluster-mode cascade (leader election) | 🟢 deferred-with-home |
| `283-284` | `#28392` IMAP creates duplicate + ghost triggers | `DeduplicatingCursor` partial; ghost = activation lifecycle bug, N4 | 🟠 |
| `287-288` | `#23893` Stripe Trigger duplicates webhooks **in Stripe** (external registration idempotency) | Tech Spec §2.6 `on_activate` doesn't specify external-registration idempotency-key contract | 🔴 STRUCTURAL — see §6 finding #4 |
| `288-289` | `#25381` webhook duplicates on each publish in GitLab | Same as above | 🔴 |
| `204-208` | Gmail uses message ID list in staticData; truncates → `#10470` duplicate emails | `DeduplicatingCursor<K, C>` `max_seen` cap addresses; truncation-at-cap is documented behavior, not silent loss | 🟡 (good but pitfalls.md should note "if your seen-window is shorter than your event-arrival-rate, you WILL emit duplicates") |
| `208` | RSS/Feed dedup not consistent per-node | DeduplicatingCursor available | 🟡 |
| `208` | Webhook no idempotency-key handling in core | See finding #2 | 🔴 |
| `283` | `#22488` trigger executes twice within scheduled time | Cluster-mode N4 | 🟢 |

### §1.5 Backpressure, concurrency, queue (4 points)

| Source line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| `211-217` | `#28488` Wait node breaks `concurrency=1`; `#23170` queue stuck; `#21364` queue hangs on 0 items | Engine concern; not action-trait scope | 🟢 |
| `217` | No adaptive throttling, no per-workflow rate-limit config | Engine concern; `PollConfig::backoff_factor` covers per-trigger backoff | 🟡 |
| `213-215` | `#20942` Activated workflow queued despite available concurrency | Engine | 🟢 |
| `216` | `#21319` Redis keep-alive missing | Engine | 🟢 |

### §1.6 Sub-workflow triggers (4 points)

| Source line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| `220-227` | `#27725` silent data/context loss between parent and child | Tech Spec scope is action contract; sub-workflow is workflow-engine concern | 🟢 |
| `225-227` | `#25832` Subworkflow executions cannot be stopped in queue mode | Engine concern (cancellation propagation) | 🟢 |
| `225` | `#21615` sub-workflow 150× slower | Engine perf | 🟢 |
| `223` | `#28214` Parent workflow stuck in Waiting | Engine | 🟢 |

### §1.7 Error workflow handling (4 points)

| Source line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| `229-234` | `#25074` Error workflow does not get triggered | No "ErrorTrigger" trait — error routing is engine concern | 🟢 |
| `231-232` | `#21767` error workflow dropdown lists archived workflows; `#24858` unable to remove | UI scope | 🟢 |
| `232-234` | `#15233` Queue Mode error workflow fails | Engine | 🟢 |
| `233` | Routing at runtime (no static validation) | Engine validation; not action-trait scope | 🟡 |

---

**Total cataloged pain points:** 48
**Severity distribution:** 🔴 STRUCTURAL: **5** | 🟠 INCOMPLETE: **9** | 🟡 OK-WITH-DOC: **9** | 🟢 INTENTIONAL: **25**

---

## §2 Windmill trigger architecture comparison

Windmill triggers (`backend/Cargo.toml` features `kafka`, `sqs_trigger`, `nats`, `mqtt_trigger`, `gcp_trigger`, `websocket`, `smtp`, `postgres_trigger`, `native_trigger`, `mcp` per `windmill-peer-research.md:42-45, 137-140`):

### What Windmill has that Nebula's design lacks

| Windmill capability | Source | Nebula equivalent? | Severity |
|---|---|---|---|
| **Native message-broker triggers** (Kafka, SQS, NATS, MQTT, GCP PubSub) as separate trigger kinds | `windmill-peer-research.md:137-138` | TriggerAction is generic (any `TriggerSource: TriggerSource`); webhook/poll are PEERS (Q7 R6); broker triggers would each be a separate peer DX trait OR a single generic "QueueAction" — Tech Spec doesn't enumerate | 🟠 — see §3 below |
| **WebSocket trigger** as separate kind | `:138` | No `WebSocketAction` peer in Tech Spec | 🟠 |
| **SMTP trigger** (incoming email) | `:138` | No `SmtpAction` peer | 🟠 |
| **Postgres LISTEN/NOTIFY trigger** | `:125, 138` | No `DbChangeStreamAction` peer | 🟠 |
| **Schedule trigger** with optional seconds-precision cron | `:137` | Absent — see §1.3 | 🔴 |
| **Suspend/resume as first-class engine primitive** (`wmill.getResumeUrls`) | `:124, 388-389` | Tech Spec ActionResult has no Suspend variant; ResumeAction not a trait family | 🔴 — but engine concern, NOT trigger family scope |
| **Process-per-job durability boundary** (worker stateless between jobs) | `:362-365, 449-450` | Adapter holds in-memory state (`WebhookTriggerAdapter::state`, `PollTriggerAdapter::started`). NOT process-per-job — within the same process, `Arc<dyn TriggerHandler>` is reused. | 🟢 (different deployment model — Nebula tasks run in same process; Windmill is per-job sandbox) |
| **Postgres-as-queue with `SELECT FOR UPDATE SKIP LOCKED`** | `:107-111` | Engine concern; `Dispatcher` trait abstracted is Strategy §3.1 hint but not action-trait | 🟢 |
| **Per-language executor modules** | `:114-116, 397-398` | Resource pattern can model this; not trigger scope | 🟢 |
| **WASM parsers in frontend** (schema inference for Apps editor) | `:357-360` | Out of trigger scope | 🟢 |
| **`tokio_unstable` is a trap** (`#3284`) | `:325-327` | Tech Spec doesn't mandate avoidance; should land in pitfalls.md | 🟡 |
| **jemalloc ARM page-size footgun** (`#4422`) | `:336-338, 437-441` | Out of action scope; is workspace-build concern | 🟢 |

### What Nebula has that Windmill lacks (deliberate divergence)

1. **Type-erased `TriggerEvent` envelope** at the dyn boundary (`trigger.rs:86-122`) — Windmill uses per-language executor modules with concrete event types; Nebula's `Box<dyn Any + Send + Sync>` + `TypeId` design lets transport families ship their own typed event without growing the base trait. Windmill's approach hard-codes "this is a script invocation" into the worker; ours is transport-shape-agnostic. **Better.**
2. **Cancel-safety floor as design invariant** (Tech Spec §3.4 floor item 4 + `webhook.rs:1266-1298` `tokio::select!` shape) — Windmill issues like `#28632` Wait-tracker crashes on Postgres are partly avoidable here.
3. **`PollCursor<C>` per-cycle wrapper with rollback semantics** (`poll.rs:439-477`) — Windmill polling lacks per-cycle checkpoint/rollback; Nebula's `PollOutcome::{Idle, Ready, Partial}` × `EmitFailurePolicy::{DropAndContinue, RetryBatch, StopTrigger}` matrix is more explicit.
4. **Sealed DX pattern** (Tech Spec §2.6 ADR-0038) — Windmill's `FlowModule` enum is open; ours seals the surface for forward-compatible extension. Windmill `#5337` "rework dedicated workers" partly stems from open extension surface.
5. **`SignaturePolicy::Required` as default** for `WebhookAction::config()` (`webhook.rs:660-663`) — fail-closed default; Windmill webhook docs don't surface this discipline.

---

## §3 Trigger family completeness — verify Q7 R6 sealed-DX peer framing

**Q7 R6 finding (Tech Spec §2.6, lines 565-619):** WebhookAction and PollAction are **PEERS of TriggerAction, not subtraits.** Each has its own associated types, lifecycle methods, and erases to `dyn TriggerHandler` via dedicated adapter (`WebhookTriggerAdapter`, `PollTriggerAdapter`).

### §3.1 Does the peer framing hold for ALL trigger types in research?

Trigger types from n8n (`n8n-trigger-pain-points.md:117-127`) + Windmill (`:137-138`):

| Trigger type | Natural shape | Fits TriggerAction? | Fits WebhookAction peer? | Fits PollAction peer? | New peer needed? |
|---|---|---|---|---|---|
| **Webhook** (GitHub, Stripe, Slack, Telegram, Discord, GitLab, Shopify) | Push: HTTP request → handle | No (TriggerAction is too generic for HTTP-specific signature/body invariants) | **YES — WebhookAction** | No | No |
| **Polling** (Gmail, IMAP, RSS, Salesforce, HubSpot, Airtable) | Pull: cron-like loop with cursor | No (no cursor abstraction) | No | **YES — PollAction** | No |
| **Schedule / Cron** | Time-driven, no event source, no cursor | **No — TriggerAction has no schedule abstraction** | No (HTTP is wrong shape) | No (not pull-from-source) | **YES — ScheduleAction peer needed** |
| **Kafka / NATS / SQS / RabbitMQ / MQTT** | Subscribe to broker, consume offsets, ack/nack | Possible via TriggerAction with `accepts_events()` true and `start()` running consumer loop, BUT no offset-management abstraction; no consumer-group identity; no ack/nack semantics | No | Partial (PollAction can pull, but broker-managed offsets ≠ cursor) | **YES — QueueAction peer recommended** |
| **WebSocket** | Long-lived bidirectional connection, reconnect, message frames | Same shape as broker — TriggerAction's `start()` shape-2 (run-until-cancelled) accommodates, but no reconnect framework | No | No | **YES — WebSocketAction peer recommended** |
| **Postgres LISTEN/NOTIFY / DB change streams** | Long-lived subscription, reconnect on failure, ordered events | TriggerAction shape-2 inline; no reconnect framework | No | Partial | 🟡 — could land under QueueAction peer |
| **SMTP / IMAP-IDLE incoming email** | Push (IMAP IDLE) or poll (IMAP poll); `n8n-trigger-pain-points.md:328-330` Gmail issues + IMAP `#28392` show this is its own pain class | Both PollAction + custom poll handling | **OR WebhookAction** if SMTP-relay model | No | 🟡 — fits PollAction or new EmailAction peer |
| **Manual / UI** (n8n Manual / Form / Chat) | One-shot user action via UI → workflow start | Yes — TriggerAction with `accepts_events()` true | Maybe (Form is HTTP-shaped) | No | 🟡 |
| **Error workflow** | Engine-side meta-trigger | Engine concern, not action trait | No | No | 🟢 N/A |
| **MCP Server trigger** | Long-lived MCP transport | TriggerAction shape-2 | No | No | Maybe MCP-specific peer |

### §3.2 Are there trigger types that naturally subtype TriggerAction?

The peer framing holds for **all listed types**. None naturally subtype TriggerAction in a way that would force re-framing as `: TriggerAction`. Webhook and Poll's peer status is correct.

**However, Q7 R6's framing leaves an open question:** Each peer carries its own associated types (`WebhookAction::State`, `PollAction::Cursor`/`Event`) AND its own adapter (`WebhookTriggerAdapter`, `PollTriggerAdapter`). Adding ScheduleAction / QueueAction / WebSocketAction peers would multiply this 3 → 6+. Each peer needs:

- Its own DX trait
- Its own adapter type (with adapter-specific runtime state shape)
- Its own `*Sealed` inner trait + blanket impl
- Its own `#[action(...)]` attribute zone (likely)
- Its own erasure path to `dyn TriggerHandler`

**§2.6 R6 amendment did not establish a meta-pattern for this growth.** Tech Spec §2.6 closes "trait-by-trait audit" but does not lock the per-peer adapter pattern as a documented contract for future peers.

### §3.3 Verdict on §3 question

Q7 R6 framing is **correct**, but Tech Spec §2.6 should add a **DX-peer authoring pattern** subsection (or §11 adapter authoring contract should explicitly cover trigger-family peer adapters) to guide future Schedule/Queue/WebSocket peer additions. **🟠 INCOMPLETE — see §6 finding #5.**

**Severity tally for §3:**
- ScheduleAction: 🔴 STRUCTURAL — missing entirely; `n8n-trigger-pain-points.md:40-46` "missed cron fires" is data-loss class
- QueueAction (Kafka/NATS/SQS/MQTT): 🔴 STRUCTURAL — Tech Spec doesn't enumerate; n8n `#14979`/`#19877`/`#28605` show broker-trigger pain class
- WebSocketAction: 🟠 INCOMPLETE — TriggerAction shape-2 accommodates but no reconnect framework
- Peer-authoring meta-pattern: 🟠 INCOMPLETE

---

## §4 State shape unification analysis

### §4.1 Three current state shapes (Q7 finding R3 + R6 + I1)

1. **WebhookAction::State** (`webhook.rs:578-586`):
   - Bounds: `Clone + Send + Sync` only
   - **No `Serialize`/`DeserializeOwned`** — explicitly documented "ephemeral, not persisted across process restarts in v1"
   - Stored at adapter level: `WebhookTriggerAdapter::state: RwLock<Option<Arc<A::State>>>`
   - Lifetime: process

2. **PollAction::Cursor** (`poll.rs:802-806`):
   - Bounds: `Serialize + DeserializeOwned + Clone + Default + Send + Sync`
   - **Persistence-shaped** — has Serde bounds — but stored as **stack-frame local** in `PollTriggerAdapter::start()` (Q7 I1: `poll.rs:1328`). Not actually persisted in v1.
   - `PollCursor<C>` is a per-cycle wrapper carrying `(current, checkpoint)` for in-cycle rollback. Not a persistence wrapper.
   - Lifetime: per-trigger-instance, single `start()` invocation

3. **TriggerAction (base)**: no associated state at all. `start()` / `stop()` / `handle()`; per-instance config in `&self` fields.

### §4.2 Research questions

**Q: Do n8n / Windmill triggers persist state across restarts?**

- **n8n**: YES, via `workflow.static_data` JSON blob (`n8n-trigger-pain-points.md:179-189`). Schemaless, untyped. `#28445` NotionTrigger stored `moment.js` object → activation fails after restart. `#24539` Gmail hangs on future timestamps stored in staticData.
- **Windmill**: Cursor/state lives in `pip_resolution_cache` table for Python deps; per-trigger state is per-job (process-per-job durability boundary), not cross-restart by design.

**Q: What happens to webhook delivery in-flight when worker crashes?**

- **n8n**: `#23807` queue-mode workers mark successful executions as crashed on restart. `#24850` webhook workflows canceled on pod redeploy despite graceful shutdown. **Not handled.**
- **Windmill**: Per-job processes — if worker crashes mid-job, job fails, no in-process state to lose.
- **Nebula Tech Spec**: `WebhookTriggerAdapter::stop()` waits for in-flight via `Arc<Notify>` (`webhook.rs:1147-1180`) — **clean within-process shutdown**; pod-redeploy crash semantics deferred to engine cluster-mode (§1.2 N4).

**Q: What happens to poll cursor when scheduler restarts?**

- **n8n**: `staticData` persists JSON blob; deserialize on next activation. Source of `#28445` shape-mismatch class.
- **Windmill**: Poll triggers (where present) use Postgres-stored cursor.
- **Nebula Tech Spec §8.1.2 (Q7 I1)**: **Cursor lost.** Re-call `initial_cursor()`. Either re-flood or silent-skip per `poll.rs:744-754` doc — explicitly named "NOT acceptable for payments/audit/CRM."

**Q: Should Nebula triggers have a UNIFIED persistence shape?**

The three shapes are unified at the **type level** by `Serialize + DeserializeOwned + Clone + Send + Sync` — modulo WebhookAction's deliberate omission of Serde bounds. They are NOT unified at the **runtime persistence level** because:

- WebhookAction state is ephemeral by design
- PollAction cursor is in-memory by spec (Q7 I1 documents this)
- TriggerAction has no state to persist

**The unification question is really: where is the engine-side persistence boundary?** Tech Spec §8.3 names `crates/storage/` / `ExecutionRepo` as engine persistence; Tech Spec §1.2 N4 defers cluster-mode coordination (which would add cursor persistence) to engine cascade.

**Issue:** Tech Spec §8.1.2 documents poll cursor as "in-memory only with cluster-mode forward-track" but does not name an engine trait/contract that the future cluster-mode cascade will implement. Compare WebhookAction state which says "if runtime needs to persist, that is the runtime's responsibility (post-v1)" (`webhook.rs:582-585`). Both defer; neither names a concrete trait.

🟠 **INCOMPLETE — Tech Spec should lock a `TriggerStateStore` / `CursorStore` engine trait shape NOW (even as `pub trait { /* TBD */ }` placeholder) so the cluster-mode cascade has a stable target.** This is the same discipline as `feedback_active_dev_mode.md` "before saying 'defer X', confirm the follow-up has a home" — the home exists (cluster-mode cascade) but the trait doesn't, so the cluster-mode cascade gets a blank check.

### §4.3 Severity summary

- 🟠 **State persistence boundary:** No engine trait shape locked for future cluster-mode cursor persistence. Strategy §3.4 row "cluster-mode coordination" defers; Tech Spec §8.1.2 forward-tracks; neither names a concrete `pub trait CursorStore` or equivalent.
- 🔴 **Restart semantics for high-value polls:** `poll.rs:744-754` explicitly says "do not ship a high-value integration against this trait unless your downstream workflow is fully idempotent" — that's a doc warning, not a type-level guard. See §6 finding #1.

---

## §5 Cluster-mode + leader election + deduplication

**Strategy §3.4 row 3 (line 172):** "Engine cluster-mode coordination implementation (leader election, reconnect orchestration) → Engine cluster-mode coordination cascade — tech-lead schedules post-action-cascade close, queued behind credential CP6 implementation cascade."

**Tech Spec §1.2 N4 (line 91):** "Action surfaces three hooks on `TriggerAction` (`IdempotencyKey`, `on_leader_*` lifecycle, `dedup_window` metadata; surface contract only per Strategy §3.1 component 7); engine-side coordination ships in the dedicated cascade."

**Tech Spec §2.2.3 trailing prose (line 376):** "Cluster-mode hooks (`IdempotencyKey`, `on_leader_*`, `dedup_window`) attach as supertrait extensions per Strategy §3.1 component 7 + §5.1.5 — exact trait shape locked at CP3 §7 (this Tech Spec section is foundational; full hook surface is Phase 3+ scope)."

**Tech Spec §15.8 / §15.10 (line 2837, 3108)**: cluster-mode hooks final trait shape DEFERRED-WITH-TRIGGER to engine cluster-mode cascade.

### §5.1 How does n8n handle multi-worker triggers?

**n8n: BADLY.**

- **Multi-main + workers**: `#15878` "All instances (main & workers) start active workflows" → 3× execution. Confirmed race. `n8n-trigger-pain-points.md:280-281`.
- **Activation atomicity**: `#27416` "Workflow activation failure permanently deactivates workflows without audit trail in multi-main setup" — open. `:64`.
- **External webhook reg dedup**: `#23893` Stripe Trigger duplicates webhooks **in Stripe itself** because each n8n instance creates its own. `:287`.
- **Schedule load distribution**: `#27103` randomized cron creates duplicates due to randomization-on-save bug `:194-196`.

n8n's mitigation is **mostly missing**. Forum threads cited (e.g., `community.n8n.io/t/...197126/`) show users running into double-execution as a category that has no clean fix.

### §5.2 How does Windmill handle?

**Windmill: BETTER but not perfect.**

- **Postgres queue with `SELECT FOR UPDATE SKIP LOCKED`** (`windmill-peer-research.md:107-111`) — single-leader semantics at the queue level; workers compete for jobs, only one wins.
- **Worker liveness**: `#6718` workers disappear from UI; `#4907` liveness DEAD; `#5519` can't be deleted. Liveness handling is brittle but the queue-as-leader pattern prevents double-execution.
- **Duplicate dependency jobs scheduled from same root** (`#6055`): "tagged across 50+ releases, unresolved." Idempotency at job-creation level is hard. `:38, 270`.

Windmill's lesson (`:340-348`): "Postgres queue scales surprisingly far — then abruptly hits wall." Their `Kafka/Redis in future` issue (`#173`, since 2022) never shipped.

### §5.3 What pain points exist around cluster coordination?

From research:

1. **Activation atomicity in multi-main** — n8n `#27416` ; engine cluster-mode hooks would address via `on_leader_acquire`/`on_leader_release`
2. **External webhook registration idempotency** — n8n `#23893`, `#24056`, `#24433`; n8n Quick Win 5 (external_subscriptions reconciler table) addresses
3. **Schedule load distribution / leader-elected cron** — n8n `#15878`, `#17187`, `#22488`, `#27103`; engine cluster-mode hooks should address via leader-only schedule firing
4. **Worker health observability** — Windmill `#6718`, `#4907`; engine concern (workers expose health endpoint, supervisor aggregates)
5. **Queue scaling ceiling** — Windmill `:340-348`; Strategy §3.4 / `feedback_adr_ecosystem_evidence.md` — Strategy §3.1 component 7 hook list doesn't include "QueueDispatcher trait" but should

### §5.4 Coverage assessment for cluster-mode

**Tech Spec coverage: SURFACE-LOCKED, BODY-DEFERRED.** Current state:

- Hook **names** locked: `IdempotencyKey`, `on_leader_*` lifecycle, `dedup_window` metadata (Tech Spec §1.2 N4 + §2.2.3 trailing)
- Hook **shapes** NOT locked — final trait signatures, default-body availability, engine-side registration contract — explicitly deferred to engine cluster-mode cascade per §15.8
- **Engine trait** for cursor persistence / leader-state-store NOT locked (§4.2 finding above)

**Risk:** When engine cluster-mode cascade activates, it inherits **three named hooks but zero locked shapes** + **zero locked persistence boundary**. The cascade will need to design all three concurrently. n8n's history (`#15878` open, `#27416` open, `#27103` open) shows this is an area where partial design produces persistent bugs across years.

🟡 **Tech Spec coverage is honest about deferral; engine cascade scope is large.** Severity is 🟡 OK-WITH-DOC because Strategy §3.4 names the home and Tech Spec §1.2 N4 defers cleanly. **However:** §6 finding #6 below recommends locking the engine trait surface NOW (placeholder shape) to constrain the future cascade.

---

## §6 Top-N findings ranked by severity

### Finding #1 — 🔴 STRUCTURAL: Poll cursor in-memory means high-value integrations are unsafe

**Source:** `poll.rs:744-754` doc + Tech Spec §8.1.2 Q7 I1 amendment.

**Pain class evidence:**
- n8n: `#10470` Gmail fetching duplicate emails (cursor truncation), `#24539` Gmail hangs on future timestamps, `#28445` NotionTrigger moment.js trap, `#25122` IMAP fires multiple times. `n8n-trigger-pain-points.md:54-60, 184-189`.
- Production doc explicitly says "**NOT acceptable** for: Payments (Stripe events, invoice sync, ledger updates) / Audit / compliance feeds / CRM sync where 'missed lead = lost revenue'" + "**No error. No warning. No health degradation.**"

**Tech Spec position:** §8.1.2 documents the in-memory limitation; §8.1.2 forward-tracks cluster-mode cursor persistence. **No abstraction locked for the engine cascade to implement.**

**Severity rationale:** This is a structural data-loss class hidden behind permissive trait bounds (`type Cursor: Serialize + DeserializeOwned + ...` looks persistable but is NOT persisted in v1). Action authors will write `PollAction` impls for payment integrations because the bounds suggest persistence; the cursor will be lost on restart; data loss with no telemetry signal. **The trait surface lies about its persistence contract.**

**Tech Spec section affected:** §8.1.2 + §2.6 PollAction trait bound docs.

**Phase 2 recommendation hook (NOT proposed here, architect synthesizes):** Either (a) tighten `PollAction::Cursor` bounds to surface "NOT persisted" honestly (e.g., split `EphemeralCursor` vs `DurableCursor` types; rust-senior won't propose specific shape); (b) lock a `TriggerStateStore` engine trait NOW so payment integrations have a forward target; (c) introduce a `PollDurability` declaration on `PollAction` (Ephemeral/Durable) that gates registration.

### Finding #2 — 🔴 STRUCTURAL: No idempotency-key contract on WebhookAction inbound

**Source:** `n8n-trigger-pain-points.md:51-53, 188-189, 287-289` + `webhook.rs:560-570` + Tech Spec §2.6 WebhookAction shape.

**Pain class evidence:**
- n8n: "every webhook triggers 2 executions within 30-70ms" (forum 197126), `#23893` Stripe duplicates webhooks **in Stripe itself**, `#28392` IMAP duplicate + ghost triggers, `#25122` IMAP same email fires multiple times.
- n8n Quick Win 8: "Idempotency-key column on trigger events `(workflow_id, trigger_id, idempotency_key) UNIQUE`. Gmail uses message_id, webhooks — header or hash."

**Tech Spec position:** `WebhookAction::handle_request(req, state, ctx) -> WebhookResponse` returns `TriggerEventOutcome::{Skip, Emit, EmitMany}` per `webhook.rs:560-570`. **No idempotency key is part of the contract.** The action can return `Emit(payload)` for the same delivery twice (e.g., webhook retry from external service); engine emits two workflow executions; downstream is on its own.

**Compare:** PollAction's `DeduplicatingCursor<K, C>` (`poll.rs:553-719`) is opt-in dedup at the cursor layer. WebhookAction has no equivalent.

**Severity rationale:** This is the single most-cited pain class in n8n research (`Hot + multi-trigger class` per executive summary `:48-52`). Webhook providers (Stripe, GitHub) explicitly send retries on 5xx; without an action-side idempotency contract, retries cause duplicate workflow executions. Tech Spec §2.6 leaves it 100% to action authors.

**Tech Spec section affected:** §2.6 WebhookAction trait + `WebhookResponse` / `TriggerEventOutcome` shape.

### Finding #3 — 🔴 STRUCTURAL: No ScheduleAction / cron trigger family

**Source:** `n8n-trigger-pain-points.md:40-46, 116-127, 268-275` + `windmill-peer-research.md:137`.

**Pain class evidence:**
- n8n: "Missed cron fires in downtime silently dropped; no catch-up." `#23906`, `#25057` workflow active 2 weeks not running, `#27103` randomized cron duplicates, `#27238` cron intersection bug, `#23943` "Hours Between Triggers" interval mode fails. **Hot + data-loss class.**
- Windmill: "Schedule (cron with optional seconds)" is a separate trigger kind.
- Nebula Tech Spec: **no ScheduleAction.** PollAction has `PollConfig::base_interval` (Duration-based, not cron-syntax). TriggerAction has `start`/`stop` only.

**Tech Spec position:** None. Schedule triggers would have to be implemented as `PollAction` with a cron-parser inside the action body — losing schedule-ledger semantics, missed-fire replay, catch-up policy, etc.

**Severity rationale:** Schedule is the second-most-cited n8n pain class. n8n Quick Win 2 (schedule fire ledger table with startup reconcile) is concrete and implementable; without a ScheduleAction trait, action authors can't declare "I am a scheduled trigger" and the engine can't apply schedule-specific recovery (catch-up, missed-fire replay).

**Tech Spec section affected:** §2.6 sealed-DX peers — would need to add `ScheduleAction` peer alongside Webhook and Poll.

### Finding #4 — 🔴 STRUCTURAL: External-registration idempotency not part of activation contract

**Source:** `n8n-trigger-pain-points.md:64-69, 168-176, 287-289` + `webhook.rs:1103, 1119` + Tech Spec §2.6.

**Pain class evidence:**
- n8n: `#24056` ClickUp doesn't remove webhook on unpublishing, `#24433` Jira creates new webhook on **every** restart, `#23893` Stripe Trigger duplicates webhooks **in Stripe**.
- n8n Quick Win 5: "External subscription reconciler: `external_subscriptions(workflow_id, provider, provider_id, desired_state)`. On activation reconcile diffs."

**Tech Spec position:** `WebhookAction::on_activate(ctx) -> Result<Self::State>` returns state; `on_deactivate(state, ctx)` consumes it. The state is opaque (`type State: Clone + Send + Sync` — no Serde). **No engine-side reconciler.** If `on_activate` succeeds but the process crashes before recording state, on next start the action calls `on_activate` again → second external registration → user has 2 GitHub webhooks pointing at same path.

**Severity rationale:** Activation idempotency is a distributed-state problem n8n hits at every external-registration trigger. Tech Spec §2.6 has the trait surface but no design for the engine-side reconciliation.

**Tech Spec section affected:** §2.6 WebhookAction + Strategy §3.1 component 7 (cluster-mode hooks) — should add reconciler hook.

### Finding #5 — 🟠 INCOMPLETE: No DX-peer authoring meta-pattern (per §3 above)

**Source:** Tech Spec §2.6 Q7 R6 amendment; `n8n-trigger-pain-points.md:117-127` + `windmill-peer-research.md:137-138` (broker triggers absent in Tech Spec).

**Pain class:** Future Schedule/Queue/WebSocket peer additions will each need: own DX trait, own adapter, own sealed inner trait, own attribute-zone, own erasure path. Tech Spec §2.6 closes "trait-by-trait audit" for the current 5 DX traits but doesn't lock the meta-pattern for adding peer #6.

**Tech Spec section affected:** §2.6 + §11 adapter authoring contract.

### Finding #6 — 🟠 INCOMPLETE: Engine-side persistence boundary not locked

**Source:** §4 above + Tech Spec §1.2 N4 + Tech Spec §15.8.

**Pain class:** Cluster-mode cascade activates with 3 named hooks (`IdempotencyKey`, `on_leader_*`, `dedup_window`) but no engine trait for cursor persistence, leader-state store, external-subscription ledger. Cascade gets a blank check.

**Tech Spec section affected:** §8.3 boundary with engine persistence + Strategy §3.1 component 7.

### Finding #7 — 🟠 INCOMPLETE: Long-lived trigger reconnect framework absent

**Source:** `n8n-trigger-pain-points.md:253` (`#26812` MCP server trigger stops after hours, `#27867` Gmail Trigger stops polling silently, `#27071` Gmail stops for one mailbox).

**Pain class:** TriggerAction shape-2 (run-until-cancelled) is the canonical home for long-lived listeners (WebSocket, MQTT, Postgres LISTEN, IMAP IDLE, MCP). All such triggers need: connect, watchdog, exponential-backoff reconnect, circuit-breaker, lifecycle-trace correlation. Tech Spec §2.2.3 has cancel-safety + setup-and-return-vs-run-until-cancelled distinction, but no reconnect framework.

**Tech Spec section affected:** §2.2.3 TriggerAction shape-2 doc + potentially a `LongLivedAction` peer or shared utilities.

### Finding #8 — 🟠 INCOMPLETE: Webhook URL stability invariant unspoken

**Source:** `n8n-trigger-pain-points.md:160-161` (`#19037` WhatsApp Trigger Production URL changes after hours active — path-rewriting breaks external registrations) + `:175-176` (`#21614` two-stage registration non-atomic).

**Pain class:** Webhook URL changing between calls breaks the external service's registration. Tech Spec §2.6 has `WebhookAction::on_activate` returning state but no contract that the URL exposed to the external service is stable across the action's lifetime.

**Tech Spec section affected:** §2.6 WebhookAction docs + ENGINE_GUARANTEES cross-ref (engine-cascade scope but Tech Spec should declare action's expectation).

### Finding #9 — 🟠 INCOMPLETE: Cross-process draining vs in-process draining

**Source:** `n8n-trigger-pain-points.md:171` (`#24850` "Webhook Workflows Canceled on Pod Redeploy Despite Graceful Shutdown") + `webhook.rs:1147-1180` (in-process drain via `Arc<Notify>`).

**Pain class:** `WebhookTriggerAdapter::stop()` waits for in-flight `handle_event` via `idle_notify` (good). But pod-redeploy / process-restart needs engine-side draining barrier — currently engine concern, not surfaced.

**Tech Spec section affected:** §2.6 `on_deactivate` docs + engine-cascade ref.

### Finding #10 — 🟡 OK-WITH-DOC: Pitfalls for promotion to `docs/pitfalls.md`

These are not Tech Spec gaps — they are recurring traps that would prevent action authors from hitting n8n-class bugs:

1. **PollAction cursor type with `serde(deny_unknown_fields)`** — prevents `#28445` NotionTrigger class (untyped JSON deserialize trap).
2. **PollAction `initial_cursor` MUST seed from "now" for high-volume integrations** — prevents first-run flood (`poll.rs:744-754` doc says this; should be in pitfalls.md).
3. **DeduplicatingCursor `max_seen` must be ≥ event-arrival-rate × max-restart-window** — n8n `#10470` Gmail cursor truncation class.
4. **WebhookAction signature verification before payload parsing** — Stripe/GitHub HMAC-then-parse order; current `webhook.rs:30-33` doc covers, should land in pitfalls.md as cross-ref.
5. **Long-lived TriggerAction shape-2 needs reconnect** — Finding #7's underlying cause.
6. **`tokio_unstable` is a trap** (Windmill `#3284`) — even if Nebula doesn't enable it, contributors pulling in tokio extensions might.

---

## §7 Cluster-mode coverage final assessment

**Verdict: HONEST DEFERRAL, BUT CASCADE GETS A BLANK CHECK.**

- Tech Spec §1.2 N4 + Strategy §3.4 row 3: cluster-mode cascade has a named home — **good per `feedback_active_dev_mode.md`**.
- Tech Spec §2.2.3 trailing: hook names locked (`IdempotencyKey`, `on_leader_*`, `dedup_window`) — **good for plugin author awareness**.
- Tech Spec §15.8 / §15.10: full hook trait shape DEFERRED-WITH-TRIGGER to cluster-mode cascade — **honest deferral**.
- Tech Spec §4 above: engine trait for cursor persistence / leader-state-store / external-subscription ledger NOT locked — **insufficient constraint on the future cascade**.

n8n's cluster-mode bug class (`#15878` 3× exec, `#27416` activation atomicity, `#23893` external-reg dedup) shows that this area produces **persistent multi-year bugs** when designed piecemeal. Phase 2 (architect synthesis) should consider whether to lock placeholder engine traits NOW (e.g., `pub trait CursorStore { /* TBD */ }`, `pub trait ExternalSubscriptionLedger { /* TBD */ }`) so the future cluster-mode cascade has concrete targets.

Strategy §3.1 component 7 hook list does not enumerate `CursorStore` / `ExternalSubscriptionLedger` / `WebhookUrlRegistry`. **§6 finding #6 above** is the consolidated 🟠.

---

## §8 Severity tally summary

**§1 pain points:** 48 cataloged → 🔴 5 / 🟠 9 / 🟡 9 / 🟢 25
**§3 trigger family completeness:** ScheduleAction missing (🔴), QueueAction missing (🔴), WebSocketAction missing (🟠), peer-authoring meta-pattern missing (🟠)
**§4 state shape unification:** engine persistence boundary not locked (🟠), high-value-poll restart semantics structurally unsafe (🔴 — duplicates §6 finding #1)
**§5 cluster-mode:** honest deferral but blank check (🟠 — duplicates §6 finding #6)
**§6 top findings:** 🔴 4 / 🟠 5 / 🟡 1

**Consolidated unique findings (deduped across §§1-7):**

| # | Severity | Title | Tech Spec section affected |
|---|---|---|---|
| 1 | 🔴 | Poll cursor in-memory unsafe for high-value integrations | §8.1.2 + §2.6 PollAction |
| 2 | 🔴 | No idempotency-key contract on WebhookAction | §2.6 + WebhookResponse/TriggerEventOutcome |
| 3 | 🔴 | No ScheduleAction / cron trigger family | §2.6 sealed-DX peers |
| 4 | 🔴 | External-registration idempotency not in activation contract | §2.6 WebhookAction + §3.1 component 7 |
| 5 | 🔴 | No QueueAction / broker trigger family (Kafka/NATS/SQS/MQTT) | §2.6 sealed-DX peers |
| 6 | 🟠 | DX-peer authoring meta-pattern missing | §2.6 + §11 |
| 7 | 🟠 | Engine-side persistence boundary not locked (CursorStore/Ledger) | §8.3 + §3.1 component 7 |
| 8 | 🟠 | Long-lived trigger reconnect framework absent | §2.2.3 |
| 9 | 🟠 | Webhook URL stability invariant unspoken | §2.6 + ENGINE_GUARANTEES |
| 10 | 🟠 | Cross-process draining unspecified | §2.6 on_deactivate |
| 11 | 🟠 | Activation atomicity multi-main not addressed | §1.2 N4 (deferred but no engine trait) |
| 12 | 🟠 | WebSocketAction peer absent | §2.6 |
| 13 | 🟠 | DbChangeStreamAction / Postgres LISTEN absent | §2.6 |
| 14 | 🟠 | Schedule load distribution / leader-elected cron | §3.1 component 7 + N4 |
| 15 | 🟡 | Pitfalls.md candidates (6 items per §6 finding #10) | docs/pitfalls.md (separate) |

---

## §9 What this audit does NOT do

Per Q8 constraints:

- Does not propose fixes (Phase 2 architect synthesizes)
- Does not modify Tech Spec
- Does not commit changes
- Does not file ADRs / amendments
- Does not extend memory beyond the post-audit hook in `MEMORY.md`

Phase 2 (architect) decisions Phase 3 (tech-lead) ratifies on this catalog.

---

## §10 Sources cross-reference

- `docs/research/n8n-trigger-pain-points.md:1-502` — full read
- `docs/research/windmill-peer-research.md:1-475` — full read with focus on `:42-45, 107-122, 137-140, 309-371` for Rust-specific lessons
- `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`:
  - §1.2 N1-N4 (`:75-95`)
  - §2.2.3 TriggerAction trait shape (`:221-376`) — Q6 + Q7 R3 amendments
  - §2.4 *Handler companion traits (`:439-545`) — Q1 + Q7 R5 amendments
  - §2.6 sealed-DX peers (`:563-720`) — Q7 R6 amendment
  - §3.5 typification path (`:1167-1198`) — Q7 R3+R5+I3
  - §8.1.2 cursor in-memory ownership (`:1934-1965`) — Q7 I1
  - §15.8 / §15.10 / §15.11 deferral closures
- `docs/superpowers/specs/2026-04-24-action-redesign-strategy.md`:
  - §3.1 component 7 (cluster-mode hooks)
  - §3.4 out-of-scope markers (line 164-183)
  - §5.1.5 cluster-mode hooks final shape (line 295-299)
  - §6.6 cluster-mode coordination cascade scheduling
- `crates/action/src/trigger.rs:1-647` (full)
- `crates/action/src/webhook.rs:1-200, 560-665, 1000-1330`
- `crates/action/src/poll.rs:1-300, 700-870, 1300-1467`
