---
name: nebula-execution
role: Execution State Machine + Journal + Local Replay Identity
status: stable
last-reviewed: 2026-04-17
canon-invariants: [L2-11.1, L2-11.2, L2-11.3, L2-11.5, L2-12.2]
related: [nebula-storage, nebula-engine, nebula-workflow, nebula-resilience, nebula-core, nebula-error]
---

# nebula-execution

## Purpose

A durable workflow engine needs an authoritative model of what a run *is*: its state machine,
its append-only event history, its pre-computed parallel schedule, and the key used for local
attempt replay/dedup. Without a shared model, the engine orchestrator and
the storage layer each invent their own state representation, producing the "two truths"
anti-pattern canon §14 forbids. `nebula-execution` is that shared model. It defines the
8-state `ExecutionStatus` machine with validated transitions, the `JournalEntry` type that
backs the durable `execution_journal` table, the `IdempotencyKey` shape, and the
`ExecutionPlan` that the engine derives from the workflow DAG. It deliberately does not own a
repository interface — persistence is `nebula-storage::ExecutionRepo`'s job.

## Role

**Execution State Machine + Journal + Local Replay Identity.**

Patterns:
- *Write-Ahead Log* (DDIA ch 3, 11) — `JournalEntry` backs the `execution_journal`
  append-only durable timeline.
- *Local replay/dedup identity* — `IdempotencyKey` shape
  `{execution_id}:{node_id}:{attempt}` is deterministic for one attempt. It is not a remote
  operation identity and does not make a provider effect atomic with Nebula persistence.
- *Optimistic Concurrency Control* (DDIA ch 7) — `ExecutionStatus` transitions are guarded
  by CAS on `version` in `nebula-storage::ExecutionRepo::transition`.

## Public API

- `ExecutionStatus` — 8-state execution state machine: `Pending`, `Running`, `Paused`,
  `Completed`, `Failed`, `Cancelled`, `Cancelling`, `TimedOut`. Transitions validated by
  the `transition` module.
- `ExecutionState`, `NodeExecutionState` — persistent state tracking per execution and per
  node; serialized into the `executions` table row.
- `ExecutionRevisions` — default-public aggregate that pins the canonical `WorkflowVersionId` and
  exact `WorkerFlavorRevisionId` together.
- `ExecutionProfile` — non-exhaustive execution model selector. Graph-v1 records `"graph"`.
- `ExecutionContractBundle` — immutable Graph-v1 semantic contract with private fields, exact
  plan/plugin/workflow/flavor/credential pins, internally stamped protocol versions, and a
  structural SHA-256 fingerprint. `ExecutionContractBundleId` is a random record identity and is
  intentionally excluded from that fingerprint.
- `RecordedExecutionContractBundleV1` and `ExecutionContractBundleIntegrityError` — untrusted
  durable/wire input plus typed validation failures for unsupported versions/profile,
  noncanonical credentials, unknown fields, and forged fingerprints.
- `ExecutionPlan` — pre-computed parallel execution schedule derived from `DependencyGraph`.
  Feeds the engine scheduler.
- `ReplayPlan` — resume plan for restarting from a checkpoint.
- `ExecutionContext` — lightweight runtime context: `execution_id`, `ExecutionBudget`, optional
  `W3cTraceContext` for M3.5 distributed trace propagation across async boundaries.
- `ExecutionResult` — post-execution summary: status, timing, node counts, outputs.
- `JournalEntry` — audit log entry type. Each entry is appended to the durable
  `execution_journal` table via `ExecutionRepo::append_journal`.
- `NodeOutput`, `ExecutionOutput` — node output data with metadata.
- `NodeAttempt` — individual attempt tracking (attempt number, started/finished timestamps,
  node status). Used as the shape of attempt-keyed output rows by
  `nebula-storage::ExecutionRepo::save_node_output`.
- `IdempotencyKey` — deterministic local replay key
  `{execution_id}:{node_id}:{attempt}`. The local check-and-mark enforcement lives in storage;
  the key changes across retries and is distinct from the future stable remote `OperationId`.
- `ExecutionError` — typed error for state machine violations and execution failures.

## Contract

- **[L2-§11.1]** `nebula-execution` defines the state machine; `ExecutionRepo` in
  `nebula-storage` is the **single source of truth** for persisted execution state.
  Transitions use optimistic CAS on `version`. No handler may mutate execution state
  except through `ExecutionRepo::transition`. Seam: `crates/storage/src/execution_repo.rs`.
  The `transition` module in this crate validates state-machine legality; storage enforces
  persistence and CAS.

- **[L2-§11.2]** Retry scheduling is **out of scope for this crate**. This crate defines
  the persisted shapes the engine uses for operator-declared retry: legal
  `Failed → WaitingRetry → Ready` node transitions, `next_attempt_at`, `total_retries`,
  `ExecutionBudget.max_total_retries`, `NodeAttempt`, and the idempotency-key shape
  `{execution_id}:{node_id}:{attempt}`. The engine owns the retry decision and
  re-dispatch for `NodeDefinition.retry_policy` / `WorkflowConfig.retry_policy`.
  `nebula-resilience` remains the in-action outbound-call retry surface, and
  result-driven `ActionResult::Retry` is not a current public capability.

- **[L2-§11.3]** `IdempotencyKey` shape is `{execution_id}:{node_id}:{attempt}`. Seam:
  `crates/execution/src/idempotency.rs`. Storage's `check_and_mark` is a local replay/dedup
  oracle only; it is not atomic with a remote provider and cannot determine whether an effect
  committed. Current remote invocation semantics are at-least-once with a possible duplicate.
  The planned effect protocol binds each intended occurrence to a storage-minted
  `EffectSlotId` and a separate runtime-minted `OperationId`, durably prepared before invocation
  and retained across retries. Same-slot fingerprint mismatch is `OperationMismatch` with no
  durable delta; distinct slots remain distinct even for identical payloads. Ambiguous provider
  acceptance permits bounded effect-call re-invocation only for the same `Prepared` operation and
  `OperationId` under a still-valid pinned stable-key contract. Reconciliation is read-only and
  never repeats the effecting call. Exhaustion or expiry becomes `OutcomeUnknown`, after which no
  effecting call may repeat. `AcknowledgementUnknown` applies separately to prepare and outcome
  database commits: prepare uncertainty forbids provider invocation until database-only
  reconciliation confirms the exact durable prepared record and ID; outcome uncertainty permits
  only ledger reads and exact frozen-evidence recommit.

- **[L2-§11.5]** `JournalEntry` type backs the durable `execution_journal` (append-only,
  replayable). Seam: `crates/storage/src/execution_repo.rs` — `ExecutionRepo::append_journal`.
  Checkpoint state is best-effort: a checkpoint write failure logs and does not abort; work
  since the last successful checkpoint may be replayed or lost.

- **[L2-§12.2]** `ExecutionStatus` machine defines what states exist and what transitions
  are legal. The `transition` module enforces legality. Persistence and CAS are in
  `nebula-storage`. No handler invents a parallel lifecycle.

- **Immutable execution contract.** Bundle reconstruction validates canonical wire structure and
  the claimed semantic fingerprint only. It does not prove existence, retention, tenant
  authority, compatibility, or admission, and it must never fall back to a latest revision.
  `PluginSetId` is an independent plugin-set pin; the ID alone does not prove schemas, runtime
  behavior, artifact authenticity, authorization, or a complete frozen registry.

- **Credential closure.** The exact loaded executable plan must carry
  slot-to-selected-`CredentialId` mappings and the corresponding credential contract revisions.
  Admission must compare the plan's unique selected credential IDs exactly with the bundle's
  sorted, deduplicated set. If the plan contains only abstract credential requirements, the v1
  bundle shape is insufficient to establish this closure.

## Non-goals

- Not the engine orchestrator — see `nebula-engine` (drives these types).
- Not the storage implementation — see `nebula-storage` (`ExecutionRepo`, `executions`
  table, `execution_journal`, `execution_control_queue`). The `ExecutionControlQueue`
  (durable outbox for cancel/dispatch signals) and the `Transactional Outbox` pattern live
  in `nebula-storage`, not here.
- Not a retry scheduler — this crate records the state shapes; `nebula-engine` drives
  operator-declared retry, while `nebula-resilience` covers in-action outbound calls
  (§11.2).
- Not a resource lifecycle manager — see `nebula-resource` for `ReleaseQueue` / `Bulkhead`.

## Maturity

See `docs/MATURITY.md` row for `nebula-execution`.

- Existing state-machine, journal, idempotency-key, and plan types are stable for workspace
  consumers and are in active use by `nebula-engine` and `nebula-storage`. This pre-1.0 internal
  crate does not itself provide the supported Rust product surface: direct implementation-crate
  use is unsupported, and the curated `nebula-sdk` façade is the supported future surface. The
  removal of the former opt-in feature names is therefore an intentional breaking cleanup for
  direct users.
- Revision, frozen-flavor, and bundle primitives are default-public but remain `partial`: they
  define a closed immutable epoch with zero production consumer until compiler, admission,
  persisted routing, and exact-flavor dispatch consume the contract end to end.
- Layer 1 lease enforcement (`lease_holder`/`lease_expires_at`) shipped via M2.2 — heartbeat-driven via `acquire_and_heartbeat_lease` (see `DEFAULT_EXECUTION_LEASE_TTL` / `DEFAULT_EXECUTION_LEASE_HEARTBEAT_INTERVAL`), verified by `crates/engine/tests/lease_takeover.rs`, `crates/storage/tests/execution_lease_pg_integration.rs`, and the loom probe at `crates/storage-loom-probe/src/lease_handoff.rs`. Layer 2 (`claimed_by`/`claimed_until` from `migrations/postgres/0011_executions.sql`) remains Sprint E (1.1) scaffolding — see the durability matrix below.
- Integration tests include the Graph-v1 bundle wire/fingerprint contract; state machine and plan
  coverage also comes from unit tests and engine-level integration tests.
- 5 `panic!` sites in `transition` and `status` modules serve as state-machine invariant
  guards; these are technical debt (candidates for `#[must_use]` or typed errors).

## Related

- Canon: `docs/PRODUCT_CANON.md` §11.1, §11.2, §11.3, §11.5, §12.2.
- Siblings: `nebula-storage` (persists via `ExecutionRepo`), `nebula-engine` (drives),
  `nebula-workflow` (DAG → `ExecutionPlan`), `nebula-resilience` (in-action retry).

## Appendix

### Idempotency key format (L4 detail, evicted from PRODUCT_CANON.md §11.3)

The deterministic key shape is `{execution_id}:{node_id}:{attempt}`, persisted in
`idempotency_keys`. The format string is an implementation detail (L4) — changing it
requires updating this README and the corresponding code; no canon revision. Its contract is
limited to deterministic, tenant-scoped local replay/dedup. It is not the stable remote
effect-slot/operation identity described by canon §11.3 and cannot establish effectively-once
behavior by itself.

### Persistence durability matrix (reference from §11.5)

| Artifact | Status | Notes |
|---|---|---|
| `executions` row + state JSON | **Durable** (CAS via `ExecutionRepo`) | Source of truth |
| `execution_journal` | **Durable** (append-only) | Replayable history |
| `execution_control_queue` | **Durable** (outbox) | At-least-once cancel/dispatch |
| `stateful_checkpoints` | **Best-effort** | Failure logs, does not abort; may replay |
| `executions.lease_holder` / `lease_expires_at` (Layer 1) | **Durable + enforced** (M2.2, ADR-0008/0015) | Heartbeat-driven; multi-runner takeover via TTL expiry |
| `executions.claimed_by` / `claimed_until` (Layer 2, Sprint E) | **Schema may precede enforcement** | Spec-16 scaffolding, deferred to 1.1 — no engine consumers today |

**Lease enforcement (Layer 1, shipped via M2.2):** the engine's
`acquire_and_heartbeat_lease` (`crates/engine/src/engine.rs:815-859`)
acquires a lease on `execute_workflow` / `resume_execution`, spawns a
heartbeat task that renews every `DEFAULT_EXECUTION_LEASE_HEARTBEAT_INTERVAL`
(10s) within `DEFAULT_EXECUTION_LEASE_TTL` (30s), and tears the runner
down on `Ok(false)` (lease stolen) or `Err(_)` (storage failure). The
durable fence is `lease_holder` string match — a stale runner whose
holder no longer matches gets rejected on `renew_lease`. Multi-runner
takeover is verified by `crates/engine/tests/lease_takeover.rs`,
`crates/storage/tests/execution_lease_pg_integration.rs`, and the
loom probe in `crates/storage-loom-probe/src/lease_handoff.rs`.

**Layer 2 status:** the `claimed_by` / `claimed_until` columns + the
two indexes (`idx_executions_pending_claim`, `idx_executions_stale_lease`)
defined by `migrations/postgres/0011_executions.sql` are **Sprint E
(1.1) scaffolding**, intentionally inert until the spec-16 row-model
engine refactor lands. See ROADMAP "Out of scope for 1.0" → "Storage
Layer 2 / spec-16 multi-tenant row model (Sprint E)". The
`Schema may precede enforcement` warning therefore applies to Layer 2
only — Layer 1 is enforced today.

### Architecture notes

- Clean separation of types vs persistence: this crate defines the state machine and types;
  `nebula-storage::ExecutionRepo` persists them. Canon §11.1 makes the persistence layer
  authoritative — this crate deliberately does not own a repository interface.
- No cross-layer dependencies: only `nebula-core`, `nebula-error`, `nebula-workflow`.
  No imports from engine, runtime, storage, or API.
