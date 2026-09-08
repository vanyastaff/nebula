-- Aggregate-neutral only on an empty dispatch queue. Legacy rows cannot be
-- assigned an exact flavor from a plugin key or target SHA. Reject every legacy
-- row, including terminal rows, without rewriting any aggregate or claim.
CREATE TABLE dispatch_flavor_migration_preflight (
    empty_queue INTEGER NOT NULL CHECK (empty_queue = 1)
);
INSERT INTO dispatch_flavor_migration_preflight
    SELECT NOT EXISTS (SELECT 1 FROM port_job_dispatch_queue);
DROP TABLE dispatch_flavor_migration_preflight;

ALTER TABLE port_job_dispatch_queue
    ADD COLUMN required_worker_flavor_id BLOB NOT NULL
        CHECK (typeof(required_worker_flavor_id) = 'blob'
            AND length(required_worker_flavor_id) = 32);
CREATE INDEX idx_port_job_dispatch_queue_flavor_status_key
    ON port_job_dispatch_queue (required_worker_flavor_id, status, required_plugin_key, id);
