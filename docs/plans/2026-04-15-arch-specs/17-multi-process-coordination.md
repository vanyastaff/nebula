# Spec 17 — Multi-process coordination (leaderless peers)

> **Status:** draft
> **Canon target:** §12.8 (new)
> **Depends on:** 08 (cancellation), 09 (retry), 10 (fair scheduling), 14 (stateful lease loss handling), 16 (storage schema)
> **Depended on by:** none — this is the top of the stack

## Problem

Single-process v1 is the default, but contracts must support multi-process. Multi-process deployment means:

- N Nebula processes share one Postgres
- They must not execute the same execution twice concurrently
- One process crashing must not block work for others
- Rolling deploys must drain gracefully without losing work
- K8s / systemd / supervisord handles process lifecycle; Nebula handles work coordination

Getting this wrong means split-brain corruption, duplicate side effects, stuck executions. Every distributed system has scars from this.

**Classic anti-patterns to avoid:**

- Dedicated leader with manual failover (single point of failure)
- Heartbeat tables that drift (duplicate liveness tracking)
- Cleanup cron that runs on every worker (thundering herd)
- RPC between workers for cancel routing (tight coupling)

## Decision

**Leaderless peer coordination through Postgres only.** No `workers` / `nodes` table. Ephemeral `node_id` generated at process startup. Coordination via `executions.claimed_by` + `claimed_until` lease + `FOR UPDATE SKIP LOCKED` claim query. Process lifecycle delegated to infrastructure. SQLite is single-process only.

## Architecture diagram

```
┌───────────────────────────────────────────────────────────────────┐
│ Infrastructure layer (K8s / systemd / supervisord)                 │
│                                                                      │
│  ┌───────────────────┐  ┌───────────────────┐  ┌────────────────┐│
│  │ Pod 1 / Process 1  │  │ Pod 2 / Process 2  │  │ Pod 3          ││
│  │ node_id=nbl_01A... │  │ node_id=nbl_02B... │  │ node_id=nbl_..││
│  │                     │  │                     │  │                ││
│  │  Dispatcher loop    │  │  Dispatcher loop    │  │  Dispatcher    ││
│  │   ├ claim query     │  │   ├ claim query     │  │   ├ claim      ││
│  │   ├ lease renewal   │  │   ├ lease renewal   │  │   ├ renewal    ││
│  │   └ execution pool  │  │   └ execution pool  │  │   └ pool       ││
│  │                     │  │                     │  │                ││
│  │ /health /ready      │  │ /health /ready      │  │ /health /ready ││
│  └─────────┬──────────┘  └─────────┬──────────┘  └────────┬───────┘│
│            │                        │                      │         │
└────────────┼────────────────────────┼──────────────────────┼────────┘
             │                        │                      │
             └────────────┬───────────┴──────────────────────┘
                          ▼
                 ┌─────────────────┐
                 │  Postgres        │
                 │                  │
                 │  executions      │  ← single source of truth
                 │  execution_nodes │
                 │  control_queue   │
                 │  ...             │
                 └─────────────────┘

Coordination:
- Peers compete for claim via FOR UPDATE SKIP LOCKED
- Each process owns leases on rows it claimed
- Lease renewal every 10s, TTL 30s
- Dead process → lease expires → peer claims via stale recovery branch
- No leader election, no cross-process RPC
```

## Ephemeral node ID

Each process generates a fresh `node_id` at startup:

```rust
// At process startup
static NODE_ID: OnceCell<NodeId> = OnceCell::new();

pub fn init_node_id() -> NodeId {
    let id = NodeId::new();  // nbl_ ULID
    NODE_ID.set(id).expect("node_id initialized twice");
    tracing::info!(node_id = %id, "nebula process started");
    id
}

pub fn node_id() -> NodeId {
    NODE_ID.get().copied().expect("node_id not initialized")
}
```

**Not persisted anywhere durable.** Lives in process memory until exit. On restart, new process gets new ID. Rows still claimed by the old ID have expired leases; new process picks them up via stale recovery branch.

**Why not persistent:** persistent ID would require tracking alive processes, which is the whole thing we're avoiding. Postgres is the only shared state.

## Unified claim query

**One query handles three cases:**

1. **New work claim** — Pending/Queued execution with no claim
2. **Stale recovery** — Running execution with expired lease (previous owner dead)
3. **Wake-up** — Suspended execution ready to resume (wake_at passed or signal delivered)

```sql
UPDATE executions
SET status = CASE
        WHEN status IN ('Pending', 'Queued') THEN 'Running'
        WHEN status = 'Running' THEN 'Running'          -- takeover, keep Running
        WHEN status = 'Suspended' THEN 'Running'        -- wake up
    END,
    claimed_by = $current_node_id,
    claimed_until = NOW() + INTERVAL '30 seconds',
    started_at = COALESCE(started_at, NOW()),
    version = version + 1
WHERE id = (
    SELECT id FROM executions
    WHERE (
        -- Case 1: new pending work
        (status IN ('Pending', 'Queued') AND claimed_until IS NULL)
        OR
        -- Case 2: stale lease recovery (previous worker crashed)
        (status = 'Running' AND claimed_until < NOW())
        OR
        -- Case 3: suspended execution ready to resume
        (status = 'Suspended' AND wake_at IS NOT NULL AND wake_at <= NOW())
        -- TODO: signal-based wake-up handled separately (spec 14)
    )
    ORDER BY
        -- Fair scheduling: workspace with least recent dispatch first
        COALESCE(
            (SELECT last_dispatched_at
             FROM workspace_dispatch_state
             WHERE workspace_id = executions.workspace_id),
            '1970-01-01'::TIMESTAMPTZ
        ) ASC,
        -- Within workspace: oldest first
        executions.created_at ASC
    LIMIT 1
    FOR UPDATE SKIP LOCKED
)
RETURNING *;
```

**Properties:**

- **Single round-trip** for new work + stale recovery + wake-up
- `FOR UPDATE SKIP LOCKED` prevents lock contention between workers
- `ORDER BY` implements fair scheduling between workspaces (spec 10)
- Returns 0 or 1 row — worker processes it and loops

**Batch version** for higher throughput — claim multiple in one query:

```sql
UPDATE executions
SET claimed_by = $1, claimed_until = NOW() + INTERVAL '30 seconds', version = version + 1,
    status = CASE WHEN status IN ('Pending','Queued','Suspended') THEN 'Running' ELSE status END,
    started_at = COALESCE(started_at, NOW())
WHERE id IN (
    SELECT id FROM executions
    WHERE [same predicate]
    ORDER BY [same ordering]
    LIMIT 10
    FOR UPDATE SKIP LOCKED
)
RETURNING *;
```

Returns 0–10 rows. Dispatcher batches claim, spawns tasks for each.

### SQLite equivalent

SQLite doesn't have `FOR UPDATE SKIP LOCKED`. Single-process deployment means no concurrent access to same row. Simple:

```sql
UPDATE executions
SET claimed_by = ?, claimed_until = ...
WHERE id = (
    SELECT id FROM executions WHERE [predicate] ORDER BY [...] LIMIT 1
)
RETURNING *;
```

Relies on SQLite's write serialization (one writer at a time). Works for single-process. **SQLite is not suitable for multi-process** and we document this prominently.

## Lease management

### Renewal loop

```rust
pub async fn run_lease_renewal(
    storage: Arc<dyn Storage>,
    my_node_id: NodeId,
    cancellation: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => {
                // Renew all leases held by this node
                let result = storage.renew_leases(my_node_id).await;
                match result {
                    Ok(count) => {
                        metrics::gauge!("nebula_owned_executions", count as f64);
                    }
                    Err(e) => {
                        tracing::warn!("lease renewal failed: {}", e);
                        // Continue — next tick will retry
                    }
                }
            }
        }
    }
}
```

```sql
-- renew_leases query
UPDATE executions
SET claimed_until = NOW() + INTERVAL '30 seconds', version = version + 1
WHERE claimed_by = $1
  AND status IN ('Running', 'Cancelling')
  AND claimed_until IS NOT NULL;
```

**30-second TTL, 10-second renewal interval.** Gives 3× safety margin — one missed renewal is recoverable, two means real problem.

### Lease loss detection

When a flush or state transition fails due to CAS mismatch, the worker knows it lost the lease:

```rust
async fn flush_node_state(
    &self,
    node_attempt_id: NodeAttemptId,
    state_bytes: Vec<u8>,
    expected_version: u64,
) -> Result<(), FlushError> {
    let result = sqlx::query!(
        "UPDATE execution_nodes
         SET state = $1, iteration_count = $2, version = version + 1
         WHERE id = $3 AND version = $4 AND claimed_by = $5",
        state_bytes,
        self.iteration_count,
        node_attempt_id.as_bytes(),
        expected_version as i64,
        self.my_node_id.as_bytes(),
    ).execute(&self.pool).await?;
    
    if result.rows_affected() == 0 {
        // Lease lost: either version bumped by another writer, or claimed_by changed
        return Err(FlushError::LeaseLost);
    }
    Ok(())
}
```

**On `LeaseLost`:**

1. Worker **stops executing** this execution immediately
2. Drops any dirty state from in-memory buffer (spec 14)
3. Aborts the `tokio::task` running the action (via cancellation token)
4. Logs `lease_lost` event for post-mortem
5. Removes execution from local tracking
6. **Does not retry** — new owner has taken over, their work is authoritative

This is the **split-brain protection** rule from spec 14.

## Takeover policy

When claim query returns a previously-running execution (case 2: stale lease), what to do?

### Default: resume from last checkpoint (T1)

```rust
async fn resume_execution(&self, exec_row: ExecutionRow) -> Result<()> {
    // Load execution_nodes to find in-flight work
    let running_nodes = self.storage
        .list_nodes_for_execution(exec_row.id)
        .await?
        .into_iter()
        .filter(|n| n.status == NodeStatus::Running);
    
    for node in running_nodes {
        // Check takeover attempt count
        let crash_count = self.storage.count_takeovers(exec_row.id, node.logical_node_id).await?;
        
        if crash_count >= 3 {
            // Too many takeovers — mark Orphaned, operator intervention needed
            self.storage.mark_orphaned(exec_row.id, "repeated takeover crashes").await?;
            return Ok(());
        }
        
        // Reset this node to Running (takeover)
        // State in execution_nodes.state is the resume point
        // Iteration count preserved
        self.storage.increment_takeover_count(exec_row.id, node.logical_node_id).await?;
    }
    
    // Continue executing from where we left off
    self.engine.run_execution(exec_row).await
}
```

**Properties:**

- Stateful actions resume from last checkpoint
- Regular (non-stateful) actions effectively retry attempt (same idempotency key)
- External systems with idempotency support return cached result — no double execution
- External systems without idempotency: duplicate side effect possible — author's responsibility via reconciliation

### Circuit breaker after N takeovers

If same execution keeps crashing workers (indicates bug in action code, not infrastructure), mark `Orphaned` after 3 takeover attempts:

```sql
ALTER TABLE executions
    ADD COLUMN takeover_count INT NOT NULL DEFAULT 0;
```

```sql
-- Increment on takeover
UPDATE executions SET takeover_count = takeover_count + 1 WHERE id = ?;

-- Check threshold
SELECT takeover_count FROM executions WHERE id = ?;

-- If >= 3, mark Orphaned
UPDATE executions SET status = 'Orphaned' WHERE id = ?;
```

**`Orphaned` status** surfaces in UI with «requires operator intervention». Operator can manually restart (R4a from spec 09) or investigate the underlying action bug.

## Cancel routing in multi-process

Control queue processing:

```rust
async fn control_queue_scanner(
    storage: Arc<dyn Storage>,
    my_node_id: NodeId,
    cancellation: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = interval.tick() => {
                let commands = storage.scan_control_queue_for_node(my_node_id).await.ok().unwrap_or_default();
                for cmd in commands {
                    self.process_control_command(cmd).await.ok();
                }
            }
        }
    }
}
```

```sql
-- scan_control_queue_for_node — only commands for our own executions
SELECT ecq.*
FROM execution_control_queue ecq
INNER JOIN executions e ON e.id = ecq.execution_id
WHERE ecq.status = 'Pending'
  AND e.claimed_by = $1                    -- we hold the lease
  AND e.status IN ('Running', 'Cancelling', 'Suspended')
FOR UPDATE SKIP LOCKED
LIMIT 10;
```

**Filter by own lease.** Commands for executions owned by other workers are ignored — other workers will pick them up.

**If execution was reassigned (takeover) between command write and scan:** new owner sees the command in its next scan. Old owner doesn't see it (no longer owns lease). No double-processing, no loss.

**Scan interval 2s.** Tunable. Lower = faster cancel response, higher DB load.

## Cron scheduling in multi-process

No leader. Each process runs cron scheduler loop independently. Coordination via `cron_fire_slots` unique constraint (spec 11):

```rust
async fn cron_scheduler_tick(&self) {
    let triggers = self.storage.list_active_cron_triggers().await.unwrap_or_default();
    
    for trigger in triggers {
        for slot in upcoming_fire_slots(&trigger, now()) {
            // Try to claim
            let claimed = self.storage
                .claim_cron_slot(trigger.id, slot, self.node_id)
                .await
                .unwrap_or(false);
            
            if !claimed { continue; }  // another process got it
            
            // Apply overlap policy, create execution
            self.fire_cron(&trigger, slot).await.ok();
        }
    }
}
```

```sql
-- claim_cron_slot
INSERT INTO cron_fire_slots (trigger_id, scheduled_for, claimed_by, claimed_at)
VALUES ($1, $2, $3, NOW())
ON CONFLICT (trigger_id, scheduled_for) DO NOTHING
RETURNING *;
```

**Unique constraint is the coordination mechanism.** Only one process can insert the same `(trigger_id, scheduled_for)` tuple — whoever wins the race creates the execution.

**Jitter** (spec 11) distributes load so multiple processes don't all try to claim the same slot at the same nanosecond.

## Process lifecycle — delegated to infrastructure

### Health probes

```rust
// GET /health — liveness
// Returns 200 if process is alive and storage is reachable
async fn health_probe(state: State<AppState>) -> StatusCode {
    // Simple ping to storage
    match state.storage.ping().await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

// GET /ready — readiness
// Returns 200 if process is accepting new work (not draining)
async fn ready_probe(state: State<AppState>) -> StatusCode {
    if state.shutdown_state.is_draining() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    StatusCode::OK
}
```

K8s configuration:

```yaml
livenessProbe:
  httpGet:
    path: /health
    port: 8080
  initialDelaySeconds: 10
  periodSeconds: 10
  timeoutSeconds: 5
  failureThreshold: 3

readinessProbe:
  httpGet:
    path: /ready
    port: 8080
  initialDelaySeconds: 5
  periodSeconds: 5
  timeoutSeconds: 3
  failureThreshold: 2

terminationGracePeriodSeconds: 900  # 15 minutes for graceful drain
```

### Shutdown flow

```
1. Infrastructure sends SIGTERM (K8s pod eviction, systemctl stop)
  ↓
2. Process signal handler fires
   shutdown_state.set_draining()  // /ready now returns 503
  ↓
3. Infrastructure removes pod from service load balancer (K8s) — no new HTTP traffic
  ↓
4. Dispatcher loop sees shutdown_state, stops claiming new work
  ↓
5. Running executions complete naturally within grace period (spec 08 cascade)
  ↓
6. process_lifecycle.shutdown(Duration::from_secs(60))
   - Cancels all tokens downstream
   - Waits for tracker
   - Up to 60s grace
  ↓
7. Force-abort any remaining tasks
  ↓
8. Close storage connections
  ↓
9. std::process::exit(0)
  ↓
10. If not exited within terminationGracePeriodSeconds → K8s SIGKILL
```

**No work is lost if the drain period is long enough.** Work still running at force-abort point → next worker picks up via lease expiration (normal takeover).

### Rolling deploy flow

```
1. K8s starts new pod (v2)
2. New pod: init node_id, connect to Postgres, run startup checks
3. /ready returns 200 → K8s adds to service
4. New pod starts claiming work
5. K8s sends SIGTERM to old pod (v1)
6. Old pod: draining, not claiming new work, finishing existing
7. Old pod: graceful shutdown within terminationGracePeriodSeconds
8. Repeat for next pod
```

**Zero downtime** achieved by `maxSurge: 1, maxUnavailable: 0` in K8s Deployment strategy — always at least the original number of pods ready.

**Work straddling version boundary:** execution started by v1 may complete under v2 (if v1 drained before finishing). Backward compatibility of workflow schema (spec 13) ensures v2 can read v1's state.

## Metrics for operator visibility

```rust
// Per-process metrics (scraped by Prometheus per-pod)
metrics::gauge!("nebula_dispatcher_active_executions", count as f64);
metrics::gauge!("nebula_lease_renewal_failures_total", failures as f64);
metrics::counter!("nebula_claim_query_total").increment(1);
metrics::counter!("nebula_claim_success_total").increment(claimed_count);
metrics::histogram!("nebula_claim_query_duration_seconds").record(duration.as_secs_f64());
metrics::counter!("nebula_takeover_total{reason=?}").increment(1);
metrics::counter!("nebula_lease_lost_total").increment(1);
```

**Aggregate across cluster via Prometheus rollups.** No dedicated Nebula-side coordination to track cluster health — delegated to observability stack.

## Edge cases

**Clock skew between workers:** use monotonic clocks for internal timing, wall clocks only for persisted timestamps. Lease TTL measured in wall clock (Postgres `NOW()`) — Postgres is the single clock source, workers just compare against it.

**Long-running transaction holding rows:** one dispatcher stuck in a long query can block others. Mitigation: query timeout on all dispatcher queries (e.g., 5 seconds). If claim query takes > 5s, it's cancelled and retried.

**Database connection pool exhaustion:** each process has its own pool. N processes × pool size = total DB connections. For Postgres: ensure `max_connections` is high enough, or use pgbouncer transaction mode.

**Dispatcher falling behind:** if work arrives faster than dispatcher claims, queue grows. Metric `nebula_pending_executions` alerts operator. Scale horizontally (add pods).

**Split brain:** worker A and worker B both claim same execution. **Cannot happen** — `FOR UPDATE SKIP LOCKED` serializes claims. Under `SKIP LOCKED`, if A has the row locked, B skips it.

**Crash during claim transaction:** transaction rolls back, no partial claim. Row returns to available state.

**All workers crash simultaneously:** all leases expire after TTL. Whenever any process comes back, it picks up all orphaned work. Work waits but isn't lost.

**Network partition between worker and Postgres:** worker can't renew lease, lease expires, other worker claims. When partition heals, old worker sees `LeaseLost` on next operation, drops work cleanly.

## Configuration surface

```toml
[multi_process]
# Node lifecycle
lease_ttl = "30s"
lease_renewal_interval = "10s"
control_queue_scan_interval = "2s"
cron_scheduler_tick_interval = "10s"

# Claim query
claim_batch_size = 10
claim_query_timeout = "5s"

# Takeover circuit breaker
max_takeover_attempts_before_orphan = 3

# Shutdown
process_grace_period = "60s"
dispatcher_drain_grace = "45s"
```

## Testing criteria

**Unit tests:**
- Node ID generation uniqueness
- Claim query builds correct SQL
- Lease renewal query updates correct rows
- Shutdown state transitions

**Integration tests:**
- **Single-process correctness:** run 1000 executions, all complete exactly once
- **Multi-process fairness:** 2 processes, 100 executions, verify distribution
- **Lease takeover:** kill worker mid-execution, verify new worker picks up within 40s
- **Split-brain prevention:** fake concurrent claim attempt, verify `SKIP LOCKED` prevents double claim
- **Rolling deploy simulation:** start v1, start v2, SIGTERM v1, verify no work lost
- **Circuit breaker on repeated takeovers:** force 3 crashes on same execution, verify marked Orphaned
- **Cancel routing after takeover:** start execution on worker A, kill worker A, worker B takes over, cancel reaches worker B
- **Cron scheduling leaderless:** 3 workers scheduling same cron, only one fire per slot
- **Dispatcher backpressure:** spawn enough work to exceed single-process pool, verify no claim on full pool

**Chaos tests:**
- Random kill of workers during load — no lost or duplicated work (verify via idempotency keys)
- Clock skew injection — no incorrect lease takeovers
- Network partition simulation — eventually consistent recovery
- DB flap (brief downtime) — workers recover, resume processing

**Performance tests:**
- Claim query latency under load: < 20 ms p99 with 100k rows in `executions`
- Lease renewal query: < 10 ms p99 for 1000 owned rows
- Throughput per process: ≥ 100 executions/sec steady state

## Performance targets

- Claim query: **< 20 ms p99** for backlog up to 100k rows
- Lease renewal: **< 10 ms p99**
- Takeover latency (crash to pickup): **< 40 seconds** (lease TTL + scan interval)
- Rolling deploy drain: **< 15 minutes** worst case

## Module boundaries

| Component | Crate |
|---|---|
| `NodeId` type | `nebula-core` |
| `LayerLifecycle` (from spec 08) | `nebula-core` |
| Dispatcher loop | `nebula-engine` |
| Lease renewal task | `nebula-engine` |
| Control queue scanner | `nebula-engine` |
| Cron scheduler | `nebula-engine` |
| Claim query SQL | `nebula-storage` |
| Lease / CAS primitives | `nebula-storage` |
| Health / ready endpoints | `nebula-api` |
| Signal handler → shutdown | `nebula-api` (main binary) |

## Migration path

**v1 self-host (single process):** no special action. Single Nebula process on SQLite or Postgres, no coordination needed but all primitives work.

**v1 cloud (multi-process on Postgres):** deploy multiple pods behind load balancer, configure K8s Deployment with `replicas`, readiness probes, graceful shutdown. No code change from self-host.

**Scaling from 1 to N:** just increase replicas. Each new process gets a node_id, starts claiming. Existing work is unaffected.

**Scaling from N to 1:** scale down replicas. Old pods drain gracefully. Work continues on remaining pods.

## Open questions

- **Auto-scaling** — K8s HPA on queue depth or CPU? Deferred until operational experience shows what metric matters most.
- **Multi-region coordination** — cross-region Postgres with active-passive? Complex, deferred until customer ask.
- **Workload isolation** — pin specific workspaces to specific pods for noisy-neighbor protection? Deferred, rely on fair scheduling + quotas first.
- **Dispatcher sharding** — partition `executions` by hash of `workspace_id`, each dispatcher handles one shard? Optimization for very high throughput, deferred.
- **Distributed tracing span propagation across workers** — single trace spanning multiple workers when takeover happens. Requires trace_id propagation through lease data. Deferred to observability spec (#18).
- **Crash-consistent test infrastructure** — ability to kill processes mid-operation reliably in CI. Infrastructure work, deferred.
