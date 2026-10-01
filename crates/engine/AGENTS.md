# nebula-engine — Agent orientation
> Local guide for `crates/engine/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Reusable graph runtime control: `WorkflowEngine`, activation/start services, durable control consumption, and action dispatch. Deployment adapter selection and process lifecycle belong to `apps/`; worker assembly lives in `nebula-worker`.
**Layer:** Exec — follow the root dependency map and `deny.toml` for every new cross-crate edge.

## Common Tasks

| Task | Steps |
|------|-------|
| Change control dispatch | Start with port-owned `ControlCommand`, then `ControlDispatch` and the `EngineControlDispatch` implementation. Update durable encoding/adapters and worker recovery where affected; preserve claim fencing and duplicate-delivery behavior. |
| Change execution state transitions | Use the owning storage port: ordinary transitions use `ExecutionStore::commit(TransitionBatch)`; start/handoff use their dedicated atomic owner ports. Preserve CAS and fencing. |
| Add a new action context field | Wire through `src/credential_accessor.rs` or `src/resource_accessor.rs` — these are the cross-layer bridges. |
| Debug retry behavior | Two disjoint surfaces: Layer 1 (`nebula-resilience::retry_with`, opaque to engine) vs Layer 2 (`retry_policy`, engine parks node in `WaitingRetry`). See ADR-0042. |

## Commands

- Features: `rotation`, `test-util` (never in prod build — ADR-0023). The refresh-coordinator
  chaos harness follows its concrete shared-L2 adapter and lives in `nebula-storage`.
  (Out-of-process plugin execution was retired — ADR-0091; the engine dispatches actions
  in-process via `InProcessRunner`.)

## Key files

- `src/lib.rs` — module map + crate-root re-exports (downstream uses `nebula_engine::X`, not deep paths).
- `src/engine/mod.rs` — `WorkflowEngine` construction and retained runtime configuration.
- `src/engine/frontier.rs`, `src/engine/checkpoint.rs`, `src/engine/persistence.rs` — dispatch, routing checkpoints, and fenced persistence.
- `src/engine/control_turn.rs` — atomic Control Start acceptance and accepted-turn recovery; `src/engine/resume/` — exact recorded resume and lease handling.
- `src/workflow_activation/`, `src/start_materialization/`, `src/revision_catalog.rs` — compile/install, atomic start admission, and exact revision loading.
- `src/effect_driver/` — execution-owned effects: the remote-effect driver and the per-node-attempt `NodeEffectJournal` (`journal.rs`) of stateless `Journaled` actions, both over the shared `LedgerSlot` core (`slot.rs`); generic action dispatch must not bypass their ledger grants, and a journal verdict always overrides the action result.
- `src/control_consumer.rs` / `src/control_dispatch.rs` — durable `execution_control_queue` consumer + `EngineControlDispatch` (Start/Resume/Restart/Cancel/Terminate; canon §12.2, ADR-0008).
- `src/credential_accessor.rs` / `src/resource_accessor.rs` — scoped accessors injected into action contexts (cross-layer bridges).
- `src/scoped_resources.rs` — per-branch resource storage, layered lookup, RAII cleanup (M6.1/M6.2).
- `src/runtime/` — `ActionRuntime` dispatch, action runner, blob/queue plumbing.

## Conventions & never-do

- **Persist execution changes through their owning storage ports.** Ordinary transitions use `ExecutionStore::commit(TransitionBatch)` with CAS/fencing; start materialization and turn handoff have dedicated atomic ports. Never replace durable mutation with a local-only state change or split an owner transaction across independent writes.
- **Engine owns the control-queue consumer** — a handler that only logs/discards rows violates canon (L2-§12.2). `Cancel` reaches the live loop via `WorkflowEngine::cancel_execution`; dispatch must be idempotent per `(execution_id, command)`.
- Accepted Start claims recover through execution-owned durable markers. Do not acknowledge the queue a second time after acceptance or uncertain commit acknowledgement; verify the combined handoff in storage and worker tests.
- **Credential accessor is deny-by-default**: only the current node's exact, site-qualified
  entries from the persisted V2 execution binding manifest are resolvable. A missing manifest,
  slot, tenant match, or capability denies access; there is no process-local population or
  fail-open path. (Resources have no allowlist — scoping is the topology layer's job.)
- Not a storage impl or expression evaluator — those are `nebula-storage` / `nebula-expression`. Action dispatch is in-process (`InProcessRunner`); plugins register in-process through `nebula-plugin` (ADR-0091).
- Two disjoint retry surfaces (ADR-0042): in-action `nebula-resilience::retry_with` (Layer 1, opaque to engine) vs operator-declared `retry_policy` (Layer 2, engine parks node in `WaitingRetry`).

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Checkpoint/routing semantics | Unit suites [checkpoint_tests](src/engine/checkpoint_tests.rs), [durable_resume_tests](src/engine/durable_resume_tests.rs), plus [retry](tests/retry.rs) and [wait_recovery](tests/wait_recovery.rs) as affected. |
| Control delivery and ownership | [control_consumer_wiring](tests/control_consumer_wiring.rs), [control_dispatch](tests/control_dispatch.rs), [lease_takeover](tests/lease_takeover.rs); include worker accepted-turn recovery for handoff changes. |
| Remote effects | [effect_protocol](tests/effect_protocol.rs) and the storage operation-ledger conformance suites for the affected backend. |
| Journaled resource effects (`effect_driver/journal.rs`, the `Journaled` dispatch arm, `EngineResourceAccessor::with_journal`) | `effect_driver::journal::tests` (key golden vectors, ordinals, the gap an earlier attempt left below a recorded position → mismatch, replay phases, refusal→verdict, no writes and one read for a reading node, crash before the attempt was recorded, grant budget, bounded cleanup, destination/recording-policy mismatch), `engine::tests::journal_exit_tests` (every exit before the action — input, credential refresh, rate limit, cancellation — concludes the journal, so an earlier dispatch's unexplained call fails the node unknown; a branch scope that shadows a global row still refuses the journaled handle), [effect_protocol](tests/effect_protocol.rs) `journal_*` suites (positional occurrences: a redeployed operation key or version at a settled position is a mismatch with nothing sent; a submission dropped unpolled before a crash takes no position, so the resume replays; a resume that skips a settled effect fails; no error strategy continues past a verdict; in-memory + SQLite; PostgreSQL in the CI evidence job), [resource_integration](tests/resource_integration.rs) `resource_handle` (storeless read-only detail, raw leases refused). Never change the provider key composition without re-pinning the golden vectors and a migration story. |
| Resource/credential bridges | [resource_slot_identity_cross_crate](tests/resource_slot_identity_cross_crate.rs), [credential_lease_lifecycle](tests/credential_lease_lifecycle.rs), and rotation-feature tests when relevant. |

## See also

- `README.md` — full design, known open debts, architecture notes.
- Canon [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §10/§11.1/§12.2/§13 · ADR-0008/0015/0016/0025/0042/0050.
