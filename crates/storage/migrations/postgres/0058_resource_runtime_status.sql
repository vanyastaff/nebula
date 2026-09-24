-- Cross-process runtime status of stored resources. Workers renew a liveness
-- heartbeat and publish one lifecycle snapshot per activated row; readers
-- trust a snapshot only while its worker's heartbeat is unexpired, so a
-- crashed worker's status disappears without anyone deleting it. Snapshots
-- carry lifecycle state only, never config or credential material. Both
-- tables start empty and reference no aggregate, so there are no foreign keys.
CREATE TABLE port_worker_heartbeats (
    worker_id TEXT PRIMARY KEY CHECK (octet_length(worker_id) BETWEEN 1 AND 128),
    expires_at_ms BIGINT NOT NULL
);

CREATE TABLE port_resource_status (
    workspace_id TEXT NOT NULL,
    org_id TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    worker_id TEXT NOT NULL CHECK (octet_length(worker_id) BETWEEN 1 AND 128),
    phase TEXT NOT NULL CHECK (phase IN ('initializing', 'ready', 'reloading', 'draining', 'shutting_down', 'failed', 'unknown')),
    healthy BOOLEAN NOT NULL,
    accepting BOOLEAN NOT NULL,
    row_version BIGINT NOT NULL CHECK (row_version >= 0),
    PRIMARY KEY (workspace_id, org_id, resource_id, worker_id)
);
CREATE INDEX port_resource_status_worker ON port_resource_status (worker_id);
