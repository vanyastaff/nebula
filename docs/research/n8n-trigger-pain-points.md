# n8n Triggers — Coverage, Ergonomics & Bugs (field report)

> Четвёртый в серии peer-research про n8n. Разбор trigger-системы
> (node'ов, которые запускают workflow): webhook, schedule/cron, polling,
> service-specific push, Form, Chat, Manual, Error, MCP Server. Структура
> по трём осям: Coverage / Ergonomics / Bugs.
>
> Связанные файлы:
> - [n8n-auth-architecture.md](./n8n-auth-architecture.md)
> - [n8n-credential-pain-points.md](./n8n-credential-pain-points.md)
> - [n8n-parameter-pain-points.md](./n8n-parameter-pain-points.md)

> **⚠️ Scope reminder.** Всё содержимое ниже — описание **n8n** как peer-research
> subject. Никаких утверждений про Nebula-архитектуру здесь НЕТ; Nebula-mitigation
> вынесены в correlation-table и Quick wins sections отдельно.

## Метаданные исследования

- **Последняя сверка:** 2026-04-20
- **Источники:**
  - `github.com/n8n-io/n8n` issues (~350 titles scanned, 18 keyword queries)
  - `community.n8n.io` форум
  - DeepWiki architectural summary
- **Охват:** Trigger contracts (`ITriggerNode`, `IWebhookNode`, `IPollFunctions`),
  Active workflow manager, WaitTracker, ExecutionRecovery, Queue Mode, ScalingService
- **Провенанс:** «hot» = жалоба повторилась 5+ раз; «confirmed» = воспроизведено
  в коде или issue.

---

## Executive Summary — топ-5 pain areas

1. **Webhook test-URL vs production-URL confusion — #1 user-facing complaint
   в всём n8n project.** Десятки forum-threads «Webhook production URL keeps
   reverting to Test mode» + «Test URL works, production returns 404».
   Root cause: два separate Express routes (`/webhook-test/*` и `/webhook/*`),
   два env vars (`N8N_EDITOR_BASE_URL` vs `WEBHOOK_URL`), и activation step,
   который юзер должен помнить вручную. **Hot + confirmed.**

2. **Missed cron fires в downtime silently dropped; нет catch-up.**
   DeepWiki подтверждает: *«If the n8n instance is down, scheduled runs
   might be missed. There is no automatic catch-up for missed triggers.»*
   Вместе с `#23906` «Missed schedule trigger can cause unexpectedly long
   delays» и `#25057` «workflow is active but hasn't been running as scheduled
   for two weeks» — это data-loss class, который юзеры замечают поздно.

3. **Duplicate executions через каждый trigger type.** `#28322` Schedule
   duplicate, `#17187` scheduled tasks executing multiple times, `#22488`
   trigger executes twice, `#15878` workflows execute 3× в Queue Mode,
   `#25122` IMAP fires multiple times, `#28392` IMAP creates duplicate +
   ghost triggers. [Forum 197126](https://community.n8n.io/t/webhook-receiving-duplicate-executions-doubling-execution-count/197126):
   «every webhook triggers 2 executions within 30-70ms». **Hot + multi-trigger class.**

4. **Polling triggers теряют state, забывают что видели, застревают в zombie mode.**
   `#18167` «Gmail Trigger continues executing after workflow deactivation and
   node deletion — zombie process», `#28445` NotionTrigger хранит moment.js
   object в staticData → `WorkflowActivationError` на каждой activation,
   `#24539` Gmail hanging на future timestamps, `#10470` Gmail fetching
   duplicate emails. Polling-state persistence через `getWorkflowStaticData('node')`
   без schema и без validation.

5. **Activation lifecycle не atomic.** `#27416` «Workflow activation failure
   permanently deactivates workflows without audit trail in multi-main setup»
   (open), `#24433` Jira creates a new webhook в Jira на *каждом* restart
   (external side-effects не deduped), `#4272` Gmail occasionally misses
   emails after restart, `#23807` queue-mode workers mark successful webhook
   executions as crashed on restart. Activation = distributed state:
   `webhook_entity` table + in-memory `ActiveWorkflows` + external registrations
   (Stripe/Jira/GitHub). **Desync — норма.**

**Secondary:** Form trigger 404 на «production URL» (`#13729`, `#15901`, `#22874`);
Chat trigger webhook-not-registered (`#26724`, `#26491`); MCP Server trigger
dies silently после часов (`#26812`); `loadOptions` на triggers evaluates без
resolved credential expressions (`#27652`); external webhooks auto-deleted
providers (`#16967` Zendesk).

---

## Architectural Overview (~200 слов)

Источник: DeepWiki + `packages/workflow/src/interfaces.ts`,
`packages/cli/src/active-workflow-manager.ts`.

**Три trigger-contract в `INodeType`:**

- **`ITriggerNode`** — реализует `trigger()` возвращающий handle; для push-style
  sources с собственным event loop (message queues, WebSocket, chat).
  Lifecycle bound to `closeFunction`.
- **`IWebhookNode`** — декларирует `webhooks: IWebhookDescription[]` в node
  description; runtime HTTP handled by `WebhookService` routing
  `/webhook/:path` (prod) и `/webhook-test/:path` (test).
- **`IPollFunctions`** — реализует `poll()`; wrapped by `ScheduledTaskManager`
  cron-job'ом derived через `toCronExpression`. Минимальный interval — 1 минута.

**`ActiveWorkflowManager.add()`** на activation: enumerate webhook nodes через
`WebhookHelpers.getWorkflowWebhooks`, insert rows в `webhook_entity` table,
вызывает `closeFunction`'ы от trigger nodes на deactivation. Polling nodes
регистрируют cron jobs в `ScheduledTaskManager`. Failures bounce в retry queue.

**State между polls** живёт в `workflow.static_data` JSON (per-node namespace)
— canonical store для «last seen id», «last timestamp».

**`WaitTracker`** handles `Wait`-node resumption с DB-backed queue.
**`ExecutionRecovery`** at startup сканирует `running` executions и reconciles.

**Schedule trigger** использует `cron` library; timezones через `moment-timezone`
и `this.getTimezone()`. **Нет missed-window replay.**

**Queue Mode** вводит redis-backed job queue с `ScalingService`; worker-процессы
executing jobs. Webhook workflows run on webhook instances; schedule workers
on main — split является source многих state-synchronization bugs.

---

## Ось 1 — COVERAGE (какие триггеры есть и чего не хватает)

### Shipped triggers (из `packages/nodes-base/nodes/*/`)

- **Core:** Webhook, ScheduleTrigger (cron), Manual, Error, ExecuteWorkflow
- **UI:** Form, Chat
- **Per-service push:** GitHub, GitLab, Stripe, Shopify, WooCommerce, Zendesk,
  Jira, Asana, ClickUp, Box, Dropbox, Google Drive, HubSpot, Notion, Airtable,
  Slack, Telegram, Discord, WhatsApp, Twilio, Outlook, Gmail, iCloud, IMAP,
  Typeform, Calendly, Clockify, Salesforce
- **Queue/streaming:** RabbitMQ, Kafka, AMQP, MQTT, Redis (pubsub), Postgres LISTEN/NOTIFY
- **File:** LocalFileTrigger
- **MCP Server Trigger** (2025+)

### Явные пробелы (что юзеры запрашивают, но нет)

| Чего не хватает | Доказательство | Что делают юзеры |
|---|---|---|
| **Generic file watcher across object stores** | `#4915` «LocalFileTrigger does not work with mounted network shares». Нет S3 object-created, GCS, Azure Blob | Schedule → HTTP → If changed (не idempotent) |
| **DB change streams beyond Postgres LISTEN** | `#13646` «Postgres Trigger listening to a channel not responsive» (open, long-running). MySQL binlog / MongoDB change streams / Debezium CDC absent | Polling + last-id tracking |
| **Native cloud event triggers** | EventBridge rules, Pub/Sub push, Azure Event Grid — absent | Webhook + manual HMAC validation |
| **Reliable Kafka/NATS consumer groups** | `#14979` LZ4 compression unsupported, `#19877` RabbitMQ stops consuming, `#28605` RabbitMQ delete-from-queue broken, `#12575` AMQP drops JSON messages | Отдельный Kafka-consumer сервис |
| **Manual trigger + partial data replay** | Нет «re-fire from this point» | Ручной clone workflow |
| **Multi-trigger workflows** | `#26491` Chat Trigger fails с multiple triggers, `#25972` Respond to Webhook hangs с two triggers | Split на отдельные workflows |

**Workaround pattern** когда trigger отсутствует: Schedule → HTTP poll с
`getWorkflowStaticData` cursor. Работает, но попадает на polling-state bugs ниже.

---

## Ось 2 — ERGONOMICS / UNIVERSALITY

### Test URL vs Production URL — главный UX-cliff

**Confirmed hot.** Core quirks:
- Два separate URLs (`/webhook-test/…` и `/webhook/…`), два env vars
  (`N8N_EDITOR_BASE_URL`, `WEBHOOK_URL`), два behavior'а.
- Test listens только **once**, потом stops. Юзер calls Stripe webhook, он
  fires, test stops, next call dropped — «Test URL works, production returns 404»
  ([thread 134549](https://community.n8n.io/t/production-webhook-not-triggering-but-test-url-works/134549), `#27976`).
- Production URL reverts to test mode на Cloud ([thread 108065](https://community.n8n.io/t/webhook-production-url-keeps-reverting-to-test-mode-on-n8n-cloud/108065),
  [79871](https://community.n8n.io/t/webhook-production-url-keeps-reverting-to-test-on-n8n-cloud/79871)).
- В Queue Mode test webhooks hit **main** instance пока production идёт через
  dedicated webhook instances — запутано at scale.
- `#19037` WhatsApp Trigger Production URL **changes** after hours active —
  path-rewriting ломает external registrations.

### Activation & Deactivation lifecycle

- Webhook entity rows в `webhook_entity` table; `ActiveWorkflowManager.add()`
  inserts; `remove()` deletes + calls `closeFunction`. Но **external side-effects**
  (Stripe/Jira/GitHub push subscription creation) в node's trigger method —
  и не всегда roll back на deactivation:
  - `#24056` ClickUp doesn't remove webhook registration after unpublishing
  - `#24433` Jira creates a new webhook on **every** restart
  - `#18167` Gmail zombies after deletion
- `#24850` «Webhook Workflows Canceled on Pod Redeploy Despite Graceful Shutdown»
  — no draining.
- `#27416` multi-main setup: activation failure permanently deactivates
  без audit trail.
- `#21614` «Deployment + Activation via API does not register webhook path»
  — two-stage registration (DB row vs HTTP listener) non-atomic.

### Polling State Persistence

- Через `workflow.static_data` (JSON blob per workflow, namespaced per node).
  Без schema, без migrations.
- `#28445` NotionTrigger хранил `moment.js` object — на next activation
  deserialization died → **every activation fails**. Symptom «JSON.stringify
  arbitrary JS in DB» без discipline.
- `#24539` Gmail hangs на future timestamps (staticData value становится trap).
- `#17795` Postgres Trigger leaks DB connections across reactivations.
- Form/Webhook не используют staticData, но также не имеют idempotency keys —
  `#25122` same email fires IMAP trigger multiple times.

### Cron / Schedule / Timezone

- `cron` library + `moment-timezone`. Workflow-level timezone; fallback to server.
- `#27103` (open) **Schedule Trigger registers duplicate crons on save due to
  randomized cron expression** — confirmed: когда юзер picks «every 5 minutes»
  UI randomizes start second чтобы spread load, но save/reload регистрирует
  **оба** крона.
- `#27238` custom cron с ranged Days of Month + Day of Week runs every day
  (intersection bug).
- `#23943` «Hours Between Triggers» interval mode fails; cron workaround works.
- `#24272`/`#24271` «every 2 hours» doesn't trigger.
- `#20945` «polling interval is too short» rejected even для valid sub-minute.

### Deduplication

- Gmail: uses message ID list в `staticData`. Truncates over time →
  `#10470` «fetching duplicate emails again».
- RSS/Feed: not implemented consistently per-node.
- Webhook: нет idempotency-key handling в core. Юзеры ставят manual `Dedupe` nodes.

### Backpressure & Concurrency

- `#20942` (closed) Activated workflow queued даже с available concurrency slots.
- `#28488` Wait node breaks `concurrency=1` in queue mode.
- `#23170` «Production queue stuck — scheduled executions never start, instance
  restart needed».
- `#21364` Queue mode hangs indefinitely когда nodes return 0 items.
- Нет adaptive throttling; нет per-workflow rate limit config.

### Sub-workflow Triggers

- `Execute Workflow Trigger` — pseudo-webhook (parent call → child returns).
- `#27725` (closed) **Silent data/context loss between parent and child.**
- `#28214` Parent Workflow Intermittently Stuck in «Waiting» в Queue Mode.
- `#25832` (open) Subworkflow executions cannot be stopped в queue mode —
  нет cancellation propagation.
- `#21615` sub-workflow 150× slower than normal node.

### Error Workflow Handling

- Set per-workflow в settings; receives `{execution, workflow, error}` payload.
- `#25074` «My error handling workflow does not get triggered» — classic.
- `#21767` (open) error workflow dropdown lists archived workflows.
- `#24858` unable to remove error workflow.
- Queue Mode regression `#15233` — Error workflow fails с
  `Cannot read properties of undefined (reading 'add')`.

### Chat Trigger

- Webhook-based, session через cookie.
- `#27872` «runs >1:30 fails without Chat Send Message node».
- `#26971` «Chat trigger response timeout after 1 minute 30 seconds» — hardcoded timeout.
- `#26724` path stale after deactivation.
- `#21303` mobile UI broken.

### MCP Server Trigger (new, 2025+)

- `#26812` stops working after a few hours (likely same hot-reload class as `#19882`).
- `#28725` `getWebhookName` crash.
- `#28414` internal stack traces leak to tool responses.
- `#16294`/`#17171` path parameters not accessible — missing feature from day one.
- `#23004` Code Tool nodes hang когда MCP runs on dedicated webhook service.

### Form Trigger

- Also webhook-based, но own rendering.
- `#15901` production URL returns 404 across many versions.
- `#23262` loops forever.
- `#26549` breaks behind Cloudflare Tunnel.
- `#24627` «on form submission 404 because Form Trigger isn't listening yet»
  — race между user clicking и webhook registration.

---

## Ось 3 — BUGS / CONFIRMED PROBLEMS

### Missed Cron Fires

- **Confirmed в architecture:** no replay. Юзеры, которые restart во время
  scheduled window, lose it forever.
- [Thread 283577](https://community.n8n.io/t/cron-node-missing-executions/283577)
  «Cron occasionally skips, especially during high load or after a restart» — hot.
- `#23906` «Missed schedule trigger can cause unexpectedly long delays» —
  closed without a real fix.
- `#25057` «active for two weeks but not running».
- `#18951` «scheduled trigger not firing every minute».

### Duplicate Executions — confirmed races

- `#15878` (Queue Mode v1.95.1) — **All instances (main & workers) start active
  workflows** → triple execution. Confirmed race.
- `#17187` scheduled tasks executing multiple times at once.
- `#22488` trigger executes twice within scheduled time.
- `#27103` randomized cron registers duplicates.
- `#28392` IMAP creates duplicate executions and ghost triggers.
- `#25122` IMAP Cloud fires multiple times for the same email.
- `#23893` Stripe Trigger duplicates webhooks в Stripe itself (external
  registration idempotency).
- `#25381` webhook duplicates on each publish в GitLab.

### Webhook URL Stability

- `#26333` (closed) **Webhook registration silently overwrites when multiple
  workflows use the same custom path pattern.**
- `#23908` duplicate webhook URL.
- `#28462` 2.16.0/2.17.0 webhook bugs.
- `#27976` all production webhook URLs return 404 on Cloud Starter.
- `#27987` URL mismatch между test и publish URL.

### Active vs Inactive State Inconsistency

- `#23046` inactive workflows still executing.
- `#18167` Gmail zombie after deactivation.
- `#22472` inactive workflow still executing, can't stop.
- `#20561` workflow keeps getting executed with old version.
- `#21824` workflow fails to activate но remains «inactive» while somehow working.

### Resume / Recovery After Crash

- `WaitTracker` с DB persistence — partial.
- `#28632` (open) «Wait node Invalid token blocking production workflows
  after 2.16.1».
- `#28541` (open) **`dbTime.getTime is not a function` crashes wait-tracker
  poller on Postgres** — hot active bug.
- `#23807` queue-mode workers mark successful executions as crashed on restart.
- `#24274`/`#24050` «Worker failed to find data for execution» persists with 2s delay.

### Queue Overflow Behavior

- `#23170` production queue stuck, restart needed.
- `#28181` memory leak в editor process (unbounded EventEmitter listeners в
  queue mode with 10+ active workflows).
- `#21319` Redis keep-alive missing.
- `#15154` Postgres node fails в Queue Mode.

### Specific Broken Triggers в топе жалоб

**Gmail** (most complaints):
- `#28733` open, `#27867` silent stop, `#27071` stops one mailbox while another works
- `#24539` future timestamps, `#15090` scheduled folder blocks polling

**Telegram:**
- `#26795` Restrict-to-Chat-IDs prevents callback queries
- `#28378` stale data on second manual test

**Slack:**
- `#26636` all Slack bot triggers stopped (provider-side API change propagation)

**GitHub/Stripe:**
- `#23893` Stripe duplicates webhooks
- GitHub push not в топ complaints — fewer reports than expected

**IMAP:**
- `#28392` open duplicate+ghost, `#19169` fires multiple times after save
- `#26226` OOM crash after ECONNRESET

---

## Correlation Table

| Ось | Проблема | Root cause в n8n | Nebula mitigation |
|---|---|---|---|
| Ergonomics | Test/Prod URL confusion | Два separate routes, два env vars, два mental models | Single URL + query-param `?mode=test` или header `X-Nebula-Test: 1`; listener state — flag, не второй route |
| Ergonomics | Test URL listens once then dies | Hard-coded single-shot pattern | TTL-gated test listener (default 5 min), multi-shot, visible countdown |
| Bug | Missed cron на downtime | Нет replay; нет durable schedule ledger | Schedule history table `(workflow_id, fire_at, fired)`; на boot replay missed с `since_last_boot` config cap |
| Bug | Duplicate executions в Queue Mode | Main + workers оба start active workflows | Single «activation owner» elected через Postgres advisory lock; workers — pure executors |
| Bug | Cron randomization duplicate (`#27103`) | Randomized start-second creates two registrations | Deterministic slot assignment by workflow_id hash; no randomization на save |
| Ergonomics | Polling staticData arbitrary JSON (`#28445`) | `JSON.stringify(any)` в DB | Typed `PollState<T>` per-node с serde-derived; reject unknown shapes at load |
| Ergonomics | External webhook not cleaned (`#24056`, `#24433`) | Trigger method handles registration без audit | Separate `external_subscriptions` table (provider, provider_id); `Reconciler` на activation diffs desired vs actual |
| Bug | Zombie triggers after delete (`#18167`) | `closeFunction` not guaranteed | `tokio::select!` + `CancellationToken` on every long-lived trigger; supervisor validates closure |
| Bug | Wait-tracker poller crash (`#28541`) | Untyped deserialize of resume payload | Typed resume envelope `{version, schedule_at, payload_hash, ...}` |
| Bug | Queue stuck scheduled (`#23170`) | Нет liveness check на scheduler | Scheduler heartbeat to DB; stale lock takeover; dead-man reset |
| Ergonomics | Form trigger 404 race (`#24627`) | User clicks перед webhook registered | Activation returns только после HTTP listener is live; UI polls health endpoint |
| Bug | MCP trigger dies silently (`#26812`) | Long-lived connection; нет reconnect loop | Exponential backoff + circuit breaker; `trace_id` correlates reconnects |
| Coverage | File watcher absent | Только LocalFileTrigger, mount-only | `S3Trigger` / `ObjectStoreTrigger` backed by SQS/EventBridge + idempotency key dedup |
| Ergonomics | Multi-trigger fragility (`#26491`, `#25972`) | Assumes single entry point | Trigger-agnostic execution model: каждый entry — `(trigger_id, invocation_id)`; multi-trigger validated at schema-load |
| Bug | Error workflow silently not triggered (`#25074`) | Error routing at runtime, нет static validation | Static validation error workflow wiring at save; UI shows «error-workflow-of» graph |
| Bug | Activation non-atomic multi-main (`#27416`) | Нет distributed lock | Activation через single leader; все replicas watch |

---

## Quick Wins для Nebula (каждый ~10 LoC)

1. **Один webhook URL, test mode — флаг.** `/webhook/:path?test=1` listens
   for a TTL window then auto-disables. Убивает топ-форум-complaint class.
2. **Schedule fire ledger table** `(workflow_id, scheduled_at, fired_at NULL)`.
   Ten-line startup reconcile — «fire all rows с `scheduled_at < now() - backfill_cap
   AND fired_at IS NULL`» (bounded). Убивает missed-cron class.
3. **Activation leader election через `pg_advisory_lock`.** Workers никогда
   не регистрируют triggers. Устраняет `#15878` 3× executions.
4. **Typed polling state:** `PollState<T>` generic per trigger type, serde с
   `#[serde(deny_unknown_fields)]`. Убивает `#28445` moment.js class.
5. **External subscription reconciler:**
   `external_subscriptions(workflow_id, provider, provider_id, desired_state)`.
   На activation reconcile diffs. Убивает `#24056`/`#24433`.
6. **`CancellationToken` + supervisor для каждого `ITriggerNode`** — zombie-proof
   by construction.
7. **Atomic activation с post-registration health probe:** return 200 только
   после HTTP listener responds to `/ping`. Убивает «form isn't listening yet» race.
8. **Idempotency-key column на trigger events**
   `(workflow_id, trigger_id, idempotency_key) UNIQUE`. Gmail uses message_id,
   webhooks — header или hash. Убивает dedup class на schema level.
9. **Per-workflow concurrency slot в config, enforced server-side** через
   `Semaphore<Arc>`. Нет больше «queue stuck» scenarios.
10. **`TriggerKind` enum с mandatory
    `describe_reentry() -> Reentry { OnceOnly, Resumable, Restartable }`.**
    Forces авторов declare «что происходит на crash»; supervisor has info
    для recovery. Makes n8n's unspoken semantics explicit.

---

## Ключевая мета-идея

Trigger-система n8n — это **distributed state без single source of truth**:
in-memory `ActiveWorkflows` map, `webhook_entity` DB table, external webhook
registrations в Stripe/Jira/GitHub, polling `staticData` JSON blob, Queue Mode
Redis jobs. Каждый из них может desync от остальных. Nebula должна:

> **Активация — one transaction с leader election, reconciler для external state,
> typed polling state с schema validation, и explicit TriggerKind-declaration
> для recovery semantics.**

Это не квартет quick-win'ов — это одно решение architecture level.

---

## Sources

### GitHub issues (selection)

**Schedule / Cron:**
- [#27103 Schedule Trigger randomized cron duplicate](https://github.com/n8n-io/n8n/issues/27103)
- [#23906 Missed schedule trigger long delays](https://github.com/n8n-io/n8n/issues/23906)
- [#25057 Workflow active но не запускается 2 недели](https://github.com/n8n-io/n8n/issues/25057)
- [#27238 Custom cron intersection bug](https://github.com/n8n-io/n8n/issues/27238)

**Queue Mode / Duplicates:**
- [#15878 Queue Mode: workflows execute 3×](https://github.com/n8n-io/n8n/issues/15878)
- [#28322 Duplicate executions в scheduled workflow](https://github.com/n8n-io/n8n/issues/28322)
- [#17187 Scheduled tasks multiple at once](https://github.com/n8n-io/n8n/issues/17187)
- [#22488 Trigger executes twice](https://github.com/n8n-io/n8n/issues/22488)
- [#23170 Production queue stuck, restart needed](https://github.com/n8n-io/n8n/issues/23170)
- [#23807 Queue mode marks successful webhook as crashed](https://github.com/n8n-io/n8n/issues/23807)

**Webhook URL / Registration:**
- [#26333 Webhook silently overwrites shared path pattern](https://github.com/n8n-io/n8n/issues/26333)
- [#27976 Production webhook URLs 404 on Cloud](https://github.com/n8n-io/n8n/issues/27976)
- [#19037 WhatsApp Trigger Production URL changes](https://github.com/n8n-io/n8n/issues/19037)
- [#21614 Deployment + Activation via API doesn't register webhook](https://github.com/n8n-io/n8n/issues/21614)
- [#24850 Webhook workflows canceled on pod redeploy](https://github.com/n8n-io/n8n/issues/24850)

**External subscriptions:**
- [#24056 ClickUp webhook not removed on unpublish](https://github.com/n8n-io/n8n/issues/24056)
- [#24433 Jira creates new webhook on each restart](https://github.com/n8n-io/n8n/issues/24433)
- [#23893 Stripe Trigger duplicates webhooks](https://github.com/n8n-io/n8n/issues/23893)

**Polling / State:**
- [#28445 NotionTrigger moment.js в staticData](https://github.com/n8n-io/n8n/issues/28445)
- [#18167 Gmail zombie after deactivation](https://github.com/n8n-io/n8n/issues/18167)
- [#24539 Gmail hangs на future timestamps](https://github.com/n8n-io/n8n/issues/24539)
- [#28392 IMAP duplicate + ghost triggers](https://github.com/n8n-io/n8n/issues/28392)
- [#25122 IMAP Cloud fires multiple times](https://github.com/n8n-io/n8n/issues/25122)

**Activation lifecycle:**
- [#27416 Multi-main silent permanent deactivation](https://github.com/n8n-io/n8n/issues/27416)
- [#23046 Inactive workflows still executing](https://github.com/n8n-io/n8n/issues/23046)
- [#22472 Inactive workflow still executing, can't stop](https://github.com/n8n-io/n8n/issues/22472)

**Wait / Recovery:**
- [#28541 Wait-tracker dbTime.getTime crash](https://github.com/n8n-io/n8n/issues/28541)
- [#28632 Wait node Invalid token blocking](https://github.com/n8n-io/n8n/issues/28632)
- [#24274 Worker failed to find data](https://github.com/n8n-io/n8n/issues/24274)

**Sub-workflows:**
- [#27725 Execute Workflow silent parent-child data loss](https://github.com/n8n-io/n8n/issues/27725)
- [#28214 Parent Workflow stuck в Waiting](https://github.com/n8n-io/n8n/issues/28214)
- [#25832 Subworkflow executions cannot be stopped](https://github.com/n8n-io/n8n/issues/25832)

**Triggers-specific:**
- [#26491 Chat Trigger fails с multiple triggers](https://github.com/n8n-io/n8n/issues/26491)
- [#26812 MCP server trigger stops after hours](https://github.com/n8n-io/n8n/issues/26812)
- [#24627 Form trigger 404 «isn't listening yet» race](https://github.com/n8n-io/n8n/issues/24627)
- [#25074 Error workflow not triggered](https://github.com/n8n-io/n8n/issues/25074)
- [#19882 Trigger nodes can't register after server runs](https://github.com/n8n-io/n8n/issues/19882)
- [#27867 Gmail Trigger stops polling silently](https://github.com/n8n-io/n8n/issues/27867)
- [#27071 Gmail stops for one mailbox](https://github.com/n8n-io/n8n/issues/27071)

### Community forum (hot threads)

- [Webhook Production URL keeps reverting to Test on n8n Cloud (79871)](https://community.n8n.io/t/webhook-production-url-keeps-reverting-to-test-on-n8n-cloud/79871)
- [Webhook stuck in Test URL mode (209142)](https://community.n8n.io/t/webhook-stuck-in-test-url-mode-unable-to-activate-production-url/209142)
- [Production webhook not triggering, test works (134549)](https://community.n8n.io/t/production-webhook-not-triggering-but-test-url-works/134549)
- [Cron Node Missing Executions (283577)](https://community.n8n.io/t/cron-node-missing-executions/283577)
- [Cron Workflow Didn't Trigger (202999)](https://community.n8n.io/t/cron-workflow-didn-t-trigger-need-help-debugging/202999)
- [Scheduler stops working sometimes (90570)](https://community.n8n.io/t/scheduler-stops-working-sometimes/90570)
- [Webhook receiving duplicate executions (197126)](https://community.n8n.io/t/webhook-receiving-duplicate-executions-doubling-execution-count/197126)
- [Schedule trigger triggering multiple times (146042)](https://community.n8n.io/t/n8n-schedule-trigger-triggering-multiple-times-constantly/146042)
- [Duplicate Execution Runs in WaitTracker (60306)](https://community.n8n.io/t/duplicate-execution-runs-in-waittracker/60306)
- [How to prevent duplicate webhook executions (116953)](https://community.n8n.io/t/how-to-prevent-duplicate-webhook-executions-within-a-short-time-window-e-g-dropbox/116953)
- [Watch folder Trigger feature request (6094)](https://community.n8n.io/t/watch-folder-trigger-got-created/6094)

---

## Связь с остальной документацией

- Архитектура auth: [`n8n-auth-architecture.md`](./n8n-auth-architecture.md)
- Credential pains: [`n8n-credential-pain-points.md`](./n8n-credential-pain-points.md)
- Parameter pains: [`n8n-parameter-pain-points.md`](./n8n-parameter-pain-points.md)
- Для Nebula-ADR по **trigger/workflow activation**: §Quick wins 1, 2, 3, 5, 10
  — кандидаты на отдельный ADR «Trigger activation & recovery model».
- Для **STYLE.md**: §Ось 3 содержит anti-pattern catalog — arbitrary JSON
  в staticData, two-route webhook split, no-replay schedule, external-reg
  без reconciler.
