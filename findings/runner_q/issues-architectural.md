# RunnerQ — Architectural Issues

## Closed issues (most architecturally significant)

### #70 — "Document scale limitations: scheduler–executor coupling, single queue, storage bottleneck"
- **URL:** https://github.com/alob-mtc/runnerq/issues/70
- **Reactions:** 0
- **Status:** Closed
- **Summary:** Acknowledges that `runnerq_activities` is a single table bottleneck; no partitioning, no horizontal shard. The scheduler and executor share the same table, limiting independent scaling.

### #67 — "Queue starvation: retrying activities never complete when queue is busy"
- **URL:** https://github.com/alob-mtc/runnerq/issues/67
- **Reactions:** 1 (EYES)
- **Status:** Closed
- **Summary:** When new `pending` activities constantly arrived, `retrying` activities were never picked up because the dequeue query preferred new items. Fixed by age-weighted fair scheduling (retry_count DESC tiebreaker).

### #36 — "Event stream uses in-memory channel, breaking cross-process event visibility"
- **URL:** https://github.com/alob-mtc/runnerq/issues/36
- **Reactions:** 0
- **Status:** Closed
- **Summary:** Original SSE implementation used a Tokio broadcast channel inside a single process. Multiple instances each had their own isolated event streams. Fixed by adopting PostgreSQL LISTEN/NOTIFY and Redis Streams.

### #33 — "Fix duplicate reaper processing in multi-node deployments"
- **URL:** https://github.com/alob-mtc/runnerq/issues/33
- **Reactions:** 0
- **Status:** Closed
- **Summary:** In multi-node deployments, multiple reaper instances could each reclaim the same expired lease, causing double-processing. Fixed via `FOR UPDATE SKIP LOCKED` in the reaper query.

### #32 — "Fix duplicate scheduled activity processing in multi-node deployments"
- **URL:** https://github.com/alob-mtc/runnerq/issues/32
- **Reactions:** 0
- **Status:** Closed
- **Summary:** Same category as #33 but for the scheduled activities processor. Fixed by same `SKIP LOCKED` approach.

### #17 — "Prevent activity loss: implement crash-safe claim → process → ack with auto requeue"
- **URL:** https://github.com/alob-mtc/runnerq/issues/17
- **Reactions:** 0
- **Status:** Closed
- **Summary:** Foundational issue requesting the lease-based at-least-once delivery guarantee. Implemented via the reaper processor.

## Open issues (architecturally relevant)

### #72 — "Redis backend: apply Postgres scale improvements (scheduling, activity type filtering)"
- **URL:** https://github.com/alob-mtc/runnerq/issues/72
- **Reactions:** 0
- **Status:** Open
- **Summary:** Redis backend ignores activity_types filter in dequeue(). Workload isolation is PostgreSQL-only. Redis also lacks native scheduling (separate polling loop required).

### #64 — "feat: Implement MongoDBBackend"
- **URL:** https://github.com/alob-mtc/runnerq/issues/64
- **Reactions:** 1 (EYES)
- **Status:** Open
- **Summary:** Community request for MongoDB storage backend.

### #58 — "feat: Add KafkaBackend implementation"
- **URL:** https://github.com/alob-mtc/runnerq/issues/58
- **Reactions:** 1 (EYES)
- **Status:** Open
- **Summary:** Community request for Kafka storage backend.

### #42 — "Experimental feature proposal (Inline-Activity)"
- **URL:** https://github.com/alob-mtc/runnerq/issues/42
- **Reactions:** 0
- **Status:** Open
- **Summary:** Proposal for lightweight inline activities that execute in the same process without going through the storage backend. Would reduce overhead for trivial sub-tasks.

### #19 — "Retry forever by default and not 3 times"
- **URL:** https://github.com/alob-mtc/runnerq/issues/19
- **Reactions:** 0
- **Status:** Open
- **Summary:** Default `max_retries=3` is considered too aggressive. Proposal to change default to unlimited retries (0 = unlimited already works; just change the default).
