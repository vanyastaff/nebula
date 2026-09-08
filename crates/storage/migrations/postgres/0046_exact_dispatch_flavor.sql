-- Aggregate-neutral only on an empty dispatch queue. Never manufacture a flavor
-- for a legacy row or rewrite terminal/live rows to make migration succeed.
LOCK TABLE port_job_dispatch_queue IN ACCESS EXCLUSIVE MODE;
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM port_job_dispatch_queue) THEN
        RAISE EXCEPTION 'exact dispatch flavor migration requires an empty dispatch queue';
    END IF;
END
$$;
ALTER TABLE port_job_dispatch_queue
    ADD COLUMN required_worker_flavor_id BYTEA NOT NULL
        CHECK (octet_length(required_worker_flavor_id) = 32);
CREATE INDEX idx_port_job_dispatch_queue_flavor_status_key
    ON port_job_dispatch_queue (required_worker_flavor_id, status, required_plugin_key, id);
