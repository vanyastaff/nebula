# Workflow Engine Pain Points — GitHub Issues Deep Dive

**Date:** 2026-03-29
**Method:** DeepWiki codebase analysis + architectural documentation review across 15 repositories
**Note:** Reaction counts not available via current tooling; findings are ranked by severity and cross-project recurrence.

---

## 1. Raw Findings Per Repository

### 1.1 Temporal (`temporalio/temporal`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| arch | Workflow history grows unbounded; `Continue-As-New` is a workaround, not a solution | Critical | Open — fundamental to event-sourcing model |
| perf | Cold start latency: replaying entire history to reconstruct `MutableStateImpl` on new workers | High | Mitigated by sticky queues; no fundamental fix |
| dx | SDK determinism requirements painful — developers must avoid non-deterministic calls, random, time | High | Open — inherent to replay model |
| dx | Worker versioning complexity: 3 generations of APIs (V1 deprecated, V2, V3 Deployments) | High | Active development on V3 |
| ops | Requires Cassandra + Elasticsearch expertise for self-hosted production | Medium | SQLite for dev only |
| state | `MutableStateSizeLimitError` — in-memory state has hard size limits | Medium | Config-tunable |
| arch | Workflow Update feature: admitted updates lost if in-memory registry cleared (no history events) | Medium | By design |
| scale | Worker polling long-poll expiration and task queue partitioning need tuning | Medium | Config-tunable |
| perf | Blob/Memo size limits restrict data payloads in workflow events | Medium | Hard limits |

**Key insight:** Temporal's event-sourcing model creates an inherent tension between durability guarantees and operational complexity. History replay is both the strength (deterministic recovery) and the weakness (cold start cost, memory pressure, bounded history).

### 1.2 Apache Airflow (`apache/airflow`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| arch | All components directly connect to metadata DB — excessive connections, scaling nightmare | Critical | Fixed in Airflow 3.x (Task Execution API) |
| perf | DAG parsing overhead: top-level Python code re-executed on every parse cycle | Critical | Documented best practice; standalone DAG processor in 3.x |
| type | XCom limited to small metadata — forces workaround via external storage (S3 paths) | High | Object storage XCom backend in 3.x |
| perf | Scheduler hangs without trace; heartbeat timeouts | High | Health check mitigations |
| scale | Task instance table grows unbounded — query performance degrades | High | `airflow db clean` manual maintenance |
| dx | DAG serialization overhead with many DAGs — impacts scheduler and webserver startup | Medium | `min_serialized_dag_update_interval` tuning |
| dx | Dynamic task mapping limitations — `TriggerRule.ALWAYS` incompatible, lazy proxy confusion | Medium | Partially fixed in 3.x |
| arch | Airflow 2 to 3 migration is painful — breaking changes to DB access, task SDK, XCom | Medium | Migration tooling via ruff rules |

**Key insight:** Airflow 2.x's fundamental flaw was that every component talked directly to the metadata database. Airflow 3.x's Task Execution API is a major architectural rewrite acknowledging this was unsustainable.

### 1.3 n8n (`n8n-io/n8n`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| perf | Memory overload with large files/batch processing — OOM on document workflows | Critical | Mitigated by filesystem mode, batching, sub-workflows |
| scale | Single-instance architecture; Queue Mode (Bull/Redis) needed for scaling but has issues | High | Multi-main support in progress |
| dx | Error handling: no "Continue On Fail" by default crashes monitoring workflows | High | Config option exists but not default |
| arch | Monolithic workflows hard to maintain/debug — sub-workflow support is recent | Medium | Sub-workflow extraction feature added |
| perf | Lazy proxy for expression evaluation: suboptimal for iterating arrays/large objects | Medium | Known limitation |
| arch | Circular references in expression data can cause infinite loops (no cycle detection) | Medium | Open |
| dx | Item linking errors: expressions using `.item` fail when n8n can't determine matching | Medium | Ongoing fixes |
| scale | Leader election reconciliation issues in multi-main setup | Medium | Active fixes |

**Key insight:** n8n's biggest pain is memory management. Processing files/documents in a Node.js single-process model hits memory limits quickly, and the workarounds (batching, sub-workflows, filesystem mode) are all band-aids.

### 1.4 Dagster (`dagster-io/dagster`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| perf | Asset graph UI slow with thousands of assets — timeouts, excessive data fetching | High | Fixed: tight-tree algorithm, viewport-aware loading, disk caching |
| perf | Auto-materialize daemon slow with large asset graphs and partition histories | High | Significant performance improvements shipped |
| perf | Backfills with thousands of partitions: DB write bottleneck | Medium | Batched event inserts added |
| dx | `AssetExecutionContext` vs `OpExecutionContext` — breaking type alias changes | Medium | Stabilized |
| dx | String type annotations (`from __future__ import annotations`) break Dagster definitions | Medium | Fixed |
| scale | Global search slow for large workspaces | Medium | Performance improvements shipped |
| arch | Asset lineage graph navigation hard for large graphs | Medium | Horizontal layout, collapsible groups added |

**Key insight:** Dagster's pain points are mostly performance-related and have been actively addressed. The asset-centric model itself is sound, but the UI and daemon performance with large graphs was a significant gap that required multiple rounds of optimization.

### 1.5 Prefect (`prefecthq/prefect`)

*DeepWiki index not available. Known issues from community knowledge:*

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| arch | Prefect 1 to Prefect 2 migration broke everything — complete API rewrite | Critical | Historical |
| dx | Deployment model confusion: multiple ways to run (local, agent, worker, push) | High | Ongoing simplification |
| arch | Server dependency for all flow runs — no offline/local-first mode | Medium | Self-hosted Prefect server available |
| state | State management complexity: task states, flow states, transitions | Medium | Open |

### 1.6 Flyte (`flyteorg/flyte`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| arch | CRD size limited by etcd — large static workflows hit hard limits on read/write | Critical | Mitigated by offloading to object storage |
| dx | Type system rigidity: custom type transformers required for unsupported types (`bytes`, `complex`) | High | Open — inherent to design |
| perf | FlytePropeller worker pool limits + K8s API rate-limiting degrade large fan-out | High | Sharding available; `max-parallelism` config |
| dx | Container image management friction — ImagePullBackOff troubleshooting | Medium | `ImageSpec` improvements |
| perf | Data catalog overhead: cache eviction required manual DB intervention | Medium | Cache eviction feature added |
| perf | WorkQueue depth growing = propeller can't keep up | Medium | Tunable worker counts |
| arch | Cache-by-value semantics for non-Flyte objects: hash computation challenges | Medium | RFC in progress |

**Key insight:** Flyte's protobuf type system provides strong guarantees but creates friction when data types don't map cleanly. The K8s-native architecture inherits all K8s scaling limitations (etcd size, API rate limiting).

### 1.7 Windmill (`windmill-labs/windmill`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| perf | Cold start ~50ms per job (poll + start + result report) | Medium | Dependency caching mitigations |
| scale | PostgreSQL job queue: `SELECT FOR UPDATE SKIP LOCKED` under high load | Medium | Index optimizations |
| perf | Lock contention on concurrency counters | Medium | Fixed |
| arch | nsjail isolation overhead; requires `privileged: true` for PID namespace | Medium | Can be disabled |
| type | Results >2MB must go to S3 — hard limit on DB result storage | Medium | S3 offloading |

**Key insight:** Windmill is impressively fast (~50ms overhead) but fundamentally limited by PostgreSQL as its job queue. The team acknowledges considering Kafka/Redis as alternatives.

### 1.8 Kestra (`kestra-io/kestra`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| dx | YAML-only workflow definition — no programmatic control for complex logic | High | By design |
| arch | Plugin dependency conflicts: Jackson/Protobuf version clashes despite BOM | High | Manual "ugly hack" overrides |
| scale | JDBC queue polling overhead under high throughput; in-memory queue not production-ready | Medium | Kafka available for scale |
| dx | Deprecated "Global Task Defaults" → "Global Plugin Defaults" migration confusion | Medium | Migration path documented |

**Key insight:** Kestra's YAML-only approach is a deliberate choice for simplicity but limits expressiveness. Plugin dependency management via Gradle BOM requires manual intervention for version conflicts.

### 1.9 Restate (`restatedev/restate`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| perf | `invocation_status` protobuf deserialization bottleneck — full scan + full deser per row | Critical | Lazy deser added; FlatBuffers considered |
| perf | Journal replay previously materialized entire journal + state into memory | High | Fixed: lazy RocksDB reads on demand |
| arch | No secondary indices in partition stores — full scans for common queries | High | Future improvement planned |
| perf | Remote scanner: one batch at a time, no pipelining | Medium | Future improvement |
| perf | Aggregations send all rows to admin node — no local per-partition aggregation | Medium | Complex architectural change needed |
| state | RocksDB memory management: leadership thrashing caused by blocking async runtime | Medium | Fixed: background thread rebalancing |
| perf | Journal entry size limit: 32 MiB default, oversized entries cause indefinite retries | Medium | Size limits enforced |

**Key insight:** Restate's biggest pain is query performance. The lack of secondary indices and the protobuf deserialization cost make invocation listing slow. This is a fundamental storage architecture limitation they're actively addressing.

### 1.10 Hatchet (`hatchet-dev/hatchet`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| scale | PostgreSQL CPU bottleneck under high insert rates (>10k tasks/sec) | High | Bulk endpoints, write batching |
| perf | Scheduler lock contention — advisory locks with warnings on failure | High | `TryAdvisoryLock` reduces contention |
| perf | Internal message queue (RabbitMQ/Postgres) adds step start latency | Medium | `QOS` tuning |
| arch | Strong PostgreSQL dependency — no alternative result stores | Medium | By design |
| perf | `hatchet_queued_to_assigned_seconds` tracks queue-to-assignment latency | Medium | Monitoring available |

**Key insight:** Hatchet trades raw throughput for durability/observability. For >10k tasks/sec without retention needs, they explicitly recommend simpler task queues like BullMQ or Celery.

### 1.11 Argo Workflows (`argoproj/argo-workflows`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| arch | CRD size limit: CEL validations stripped, 200-template max, 1MB CRD limit | Critical | Server-side apply; minimization for old K8s |
| scale | Workflow controller cannot be horizontally scaled — single active controller | High | Hot-standby for HA only |
| scale | K8s API overwhelmed by pod creation at scale | High | Emissary executor, rate limiting, QPS tuning |
| scale | Database overload from large workflows: `DEFAULT_REQUEUE_TIME` must be tuned | Medium | Config-tunable |
| dx | DAG task constraints: names can't start with digits, `continueOn` incompatible with `depends` | Medium | By design |

**Key insight:** Argo inherits all Kubernetes scaling limitations. The single workflow controller and CRD size limits are fundamental architectural constraints of the K8s-native approach.

### 1.12 Netflix Conductor (`Netflix/conductor`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| perf | Task polling overhead: workers repeatedly poll `/tasks/poll` API | High | Configurable intervals, isolation groups |
| arch | Elasticsearch tightly coupled with persistence in v1.x | High | Decoupled in v2.x |
| scale | Concurrent `decide` operations on workflow cause inconsistent state | High | Distributed locking (Zookeeper) + fencing tokens |
| perf | Stale data updates from timed-out locks | Medium | Fencing tokens added |
| arch | Rate limiting only supported with Redis persistence module | Medium | Limitation |
| ops | 16GB+ memory required for Docker deployment with all components | Medium | Resource-intensive |

### 1.13 Inngest (`inngest/inngest`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| perf | Per-step HTTP round-trip: every step = full HTTP POST/response cycle to SDK | Critical | Inherent to serverless model |
| scale | Step limit: 1,000 default, 10,000 absolute max — `ErrFunctionOverflowed` | High | Hard limits |
| perf | Large payloads (>4MB) trigger `use_api` flag requiring extra HTTP GETs | High | Automatic but adds latency |
| dx | Non-determinism: function code changes mid-run cause step mismatch | Medium | Warning + recovery logic |
| perf | Cold start for serverless SDK endpoints adds to step latency | Medium | Inherent to serverless |

**Key insight:** Inngest's per-step HTTP overhead is its fundamental cost. Each step = network round-trip, serialization, deserialization. For workflows with hundreds of steps, this cumulative latency is significant.

### 1.14 Camunda/Zeebe (`camunda/camunda`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| arch | Data migrations from version upgrades are expensive for large runtime states | High | Team actively avoids migrations |
| perf | Job worker `maxJobsActive` capacity exhaustion causes timeout/reactivation | Medium | Scale workers or increase limits |
| scale | Message subscription relocation during dynamic partition scaling not implemented | Medium | Planned for future |
| arch | SBE protocol changes risk silent data corruption if not backward-compatible | Medium | Strict compatibility requirements |

### 1.15 DolphinScheduler (`apache/dolphinscheduler`)

| Category | Problem | Severity | Status |
|----------|---------|----------|--------|
| arch | ZooKeeper dependency for cluster coordination — network jitter causes node removal | High | Service stops on timeout |
| scale | Master overload with many DAG tasks — centralized scheduling pressure | High | Decentralized model, load protection |
| perf | Task status loss between Worker and Master | Medium | `RetryReportTaskStatusThread` mitigation |
| ops | Minimum 4 cores / 16GB RAM for single-machine deployment (5 services) | Medium | Resource-intensive |

---

## 2. Pain Clusters — Problems Appearing Across 3+ Projects

### Cluster 1: Database as Bottleneck (11 projects)
**The single most common pain point across all workflow engines.**

Every project that uses a relational database as its primary store eventually hits scaling limits:

| Project | Manifestation |
|---------|--------------|
| Temporal | Cassandra/SQL persistence for workflow history; history replay loads from DB |
| Airflow | Metadata DB overwhelmed by scheduler + workers + webserver connections |
| n8n | Not primary bottleneck but DB stores all executions |
| Dagster | Event log grows unbounded; backfill DB write bottleneck |
| Hatchet | PostgreSQL CPU bottleneck at >10k tasks/sec; lock contention |
| Windmill | PostgreSQL job queue with `SELECT FOR UPDATE SKIP LOCKED` |
| Kestra | JDBC queue polling overhead |
| Conductor | Persistence layer bottleneck; external payload storage needed |
| Restate | RocksDB protobuf deserialization; no secondary indices |
| DolphinScheduler | Task status loss between components |
| Argo | Database overload from large workflow reconciliation |

**Nebula relevance:** HIGH. Our execution engine must design storage access patterns that don't create a database hotspot. Consider: write-ahead log with async DB flush, partitioned storage, or tiered storage (hot state in memory, cold in DB).

### Cluster 2: Workflow History / State Size Limits (7 projects)
**Long-running workflows hit hard limits on state size.**

| Project | Manifestation |
|---------|--------------|
| Temporal | `HistorySizeLimitError`, `MutableStateSizeLimitError` — requires `Continue-As-New` |
| Argo | CRD size limit (1MB etcd), 200-template max per workflow |
| Flyte | etcd CRD size limits for `FlyteWorkflow` — offload to object storage |
| Restate | Journal entry size limit (32 MiB default) |
| Airflow | XCom limited to small data; task instance table unbounded |
| Inngest | Step count limits (1,000 default / 10,000 max) |
| Hatchet | Result persistence to PostgreSQL creates DB load at scale |

**Nebula relevance:** CRITICAL. Workflow state must be designed with bounded growth from day one. Options: automatic checkpointing with state compaction, tiered storage (recent in memory, older compressed/on disk), streaming state instead of accumulating history.

### Cluster 3: Cold Start / Replay Overhead (6 projects)
**Resuming or starting workflow execution has non-trivial latency.**

| Project | Manifestation |
|---------|--------------|
| Temporal | Full history replay to reconstruct `MutableStateImpl` on cold start |
| Restate | Journal replay materialized entire journal + state into memory (now lazy) |
| Inngest | Per-step HTTP round-trip + serverless cold start |
| Windmill | ~50ms per job (poll + start + result); dependency cache helps |
| Conductor | Task polling overhead — workers repeatedly poll API |
| Camunda | Job worker timeout/reactivation when capacity exhausted |

**Nebula relevance:** HIGH. Our execution model should avoid full state replay. Options: checkpoint-based recovery (only replay from last checkpoint), incremental state loading, pre-warmed worker pools.

### Cluster 4: Type System / Data Passing Pain (6 projects)
**Moving data between workflow steps is harder than it should be.**

| Project | Manifestation |
|---------|--------------|
| Airflow | XCom limited to small data; external storage workaround |
| Flyte | Protobuf type system requires custom transformers for unsupported types |
| Inngest | Large payloads (>4MB) require extra API calls; serialization overhead |
| Dagster | Type system changes broke type hints; `AssetExecutionContext` confusion |
| n8n | Item linking errors; lazy proxy confusion |
| Hatchet | Results >DB limit need external storage |

**Nebula relevance:** CRITICAL. The parameter/data system must handle arbitrary sizes without workarounds. Design: streaming data references for large payloads, zero-copy where possible, type system that's helpful not restrictive.

### Cluster 5: Worker / Task Polling Inefficiency (5 projects)
**Pull-based task distribution creates unnecessary overhead.**

| Project | Manifestation |
|---------|--------------|
| Temporal | Long-poll expiration, task queue partitioning complexity |
| Conductor | Workers repeatedly poll `/tasks/poll`; isolation groups needed |
| Camunda | Job worker polling; experimental job streaming to reduce overhead |
| Windmill | Workers poll PostgreSQL every 50ms |
| Hatchet | Internal message queue adds step start latency |

**Nebula relevance:** HIGH. Push-based task distribution (via EventBus) eliminates polling overhead. Consider: event-driven worker notification, backpressure signals, work stealing for load balancing.

### Cluster 6: Scheduling / Orchestration Bottleneck (5 projects)
**The scheduler/controller is a single point of contention.**

| Project | Manifestation |
|---------|--------------|
| Airflow | Scheduler hangs, heartbeat timeouts, single-scheduler bottleneck |
| Argo | Single workflow controller — cannot horizontally scale |
| DolphinScheduler | Master overload with many DAG tasks |
| Conductor | Concurrent `decide` operations cause inconsistent state |
| Dagster | Auto-materialize daemon slow with large asset graphs |

**Nebula relevance:** HIGH. The execution engine must be horizontally scalable from design. Avoid single-scheduler architecture. Consider: partitioned scheduling, sharded workflow ownership, distributed evaluation.

### Cluster 7: Operational Complexity (5 projects)
**Running these systems in production requires deep expertise.**

| Project | Manifestation |
|---------|--------------|
| Temporal | Cassandra + Elasticsearch cluster management |
| DolphinScheduler | ZooKeeper dependency; 5 services minimum |
| Conductor | 16GB+ RAM; Zookeeper for distributed locking |
| Airflow | Database tuning, scheduler health monitoring |
| Kestra | Plugin dependency conflicts; Kafka for production scale |

**Nebula relevance:** MEDIUM. Nebula should be simple to operate: single binary or minimal services, embedded storage option, no external coordination service required.

### Cluster 8: SDK / Developer Experience Pain (4 projects)
**The programming model is confusing or restrictive.**

| Project | Manifestation |
|---------|--------------|
| Temporal | Determinism requirements; 3 generations of versioning APIs |
| Inngest | Non-determinism detection; step mismatch on code changes |
| Kestra | YAML-only — no programmatic workflow definition |
| n8n | Error handling defaults are unsafe; expression system confusing |

**Nebula relevance:** HIGH. Nebula's Rust-native approach + action system should make the programming model straightforward. No replay-based determinism requirements. Actions are pure functions with DI.

---

## 3. Top 20 Pain Points Across All Repos (by severity and recurrence)

| Rank | Problem | Projects Affected | Category | Nebula Relevance |
|------|---------|-------------------|----------|-----------------|
| 1 | Database becomes bottleneck under load | 11 | scale | Design storage to avoid hotspots |
| 2 | Workflow state/history grows unbounded | 7 | state | Bounded state with compaction from day 1 |
| 3 | Cold start / replay overhead | 6 | perf | Checkpoint-based recovery, no full replay |
| 4 | Data passing between steps limited or painful | 6 | type | Streaming refs for large data, ergonomic types |
| 5 | Worker polling creates unnecessary overhead | 5 | perf | Push-based task distribution via EventBus |
| 6 | Single scheduler/controller is bottleneck | 5 | sched | Horizontally scalable execution engine |
| 7 | Operational complexity for self-hosting | 5 | ops | Single binary, embedded storage option |
| 8 | SDK determinism / versioning complexity | 4 | dx | No replay model = no determinism burden |
| 9 | CRD / etcd size limits (K8s-native engines) | 3 | arch | Not K8s-dependent |
| 10 | Memory leaks / OOM with large workflows | 3 | perf | Rust memory safety, bounded allocations |
| 11 | Plugin / dependency version conflicts | 3 | arch | Cargo workspace, `deny.toml` enforcement |
| 12 | Error handling defaults are unsafe | 3 | dx | Error-first design, explicit error handling |
| 13 | DAG parsing / definition overhead | 3 | perf | Compiled Rust DAGs, no parsing at runtime |
| 14 | Multi-instance / HA complexity | 3 | scale | Designed for distributed from start |
| 15 | Migration between major versions is painful | 3 | dx | Stable public API with deprecation cycle |
| 16 | Queue / task scheduling latency | 3 | perf | Lock-free queue design |
| 17 | Result/output size limits force external storage | 3 | type | Tiered result storage built-in |
| 18 | Protobuf / serialization deserialization cost | 2 | perf | Efficient binary serialization (no protobuf overhead) |
| 19 | Invocation/execution listing is slow | 2 | perf | Indexed execution store |
| 20 | Container build / image management friction | 2 | dx | Not container-per-step (in-process sandbox) |

---

## 4. Unsolved Problems — Fundamental, Not Fixable by Patch

These represent architectural limitations that cannot be resolved without fundamental redesign:

### 4.1 Event-Sourcing History Growth (Temporal, Restate)
**Problem:** Event-sourced state requires replaying the full history to reconstruct current state. History grows linearly with workflow activity. `Continue-As-New` is a workaround that fragments the logical workflow.
**Why unsolvable:** Removing event sourcing removes the durability guarantee. The tension between "complete audit trail" and "bounded state" is inherent.
**Nebula opportunity:** Checkpoint-based recovery with optional audit log. State is the current snapshot, not the sum of all events.

### 4.2 K8s-Native Scaling Limits (Argo, Flyte)
**Problem:** etcd limits CRD size to ~1MB. K8s API rate limiting constrains pod creation throughput. Single workflow controller cannot scale horizontally.
**Why unsolvable:** These are Kubernetes platform limits, not application bugs.
**Nebula opportunity:** Not K8s-dependent. Can run on K8s but doesn't require it.

### 4.3 Relational DB as Queue (Airflow, Hatchet, Windmill, Kestra)
**Problem:** Using PostgreSQL/MySQL for job queuing via `SELECT FOR UPDATE SKIP LOCKED` creates lock contention, polling overhead, and scaling ceilings.
**Why unsolvable:** Relational databases are not designed for high-throughput message queuing. Adding Kafka/Redis adds operational complexity.
**Nebula opportunity:** Embedded queue (e.g., crossbeam channels for in-process, or dedicated queue abstraction) that doesn't require external infrastructure.

### 4.4 Per-Step Network Overhead (Inngest)
**Problem:** Every step = HTTP POST to SDK + response. Serialization + network latency + potential cold start per step.
**Why unsolvable:** Serverless execution model requires network communication between orchestrator and execution environment.
**Nebula opportunity:** In-process action execution. Steps are function calls, not HTTP requests.

### 4.5 Determinism Requirements (Temporal, Inngest)
**Problem:** Workflow code must be deterministic for replay. Developers must avoid `rand()`, `time()`, I/O, etc. This is a significant mental model burden.
**Why unsolvable:** Replay-based durability fundamentally requires determinism.
**Nebula opportunity:** No replay model. Actions are independent, retriable units. State is persisted explicitly, not derived from replay.

### 4.6 YAML Configuration Ceiling (Kestra, Airflow)
**Problem:** YAML-based workflow definition hits complexity ceiling for large workflows with conditional logic, loops, or dynamic behavior.
**Why unsolvable:** YAML is a data format, not a programming language.
**Nebula opportunity:** Workflows defined in Rust code with full type safety and IDE support.

---

## 5. Strategic Implications for Nebula

Based on this analysis, Nebula's architecture should prioritize:

1. **Checkpoint-based recovery** instead of event-sourcing replay — avoids history growth and cold start problems
2. **Push-based task distribution** via EventBus — eliminates polling overhead
3. **Embedded queue** with optional external queue adapter — no PostgreSQL-as-queue anti-pattern
4. **In-process action execution** — no per-step network overhead
5. **Bounded state with compaction** — automatic state management, no `Continue-As-New` workaround
6. **Streaming data references** for large payloads — no XCom-style size limits
7. **Single binary deployment** option — minimal operational complexity
8. **No determinism requirement** — actions are independent, retriable functions
9. **Horizontally scalable execution** from day 1 — no single scheduler bottleneck
10. **Compiled workflow definitions** — no DAG parsing overhead at runtime
