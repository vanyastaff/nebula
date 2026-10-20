-- The job-dispatch channel was removed as an unused second control path:
-- starts, resumes and restarts flow through the durable control queue, and
-- no adapter reads or writes `job_dispatch_queue` any more. Its indexes and
-- constraints go with it. Observation receipts keep their historical
-- `job_dispatch` source kind, which does not reference this table.
DROP TABLE job_dispatch_queue;
