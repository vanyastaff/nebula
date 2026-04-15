# Engine Guarantees

Scope: expanded operator-oriented version of `docs/PRODUCT_CANON.md` §11.

## Source of Truth

- Execution lifecycle authority: `nebula-execution` + `ExecutionRepo` (CAS/versioned transitions).
- Durable history: `execution_journal` (append-only).
- Durable control signals: `execution_control_queue` (outbox, at-least-once dispatch path).

## Durability Matrix


| Artifact                    | Durability                              | Operator Meaning                                       |
| --------------------------- | --------------------------------------- | ------------------------------------------------------ |
| executions row + state JSON | Durable                                 | Authoritative run state                                |
| execution_journal           | Durable                                 | Replayable timeline                                    |
| execution_control_queue     | Durable                                 | Command path for run/cancel                            |
| stateful_checkpoints        | Durable write, best-effort failure mode | Resume anchor; may lag latest side effect              |
| execution_leases            | Partial / evolving                      | Do not assume full lease enforcement unless documented |
| in-process channels         | Ephemeral                               | Never source of truth                                  |


## Critical Failure Semantics

- Checkpoint/side-effect race: if side effect commits and checkpoint write fails, replay may re-enter step.
- Protection is idempotency-key based (`{execution_id}:{node_id}:{attempt}`) before side effect.
- Exactly-once is not implied unless explicitly documented for a specific path.

## Resource Lifecycle Guarantees

- Acquire/release is engine-owned in-process contract.
- Crash-time cleanup is best-effort and may require next-process drain.
- External exclusive resources require TTL/dead-man strategy outside Nebula v1.

## Required Status Vocabulary

Use the same terms as canon §11.6:

- `implemented`
- `best-effort`
- `experimental`
- `planned`
- `demo-only`
- `false capability`