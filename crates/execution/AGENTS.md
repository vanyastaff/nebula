# nebula-execution — Agent orientation
> Local guide for `crates/execution/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Shared execution-time model — the 8-state `ExecutionStatus` machine, `JournalEntry` (WAL), `IdempotencyKey`, and the `ExecutionPlan` derived from the workflow DAG — so engine and storage share one truth, not two.
**Layer:** Core — depends only downward (`nebula-core`, `nebula-error`, `nebula-workflow`); no engine/storage/api/runtime imports.

## Commands

- Snapshot tests use `insta` (state/plan serialization); inspect the wire-format change before accepting snapshots. Run `cargo insta review` from `crates/execution/` to scope review to this crate.

## Key files

- `src/lib.rs` — module roots + public re-exports (the crate's whole surface).
- `src/status.rs` — `ExecutionStatus` 8-state enum (`Created`…`TimedOut`), plus terminal reasons; serialized state names are persisted contracts.
- `src/transition.rs` — validates state-machine transition *legality* only (persistence/CAS is storage's job).
- `src/state.rs` — `ExecutionState` / `NodeExecutionState`, serialized into the `executions` row (largest module).
- `src/journal.rs` — `JournalEntry`, backs the append-only `execution_journal` table.
- `src/idempotency.rs` — `IdempotencyKey` shape `{execution_id}:{node_id}:{attempt}` (format only; dedup lives in storage).
- `src/revision.rs` — default-public workflow-version and worker-flavor revision-pin aggregate.
- `src/bundle.rs` — immutable Graph-v1 execution-contract bundle, canonical structural
  fingerprint, recorded wire input, and typed structural-integrity errors.
- `src/plan.rs` / `src/replay.rs` — `ExecutionPlan` (parallel schedule) and `ReplayPlan` (checkpoint resume).

## Conventions & never-do

- This crate defines *types + transition legality only*. It must NOT own a repository interface, persist state, or enforce CAS — the spec-16 storage port (`nebula-storage-port::ExecutionStore` + `TransitionBatch`, implemented by `nebula-storage`) is the single source of truth for persisted state (canon §11.1; ADR-0072).
- Revision and bundle types do not by themselves prove end-to-end support. Check compiler,
  admission, persisted routing, and exact-flavor dispatch for the specific workflow being
  changed. `WorkflowVersionId` remains the workflow revision identity.
- A Graph-v1 bundle's exact loaded plan must contain slot-to-selected-`CredentialId` mappings and
  credential contract revisions. Admission compares the unique selected IDs exactly with the
  bundle set; abstract credential requirements alone are insufficient.
- `PluginSetId` is an independent plugin-set pin, not proof of schemas, runtime behavior,
  artifact authenticity, authorization, or a complete frozen registry.
- This crate defines retry state shapes only: legal `Failed → WaitingRetry → Ready` node transitions, `next_attempt_at`, `total_retries`, `ExecutionBudget.max_total_retries`, `NodeAttempt`, and idempotency-key shape. The engine owns operator-declared node retry (`retry_policy`) and re-dispatch; `nebula-resilience` remains the in-action outbound-call retry surface. Do not add an `ActionResult::Retry` scheduler here.
- `IdempotencyKey` here is only the deterministic per-attempt local replay/dedup shape (§11.3);
  storage owns `check_and_mark`. It changes across retries and is not the future stable remote
  `OperationId` or storage-minted `EffectSlotId`; neither primitive makes a provider effect
  atomic with Nebula persistence. Only a pinned stable-key destination may make bounded
  same-`OperationId` effect-call re-invocations while its guarantee remains valid;
  reconciliation is read-only. Exhaustion or expiry becomes `OutcomeUnknown`, after which no
  effecting call may repeat. `AcknowledgementUnknown` applies to prepare and outcome DB commits:
  prepare uncertainty forbids provider invocation until the exact durable prepared record is
  confirmed; outcome uncertainty permits only ledger reads and exact frozen-evidence recommit.
  The control-queue / outbox also live in storage, not here.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Persisted state and transitions | Unit tests in [src/status.rs](src/status.rs), [src/transition.rs](src/transition.rs), and [src/state.rs](src/state.rs); inspect serialization snapshots before accepting them. |
| Bundle pins or fingerprints | [execution_contract_bundle](tests/execution_contract_bundle.rs); changes also require the plugin compiler and engine recorded-contract consumers to agree. |

## See also

- `README.md` — full design, durability matrix, lease-enforcement notes.
- Canon [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §11.1/§11.2/§11.3/§11.5/§12.2.
