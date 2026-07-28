-- Dormant exact-revision retention schema. Runtime activation and mutation
-- remain outside this migration: this file establishes only closed, durable
-- data shapes and their referential constraints.

CREATE TABLE port_worker_flavor_revisions (
    worker_flavor_id BYTEA PRIMARY KEY
        CHECK (octet_length(worker_flavor_id) = 32),
    record_format TEXT NOT NULL
        CHECK (record_format = 'v1_json'),
    lifecycle TEXT NOT NULL
        CHECK (lifecycle IN ('active', 'draining', 'deleted')),
    record_bytes BYTEA,
    CONSTRAINT port_worker_flavor_revision_record_shape
        CHECK (
            (
                lifecycle IN ('active', 'draining')
                AND record_bytes IS NOT NULL
                AND octet_length(record_bytes) > 0
            )
            OR (lifecycle = 'deleted' AND record_bytes IS NULL)
        )
);

CREATE TABLE port_executable_plan_revisions (
    executable_plan_id BYTEA PRIMARY KEY
        CHECK (octet_length(executable_plan_id) = 32),
    worker_flavor_id BYTEA NOT NULL
        CHECK (octet_length(worker_flavor_id) = 32),
    record_format TEXT NOT NULL
        CHECK (record_format = 'graph_v1_json'),
    lifecycle TEXT NOT NULL
        CHECK (lifecycle IN ('active', 'draining', 'deleted')),
    record_bytes BYTEA,
    CONSTRAINT port_executable_plan_revision_record_shape
        CHECK (
            (
                lifecycle IN ('active', 'draining')
                AND record_bytes IS NOT NULL
                AND octet_length(record_bytes) > 0
            )
            OR (lifecycle = 'deleted' AND record_bytes IS NULL)
        ),
    CONSTRAINT port_executable_plan_revision_flavor_fk
        FOREIGN KEY (worker_flavor_id)
        REFERENCES port_worker_flavor_revisions (worker_flavor_id)
        ON DELETE RESTRICT,
    CONSTRAINT port_executable_plan_revision_exact_pair
        UNIQUE (executable_plan_id, worker_flavor_id)
);

CREATE TABLE port_execution_revision_refs (
    execution_id TEXT PRIMARY KEY
        CHECK (
            octet_length(execution_id) = 30
            AND execution_id COLLATE "C"
                ~ '^exe_[0-7][0-9A-HJKMNP-TV-Z]{25}$'
        ),
    execution_contract_bundle_id BYTEA NOT NULL
        CHECK (octet_length(execution_contract_bundle_id) = 16),
    executable_plan_id BYTEA NOT NULL
        CHECK (octet_length(executable_plan_id) = 32),
    worker_flavor_id BYTEA NOT NULL
        CHECK (octet_length(worker_flavor_id) = 32),
    reference_state TEXT NOT NULL
        CHECK (reference_state IN ('live', 'rollback', 'released')),
    rollback_window_id BYTEA
        CHECK (
            rollback_window_id IS NULL
            OR octet_length(rollback_window_id) = 16
        ),
    retain_until_ms BIGINT,
    CONSTRAINT port_execution_revision_ref_state_shape
        CHECK (
            (
                reference_state = 'live'
                AND rollback_window_id IS NULL
                AND retain_until_ms IS NULL
            )
            OR (
                reference_state = 'rollback'
                AND rollback_window_id IS NOT NULL
                AND retain_until_ms IS NOT NULL
            )
            OR (
                reference_state = 'released'
                AND (
                    (
                        rollback_window_id IS NULL
                        AND retain_until_ms IS NULL
                    )
                    OR (
                        rollback_window_id IS NOT NULL
                        AND retain_until_ms IS NOT NULL
                    )
                )
            )
        ),
    CONSTRAINT port_execution_revision_ref_execution_fk
        FOREIGN KEY (execution_id)
        REFERENCES port_executions (id)
        ON DELETE RESTRICT,
    CONSTRAINT port_execution_revision_ref_exact_plan_flavor_fk
        FOREIGN KEY (executable_plan_id, worker_flavor_id)
        REFERENCES port_executable_plan_revisions (
            executable_plan_id,
            worker_flavor_id
        )
        ON DELETE RESTRICT
);

CREATE INDEX idx_port_executable_plan_revisions_worker_flavor
    ON port_executable_plan_revisions (worker_flavor_id)
    WHERE lifecycle <> 'deleted';

CREATE INDEX idx_port_execution_revision_refs_live_plan
    ON port_execution_revision_refs (executable_plan_id)
    WHERE reference_state = 'live';

CREATE INDEX idx_port_execution_revision_refs_live_flavor
    ON port_execution_revision_refs (worker_flavor_id)
    WHERE reference_state = 'live';

CREATE INDEX idx_port_execution_revision_refs_rollback_plan
    ON port_execution_revision_refs (executable_plan_id, retain_until_ms)
    WHERE reference_state = 'rollback';

CREATE INDEX idx_port_execution_revision_refs_rollback_flavor
    ON port_execution_revision_refs (worker_flavor_id, retain_until_ms)
    WHERE reference_state = 'rollback';
