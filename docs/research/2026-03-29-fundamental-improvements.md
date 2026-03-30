# Фундаментальные улучшения для Nebula: Исследование

**Дата**: 2026-03-29
**Методология**: DeepWiki MCP, GitHub Issues (gh CLI), Web Search, академические источники
**Покрытие**: 90 проектов из 7 категорий

---

## Фаза 1 — Архитектурный анализ

### A. Workflow / Orchestration движки

**Temporal** (Go)
- Execution model: Event-sourced append-only history log с deterministic replay; workers long-poll task queues через Matching Service
- State: Append-only event log в Cassandra/MySQL; MutableState derived из events; sticky task queues кэшируют state на workers
- Type system: Нет — payloads это opaque byte arrays с optional protobuf metadata
- Bottleneck: History size — large histories вызывают expensive full replay при worker eviction; persistence QPS cap
- Unique: Sticky task queues — кэш workflow state на том же worker, delta history вместо full replay

**n8n** (TypeScript)
- Execution model: Stack-based graph traversal; single-threaded in-process или dispatch в Bull/Redis workers
- State: Full execution data (все node outputs) в SQLite/Postgres после завершения; нет mid-execution checkpointing
- Type system: Нет — ITaskData (JSON blobs) без schema validation
- Bottleneck: Все intermediate data в памяти; large payloads → OOM; нет streaming между nodes
- Unique: Regular vs queue mode toggle — single-server без Redis

**Airflow** (Python)
- Execution model: Scheduler polls metadata DB; DagProcessor парсит Python → SerializedDAG JSON; executors dispatch task instances
- State: Единая PostgreSQL metadata DB (task instances, XComs, DAG serializations)
- Type system: Нет — XCom это serialized Python objects без schema
- Bottleneck: Metadata DB — single chokepoint; XCom не для больших данных; DAG parsing overhead ~ O(файлов)
- Unique: Decoupled DagProcessor (Airflow 3.0) — scheduler не исполняет user code

**Prefect** (Python)
- Execution model: Python @flow/@task decorators; Orion server — rules engine для state transitions; pluggable task runners (Dask, Ray)
- State: State machine с canonical states в Postgres; result persistence per-task в S3/GCS
- Type system: Pydantic validation на flow inputs; нет enforcement на task-to-task edges
- Bottleneck: Concurrency limit slots через SELECT FOR UPDATE — deadlocks и leaked slots; DB latency per state transition
- Unique: Pluggable task runners — fan out в Dask/Ray без изменения кода

**Windmill** (Rust + polyglot)
- Execution model: Rust backend с PostgreSQL-as-queue; workers poll v2_job_queue; nsjail sandbox per script
- State: PostgreSQL для job queue и state; S3 для environment tarballs
- Type system: JSON Schema на script inputs/outputs с validation при dispatch
- Bottleneck: Cold starts для тяжёлых runtimes; nsjail overhead; PostgreSQL polling latency floor
- Unique: Database-as-queue (без Redis/Kafka); multi-tier caching для cold start mitigation

**Cadence** (Go)
- Execution model: Предшественник Temporal — decision tasks, activity tasks; History Service sharded по workflow ID
- State: Append-only history в Cassandra/MySQL; shard-per-workflow-ID
- Type system: Нет — opaque payloads
- Bottleneck: ACK level stalls при blocked buffered tasks; full replay on failover; thrift coupling
- Unique: Shard-per-workflow-ID с independent queue processors

**Dagster** (Python)
- Execution model: Software-defined assets (SDA) — граф по data dependencies, не task order; IO managers между ops
- State: Event log в PostgreSQL (materializations, observations, asset versions)
- Type system: DagsterType с runtime type checking на op I/O; DagsterTypeCheckDidNotPass
- Bottleneck: IO managers serialize/deserialize на каждом op boundary — slow для больших DataFrames
- Unique: Asset-centric модель — dependencies на данных, не на execution steps; automatic staleness detection

**Flyte** (Go + K8s)
- Execution model: FlyteAdmin → K8s CRDs; FlytePropeller reconciles FlyteWorkflow CRDs; tasks как K8s pods
- State: PostgreSQL для metadata + object storage (S3/GCS) для inputs/outputs; DataCatalog для memoization
- Type system: Сильная protobuf-based типизация через FlyteIDL; Literal/LiteralType с compile + runtime validation
- Bottleneck: etcd size limits для workflow CRDs; K8s API rate limiting под нагрузкой
- Unique: DataCatalog — automatic task-level caching по hash(task signature + inputs)

**Argo Workflows** (Go + K8s)
- Execution model: Container-per-step — K8s pod с init/main/wait containers; workflow-controller reconciles CRDs
- State: CRDs в etcd; optional archive в PostgreSQL
- Type system: Нет — parameters как strings, artifacts как files через S3
- Bottleneck: Container startup overhead (image pull + init); etcd size limits; single controller bottleneck
- Unique: Full container isolation — любой Docker image = valid task, zero SDK coupling

**Luigi** (Python)
- Execution model: Target-based — tasks declare requires()/output() Targets; completion = Target.exists()
- State: Scheduler state in-memory + optional pickle; данные в Targets (HDFS, DB)
- Type system: Parameter types (IntParameter, DateParameter) для config; нет типизации data flow
- Bottleneck: Monolithic single-process scheduler; нет horizontal scaling; нет dynamic workflows
- Unique: Target-as-completion-signal — естественная idempotency без execution state tracking

**DolphinScheduler** (Java)
- Execution model: Master-worker; MasterServer segments DAGs и dispatches; dynamic weighted round-robin
- State: MySQL/PostgreSQL для metadata; ZooKeeper для coordination
- Type system: Нет
- Bottleneck: ZooKeeper dependency; database contention
- Unique: Dynamic weighted round-robin с real-time CPU/memory/thread-pool metrics

**Kestra** (Java)
- Execution model: Event-driven с JDBC-based queues; JdbcExecutor → WorkerTask → WorkerTaskResult
- State: PostgreSQL/MySQL для queues и execution state
- Type system: YAML input types; нет runtime checking между tasks
- Bottleneck: JDBC polling latency и row-lock contention
- Unique: Plugin architecture — каждый task type как standalone plugin JAR

**Restate** (Rust + TS)
- Execution model: Durable execution через journal-based replay; каждая non-deterministic operation — journal entry
- State: Bifrost distributed log (WAL); per-partition RocksDB; recovery через replay от last LSN
- Type system: Protobuf/JSON service definitions с typed state keys
- Bottleneck: Journal size растёт с long-running invocations; replay cost ~ O(journal length)
- Unique: Virtual objects — actor model + durable execution + isolated KV state per entity

**Inngest** (Go)
- Execution model: Event-driven step functions; SDK yields generator opcodes (OpcodeStep, OpcodeSleep, OpcodeWaitForEvent)
- State: Redis для inter-step state; DB для metadata
- Type system: Event schemas с optional JSON Schema
- Bottleneck: Round-trip per step (SDK call → opcode → enqueue); Redis payload size limits
- Unique: Generator opcode protocol — SDK yields opcodes, server decides scheduling

**Hatchet** (Go)
- Execution model: Durable task queue + DAG workflows; workers pull tasks; DurableContext с SleepFor/WaitForEvent
- State: PostgreSQL для всего — runs, steps, events
- Type system: Нет beyond SDK structs
- Bottleneck: PostgreSQL как queue + state store — write amplification
- Unique: Simplified Temporal на PostgreSQL — trade scale ceiling за operational simplicity

**Netflix Conductor** (Java)
- Execution model: DeciderService — stateless evaluator: takes workflow state, returns DeciderOutcome (tasks to schedule)
- State: External stores (Dynamo, Redis, Elasticsearch)
- Bottleneck: Concurrent decide calls → inconsistent state; needs distributed locks
- Unique: Pure-function decider — deterministic, testable без I/O

**Apache NiFi** (Java)
- Execution model: FlowFiles = metadata pointers; Content Repository хранит payload отдельно; copy-on-write
- State: Content Repository + Provenance Repository
- Bottleneck: I/O-bound на repositories; JVM GC при большом количестве FlowFiles
- Unique: Claim-check pattern — данные хранятся отдельно, FlowFiles несут только metadata

**Camunda/Zeebe** (Java)
- Execution model: Append-only logstream как source of truth; state = projection of log; single-threaded per partition
- State: Event-sourced log; deterministic replay
- Bottleneck: Single-threaded per partition caps throughput
- Unique: Event-sourced execution log — deterministic replay, time-travel debugging, audit

**XState** (TypeScript)
- Execution model: Formal statecharts (Harel) + actor model; hierarchical/parallel states
- State: In-memory; нет persistence
- Bottleneck: Single-process, нет distribution
- Unique: Formal state machine model с compile-time transition enforcement

**Node-RED** (JavaScript)
- Execution model: Wire-based dataflow; message cloning на fan-out; dynamic node loading через npm
- State: Context stores (node/flow/global scope)
- Bottleneck: Single-threaded event loop; нет параллелизма внутри flow
- Unique: Three-tier context scoping (node/flow/global)

**Apache Camel** (Java)
- Execution model: Enterprise Integration Patterns как first-class routing primitives; 350+ connectors
- Bottleneck: Route complexity explosion; debugging глубоких EIP chains
- Unique: EIP как composable primitives (split/aggregate, content-based router, dead letter channel)

**Activepieces** (TypeScript)
- Execution model: "Piece" system — typed props, triggers, actions; V8 isolate per execution
- Bottleneck: Sandbox overhead; нет streaming между steps
- Unique: Typed property schemas с auto-generated UI

### B–C. Rust крейты — ключевые алгоритмические идеи

| Крейт | Ключевая идея | Применение к Nebula |
|-------|--------------|---------------------|
| **tower** | Token-based readiness (#412) — poll_ready() возвращает token для call() | Type-safe dispatch: node produces ReadyToken, executor consumes it |
| **petgraph** | Mature DAG algorithms, но Copy requirement на nodes (#587) | Не навязывать trait bounds на node data; graph merging для workflow composition |
| **salsa** | Query-based memoization с revision tracking; push-based invalidation (#41) | Incremental re-execution: memoize node outputs, invalidate downstream при изменении input |
| **crossbeam** | Chase-Lev work-stealing deques, epoch-based GC | Custom task scheduler primitive; lock-free node status updates |
| **rayon** | Work-stealing fork-join; parallel-to-sequential bridge отсутствует (#210, +66) | Fan-out/fan-in — Nebula нужен свой ordered fan-in collector |
| **slotmap/thunderdome** | Generational arena allocation — O(1) access, safe index reuse | NodeId = (slot_index, generation); заменить HashMap<NodeId, Node> |
| **left-right** | Double-buffered reads с zero contention | Execution state monitoring без блокировки executor |
| **arc-swap** | Atomic Arc swapping | Hot-reload конфигурации (retry policies, timeouts) без остановки |
| **moka** | TinyLFU admission + W-TinyLFU eviction | Cache node outputs для incremental re-execution; prevents cache pollution |
| **daggy** | DAG wrapper over petgraph — acyclicity enforcement на insertion | Валидация DAG при построении, не после |
| **hydroflow** | Two-layer architecture: compile-time fusion inner + runtime scheduling outer; microbatch processing; semilattice formalism | Fuse tightly-coupled steps into optimized unit; outer scheduler handles inter-group flow |
| **acts** | IRQ (interrupt-request) pattern: workflow pauses, resumes on external completion; timeout escalation chains (multiple thresholds → different handlers) | Human-in-the-loop pause/resume; per-step escalation policies |
| **orka** | Type-keyed pipeline registry; sub-context extraction (handlers see typed sub-view) | Action isolation через typed context sub-views |
| **apalis** | Tower Service trait as universal middleware; combinator workflows (and_then, filter_map) | Actions as tower Services → free retry/timeout/metrics middleware |

### D. Функциональные языки и другие экосистемы

| Проект | Ключевая идея | Трансфер в Nebula |
|--------|--------------|-------------------|
| **Broadway** (Elixir) | Demand-driven backpressure через GenStage | Pull model для scheduling — downstream nodes signal readiness |
| **Commanded** (Elixir) | Process Managers как workflow coordinators; `interested?` routing | Long-running workflows как process managers с event routing |
| **ZIO** (Scala) | `ZIO[R, E, A]` — typed error channel `E` как часть signature | `Action<Ctx, E, A>` — compile-time exhaustive error handling |
| **cats-effect** (Scala) | `Resource[F, A]` monad — bracket acquire/use/release | Resource pattern для credential/connection lifecycle |
| **Taskflow** (C++) | Condition tasks с integer branch selection; composable subflows | Condition nodes → branch index; subflows как first-class DAG nodes |
| **Intel TBB** (C++) | Flow graph, parallel pipeline, task arena | Pipeline stages с bounded buffers и cancellation |
| **Ray** (Python) | Distributed object store с zero-copy reads; ownership-based refcount | DataRef handles вместо serde_json::Value для больших payloads |
| **Dask** (Python) | Graph optimization passes (fuse, cull, inline) перед execution | Pre-execution оптимизация: fuse linear chains, cull dead branches |
| **LangGraph** (Python) | BSP execution + checkpoint для human-in-the-loop | Checkpoint barriers для approval nodes и crash recovery |
| **AutoGen** (Python) | Composable termination conditions | `MaxSteps(10) \| TextContains("DONE") \| Timeout(30s)` |
| **Dagger.jl** (Julia) | Type-driven scheduling — route tasks по типу I/O | Type-based action placement на typed worker pools |
| **Ballerina** | Integration-first language; sequence diagrams from code | Workflow → sequence diagram visualization |

### E–G. Streaming, Research, AI Orchestration

| Проект | Ключевая идея | Bottleneck | Трансфер |
|--------|--------------|------------|----------|
| **Timely Dataflow** (Rust) | Progress tracking через frontiers (timestamp antichains) | Complexity; steep learning curve | Precise "subgraph done through timestamp T" notifications |
| **Differential Dataflow** (Rust) | Incremental computation через (data, time, diff) triples | Memory для arrangements | Efficient re-execution для iterative workflows |
| **Materialize** (Rust) | Arrangement-based IVM | Single-threaded timestamp oracle | Incremental re-execution only affected subgraph |
| **Arroyo** (Rust) | Epoch-based checkpoint с aligned barriers | Designed for throughput, не latency | Checkpoint barrier injection для consistent snapshots |
| **RisingWave** (Rust) | Actor-per-operator; barrier checkpoints; Hummock LSM state | Barrier alignment stalls | Actor-per-node execution model |
| **Vector** (Rust) | Adaptive concurrency controller (AIMD) | Topology reconfiguration | Auto-tune parallelism per external service |
| **Fluvio** (Rust) | SmartModules — Wasm functions inline в pipeline | Wasm memory limits | Wasm-based user transforms |
| **Apache Beam** | Unified batch+stream; windowing, watermarks | Runner abstraction overhead | Window-based data aggregation patterns |
| **Apache Flink** | Checkpoint barriers; exactly-once | Complex state management | Barrier-based checkpointing protocol |

---

## Фаза 1.5 — Pain Clusters из GitHub Issues

### Cluster 1: Scheduler Deadlocks и Concurrency Limit Misbehavior (7 проектов)

**Severity**: CRITICAL

| Project | Issue | Signal | Summary |
|---------|-------|--------|---------|
| Airflow | #9975 | 71 comments | max_active_runs=1 creates multiple active runs |
| Airflow | #14205 | 27 comments | Scheduler deadlocks при max_active_runs + retry |
| Airflow | #38968 | 22 comments | Tasks queued, executor slots exhausted |
| Dagster | #20325 | 36 comments | Implausible number of PostgreSQL connections |
| Dagster | #15488 | 25 comments | Asset-level concurrency: skip when another ongoing |
| Temporal | #1507 | 27 comments (open 2021) | Priority task queues |
| Prefect | #17415 | open | Orphaned concurrency limit slots |
| Restate | #3291 | open | Rate-limit / concurrency APIs |
| Kestra | #1400 | closed | Concurrent execution limits |

**Root cause**: Concurrency permits implemented через DB row locks или in-memory counters, которые ломаются при retry, failure states, distributed execution. "Holds slot during retry backoff?" — нет правильного ответа при текущих архитектурах.

**Варианты решения для Nebula**:

| Вариант | Суть | Плюсы | Минусы | Сложность |
|---------|------|-------|--------|-----------|
| **A. RAII Permit Guards** | `Semaphore::acquire()` возвращает `PermitGuard` — Drop автоматически release. Permit привязан к lifetime action execution, не к DB row | Leak-proof by construction; нулевой overhead; идиоматичный Rust | Только in-process; при crash permits теряются (нужен recovery) | Easy |
| **B. Permit + Heartbeat** | RAII guard + periodic heartbeat в storage. Отсутствие heartbeat > TTL = permit expired, slot freed | Переживает crashes; distributed-ready | Heartbeat latency = TTL окно где slot "зависает"; нужен background reaper | Medium |
| **C. Event-Sourced Permits** | Каждый acquire/release — append-only event. Current count = projection. При recovery — replay events | Full audit trail; consistent state after crash; debuggable | Storage overhead; нужен compaction; replay cost при restart | Hard |

**Рекомендация**: **A** как базовый вариант (уже есть `Bulkhead` в `nebula-resilience` с RAII `BulkheadPermit`). Расширить до **B** когда появится persistence layer. **C** — если нужен audit trail для compliance.

**Крейты**: `nebula-resilience` (Bulkhead), `nebula-engine` (lifecycle binding), `nebula-execution` (heartbeat)

---

### Cluster 2: Priority Scheduling и Resource-Aware Execution (7 проектов)

**Severity**: CRITICAL

| Project | Issue | Signal | Summary |
|---------|-------|--------|---------|
| Tokio | #1879 | +133 reactions | Structured concurrency support |
| Tokio | #3150 | +59 reactions | Task priority |
| Temporal | #1507 | 27 comments (open 2021) | Priority task queues |
| Temporal | #8356 | open | Resource-aware worker concurrency |
| Ray | #51173 | +37 reactions | GPU object store (heterogeneous resources) |
| Ray | #20495 | +10 | Memory-aware scheduling |
| Rayon | #319 | +29 reactions | NUMA-aware scheduling |
| Taskflow | #74 | +2 | Mutual exclusion constraints |
| Dagster | #4041 | 16 comments (open 2021) | Multi-threaded executor |

**Root cause**: Все engines treat tasks as equal и workers as interchangeable → head-of-line blocking, resource misallocation. Priority невозможно retrofit.

**Варианты решения для Nebula**:

| Вариант | Суть | Плюсы | Минусы | Сложность |
|---------|------|-------|--------|-----------|
| **A. Priority Queue + Resource Tags** | `BinaryHeap` с priority; каждый action декларирует `ResourceReq { priority: u8, tags: Set<Tag> }`. Scheduler берёт highest-priority ready node | Простая реализация; deterministic ordering; low overhead | Starvation low-priority tasks без aging; нет heterogeneous pools | Easy |
| **B. Typed Worker Pools** | Отдельные pool'ы: `io_pool(128)`, `cpu_pool(num_cpus)`, `rate_limited_pool(service, 10/s)`. Action routing по ResourceReq tags | Heterogeneous execution; нет head-of-line blocking между типами; per-pool backpressure | Конфигурация pool'ов; неоптимальная утилизация если один pool пуст | Medium |
| **C. AdaptiveHEFT** | HEFT scheduling: upward-rank priority + earliest-finish-time pool selection. EWMA observed durations для adaptive estimates | Near-optimal makespan; self-tuning; учитывает real performance | O(n²p) per scheduling round; нужна history of execution times | Hard |
| **D. Mutual Exclusion Constraints** | Orthogonal к DAG edges: `exclusive_with(resource_key)`. Scheduler не запускает два action с тем же exclusive resource | Решает shared-resource conflicts (API rate limits, DB locks) | Доп. constraint graph; потенциальные deadlocks между exclusion groups | Medium |

**Рекомендация**: **A + B** как первый шаг (priority queue + 2-3 typed pools: io/cpu/rate-limited). **D** добавить для API rate limits. **C** — long-term goal когда есть execution history.

**Крейты**: `nebula-engine` (scheduler), `nebula-core` (ResourceReq type), `nebula-resilience` (pool limits), `nebula-execution` (worker pools)

---

### Cluster 3: Memory Exhaustion / OOM Under Load (6 проектов)

**Severity**: CRITICAL

| Project | Issue | Signal | Summary |
|---------|-------|--------|---------|
| n8n | #19359 | 108 comments | OOM при Extract from PDF |
| Vector | #11942 | +76 reactions | Make Vector memory-aware |
| Ray | #20495 | +10 | Memory-aware task scheduling |
| n8n | #7939 | 22 comments | RAM memory excessive usage |
| Dagster | #20325 | 36 comments | Implausible PG connections |
| RisingWave | #3750 | +25 | Online scaling |

**Root cause**: Unbounded input без memory accounting. Schedulers сами потребляют O(tasks) памяти.

**Варианты решения для Nebula**:

| Вариант | Суть | Плюсы | Минусы | Сложность |
|---------|------|-------|--------|-----------|
| **A. Memory Budget per Action** | Каждый action декларирует `estimated_memory: Bytes`. Scheduler суммирует running actions, отказывает при превышении budget | Простой; prevent OOM; user-declared → transparent | Estimates неточные; user burden; conservative estimates waste capacity | Easy |
| **B. Measured Memory + Backpressure** | Track actual RSS через `/proc/self/statm` (Linux) или `GetProcessMemoryInfo` (Windows). При >80% capacity — pause scheduling new tasks | Реальные данные; не нужны user estimates; automatic | OS-specific; measurement latency; не granular per-action | Medium |
| **C. Arena-Based Data Budget** | Данные между actions хранятся в arena с memory limit. Overflow → spill to disk (mmap). Scheduling учитывает arena usage | Granular per-data; zero-copy reads; spill prevents OOM | Arena complexity; disk I/O overhead при spill; нужен DataRef refactor | Hard |
| **D. Fjall-Style Watermark** | Monitor thread отслеживает backlog. >N pending tasks → throttle new workflow starts. Watermark = "safe level" | Proactive; prevents scheduler OOM (не только data OOM) | Latency при throttling; requires tuning thresholds | Medium |

**Рекомендация**: **B + D** — measured memory backpressure (pause scheduling at 80%) + watermark on pending queue size. **C** — следующий шаг вместе с ClaimFlow (Top-5 #2). **A** — interim solution до B.

**Крейты**: `nebula-engine` (memory monitor, backpressure), `nebula-system` (OS memory query), `nebula-execution` (arena)

---

### Cluster 4: Timeout Inflexibility (6 проектов)

**Severity**: HIGH

| Project | Issue | Signal | Summary |
|---------|-------|--------|---------|
| n8n | #11886 | 50 comments | AI nodes hardcoded 5min timeout |
| n8n | #24496 | 10 comments | Hard-cuts at 300s despite ENV |
| Dagster | #3666 | 5 comments (open 2021) | Op-level timeouts unsupported |
| Dagster | #17498 | 12 comments | Asset-level timeouts unsupported |
| Flyte | #5125 | 15 comments | Workflow-level throttling |
| Temporal | #680 | 15 comments (open 2020) | Wait for external workflow completion |

**Root cause**: Timeout hardcoded в transport layers; нет hierarchical timeout budgets.

**Варианты решения для Nebula**:

| Вариант | Суть | Плюсы | Минусы | Сложность |
|---------|------|-------|--------|-----------|
| **A. Hierarchical Timeout Budget** | Workflow timeout = total budget. Каждый action получает `remaining = workflow_deadline - now()`. Action timeout = `min(own_timeout, remaining)` | Natural composition; action не может превысить workflow deadline; zero config для простых случаев | Нужен propagation mechanism; action не знает сколько "осталось" без Context | Easy |
| **B. Timeout Escalation Chains** (из `acts` crate) | Per-action: `[{after: 1m, handler: retry}, {after: 5m, handler: alert}, {after: 30m, handler: abort}]` | Graduated response; разные стратегии на разных порогах; DX для AI/LLM actions | Configuration complexity; нужен timer infrastructure | Medium |
| **C. Moka-style Timer Wheel** | Hierarchical timer wheel (5 levels) для O(1) timeout scheduling. Generation counters для cancel/reschedule | Масштабируется до миллионов timeouts; O(1) per operation | Implementation complexity; overkill для <10K concurrent actions | Hard |

**Рекомендация**: **A** — обязательный базовый вариант. Workflow deadline propagates через `Context`. **B** — добавить для long-running actions (AI inference, human-in-the-loop). **C** — если количество concurrent actions >10K.

**Крейты**: `nebula-resilience` (Timeout уже есть — расширить до hierarchical), `nebula-engine` (deadline propagation), `nebula-execution` (Context carries remaining budget)

---

### Cluster 5: Data Passing / Serialization (5 проектов)

**Severity**: HIGH

| Project | Issue | Signal | Summary |
|---------|-------|--------|---------|
| Flyte | #4505 | 31 comments | int → float silent cast между tasks |
| Flyte | #4740 | 8 comments | dict keys int → str silent cast |
| Airflow | #13487 | 19 comments | XCom can't serialize to JSON |
| n8n | #17251 | 30 comments | Output incorrect data |
| Salsa | #10 | +29 | Serialization to disk |

**Root cause**: JSON теряет type information; нет intermediate format с full type fidelity.

**Варианты решения для Nebula**:

| Вариант | Суть | Плюсы | Минусы | Сложность |
|---------|------|-------|--------|-----------|
| **A. Schema Validation на Edges** | Каждый action декларирует JSON Schema для input/output. DAG builder проверяет совместимость schemas при construction | Ловит mismatches до execution; JSON Schema — стандарт; совместимо с текущим serde_json::Value | Не ловит subtle issues (int→float); schema overhead; не compile-time | Easy |
| **B. Content-Addressed DataRef** (ClaimFlow) | Данные хранятся в arena; actions получают `DataRef` handle. Zero-copy reads. Spill to disk для больших payloads | Eliminates serialize/copy per edge; 10× memory reduction; O(1) dedup | Refactor data passing layer; arena lifecycle management; API change | Medium |
| **C. Typed Ports с Bidirectional Checking** (TypeDAG) | Actions определяют typed ports (`Port<JsonObject>`, `Port<u64>`). DAG builder inference + checking | Compile-time type safety; eliminates ALL runtime type errors; Rust type system advantage | Requires type algebra; complex implementation; migration от serde_json::Value | Hard |
| **D. Explicit Coercion Nodes** | Вместо silent casts — explicit `CoerceNode { from: Schema, to: Schema, strategy: Lossy|Strict }` в DAG | Transparent; user controls coercion; no surprises | Verbose DAGs; user must add coerce nodes manually | Easy |

**Рекомендация**: **A** сейчас (schema validation на edges — быстрая победа). **D** для явного контроля coercion. **B** — средний срок (ClaimFlow, Top-5 #2). **C** — долгосрочная цель (TypeDAG, Top-5 #3).

**Крейты**: `nebula-core` (Port types), `nebula-validator` (schema checking), `nebula-execution` (DataRef arena), `nebula-parameter` (schema definitions)

---

### Cluster 6: Retry Inadequacies (6 проектов)

**Severity**: HIGH

| Project | Issue | Signal | Summary |
|---------|-------|--------|---------|
| Airflow | #21867 | 48 comments (open) | TaskGroup retry unsupported |
| Temporal | #131 | 4 comments (open 2020) | Retry per failure type |
| Flyte | #1276 | 2 | Map tasks retry whole map, not subtasks |
| Vector | #10870 | +67 reactions | Expand retry cases |
| Tower | #682 | +13 | Retry middleware improvements |
| n8n | #24042 | 12 comments | Tool errors fail workflow instead of error routing |

**Root cause**: Retry как simple loop counter; нет conditional/hierarchical/budget retry.

**Варианты решения для Nebula**:

| Вариант | Суть | Плюсы | Минусы | Сложность |
|---------|------|-------|--------|-----------|
| **A. Error-Classified Retry** | `RetryConfig::when(|err| err.is_transient())` — retry только transient errors (timeout, rate limit, 503). Permanent errors (auth, validation) → fail fast | Избегает бесполезных retry; экономит time budget; уже частично есть (`CallError::is_retriable()`) | Action должен классифицировать ошибки; нужен `ErrorClassify` trait | Easy |
| **B. Sub-DAG / Group Retry** | Retry целой группы nodes как unit. `RetryGroup { nodes: [A, B, C], policy: Retry(3) }`. При failure — re-execute всю группу | Решает Airflow#21867 (TaskGroup retry); atomic retry unit | Complexity: partial results от первой попытки; нужен state rollback | Medium |
| **C. Global Retry Budget** | `WorkflowConfig { total_retry_budget: Duration::from_secs(300) }`. Все retries в workflow делят общий бюджет. Исчерпан → все pending retries cancelled | Prevents thundering herd; bounded total cost; composition-safe | Hard to attribute budget fairly; one hot loop can starve others | Easy |
| **D. Retry per Fan-Out Item** | При fan-out: retry failed items individually, не всю map-операцию. Partial results: `FanInResult { succeeded: Vec<T>, failed: Vec<(Index, E)> }` | Решает Flyte#1276; granular retry; partial success | Fan-in collector complexity; downstream node must handle partial data | Medium |
| **E. Supervision Strategies** (из Bastion) | OneForOne (retry only failed), RestForOne (retry failed + downstream), OneForAll (retry entire group) | Формализованная модель; covers all cases; composable | Config complexity; user must choose strategy | Medium |

**Рекомендация**: **A** — уже почти есть в `nebula-resilience` (`is_retriable()`), расширить до configurable classifier. **C** — добавить global budget (уже есть `total_budget` в `RetryConfig`). **D** — при реализации fan-out/fan-in. **E** — формализация B через supervision vocabulary.

**Крейты**: `nebula-resilience` (RetryConfig расширение), `nebula-engine` (group retry, budget enforcement), `nebula-execution` (fan-out item retry)

---

### Cluster 7: Graceful Shutdown (6 проектов)

**Severity**: HIGH

| Project | Issue | Signal | Summary |
|---------|-------|--------|---------|
| Airflow | #18041 | 71 comments | SIGTERM kills tasks on K8s |
| n8n | #14653 | 80 comments | Connection lost after upgrade |
| Vector | #11405 | +32 | Flush sinks during shutdown |
| Vector | #19600 | +53 | Disk buffers lost at shutdown |
| Flyte | #634 | 16 comments | Intra-task checkpointing |
| Tokio | #4516 | +28 | Unhandled panic behavior |

**Root cause**: Нет lifecycle management для in-flight state; каждый компонент нуждается в defined shutdown path.

**Варианты решения для Nebula**:

| Вариант | Суть | Плюсы | Минусы | Сложность |
|---------|------|-------|--------|-----------|
| **A. CancellationToken + Grace Period** | `tokio_util::CancellationToken` propagated через Context. On shutdown: signal cancel → wait grace period → force abort. Actions check `ctx.is_cancelled()` | Cooperative; clean; Rust Drop guarantees resource cleanup; уже есть `CancellationContext` в resilience | Requires action cooperation; rogue actions ignore cancel | Easy |
| **B. Checkpoint-on-Signal** | On SIGTERM/SIGINT: trigger checkpoint barrier → persist all in-flight state → shutdown. On restart: resume from checkpoint | Zero data loss; clean restart; compose с BarrierSnap (Top-5 #5) | Checkpoint latency delays shutdown; requires persistence layer | Hard |
| **C. Drain Mode** | `Engine::drain()`: stop accepting new workflows; finish in-flight; timeout → force abort. Status API reports drain progress | Operational-friendly; predictable; enables rolling deploys | In-flight workflows must finish within timeout; long workflows problematic | Easy |
| **D. Two-Phase Shutdown** (из Vector) | Phase 1: flush outputs (sinks write remaining data). Phase 2: drop resources. Configurable timeout per phase | Separated concerns; data-safe; configurable | Two timeout configs; complexity for action developers | Medium |

**Рекомендация**: **A + C** — CancellationToken (уже есть) + drain mode для operational use. **D** — при наличии pipeline stages с buffered data. **B** — long-term с persistence layer.

**Крейты**: `nebula-resilience` (CancellationContext), `nebula-engine` (drain mode), `nebula-runtime` (signal handling, shutdown orchestration)

---

### Cluster 8: Silent Failures / Phantom States (5 проектов)

**Severity**: HIGH

| Project | Issue | Signal | Summary |
|---------|-------|--------|---------|
| Airflow | #42136 | 97 comments | Task fails, can't read logs |
| Airflow | #17507 | 66 comments | "pid does not match" — phantom tasks |
| Airflow | #34339 | 28 comments | Successful tasks marked failed |
| n8n | #13135 | 42 comments | Sub-workflow returns wrong data |
| Flyte | #4466 | 15 comments | Error messages not propagated |

**Root cause**: State store и execution process рассинхронизированы — distributed systems fundamental.

**Варианты решения для Nebula**:

| Вариант | Суть | Плюсы | Минусы | Сложность |
|---------|------|-------|--------|-----------|
| **A. Event-Sourced Execution Log** | Каждый state transition — append-only event. "Running" — не mutable row, а latest event. Recovery = replay events | Нет phantom states (never overwrite); full audit trail; debuggable | Replay cost grows; нужен snapshotting; storage growth | Medium |
| **B. Structured Error Propagation** | Каждая ошибка несёт: original error, node_id, correlation_id, attempt_number. `nebula-error` NebulaError<E> уже частично поддерживает | Нет masked errors (Flyte#4466); traceable; OTel-compatible | Requires discipline: all actions must use structured errors | Easy |
| **C. Heartbeat Liveness** | Running actions emit periodic heartbeat events. Отсутствие heartbeat > TTL → declared dead, slot freed, marked failed | Detects phantom tasks; automatic cleanup; no pid matching | TTL latency; heartbeat overhead; false positives при slow actions | Medium |
| **D. Single Source of Truth** (из Zeebe) | Execution state = projection of append-only log. Нет отдельного "state store" и "execution" — одна система | Eliminates sync problem by design; deterministic replay | Requires log infrastructure; single-threaded per partition (Zeebe's limit) | Hard |

**Рекомендация**: **B** сейчас (structured errors — `nebula-error` уже есть). **C** — при distributed execution. **A** — при реализации persistence layer. **D** — архитектурное решение для Nebula v2.

**Крейты**: `nebula-error` (structured propagation), `nebula-engine` (heartbeat, event log), `nebula-storage` (append-only log), `nebula-telemetry` (OTel trace context)

---

### Cluster 9: Dynamic Fan-Out/Fan-In (5 проектов)

**Severity**: HIGH

| Project | Issue | Signal | Summary |
|---------|-------|--------|---------|
| Airflow | #23020 | 31 comments | Names for mapped tasks |
| Airflow | #40799 | 20 comments | Named mapping for task group |
| Dagster | #4364 | 12 comments (open 2021) | Downstream of multiple dynamic outputs |
| Rayon | #210 | +66 reactions | Parallel-to-sequential iterator |
| Flyte | #1276 | 2 | Map task retry granularity |

**Root cause**: Static DAG → dynamic DAG ломает naming, retry granularity, result collection.

**Варианты решения для Nebula**:

| Вариант | Суть | Плюсы | Минусы | Сложность |
|---------|------|-------|--------|-----------|
| **A. First-Class FanOut/FanIn Nodes** | `FanOutNode { input: Vec<T>, action: ActionKey }` порождает N dynamic tasks. `FanInNode { strategy: All|Partial(min_success) }` собирает | Явная семантика; stable naming (parent_id + index); individual retry per item | Нужен новый node type; fan-in complexity | Medium |
| **B. Stable Dynamic NodeId** | `DynamicNodeId = (parent_node_id, index: u32)`. Index-based identity переживает retry и restart. Display: `"process_item[3]"` | Решает Airflow#23020 (naming); стабильные ID для monitoring и retry | Index может быть unstable если input order changes | Easy |
| **C. Partial Result Collection** | `FanInResult<T> { results: Vec<Result<T, E>>, succeeded: usize, failed: usize }`. Downstream node получает partial results | Решает Flyte#1276; partial success; downstream decides how to handle | Downstream action must handle partial data; type complexity | Medium |
| **D. Composable Sub-Workflows** (из Taskflow) | Sub-workflow = DAG node. Dynamic: `map(items, sub_workflow)` produces N instances. Each instance is independent DAG | Max flexibility; isolation; independent lifecycle per instance | Sub-workflow overhead; orchestration complexity | Hard |

**Рекомендация**: **A + B** — FanOut/FanIn как first-class primitive с stable index-based naming. **C** — в FanIn node по умолчанию. **D** — для complex dynamic patterns.

**Крейты**: `nebula-core` (FanOutNode, FanInNode, DynamicNodeId), `nebula-engine` (dynamic task spawning), `nebula-execution` (partial result collection)

---

### Cluster 10: Dead Letter Queue / Error Routing (4 проекта)

**Severity**: HIGH

| Project | Issue | Signal | Summary |
|---------|-------|--------|---------|
| Vector | #1772 | +85 reactions | Dead letter queue on sinks |
| n8n | #24042 | 12 comments | Errors fail workflow instead of routing to agent |
| n8n | #18452 | 23 comments | Exceptions masked |
| Airflow | #42136 | 97 comments | Logs inaccessible on failure |

**Root cause**: Binary success/failure model; нет error routing в альтернативные пути.

**Варианты решения для Nebula**:

| Вариант | Суть | Плюсы | Минусы | Сложность |
|---------|------|-------|--------|-----------|
| **A. Error Edge Type** | DAG edges имеют type: `DataEdge` (normal) или `ErrorEdge` (on failure). Failed node → output flows через ErrorEdge к error handler node | Явный; визуализируемый; composable; каждый node может иметь свой error path | Усложняет DAG model; нужен edge type в nebula-core | Medium |
| **B. Dead Letter Node** | Специальный `DeadLetterNode` per workflow. Все unhandled errors маршрутизируются туда. Configurable: log, alert, store, retry later | Catch-all safety net; простая конфигурация; не требует error edges на каждом node | Менее granular (все ошибки в одно место); нет per-node recovery | Easy |
| **C. Error-Kind Routing** (из `PriorityFallback`) | `ErrorRouter { Timeout → retry_node, RateLimited → wait_node, AuthFailed → alert_node }`. Routing по `CallErrorKind` | Granular; type-safe; уже есть `CallErrorKind` в resilience | Configuration per error kind; can miss unknown errors | Medium |
| **D. Try/Catch Sub-DAG** | `TryCatchNode { try: sub_dag, catch: |error| -> sub_dag }`. Ошибка в try → catch sub-dag получает error + partial results | Structured; familiar semantics; composable nesting | Sub-DAG complexity; how to pass partial state to catch | Hard |

**Рекомендация**: **A + B** — Error edges для explicit routing + DeadLetter node как catch-all. **C** — естественное расширение (уже есть `PriorityFallback` с `CallErrorKind` dispatch). **D** — для advanced error recovery.

**Крейты**: `nebula-core` (EdgeType, ErrorEdge), `nebula-workflow` (DeadLetterNode), `nebula-resilience` (ErrorRouter via CallErrorKind), `nebula-engine` (error edge traversal)

---

## Фаза 2 — Академические работы и теоретическая база

### Scheduling

| Paper | Key Result | Relevance |
|-------|-----------|-----------|
| **Graham 1966** — Bell Sys Tech J, 45(9) | List scheduling: makespan ≤ (2 - 1/m) × OPT | Baseline: naive scheduler at most 2× worse than optimal |
| **Hu 1961** — Operations Research, 9:841 | Optimal for tree-DAGs с unit-time tasks | Tree-shaped subgraphs → critical-path scheduling optimal |
| **Coffman-Graham 1972** — Acta Informatica, 1:200 | Optimal for 2 processors; (2-2/W) для W processors | Tight bound для малого числа workers |
| **Blumofe-Leiserson 1999** — JACM, 46(5):720 | Work-stealing: T₁/P + O(T∞); linear speedup при T₁/(P·T∞) >> 1 | Tokio/Rayon уже дают это; critical path — irreducible cost |
| **HEFT (Topcuoglu et al. 2002)** — IEEE TPDS, 13(3):260 | Heterogeneous Earliest-Finish-Time; O(n²·p) | Routing actions к typed worker pools по earliest finish time |
| **General DAG scheduling NP-hardness** | Strongly NP-hard на 3+ processors | Не искать optimal; heuristics — правильный подход |

### State и Persistence

| Paper | Key Result | Relevance |
|-------|-----------|-----------|
| **Young 1974** — CACM, 17(9):530 / **Daly 2006** — FGCS, 22(3):303 | Optimal checkpoint interval = √(2·C·M) | Формула для частоты snapshot'ов |
| **Shapiro et al. 2011** — INRIA RR-7506 | CRDTs: semilattice merge → eventual consistency без coordination | Distributed workflow state без locks |

### Type Systems

| Paper | Key Result | Relevance |
|-------|-----------|-----------|
| **Dunfield-Krishnaswami 2021** — ACM Comput Surv, 54(5) | Bidirectional typing: synthesis + checking | Synthesize output type → check against downstream input type |
| **Honda-Yoshida-Carbone 2008** — JACM, 63(1):9 | Multiparty session types: communication safety, session fidelity, deadlock freedom | Typed DAG edges = typed communication channels |
| **Caires-Pfenning 2010** — CONCUR 2010 / **Wadler 2012** — ICFP | Linear logic ↔ session types; use-exactly-once | Data через edge consumed exactly once — нет accidental duplication |
| **Rondon et al. 2008** — PLDI | Liquid Types: decidable refinement inference | "port 1-65535", "non-empty string" — static validation constraints |
| **van der Aalst 1997/2011** — FAC, 23(3):333 | Workflow net soundness decidable (EXPSPACE) | Pre-execution validation: deadlock detection, proper completion |

### Incremental Computation

| Paper | Key Result | Relevance |
|-------|-----------|-----------|
| **Acar et al. 2002** — POPL | Self-adjusting computation: DDG + change propagation | Re-execute only downstream of changes |
| **Hammer et al. 2014** — PLDI (Adapton) | Demand-driven incremental computation (DCG) | Skip branches nobody reads |
| **McSherry et al. 2013** — CIDR (Differential Dataflow) | (data, time, diff) triples для nested iteration | Efficient re-execution для iterative/retry workflows |
| **Mokhov et al. 2018** — ICFP (Build Systems à la Carte) | Scheduler × Rebuilder matrix; minimal + early cutoff | Hash inputs → cache check → re-execute on miss → stop propagation if same output |

### Execution Models

| Paper | Key Result | Relevance |
|-------|-----------|-----------|
| **Graefe 1994** (Volcano) — IEEE TKDE, 6(1):120 | Pull-based iterator: open-next-close; exchange operator for parallelism | Natural backpressure; exchange operator для transparent parallelism |
| **Neumann 2011** — PVLDB, 4(9):539 | Push-based compiled plans: produce/consume; 6.5 GB/s throughput | Push between pipeline stages для cache efficiency |
| **Tomasulo 1967** — IBM J R&D | Dynamic scheduling: reservation stations + register renaming + CDB | Conceptually identical to DAG executor — validates dataflow model |
| **van der Aalst et al. 2003** — D&PD, 14(1):5 | 43+ workflow patterns taxonomy | Completeness checklist для Nebula's execution model |

### Resource Management

| Result | Application |
|--------|------------|
| **Little's Law**: L = λ·W | Pool sizing: λ × W = minimum slots |
| **Erlang-C model** | Queue probability → optimal pool size для target wait% |
| **Reactive Streams spec** | Bounded-buffer backpressure с demand signaling |
| **Cache-oblivious (Frigo et al. 1999)** — FOCS | Recursive divide-and-conquer для memory hierarchy |

---

## Фаза 3 — Opportunity Gaps Matrix

| Область | Текущий подход у всех | Фундаментальная неэффективность | Issues (repo#, reactions) | Lower bound? | Impact | Feasibility | Nebula fit |
|---------|----------------------|--------------------------------|--------------------------|-------------|--------|-------------|------------|
| **Incremental re-execution** | Full re-run или manual skip | O(N) re-execution при O(1) change | Dagster#15488(25), Flyte DataCatalog, Salsa#10(+29) | Acar 2002: O(affected) | 10-100× для re-runs | Medium | High — typed ports enable content-hash |
| **Data passing** | Serialize everything to JSON/bytes | O(data_size) per edge × edges | n8n#19359(108), Flyte#4505(31), Airflow#13487(19) | O(pointer) с claim-check | 10-1000× для large data | Medium | High — serde_json::Value → DataRef |
| **Scheduling heterogeneity** | Homogeneous queue | Critical-path + IO tasks в одной очереди | Tokio#3150(+59), Temporal#1507(27), Ray#51173(+37) | HEFT: NP-hard, O(n²p) heuristic | 2-5× makespan | Medium | High — typed actions → typed pools |
| **Concurrency lifecycle** | DB locks / counters | Leaked slots при failure/retry | Airflow#9975(71), Prefect#17415, Dagster#20325(36) | Нет — engineering | ∞ reliability | Easy | High — RAII guards в Rust |
| **Graph optimization** | Execute DAG as-is | Scheduling overhead O(nodes) | Dask graph opts, Tokio#1879(+133) | O(critical path) | 30-50% для complex DAGs | Medium | High — petgraph |
| **Error routing** | Binary success/fail | No recovery path | Vector#1772(+85), n8n#24042(12), n8n#18452(23) | Нет — design | DX + reliability | Easy | High — EIP patterns |
| **Checkpoint barriers** | Full snapshot или nothing | O(state) per checkpoint; stop-the-world | Flyte#634(16), Vector#19600(+53), Arroyo model | Young/Daly √(2CM) | Crash recovery time | Hard | Medium — need persistence layer |

---

## Фаза 4 — Top-5 Кандидатов

### 1. IncrementalDAG — Incremental Re-execution через Content-Addressed Caching

**Проблема**: Все workflow engines re-execute весь DAG при изменении одного input. Temporal replays всю history. Airflow перезапускает все downstream tasks. Для workflow из 100 nodes где изменился 1 input — 99% работы выбрасывается.

**Evidence из issues**:
- `dagster-io/dagster#15488` (25 comments) — "skip materialization when another is already ongoing" → users хотят granular re-execution
- `salsa-rs/salsa#10` (+29 reactions) — serialization to disk для persistent memoization
- `salsa-rs/salsa#41` — push-based vs pull invalidation tradeoffs
- `flyteorg/flyte` DataCatalog — task-level caching по hash(signature + inputs), но только для identical tasks, не incremental
- `dagster-io/dagster#22553` (29 comments) — asset staleness detection broken → users rely on incremental model

**Кто страдает**: Temporal (full replay O(history)), Airflow (full DAG re-run), Dagster (IO manager overhead), Flyte (container restart per task), n8n (no caching at all)

**Почему никто не решил**: Требует typed edges для content hashing. Go/Python engines не имеют compile-time type info на edges. Temporal's replay model привязан к event sourcing. Airflow's XCom не содержит достаточно type info для content addressing.

**Предлагаемый подход**:
1. Каждый action output получает content hash: `hash(action_key + action_version + hash(inputs))`
2. При re-execution — проверка cache по content hash
3. **Early cutoff** (Mokhov 2018): если output не изменился, downstream не re-triggers
4. **Demand-driven** (Adapton): re-execute только ветки, чьи outputs запрошены

Алгоритм:
```
fn should_reexecute(node, cache) -> bool {
    let input_hash = hash(node.inputs.iter().map(|i| cache.get_hash(i)));
    let cached = cache.lookup(node.action_key, input_hash);
    match cached {
        Some(entry) => false,  // cache hit — skip
        None => true,          // cache miss — execute
    }
}
// After execution: if output_hash == previous_output_hash, don't propagate
```

**Cross-pollination**: Build Systems à la Carte (Mokhov et al. 2018) + Salsa incremental computation + Flyte DataCatalog

**Уникальное преимущество Nebula**: Typed ports дают content-hash на каждом edge. Go/Python engines не имеют этой информации в compile time. Nebula может вычислить "что изменилось" через type-aware diffing.

**Теоретическая база**:
- Mokhov, Mitchell, Peyton Jones. "Build Systems à la Carte." ICFP 2018 / JFP 2020 — scheduler × rebuilder taxonomy; verifying traces + early cutoff
- Acar, Blelloch, Harper. "Adaptive Functional Programming." POPL 2002 — DDG change propagation; update cost bounded by trace distance
- Hammer et al. "Adapton: Composable, Demand-Driven Incremental Computation." PLDI 2014 — demand-driven DCG

**Измеримый результат**: Для workflow из N nodes где K inputs изменились → re-execution cost O(affected subgraph) вместо O(N). Expected speedup: 10-100× для large workflows с small changes. Benchmark: workflow из 50 nodes, change 1 input, measure execution time vs full re-run.

**Крейты Nebula**: `nebula-execution` (cache lookup/store), `nebula-engine` (scheduling decisions), `nebula-core` (content hash на typed ports), `nebula-storage` (cache backend)

**Proof of concept**: Benchmark: 50-node linear DAG, change input 1, measure time to re-execute с кешем vs без. Target: <5% of full execution time.

**Путь к публикации**: OSDI / EuroSys — "Incremental Workflow Execution via Content-Addressed DAG Caching" — combining build system theory с workflow orchestration

---

### 2. ClaimFlow — Zero-Copy Data Passing через Content-Addressed Object Store

**Проблема**: Workflow engines serialize и copy данные между steps. n8n держит всё в памяти → OOM. Airflow XCom сериализует в DB. Flyte записывает в S3 per step. Для pipeline из 10 steps обрабатывающего 100MB JSON — 1GB memory + 10× serialization overhead.

**Evidence из issues**:
- `n8n-io/n8n#19359` (108 comments) — OOM при Extract from PDF
- `n8n-io/n8n#7939` (22 comments) — excessive RAM usage
- `flyteorg/flyte#4505` (31 comments) — silent int→float cast при serialization
- `apache/airflow#13487` (19 comments) — XCom can't serialize to JSON
- `vectordotdev/vector#11942` (+76 reactions) — "make Vector aware of available memory"
- `ray-project/ray#20495` (+10) — memory-aware scheduling

**Кто страдает**: n8n (all data in memory), Airflow (XCom в DB), Flyte (S3 roundtrip per step), Dagster (IO manager serialize/deserialize), Argo (artifact via S3)

**Почему никто не решил**: Требует content-addressable storage + reference counting. Python/JS engines не имеют ownership model — GC languages не могут гарантировать когда data freed. Go engines копируют по default.

**Предлагаемый подход**:
1. Action outputs записываются в **content-addressed arena** (in-process для small data, memory-mapped для large)
2. Downstream actions получают `DataRef(content_hash, arena_id)` вместо `serde_json::Value`
3. Arena использует reference counting (Arc) — data freed когда все consumers обработали
4. **Zero-copy reads**: downstream actions получают `&serde_json::Value` через arena lookup
5. **Spill to disk**: arena с memory budget; overflow → memory-mapped file

```
struct DataRef {
    content_hash: u64,
    arena: Arc<Arena>,
}

impl DataRef {
    fn read(&self) -> &serde_json::Value {
        self.arena.get(self.content_hash)  // O(1) lookup, zero copy
    }
}
```

**Cross-pollination**:
- NiFi's claim-check pattern (FlowFile metadata + Content Repository)
- Ray's distributed object store с zero-copy shared memory reads
- Rust's ownership model для deterministic lifetime management

**Уникальное преимущество Nebula**: Rust ownership model гарантирует deterministic lifetime. `Arc<Arena>` + `DataRef` = zero-copy reads + automatic cleanup. Невозможно в GC languages (Python/JS/Go) где lifetime non-deterministic.

**Теоретическая база**:
- Content-addressable storage: O(1) dedup по hash
- Arena allocation: O(1) alloc, bulk free, cache-friendly layout (slotmap pattern)
- Little's Law: L = λW — arena size = arrival_rate × processing_time

**Измеримый результат**:
- Memory: 10× reduction для multi-step pipelines (1 copy вместо N)
- Throughput: 5-10× для large payloads (eliminate serialize/deserialize per edge)
- Benchmark: 10-step pipeline, 100MB JSON input, measure memory peak + latency vs current approach

**Крейты Nebula**: `nebula-execution` (DataRef, Arena), `nebula-core` (DataRef type), `nebula-engine` (arena lifecycle), `nebula-memory` (spill-to-disk)

**Proof of concept**: Benchmark: 10-node pipeline, 50MB JSON payload, compare memory peak и latency: DataRef arena vs clone-per-edge.

**Путь к публикации**: VLDB — "Zero-Copy Data Flow in Typed Workflow Engines" — combining ownership semantics с dataflow execution

---

### 3. TypeDAG — Bidirectional Type Inference для Workflow Graphs

**Проблема**: Большинство engines обнаруживают type mismatches в runtime. Flyte silent casts int→float (#4505, 31 comments). Airflow XCom fails при serialization (#13487). n8n возвращает wrong data (#17251, 30 comments). Ни один engine не делает полную static validation DAG перед execution.

**Evidence из issues**:
- `flyteorg/flyte#4505` (31 comments) — silent int→float between tasks
- `flyteorg/flyte#4740` (8 comments) — dict keys int→str silently
- `flyteorg/flyte#1349` (2) — no Union type support
- `apache/airflow#13487` (19 comments) — serialization failure at runtime
- `n8n-io/n8n#17251` (30 comments) — output incorrect data
- `n8n-io/n8n#16195` (19 comments) — Switch node output wrong for multiple branches
- `TimelyDataflow/timely-dataflow#358` (+2) — MustUse streams (compile-time unconsumed output detection)

**Кто страдает**: Все. Flyte — самая типизированная система — всё равно silent casts. Airflow, n8n, Temporal, Cadence, Argo — нулевая типизация.

**Почему никто не решил**:
1. Python/JS engines не имеют compile-time types
2. Flyte's protobuf IDL не поддерживает refinement types
3. Go engines используют `interface{}` / `any`
4. Bidirectional type inference для dataflow graphs — нетривиальный алгоритм

**Предлагаемый подход**:
1. Каждый action declares input/output types через Rust type system
2. **Bidirectional checking** (Dunfield-Krishnaswami 2021): synthesize output types → check against downstream input types
3. **Refinement predicates** (Liquid Types): `Port<u16> where 1..=65535`, `Port<String> where !empty`
4. **Soundness check** (van der Aalst 2011): verify no deadlocks, all nodes reachable, proper completion
5. **MustUse** (timely #358): compile-time error для unconsumed outputs

```
// Type synthesis: action declares output type
impl HttpAction {
    type Output = JsonObject;  // synthesized
}

// Type checking: downstream input checked against upstream output
impl ParseAction {
    type Input = JsonObject;   // checked against HttpAction::Output
}

// Refinement: runtime constraint validated at definition time
type Port = Refined<u16, InRange<1, 65535>>;
```

**Cross-pollination**:
- Dunfield-Krishnaswami bidirectional typing (compilers → workflow DAGs)
- Honda-Yoshida-Carbone session types (protocol verification → edge contracts)
- Liquid Types (Rondon et al.) → refinement predicates on parameters
- van der Aalst workflow net soundness (Petri nets → DAG validation)

**Уникальное преимущество Nebula**: Rust's type system + typed ports = compile-time edge validation. Невозможно в dynamically-typed engines (Python/JS). Even Flyte's protobuf types lose precision at language boundaries.

**Теоретическая база**:
- Dunfield, Krishnaswami. "Bidirectional Typing." ACM Comput Surv, 54(5), 2021
- Honda, Yoshida, Carbone. "Multiparty Asynchronous Session Types." JACM 63(1):9, 2016
- Rondon, Kawaguchi, Jhala. "Liquid Types." PLDI 2008
- van der Aalst. "Verification of Workflow Nets." FAC 23(3):333, 2011

**Измеримый результат**:
- Eliminate 100% of runtime type-mismatch errors (move to definition-time)
- Catch issues like Flyte#4505 at DAG construction, not at execution
- Soundness check: prove no deadlocks in O(EXPSPACE) but practical for <1000 nodes

**Крейты Nebula**: `nebula-core` (type algebra), `nebula-validator` (soundness check), `nebula-parameter` (refinement predicates), `nebula-workflow` (DAG validation)

**Proof of concept**: Define 10 actions with typed ports. Construct a DAG with one intentional type mismatch. Verify caught at construction time, not execution.

**Путь к публикации**: POPL / ICFP — "Session-Typed Workflow Graphs: Static Verification of Dataflow Correctness"

---

### 4. AdaptiveHEFT — Heterogeneous Resource-Aware DAG Scheduling

**Проблема**: Workflow engines используют homogeneous FIFO queue. HTTP call (100ms IO-bound) и ML inference (10s CPU-bound) конкурируют за те же workers. Critical-path tasks блокируются за low-priority batch tasks. Нет resource-type awareness.

**Evidence из issues**:
- `tokio-rs/tokio#3150` (+59 reactions) — task priority
- `tokio-rs/tokio#1879` (+133 reactions) — structured concurrency
- `temporalio/temporal#1507` (27 comments, open 2021) — priority task queues
- `temporalio/temporal#8356` — resource-aware worker concurrency
- `ray-project/ray#51173` (+37) — GPU object store
- `ray-project/ray#20495` (+10) — memory-aware scheduling
- `rayon-rs/rayon#319` (+29) — NUMA-aware scheduling
- `taskflow/taskflow#74` (+2) — mutual exclusion constraints
- `dagster-io/dagster#4041` (16 comments, open 2021) — multi-threaded executor
- `kestra-io/kestra#565` — worker type differentiation

**Кто страдает**: Temporal (single queue per namespace), Airflow (executor slots without type), Dagster (single-threaded), Flyte (K8s pods without affinity intelligence), n8n (single-threaded), Kestra (undifferentiated workers)

**Почему никто не решил**:
1. Go/Python engines не имеют zero-cost abstractions для scheduling
2. Retrofitting priority в existing FIFO queue breaks fairness guarantees
3. HEFT algorithm (O(n²p)) считается слишком expensive для per-execution scheduling
4. Resource type система требует DI framework (Nebula's Context)

**Предлагаемый подход**:
1. Каждый action объявляет **resource profile**: `ResourceReq { cpu: Weight, io: Weight, memory: Bytes, tags: Set<Tag> }`
2. Engine maintains **typed worker pools**: CPU-pool, IO-pool, GPU-pool, rate-limited-pool (per external service)
3. **AdaptiveHEFT scheduler**:
   - Compute upward rank (critical path priority)
   - Для каждого ready node: select pool с earliest-finish-time
   - **Adaptive**: корректировать finish-time estimates по observed execution times (EWMA)
4. **Mutual exclusion**: actions sharing exclusive resources (API rate limit) get exclusion constraints orthogonal to DAG edges
5. **Memory-aware**: reject scheduling если estimated memory exceeds budget (Vector #11942 pattern)

```
struct AdaptiveScheduler {
    pools: HashMap<PoolType, WorkerPool>,
    history: HashMap<ActionKey, ExecutionStats>,  // EWMA of duration, memory
}

fn schedule_next(&mut self, ready: &[NodeId], dag: &DAG) -> Vec<(NodeId, PoolType)> {
    let ranked = self.compute_upward_ranks(ready, dag);  // critical path priority
    ranked.iter().map(|node| {
        let req = dag.resource_req(node);
        let pool = self.earliest_finish_pool(req);
        (*node, pool)
    }).collect()
}
```

**Cross-pollination**:
- HEFT algorithm (HPC → workflow orchestration)
- Vector's AIMD adaptive concurrency (observability → workflow)
- Taskflow's mutual exclusion (C++ DAG scheduler → Rust)
- Tokio's work-stealing (runtime → application-level scheduling)

**Уникальное преимущество Nebula**:
1. Typed actions → automatic resource profile inference
2. Rust zero-cost abstractions → scheduling overhead amortized
3. `nebula-resilience` already has Bulkhead + RateLimiter → extend to resource pools

**Теоретическая база**:
- Topcuoglu, Hariri, Wu. "HEFT." IEEE TPDS 13(3):260, 2002
- Graham. "Bounds for Multiprocessing Anomalies." Bell Sys TJ 45(9), 1966 — (2-1/m) baseline
- Blumofe, Leiserson. "Work Stealing." JACM 46(5):720, 1999

**Измеримый результат**:
- Makespan reduction: 2-5× для mixed IO/CPU workflows vs FIFO
- Memory: prevent OOM via budget enforcement
- Benchmark: 30-node DAG с mix of 1ms IO и 1s CPU tasks, measure makespan FIFO vs AdaptiveHEFT

**Крейты Nebula**: `nebula-engine` (scheduler), `nebula-execution` (worker pools), `nebula-resilience` (resource budgets), `nebula-core` (ResourceReq type)

**Proof of concept**: 30-node DAG: 15 HTTP calls (IO-bound, 100ms), 15 compute tasks (CPU-bound, 1s). Two pools: IO-pool (100 concurrent), CPU-pool (4 concurrent). Measure makespan vs single FIFO pool.

**Путь к публикации**: EuroSys / SoCC — "Heterogeneous Resource Scheduling for Typed Workflow DAGs"

---

### 5. BarrierSnap — Consistent Snapshots через Barrier Injection

**Проблема**: Workflow engines либо checkpoint всё (stop-the-world), либо ничего (lose progress on crash). Temporal replays full history. Flyte restarts from container. n8n теряет всю execution. Нет incremental consistent snapshots без остановки pipeline.

**Evidence из issues**:
- `flyteorg/flyte#634` (16 comments) — intra-task checkpointing and resumable execution
- `vectordotdev/vector#19600` (+53 reactions) — disk buffers lost at shutdown
- `vectordotdev/vector#11405` (+32) — flush sinks during shutdown
- `apache/airflow#18041` (71 comments) — SIGTERM kills tasks, loses progress
- `n8n-io/n8n#14653` (80 comments) — connection lost, execution lost
- `tokio-rs/tokio#4516` (+28) — unhandled panic behavior

**Кто страдает**: Temporal (full replay O(history)), Airflow (task instance killed), n8n (no checkpointing), Flyte (container restart), Dagster (IO manager checkpoint is coarse), Hatchet/Inngest (PostgreSQL-dependent)

**Почему никто не решил**:
1. Consistent snapshot без stop-the-world требует barrier protocol (Chandy-Lamport)
2. Streaming engines (Flink, Arroyo) решили это, но workflow engines не переняли
3. Checkpoint cost для JSON workflow state считался "too cheap to optimize" — но при scale это доминирует
4. Python/JS engines не имеют deterministic cleanup (GC)

**Предлагаемый подход**:
1. **Barrier injection** (inspired by Flink/Arroyo): inject checkpoint marker через DAG edges
2. When barrier reaches a node: node persists its current state + input hash
3. When all nodes acknowledge barrier: snapshot is consistent
4. **Recovery**: load last consistent snapshot → re-execute only nodes after the barrier
5. **Human-in-the-loop** (LangGraph pattern): barrier at "approval" nodes → pause → resume from snapshot
6. **Optimal interval** (Young/Daly): checkpoint every √(2·C·M) where C = checkpoint cost, M = MTBF

```
enum PipelineMessage<T> {
    Data(T),
    Barrier(BarrierId),
}

impl Node {
    async fn process(&mut self, msg: PipelineMessage<DataRef>) {
        match msg {
            PipelineMessage::Data(data) => { /* normal processing */ }
            PipelineMessage::Barrier(id) => {
                self.persist_state(id).await;  // checkpoint
                self.forward_barrier(id).await; // propagate downstream
            }
        }
    }
}
```

**Cross-pollination**:
- Flink's checkpoint barriers (stream processing → workflow orchestration)
- Arroyo's epoch-based checkpointing (Rust streaming → Rust workflows)
- Chandy-Lamport algorithm (distributed systems → workflow snapshots)
- LangGraph's human-in-the-loop via checkpoints (AI orchestration → general workflows)
- Young/Daly formula (HPC checkpointing → workflow persistence)

**Уникальное преимущество Nebula**:
1. Rust's `Drop` + RAII → deterministic state cleanup even on panic
2. Single-process (for now) eliminates distributed barrier complexity
3. `nebula-resilience` CancellationContext → cooperative cancellation already exists
4. Typed ports → barrier can carry type information for validation on resume

**Теоретическая база**:
- Chandy, Lamport. "Distributed Snapshots." ACM TOCS 3(1):63-75, 1985
- Young. "Optimum Checkpoint Interval." CACM 17(9):530, 1974
- Daly. "Higher Order Checkpoint Estimate." FGCS 22(3):303, 2006
- Carbone et al. "Apache Flink: Stream and Batch Processing in a Single Engine." IEEE Data Eng Bull 38(4), 2015

**Измеримый результат**:
- Recovery time: O(since_last_checkpoint) вместо O(full_history)
- For 100-node workflow running 1 hour: recovery from 30s checkpoint vs 1h replay = 120× improvement
- Checkpoint overhead: <5% of execution time (Young/Daly optimal interval)
- Enables human-in-the-loop without re-execution

**Крейты Nebula**: `nebula-engine` (barrier injection), `nebula-execution` (checkpoint persist/restore), `nebula-storage` (snapshot backend), `nebula-workflow` (approval node support)

**Proof of concept**: 20-node pipeline, inject barrier every 10 nodes, kill process at node 15, recover from checkpoint at node 10, measure recovery time vs full restart.

**Путь к публикации**: VLDB / SIGMOD — "Barrier-Based Checkpointing for Workflow DAGs: Unifying Crash Recovery and Human-in-the-Loop"

---

## Фаза 5 — Cross-Pollination карта

### Из баз данных → Workflow Scheduling

| Идея | Откуда | Куда в Nebula | Почему не очевидно | Потенциал |
|------|--------|--------------|-------------------|-----------|
| Content-addressed storage | Git, IPFS, Nix | Node output caching | Workflow engines не рассматривают outputs как addressable content | 10-100× re-execution |
| Volcano pull model | Query engines (Graefe 1994) | Pipeline backpressure | Workflow engines используют push (fire-and-forget), не pull | Natural backpressure |
| Compiled push plans | Neumann 2011 | High-throughput pipelines | Считается database-only optimization | 5-10× throughput |
| Arrangement-based IVM | Materialize | Incremental workflow re-execution | Streaming SQL technique не связана с workflow в литературе | Only re-process diffs |

### Из компиляторов → DAG Optimization

| Идея | Откуда | Куда в Nebula | Почему не очевидно | Потенциал |
|------|--------|--------------|-------------------|-----------|
| Bidirectional type checking | Type inference (Dunfield 2021) | Typed port validation | Workflow engines не формализуют edge types | Eliminate runtime type errors |
| Session types | Protocol verification (Honda 1998) | Edge contracts | "Communication protocol" ↔ "data flow edge" mapping non-obvious | Deadlock-free by construction |
| Liquid/refinement types | Dependent types (Rondon 2008) | Parameter validation | "port: 1-65535" как type, не runtime check | Static constraint validation |
| SSA → Graph optimization | Compiler IR passes | DAG fusion, dead branch elimination | Workflow as "IR" that can be optimized before "execution" | 30-50% scheduling overhead |

### Из streaming engines → Workflow State

| Идея | Откуда | Куда в Nebula | Почему не очевидно | Потенциал |
|------|--------|--------------|-------------------|-----------|
| Checkpoint barriers | Flink, Arroyo | Consistent workflow snapshots | Streaming ≠ workflow в обычном представлении | 120× recovery time |
| Watermarks | Beam, Flink | Progress tracking | Workflow engines poll, не track progress | Precise completion notification |
| Frontier tracking | Timely Dataflow | Subgraph completion | "Timestamp frontier" = "dependencies complete" | Eliminates polling |

### Из HPC / Task Systems → Execution Engine

| Идея | Откуда | Куда в Nebula | Почему не очевидно | Потенциал |
|------|--------|--------------|-------------------|-----------|
| HEFT scheduling | Heterogeneous computing (Topcuoglu 2002) | Resource-aware dispatch | "HPC scheduling" не ассоциируется с "workflow automation" | 2-5× makespan |
| Mutual exclusion constraints | Taskflow (C++) | Resource lock scheduling | DAG edges ≠ all constraints; exclusion is orthogonal | Correct resource sharing |
| Condition tasks | Taskflow | Dynamic branching | Integer branch selection vs boolean conditions | Simpler control flow |
| Graph fusion | Dask, TBB | Pre-execution optimization | "Fuse linear chains" — trivial but nobody does it | 30-50% overhead |

### Из FP / Type Theory → Workflow Model

| Идея | Откуда | Куда в Nebula | Почему не очевидно | Потенциал |
|------|--------|--------------|-------------------|-----------|
| ZIO[R, E, A] | Scala ZIO | Action<Ctx, E, Output> | Typed error channel как часть signature | Exhaustive error handling |
| Resource monad | cats-effect | Connection lifecycle | Bracket pattern для acquire/use/release | Leak-free resources |
| Process managers | Commanded (Elixir) | Long-running workflow coordination | CQRS/ES pattern → workflow orchestration | Event-driven workflows |
| Demand-driven backpressure | GenStage (Elixir) | Action scheduling | Consumer drives producer → natural load balancing | No queue overflow |

### Из Build Systems → Re-execution Strategy

| Идея | Откуда | Куда в Nebula | Почему не очевидно | Потенциал |
|------|--------|--------------|-------------------|-----------|
| Verifying traces | Shake, Bazel (Mokhov 2018) | Cache validation | "Build system" = "workflow engine" (same formal model) | Minimal re-execution |
| Early cutoff | Shake (Mokhov 2018) | Propagation optimization | Same output → skip downstream, even if input changed | Exponential savings |
| Constructive traces | Cloud Build (Mokhov 2018) | Distributed caching | Share cached results across workflow instances | Cross-workflow dedup |
| Self-adjusting computation | Acar et al. 2002 | DDG change propagation | Academic theory → production system | Optimal incremental |

### Из Network Protocols → Execution Flow

| Идея | Откуда | Куда в Nebula | Почему не очевидно | Потенциал |
|------|--------|--------------|-------------------|-----------|
| AIMD congestion control | TCP, Vector | Adaptive concurrency | "Network congestion" ↔ "API rate limiting" isomorphism | Auto-tuned parallelism |
| Token bucket | Network rate limiting | Already in nebula-resilience | — | — |
| Circuit breaker | Hystrix pattern | Already in nebula-resilience | — | — |

### Из Game Engines → Task Scheduling

| Идея | Откуда | Куда в Nebula | Почему не очевидно | Потенциал |
|------|--------|--------------|-------------------|-----------|
| Generational arena (slotmap) | ECS game engines | Node storage | "Game engine memory" → "workflow node storage" | O(1) node lookup |
| Job system | Unity DOTS, Bevy ECS | Action execution | Frame-based job scheduling → step-based action scheduling | Cache-friendly execution |

---

## Полный каталог improvement-кандидатов (25 идей)

Ниже — ВСЕ идеи, отсортированные по impact × feasibility × uniqueness. Top-5 описаны подробно в Фазе 4. Остальные 20 — в компактном формате.

### Tier S — Breakthrough (Top-5, описаны в Фазе 4)

| # | Название | Суть | Impact | Feasibility | Venue |
|---|----------|------|--------|-------------|-------|
| S1 | **IncrementalDAG** | Content-addressed caching + early cutoff + demand-driven re-execution | 10-100× re-execution | Medium | OSDI/EuroSys |
| S2 | **ClaimFlow** | Zero-copy data passing через content-addressed arena + DataRef handles | 10× memory, 5-10× throughput | Medium | VLDB |
| S3 | **TypeDAG** | Bidirectional type inference + session types + refinement types на workflow graphs | Eliminate runtime type errors | Hard | POPL/ICFP |
| S4 | **AdaptiveHEFT** | Heterogeneous resource-aware DAG scheduling с EWMA-adaptive estimates | 2-5× makespan | Medium | EuroSys/SoCC |
| S5 | **BarrierSnap** | Flink-style checkpoint barriers для consistent workflow snapshots без stop-the-world | 120× recovery time | Hard | VLDB/SIGMOD |

### Tier A — High Impact (следующие 10)

#### A1. DemandFlow — Demand-Driven Backpressure (из GenStage/Broadway)

**Проблема**: Push-based execution без flow control → OOM при медленных downstream steps.
**Механизм**: Downstream steps отправляют demand tokens upstream. Producers не emit больше чем запрошено. Min/max demand thresholds для automatic replenishment.
**Откуда**: Elixir GenStage demand-driven model.
**Impact**: 10× reduction memory usage для pipelines с heterogeneous step speeds.
**Feasibility**: Medium — требует изменение execution model с push на pull.
**Крейты**: `nebula-engine` (demand tracking), `nebula-execution` (backpressure signals).
**Evidence**: n8n#19359 (108 comments, OOM), Vector#11942 (+76, memory awareness).

#### A2. SupervisorDAG — Erlang-Style Supervision Trees для Workflow Groups

**Проблема**: Binary failure model — или fail-fast (один node упал → всё отменяется), или continue-on-error (игнорируем ошибки).
**Механизм**: Три supervision strategy из Bastion/Erlang OTP:
- `OneForOne`: restart только failed node
- `OneForAll`: restart все nodes в группе (shared state)
- `RestForOne`: restart failed + все downstream
**Откуда**: Bastion (Rust), kotlinx.coroutines SupervisorJob, Erlang OTP.
**Impact**: Significant — enables workflows где одни ветки critical, другие best-effort.
**Feasibility**: Medium.
**Крейты**: `nebula-engine` (supervision groups), `nebula-workflow` (group definition).
**Evidence**: Airflow#21867 (48 comments, TaskGroup retry), tokio#1879 (+133, structured concurrency).

#### A3. AdaptiveConcurrency — AIMD Auto-Tuning для External API Calls

**Проблема**: Статические rate limits — или слишком aggressive (429 errors), или слишком conservative (waste capacity).
**Механизм**: TCP congestion control (Additive Increase, Multiplicative Decrease) применённый к HTTP concurrency. EWMA of response time как signal. Increase concurrency при stable RTT, decrease при RTT growth.
**Откуда**: Vector's Adaptive Request Concurrency controller.
**Impact**: 2-5× throughput; eliminates manual concurrency tuning.
**Feasibility**: Easy — можно реализовать как альтернативный rate limiter в `nebula-resilience`.
**Крейты**: `nebula-resilience` (AdaptiveRateLimiter уже есть — добавить AIMD strategy).
**Evidence**: Vector#10870 (+67, expand retry scope), Temporal#8356 (resource-aware concurrency).

#### A4. GraphFusion — Pre-Execution DAG Optimization Passes

**Проблема**: Scheduling overhead O(nodes) даже для простых linear chains.
**Механизм**: Перед execution — optimization passes:
1. **Fuse linear chains**: A→B→C (все sequential, один consumer) → fused single task
2. **Cull dead branches**: branches whose outputs не consumed — remove
3. **Inline constants**: nodes с constant output → replace with literal
4. **Dominator analysis**: identify synchronization barriers → schedule non-barriers freely
**Откуда**: Dask HighLevelGraph optimization, compiler SSA passes, petgraph dominators.
**Impact**: 30-50% scheduling overhead reduction для complex DAGs.
**Feasibility**: Medium.
**Крейты**: `nebula-workflow` (optimization passes), `nebula-engine` (fused execution).
**Evidence**: petgraph#551 (+30, crate redesign), Dask graph optimization passes.

#### A5. SagaCompensation — Compensating Transactions для Multi-Step Workflows

**Проблема**: При partial failure multi-step workflow — нет built-in way to undo completed steps.
**Механизм**: Каждый action опционально декларирует `compensate()` method. При failure — engine вызывает compensations в reverse order для уже завершённых steps.
**Откуда**: Apache Camel Saga EIP, Commanded (Elixir) process managers.
**Impact**: Enables reliable distributed workflows (payments, multi-service updates).
**Feasibility**: Medium.
**Крейты**: `nebula-action` (CompensatingAction trait), `nebula-engine` (compensation orchestration).
**Evidence**: Нет прямых issues — но это gap во ВСЕХ workflow engines.

#### A6. ChannelReducers — Merge Functions для Multi-Input Ports

**Проблема**: Когда несколько upstream nodes пишут в один downstream input — как merge?
**Механизм**: Каждый input port получает configurable reducer: `Append` (collect all), `Overwrite` (last wins), `Merge(fn)` (custom merge function), `Race` (first arrival wins).
**Откуда**: LangGraph channel reducers, AutoGen activation policies ("all" vs "any").
**Impact**: Solves fan-in merge problem cleanly; enables race patterns.
**Feasibility**: Easy — расширение существующей port системы.
**Крейты**: `nebula-action` (ReducerPolicy на InputPort), `nebula-engine` (merge execution).
**Evidence**: Dagster#4364 (12 comments, downstream of multiple dynamic outputs).

#### A7. TaskArena — Isolated Resource Pools per Workflow Group

**Проблема**: Один runaway workflow starves всех остальных (noisy neighbor).
**Механизм**: Каждая workflow group (tenant, priority class) получает isolated task arena с guaranteed minimum resources. Inspired by Intel TBB task arenas.
**Откуда**: Intel TBB task arenas, Ray resource isolation.
**Impact**: Prevents noisy-neighbor; enables SLA guarantees.
**Feasibility**: Medium.
**Крейты**: `nebula-engine` (arena allocation), `nebula-execution` (resource budgets per arena).
**Evidence**: Temporal#1507 (27 comments, priority queues), tokio#3150 (+59, task priority).

#### A8. WasmSteps — WASM-Based User-Defined Workflow Steps

**Проблема**: User-defined actions must run in-process (unsafe) или в containers (slow).
**Механизм**: User actions compiled to WASM → sandboxed execution с memory limits и timeout. Hot-reloadable без restart. Chain lightweight WASM steps into fused execution units.
**Откуда**: Fluvio SmartModules, Arroyo WASM UDFs, Nebula Phase 3 sandbox plans.
**Impact**: Safe, portable, hot-reloadable custom steps; 10-100× vs container-per-step.
**Feasibility**: Medium — requires WASM runtime integration (wasmtime).
**Крейты**: `nebula-runtime` (WASM executor), `nebula-plugin` (WASM plugin loading).
**Evidence**: ADR 008 (InProcessSandbox Phase 3 target).

#### A9. ProgressFrontier — Timely-Style Progress Tracking

**Проблема**: Workflow engines poll для completion status; нет precise progress information.
**Механизм**: Nodes track "frontiers" — the minimal set of input versions that may still arrive. When frontier advances past version T → node finalizes results for T and releases resources.
**Откуда**: Timely Dataflow progress tracking protocol (Naiad paper).
**Impact**: Order-of-magnitude memory efficiency; enables workflow loops with well-defined termination.
**Feasibility**: Hard — requires fundamental execution model changes.
**Крейты**: `nebula-execution` (frontier tracking), `nebula-engine` (progress protocol).
**Evidence**: Теоретический — но solving workflows' loop problem (DAGs can't cycle; frontiers enable it).

#### A10. DiffExecution — Differential Dataflow для Incremental Workflow Updates

**Проблема**: При incremental input changes — весь workflow re-executes.
**Механизм**: Track differences `(data, time, +1/-1)` через workflow graph. Propagate only deltas.
**Откуда**: Differential Dataflow (McSherry), Materialize.
**Impact**: 100-1000× для incremental re-execution with small changes.
**Feasibility**: Hard — requires difference algebra on serde_json::Value.
**Крейты**: `nebula-execution` (diff tracking), `nebula-engine` (delta propagation).
**Evidence**: Теоретический — strongest incremental computation model available.

### Tier B — Moderate Impact (следующие 5)

#### B1. AutoBatch — Cross-Instance Operation Batching

**Суть**: Когда N workflow instances делают один и тот же API call с разными параметрами → автоматически batch в один API call (если API поддерживает batching). Compatibility checking ensures только safe batching.
**Откуда**: Meilisearch autobatcher, Broadway batch_key+batch_size+batch_timeout.
**Impact**: 10× reduction API calls. **Feasibility**: Medium.
**Крейты**: `nebula-engine` (batch detection), `nebula-action` (BatchableAction trait).

#### B2. ContentAddressedAudit — Tamper-Evident Execution Traces

**Суть**: Hash(inputs + action_key + output) → content-addressed execution trace. Any tampering = different hash. Enables trustless workflow verification для compliance.
**Откуда**: DAML content-addressed contracts.
**Impact**: Audit compliance + aggressive memoization (same inputs → same output guaranteed). **Feasibility**: Easy.
**Крейты**: `nebula-execution` (trace hashing), `nebula-storage` (content-addressed log).

#### B3. VNodePartitioning — Fine-Grained State Partitioning для Elastic Scaling

**Суть**: Stateful workflow step state разбит на virtual nodes (vnodes). При scale up/down — только affected vnodes мигрируют, не весь state.
**Откуда**: RisingWave vnode model.
**Impact**: 10× faster scale-up/down для stateful workflows. **Feasibility**: Hard.
**Крейты**: `nebula-execution` (vnode partitioning), `nebula-storage` (vnode migration).

#### B4. LocalityScheduling — Rendezvous Hashing для Cache-Aware Worker Assignment

**Суть**: Steps consistently assigned к тому же worker через rendezvous hashing. Maximizes connection pool reuse и local caching.
**Откуда**: Quickwit rendezvous hashing, Ray locality-aware scheduling.
**Impact**: 3-5× для workflows с high cache locality. **Feasibility**: Easy.
**Крейты**: `nebula-engine` (scheduler hashing).

#### B5. ReactiveOutputs — Continuously-Updated Workflow Results (Materialized Views)

**Суть**: Workflow output — не ephemeral result, а continuously-maintained materialized view. При input change → output incrementally updated.
**Откуда**: Materialize arrangement-based IVM.
**Impact**: Enables real-time workflow outputs. **Feasibility**: Hard.
**Крейты**: `nebula-execution` (materialized output), `nebula-storage` (incremental state).

### Tier C — Niche / Long-Term (5 идей)

| # | Название | Суть | Откуда | Impact | Feasibility |
|---|----------|------|--------|--------|-------------|
| C1 | **CRDTState** | Workflow state как CRDTs (grow-only set) → coordination-free distributed execution | Shapiro 2011 | Enables distributed execution | Hard |
| C2 | **WindowedSteps** | Beam-style windowing (fixed/sliding/session) для streaming workflows | Apache Beam | New workflow class | Hard |
| C3 | **CombineLatest** | Re-execute step whenever ANY upstream changes, with latest values from ALL | Swift AsyncAlgorithms | Reactive patterns | Medium |
| C4 | **RoutingSlip** | Dynamic routing где path определяется data в message, не graph structure | Apache Camel EIP | Flexible routing | Easy |
| C5 | **PerEntityOrdering** | Parallel across entities, ordered within entity (per-key ordering guarantee) | Propulsion (F#) | 10-100× event workflows | Easy-Medium |

---

## Приложение 0: 8 фундаментальных неэффективностей домена

Каждая из этих неэффективностей присутствует у ВСЕХ движков в своём классе и не решена ни одним из них:

| # | Неэффективность | Кто страдает | Текущий workaround | Nebula-решение |
|---|----------------|-------------|-------------------|----------------|
| 1 | **Replay Tax** — O(n) history replay при каждом workflow task | Temporal, Cadence, Restate | Sticky queues (delta replay), Continue-As-New | Snapshot-based checkpointing (BarrierSnap, S5) |
| 2 | **Scheduler Poll Loop** — 50ms-1s latency floor от polling | Temporal, Conductor, Windmill, Hatchet, Camunda | Long-poll, interval tuning | Push-based reactive dispatch via EventBus |
| 3 | **Container Startup Tax** — 1-30s per step в K8s engines | Argo, Flyte | Image caching, sidecar executors | In-process + WASM sandboxing (WasmSteps, A8) |
| 4 | **Database-as-Queue** — SQL lock contention ceiling | Airflow, Windmill, Hatchet, Kestra, DolphinScheduler | Index tuning, Kafka/Redis sidecars | Embedded lock-free queue (crossbeam channels) |
| 5 | **Python Runtime Tax** — DAG parsing, GIL, import overhead | Airflow, Dagster, Prefect, Luigi | Lazy imports, standalone processor | Rust-native (уже есть) |
| 6 | **etcd/CRD Size Wall** — K8s state limits + write amplification | Argo, Flyte | CRD compression, S3 offloading | Purpose-built state store (не K8s-dependent) |
| 7 | **Inter-Task Data Copy** — serialize/deserialize на каждом edge | n8n (OOM), Airflow (XCom→DB), Flyte (→S3), Dagster (IO managers) | External storage references | Zero-copy DataRef arena (ClaimFlow, S2) |
| 8 | **Static Concurrency** — фиксированные limits без adaptive feedback | Все (Temporal, Airflow, Dagster, n8n, Kestra, Flyte) | Manual tuning | AIMD adaptive concurrency (AdaptiveConcurrency, A3) |

### "Ideas to Steal" — конкретные паттерны для заимствования

| Откуда | Что | Как применить |
|--------|-----|--------------|
| Windmill | QuickJS для expression evaluation (13-16× faster than V8) | `nebula-expression` engine: evaluate template expressions через lightweight JS/expr runtime |
| Flyte | Protobuf type system + `TypeTransformer` pattern | Typed ports с configurable transformers между типами (TypeDAG, S3) |
| Dagster | Software-defined assets — dependency inference from function signatures | Action metadata auto-derives port dependencies из Rust type system |
| Temporal | Deterministic replay concept (но с snapshots, не full replay) | BarrierSnap (S5): snapshot state → replay only from last checkpoint |
| Flyte | `ResourceVersionCache` "turbo-mode" — batch state evaluations | Batch node state checks в frontier scheduler вместо per-node queries |
| n8n | "Paired items" — data lineage tracking через workflow execution | DataRef provenance: каждый DataRef несёт lineage (source node + transform chain) |
| Luigi | Idempotency-by-design — Target.exists() = completion signal | Content-addressed cache check в IncrementalDAG (S1): hash(inputs) → cache hit = skip |
| NiFi | Claim-Check pattern — FlowFile metadata + Content Repository | ClaimFlow (S2): metadata on edges, payload in arena |
| Restate | Virtual Objects — actor + durable state per entity | Stateful workflow steps с per-entity isolated KV state |
| Camunda/Zeebe | Append-only log as source of truth → deterministic replay | Event-sourced execution log для audit + time-travel debugging |

---

## Приложение A: Глубокие алгоритмические идеи из Rust крейтов

### Petgraph: Dominator Tree для Critical Path

Алгоритм `dominators`/`immediate_dominator` из petgraph находит "must-pass-through" nodes в DAG. Это идентифицирует join points и critical path автоматически, давая optimal parallel execution schedule. В workflow engine: dominator tree показывает какие nodes являются synchronization barriers — все пути от root к ним проходят через одни и те же predecessors.

### Salsa: Revision + Durability + Backdating

Salsa's подход к incremental computation глубже чем простой cache:
- **Revision tracking**: Каждая мутация инкрементирует global revision counter; memoized results tagged с revision
- **Red-green algorithm**: Shallow verify O(1) → deep verify (walk deps) только если durability level changed
- **Durability levels**: Inputs classified по частоте изменений — shallow verify пропускает всю dependency walk для stable inputs
- **Backdating**: Если dependency изменился но output тот же → dependents НЕ инвалидируются (early cutoff)
- **Cycle detection**: Три стратегии — Panic, Fixpoint (iterate до convergence), FallbackImmediate

Это прямо применимо к IncrementalDAG: durability levels маппятся на "workflow parameters that rarely change" vs "input data that changes every run".

### Moka: Hierarchical Timer Wheel

O(1) amortized для timeout scheduling через 5-level timer wheel (секунды → дни). Expiry-generation counters инвалидируют stale timer events без удаления. Для Nebula: управление миллионами concurrent step timeouts с O(1) per operation, что решает Cluster 4 (timeout inflexibility).

### Bastion: Supervision Strategies

Три стратегии прямо из Erlang/OTP, маппятся на workflow error handling:
- **OneForOne**: Restart только failed node
- **OneForAll**: Restart все nodes в группе (shared state)
- **RestForOne**: Restart failed + все downstream

+ Restart policies: `Always`, `Never`, `Tries(n)` + backoff. Это формализованная модель retry на уровне node groups, не отдельных nodes.

### Sled: FlushEpoch для State Checkpointing

Writers "check in" в current epoch → perform mutations → epoch flushes atomically. Background flusher ставит SEAL_BIT, blocking new writers from sealed epoch. **Cooperative serialization**: writer который трогает данные старого epoch помогает их сериализовать.

Для BarrierSnap: batch workflow state mutations within an execution epoch, flush atomically. Writers (action executions) cooperatively help with checkpointing.

### Tokio: Task Budgeting

Каждая task получает budget = 128 polls. После исчерпания — принудительный yield. Prevents monopolization executor'а. LIFO slot: most recently woken task идёт в special slot для cache locality, disabled после 3 consecutive uses (starvation guard).

Для Nebula: workflow steps с tight loops не могут monopolize executor; critical-path tasks получают cache-hot execution через LIFO slot.

### Rayon: JEC (Jobs-Event-Counter)

Вместо wake-all-sleeping-threads при new work — increment counter. Sleeping threads проверяют counter перед commit to sleep. Sequential consistency fences prevent deadlocks. StackJob vs HeapJob: stack-allocate для bounded lifetime (cheap), heap для unbounded.

### Fjall: Backpressure с Watermarks

Writes throttled когда sealed memtables накапливаются (>4) или L0 runs exceed thresholds. Monitor thread proactively triggers rotation. Journal eviction watermark: logs удаляются только когда все зависимые данные flushed.

Для Nebula: throttle new workflow launches когда execution backlog растёт; audit log retention через eviction watermarks.

## Приложение B: Суммарная таблица Pain Points (top по reactions)

| Reactions | Issue | Суть |
|-----------|-------|------|
| +133 | tokio-rs/tokio#1879 | Structured concurrency |
| 108 comments | n8n-io/n8n#19359 | OOM при PDF extraction |
| 97 comments | apache/airflow#42136 | Task fails, logs inaccessible |
| +85 | vectordotdev/vector#1772 | Dead letter queue |
| 80 comments | n8n-io/n8n#14653 | Connection lost after upgrade |
| +76 | vectordotdev/vector#11942 | Memory awareness |
| 71 comments | apache/airflow#9975 | max_active_runs violated |
| 71 comments | apache/airflow#18041 | SIGTERM kills tasks |
| +70 | ray-project/ray#20609 | Rust API demand |
| +67 | vectordotdev/vector#10870 | Expand retry scope |
| +66 | rayon-rs/rayon#210 | Parallel → sequential bridge |
| 66 comments | apache/airflow#17507 | PID mismatch phantom tasks |
| +59 | tokio-rs/tokio#3150 | Task priority |
| +53 | vectordotdev/vector#19600 | Disk buffers lost at shutdown |
| 48 comments | apache/airflow#21867 | TaskGroup retry unsupported |
| 42 comments | n8n-io/n8n#13135 | Sub-workflow wrong data |
| +37 | ray-project/ray#51173 | GPU object store |
| 36 comments | dagster-io/dagster#20325 | Excessive PG connections |
| 31 comments | flyteorg/flyte#4505 | Silent int→float cast |
| +32 | vectordotdev/vector#11405 | Flush sinks at shutdown |
| +30 | petgraph/petgraph#551 | Crate redesign needed |
| +29 | salsa-rs/salsa#10 | Serialization to disk |
| +29 | rayon-rs/rayon#319 | NUMA-aware scheduling |
| 28 comments | apache/airflow#34339 | Successful tasks marked failed |
| 27 comments | temporalio/temporal#1507 | Priority task queues |
