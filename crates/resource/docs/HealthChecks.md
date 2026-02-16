# Health Checks

How `nebula-resource` monitors resource health, quarantines failures, and
propagates state through the dependency graph.

## Health States

`HealthState` enum in `crates/resource/src/health.rs`:

| State | `is_usable()` | `score()` | Acquire behavior |
|---|---|---|---|
| `Healthy` | true | 1.0 | Allowed |
| `Degraded { reason, performance_impact }` | true if impact < 0.8 | 1.0 - impact | Allowed with warning |
| `Unhealthy { reason, recoverable }` | false | 0.0 | Blocked |
| `Unknown` | false | 0.5 | Blocked |

`performance_impact` is clamped to `[0.0, 1.0]`. `HealthStatus` also carries
optional `latency` and `metadata` via builder methods `with_latency()` and
`with_metadata()`.

## HealthCheckable Trait

Resources that support health checking implement `HealthCheckable`:

```rust
pub trait HealthCheckable: Send + Sync {
    fn health_check(&self) -> impl Future<Output = Result<HealthStatus>> + Send;
    fn detailed_health_check(&self, _context: &Context) -> ...; // defaults to health_check()
    fn health_check_interval(&self) -> Duration;  // default: 30s
    fn health_check_timeout(&self) -> Duration;   // default: 5s
}
```

## HealthChecker (Background Monitoring)

`HealthChecker` spawns a tokio task per monitored instance, polling
`health_check()` at a configured interval and tracking consecutive failures.

```rust
let config = HealthCheckConfig {
    default_interval: Duration::from_secs(30),
    failure_threshold: 3,
    check_timeout: Duration::from_secs(5),
};
let checker = HealthChecker::new(config);
// Or with event bus for HealthChanged events:
let checker = HealthChecker::with_event_bus(config, event_bus);
```

```rust
checker.start_monitoring(instance_id, "postgres".to_string(), instance);
checker.get_health(&instance_id);          // Option<HealthRecord>
checker.get_unhealthy_instances();         // Vec<HealthRecord>
checker.get_critical_instances();          // consecutive_failures >= threshold
checker.stop_monitoring(&instance_id);     // cancel one task
checker.shutdown();                        // cancel all tasks
```

Calling `start_monitoring` with an already-monitored `instance_id` cancels the
previous task before starting a new one.

Each check result updates `consecutive_failures`: usable results reset to 0;
non-usable results, errors, and timeouts increment it.

## HealthPipeline and Stages

Compose multi-step health checks with `HealthPipeline`. Stages run in order;
the first `Unhealthy` result short-circuits. The final result is the worst
status observed.

```rust
pub trait HealthStage: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self, ctx: &Context) -> impl Future<Output = Result<HealthStatus>> + Send;
}
```

```rust
let mut pipeline = HealthPipeline::new();
pipeline.add_stage(ConnectivityStage::new(|exec_id: &str| async {
    tcp_ping(exec_id).await.is_ok()
}));
pipeline.add_stage(
    PerformanceStage::new(Duration::from_millis(100), Duration::from_millis(500))
        .with_probe(|exec_id: &str| async { measure_latency(exec_id).await })
);
let result = pipeline.run(&ctx).await?;
```

**`ConnectivityStage`**: takes `Fn(&str) -> Future<Output = bool>`. Returns
`Healthy` when true, `Unhealthy` otherwise.

**`PerformanceStage`**: below `warn_threshold` = Healthy, between warn and
fail = Degraded (impact proportional), above `fail_threshold` = Unhealthy.
Without `with_probe()`, returns Healthy.

## Quarantine

`QuarantineManager` blocks acquisition of resources that exceed the failure
threshold. Recovery uses exponential backoff.

```rust
let config = QuarantineConfig {
    failure_threshold: 3,
    max_recovery_attempts: 5,
    recovery_strategy: RecoveryStrategy {
        base_delay: Duration::from_secs(1),  // 1s, 2s, 4s, 8s, ... capped at max_delay
        max_delay: Duration::from_secs(60),
        multiplier: 2.0,
    },
};
let qm = QuarantineManager::new(config);
```

```rust
// Quarantine (no-op if already quarantined)
qm.quarantine("postgres", QuarantineReason::HealthCheckFailed {
    consecutive_failures: 5,
});
// Or manual quarantine
qm.quarantine("redis", QuarantineReason::ManualQuarantine {
    reason: "maintenance".into(),
});

qm.is_quarantined("postgres");      // true
qm.record_failed_recovery("postgres"); // increments attempts, schedules next retry
qm.due_for_recovery();              // entries past next_recovery_at
qm.release("postgres");             // remove from quarantine
```

Once `recovery_attempts >= max_recovery_attempts`, the entry is exhausted and
no more automatic retries are scheduled. In `Manager::acquire()`, quarantine
is checked first -- quarantined resources return `Error::Unavailable` with
`retryable: true`.

## Dependency Cascade

`Manager::set_health_state()` propagates health changes through the
`DependencyGraph`:

- **Unhealthy**: direct dependents become `Degraded` with reason
  `"Dependency {id} is unhealthy"` (impact 0.5). Already-unhealthy
  dependents are not overwritten.
- **Healthy**: clears `Degraded` states on dependents whose reason mentions
  this resource. Degraded states from other resources are untouched.

```rust
mgr.deps.write().add_dependency("app", "db").unwrap();
mgr.set_health_state("db", HealthState::Unhealthy {
    reason: "connection refused".into(), recoverable: true,
});
// "app" is now Degraded { reason: "Dependency db is unhealthy", ... }
mgr.set_health_state("db", HealthState::Healthy);
// "app" is cleared back to Healthy
```

## Configuration Defaults

| Parameter | Default |
|---|---|
| `HealthCheckConfig::default_interval` | 30s |
| `HealthCheckConfig::failure_threshold` | 3 |
| `HealthCheckConfig::check_timeout` | 5s |
| `QuarantineConfig::failure_threshold` | 3 |
| `QuarantineConfig::max_recovery_attempts` | 5 |
| `RecoveryStrategy::base_delay` | 1s |
| `RecoveryStrategy::max_delay` | 60s |
| `RecoveryStrategy::multiplier` | 2.0 |

## Event Bus Integration

`HealthChecker::with_event_bus()` emits `ResourceEvent::HealthChanged { resource_id, from, to }`
on every state transition. The bus uses `tokio::sync::broadcast` with
fire-and-forget semantics -- events are silently dropped if no subscribers exist.
