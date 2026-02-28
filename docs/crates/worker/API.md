# API

## Public Surface (Planned)

```rust
pub struct WorkerConfig {
    pub worker_id: WorkerId,
    pub max_in_flight: u32,
    pub poll_interval: Duration,
    pub lease_ttl: Duration,
    pub shutdown_grace: Duration,
}

pub trait TaskSource {
    async fn claim(&self, worker_id: WorkerId, limit: u32) -> Result<Vec<TaskLease>, WorkerError>;
    async fn heartbeat(&self, lease: &TaskLease) -> Result<(), WorkerError>;
    async fn ack(&self, lease: TaskLease, result: TaskResultEnvelope) -> Result<(), WorkerError>;
    async fn nack(&self, lease: TaskLease, reason: NackReason) -> Result<(), WorkerError>;
}

pub trait TaskExecutor {
    async fn execute(&self, lease: TaskLease, cancel: CancellationToken)
        -> Result<TaskResultEnvelope, WorkerError>;
}

pub struct WorkerHandle;
impl WorkerHandle {
    pub async fn start(config: WorkerConfig, deps: WorkerDeps) -> Result<Self, WorkerError>;
    pub async fn drain(&self) -> Result<(), WorkerError>;
    pub async fn stop(&self) -> Result<(), WorkerError>;
}
```

## Error Taxonomy

- `WorkerError::Config`: invalid startup config.
- `WorkerError::Unavailable`: transient upstream issue (`queue`, `runtime`, `sandbox`).
- `WorkerError::Timeout`: execution or upstream timeout.
- `WorkerError::CapacityExceeded`: local admission rejection.
- `WorkerError::InvariantViolation`: bug/contract breach, triggers fail-fast path.

## Behavioral Contracts

- `claim` must not return tasks without lease metadata.
- `ack/nack` must be idempotent for repeated deliveries.
- `drain()` stops new claims, keeps in-flight until completion or timeout.
- cancellation must propagate into sandbox and action execution.

## Versioning Rules

- Additive config fields only in minor release.
- Lease/result schema breaks require major release + migration guide.
- Deprecated fields stay for at least one minor release.
