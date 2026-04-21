# Temporal — Peer Research (durable execution engine)

> Шестой в серии peer-research. Разбор **Temporal** (temporal.io) — durable
> execution engine с SDKs на Go/Java/TS/Python/.NET/Ruby и Rust (prerelease).
> Архитектурно самый сложный peer Nebula'у — если Nebula хочет настоящую
> durable execution, концепции отсюда.
>
> Связанные файлы:
> - n8n series: [auth-architecture](./n8n-auth-architecture.md),
>   [credentials](./n8n-credential-pain-points.md),
>   [parameters](./n8n-parameter-pain-points.md),
>   [triggers](./n8n-trigger-pain-points.md),
>   [actions](./n8n-action-pain-points.md)

> **⚠️ Scope reminder.** Всё содержимое ниже — описание **Temporal** как
> peer-research subject. Nebula-mitigation — в отдельных correlation-table
> и Quick wins секциях; ничего про Nebula-схему/код здесь НЕТ.

## Метаданные исследования

- **Последняя сверка:** 2026-04-20
- **Источники:**
  - `temporalio/temporal` (server, Go, 19.7k stars, 697 open)
  - `temporalio/sdk-core` (Rust core, 452 stars, 70 open)
  - `temporalio/sdk-typescript` (828 stars, 170 open)
  - `temporalio/sdk-python` (1047 stars, 132 open)
  - `temporalio/sdk-java` (375 stars, 255 open)
  - community.temporal.io форум
  - DeepWiki architectural summary
- **Провенанс:** «hot» = 5+ повторов; «confirmed» = в коде или repro'ится

---

## Executive Summary — топ-5 pain areas

1. **Non-determinism errors (NDE) остаются #1 user-facing failure mode**
   даже после 5+ лет зрелости SDK — каждая SDK имеет open NDE bugs,
   многие на edge-case event-history transitions. **Hot, confirmed.**

2. **Workflow versioning (Patch/GetVersion) правильный, но painful** —
   каждое breaking change accumulates legacy code branches forever;
   community still asks for better ergonomics. **Hot, confirmed.**

3. **Event-history size blowup** — 2 MB event / 50 MB history soft limits
   push users to `Continue-As-New`, что leaks semantics upward
   (signal/query handoff — user-coded). **Hot, confirmed.**

4. **Rust SDK explicitly prerelease** — `temporalio-sdk` crate marked
   «prototype/prerelease» на crates.io и README; real Temporal deployments
   still go through TS/Go/Java. **Confirmed.**

5. **Worker sticky-queue tuning** — operators hit opaque slot contention,
   cache evictions, и 9-second gaps между «Workflow Task Started» и
   «Completed» которые actually user code blocking event loop. **Hot, confirmed.**

---

## Architectural Overview (~350 слов)

Temporal — **durable-execution** engine. Contract: *workflow* — обычный
host-language код (Go, Java, TS, Python, .NET, Ruby, Rust-prerelease).
Runtime записывает каждый side effect как **event** в workflow's **history**,
persisted by Temporal server в Cassandra/Postgres/MySQL. Когда worker
crashes и другой worker picks up workflow, SDK replays код из history:
каждый `await activity(...)`, timer, signal, random number returns *the same value it returned the first time*. Код «resumes» как если бы ничего не случилось.

**Three tiers:**

- **Server** (Go, 19.7k stars) — gRPC front-end + history service +
  matching service + worker service + visibility store (Elasticsearch optional).
  Handles sharding, task queues, schedules, namespaces, cross-cluster replication,
  и (новый) **Nexus** для cross-namespace workflow calls.
- **Core SDK** (Rust, 452 stars) — shared state machine в Rust handles
  gRPC, event translation, replay machinery, retries, heartbeats. Exposes
  C-bridge consumed by TS/Python/.NET/Ruby, plus prerelease native Rust SDK.
- **Language SDKs** — thin wrappers over Core intercepting async primitives:
  TS patches `setTimeout`/`Promise`/`Math.random`, Python wraps `asyncio`,
  Go/Java — separate native SDKs (Go SDK предшествует Core).

**Core concepts worker deals with:**
- **Workflow** (deterministic, replayable) vs **Activity** (side-effecting,
  retried, timed out)
- **Local activity** (in-worker, fast, no task-queue round-trip, capped duration)
- **Signal** (async message to workflow) / **Query** (read-only RPC) /
  **Update** (RPC с validator + handler, durable)
- **Child workflow** и **Continue-As-New** (finite-history restart)
- **Timers** (server-side, up to ~100 years)
- **Task queue** с **sticky queue** (cached workflow stays on one worker 10s by default)
- **Search attributes** (indexed, typed, queryable via SQL-like syntax)
- **Schedules** (first-class; не cron wrapper — это workflows-as-data с
  calendar/interval/jitter)
- **Interceptors** и **payload codecs** (для encryption/compression/signing)
- **Patch / GetVersion** (in-place workflow versioning surviving replay)
- **Nexus** (endpoints позволяющие одному namespace/service call another's
  workflows as operations)

---

## Ось 1 — COVERAGE

Temporal имеет broadest feature set among workflow engines surveyed.
Concepts which Nebula currently lacks:

- **Update API** — synchronous request с validator, handler, durable result
  ([sdk-core#1136](https://github.com/temporalio/sdk-core/issues/1136))
- **Patch / GetVersion** — code-level migration in place
  ([sdk-core#869](https://github.com/temporalio/sdk-core/issues/869))
- **Worker Deployment Versioning** (новый API replacing old Build IDs) —
  [sdk-core#866](https://github.com/temporalio/sdk-core/issues/866),
  [#889](https://github.com/temporalio/sdk-core/issues/889)
- **Nexus cross-namespace operations** —
  [sdk-core#1145](https://github.com/temporalio/sdk-core/issues/1145),
  [#1210](https://github.com/temporalio/sdk-core/issues/1210)
- **Schedules as first-class entities** (не cron wrappers)
- **Interceptors at SDK and Activity level** —
  [sdk-core#1139](https://github.com/temporalio/sdk-core/issues/1139),
  [#1140](https://github.com/temporalio/sdk-core/issues/1140)
- **Search attributes** — indexed workflow metadata queryable via SQL
- **Payload codecs** — pluggable encryption/compression at wire-format layer

**Gaps даже Temporal has:**
- **No built-in per-workflow-type concurrency semaphore:**
  [temporal#7666](https://github.com/temporalio/temporal/issues/7666) (hot — 14 upvotes)
- **Continue-As-New on signal flood:**
  [temporal#1289](https://github.com/temporalio/temporal/issues/1289) (open с 2021)
- **No input/output suppression from history:**
  [temporal#4389](https://github.com/temporalio/temporal/issues/4389) —
  activity inputs always stored, blowing up history

---

## Ось 2 — ERGONOMICS / UNIVERSALITY

### Determinism — constant tax

Каждый SDK имеет dedicated docs pages explaining what you may *not* do в
workflow коде: no `Date.now()`, no `Math.random()`, no direct I/O, no
`Promise.race` с non-deterministic inputs.

- [sdk-python#1109](https://github.com/temporalio/sdk-python/issues/1109):
  `workflow.random().randint` called before `workflow.uuid4` breaks replay.
- [sdk-typescript#1744](https://github.com/temporalio/sdk-typescript/issues/1744):
  NDE replaying nested `Promise.all` — activity scheduling order differs
  между first run и replay. **Hot, confirmed с repro.**
- [sdk-typescript#1966](https://github.com/temporalio/sdk-typescript/issues/1966):
  «Invalid transition while handling update response in state Accepted»
  — internal state-machine NDE, не user error.
- [sdk-typescript#1652](https://github.com/temporalio/sdk-typescript/issues/1652):
  «Throw on usage of Workflow APIs that modify state from non-replayable context»
  — users ask for *earlier* detection.

### Versioning burden

- [sdk-core#640](https://github.com/temporalio/sdk-core/issues/640) «Add flag to disable signal/update reordering»
- [sdk-core#658](https://github.com/temporalio/sdk-core/issues/658) «distinguish between deprecated and non deprecated patch in visibility»
- [sdk-core#535](https://github.com/temporalio/sdk-core/issues/535) «Permit removal + change of adjacent deprecated patches»

Each patch — permanent code branch; users ask for tooling чтобы clean old ones.

### Local vs normal activity trap

[sdk-core#886](https://github.com/temporalio/sdk-core/issues/886):
«Buffering WFT because cache is full because of using local activities»
— local activities keep workflow pinned, fill worker cache, cause back-pressure.

### Sticky-queue pain

[temporal#9563](https://github.com/temporalio/temporal/issues/9563) (hot —
long thread) — user saw 10-second gaps в prod; root cause — sync code blocking
`asyncio` inside activities, не Temporal itself. UX clue — *opaque*:
ничего в UI не говорит что event loop blocked.

### Rust SDK maturity

Confirmed explicitly из [sdk-core README](https://github.com/temporalio/sdk-core):
*«Currently prerelease, see more in the SDK README.md»*. Crate `temporalio-sdk`
на crates.io marked prerelease. Real Temporal users в Rust today use
`temporal-sdk-core` + build their own glue, или call TS/Python worker из
Rust code. **Comparable to Nebula's position today.**

### Payload & history size limits

- [sdk-typescript#1914](https://github.com/temporalio/sdk-typescript/issues/1914)
  «Warn if SDK detects a workflow history over a certain size» — users ask
  for warnings потому что hitting server-side hard limit (~50 MB / 50k events)
  terminates workflow.
- [sdk-core#1223](https://github.com/temporalio/sdk-core/issues/1223) «Patch's
  UpsertSearchAttribute command may result in NDE if payload size exceeds 2KB»
  — opened 2026-04-20.

---

## Ось 3 — BUGS / PROBLEMS

Real production breakage:

- [temporal#9987](https://github.com/temporalio/temporal/issues/9987) «Ringpop
  membership churn after upgrade to v1.30.x» — 48h Cassandra TTL causing
  SWIM zombie IP flooding (**production outage class**).
- [temporal#9945](https://github.com/temporalio/temporal/issues/9945): Matching
  service unbounded Prometheus metric cardinality из otel gauges (memory leak).
- [temporal#9954](https://github.com/temporalio/temporal/issues/9954): Negative
  values в `cache_pinned_usage` (counter underflow).
- [temporal#9959](https://github.com/temporalio/temporal/issues/9959): Pagination
  broken в Elasticsearch visibility store (recent, open).
- [temporal#9974](https://github.com/temporalio/temporal/issues/9974): Multiple
  custom search attributes с same alias — schema conflict.
- [sdk-typescript#2000](https://github.com/temporalio/sdk-typescript/issues/2000):
  Orphaned childWorkflowComplete entries on start failure — state machine leak.
- [sdk-python#1445](https://github.com/temporalio/sdk-python/issues/1445):
  `CancelledError` swallowed during `workflow.start_child_workflow`.
- [sdk-typescript#1843](https://github.com/temporalio/sdk-typescript/issues/1843):
  Default `maxCachedWorkflows` calculation не accounts for VM isolate memory
  outside V8 heap (OOM class).
- [sdk-typescript#2003](https://github.com/temporalio/sdk-typescript/issues/2003):
  `quinn-proto` CVE-2026-31812 shipped в SDK — supply-chain issue.
- [sdk-typescript#1580](https://github.com/temporalio/sdk-typescript/issues/1580):
  Workflows getting stuck after cancellation.
- [sdk-core#1146](https://github.com/temporalio/sdk-core/issues/1146): Empty
  WFT boundary → NDE when followed by inbound Update.

**Pattern:** Temporal's bug surface живёт в двух zones — event-history
replay state machine, и matching/visibility subsystem at scale. Neither
is easy to get right from scratch.

---

## Что Temporal делает ПРАВИЛЬНО — patterns для Nebula

*(Этот Temporal-specific «model answer» раздел. Nebula visual/declarative-first
today, так что translate as design principles, не verbatim APIs.)*

1. **Replay-based durability.** Engine не checkpoint *state*; replays
   *code* из event log. Durable boundary — automatic — каждый `await` —
   potential crash/resume point. **ADR-worthy:** state — это *view*,
   events — *source of truth*.

2. **Patch / GetVersion as first-class API**, не code comment. Workflow
   code can say: `if workflow.patched("v2-retry-logic"): ... else: ...`.
   Engine records patch marker в history. All running workflows continue
   на old branch until finish; new workflows take new branch.

3. **Interceptors at every layer** — client interceptor, workflow outbound,
   workflow inbound, activity inbound. Used for auth, tracing, metrics,
   redaction, chaos.

4. **Search attributes** — indexed, typed, queryable workflow metadata.
   Users can `ListWorkflows` с `WHERE OrderId = 'X' AND Status = 'Running'`.

5. **Schedules as data, не cron strings.** Schedules — own entity с calendar
   specs, overlap policy, catchup window, jitter — return `Schedule` handle
   which можно pause/resume/describe. Much richer than n8n's cron trigger.
   См. [temporal#9383](https://github.com/temporalio/temporal/issues/9383),
   [temporal#8205](https://github.com/temporalio/temporal/issues/8205) (DST).

6. **Payload codec pipeline** separated from transport. Encryption, compression,
   PII redaction — composable Codec traits.

7. **Sticky queue с explicit eviction metrics.** Expensive to run well,
   но observability story honest — cache fullness, eviction reason,
   pinned vs unpinned exposed.

8. **Determinism detection at Core level**, не per-language. Core raises
   NDE с stable error code (`TMPRL1100`). Каждая SDK surfaces same code.

9. **`Continue-As-New` as explicit API**, не magic. Users must trigger it.
   Это good — forces workflow author handle state carry-over.

10. **Replay tests as first-class.** Каждая SDK ships replayer который
    takes history JSON и runs code against it. Only way to catch
    determinism regressions before deploy.
    [sdk-python#994](https://github.com/temporalio/sdk-python/issues/994),
    [sdk-typescript#1362](https://github.com/temporalio/sdk-typescript/issues/1362).
    **Nebula should ship `nebula replay <execution-id>` from day one of durable mode.**

---

## Correlation Table — Temporal problem → Nebula mitigation

| # | Ось | Temporal problem | Issue | Nebula mitigation |
|---|---|---|---|---|
| 1 | 2 | NDE from nested `Promise.all` scheduling order | [ts#1744](https://github.com/temporalio/sdk-typescript/issues/1744) | Nebula visual/DAG-first — scheduling order static из graph. Preserve this; do not add dynamic in-node scheduling without stable-order contract. |
| 2 | 2 | `Math.random`/`Date.now` forbidden; users trip constantly | [py#1109](https://github.com/temporalio/sdk-python/issues/1109) | Any Nebula «code node» MUST expose `ctx.now()`, `ctx.random()`, `ctx.uuid()` which are durable. Ban direct `SystemTime::now()` via clippy lint. |
| 3 | 1 | Versioning burden — patches forever | [sdk-core#535](https://github.com/temporalio/sdk-core/issues/535), [#658](https://github.com/temporalio/sdk-core/issues/658) | Nebula node versions should include «deprecation window» в ADR. Once all live executions past node version, GC the branch. |
| 4 | 3 | History size blowup → workflow termination | [ts#1914](https://github.com/temporalio/sdk-typescript/issues/1914), [temporal#4389](https://github.com/temporalio/temporal/issues/4389) | Nebula already uses outbox pattern; extend так что large payloads go to object store с reference в history. Observability metric: history-size-per-execution. |
| 5 | 1 | No per-workflow-type concurrency limit | [temporal#7666](https://github.com/temporalio/temporal/issues/7666) (14 upvotes) | Add per-action-type semaphore config в engine с day one — cheap, operationally critical. |
| 6 | 3 | Search attribute schema collision | [temporal#9974](https://github.com/temporalio/temporal/issues/9974) | Nebula metadata crate already has typed schema; enforce unique alias at registry level, не at query time. |
| 7 | 2 | Sticky-queue slot contention diagnosed as NDE but was blocked event loop | [temporal#9563](https://github.com/temporalio/temporal/issues/9563) | Expose «task picked up at T1, started at T2, completed at T3» breakdown в every execution view. Cheap, huge DX win. |
| 8 | 2 | Local-activity cache fill | [sdk-core#886](https://github.com/temporalio/sdk-core/issues/886) | If Nebula adds «fast path» node variant, document cache-pinning failure mode before shipping. |
| 9 | 2 | Rust SDK prerelease — no stable users | sdk-core README | **Nebula Rust-native; main competitor не yet GA.** Ship credible docs + examples before Temporal Rust SDK GAs. |
| 10 | 3 | `quinn-proto` CVE shipped | [ts#2003](https://github.com/temporalio/sdk-typescript/issues/2003) | Keep `cargo deny` + `audit_dependencies` in CI; lefthook mirror в place. |
| 11 | 3 | Ringpop churn after minor version bump | [temporal#9987](https://github.com/temporalio/temporal/issues/9987) | Avoid introducing gossip-based cluster membership в Nebula. Keep storage-backed coordination. |
| 12 | 2 | NDE on empty WFT + Update | [sdk-core#1146](https://github.com/temporalio/sdk-core/issues/1146) | Engine decision: make «no-op transitions» explicit events, not invisible ones. |

---

## Quick Wins для Nebula (10 пунктов)

1. **Reserve `NEB1100` / `NEB11xx` error-code range для determinism / replay
   violations сейчас**, так future detectors share one namespace.
2. **`ctx.now() / ctx.random() / ctx.uuid()` на every action context**,
   shadow-forbid `SystemTime::now()` с clippy lint + CI check.
3. **Add `ExecutionTimeline` view**: `(task_scheduled, task_started, task_completed)`
   per node — copy timeline Temporal's UI shows; exactly what #9563 user needed.
4. **Typed search attributes на `Execution`**: at least `workflow_id`, `status`,
   `started_at` as indexed columns — + `custom_attrs: JSONB` для user attributes,
   с validation at registration time.
5. **`nebula replay <execution-id>`** CLI — reads event history, re-runs
   workflow graph, diffs outputs. **Day-one feature.**
6. **Codec trait** в `nebula-core` для payload transform pipelines
   (encrypt/compress/redact). Separate from credentials.
7. **Per-action-type concurrency limit** в `ActionConfig` — semaphore bucket.
   Temporal still doesn't have this; Nebula can ship it.
8. **Payload-size warning at 1 MB, error at 4 MB** per node output — с
   metric + log. Prevents history-blowup class.
9. **Schedule entity separate from Trigger**:
   `Schedule { spec: CalendarSpec | IntervalSpec, overlap_policy, jitter, catchup_window }`.
   Do not couple schedule to one specific trigger type.
10. **Pre-emptive determinism docs page**: one-page «things forbidden в
    durable node» с 10 examples. Temporal's NDE issue volume suggests
    это pays back 100x the effort.

---

## Sources

- `temporalio/sdk-core` (Rust Core): https://github.com/temporalio/sdk-core
  - Issues: [#1223](https://github.com/temporalio/sdk-core/issues/1223),
    [#1146](https://github.com/temporalio/sdk-core/issues/1146),
    [#1136](https://github.com/temporalio/sdk-core/issues/1136),
    [#1139](https://github.com/temporalio/sdk-core/issues/1139),
    [#1140](https://github.com/temporalio/sdk-core/issues/1140),
    [#1145](https://github.com/temporalio/sdk-core/issues/1145),
    [#869](https://github.com/temporalio/sdk-core/issues/869),
    [#886](https://github.com/temporalio/sdk-core/issues/886),
    [#640](https://github.com/temporalio/sdk-core/issues/640),
    [#658](https://github.com/temporalio/sdk-core/issues/658),
    [#535](https://github.com/temporalio/sdk-core/issues/535)
- `temporalio/temporal` (server): https://github.com/temporalio/temporal
  - Issues: [#9987](https://github.com/temporalio/temporal/issues/9987),
    [#9974](https://github.com/temporalio/temporal/issues/9974),
    [#9959](https://github.com/temporalio/temporal/issues/9959),
    [#9954](https://github.com/temporalio/temporal/issues/9954),
    [#9945](https://github.com/temporalio/temporal/issues/9945),
    [#9563](https://github.com/temporalio/temporal/issues/9563),
    [#9383](https://github.com/temporalio/temporal/issues/9383),
    [#8298](https://github.com/temporalio/temporal/issues/8298),
    [#8205](https://github.com/temporalio/temporal/issues/8205),
    [#7666](https://github.com/temporalio/temporal/issues/7666),
    [#4389](https://github.com/temporalio/temporal/issues/4389),
    [#1289](https://github.com/temporalio/temporal/issues/1289)
- `temporalio/sdk-typescript`: https://github.com/temporalio/sdk-typescript
  - Issues: [#2003](https://github.com/temporalio/sdk-typescript/issues/2003),
    [#2000](https://github.com/temporalio/sdk-typescript/issues/2000),
    [#1966](https://github.com/temporalio/sdk-typescript/issues/1966),
    [#1914](https://github.com/temporalio/sdk-typescript/issues/1914),
    [#1843](https://github.com/temporalio/sdk-typescript/issues/1843),
    [#1744](https://github.com/temporalio/sdk-typescript/issues/1744),
    [#1652](https://github.com/temporalio/sdk-typescript/issues/1652),
    [#1580](https://github.com/temporalio/sdk-typescript/issues/1580)
- `temporalio/sdk-python`: https://github.com/temporalio/sdk-python
  - Issues: [#1445](https://github.com/temporalio/sdk-python/issues/1445),
    [#1201](https://github.com/temporalio/sdk-python/issues/1201),
    [#1109](https://github.com/temporalio/sdk-python/issues/1109),
    [#994](https://github.com/temporalio/sdk-python/issues/994),
    [#429](https://github.com/temporalio/sdk-python/issues/429)
- Temporal blog — «Why Rust powers Temporal's new Core SDK»:
  https://temporal.io/blog/why-rust-powers-core-sdk
- `sdk-core` README confirming Rust SDK prerelease status (fetched 2026-04-20)
