# Contract: Lifecycle Hooks

**Status**: Planned (Phase 5)
**Location**: `crates/resource/src/hooks.rs` (to be created)

## ResourceHook Trait

Pluggable callbacks for resource lifecycle events. Hooks run synchronously in priority order.

```rust
#[async_trait]
pub trait ResourceHook: Send + Sync {
    /// Called before a lifecycle operation.
    /// Return Err to cancel the operation.
    /// Default: allow (Ok).
    async fn before(&self, event: &HookEvent) -> Result<(), Error> {
        let _ = event;
        Ok(())
    }

    /// Called after a lifecycle operation completes.
    /// Cannot cancel — operation already done.
    /// Default: no-op.
    async fn after(&self, event: &HookEvent, result: &HookResult) {
        let _ = (event, result);
    }

    /// Execution priority. Lower number = earlier execution.
    /// Default: 100.
    fn priority(&self) -> u32 {
        100
    }

    /// Which resources this hook applies to.
    /// Default: all resources.
    fn filter(&self) -> HookFilter {
        HookFilter::All
    }
}
```

## HookEvent

```rust
#[derive(Debug, Clone)]
pub enum HookEvent {
    BeforeCreate { resource_id: String, config: serde_json::Value },
    AfterCreate { resource_id: String },
    BeforeAcquire { resource_id: String },
    AfterAcquire { resource_id: String, wait_duration: Duration },
    BeforeRelease { resource_id: String, usage_duration: Duration },
    AfterRelease { resource_id: String },
    BeforeCleanup { resource_id: String, reason: CleanupReason },
    AfterCleanup { resource_id: String },
    HealthChanged { resource_id: String, from: HealthState, to: HealthState },
}
```

## HookFilter

```rust
#[derive(Debug, Clone)]
pub enum HookFilter {
    /// Fires for all resources
    All,
    /// Fires only for the specified resource
    ResourceId(String),
    /// Fires for any resource in the set
    ResourceIds(Vec<String>),
}
```

## HookResult

```rust
#[derive(Debug)]
pub enum HookResult {
    Success,
    Error(Error),
}
```

## HookRegistry

```rust
pub struct HookRegistry {
    hooks: Vec<Box<dyn ResourceHook>>,
}

impl HookRegistry {
    pub fn new() -> Self;

    /// Register a hook. Hooks are sorted by priority on insertion.
    pub fn register(&mut self, hook: Box<dyn ResourceHook>);

    /// Execute all matching "before" hooks in priority order.
    /// Returns Err if any hook cancels the operation.
    pub async fn run_before(&self, event: &HookEvent) -> Result<(), Error>;

    /// Execute all matching "after" hooks in priority order.
    /// Errors are logged but do not affect the operation.
    pub async fn run_after(&self, event: &HookEvent, result: &HookResult);
}
```

## Built-in Hooks

### AuditHook (priority: 10)

Logs all lifecycle operations for compliance/audit trails.

```rust
pub struct AuditHook;

// Logs: timestamp, resource_id, event_type, result
// Uses tracing::info! with structured fields
```

### MetricsHook (priority: 20)

Collects timing metrics for lifecycle operations.

```rust
pub struct MetricsHook;

// Records: acquire_duration, release_duration, create_duration
// Uses metrics crate counters/histograms
```

### CredentialRefreshHook (priority: 50)

Refreshes credentials before acquire if they are expiring.

```rust
pub struct CredentialRefreshHook {
    credential_provider: Arc<dyn CredentialProvider>,
    refresh_threshold: Duration,  // Refresh if expiring within this window
}

// Before acquire: check credential expiry
// If expiring soon: refresh and update resource instance
```

### SlowAcquireHook (priority: 90)

Emits warning when acquire takes longer than threshold.

```rust
pub struct SlowAcquireHook {
    threshold: Duration,  // Default: 5 seconds
}

// After acquire: if wait_duration > threshold, emit tracing::warn!
```

## Execution Flow

```
Manager::acquire(key, ctx)
    → HookRegistry::run_before(BeforeAcquire { resource_id })
        → Hook(priority=10): AuditHook.before() → Ok
        → Hook(priority=50): CredentialRefreshHook.before() → Ok or Err
        → Hook(priority=90): SlowAcquireHook.before() → Ok
    → Pool::acquire(ctx)  // actual acquisition
    → HookRegistry::run_after(AfterAcquire { resource_id, wait_duration }, result)
        → Hook(priority=10): AuditHook.after()
        → Hook(priority=20): MetricsHook.after()
        → Hook(priority=90): SlowAcquireHook.after()
    → Return Guard to caller
```

## Invariants

1. "Before" hooks run BEFORE the operation. If any returns Err, the operation is cancelled.
2. "After" hooks run AFTER the operation. Errors are logged, not propagated.
3. Hooks execute in priority order (ascending — lower = earlier).
4. `HookFilter` is checked before invocation — non-matching hooks are skipped.
5. Hook execution is synchronous within the lifecycle operation (not spawned).
6. Hooks MUST NOT acquire resources from the same manager (deadlock risk).
7. Hook execution time counts against the operation's timeout.
