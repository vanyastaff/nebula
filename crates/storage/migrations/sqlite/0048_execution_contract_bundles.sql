-- Additive owner records. Legacy executions acquire no fabricated contracts.
CREATE UNIQUE INDEX port_executions_identity_scope ON port_executions(id, workspace_id, org_id);
CREATE TABLE port_execution_contract_bundles (
    execution_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    org_id TEXT NOT NULL,
    bundle_id BLOB NOT NULL UNIQUE CHECK(typeof(bundle_id) = 'blob' AND length(bundle_id) = 16),
    executable_plan_id BLOB NOT NULL CHECK(typeof(executable_plan_id) = 'blob' AND length(executable_plan_id) = 32),
    worker_flavor_id BLOB NOT NULL CHECK(typeof(worker_flavor_id) = 'blob' AND length(worker_flavor_id) = 32),
    record_format TEXT NOT NULL CHECK(record_format = 'v1_json'),
    record_bytes BLOB NOT NULL CHECK(typeof(record_bytes) = 'blob' AND length(record_bytes) BETWEEN 1 AND 1048576),
    commitment_format TEXT NOT NULL CHECK(commitment_format = 'v1_sha256'),
    commitment BLOB NOT NULL CHECK(typeof(commitment) = 'blob' AND length(commitment) = 32),
    FOREIGN KEY(execution_id, workspace_id, org_id) REFERENCES port_executions(id, workspace_id, org_id) ON DELETE RESTRICT
);
