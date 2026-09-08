-- Additive protocol records: legacy ledger rows remain unclassified and cannot invoke.
CREATE UNIQUE INDEX port_operation_ledger_owner_identity
    ON port_operation_ledger (slot_id, workspace_id, org_id, execution_id);

CREATE TABLE port_operation_protocol (
    slot_id BYTEA PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    org_id TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    payload TEXT NOT NULL CHECK (octet_length(payload) BETWEEN 1 AND 5300000),
    FOREIGN KEY (slot_id, workspace_id, org_id, execution_id)
        REFERENCES port_operation_ledger (slot_id, workspace_id, org_id, execution_id),
    FOREIGN KEY (execution_id, workspace_id, org_id)
        REFERENCES port_executions (id, workspace_id, org_id)
);
