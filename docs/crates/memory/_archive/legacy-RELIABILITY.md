# Reliability

## SLO Targets

### Availability

- **Target**: 99.99% allocation success rate under normal pressure
- **Measurement**: `allocation_count` vs `failed_allocation_count` in `AllocatorStats`
- **Degradation threshold**: 99.9% triggers warning

### Latency

- **Pool acquire**: < 1us p99 (lock-free path)
- **Arena alloc**: < 100ns p99 (bump pointer)
- **Cache lookup**: < 500ns p99 (hash + LRU update)
- **Measurement**: Via `stats::profiler` when enabled

### Error Budget

- **Allocation failures**: < 0.01% under normal load
- **Pool exhaustion**: Acceptable if workload exceeds configured capacity
- **Cache eviction**: Expected behavior, not a failure

## Failure Modes

### Dependency Outage

| Dependency | Impact | Mitigation |
|------------|--------|------------|
| `nebula-system` unavailable | `MemoryMonitor` returns defaults | Graceful degradation, allocations continue |
| `nebula-log` unavailable | No logging output | Feature-gated, no runtime impact |
| `nebula-core` unavailable | Compile failure | Required dependency, no runtime concern |

### Timeout/Backpressure

- **Pool exhaustion**: `PoolExhausted` error returned immediately (no blocking)
- **Arena exhaustion**: `ArenaExhausted` with available/requested bytes
- **Budget exceeded**: `BudgetExceeded` prevents allocation; caller must release first

### Partial Degradation

| Condition | Behavior |
|-----------|----------|
| High memory pressure | `MemoryMonitor` recommends `ReduceAllocations` |
| Critical pressure | `PressureAction::Emergency` blocks large allocations |
| Stats overflow | Counters saturate at `usize::MAX`, no panic |

### Data Corruption

- **Detection**: `MemoryError::Corruption` with component/details
- **Response**: Log error, refuse further operations on affected region
- **Recovery**: Requires process restart; no automatic repair

## Resilience Strategies

### Retry Policy

- **Retryable errors**: `PoolExhausted`, `ArenaExhausted`, `BudgetExceeded`, `CacheMiss`
- **Not retryable**: `InvalidLayout`, `InvalidAlignment`, `Corruption`
- **Retry decision**: Caller responsibility; `MemoryError::is_retryable()` helper provided

### Circuit Breaking

- Not applicable at crate level
- Consumers may wrap memory operations in `nebula-resilience` circuit breakers

### Fallback Behavior

| Primary | Fallback |
|---------|----------|
| Custom allocator | System allocator (`SystemAllocator`) |
| Object pool | Direct heap allocation |
| Multi-level cache | Single-level or no caching |
| Monitored allocator | Unmonitored allocator |

### Graceful Degradation

1. **Pressure::Medium**: Log warning, continue normal operation
2. **Pressure::High**: Reduce max allocation size
3. **Pressure::Critical**: Deny large allocations, force cleanup

## Operational Runbook

### Alert Conditions

| Alert | Threshold | Action |
|-------|-----------|--------|
| High memory pressure | `MemoryPressure::High` sustained > 1min | Investigate memory consumers |
| Pool exhaustion rate | > 1% of requests | Increase pool capacity or reduce concurrency |
| Allocation failure spike | > 10x baseline | Check for memory leaks, system pressure |

### Dashboards

Metrics available via `AllocatorStats` and `MonitoringStats`:

- `allocated_bytes` / `total_bytes_allocated`
- `allocation_count` / `deallocation_count`
- `current_pressure` / `pressure_changes`
- `memory_usage_percent` / `health_score`

### Incident Triage Steps

1. **Check system pressure**: `MemoryMonitor::get_stats()` or OS tools
2. **Review allocation patterns**: `AllocatorStats` for anomalies
3. **Identify hotspots**: Profile if `profiling` feature enabled
4. **Correlate with workload**: Workflow execution logs
5. **Mitigate**: Increase limits, restart pools, or shed load

## Capacity Planning

### Load Profile Assumptions

| Workload | Expected Allocation Pattern |
|----------|----------------------------|
| Low (< 100 workflows/min) | Single arena/pool sufficient |
| Medium (100-1000/min) | Per-workflow pools, shared cache |
| High (> 1000/min) | Thread-local pools, partitioned caches |

### Scaling Constraints

- **Vertical**: Pool/arena sizes limited by system memory
- **Horizontal**: Each process has independent allocators
- **Memory overhead**: ~1KB per `ObjectPool`, ~64 bytes per arena chunk header

### Recommended Configurations

| Scenario | Configuration |
|----------|---------------|
| Low memory system (< 4GB) | Small pools (32-128 items), disable multi-level cache |
| Standard (4-16GB) | Default configs, enable monitoring |
| High memory (> 16GB) | Larger pools, enable NUMA-aware (Linux) |
