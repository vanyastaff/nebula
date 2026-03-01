# nebula-worker Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

In a distributed deployment, workflow tasks are queued (e.g. by API or trigger). Workers pull tasks, execute them in isolation (via runtime/sandbox), and report results. Worker lifecycle, heartbeats, backpressure, and graceful shutdown keep the fleet stable and observable.

**nebula-worker is the distributed worker runtime for task acquisition, isolated node execution, and result reporting.**

It answers: *How do workers discover and claim tasks from a queue, run them with bounded concurrency and isolation, and report success or failure without losing or duplicating work?*

```
Queue (broker) holds tasks
    ↓
Worker pulls task (claim/ack protocol); heartbeat keeps lease
    ↓
Runtime + sandbox execute node; resilience and resource used
    ↓
Worker reports result (success/fail); task state updated
    ↓
On shutdown: drain in-flight, hand off lease or requeue
```

This is the worker contract: deterministic task state machine; bounded concurrency; graceful drain; no business logic in worker beyond task execution orchestration.

---

## User Stories

### Story 1 — Worker Claims and Runs Task (P1)

Worker starts, connects to queue broker, pulls next task (or blocks). It claims the task (so other workers do not run it), runs it via runtime/sandbox, then acks success or nacks failure. Task state is queued → claimed → running → succeeded/failed.

**Acceptance**:
- Task state machine documented and implemented
- Claim/ack protocol with timeout or lease; heartbeat to extend lease
- Single execution per task (no duplicate run on ack failure when idempotency is applied)

### Story 2 — Bounded Concurrency and Backpressure (P1)

Worker has max concurrent tasks (e.g. N). When N tasks are in flight, worker does not pull more until one completes. Queue backpressure (e.g. reject or delay) when all workers are busy.

**Acceptance**:
- Configurable concurrency limit
- Pull only when under limit; backpressure visible to queue or API
- No unbounded task accumulation in worker memory

### Story 3 — Graceful Shutdown and Drain (P2)

On SIGTERM or shutdown signal, worker stops pulling new tasks, waits for in-flight tasks to complete (with timeout), then releases leases or requeues unacked tasks. No silent task loss.

**Acceptance**:
- Shutdown triggers drain mode
- In-flight tasks get best-effort completion window
- Lease handoff or requeue so task is not lost
- Document timeout and behavior when timeout hits

### Story 4 — Observability and Health (P2)

Operator needs worker health (alive, busy, last heartbeat) and task metrics (claimed, completed, failed, latency). Worker exposes health check and optional metrics.

**Acceptance**:
- Health endpoint or callback: alive, current load
- Metrics: tasks_claimed, tasks_completed, tasks_failed, task_duration
- Structured logs for claim, start, complete, fail
- Integration with telemetry/metrics crate optional

---

## Core Principles

### I. Worker Does Not Own Workflow Graph or Scheduling

**Worker runs tasks that are already scheduled and queued. Engine or API owns workflow DAG and scheduling.**

**Rationale**: Separation of concerns. Worker is execution capacity; engine/API are decision makers.

**Rules**:
- No workflow or DAG types in worker public API for scheduling
- Task payload is opaque or minimal (execution_id, node_id, input); worker passes to runtime
- Queue protocol is abstraction (trait or adapter) so broker can be swapped

### II. Deterministic Task State Machine

**Task states (queued, claimed, running, succeeded, failed) are well-defined. Transitions are deterministic and observable.**

**Rationale**: Prevents duplicate execution and lost tasks. Operators can reason about state.

**Rules**:
- State machine documented; only defined transitions allowed
- Claim/ack/requeue semantics documented (at-most-once vs at-least-once and how idempotency is applied)
- Idempotency rules fixed before implementation (see RELIABILITY)

### III. Isolation and Policy Per Task

**Each task runs with resource isolation and policy (timeout, memory, sandbox) so that one task cannot starve or crash others.**

**Rationale**: Multi-tenant or untrusted tasks require isolation. Policy enforcement is worker's responsibility in concert with runtime/sandbox.

**Rules**:
- Timeout and resource limits per task
- Sandbox or isolation level per task type (trusted vs sandboxed)
- Policy configurable; document defaults

### IV. No Business Logic in Worker

**Worker orchestrates task pull, run, and report. It does not implement actions, workflow logic, or queue broker internals.**

**Rationale**: Actions and workflow are in action/engine; queue is external or adapter. Worker is the glue.

**Rules**:
- Worker calls runtime to execute; runtime calls action
- Queue is trait or adapter; worker does not implement broker protocol
- Credential and resource access are via runtime context

### V. Graceful Drain on Shutdown

**Shutdown must drain in-flight tasks and release or requeue so that no task is lost and no lease is left dangling.**

**Rationale**: Rolling deploys and scaling down require clean shutdown. Lost tasks or stuck leases cause duplicate or missing work.

**Rules**:
- Drain mode: stop pull, wait for in-flight (with timeout)
- Requeue or lease handoff for unacked tasks
- Document timeout and edge cases

---

## Production Vision

### The worker in an n8n-class fleet

In production, a fleet of workers connects to a queue (Redis, SQS, Kafka, or custom). Each worker pulls tasks, claims them, runs via runtime and sandbox, and acks or nacks. Concurrency is bounded; backpressure is applied when all workers are busy. On shutdown, workers drain and hand off leases. Metrics and health are exposed for autoscaling and alerting.

```
Worker
    ├── Queue adapter: pull, claim, ack, nack, heartbeat
    ├── Concurrency limiter: max N in-flight
    ├── Runtime: execute_task(payload) → runtime.execute_action(...)
    ├── Shutdown: drain + lease handoff / requeue
    └── Health + metrics: alive, busy, tasks_claimed/completed/failed
```

Task state: queued → claimed → running → succeeded | failed. Idempotency (e.g. nebula-idempotency) ensures at-most-once execution when ack fails after success.

### From the archives: pool, scaling, and reliability

The archive `_archive/archive-nebula-complete.md` and worker docs describe pool, scaling, progress/health, graceful shutdown. Production vision: deterministic task state machine, bounded concurrency, queue backpressure, graceful drain, hard resource isolation, first-class observability. Wire and trait contracts versioned; breaking execution semantics only in major with migration.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Executable contract (trait, queue adapter) | Critical | crates/worker not implemented yet; fix interface with runtime/queue/sandbox |
| Idempotency and at-most-once semantics | High | Integrate with nebula-idempotency; document ack/requeue |
| Queue broker adapters (Redis, SQS, etc.) | High | At least one production adapter |
| Health and metrics API | Medium | For autoscaling and dashboard |
| Lease timeout and heartbeat protocol | High | Prevent duplicate run when worker dies mid-task |

---

## Key Decisions

### D-001: Worker Depends on Runtime, Not Engine

**Decision**: Worker calls runtime to execute a task; it does not call engine for scheduling.

**Rationale**: Runtime is the execution boundary; engine is the scheduler. Worker is another consumer of runtime.

**Rejected**: Worker holding engine reference — would complicate deployment (worker and engine often separate processes).

### D-002: Queue as Trait or Adapter

**Decision**: Worker depends on a queue abstraction (pull, claim, ack, nack); concrete broker (Redis, SQS) is adapter.

**Rationale**: Allows different backends and testing with in-memory queue.

**Rejected**: Worker depending on single broker crate — would lock deployment to one broker.

### D-003: Task Payload Opaque to Worker

**Decision**: Worker receives task payload (execution_id, node_id, input ref, etc.); it does not interpret workflow graph.

**Rationale**: Worker does not need workflow structure; runtime and engine own that. Payload is enough to run one node.

**Rejected**: Worker parsing full workflow — would duplicate engine logic.

### D-004: Graceful Shutdown With Timeout

**Decision**: Drain waits for in-flight tasks up to a configurable timeout; then force-release or requeue.

**Rationale**: Prevents indefinite hang on shutdown while giving tasks a chance to complete.

**Rejected**: No timeout — could hang forever. No drain — could lose tasks.

---

## Open Proposals

### P-001: Queue Adapter Interface

**Problem**: No executable contract yet; risk of drift with runtime and queue.

**Proposal**: Define QueueAdapter trait (pull, claim, ack, nack, heartbeat) and Task type; implement in-memory and one production adapter.

**Impact**: Establishes worker contract; runtime and queue crates must align.

### P-002: Idempotency Integration

**Problem**: At-least-once delivery can cause duplicate execution.

**Proposal**: Worker or runtime checks idempotency key (execution_id + node_id); skip or dedupe when already completed. Document in RELIABILITY.

**Impact**: Requires nebula-idempotency and storage; design before implementation.

---

## Non-Negotiables

1. **Deterministic task state machine** — queued → claimed → running → succeeded/failed; documented.
2. **Bounded concurrency** — no unbounded pull; backpressure when at limit.
3. **Graceful drain on shutdown** — stop pull, wait in-flight (with timeout), release/requeue.
4. **No workflow scheduling in worker** — worker runs tasks; engine/API schedule and enqueue.
5. **Isolation and policy per task** — timeout, sandbox, resource limits.
6. **Breaking task or queue contract = major + MIGRATION.md** — queue and runtime depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No change to task state machine or queue contract.
- **MINOR**: Additive (new metrics, new config). No removal.
- **MAJOR**: Breaking changes to task semantics or queue adapter. Requires MIGRATION.md.
