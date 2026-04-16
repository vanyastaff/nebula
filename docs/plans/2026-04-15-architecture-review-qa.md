# Architecture Review — Q&A Session Draft

> **Status:** DRAFT — record of design Q&A session, **not canon**
> **Date:** 2026-04-15
> **Scope:** Decisions from questions #1–#20 of the architecture Q&A session
> **Authority:** Subordinate to `docs/PRODUCT_CANON.md`. If a decision here conflicts with canon, canon wins — fix the draft or propose a canon update deliberately.
> **How to use:** Each section summarizes one decision. «Canon target» column shows where this belongs — §N existing or §N new. Treat as input for canon PR review, not as normative yet.
> **Related detail:** [`2026-04-15-arch-specs/`](2026-04-15-arch-specs/) contains per-decision implementation specs with SQL, Rust types, and flow diagrams.
>
> **Naming conventions (locked in during Q18):**
> - **Tenant mid-level:** `Workspace` / `WorkspaceId` / `ws_` prefix — NOT `Project` (rename `nebula-core::ProjectId` → `WorkspaceId`)
> - **Node attempt row:** `AttemptId` / `att_` prefix — NOT `NodeAttemptId`
> - **Logical workflow-graph node:** `NodeId` (kept from `nebula-core`) — stable string reference
> - **OS process / Nebula instance:** `InstanceId` / `nbl_` prefix — NOT `NodeId` (was overloaded)

---

## 1. Product positioning — who Nebula targets

| Decision | Value |
|---|---|
| **Primary path** | **D** — OSS self-host (one binary, SQLite) + managed cloud (Postgres, our infra) |
| **Reference model** | n8n — both solo operator and cloud customers |
| **Non-goals** | Enterprise-first SaaS, Kubernetes-only deployment, Temporal-grade infra requirements |
| **Architectural implication** | Contracts must be «multi-worker ready», but v1 default is single-process local |
| **Canon target** | §2 (existing), add explicit distribution section |

---

## 2. Tenancy — namespace isolation model

| Decision | Value |
|---|---|
| **Model** | **B** — Org → Workspace two-level hierarchy |
| **Primary isolation key** | `workspace_id` threaded through storage, API, events, metrics |
| **Default for self-host** | Implicit `default` org + `default` workspace, invisible until user invites collaborator |
| **Credentials sharing** | **M2** — credentials live in workspace OR org, org-level with explicit workspace allowlist |
| **Cross-workspace workflow references** | **Not supported** — forks / imports only |
| **Transitive permission rule** | Org role → implicit minimum workspace role (`OrgOwner`/`OrgAdmin` → `WorkspaceAdmin`) |
| **Canon target** | §5 scope table, new §11.8 tenancy section |

---

## 3. Identity, auth, and product telemetry

| Decision | Value |
|---|---|
| **Model** | **A** — built-in auth via `nebula-auth` crate on Rust ecosystem libs |
| **Core stack** | `argon2`, `oauth2`, `openidconnect`, `samael`, `lettre`, `totp-rs`, `tower-sessions`, `governor` |
| **v1 MVP** | email+password, session cookies, PAT for API/CLI |
| **v1.5** | OAuth Google/GitHub, TOTP MFA |
| **v2** | SAML/OIDC SSO, SCIM |
| **Local dev escape** | `NEBULA_AUTH_MODE=none` → `default_user`, warn in logs |
| **Identity tables** | `users`, `org_members` (user_id + org_id + role) |
| **Product telemetry** | Separate `nebula-diagnostics` crate — anonymous opt-out, disclosure on first run, `telemetry preview` / `telemetry disable` CLI |
| **Telemetry red line** | No PII, no workflow content, no credentials, no endpoint URLs, no hostnames |
| **Crash reports** | Separate opt-in layer (not same as usage metrics) |
| **Version update check** | Separate opt-out layer |
| **Canon target** | §12.5 extended, new §12.9 telemetry honesty |

---

## 4. RBAC model

| Decision | Value |
|---|---|
| **Model** | GCP-inspired hierarchy + fixed roles (no custom in v1) |
| **Org roles** | `OrgOwner`, `OrgAdmin`, `OrgBilling`, `OrgMember` |
| **Workspace roles** | `WorkspaceAdmin`, `WorkspaceEditor`, `WorkspaceRunner`, `WorkspaceViewer` |
| **Service accounts** | First-class principal, org-scoped, has PAT tokens, can be workspace member |
| **Credential visibility** | Role-based, not workflow-derived. `Viewer` sees metadata only, `Runner`+ can use without reading token |
| **Scheduled workflow identity** | Runs as service account (no user ownership for cron) |
| **Workflow sharing semantics** | Transfer ownership + fork + export/import JSON in v1; public link v1.5; cross-workspace reference never |
| **Canon target** | New §11.9 RBAC reference |

---

## 5. API routing and versioning

| Decision | Value |
|---|---|
| **Strategy** | **A** — nested path slugs, one domain, self-host + cloud identical |
| **Pattern** | `/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}` |
| **URL identifier** | Slug OR ULID both accepted, canonical form returned in response |
| **Versioning** | Path-based `/api/v1/`, semver within major, breaking changes require major bump |
| **Sessions** | Cookie on one domain, tenant from path (not cookie), allows multi-tab different orgs |
| **PAT behaviour** | Contains only user_id, works across all user's orgs, tenant still from path |
| **Subdomain routing** | Not in v1, future enhancement for cloud branding with fallback to path |
| **Canon target** | New §12.10 API surface |

---

## 6. IDs — Prefixed ULID family

| Decision | Value |
|---|---|
| **Format** | Prefixed ULID (Stripe style) |
| **Storage** | 16-byte binary (`BYTEA` / `BLOB` / `UUID` column) |
| **Wire / logs / URLs** | `xxx_01J9...` human form |
| **Generation** | App-side, extending existing `domain_key` in `nebula-core` |
| **Monotonic** | Yes in hot append paths (`execution_journal`) |
| **v1 prefixes** | `org_`, `ws_`, `user_`, `sa_`, `wf_`, `wfv_`, `exec_`, `node_`, `cred_`, `res_`, `action_`, `plugin_`, `pat_`, `job_`, `nbl_` |
| **Type safety** | Typed newtypes per kind, generated via macro, never raw `String` |
| **Serde** | Serializes as prefixed string, deserializes from prefixed string |
| **Canon target** | §3.10 extended, add ID table in `GLOSSARY.md` |

---

## 7. Slugs — human-friendly aliases

| Decision | Value |
|---|---|
| **Regex** | `^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$` |
| **Model** | Nickname/alias for ULID (strategy S2), not primary key — prevents squatting |
| **Uniqueness** | Org global, workspace per-org, workflow/resource/credential per-workspace, service account per-org |
| **Reserved words** | ~300 names in `nebula-core::reserved_slugs` (technical + ID prefixes + top brands) |
| **Rename** | Allowed, old slug reserved via `slug_history` (org 90d, workspace 30d, workflow 7d) |
| **Auto-gen** | `slugify(display_name)` via `deunicode` + trim + dedup suffix |
| **Display name** | Separate field, Unicode allowed |
| **Length ranges** | Org 3-39, workspace 1-50, workflow/credential/resource 1-63, service account 3-63 |
| **Canon target** | New §12.11 slug contract |

---

## 8. Cancellation — hierarchical structured concurrency

| Decision | Value |
|---|---|
| **Model** | **C2** — cooperative cancel + hard kill escalation, `cancel` vs `terminate` as two distinct actions |
| **Architecture** | Hierarchical `CancellationToken` + `TaskTracker` — process → engine → execution → node |
| **Grace waterfall** | Process 60s > engine 45s > execution 30s > node 30s (per `ActionMetadata::cancel_grace`) |
| **Action API** | Through `ctx.cancellation.check()` / `.is_cancelled()` / `.as_future()` — author never sees raw token |
| **Runtime responsibility** | Automatic propagation + cleanup + escalation + journal write |
| **Author responsibility** | Only cooperative checks + `ctx.spawn` / `ctx.scope` for child tasks (never bare `tokio::spawn`) |
| **Global max** | 5 minutes hard cap |
| **Error variant** | `ActionError::Cancelled` — typed, distinguished from failure |
| **Cascade rule** | Execution cancel → all non-terminal nodes atomically cancelled via `ExecutionRepo::transition_to_cancelled` |
| **Cancel vs Terminate endpoints** | `DELETE .../executions/{id}` (cancel, `WorkspaceRunner`+), `POST .../terminate` (force, `WorkspaceAdmin`+) |
| **Canon target** | §12.2 extended, cross-ref to §12.7 |

---

## 9. Retry — four-layer cascade

| Layer | Who | How | Status |
|---|---|---|---|
| **R1** in-action retry | Author | `nebula-resilience` pipeline for outbound calls | `implemented` |
| **R2** engine attempt retry | Runtime | `ActionMetadata::retry_policy` + `Classify::retryable()` + `node_attempts` persisted | **planned → implemented** (blocker to close §11.2 false capability) |
| **R3** DAG `on_error` routing | Workflow author | `ErrorMatcher` edges in DAG, **no cycles** | `implemented` (types exist) |
| **R4a** manual restart from scratch | Operator | API endpoint creates new execution | `implemented` |
| **R4b** restart from failed node | Operator | `planned` v2 |

**Error taxonomy:** `ActionError::{Transient, Permanent, Cancelled, Fatal}`

**Classification priority:** explicit `ActionMetadata::retry_policy.retry_on` override → author's `Transient`/`Permanent` → `Classify::retryable()` on error

**Cancel wins over retry** — cancel at any stage releases pending retry

**Idempotency key per attempt:** `{execution_id}:{node_id}:{attempt}`

**Default policy:** `{ max_attempts: 3, backoff: ExponentialJitter { 1s, 2.0, 60s }, retry_on: ClassifyBased, total_timeout: Some(10min) }`

**Canon target:** §11.2 rewrite (remove false capability), §14 anti-pattern cross-ref

---

## 10. Timeouts, budgets, quotas, rate limits — four distinct concepts

| Concept | Instance | Default | Failure |
|---|---|---|---|
| **Node attempt timeout** | One `execute()` call | 5 min | `Failed(Timeout)`, maybe retry |
| **Retry total budget** | All attempts together | 30 min | `Failed(RetryBudgetExhausted)` |
| **Execution wall-clock** | Whole run | 24h | Cancel cascade |
| **Stateful max duration** | Long-running StatefulAction | 7 days | `Failed(StatefulTimeout)` |
| **Step timeout** (stateful) | One `step()` call | 5 min | Step re-runs after checkpoint restore |
| **Concurrent executions** | per workspace / org | 100 / 500 | `429 Too Many Requests` |
| **Executions per month** | per org | 10k free / plan | `402 Payment Required` |
| **Active workflows** | per workspace | 50 free / plan | `402` |
| **Storage** | per org | 1 GB free / 100 GB | soft warning → `507 Insufficient Storage` |
| **API calls per PAT** | per identity | 100/sec | `429 + Retry-After` |
| **Execution start rate per workspace** | per workspace | 50/sec | `429` |

**Quotas:** atomic `UPDATE ... WHERE +1 <= limit` (no check-then-set races)

**Rate limits:** `governor` crate, in-memory per-process, v1 single-process acknowledges limit in docs; distributed v2

**Waterfall rule:** `attempt ≤ retry_total ≤ execution`, validated by workflow validator

**Fair scheduling:** weighted round-robin by workspace via `last_dispatched_at`, avoids Airflow FIFO starvation

**Canon target:** new §11.7 four concepts, §12.3 update on quotas defaults

---

## 11. Triggers

| Type | Trait | Dedup | Notes |
|---|---|---|---|
| **Manual** | API direct | — | User or service account as source |
| **Cron** | `CronTrigger` config | by `fire_slot` | `overlap: Skip`, `catch_up: Skip`, `jitter: 30s` defaults; leaderless via unique constraint |
| **Webhook** | `WebhookAction` trait | by configurable `event_id` (header / body path / hash) | `AcknowledgeAndQueue` default response, plugin-provided auth presets |
| **Event / queue** | `EventAction` trait | by `event_id` from source | Plugin-based, queue-native offset commit, shared dedup infrastructure |
| **Polling** | `PollingAction` trait | by item identity | Same dedup path |

**Trigger inbox table:** `trigger_events` with `UNIQUE (trigger_id, event_id)` — dedup built-in

**Delivery contract:** at-least-once with dedup → effectively-once when identity available

**Execution source tracking:** `executions.source` JSONB with variant: `Manual | Api | ServiceAccount | Cron | Webhook | Queue | RestartOf`

**Cron timezone:** per-workflow IANA TZ, DST-safe via `chrono-tz`

**Trigger action specialization:** `TriggerAction` base trait + `PollingAction` / `WebhookAction` / `EventAction` specializations with blanket impls

**Anti-patterns avoided:** Airflow `catchup=True` default, thundering herd on `:00`, synchronous webhook default, no dedup, webhook without auth, in-memory cron in multi-process

**Canon target:** new §9.1–§9.5 trigger taxonomy

---

## 12. Expression language

| Decision | Value |
|---|---|
| **Family** | Non-Turing-complete, CEL-inspired |
| **Two surfaces** | Expression (conditions, computations) + Template (`"Hello ${x}"`) |
| **Security** | No filesystem, network, credentials, process env — only explicit `EvalContext` |
| **Allowed context fields** | `input`, `nodes`, `trigger`, `vars`, `workflow`, `execution`, `env` (whitelist only) |
| **Compilation** | Compile once on workflow save, cached bytecode |
| **Extension functions** | Pure + deterministic default, non-deterministic marked, results persisted for replay |
| **Evaluation scope** | Only explicitly designated fields — never incoming data (blocks SSTI class of bugs) |
| **Type system** | Untyped `serde_json::Value` in v1, optional typed mode via schema in v2 |
| **Error location tracking** | Span-based errors for UI-friendly debugging |
| **Canon target** | New §3.11 |

---

## 13. Workflow versioning

| Decision | Value |
|---|---|
| **Model** | Two-table: `workflows` (current pointer) + `workflow_versions` (immutable history) |
| **Per-execution pin** | `executions.workflow_version_id` immutable after start |
| **States** | `Draft` → `Published` → `Archived` → `(GC'd)`; one `Published` per workflow |
| **Version identity** | Prefixed ULID primary (`wfv_01J9...`) + per-workflow `version_number: int` user-facing |
| **Automatic triggers** | Resolve to latest `Published` at claim time (not receive time) |
| **Storage model** | Full copies, retention: keep referenced + current + last 20 orphaned + 90 days |
| **Rollback** | «Publish copy of older version» — linear history, not time-travel |
| **Schema evolution** | `schema_version int`, forward-only migrations in engine |
| **Draft concurrent editing** | v1 single draft per workflow; v2 multi-user merge |
| **Canon target** | New §7.3, §11.5 matrix update |

---

## 14. Stateful actions — in-memory buffer + batch flush

| Decision | Value |
|---|---|
| **Trait shape** | `StatefulAction` with typed `State`, `initialize` + `step` + optional `on_cancel` |
| **StepOutcome** | `Continue` / `CheckpointAndContinue` / `Done(output)` / `WaitUntil(condition)` |
| **CheckpointPolicy** | Default `Hybrid { 10 steps, 30s }`; options: `EveryStep`, `EveryNSteps`, `EveryInterval`, `Manual` |
| **Buffer strategy** | Write-behind — state in-memory, flushed on policy trigger / terminal / suspended / SIGTERM / memory pressure / lease loss (drop, don't flush) |
| **Memory bounds** | 100 MB total dirty / 1000 dirty count / 1 MB per state hard cap |
| **Storage** | **Column `state` JSONB in `execution_nodes`** (not a separate table); optional `state_blob_ref` for >1 MB in v1.5 |
| **Serialization** | JSON default, MessagePack/CBOR optional, zstd compression optional, schema_hash header |
| **Idempotency** | `iteration_idempotency_key` per iteration for external dedup; author-tracked committed marker pattern |
| **Progress UI** | `GET .../state` → latest checkpoint; separate `ctx.emit_progress(...)` for real-time via websocket (ephemeral) |
| **Timeouts** | Step 5 min, `stateful_max_duration` 7d (separate from retry total timeout) |
| **WaitUntil** | `Timer(time)` / `Signal(name, timeout)` / combination — durable suspension, releases slot |
| **State scope** | Per node_attempt, not shared (cross-node via `$vars`) |
| **Schema migration** | Author-provided migration chain on deserialization failure, fail loud if incompatible |
| **Canon target** | New §3.8.1, §11.5 matrix sharpening |

---

## 15. Delivery semantics — four explicit guarantees

**1. Trigger ingestion:** at-least-once, `trigger_events` dedup built-in via unique constraint on `(trigger_id, event_id)`

**2. Node dispatch:** at-least-once execution, stable `idempotency_key = {execution_id}:{node_id}:{attempt}` per attempt

**3. Side effects:** effectively-once **when idempotency contract honored** (two-sided: engine provides key, author propagates, external system dedups)

**4. Cancellation:** eventually-terminated, bounded by grace period waterfall (up to ~2 minutes graceful, with hard escalation beyond)

**Marketing forbidden words:** «exactly-once», «guaranteed no duplicates», «100% reliable», «never lose data»

**Marketing allowed words:** «durable execution», «at-least-once with dedup», «effectively-once with idempotent APIs», «graceful cancellation»

**Canon target:** new §9.6, §4.5.1 marketing anti-patterns, §11.3 extension on two-sided idempotency contract

---

## 16. Multi-process coordination — leaderless through Postgres

| Decision | Value |
|---|---|
| **Model** | Leaderless peer nodes coordinating through Postgres only |
| **No dedicated tables** | No `workers` / `nodes` table, no heartbeat writes, no cleanup job |
| **Ephemeral node ID** | ULID generated at process startup, stored in memory only (`nbl_01J9...`) |
| **Lease mechanism** | `executions.claimed_by` + `claimed_until` (30s TTL, 10s renewal) |
| **Claim query** | Unified: new work OR stale recovery OR wake-up in one `FOR UPDATE SKIP LOCKED` statement |
| **Crash recovery** | Passive lease expiration — next claim query picks up orphaned work |
| **Takeover policy** | Resume from last checkpoint; after 3 takeover crashes → `Orphaned` status (operator intervention) |
| **Cancel routing** | Each process scans `execution_control_queue` filtered by own `node_id`, 2s interval |
| **Cron in multi-process** | Leaderless via `UNIQUE (trigger_id, fire_slot)` constraint |
| **Fair scheduling** | Ordered claim query (workspace least-recently-dispatched first) |
| **SQLite** | Single-process only (documented limitation) |
| **Postgres** | Multi-process capable |
| **Process lifecycle** | Delegated to K8s / systemd / Docker / supervisord; Nebula exposes `/health` + `/ready` HTTP endpoints |
| **Operational requirement** | Multi-process deployment must have a process supervisor that restarts crashed processes |
| **Canon target** | New §12.8 |

---

## 17. Intra-process architecture — three levels of parallelism

| Level | Component | Responsibility |
|---|---|---|
| **Level 1** | Dispatcher loop + execution task pool | Intra-process parallelism via tokio tasks, bounded by `NEBULA_MAX_CONCURRENT` |
| **Level 2** | Multi-process peer coordination (§16 above) | Between-process via Postgres |
| **Level 3** | K8s / systemd / supervisord | Process lifecycle — not Nebula code |

**Terminology (clean):**

- `dispatcher` — the loop that polls DB and claims work (was «master»)
- `execution task` / `executor slot` — tokio task running one execution (was «slave»)
- `node` / `instance` — one OS process / one K8s pod (was «worker» — overloaded)
- `Nebula cluster` — N nodes coordinating through shared Postgres

**Execution pool crash recovery:**

- Tokio task panic → caught by runtime → dropped → counter decrements → dispatcher spawns replacement
- Dispatcher panic → caught via `std::panic::catch_unwind` → logs → restarts loop
- Process crash → K8s restarts pod → new process claims work via unified claim query

**K8s integration:**

- `replicas: N` gives N Nebula nodes sharing Postgres
- `terminationGracePeriodSeconds` matches Nebula drain timeout (15 min recommended)
- `livenessProbe: /health` restarts pod on deadlock
- `readinessProbe: /ready` excludes draining pod from load balancer
- Rolling update strategy for zero-downtime deploys

---

## 18. Observability stack — OTel-based unified telemetry

| Decision | Value |
|---|---|
| **Model** | OpenTelemetry-compliant stack with unified correlation via `trace_id` |
| **Layers** | Four: logs (stdout JSON + OTLP), metrics (Prometheus + OTLP), traces (OTLP), events (eventbus → 4 subscribers) |
| **Correlation primitive** | `ObservabilityContext` — parallel surface to `ScopeLevel`, reuses underlying IDs, generated at ingress, persisted in `executions.trace_id` |
| **ScopeLevel relation** | **Separate surfaces.** `ScopeLevel` = resource lifecycle. `ObservabilityContext` = telemetry. Both share IDs, neither derives from the other. |
| **Log format** | Structured JSON with mandatory fields: `trace_id`, `span_id`, `nebula.org_id`, `nebula.workspace_id`, `nebula.execution_id`, `nebula.attempt_id`, `nebula.instance_id` |
| **Metric naming** | `nebula_*` prefix, OpenTelemetry semantic conventions, `LabelAllowlist` enforces cardinality caps |
| **Per-workspace metric labels** | On for self-host, off for cloud free tier (cardinality protection) |
| **Trace propagation** | Generated at ingress, persisted in `executions.trace_id`, propagated via `nebula-eventbus` `TracedEvent<E>`, via HTTP `traceparent` header on outbound calls |
| **Event bus subscribers (four standard)** | Storage batch writer (journal), metrics collector, websocket broadcaster, audit writer |
| **Websocket protocol** | `wss://.../orgs/{org}/workspaces/{ws}/live` with room subscriptions (`execution:id`, `workspace:id/active`), RBAC-gated |
| **Multi-process fan-out** | v1 in-process only, v2 Postgres LISTEN/NOTIFY, v3 Redis Pub/Sub for cloud scale |
| **Audit log vs execution journal** | **Two separate tables.** Journal = per-execution timeline (operator debugging). Audit = tenant-wide security events (compliance). Different retention, different RBAC. |
| **Deployment tiers** | Minimal (stdout + Prometheus scrape), Standard (docker-compose ships Grafana + Loki + Prometheus + Tempo), Cloud (OTLP to managed backend) |
| **Canon §4.6 compliance test** | Operator must answer «what/why/retries/context» for any failed execution in under 2 min without reading Rust source |
| **Canon target** | New §4.7 observability contract, §3.10 update on nebula-log/metrics/telemetry scope |

**ObservabilityContext shape:**

```rust
pub struct ObservabilityContext {
    pub trace_id: Option<TraceId>,
    pub parent_span_id: Option<SpanId>,
    pub instance_id: Option<InstanceId>,         // nbl_
    pub org_id: Option<OrgId>,
    pub workspace_id: Option<WorkspaceId>,       // ws_ (renamed from ProjectId)
    pub workflow_id: Option<WorkflowId>,
    pub workflow_version_id: Option<WorkflowVersionId>,
    pub execution_id: Option<ExecutionId>,
    pub logical_node_id: Option<NodeId>,         // stable graph node ref from nebula-core
    pub attempt_id: Option<AttemptId>,           // att_ (renamed from NodeAttemptId)
    pub attempt_number: Option<u32>,
    pub user_id: Option<UserId>,
    pub service_account_id: Option<ServiceAccountId>,
    pub request_id: Option<String>,
}
```

**Marketing forbidden words** (extends §4.5.1): never claim «full observability», «zero blind spots». Allowed: «OpenTelemetry-compliant», «structured logging with trace correlation», «live execution timeline».

---

# Storage schema summary

| Table | Purpose | Lifecycle |
|---|---|---|
| `trigger_events` | Inbox for incoming trigger events, dedup via `UNIQUE (trigger_id, event_id)` | Append, eventually GC after dispatch + retention |
| `executions` | Run entity — one row per workflow run, all states from Pending to terminal | Persistent, retention per quota/plan |
| `execution_nodes` | Per-attempt node details including StatefulAction state column | Persistent, linked to executions |
| `execution_journal` | Append-only audit event stream | Durable, append-only |
| `execution_control_queue` | Cancel/run signals outbox (canon §12.2) | Processed and cleaned |
| `workflows` | Current pointer to workflow version | Mutable |
| `workflow_versions` | Immutable workflow definition history | Retention per policy |
| `slug_history` | Reserved old slugs for redirects during rename grace period | TTL-based GC |
| `cron_firings` | Claim table for cron slot uniqueness | Append, GC old |
| `users`, `org_members`, `orgs`, `workspaces` | Identity and tenant structure | Mutable |
| `sessions`, `personal_access_tokens` | Auth state | Mutable, TTL |
| `credentials` | Credential storage with scope (workspace/org) + allowlist | Mutable |
| `org_quota_usage`, `org_quotas` | Billing enforcement | Mutable |

**Not needed** (decided against during this session):

- No `stateful_checkpoints` — state lives in `execution_nodes.state` column
- No `workers` / `nodes` table — ephemeral `node_id` in memory only, passive lease expiration
- No `execution_queues` — unified into `executions` status + claim lease
- No `webhook_event_seen` separate — merged into `trigger_events` dedup constraint

---

# Canon update map

| Canon section | Action | Source question |
|---|---|---|
| §2 | Extend: explicit D-model (OSS + cloud) positioning | Q1 |
| §3.8.1 | **New**: StatefulAction durability contract | Q14 |
| §3.11 | **New**: Expression language | Q12 |
| §4.5.1 | **New**: Marketing anti-patterns | Q15 |
| §5 | Update scope table: tenancy, marketing claims | Q2, Q15 |
| §7.3 | **New**: Workflow versioning | Q13 |
| §9.1–§9.6 | **New**: Trigger taxonomy + delivery semantics | Q11, Q15 |
| §11.2 | **Rewrite**: Retry as 4-layer cascade, close false capability | Q9 |
| §11.3 | Extend: two-sided idempotency contract | Q15 |
| §11.5 | Sharpen: durability matrix final form | Q14, Q16 |
| §11.7 | **New**: Timeouts/budgets/quotas/rate limits taxonomy | Q10 |
| §11.8 | **New**: Tenancy reference | Q2 |
| §11.9 | **New**: RBAC reference | Q4 |
| §12.2 | Extend: hierarchical cancel propagation | Q8 |
| §12.3 | Update: multi-process requirements, defaults, storage backends | Q16 |
| §12.5 | Extend: auth model | Q3 |
| §12.8 | **New**: Multi-process coordination (leaderless) | Q16 |
| §12.9 | **New**: Product telemetry (opt-out) | Q3 |
| §12.10 | **New**: API routing contract | Q5 |
| §12.11 | **New**: Slug contract | Q7 |

---

# Open items / known debt

| Item | Status | When to revisit |
|---|---|---|
| `nebula-core` `domain_key` may need rename/extend for ULID model | Author check | Before implementation of Q6 |
| `execution_nodes.state_blob_ref` for >1 MB states | `planned v1.5` | When first user hits 1 MB state |
| WebSocket broadcaster implementation for real-time UI | `planned v1` | Part of event bus subscriber set |
| Enterprise SSO (SAML/OIDC) | `planned v2` | After cloud launch with first enterprise ask |
| R4b restart from failed node | `planned v2` | After first operator ask |
| Workflow compensation / saga DAG support | `planned v2` | After first complex multi-step integration |
| Custom roles (beyond fixed v1 set) | `planned v2` | After first enterprise request |
| Distributed rate limiting (shared backend) | `planned v2` | When multi-process load requires it |
| Per-workspace priority weighted scheduling | `planned v2` | When first workspace needs weighted fair queueing |
| Workflow schema v1→v2 migration infrastructure | `planned` | When first breaking schema change lands |
| CEL vs custom DSL choice for `nebula-expression` | Author check | Implementation-level, not architectural |

---

## 19. Error taxonomy at boundaries

| Decision | Value |
|---|---|
| **Foundation** | `nebula-error` crate **already implemented** — `Classify`, `NebulaError<E>`, `ErrorCategory`, `ErrorCode`, `ErrorDetails`, 14 detail types. Spec 19 builds on it, does NOT rewrite |
| **Per-layer types** | `ActionError`, `RuntimeError`, `EngineError`, `WorkflowError`, `StorageError`, `ApiError` — each a `thiserror` enum implementing `Classify` |
| **Boundary discipline** | At cross-crate call, wrap domain error in `NebulaError<E>` with `.context(...)` + `.with_source(prev)`; preserves full chain |
| **Codes catalog** | `nebula-error::codes` for shared (~14 canonical), per-crate `pub const` for specific (~35 more in v1). Total ~50 codes. Never rename, never reuse |
| **API boundary** | `nebula-api::ApiError` → `ProblemJson` (RFC 9457); logs full fidelity internally, public response strips PII and internal details |
| **Public-safe details allowlist** | `ResourceInfo`, `BadRequest`, `QuotaInfo`, `RetryHint`, `HelpLink`, `RequestInfo`, `TypeMismatch`, `PreconditionFailure` serialized; `DebugInfo` and `ExecutionContext` (partial) never in public |
| **Panic contract** | `tokio::spawn` wrapper catches `JoinError::is_panic()`, converts to `RuntimeError::ActionPanicked` with sanitized message (no stack, no PII); internal logs have full `?join_err` |
| **Classification priority** (spec 9 recap) | explicit `retry_policy.retry_on` override → `ActionError` variant → `Classify::retryable()` |
| **Correlation with spec 18** | Every error emission uses structured `tracing` fields: `error.code`, `error.category`, `error.retryable`, `error.severity`; `trace_id` from `ObservabilityContext` threads through |
| **Forbidden** | `anyhow` in library crates (enforced via `cargo deny`), `Box<dyn Error>` in public returns, `String`-as-error, `unwrap` without safety comment, `Other(String)` catch-all variants |
| **Canon target** | Extend §12.4 with propagation contract, two-tier projection rules, panic handling, PII allowlist |

**Code catalog additions** (~35 codes beyond the 14 canonical):
- Auth: `INSUFFICIENT_ROLE`, `NOT_AUTHENTICATED`, `SESSION_EXPIRED`, `MFA_REQUIRED`, `ACCOUNT_LOCKED`
- Workflow: `WORKFLOW_NOT_FOUND`, `WORKFLOW_VERSION_NOT_FOUND`, `WORKFLOW_NOT_PUBLISHED`, `WORKFLOW_VALIDATION_FAILED`, `WORKFLOW_CYCLE_DETECTED`, `EXPRESSION_COMPILE_ERROR`
- Execution: `EXECUTION_NOT_FOUND`, `EXECUTION_NOT_CANCELLABLE`, `EXECUTION_ORPHANED`
- Action/runtime: `ACTION_TRANSIENT`, `ACTION_PERMANENT`, `ACTION_FATAL`, `ACTION_PANICKED`, `ACTION_CANCELLED_ESCALATED`, `RETRY_BUDGET_EXHAUSTED`, `STATEFUL_MAX_DURATION_EXCEEDED`, `STATE_SCHEMA_INCOMPATIBLE`, `STATE_PERSISTENCE_FAILED`, `CHECKPOINT_FAILED`, `LEASE_LOST`
- Quotas: `QUOTA_EXCEEDED`, `MONTHLY_QUOTA_EXCEEDED`, `STORAGE_QUOTA_EXCEEDED`
- Storage: `STORAGE_UNAVAILABLE`, `VERSION_MISMATCH`, `DUPLICATE_SLUG`
- Triggers: `WEBHOOK_AUTH_FAILED`, `WEBHOOK_REPLAY_REJECTED`, `TRIGGER_EVENT_DEDUPLICATED`

---

## 20. Testing story for action and trigger authors

| Decision | Value |
|---|---|
| **Harness** | New `nebula-testing` workspace crate, published to crates.io for plugin authors |
| **Approach** | Thin adapters over `mockall` / `wiremock` / `proptest` / `tokio::test` — don't reinvent testing tooling |
| **Three tiers** | Unit (< 10 ms, no I/O), Component (< 500 ms, ephemeral SQLite + real runtime), Integration (1–10 s, full stack with fake triggers) |
| **Tier 1 primitive** | `ActionContextBuilder` — constructs real `ActionContext` with mocks attached; 30-line test for typical action |
| **Tier 2 primitive** | `ActionTest` / `StatefulActionTest` — runs through real retry/checkpoint wrappers; `fail_n_times()`, `run_until_waiting()`, `send_signal()`, `advance_time()`, `simulate_crash()` |
| **Tier 3 primitive** | `TestEnvironment::ephemeral()` — full engine + SQLite + eventbus + metrics; `submit_workflow`, `start_execution`, `wait_for_execution`, `advance_clock` |
| **Time control** | `tokio::time::pause` + `advance` for Tier 1/2; `TestClock` trait injected at storage layer for Tier 3 (engine/scheduler read `Clock::now()`) |
| **Trigger testing** | Dedicated harnesses per kind: `CronTriggerTest`, `WebhookTriggerTest`, `QueueTriggerTest`, `PollingTriggerTest` + `MockQueue` / `MockHttpServer` / `MockPollingSource` |
| **Cron scenarios** | fires at scheduled time, catch_up=Skip ignores missed, catch_up=LatestOnly fires most recent, leaderless claim unique constraint (2 schedulers → 1 fire) |
| **Webhook scenarios** | HMAC signature verify, Stripe-Signature + timestamp tolerance, dedup on same event_id, `AcknowledgeAndQueue` 202 response |
| **Queue scenarios** | consumer commits after emit, crash recovery doesn't double-process (dedup), backpressure pauses on quota exceeded |
| **External mocking** | `wiremock` wrapper with Nebula-specific helpers (`MockStripe`, `mock_charge_success`, `verify_charge_called`) |
| **Property testing** | `proptest` strategies for Nebula types; canonical idempotency invariant test |
| **Assertions** | Macros: `assert_execution_succeeded!`, `assert_node_retried!`, `assert_metric_incremented!`, `assert_journal_contains!`, `assert_deduplicated!` |
| **Contract tests** | `verify_plugin_contract(plugin)` — mandatory checks per plugin: idempotency, cancellation, no-panic-escape, no-credential-leak, error classification |
| **Knife fixture** | Canon §13 shipped as `run_knife_scenario(env)` — merge gate on every canon-touching PR |
| **CI tiers** | Unit every commit (< 30s), Component every commit (< 2 min), Integration every commit targeted (< 10 min), nightly full suite |
| **Coverage** | Plugin `execute()` ≥ 70%, core crates ≥ 80%, security-critical (`nebula-error`, `nebula-credential`) ≥ 95% |
| **Forbidden** | Real external credentials in tests, sleep-based time control, manual assertion logic where macros exist, skipping contract verification |
| **Canon target** | New §12.12 testing contract |

---

# Not solved by this session (future Q&A blocks)

- **#21 Plugin distribution** — `cargo-nebula` tooling, signing pipeline, plugin registry
- **#20 Testing story** — harness for action authors, integration test framework, sandboxed fixture setup
- **#21 Plugin binary distribution** — `cargo-nebula` tooling, signing pipeline, registry (if any)
- **#22 Storage migration tooling** — how operators apply migrations safely on running cluster
- **#23 Deployment modes** — systemd service, Docker image, K8s Helm chart, desktop app vs server
- **#24 Backup / disaster recovery** — what exactly survives `pg_dump`, how to restore, point-in-time recovery
- **#25 Upgrade path details** — how a running cluster upgrades its engine binary without data loss
- **#26+ TBD**

---

# How this draft relates to canon

- Decisions here are **subordinate** to `docs/PRODUCT_CANON.md`
- Decisions may be folded into canon through targeted PRs per the «Canon update map» above
- Until folded, treat this as **design intent**, not enforcement
- If a PR would violate this draft without updating it, flag for re-discussion
- If the canon grows a new rule that conflicts with this draft, **canon wins** and this draft must be updated or deleted

---

# Changelog

- **2026-04-15** — initial draft from Q&A session questions #1–#17
