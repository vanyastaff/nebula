-- A cancel request for an execution whose lease is held on another worker.
-- The control consumer records it here; the lease holder observes it from its
-- heartbeat and any later holder observes it on acquire. Set once, never
-- cleared, and outside the version/fencing gate of the aggregate's commits.
ALTER TABLE executions ADD COLUMN cancel_requested_at TIMESTAMPTZ;
