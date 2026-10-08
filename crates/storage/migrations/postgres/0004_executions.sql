-- Executions: the exact revision catalog they run against, the execution
-- aggregate and everything recorded beneath an execution.
--
-- The revision catalog is declared here before its execution references.

-- ── Revision catalog ─────────────────────────────────────────────────────
--
-- Immutable compiled artifacts addressed by their 32-byte digests. A deleted
-- revision keeps its row (the digest stays taken) and drops its bytes.

CREATE TABLE worker_flavor_revisions (
    worker_flavor_id BYTEA NOT NULL,
    record_format    TEXT  NOT NULL,
    lifecycle        TEXT  NOT NULL,
    record_bytes     BYTEA,
    CONSTRAINT pk_worker_flavor_revisions PRIMARY KEY (worker_flavor_id),
    CONSTRAINT ck_worker_flavor_revisions__worker_flavor_id_length
        CHECK (octet_length(worker_flavor_id) = 32),
    CONSTRAINT ck_worker_flavor_revisions__record_format CHECK (record_format = 'v1_json'),
    CONSTRAINT ck_worker_flavor_revisions__lifecycle
        CHECK (lifecycle IN ('active', 'draining', 'deleted')),
    CONSTRAINT ck_worker_flavor_revisions__record_shape CHECK (
        (lifecycle IN ('active', 'draining')
            AND record_bytes IS NOT NULL AND octet_length(record_bytes) > 0)
        OR (lifecycle = 'deleted' AND record_bytes IS NULL)
    )
);

CREATE TABLE executable_plan_revisions (
    executable_plan_id BYTEA NOT NULL,
    worker_flavor_id   BYTEA NOT NULL,
    record_format      TEXT  NOT NULL,
    lifecycle          TEXT  NOT NULL,
    record_bytes       BYTEA,
    CONSTRAINT pk_executable_plan_revisions PRIMARY KEY (executable_plan_id),
    CONSTRAINT uq_executable_plan_revisions__executable_plan_worker_flavor
        UNIQUE (executable_plan_id, worker_flavor_id),
    -- A reference, not ownership: a flavor in use by a plan cannot go.
    CONSTRAINT fk_executable_plan_revisions__worker_flavor_revisions
        FOREIGN KEY (worker_flavor_id) REFERENCES worker_flavor_revisions (worker_flavor_id)
        ON DELETE RESTRICT,
    CONSTRAINT ck_executable_plan_revisions__executable_plan_id_length
        CHECK (octet_length(executable_plan_id) = 32),
    CONSTRAINT ck_executable_plan_revisions__worker_flavor_id_length
        CHECK (octet_length(worker_flavor_id) = 32),
    CONSTRAINT ck_executable_plan_revisions__record_format
        CHECK (record_format = 'graph_v1_json'),
    CONSTRAINT ck_executable_plan_revisions__lifecycle
        CHECK (lifecycle IN ('active', 'draining', 'deleted')),
    CONSTRAINT ck_executable_plan_revisions__record_shape CHECK (
        (lifecycle IN ('active', 'draining')
            AND record_bytes IS NOT NULL AND octet_length(record_bytes) > 0)
        OR (lifecycle = 'deleted' AND record_bytes IS NULL)
    )
);
CREATE INDEX ix_executable_plan_revisions__worker_flavor_id__undeleted
    ON executable_plan_revisions (worker_flavor_id) WHERE lifecycle <> 'deleted';

-- ── Executions ───────────────────────────────────────────────────────────
--
-- Execution ids are unique across tenants; (org_id, workspace_id, id) is the
-- key every row beneath an execution references, so each names its tenant.
-- An execution belongs to its workflow and is purged with it.

CREATE TABLE executions (
    org_id             TEXT        NOT NULL,
    workspace_id       TEXT        NOT NULL,
    id                 TEXT        NOT NULL,
    workflow_id        TEXT        NOT NULL,
    status             TEXT        NOT NULL,
    state              JSONB       NOT NULL,
    version            BIGINT      NOT NULL DEFAULT 0,
    lease_holder       TEXT,
    lease_expires_at   TIMESTAMPTZ,
    fencing_generation BIGINT      NOT NULL DEFAULT 0,
    created_at         TIMESTAMPTZ NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL,
    started_at         TIMESTAMPTZ,
    finished_at        TIMESTAMPTZ,
    CONSTRAINT pk_executions PRIMARY KEY (id),
    CONSTRAINT uq_executions__org_id_workspace_id_id UNIQUE (org_id, workspace_id, id),
    CONSTRAINT fk_executions__workflows
        FOREIGN KEY (org_id, workspace_id, workflow_id)
        REFERENCES workflows (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_executions__status CHECK (status IN (
        'created', 'running', 'paused', 'cancelling',
        'completed', 'failed', 'cancelled', 'timed_out'
    )),
    CONSTRAINT ck_executions__version CHECK (version >= 0),
    CONSTRAINT ck_executions__fencing_generation CHECK (fencing_generation >= 0)
);
CREATE INDEX ix_executions__org_id_workspace_id_created_at_id
    ON executions (org_id, workspace_id, created_at DESC, id COLLATE "C" DESC);
CREATE INDEX ix_executions__org_id_workspace_id_workflow_id_created_at_id
    ON executions (org_id, workspace_id, workflow_id, created_at DESC, id COLLATE "C" DESC);
CREATE INDEX ix_executions__status__active
    ON executions (status) WHERE status IN ('created', 'running', 'paused', 'cancelling');

CREATE TABLE execution_journal (
    org_id       TEXT   NOT NULL,
    workspace_id TEXT   NOT NULL,
    execution_id TEXT   NOT NULL,
    seq          BIGINT NOT NULL,
    payload      JSONB  NOT NULL,
    CONSTRAINT pk_execution_journal PRIMARY KEY (execution_id, seq),
    CONSTRAINT fk_execution_journal__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_execution_journal__seq CHECK (seq >= 0)
);

CREATE TABLE execution_contract_bundles (
    org_id             TEXT  NOT NULL,
    workspace_id       TEXT  NOT NULL,
    execution_id       TEXT  NOT NULL,
    bundle_id          BYTEA NOT NULL,
    executable_plan_id BYTEA NOT NULL,
    worker_flavor_id   BYTEA NOT NULL,
    record_format      TEXT  NOT NULL,
    record_bytes       BYTEA NOT NULL,
    commitment_format  TEXT  NOT NULL,
    commitment         BYTEA NOT NULL,
    CONSTRAINT pk_execution_contract_bundles PRIMARY KEY (execution_id),
    CONSTRAINT uq_execution_contract_bundles__bundle_id UNIQUE (bundle_id),
    CONSTRAINT fk_execution_contract_bundles__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_execution_contract_bundles__bundle_id_length
        CHECK (octet_length(bundle_id) = 16),
    CONSTRAINT ck_execution_contract_bundles__executable_plan_id_length
        CHECK (octet_length(executable_plan_id) = 32),
    CONSTRAINT ck_execution_contract_bundles__worker_flavor_id_length
        CHECK (octet_length(worker_flavor_id) = 32),
    CONSTRAINT ck_execution_contract_bundles__record_format
        CHECK (record_format IN ('v1_json', 'v2_json')),
    CONSTRAINT ck_execution_contract_bundles__record_bytes_length
        CHECK (octet_length(record_bytes) BETWEEN 1 AND 1048576),
    CONSTRAINT ck_execution_contract_bundles__commitment_format
        CHECK (commitment_format = 'v1_sha256'),
    CONSTRAINT ck_execution_contract_bundles__commitment_length
        CHECK (octet_length(commitment) = 32)
);

-- The exact plan/flavor pair an execution runs, and whether it still holds
-- the catalog revision live, in a rollback window, or released it.
CREATE TABLE execution_revision_references (
    org_id                       TEXT  NOT NULL,
    workspace_id                 TEXT  NOT NULL,
    execution_id                 TEXT  NOT NULL,
    execution_contract_bundle_id BYTEA NOT NULL,
    executable_plan_id           BYTEA NOT NULL,
    worker_flavor_id             BYTEA NOT NULL,
    reference_state              TEXT  NOT NULL,
    rollback_window_id           BYTEA,
    retain_until                 TIMESTAMPTZ,
    CONSTRAINT pk_execution_revision_references PRIMARY KEY (execution_id),
    CONSTRAINT fk_execution_revision_references__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    -- A reference, not ownership: a revision an execution holds cannot go.
    CONSTRAINT fk_execution_revision_references__executable_plan_revisions
        FOREIGN KEY (executable_plan_id, worker_flavor_id)
        REFERENCES executable_plan_revisions (executable_plan_id, worker_flavor_id)
        ON DELETE RESTRICT,
    CONSTRAINT ck_execution_revision_references__execution_contract_bundle_len
        CHECK (octet_length(execution_contract_bundle_id) = 16),
    CONSTRAINT ck_execution_revision_references__executable_plan_id_length
        CHECK (octet_length(executable_plan_id) = 32),
    CONSTRAINT ck_execution_revision_references__worker_flavor_id_length
        CHECK (octet_length(worker_flavor_id) = 32),
    CONSTRAINT ck_execution_revision_references__rollback_window_id_length
        CHECK (octet_length(rollback_window_id) = 16),
    CONSTRAINT ck_execution_revision_references__reference_state
        CHECK (reference_state IN ('live', 'rollback', 'released')),
    CONSTRAINT ck_execution_revision_references__state_shape CHECK (
        (reference_state = 'live' AND rollback_window_id IS NULL AND retain_until IS NULL)
        OR (reference_state = 'rollback'
            AND rollback_window_id IS NOT NULL AND retain_until IS NOT NULL)
        OR (reference_state = 'released'
            AND (rollback_window_id IS NULL) = (retain_until IS NULL))
    )
);
CREATE INDEX ix_execution_revision_references__executable_plan_id__live
    ON execution_revision_references (executable_plan_id) WHERE reference_state = 'live';
CREATE INDEX ix_execution_revision_references__worker_flavor_execution__live
    ON execution_revision_references (worker_flavor_id, execution_id)
    WHERE reference_state = 'live';
CREATE INDEX ix_execution_revision_references__executable_plan__rollback
    ON execution_revision_references (executable_plan_id, retain_until)
    WHERE reference_state = 'rollback';
CREATE INDEX ix_execution_revision_references__worker_flavor__rollback
    ON execution_revision_references (worker_flavor_id, retain_until)
    WHERE reference_state = 'rollback';

-- The last accepted execution turn and the queue row that carried it.
CREATE TABLE execution_turn_acceptances (
    org_id                           TEXT   NOT NULL,
    workspace_id                     TEXT   NOT NULL,
    execution_id                     TEXT   NOT NULL,
    last_accepted_fencing_generation BIGINT NOT NULL,
    source_kind                      TEXT   NOT NULL,
    source_queue_id                  BYTEA  NOT NULL,
    CONSTRAINT pk_execution_turn_acceptances PRIMARY KEY (execution_id),
    CONSTRAINT fk_execution_turn_acceptances__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_execution_turn_acceptances__last_accepted_fencing_generation
        CHECK (last_accepted_fencing_generation > 0),
    CONSTRAINT ck_execution_turn_acceptances__source_kind
        CHECK (source_kind IN ('Job', 'ControlStart', 'ControlResume', 'ControlRestart')),
    CONSTRAINT ck_execution_turn_acceptances__source_queue_id_length
        CHECK (octet_length(source_queue_id) = 16)
);

-- Exactly-once receipts for control decisions observed against an execution.
CREATE TABLE execution_control_observation_receipts (
    org_id             TEXT   NOT NULL,
    workspace_id       TEXT   NOT NULL,
    execution_id       TEXT   NOT NULL,
    source_kind        TEXT   NOT NULL,
    source_queue_id    BYTEA  NOT NULL,
    source_generation  BIGINT NOT NULL,
    decision_key       TEXT   NOT NULL,
    outcome            TEXT   NOT NULL,
    expected_flavor_id BYTEA,
    actual_flavor_id   BYTEA,
    CONSTRAINT pk_execution_control_observation_receipts PRIMARY KEY (
        org_id, workspace_id, execution_id, source_kind, source_queue_id,
        source_generation, decision_key, outcome
    ),
    CONSTRAINT fk_execution_control_observation_receipts__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_execution_control_observation_receipts__source_kind CHECK (
        source_kind IN ('control_queue', 'job_dispatch', 'control_accepted_turn', 'job_accepted_turn')
    ),
    CONSTRAINT ck_execution_control_observation_receipts__source_queue_length
        CHECK (octet_length(source_queue_id) = 16),
    CONSTRAINT ck_execution_control_observation_receipts__source_generation
        CHECK (source_generation > 0),
    CONSTRAINT ck_execution_control_observation_receipts__outcome
        CHECK (outcome IN ('fenced', 'flavor-mismatch', 'deferred', 'throttled')),
    CONSTRAINT ck_execution_control_observation_receipts__flavor_shape CHECK (
        (outcome = 'flavor-mismatch'
            AND octet_length(expected_flavor_id) = 32
            AND octet_length(actual_flavor_id) = 32
            AND expected_flavor_id <> actual_flavor_id)
        OR (outcome <> 'flavor-mismatch'
            AND expected_flavor_id IS NULL AND actual_flavor_id IS NULL)
    )
);

-- Durable per-iteration state of a stateful node, keyed by the exact action
-- version that wrote it.
CREATE TABLE iteration_checkpoints (
    org_id                TEXT        NOT NULL,
    workspace_id          TEXT        NOT NULL,
    execution_id          TEXT        NOT NULL,
    node_key              TEXT        NOT NULL,
    action_key            TEXT        NOT NULL,
    action_version        TEXT        NOT NULL,
    action_version_digest BYTEA       NOT NULL,
    iteration             INTEGER     NOT NULL,
    state                 BYTEA       NOT NULL,
    state_digest          BYTEA       NOT NULL,
    resume_delay_ms       BIGINT,
    attested_positions    INTEGER     NOT NULL,
    attempt_generation    BIGINT      NOT NULL,
    fencing_generation    BIGINT      NOT NULL,
    written_at            TIMESTAMPTZ NOT NULL,
    CONSTRAINT pk_iteration_checkpoints PRIMARY KEY (
        org_id, workspace_id, execution_id, node_key, action_key, action_version_digest
    ),
    CONSTRAINT fk_iteration_checkpoints__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_iteration_checkpoints__node_key CHECK (length(node_key) > 0),
    CONSTRAINT ck_iteration_checkpoints__action_key CHECK (length(action_key) > 0),
    CONSTRAINT ck_iteration_checkpoints__action_version CHECK (length(action_version) > 0),
    CONSTRAINT ck_iteration_checkpoints__action_version_digest_length
        CHECK (octet_length(action_version_digest) = 32),
    CONSTRAINT ck_iteration_checkpoints__iteration CHECK (iteration BETWEEN 1 AND 10000),
    CONSTRAINT ck_iteration_checkpoints__state_length CHECK (octet_length(state) <= 1048576),
    CONSTRAINT ck_iteration_checkpoints__state_digest_length
        CHECK (octet_length(state_digest) = 32),
    CONSTRAINT ck_iteration_checkpoints__resume_delay_ms CHECK (resume_delay_ms >= 0),
    CONSTRAINT ck_iteration_checkpoints__attested_positions CHECK (attested_positions >= 0),
    CONSTRAINT ck_iteration_checkpoints__attempt_generation CHECK (attempt_generation >= 0),
    CONSTRAINT ck_iteration_checkpoints__fencing_generation CHECK (fencing_generation >= 0)
);

-- Tokens that resume a waiting node (webhook callback, approval); the hash is
-- the only stored form of the token.
CREATE TABLE resume_tokens (
    org_id         TEXT        NOT NULL,
    workspace_id   TEXT        NOT NULL,
    execution_id   TEXT        NOT NULL,
    token_hash     BYTEA       NOT NULL,
    node_key       TEXT        NOT NULL,
    wait_kind      TEXT        NOT NULL,
    callback_label TEXT        NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL,
    expires_at     TIMESTAMPTZ,
    CONSTRAINT pk_resume_tokens PRIMARY KEY (token_hash),
    CONSTRAINT uq_resume_tokens__execution_id_node_key UNIQUE (execution_id, node_key),
    CONSTRAINT fk_resume_tokens__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_resume_tokens__token_hash_length CHECK (octet_length(token_hash) = 32),
    CONSTRAINT ck_resume_tokens__wait_kind CHECK (wait_kind IN ('webhook', 'approval'))
);
CREATE INDEX ix_resume_tokens__org_id_workspace_id_execution_id
    ON resume_tokens (org_id, workspace_id, execution_id);

-- One row per accepted start key, written in the same transaction as the
-- execution and its Start control row.
CREATE TABLE start_key_reservations (
    org_id              TEXT        NOT NULL,
    workspace_id        TEXT        NOT NULL,
    start_key           TEXT        NOT NULL,
    fingerprint_version INTEGER     NOT NULL,
    fingerprint         BYTEA       NOT NULL,
    execution_id        TEXT        NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL,
    CONSTRAINT pk_start_key_reservations PRIMARY KEY (org_id, workspace_id, start_key),
    -- Deferred: the key is reserved first, so a replay finds it before the
    -- execution row exists; the execution lands in the same transaction.
    CONSTRAINT fk_start_key_reservations__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT ck_start_key_reservations__fingerprint_version CHECK (fingerprint_version >= 0),
    CONSTRAINT ck_start_key_reservations__fingerprint_length
        CHECK (octet_length(fingerprint) = 32)
);
CREATE INDEX ix_start_key_reservations__created_at ON start_key_reservations (created_at);

-- Node-attempt idempotency marks: the first writer of an attempt wins.
CREATE TABLE idempotency_marks (
    org_id       TEXT   NOT NULL,
    workspace_id TEXT   NOT NULL,
    execution_id TEXT   NOT NULL,
    node_key     TEXT   NOT NULL,
    attempt      BIGINT NOT NULL,
    CONSTRAINT pk_idempotency_marks
        PRIMARY KEY (org_id, workspace_id, execution_id, node_key, attempt),
    CONSTRAINT fk_idempotency_marks__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_idempotency_marks__attempt CHECK (attempt >= 0)
);

-- ── Effect operations ────────────────────────────────────────────────────
--
-- One slot per side effect a node attempts, and the protocol record that
-- drives it to a recorded outcome.

CREATE TABLE operation_ledger (
    org_id                TEXT        NOT NULL,
    workspace_id          TEXT        NOT NULL,
    execution_id          TEXT        NOT NULL,
    slot_id               BYTEA       NOT NULL,
    node_key              TEXT        NOT NULL,
    occurrence            TEXT        NOT NULL,
    attempt_generation    BIGINT      NOT NULL,
    fingerprint_version   INTEGER     NOT NULL,
    fingerprint           BYTEA       NOT NULL,
    destination           TEXT        NOT NULL,
    operation_id          BYTEA       NOT NULL,
    state                 TEXT        NOT NULL,
    prepared_at           TIMESTAMPTZ NOT NULL,
    outcome_at            TIMESTAMPTZ,
    adjudication_evidence TEXT,
    adjudicated_at        TIMESTAMPTZ,
    CONSTRAINT pk_operation_ledger PRIMARY KEY (slot_id),
    CONSTRAINT uq_operation_ledger__operation_id UNIQUE (operation_id),
    CONSTRAINT uq_operation_ledger__execution_id_node_key_occurrence
        UNIQUE (org_id, workspace_id, execution_id, node_key, occurrence),
    CONSTRAINT uq_operation_ledger__org_id_workspace_id_execution_id_slot_id
        UNIQUE (org_id, workspace_id, execution_id, slot_id),
    CONSTRAINT fk_operation_ledger__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_operation_ledger__slot_id_length CHECK (octet_length(slot_id) = 16),
    CONSTRAINT ck_operation_ledger__node_key CHECK (length(node_key) > 0),
    CONSTRAINT ck_operation_ledger__occurrence CHECK (length(occurrence) > 0),
    CONSTRAINT ck_operation_ledger__attempt_generation CHECK (attempt_generation >= 0),
    CONSTRAINT ck_operation_ledger__fingerprint_version CHECK (fingerprint_version >= 0),
    CONSTRAINT ck_operation_ledger__fingerprint_length CHECK (octet_length(fingerprint) = 32),
    CONSTRAINT ck_operation_ledger__destination
        CHECK (destination IN ('stable_key', 'reconcilable', 'opaque')),
    CONSTRAINT ck_operation_ledger__operation_id_length CHECK (octet_length(operation_id) = 16),
    CONSTRAINT ck_operation_ledger__state
        CHECK (state IN ('prepared', 'succeeded', 'failed', 'outcome_unknown')),
    CONSTRAINT ck_operation_ledger__outcome_shape CHECK (
        (state = 'prepared' AND outcome_at IS NULL)
        OR (state <> 'prepared' AND outcome_at IS NOT NULL)
    ),
    CONSTRAINT ck_operation_ledger__adjudication_evidence
        CHECK (length(adjudication_evidence) > 0),
    CONSTRAINT ck_operation_ledger__adjudication_shape CHECK (
        (adjudication_evidence IS NULL AND adjudicated_at IS NULL)
        OR (adjudication_evidence IS NOT NULL AND adjudicated_at IS NOT NULL
            AND state IN ('succeeded', 'failed'))
    )
);
CREATE INDEX ix_operation_ledger__prepared_at__unresolved
    ON operation_ledger (org_id, workspace_id, prepared_at)
    WHERE state IN ('prepared', 'outcome_unknown');

CREATE TABLE operation_protocol_records (
    org_id       TEXT  NOT NULL,
    workspace_id TEXT  NOT NULL,
    execution_id TEXT  NOT NULL,
    slot_id      BYTEA NOT NULL,
    payload      TEXT  NOT NULL,
    CONSTRAINT pk_operation_protocol_records PRIMARY KEY (slot_id),
    CONSTRAINT fk_operation_protocol_records__operation_ledger
        FOREIGN KEY (org_id, workspace_id, execution_id, slot_id)
        REFERENCES operation_ledger (org_id, workspace_id, execution_id, slot_id)
        ON DELETE CASCADE,
    CONSTRAINT ck_operation_protocol_records__payload_length
        CHECK (octet_length(payload) BETWEEN 1 AND 5300000)
);
