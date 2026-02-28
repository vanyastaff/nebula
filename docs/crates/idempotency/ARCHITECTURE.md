# Architecture

## Problem Statement

- **Business problem:** Workflow retries and duplicate requests must not cause duplicate side effects (e.g. double charge, double email). Need exactly-once or at-least-once with deduplication.
- **Technical problem:** Provide idempotency key generation, storage, and integration at node-attempt, workflow, and HTTP request levels.

## Current Architecture

### Module Map (Current — In nebula-execution)

| Location | Responsibility |
|----------|----------------|
| `nebula-execution::idempotency` | `IdempotencyKey`, `IdempotencyManager` |
| `nebula-execution::attempt` | `NodeAttempt` with `idempotency_key` |
| `nebula-execution::error` | `DuplicateIdempotencyKey` |
| Migrations | `idempotency_keys` table; `executions.idempotency_key` |
| `nebula-resilience` | `Operation::IDEMPOTENT` for retry policy |
| `nebula-credential` | `create_credential_idempotent` (user key) |

### Data/Control Flow

1. **Key generation:** `IdempotencyKey::generate(execution_id, node_id, attempt)` → `"{exec_id}:{node_id}:{attempt}"`
2. **Deduplication:** `IdempotencyManager::check_and_mark(key)` → `true` if new, `false` if duplicate
3. **Node attempt:** Each `NodeAttempt` has `idempotency_key`; used for journaling and retry detection

### Known Bottlenecks

- **In-memory only:** IdempotencyManager uses HashSet; lost on restart; single-process
- **No persistent storage:** idempotency_keys table exists but no code uses it yet
- **No HTTP layer:** No Idempotency-Key header handling
- **No action-level:** No IdempotentAction trait or executor wrapper

## Target Architecture

### Target Module Map (Planned)

```
nebula-idempotency/ (future crate)
├── core/           — IdempotencyKey, IdempotencyConfig, error
├── action/         — IdempotentAction trait, executor
├── workflow/       — Checkpointing, resume
├── request/        — HTTP IdempotencyLayer
├── storage/        — Memory, Postgres, Redis backends
└── integration/    — Action, credential, context
```

### Public Contract Boundaries

- `IdempotencyKey` format: `{execution_id}:{node_id}:{attempt}` for node-level; user keys for request-level
- `IdempotencyStorage` trait for backends
- `IdempotentAction` trait for action composition

### Internal Invariants

- Key generation deterministic for same inputs
- check-and-mark atomic; duplicate returns cached result or error

## Design Reasoning

### Key Trade-off 1: In-memory vs persistent

- **Current:** In-memory (HashSet); simple, no I/O
- **Target:** Persistent storage for production; in-memory for dev/test
- **Consequence:** Storage backend abstraction; migration from execution module

### Key Trade-off 2: Node-level vs multi-level

- **Current:** Node-attempt level only (execution_id:node_id:attempt)
- **Target:** Action, Workflow, Request, Transaction levels
- **Consequence:** Different key strategies per level

### Rejected Alternatives

- **No idempotency:** Unacceptable for financial/critical workflows
- **Client-only keys:** Need server-generated keys for node retries

## Comparative Analysis

Sources: n8n, Node-RED, Temporal, Stripe, AWS.

| Pattern | Verdict | Rationale |
|---------|---------|------------|
| Idempotency-Key header | **Adopt** | Stripe standard; HTTP API dedup |
| Content-based keys | **Adopt** | Automatic dedup for actions |
| Checkpoint/resume | **Adopt** | Temporal, Prefect; workflow recovery |
| 24h dedup window | **Adopt** | Stripe; balances safety vs storage |
| Distributed locks | **Defer** | Phase 2; single-node first |

## Breaking Changes (if any)

- None until idempotency crate extracted; execution module types may move.

## Open Questions

- Q1: Extract to nebula-idempotency or keep in execution?
- Q2: Key format for workflow-level (execution_id only?) vs request-level (user-provided)?
