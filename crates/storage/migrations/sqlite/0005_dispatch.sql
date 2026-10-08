-- Dispatch: the queues that carry an execution's work to its runners, the
-- trigger rows that start executions, the reservations that make a trigger
-- delivery start at most one execution, and the webhook activations that
-- route an incoming request to its trigger.
--
-- Instants are INTEGER microseconds since the Unix epoch.

-- ── Execution queues ─────────────────────────────────────────────────────
--
-- Both queues are outboxes of one execution: a row names the execution it
-- carries work for and is purged with it. Ids are raw 16-byte ULIDs; a claim
-- mints `claim_generation`, which fences every acknowledgement (never
-- decremented or reused). `processed_by` is observability only.

-- Accepted lifecycle commands (Start, Cancel, ...), drained by the control
-- consumer.
CREATE TABLE execution_control_queue (
    org_id           TEXT    NOT NULL,
    workspace_id     TEXT    NOT NULL,
    execution_id     TEXT    NOT NULL,
    id               BLOB    NOT NULL,
    command          TEXT    NOT NULL,
    status           TEXT    NOT NULL DEFAULT 'Pending',
    resume_target    TEXT,
    w3c_traceparent  TEXT,
    reclaim_count    INTEGER NOT NULL DEFAULT 0,
    claim_generation INTEGER NOT NULL DEFAULT 0,
    processed_by     BLOB,
    processed_at     INTEGER,
    error_message    TEXT,
    CONSTRAINT pk_execution_control_queue PRIMARY KEY (id),
    CONSTRAINT fk_execution_control_queue__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_execution_control_queue__id_length
        CHECK (typeof(id) = 'blob' AND length(id) = 16),
    CONSTRAINT ck_execution_control_queue__command
        CHECK (command IN ('Start', 'Cancel', 'Terminate', 'Resume', 'Restart')),
    CONSTRAINT ck_execution_control_queue__status
        CHECK (status IN ('Pending', 'Processing', 'Completed', 'Failed')),
    CONSTRAINT ck_execution_control_queue__resume_target_json
        CHECK (json_valid(resume_target)),
    CONSTRAINT ck_execution_control_queue__reclaim_count CHECK (reclaim_count >= 0),
    CONSTRAINT ck_execution_control_queue__claim_generation CHECK (claim_generation >= 0),
    CONSTRAINT ck_execution_control_queue__processed_by_length
        CHECK (typeof(processed_by) IN ('null', 'blob') AND length(processed_by) = 16)
);
CREATE INDEX ix_execution_control_queue__id__pending
    ON execution_control_queue (id) WHERE status = 'Pending';
CREATE INDEX ix_execution_control_queue__processed_at__processing
    ON execution_control_queue (processed_at) WHERE status = 'Processing';
CREATE INDEX ix_execution_control_queue__org_id_workspace_id_execution_id
    ON execution_control_queue (org_id, workspace_id, execution_id);

-- Capability-routed jobs: a runner claims a row only when it advertises the
-- exact worker flavor and every required plugin. The flavor is a routing
-- predicate the runner matches against its own identity, not a reference
-- into the revision catalog (as the observation receipts' flavor ids).
-- `processed_at` is the claim instant while Processing and the terminal
-- instant once Dispatched or Failed, which retention measures from.
CREATE TABLE job_dispatch_queue (
    org_id                    TEXT    NOT NULL,
    workspace_id              TEXT    NOT NULL,
    execution_id              TEXT    NOT NULL,
    id                        BLOB    NOT NULL,
    command                   TEXT    NOT NULL,
    status                    TEXT    NOT NULL DEFAULT 'Pending',
    payload                   TEXT    NOT NULL,
    event_id                  TEXT,
    required_worker_flavor_id BLOB    NOT NULL,
    required_plugin_key       TEXT    NOT NULL,
    required_plugins          TEXT    NOT NULL,
    w3c_traceparent           TEXT,
    reclaim_count             INTEGER NOT NULL DEFAULT 0,
    claim_generation          INTEGER NOT NULL DEFAULT 0,
    processed_by              BLOB,
    processed_at              INTEGER,
    error_message             TEXT,
    CONSTRAINT pk_job_dispatch_queue PRIMARY KEY (id),
    CONSTRAINT fk_job_dispatch_queue__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_job_dispatch_queue__id_length
        CHECK (typeof(id) = 'blob' AND length(id) = 16),
    CONSTRAINT ck_job_dispatch_queue__command
        CHECK (command IN ('Start', 'Cancel', 'Terminate', 'Resume', 'Restart')),
    CONSTRAINT ck_job_dispatch_queue__status
        CHECK (status IN ('Pending', 'Processing', 'Dispatched', 'Failed')),
    CONSTRAINT ck_job_dispatch_queue__payload_json CHECK (json_valid(payload)),
    CONSTRAINT ck_job_dispatch_queue__required_worker_flavor_id_length CHECK (
        typeof(required_worker_flavor_id) = 'blob' AND length(required_worker_flavor_id) = 32
    ),
    CONSTRAINT ck_job_dispatch_queue__required_plugins_json
        CHECK (json_valid(required_plugins)),
    CONSTRAINT ck_job_dispatch_queue__required_plugins_array
        CHECK (json_type(required_plugins) = 'array'),
    CONSTRAINT ck_job_dispatch_queue__reclaim_count CHECK (reclaim_count >= 0),
    CONSTRAINT ck_job_dispatch_queue__claim_generation CHECK (claim_generation >= 0),
    CONSTRAINT ck_job_dispatch_queue__processed_by_length
        CHECK (typeof(processed_by) IN ('null', 'blob') AND length(processed_by) = 16)
);
CREATE INDEX ix_job_dispatch_queue__required_worker_flavor__pending
    ON job_dispatch_queue (required_worker_flavor_id, required_plugin_key, id)
    WHERE status = 'Pending';
CREATE INDEX ix_job_dispatch_queue__status_processed_at
    ON job_dispatch_queue (status, processed_at);
CREATE INDEX ix_job_dispatch_queue__org_id_workspace_id_execution_id
    ON job_dispatch_queue (org_id, workspace_id, execution_id);

-- ── Triggers ─────────────────────────────────────────────────────────────
--
-- A trigger belongs to the workflow it starts and is purged with it.

CREATE TABLE triggers (
    org_id       TEXT    NOT NULL,
    workspace_id TEXT    NOT NULL,
    id           TEXT    NOT NULL,
    workflow_id  TEXT    NOT NULL,
    slug         TEXT    NOT NULL,
    display_name TEXT    NOT NULL,
    kind         TEXT    NOT NULL,
    config       TEXT    NOT NULL,
    state        TEXT    NOT NULL,
    run_as       TEXT,
    webhook_path TEXT,
    created_at   INTEGER NOT NULL,
    created_by   TEXT    NOT NULL,
    version      INTEGER NOT NULL DEFAULT 0,
    deleted_at   INTEGER,
    CONSTRAINT pk_triggers PRIMARY KEY (org_id, workspace_id, id),
    CONSTRAINT fk_triggers__workflows
        FOREIGN KEY (org_id, workspace_id, workflow_id)
        REFERENCES workflows (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_triggers__kind
        CHECK (kind IN ('manual', 'cron', 'webhook', 'event', 'polling')),
    CONSTRAINT ck_triggers__config_json CHECK (json_valid(config)),
    CONSTRAINT ck_triggers__state CHECK (state IN ('active', 'paused', 'archived')),
    CONSTRAINT ck_triggers__version CHECK (version >= 0)
);
CREATE INDEX ix_triggers__org_id_workspace_id_workflow_id
    ON triggers (org_id, workspace_id, workflow_id);

-- One row per trigger delivery that started an execution: a redelivered
-- event replays to the execution it already started. `trigger_id` is the
-- source's own trigger identity (a binding node key, a fan-out key), not a
-- `triggers` row, so the reservation belongs to the execution it started.
CREATE TABLE trigger_start_reservations (
    org_id       TEXT    NOT NULL,
    workspace_id TEXT    NOT NULL,
    trigger_id   TEXT    NOT NULL,
    event_id     TEXT    NOT NULL,
    execution_id TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    CONSTRAINT pk_trigger_start_reservations
        PRIMARY KEY (org_id, workspace_id, trigger_id, event_id),
    -- Deferred: the delivery is reserved first, so a replay finds it before
    -- the execution row exists; the execution lands in the same transaction.
    CONSTRAINT fk_trigger_start_reservations__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE
        DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX ix_trigger_start_reservations__org_id_workspace_id_execution_id
    ON trigger_start_reservations (org_id, workspace_id, execution_id);

-- Incoming webhook routes, by tenant-unique slug and by capability-token
-- hash. `trigger_id` is the dispatch routing key (the workflow's trigger
-- binding node key); `spec_trigger_id` is the `triggers` row the activation
-- was built from and owns it. A NULL `token_hash` has no token assigned.
CREATE TABLE webhook_activations (
    org_id          TEXT    NOT NULL,
    workspace_id    TEXT    NOT NULL,
    slug            TEXT    NOT NULL,
    trigger_id      TEXT    NOT NULL,
    spec_trigger_id TEXT,
    workflow_id     TEXT,
    active          INTEGER NOT NULL DEFAULT 1,
    webhook_mode    TEXT    NOT NULL DEFAULT 'test',
    token_hash      BLOB,
    CONSTRAINT pk_webhook_activations PRIMARY KEY (org_id, workspace_id, slug),
    CONSTRAINT uq_webhook_activations__token_hash UNIQUE (token_hash),
    CONSTRAINT fk_webhook_activations__triggers
        FOREIGN KEY (org_id, workspace_id, spec_trigger_id)
        REFERENCES triggers (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT fk_webhook_activations__workflows
        FOREIGN KEY (org_id, workspace_id, workflow_id)
        REFERENCES workflows (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_webhook_activations__active CHECK (active IN (0, 1)),
    CONSTRAINT ck_webhook_activations__webhook_mode CHECK (webhook_mode IN ('test', 'prod')),
    CONSTRAINT ck_webhook_activations__token_hash_length
        CHECK (typeof(token_hash) IN ('null', 'blob') AND length(token_hash) = 32)
);
CREATE INDEX ix_webhook_activations__org_id_workspace_id_trigger_id
    ON webhook_activations (org_id, workspace_id, trigger_id);
CREATE INDEX ix_webhook_activations__org_id_workspace_id_spec_trigger_id
    ON webhook_activations (org_id, workspace_id, spec_trigger_id);
CREATE INDEX ix_webhook_activations__org_id_workspace_id_workflow_id
    ON webhook_activations (org_id, workspace_id, workflow_id);
