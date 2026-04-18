---
id: 0008
title: resume-persistence-schema
status: accepted
date: 2026-04-18
supersedes: []
superseded_by: []
tags: [storage, execution, resume, persistence, schema]
related:
  - crates/storage/src/execution_repo.rs
  - crates/storage/migrations/
  - docs/PRODUCT_CANON.md#115
  - docs/PRODUCT_CANON.md#111
  - docs/PRODUCT_CANON.md#10
---

# 0008. Resume persistence schema

## Context

`WorkflowEngine::resume_execution` (`crates/engine/src/engine.rs`) reconstructs
runtime decisions from a persistence record that was never designed to carry
everything replay needs. Four open issues all trace to the same root cause —
the persisted shape describes what finished, not what the engine decided:

| Issue | What is lost on resume | Current symptom |
|---|---|---|
| [#311](https://github.com/vanyastaff/nebula/issues/311) | Original workflow trigger input | Resume passes `Value::Null` to entry nodes |
| [#324](https://github.com/vanyastaff/nebula/issues/324) | OnError edge activations from `Failed` predecessors | Reconstruction only marks `Completed\|Skipped` sources active |
| [#336](https://github.com/vanyastaff/nebula/issues/336) | Per-edge condition (branch key, port) | All outgoing edges of `Completed` nodes unconditionally activate |
| [#299](https://github.com/vanyastaff/nebula/issues/299) | `ActionResult` variant (Branch / Route / MultiOutput / Skip / Wait) | `check_and_apply_idempotency` synthesizes `ActionResult::success(output)` |

The `ExecutionRepo` layer persists only the primary output payload
(`node_outputs.output`) and the execution state JSON. `ActionResult` variants
and their flow-control intent (selected branch key, output port, skip reason,
wait condition, …) never reach storage. Workflow trigger input also never
reaches storage — the engine owns it in memory only.

This chip (B1 of the engine-lifecycle canon cluster 2026-04 plan) is
**foundation only**: schema + repo seam, no engine behavior change.

Canon impact:

- `§11.1` — resumed execution must be byte-equivalent to non-crashed run.
- `§11.5` — extends what is durable; adds rows/columns.
- `§10 step 7` — persistence story must be explicit.

## Decision

### 1. Persistence choice — persist the full `ActionResult<Value>` per node

**Option 1** from [#299](https://github.com/vanyastaff/nebula/issues/299) body:
persist the full `ActionResult<Value>` next to each node output. The engine
keeps `evaluate_edge` as the single source of truth — on resume it re-runs the
same decision over the persisted variant and produces identical edge
activations to the non-crash run.

We considered **Option 3** (persist edge-activation decisions per edge) and
rejected it:

- Requires a new `edge_activations` table plus a write path coordinated with
  `evaluate_edge` on every dispatch — larger implementation surface for the
  same replay guarantee.
- Splits the source of truth: `evaluate_edge` at dispatch time; persisted
  activation rows at resume time. Option 1 keeps one path.
- Future changes to edge semantics (new conditions, new variants) require
  updating two places instead of one.

Option 1 costs a few extra columns and a JSON blob per node attempt;
Option 3 costs a table and a coordinated write. For a foundation chip that
must not preempt later engine refactors, Option 1 wins.

### 2. Forward-compatibility contract — explicit schema version column

Persisting full `ActionResult<Value>` means a future variant added to
`ActionResult` breaks deserialization in an older binary that reads a record
written by a newer one. We pick **explicit `result_schema_version INTEGER`**
over the two alternatives:

- `#[serde(other)]` fallback on the tag field — does not compose well with
  tagged enums and silently degrades variant identity on resume (a `Branch`
  becomes "unknown", which is exactly the bug we are trying to avoid).
- `#[non_exhaustive]` deserialization catching unknown variants — works for
  variant-only changes but gives no signal for field-shape changes within a
  variant.

**Contract:**

- Every persisted node-result row carries `result_schema_version INTEGER`
  (current value: `1`).
- Any change that could make an older binary fail to decode — new variant,
  new required field, changed field semantics — **must** bump the version.
- On load, if `result_schema_version > MAX_SUPPORTED_SCHEMA_VERSION`,
  `ExecutionRepo::load_node_result` returns
  `ExecutionRepoError::UnknownSchemaVersion { version, max_supported }`.
  Callers (the engine) surface this as a resume failure with operator-
  actionable context — never default to Null, never synthesize a fallback.
- `#[non_exhaustive]` stays on `ActionResult` (today) and on the new
  `NodeResultRecord` so additions outside version bumps remain source-
  compatible for first-party consumers.

**Mixed-binary deployment:** an older binary resuming an execution written by
a newer binary fails loud (typed error). An operator sees this in the resume
error; it is a deploy-ordering bug, not silent data loss.

### 3. Workflow input persistence — column on `executions`

Workflow input lives in a new `executions.input` column (JSONB in Postgres,
JSON in SQLite). Rationale:

- Same lifecycle as the execution row (created once at start, never mutated),
  same FK scope. A separate `execution_inputs` table adds a join for every
  resume without a single query that needs it standalone.
- Existing `executions` row size stays modest — workflow inputs in practice
  are small trigger payloads. Very large inputs go through the existing blob
  reference pattern (`crates/storage/src/repos/blob.rs`, out of scope here).
- Size bound: the engine passes the column through `serde_json::Value`. A
  soft bound of 1 MiB matches Postgres's practical TOAST crossover; inputs
  larger than that should be referenced blobs. This is **guidance, not an
  enforced check** at this chip — B2 wires the consumer and may add the
  check if needed.
- Missing / null on resume: the repo returns `Ok(None)` from
  `get_workflow_input`. B2 (the resume consumer chip) converts `None` to a
  typed `ResumeError::MissingInput` per `feedback_no_shims.md` and §4.5 —
  no silent `Value::Null` default. This chip only exposes the seam.

### 4. Migration — forward-only, both dialects

Two parallel migration paths exist in `crates/storage/migrations/` today
(Layer 1 Postgres schema consumed by `PgExecutionRepo`; Layer 2 spec-16
schema pending adoption). B1 lands matching changes in both so they stay in
sync as Layer 2 moves forward:

- **Layer 1** (`migrations/00000000000009_add_resume_persistence.sql`,
  Postgres dialect — the only Layer 1 dialect today):
  - `ALTER TABLE executions ADD COLUMN input JSONB`.
  - `ALTER TABLE node_outputs` adds `result_schema_version INTEGER NOT NULL
    DEFAULT 1`, `result_kind TEXT`, `result JSONB`. Legacy rows (no
    `result`) return `None` from `load_node_result`; B3 writes the new
    columns alongside the existing `output`.
- **Layer 2** (`migrations/{postgres,sqlite}/0020_add_resume_result_persistence.sql`):
  - `executions.input` already exists in the Layer 2 schema — no change.
  - `ALTER TABLE execution_nodes` adds the same three result columns.

Both are **forward-only**. Rollback means dropping the added columns, which
loses any persisted resume context; this is acceptable because the columns
are nullable and pre-migration engines never read them.

### 5. Coordination with A1 (control plane consumer ADR)

B1 assumes **nothing** about A1's choice of control-plane consumer wiring.
The schema changes are orthogonal:

- A1 decides how cancel/dispatch signals reach the engine from
  `execution_control_queue` (§12.2). That does not constrain what each node
  attempt persists.
- B1 decides what is durable per node attempt for replay. That does not
  constrain how control signals are consumed.

If A1 lands first with a consumer that needs to read persisted results, it
reads through the trait surface defined here. If A1 lands later, nothing in
B1 has to change. The two ADRs close the loop independently; any
integration friction surfaces in B4 or A2 where both paths meet the engine.

## Consequences

Positive:

- **Resume becomes replay-complete.** B4 can use the same `evaluate_edge`
  path over persisted results to reconstruct edge activations (fixes
  [#299](https://github.com/vanyastaff/nebula/issues/299), [#324](https://github.com/vanyastaff/nebula/issues/324),
  [#336](https://github.com/vanyastaff/nebula/issues/336)).
- **Workflow input becomes durable** (fixes
  [#311](https://github.com/vanyastaff/nebula/issues/311) once B2 wires the
  resume read).
- **Schema version discipline** surfaces compat breaks loudly instead of
  silently corrupting resume state.
- **One source of truth for edge decisions** — the engine's `evaluate_edge`
  stays canonical; no second implementation for resume.

Negative / accepted costs:

- Storage size grows by one JSONB per node attempt (roughly the size of the
  action output — variant tagging adds a few bytes) plus one JSONB per
  execution for the input. For workloads with small action outputs this is
  a modest increase; heavy workloads that already use reference outputs
  (`ActionOutput::Reference`) already pay small storage today.
- Any addition to `ActionResult` shape now requires a schema version bump
  and matching load-path coverage. This is the price of forward-compat
  honesty.
- Legacy `node_outputs` rows (written before this migration) cannot be
  resumed via the new path — `load_node_result` returns `None`. Resume
  falls through to legacy `load_node_output` behavior, which is the
  pre-existing (broken) path. B3/B4 handle the transition; this is fine
  for an alpha-stage engine with no durable historical executions.

Follow-up:

- **B2** — persist workflow input on start; resume reads it and surfaces
  `MissingInput` via typed error.
- **B3** — engine writes `NodeResultRecord` alongside the legacy `output`
  column for every dispatch.
- **B4** — resume reads results, replays `evaluate_edge`, closes #299 /
  #324 / #336.
- **E1** — removal of `ActionResult::Retry` bumps `result_schema_version`
  to 2 under this contract.

## Alternatives considered

- **Option 3 — per-edge activation rows.** Cleaner long-term but doubles
  the write path and splits the source of truth. See decision 1.
- **Separate `execution_inputs` table for workflow input.** Adds a join on
  every resume for a column that is always fetched alongside the execution
  row. See decision 3.
- **Silent fallback on unknown variant / version.** Violates §4.5 (silent
  false capability). Rejected — see decision 2.
- **Backward-rolling migrations.** Dropping columns after a rollback loses
  persisted context for executions already observed by the newer binary.
  Forward-only matches the alpha stage and the crash-is-normal model
  (§4.3). A future migration framework change could revisit this.

## Seam / verification

Seams (this ADR):

- `crates/storage/src/execution_repo.rs` —
  `ExecutionRepo::{set_workflow_input, get_workflow_input, save_node_result,
  load_node_result, load_all_results}`, `NodeResultRecord`,
  `ExecutionRepoError::UnknownSchemaVersion`.
- `crates/storage/src/backend/pg_execution.rs` — Postgres impl.
- `crates/storage/migrations/00000000000009_add_resume_persistence.sql` —
  Layer 1 schema.
- `crates/storage/migrations/{postgres,sqlite}/0020_add_resume_result_persistence.sql` —
  Layer 2 parity.

Tests (this ADR):

- `crates/storage/src/execution_repo.rs` unit tests —
  round-trip for each `ActionResult` variant, forward-compatibility test
  for `UnknownSchemaVersion`, workflow-input get/set.
- `crates/storage/src/backend/pg_execution.rs` tests (feature-gated) —
  same round-trips against Postgres when `DATABASE_URL` is available.

Related ADRs: 0007 (`ExecutionId` / `WorkflowId` shape used by the new
schema rows).
