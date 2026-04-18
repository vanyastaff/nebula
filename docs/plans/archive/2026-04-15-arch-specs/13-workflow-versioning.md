# Spec 13 — Workflow versioning

> **Status:** draft
> **Canon target:** §7.3 (new), §11.5 (matrix update)
> **Depends on:** 06 (IDs), 12 (expression compilation), 16 (storage)
> **Depended on by:** 11 (triggers resolve to workflow version), 14 (stateful execution pinned to version)

## Problem

Workflows are living artefacts — users edit, fix bugs, add nodes. **Executions are also live** — they can run for minutes, hours, or days. Without explicit versioning, a running execution might suddenly use different workflow logic when the user saves an edit. Airflow has this problem (re-parses DAG on every scheduler cycle), causing subtle inconsistencies.

Every platform has a war story:

- **Airflow** — DAG changes mid-run cause silent inconsistencies. No fix; operators warned to «wait for runs to finish before editing»
- **n8n** — snapshots workflow JSON in execution row, but has no version history or rollback
- **Temporal** — explicit `Workflow.getVersion()` API, works for developers but is a learning cliff
- **GitHub Actions** — workflow file pinned to commit SHA, clean but requires git
- **Windmill** — scripts versioned, flows reference specific versions, closest to what we need

## Decision

**Two-table model: `workflows` (mutable pointer) + `workflow_versions` (immutable history).** Each execution pins to a specific `workflow_version_id` at start time, never changes. Draft/Published state machine prevents accidentally activating in-progress edits. Retention policy keeps history bounded.

## Data model

### `workflows` — current pointer

```rust
pub struct WorkflowId(Ulid);  // "wf_"

pub struct Workflow {
    pub id: WorkflowId,
    pub workspace_id: WorkspaceId,
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    pub current_version_id: WorkflowVersionId,  // points to latest Published
    pub state: WorkflowState,
    pub created_at: DateTime<Utc>,
    pub created_by: UserId,
    pub updated_at: DateTime<Utc>,
    pub version: u64,  // CAS
}

pub enum WorkflowState {
    Active,     // can be triggered, edited
    Paused,     // triggers disabled, but still viewable
    Archived,   // hidden by default, kept for history
}
```

### `workflow_versions` — immutable history

```rust
pub struct WorkflowVersionId(Ulid);  // "wfv_"

pub struct WorkflowVersion {
    pub id: WorkflowVersionId,
    pub workflow_id: WorkflowId,
    pub version_number: u32,         // 1, 2, 3, ... per workflow, user-facing
    pub definition: WorkflowDefinition,  // full DAG, params, connections
    pub schema_version: u16,          // see 13.6
    pub state: VersionState,
    pub created_at: DateTime<Utc>,
    pub created_by: UserId,
    pub description: Option<String>,  // optional commit message
    pub compiled_expressions: Option<Vec<u8>>,  // cached bytecode from spec 12
    pub compiled_validation: Option<Vec<u8>>,   // cached validation result
}

pub enum VersionState {
    Draft,       // being edited, not active for triggers
    Published,   // live, used by automatic triggers
    Archived,    // superseded but kept for history + running executions
    Deleted,     // soft-deleted, will be GC'd per retention policy
}
```

### SQL

```sql
CREATE TABLE workflows (
    id                  BYTEA PRIMARY KEY,
    workspace_id        BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    slug                TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    description         TEXT,
    current_version_id  BYTEA NOT NULL,  -- FK added after workflow_versions exists
    state               TEXT NOT NULL,    -- Active / Paused / Archived
    created_at          TIMESTAMPTZ NOT NULL,
    created_by          BYTEA NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL,
    version             BIGINT NOT NULL DEFAULT 0,
    deleted_at          TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_workflows_workspace_slug
    ON workflows (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;

CREATE INDEX idx_workflows_state
    ON workflows (workspace_id, state)
    WHERE deleted_at IS NULL;

CREATE TABLE workflow_versions (
    id                      BYTEA PRIMARY KEY,
    workflow_id             BYTEA NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    version_number          INT NOT NULL,
    definition              JSONB NOT NULL,       -- full workflow
    schema_version          INT NOT NULL,
    state                   TEXT NOT NULL,        -- Draft / Published / Archived / Deleted
    created_at              TIMESTAMPTZ NOT NULL,
    created_by              BYTEA NOT NULL,
    description             TEXT,
    compiled_expressions    BYTEA,                -- serialized bytecode cache
    compiled_validation     BYTEA,                -- cached validation ok/errors
    
    UNIQUE (workflow_id, version_number)
);

-- Only one Published version per workflow at a time
CREATE UNIQUE INDEX idx_workflow_versions_published
    ON workflow_versions (workflow_id)
    WHERE state = 'Published';

CREATE INDEX idx_workflow_versions_by_workflow
    ON workflow_versions (workflow_id, version_number DESC);

-- Add FK after table exists
ALTER TABLE workflows
    ADD CONSTRAINT fk_workflows_current_version
    FOREIGN KEY (current_version_id) REFERENCES workflow_versions(id);
```

### `executions` pin to version

```sql
ALTER TABLE executions
    ADD COLUMN workflow_version_id BYTEA NOT NULL REFERENCES workflow_versions(id);

CREATE INDEX idx_executions_version ON executions(workflow_version_id);
```

**Invariant: `executions.workflow_version_id` is immutable after INSERT.** The version pinned at start is the version used for the entire lifetime of the execution.

## State machine

```
                    ┌────────────────────┐
                    │                    │
               ┌───►│      Draft         │
               │    │  (being edited)    │
               │    └──────────┬─────────┘
               │               │ user clicks Publish
               │               ▼
  user clicks  │    ┌────────────────────┐
   Edit / new  │    │    Published       │◄──────┐
   draft       │    │  (used by triggers)│       │
               │    └──────────┬─────────┘       │
               │               │ new version published  │
               │               ▼                 │
               │    ┌────────────────────┐       │
               └────┤     Archived       │       │
                    │ (kept for history) │       │
                    └──────────┬─────────┘       │
                               │ GC job + retention met
                               ▼
                    ┌────────────────────┐       │
                    │     Deleted        │       │
                    │ (soft, then hard)  │       │
                    └────────────────────┘       │
                               │ admin «revert»  │
                               └─────────────────┘
                                 (creates copy as new Draft)
```

**Only one Published at a time.** Unique index enforces. New publish atomically archives the old published and marks new one published.

## Flows

### Create initial workflow

```
POST /api/v1/orgs/{org}/workspaces/{ws}/workflows
Body: { "slug": "onboard-user", "display_name": "Customer Onboarding", "definition": { ... } }
  ↓
Permission check: WorkspaceEditor
  ↓
Validate slug (unique in workspace, not reserved)
Validate workflow definition (acyclic, all nodes reference existing actions, etc.)
Compile expressions (spec 12)
  ↓
BEGIN TRANSACTION
  INSERT workflow_versions (version_number=1, state='Published', ...)
  INSERT workflows (current_version_id=version_id_we_just_inserted, ...)
COMMIT
  ↓
Return 201 Created with workflow + version details
```

### Edit workflow (create draft)

```
POST /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/edit
  ↓
Permission check: WorkspaceEditor
  ↓
Check if Draft exists for this workflow
  - If yes: return existing draft
  - If no: copy current Published into new Draft row
  ↓
BEGIN TRANSACTION
  INSERT workflow_versions (version_number=current.next, state='Draft', definition=copy, ...)
COMMIT
  ↓
Return draft details
  ↓
User edits in UI (updates draft JSON through PATCH endpoint)
  ↓
PATCH /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/versions/{wfv}
Body: { "definition": { ... } }
  ↓
Permission check: WorkspaceEditor
Ensure version is in Draft state (not Published)
Validate new definition
Compile expressions
  ↓
UPDATE workflow_versions SET definition=?, compiled_expressions=?, ...
  WHERE id=? AND state='Draft'
```

### Publish draft

```
POST /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/versions/{wfv}/publish
  ↓
Permission check: WorkspaceEditor (or Admin?)
  ↓
Load draft, re-validate (defense-in-depth — may have been edited while workflow state changed)
  ↓
BEGIN TRANSACTION
  UPDATE workflow_versions SET state='Archived' WHERE workflow_id=? AND state='Published'
  UPDATE workflow_versions SET state='Published' WHERE id=?
  UPDATE workflows SET current_version_id=?, version=version+1, updated_at=NOW() WHERE id=?
COMMIT
  ↓
Emit event: WorkflowVersionPublished
  ↓
Trigger cascades (running cron jobs for this workflow start using new version on next fire)
```

### Start execution (pin to version)

```
POST /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/executions
Body: { "input": { ... } }
  ↓
Permission check: WorkflowExecute
Quota check
  ↓
Load workflow + current_version_id (snapshot at start)
Load workflow_version with full definition
  ↓
INSERT executions (workflow_version_id=current_version_id, status='Pending', ...)
  ↓
Return 202 with execution_id
```

**Key:** after INSERT, changes to `workflows.current_version_id` don't affect this execution. It's pinned.

### Rollback (publish copy of old version)

```
POST /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/versions/{wfv}/revert
Body: { "description": "rolling back breaking change from yesterday" }
  ↓
Permission check: WorkspaceEditor
  ↓
Load target version (must be Archived, not Deleted)
  ↓
BEGIN TRANSACTION
  INSERT workflow_versions (
    workflow_id=target.workflow_id,
    version_number=max(existing) + 1,
    definition=target.definition,  -- copy
    state='Published',
    description='Revert to v{target.version_number}: {user_reason}',
    ...
  )
  UPDATE workflow_versions SET state='Archived' WHERE id=current_published_id
  UPDATE workflows SET current_version_id=new_version_id
COMMIT
```

**Why not «activate v5 directly»:** linearity. If we allowed «set published to v5», history becomes non-linear — which version was published at time X? Copy-as-new-version keeps audit trail clean.

### Delete workflow (soft)

```
DELETE /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}
  ↓
Permission check: WorkspaceAdmin
  ↓
UPDATE workflows SET state='Archived', deleted_at=NOW() WHERE id=?
  ↓
Running executions continue (pinned to their version, which still exists)
New executions rejected with 404 or 410 Gone
```

Hard delete of workflow + all versions happens via retention job after 30 days.

## Automatic triggers and versioning

**Rule:** automatic triggers (cron, webhook, event) resolve to **latest Published version at claim time**, not receive time.

### Webhook nuance

```
Day 1, 10:00 → webhook received, trigger_events row inserted (claim_state='pending')
Day 1, 10:01 → user publishes new version v6
Day 1, 10:02 → worker claims trigger_events row
                reads workflows.current_version_id = v6
                creates execution pinned to v6
```

Execution uses **v6** (version at claim time), not v5 (version at receive time).

**Documented behavior.** User intends edits to affect future handling, and claim happens after edit.

If some users want «events received before edit use old version», that's pinned-at-receive semantics, which we don't implement in v1. Deferred if strong need.

### Cron nuance

```
Cron config uses latest Published at each fire.
If user publishes new version mid-day, next cron fire uses new version.
```

This is the desired behavior — user fixed a bug, wants next run to use fixed version.

## Retention policy

**Core question:** which versions to keep forever, which to GC?

**Always keep:**

- Current `Published` version (one per workflow)
- Any version referenced by an execution (join with `executions.workflow_version_id`)
- User-pinned versions (flag `pinned=true`, set by admin action, default false)

**Eligible for GC:**

- Archived versions **without** executions referencing them
- After grace period (30 days from archival)
- Keep last N unpinned archived versions regardless (default 20) for «oh I need to see v15 real quick» recovery

**GC job:**

```sql
-- Daily background job
DELETE FROM workflow_versions
WHERE state = 'Archived'
  AND id NOT IN (SELECT DISTINCT workflow_version_id FROM executions)
  AND id != (SELECT current_version_id FROM workflows WHERE id = workflow_versions.workflow_id)
  AND NOT COALESCE(pinned, false)
  AND created_at < NOW() - INTERVAL '90 days'
  AND id NOT IN (
    -- Keep last 20 archived versions per workflow
    SELECT id FROM workflow_versions wv2
    WHERE wv2.workflow_id = workflow_versions.workflow_id
      AND wv2.state = 'Archived'
    ORDER BY wv2.created_at DESC
    LIMIT 20
  );
```

**Storage accounting:** each version is a full JSON copy. Typical workflow is 5–50 KB, up to maybe 500 KB for complex ones. 100 versions × 50 KB = 5 MB per workflow. Hundreds of workflows per org → low tens of MB. Acceptable.

**If storage quota pressure:** GC triggered eagerly. Last resort: admin force-delete non-essential versions.

## Schema versioning (engine upgrades)

Separate from user-facing version numbers. `schema_version` int tracks the workflow JSON format across engine versions.

```rust
pub const CURRENT_SCHEMA_VERSION: u16 = 3;

pub fn load_workflow_version(row: WorkflowVersionRow) -> Result<WorkflowDefinition> {
    let raw: serde_json::Value = row.definition;
    let definition = match row.schema_version {
        1 => {
            let v1: WorkflowDefinitionV1 = serde_json::from_value(raw)?;
            migrate_v1_to_v2(v1).and_then(migrate_v2_to_v3)?
        }
        2 => {
            let v2: WorkflowDefinitionV2 = serde_json::from_value(raw)?;
            migrate_v2_to_v3(v2)?
        }
        3 => serde_json::from_value(raw)?,  // current
        v => return Err(LoadError::UnsupportedSchemaVersion(v)),
    };
    Ok(definition)
}
```

**Migration chain:** v1 → v2 → v3 → current. Forward only, never backward. Each release adding a new schema version must include its migration function.

**On save:** always serialize as current schema version. No «save as v2 for compatibility».

**On load:** walk the migration chain.

**`docs/UPGRADE_COMPAT.md`** gets a table:

| Schema version | Engine range | Breaking change | Migration |
|---|---|---|---|
| 1 | 0.1.0–0.2.x | initial | — |
| 2 | 0.3.0+ | added X | auto-migrate on load |
| 3 | 0.5.0+ | renamed Y | auto-migrate on load |

## Validation

### At workflow save / publish

Canon §10 requires validation before activation. This happens at Draft save and Publish:

```rust
pub fn validate_workflow_version(def: &WorkflowDefinition) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    
    // 1. Acyclicity (from spec 09 R3 constraint)
    if let Err(cycle) = petgraph::algo::toposort(&def.graph(), None) {
        errors.push(ValidationError::CycleDetected { .. });
    }
    
    // 2. All nodes reference valid actions
    for node in &def.nodes {
        if !action_registry.has(&node.action_key) {
            errors.push(ValidationError::UnknownAction { .. });
        }
    }
    
    // 3. All expressions compile (from spec 12)
    for (loc, expr) in def.all_expressions() {
        if let Err(e) = compile(&expr) {
            errors.push(ValidationError::ExpressionError { location: loc, cause: e });
        }
    }
    
    // 4. Parameter schemas match action metadata
    for node in &def.nodes {
        let meta = action_registry.metadata(&node.action_key)?;
        if let Err(e) = validate_params(&node.params, &meta.parameters) {
            errors.push(ValidationError::ParameterError { .. });
        }
    }
    
    // 5. Timeout waterfall (from spec 10)
    if let Err(e) = validate_timeout_waterfall(def) {
        errors.push(ValidationError::TimeoutWaterfall { .. });
    }
    
    // 6. Credential / resource references valid in this workspace
    for node in &def.nodes {
        for cred_ref in node.credentials() {
            if !credential_accessible_in_workspace(cred_ref, workspace_id)? {
                errors.push(ValidationError::CredentialNotAccessible { .. });
            }
        }
    }
    
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}
```

**Validation is called:**
- On Draft save (returns errors in 422 response, user fixes and retries)
- On Publish (defence-in-depth, should rarely fail here if Draft was valid)
- On trigger firing (final check, should not fail — if it does, platform bug)

**Validation results cached** in `workflow_versions.compiled_validation` column to avoid re-validating on every execution start.

## Configuration surface

```toml
[workflows]
# Retention
archived_grace_period = "90d"
keep_last_archived_versions = 20

# Validation
max_nodes_per_workflow = 1000
max_connections_per_workflow = 2000
max_workflow_json_size = "1MB"

# GC
gc_interval = "24h"
```

## Testing criteria

**Unit tests:**
- State machine transitions (Draft → Published → Archived, no illegal)
- `version_number` increments monotonically per workflow
- Validation catches cycles, unknown actions, bad expressions
- Schema migration chain works for each supported version
- Retention GC query selects correct rows

**Integration tests:**
- Create workflow → v1 Published
- Edit → Draft created, v1 still Published
- Publish Draft → v2 Published, v1 Archived
- Start execution while v2 Published → pinned to v2
- Edit to v3 Draft → running execution still v2
- Publish v3 → running execution still v2, new ones v3
- Trigger fires → claims current_version_id at claim time (test timing window)
- Rollback creates new version that's copy of old
- Delete workflow → running executions complete, new trigger fires rejected
- Retention GC: old archived versions without references removed
- Retention GC: referenced versions kept forever

**Race tests:**
- Concurrent Publish + Edit on same workflow → one wins, other gets 409 Conflict
- Concurrent Publish of different Drafts → only one becomes Published (CAS on workflows.version)

**Schema migration tests:**
- Load v1 schema → produces correct current version
- Load unknown schema → returns UnsupportedSchemaVersion error
- Save always writes current schema version

## Performance targets

- Workflow save (with validation + compile): **< 100 ms p99** for typical workflow
- Workflow load by version_id: **< 5 ms p99**
- Retention GC job: **< 1 minute** for org with 10_000 workflows × 100 versions
- Validation: **< 50 ms p99** for typical workflow (10–50 nodes)

## Module boundaries

| Component | Crate |
|---|---|
| `WorkflowId`, `WorkflowVersionId` | `nebula-core` |
| `Workflow`, `WorkflowVersion`, `VersionState` | `nebula-workflow` |
| `WorkflowDefinition` (existing) | `nebula-workflow` |
| `validate_workflow_version`, `ValidationError` | `nebula-workflow` |
| Schema migration functions | `nebula-workflow::schema_migration` |
| `WorkflowRepo`, `WorkflowVersionRepo` | `nebula-storage` |
| Retention GC job | `nebula-engine` (background) |
| Workflow CRUD handlers | `nebula-api` |
| Publish / rollback handlers | `nebula-api` |

## Migration path

**Greenfield** — new tables, no prior data. On first deployment, create tables, no migration needed.

**Future schema bump discipline:** when changing `WorkflowDefinition` struct:

1. Define `WorkflowDefinitionV{n}` as new variant
2. Bump `CURRENT_SCHEMA_VERSION` const
3. Write `migrate_v{n-1}_to_v{n}` function
4. Test with real v{n-1} workflows to ensure migration is lossless
5. Deploy
6. Over time, run batch job to re-save all workflows in new schema (optional optimization)

## Open questions

- **Concurrent Draft editing** — two users editing same Draft — last write wins or merge conflict? v1: last write wins (no merge). v2: OT-style or lock.
- **Partial Publish** — publish subset of workflow (just one node) — no, always full-workflow versions.
- **Workflow templates** — reusable partial workflows? Out of scope, use fork+edit instead.
- **Version diff in UI** — show JSON diff or smart node-level diff between v3 and v4? Nice-to-have for v1.5.
- **Publish approval workflow** — require reviewer before Publish? Enterprise feature, deferred.
- **Workflow import/export** — YAML or JSON format? Deferred.
- **Workflow as code** — define workflow in Rust code and sync to DB? Interesting for developers, deferred.
