# Spec 14 — Stateful actions and checkpoint management

> **Status:** draft
> **Canon target:** §3.8.1 (new), §11.5 (sharpen)
> **Depends on:** 08 (cancel — on_cancel hook), 09 (retry — idempotency per attempt), 16 (storage — state column in execution_nodes)
> **Depended on by:** 11 (triggers — long-running TriggerAction is a stateful pattern)

## Problem

Canon §4.3 says «runs that last minutes through days are a normal design target». Long-running logic comes in three flavors:

1. **Iterator** — process 10 000 items in batches, checkpoint every batch
2. **Aggregator** — collect N events, emit when threshold reached
3. **Long poll / saga** — external wait + multi-step state machine

Without stateful primitives, authors either:
- **Pack everything into one node** — memory pressure, no intermediate checkpoints, crash = full restart
- **Split into many sequential nodes** — state passing via outputs becomes awkward, DAG grows unwieldy
- **Store state in external DB** — author owns persistence, bug surface, DX regression

Temporal solves this with event-sourced workflows (replay history from beginning) — powerful but complex. We want something simpler that handles the 95% case without replay-determinism pain.

## Decision

**`StatefulAction` trait with typed `State`, in-memory buffer + batch flush on `CheckpointPolicy`, `StepOutcome::WaitUntil` for durable suspension.** State lives in `execution_nodes.state` column (spec 16). Iteration idempotency key for external dedup. Memory-bounded with force-flush on pressure. Drop-on-lease-loss for split-brain protection.

## Trait

```rust
// nebula-action/src/stateful.rs

#[async_trait]
pub trait StatefulAction: Send + Sync + 'static {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type State: Serialize + DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;
    
    /// Called once at start, or after restore from checkpoint when state is None.
    /// Returns initial state.
    async fn initialize(
        &self,
        ctx: &StatefulContext,
        input: Self::Input,
    ) -> Result<Self::State, ActionError>;
    
    /// One iteration step. State is mutable — author updates in place.
    /// Returns outcome describing what to do next.
    /// Runtime calls this in a loop until Done or Cancelled.
    async fn step(
        &self,
        ctx: &StatefulContext,
        state: &mut Self::State,
    ) -> Result<StepOutcome<Self::Output>, ActionError>;
    
    /// Optional cleanup hook if action is cancelled mid-iteration.
    /// Runtime calls with final state before transitioning to Cancelled.
    async fn on_cancel(
        &self,
        _ctx: &StatefulContext,
        _state: &Self::State,
    ) -> Result<(), ActionError> {
        Ok(())  // default no-op
    }
    
    /// Optional: describes what to log / report as progress.
    /// Called periodically between steps for UI updates.
    fn progress(&self, _state: &Self::State) -> Option<ProgressUpdate> {
        None
    }
}

pub struct ProgressUpdate {
    pub percent: Option<f32>,      // 0.0..=100.0
    pub message: String,
    pub detail: Option<Value>,
}
```

### `StepOutcome`

```rust
pub enum StepOutcome<O> {
    /// Keep iterating. Runtime decides checkpoint based on policy.
    Continue,
    
    /// Keep iterating AND flush checkpoint now (explicit hint from author).
    /// Use after committing important side effects to preserve progress.
    CheckpointAndContinue,
    
    /// Iteration finished successfully with final output.
    Done(O),
    
    /// Suspend until a specific time or external signal.
    /// State is checkpointed, slot released, scheduler resumes later.
    WaitUntil(WaitCondition),
}

pub enum WaitCondition {
    /// Wake up at specific time. Minimum granularity ~1 second.
    Timer(DateTime<Utc>),
    
    /// Wake up when named signal fires (via API).
    /// Timeout: if signal not received by deadline, resume anyway and check state.
    Signal {
        name: String,
        deadline: Option<DateTime<Utc>>,
    },
    
    /// Combine: whichever fires first.
    SignalOrTimer {
        name: String,
        deadline: DateTime<Utc>,
    },
}
```

### `StatefulContext`

```rust
pub struct StatefulContext {
    // Standard action context
    pub cancellation: CancellationSignal,
    pub tasks: ActionTasks,
    pub logger: Logger,
    pub metrics: MetricsHandle,
    
    // Stateful-specific
    pub execution_id: ExecutionId,
    pub node_attempt_id: NodeAttemptId,
    pub logical_node_id: String,
    pub iteration_count: u32,  // 0-based, increments after each step
    
    // Idempotency
    pub attempt_idempotency_key: String,  // stable per attempt
    
    // Signals inbox (for WaitUntil resume)
    pub pending_signals: Vec<SignalPayload>,  // populated on resume
}

impl StatefulContext {
    /// Deterministic idempotency key for the current iteration.
    /// Format: {execution_id}:{logical_node_id}:{attempt}:iter:{iteration_count}
    /// Stable across restart: same iteration number → same key.
    pub fn iteration_idempotency_key(&self) -> String {
        format!(
            "{}:{}:{}:iter:{}",
            self.execution_id,
            self.logical_node_id,
            self.attempt_idempotency_key,
            self.iteration_count,
        )
    }
    
    /// Emit real-time progress to UI via websocket.
    /// Not persisted — ephemeral hint.
    pub async fn emit_progress(&self, update: ProgressUpdate) {
        self.metrics.record_progress(update);
    }
    
    /// Pop signals that arrived while suspended.
    /// Returns empty if no signals or not resuming from WaitUntil.
    pub fn pop_signals(&mut self) -> Vec<SignalPayload> {
        std::mem::take(&mut self.pending_signals)
    }
}
```

## Checkpoint policy

```rust
// nebula-action/src/metadata.rs
#[derive(Debug, Clone)]
pub struct CheckpointPolicy {
    pub strategy: CheckpointStrategy,
}

#[derive(Debug, Clone)]
pub enum CheckpointStrategy {
    /// Checkpoint after every step. Safest, slowest.
    EveryStep,
    
    /// Checkpoint every N steps.
    EveryN(u32),
    
    /// Checkpoint when T time elapsed since last checkpoint.
    EveryInterval(Duration),
    
    /// Author decides via StepOutcome::CheckpointAndContinue.
    Manual,
    
    /// Whichever fires first: N steps, T interval, OR explicit.
    /// Recommended default.
    Hybrid {
        max_steps: u32,
        max_interval: Duration,
    },
}

impl Default for CheckpointPolicy {
    fn default() -> Self {
        Self {
            strategy: CheckpointStrategy::Hybrid {
                max_steps: 10,
                max_interval: Duration::from_secs(30),
            },
        }
    }
}
```

**Default Hybrid { 10 steps, 30s }** trade-off:

- **10 steps** covers iterator case: batch of 100 items, checkpoint every 1000 items. Acceptable replay window.
- **30s** covers long-poll case: step sleeps for external event, checkpoint periodically even without new work.
- Either condition triggers flush — whichever first.

**Override per action:**

```rust
impl MyStatefulAction {
    fn metadata() -> ActionMetadata {
        ActionMetadata {
            checkpoint_policy: CheckpointPolicy {
                strategy: CheckpointStrategy::Hybrid {
                    max_steps: 100,  // more batching for heavy action
                    max_interval: Duration::from_secs(60),
                },
            },
            // ...
        }
    }
}
```

## Execution flow

```
Runtime creates NodeAttempt for StatefulAction
  ↓
Load existing state from execution_nodes.state (if resuming from checkpoint)
  OR call action.initialize() if fresh attempt
  ↓
Store state in in-memory buffer (not yet persisted)
  ↓
┌────────────────────────────────────────────────────────┐
│ Iteration loop                                          │
│                                                          │
│   steps_since_checkpoint = 0                           │
│   last_checkpoint_at = now                             │
│                                                          │
│   loop:                                                 │
│     action.step(&ctx, &mut state).await                │
│       ↓                                                 │
│     match outcome:                                     │
│       Continue →                                        │
│         steps_since_checkpoint += 1                    │
│         if should_checkpoint(steps, elapsed):          │
│           flush_state_to_db()                          │
│                                                          │
│       CheckpointAndContinue →                          │
│         flush_state_to_db()  (immediate)              │
│                                                          │
│       Done(output) →                                    │
│         flush_state_to_db()  (final)                  │
│         transition to Succeeded                        │
│         break                                          │
│                                                          │
│       WaitUntil(condition) →                           │
│         flush_state_to_db()  (before release)         │
│         transition to Suspended                        │
│         store wake_at / wake_signal_name              │
│         release execution slot                        │
│         break                                          │
│                                                          │
│     if cancellation.is_cancelled():                    │
│       action.on_cancel(&ctx, &state).await            │
│       flush_state_to_db()                             │
│       transition to Cancelled                         │
│       break                                          │
└────────────────────────────────────────────────────────┘
```

## Write-behind buffer

### Why batched flush

Naive approach: every step → UPDATE. For fast iteration (millisecond-per-step), DB becomes bottleneck. 1000 steps/sec → 1000 UPDATEs/sec — unacceptable.

**Solution: write-behind buffer.** State lives in memory between steps. Flush happens on policy trigger. Many changes batched into one UPDATE.

### Implementation

```rust
// nebula-runtime/src/stateful/buffer.rs
pub struct CheckpointBuffer {
    dirty: Arc<DashMap<NodeAttemptId, DirtyState>>,
    total_bytes: AtomicU64,
    max_bytes: u64,
    max_entries: usize,
}

pub struct DirtyState {
    pub serialized: Vec<u8>,       // latest serialized state
    pub iteration_count: u32,
    pub last_checkpoint_at: Instant,
    pub created_version: u64,      // starting version for CAS
    pub storage_metadata: StorageMetadata,  // execution_id, logical_node_id, etc.
}

impl CheckpointBuffer {
    pub async fn set_state<S: Serialize>(
        &self,
        key: NodeAttemptId,
        state: &S,
        iteration_count: u32,
    ) -> Result<(), BufferError> {
        let serialized = serde_json::to_vec(state)?;
        let size = serialized.len();
        
        // Memory pressure check
        if self.total_bytes.load(Ordering::Relaxed) + size as u64 > self.max_bytes {
            self.force_flush_oldest(size as u64 * 2).await?;
        }
        
        // Per-state hard cap
        if size > 1_048_576 {  // 1 MB
            return Err(BufferError::StateTooLarge { size, cap: 1_048_576 });
        }
        
        let old_size = self.dirty.get(&key)
            .map(|d| d.serialized.len())
            .unwrap_or(0);
        
        self.dirty.insert(key, DirtyState {
            serialized,
            iteration_count,
            last_checkpoint_at: Instant::now(),
            // ... etc
        });
        
        self.total_bytes.fetch_add(size as u64, Ordering::Relaxed);
        self.total_bytes.fetch_sub(old_size as u64, Ordering::Relaxed);
        
        Ok(())
    }
    
    pub async fn flush(&self, storage: &dyn Storage) -> Result<FlushStats> {
        let snapshots: Vec<_> = self.dirty.iter()
            .map(|e| (*e.key(), e.value().clone()))
            .collect();
        
        if snapshots.is_empty() {
            return Ok(FlushStats::empty());
        }
        
        // Batch transaction
        let mut tx = storage.begin().await?;
        let mut committed = Vec::new();
        
        for (key, dirty) in snapshots {
            let result = tx.execute(sqlx::query!(
                "UPDATE execution_nodes
                 SET state = $1, iteration_count = $2, version = version + 1
                 WHERE id = $3 AND version = $4 AND claimed_by = $5",
                dirty.serialized,
                dirty.iteration_count,
                key.as_bytes(),
                dirty.created_version,
                self.node_id.as_bytes(),  // CAS on claim
            )).await?;
            
            if result.rows_affected() == 1 {
                committed.push(key);
            } else {
                // Stale — we lost the lease, someone else owns this row
                // Drop the dirty entry, DO NOT retry
                self.drop_dirty(key);
            }
        }
        
        tx.commit().await?;
        
        // Remove committed from dirty
        for key in committed {
            self.dirty.remove(&key);
        }
        
        Ok(FlushStats { /* ... */ })
    }
    
    pub fn drop_dirty(&self, key: NodeAttemptId) {
        if let Some((_, dirty)) = self.dirty.remove(&key) {
            self.total_bytes.fetch_sub(dirty.serialized.len() as u64, Ordering::Relaxed);
        }
    }
}
```

### Flush triggers

| Trigger | Source |
|---|---|
| CheckpointPolicy hit (N steps or T time) | runtime iteration loop |
| Explicit `CheckpointAndContinue` | action return |
| `WaitUntil` transition (state must be durable before slot release) | runtime |
| Terminal transition (`Done`, `Cancelled`, escalated fail) | runtime |
| Process SIGTERM (drain all dirty) | signal handler + process shutdown |
| Memory pressure (total_bytes > 100 MB) | buffer internal |
| Entry count pressure (> 1000 dirty) | buffer internal |
| Lease renewal failure (CAS fails) | buffer internal — **drop**, not flush |

## CAS + lease check on flush

**Critical for split-brain protection.** Flush query includes `claimed_by = $this_node`:

```sql
UPDATE execution_nodes
SET state = $1, iteration_count = $2, version = version + 1
WHERE id = $3
  AND version = $4
  AND claimed_by = $5
```

If worker A lost its lease to worker B (via lease expiration), worker A's flush fails (claimed_by mismatch). **Worker A drops its dirty buffer — never overwrites worker B's work.** Worker B reads its own last checkpoint, resumes from there.

This implements the «drop on lease loss» rule described in spec 17.

## Memory pressure handling

```rust
pub async fn force_flush_oldest(&self, target_bytes_freed: u64) -> Result<()> {
    let mut candidates: Vec<_> = self.dirty.iter()
        .map(|e| (*e.key(), e.last_checkpoint_at))
        .collect();
    
    // Sort by oldest last_checkpoint_at
    candidates.sort_by_key(|(_, ts)| *ts);
    
    let mut freed = 0u64;
    for (key, _) in candidates {
        if freed >= target_bytes_freed { break; }
        
        if let Some(dirty) = self.dirty.get(&key) {
            freed += dirty.serialized.len() as u64;
        }
        
        self.flush_one(key).await?;
    }
    
    Ok(())
}
```

**Metrics exposed:**

- `nebula_stateful_buffer_bytes` — current dirty bytes
- `nebula_stateful_buffer_entries` — current dirty count
- `nebula_stateful_flush_total{trigger}` — flushes by trigger type
- `nebula_stateful_flush_duration_seconds` — flush latency
- `nebula_stateful_force_flush_total{reason}` — forced flushes (pressure, pressure-count)
- `nebula_stateful_state_size_bytes` — histogram of state sizes

If `force_flush_total{reason="memory"}` is non-zero in production, operator sees «workflows have too-large states» — tune policies or workflows.

## Idempotency — two-layer pattern

### Layer 1 — engine-provided key per iteration

```rust
async fn step(&self, ctx: &StatefulContext, state: &mut Self::State) -> Result<...> {
    let key = ctx.iteration_idempotency_key();
    // "{exec_id}:{node_id}:{attempt}:iter:42"
    
    let response = stripe_client
        .charges()
        .create_with_idempotency(&key, &charge_request)
        .await?;
    
    // ...
}
```

Stable per iteration. If action crashes mid-iteration and replays from last checkpoint, replaying iteration N uses same key. Stripe dedup returns cached result.

### Layer 2 — author-tracked committed state

For cases where external dedup is not available, author tracks explicitly:

```rust
#[derive(Serialize, Deserialize)]
struct MyState {
    current_batch: usize,
    last_committed_batch: Option<usize>,
    processed_total: u64,
}

async fn step(&self, ctx: &StatefulContext, state: &mut Self::State) -> Result<...> {
    // Skip if already committed this batch
    if state.last_committed_batch.is_some() 
        && state.current_batch <= state.last_committed_batch.unwrap() 
    {
        state.current_batch += 1;
        return Ok(StepOutcome::Continue);
    }
    
    // Do side effect
    do_thing(&state.current_batch).await?;
    
    // Mark committed
    state.last_committed_batch = Some(state.current_batch);
    state.processed_total += batch_size;
    state.current_batch += 1;
    
    // Explicit checkpoint hint — persist the committed marker ASAP
    Ok(StepOutcome::CheckpointAndContinue)
}
```

**Documented pattern** in author docs. Not enforceable by runtime. Spec 15 (delivery semantics) calls this «two-sided idempotency contract» — engine provides the key, author applies the discipline.

## `WaitUntil` — durable suspension

### Suspend flow

```
action returns StepOutcome::WaitUntil(condition)
  ↓
Runtime flushes state one last time
  ↓
BEGIN TRANSACTION
  UPDATE execution_nodes
    SET status = 'Suspended',
        state = <latest>,
        wake_at = <from condition>,
        wake_signal_name = <from condition>,
        claimed_by = NULL,        -- release slot
        claimed_until = NULL,
        version = version + 1
    WHERE id = ?
COMMIT
  ↓
Dispatcher slot released — another action can use it
```

### Resume flow

```
Scheduler scans execution_nodes WHERE status='Suspended' AND wake_at <= NOW()
  OR status='Suspended' AND pending signal delivered for wake_signal_name
  ↓
Worker claims via CAS (new claimed_by)
  ↓
Load state from execution_nodes.state
  ↓
Construct StatefulContext with pending_signals populated
  ↓
Runtime calls action.step(&ctx, &mut state)
  ↓
Action may read ctx.pop_signals() to see what triggered wake
  ↓
Iteration loop continues
```

### Signal delivery

```
POST /api/v1/orgs/{org}/workspaces/{ws}/executions/{id}/nodes/{node_id}/signal
Body: { "name": "approval_received", "payload": { "approved_by": "user_01..." } }
  ↓
Permission: WorkspaceRunner+
  ↓
Look up execution_nodes by (execution_id, logical_node_id, latest attempt)
  ↓
If status = 'Suspended' AND wake_signal_name = 'approval_received':
  INSERT signal into pending_signals table (or column on execution_nodes)
  UPDATE execution_nodes SET wake_at = NOW() (trigger scan immediately)
  ↓
Scheduler wakes up node (via claim query that handles Suspended)
```

Optional table:

```sql
CREATE TABLE pending_signals (
    id                BYTEA PRIMARY KEY,
    node_attempt_id   BYTEA NOT NULL REFERENCES execution_nodes(id) ON DELETE CASCADE,
    signal_name       TEXT NOT NULL,
    payload           JSONB,
    received_at       TIMESTAMPTZ NOT NULL,
    consumed_at       TIMESTAMPTZ
);

CREATE INDEX idx_pending_signals_unconsumed
    ON pending_signals (node_attempt_id, signal_name)
    WHERE consumed_at IS NULL;
```

Signals consumed when action resumes. Unconsumed signals persist until action wakes.

**Use cases:**

- **Human approval:** suspend until `approval_received` signal
- **External webhook:** suspend until `stripe.payment_succeeded` signal
- **Coordinated steps:** suspend until another workflow sends `upstream_ready`
- **Scheduled resume:** suspend for 3 days via Timer

## Timeouts for stateful

From spec 10, stateful-specific:

| Timeout | Default | Applies to |
|---|---|---|
| **Step timeout** | 5 min | One `step()` call |
| **Stateful max duration** | 7 days | Total action lifetime including all suspensions |

**Step timeout:** runtime wraps each `step()` call in `tokio::time::timeout`. If step takes > 5 min → `Err(ActionError::StepTimeout)`. Runtime transitions to `Failed`, or retries from last checkpoint if policy allows.

**Stateful max duration:** runtime records `started_at` on first attempt. If `now - started_at > stateful_max_duration`, action marked `Failed(StatefulTimeout)`. Prevents runaway stateful actions from living forever.

## Serialization format

### Payload structure

```
[1 byte format][1 byte compression][4 bytes schema_hash][N bytes payload]
```

- **Format:** 0=JSON, 1=MessagePack, 2=CBOR
- **Compression:** 0=none, 1=zstd, 2=gzip
- **Schema hash:** first 4 bytes of SHA-256 of `std::any::type_name::<State>()` (or explicit fingerprint)
- **Payload:** actual serialized state

### Deserialization check

```rust
pub fn deserialize_state<S: DeserializeOwned>(
    bytes: &[u8],
    expected_type: &str,
) -> Result<S, DeserializeError> {
    if bytes.len() < 6 {
        return Err(DeserializeError::TooShort);
    }
    
    let format = bytes[0];
    let compression = bytes[1];
    let actual_hash = &bytes[2..6];
    
    let expected_hash = &sha256(expected_type.as_bytes())[..4];
    if actual_hash != expected_hash {
        return Err(DeserializeError::SchemaHashMismatch {
            expected: hex::encode(expected_hash),
            actual: hex::encode(actual_hash),
            hint: "state schema changed; migration or manual intervention required",
        });
    }
    
    let payload = &bytes[6..];
    let decompressed = match compression {
        0 => payload.to_vec(),
        1 => zstd::decode_all(payload)?,
        2 => gzip_decode(payload)?,
        _ => return Err(DeserializeError::UnknownCompression),
    };
    
    match format {
        0 => serde_json::from_slice(&decompressed).map_err(Into::into),
        1 => rmp_serde::from_slice(&decompressed).map_err(Into::into),
        2 => ciborium::de::from_reader(&decompressed[..]).map_err(Into::into),
        _ => Err(DeserializeError::UnknownFormat),
    }
}
```

### Schema migration

When author changes `State` struct, schema_hash changes. Options:

1. **Deploy with migration function** registered per action:

```rust
pub trait StatefulMigration {
    fn migrate(&self, old_bytes: &[u8], old_hash: [u8; 4]) -> Result<Vec<u8>, MigrationError>;
}
```

Runtime calls `migrate` on hash mismatch, retries deserialization.

2. **Hard fail** — action can't be loaded, marked `Failed(StateIncompatible)`. Operator manually intervenes (restart execution or write migration).

**v1:** option 2 (hard fail with clear error). Option 1 deferred until real need.

## Progress UI

### Two surfaces

**1. Persisted state query (slow, consistent):**

```
GET /api/v1/orgs/{org}/workspaces/{ws}/executions/{id}/nodes/{node_id}/state
```

Returns latest persisted checkpoint. **Eventually consistent** — may lag by checkpoint interval (up to 30s default). Good for «what's the current state of this long-running action» dashboard view.

**2. Real-time progress (fast, ephemeral):**

```rust
impl ActionContext {
    pub async fn emit_progress(&self, update: ProgressUpdate) {
        // Writes to in-memory watch channel, pushes to websocket subscribers
        // NOT persisted — lost on restart
    }
}
```

Used by action to push «I just processed 3000/10000» to UI via websocket. Non-durable hint. On restart, UI sees last persisted state, not the lost progress.

### Websocket subscription

```
WebSocket upgrade:
  GET /api/v1/orgs/{org}/workspaces/{ws}/executions/{id}/live
  Upgrade: websocket
  ↓
Server subscribes client to eventbus for this execution
  ↓
Client receives: {event: "progress", node_id: "batch_processor", percent: 30, message: "..."}
```

## Edge cases

**State too large:** 100 MB hard cap. Action gets `Err(BufferError::StateTooLarge)`. Author must reduce state (offload large data to external storage, keep only references).

**State blob reference:** v1.5 feature — if state serialized > 1 MB, offload to `BlobStorage`, store `state_blob_ref` in row. State_inline is then NULL. `execution_nodes` stays small.

**Crash mid-iteration with no checkpoint yet:** next worker loads `state = NULL` from DB, calls `initialize()` again. Author must make `initialize` idempotent (safe to call twice for same execution).

**`Done(output)` but checkpoint fails:** transaction rolls back, action not marked complete, same iteration replays. Eventually succeeds or fails permanently. Side effect idempotency (spec 15) prevents double execution.

**Signal delivered to completed action:** `Done` transition was atomic, signal goes into `pending_signals` but never consumed — stays as orphan. Cleanup job removes signals for completed executions.

**Memory pressure forces flush of active action:** worker's own state flushed before buffer overflows. Worker continues, next iteration reads from buffer (still there) or loads from DB if buffer evicted entry under extreme pressure. Rare but handled.

## Configuration surface

```toml
[stateful]
# Buffer limits
max_dirty_bytes = 104_857_600       # 100 MB
max_dirty_entries = 1000
max_state_bytes = 1_048_576         # 1 MB per state hard cap
state_blob_offload_threshold = 1_048_576  # offload to blob if > this (v1.5)

[stateful.defaults]
checkpoint_strategy = "hybrid"
checkpoint_max_steps = 10
checkpoint_max_interval = "30s"
step_timeout = "5m"
stateful_max_duration = "7d"

[stateful.serialization]
default_format = "json"              # or "msgpack" / "cbor"
default_compression = "none"         # or "zstd" / "gzip"
compression_threshold_bytes = 10_240 # compress only if > 10 KB
```

## Testing criteria

**Unit tests:**
- `CheckpointPolicy::should_checkpoint` matrix (N steps, T time, Hybrid)
- Buffer set/flush lifecycle
- Schema hash computation is stable
- Serialization header parsing

**Integration tests (critical for this spec):**

1. **Basic iteration:** action processes 100 items in 10 batches, all succeed, `Done(output)`, all checkpoints flushed
2. **Crash + resume from last checkpoint:** kill worker mid-iteration, new worker reads state, resumes correctly, produces same final output
3. **Checkpoint hybrid policy:** verify N steps triggers flush, verify T interval triggers flush, verify whichever first wins
4. **Memory pressure force-flush:** fill buffer beyond threshold, verify force_flush triggered, oldest evicted first
5. **WaitUntil Timer:** suspend for 5 seconds, slot released, wake_at respected, action resumes
6. **WaitUntil Signal:** suspend indefinitely, POST signal API, action wakes and processes
7. **WaitUntil SignalOrTimer:** signal arrives before deadline → wake on signal; signal doesn't arrive → wake on timer
8. **Lease loss mid-action:** force lease expiration to simulate multi-process takeover, verify dirty buffer dropped, new worker resumes from DB state
9. **Cancel during iteration:** send cancel mid-iteration, verify `on_cancel` called with current state, final checkpoint flushed, status Cancelled
10. **Schema hash mismatch:** deploy new action with different State struct, old execution fails with clear `StateIncompatible` error
11. **State size over 1 MB:** action tries to put huge state, gets `StateTooLarge` error

**Performance tests:**
- 1000 steps/sec on small state (10 KB) — flush overhead acceptable
- 10 concurrent stateful actions with 500 KB states — memory usage bounded
- Checkpoint flush batches properly — measure SQL query count

**Property tests:**
- State persists and deserializes losslessly (roundtrip)
- Iteration count monotonically increases
- Serialize → deserialize → serialize produces identical bytes

## Performance targets

- Step execution overhead: **< 1 ms** beyond action's own work (buffer insert + decisions)
- Checkpoint flush (batch of 10): **< 20 ms p99** (one transaction)
- Buffer insert: **< 100 µs** p99
- State deserialization (100 KB JSON): **< 5 ms p99**
- Memory per dirty state: **serialized size + ~200 bytes overhead**

## Module boundaries

| Component | Crate |
|---|---|
| `StatefulAction`, `StepOutcome`, `WaitCondition` traits | `nebula-action` |
| `StatefulContext`, `ProgressUpdate`, `SignalPayload` | `nebula-action` |
| `CheckpointPolicy`, `CheckpointStrategy` | `nebula-action` |
| `CheckpointBuffer`, flush logic | `nebula-runtime::stateful` |
| Stateful executor (iteration loop) | `nebula-runtime::stateful` |
| Signal delivery API | `nebula-api` |
| `pending_signals` repo | `nebula-storage` |
| Schema hash utilities, serialization format | `nebula-action::serialization` |

## Open questions

- **State blob offload (v1.5)** — design in next iteration if real need. Simple: inline ≤1 MB, blob reference for larger.
- **Event sourcing alternative** — like Temporal, replay from event log instead of storing state. Much more complex, deferred unless strong need for deterministic replay across code changes.
- **State diff / delta compression** — store incremental diffs instead of full state per checkpoint. Nice optimization, probably YAGNI.
- **Concurrent signal delivery** — what if two signals with same name arrive before action wakes? v1: deliver both as list. Author processes in order.
- **Cross-execution state sharing** — «workflow A's state visible to workflow B». Explicitly not supported — use `$vars` or external state store.
- **Migration tooling** — runtime helper for writing state migrations. Deferred until first breaking change in author's state schema.
