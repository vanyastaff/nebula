# Reliability

## SLO Targets

| Metric | Target | Notes |
|--------|--------|------|
| **Availability** | Best-effort with engine | Runtime is library; no standalone process |
| **Latency** | execute_action overhead < 1ms | Excludes action execution time |
| **Error budget** | N/A | Errors propagated to engine |

## Failure Modes

### Dependency Outage

- **Registry:** ActionNotFound if handler missing; fail fast.
- **Sandbox:** Sandbox failure propagates as ActionError.
- **Telemetry:** Emit/metrics best-effort; no impact on execution.

### Timeout/Backpressure

- **Action hang:** Execution context (NodeContext today; ActionContext target) has CancellationToken; action should respect it. No runtime-level timeout yet.
- **Data overflow:** DataLimitExceeded; execution fails.

### Partial Degradation

- **One action fails:** NodeFailed emitted; engine handles (error edges, retry).
- **Registry empty:** ActionNotFound for all; engine-level issue.

### Data Corruption

- **Output corruption:** Action responsibility; runtime passes through.
- **Policy bypass:** check_output_size runs on primary output only; other outputs not checked.

## Resilience Strategies

### Retry Policy

- Runtime does not retry. Engine or resilience layer may retry based on RuntimeError::is_retryable().
- ActionError::retryable propagates; caller decides.

### Circuit Breaking

- N/A at runtime level. Engine may implement per-action or per-workflow.

### Fallback Behavior

- **ActionNotFound:** Return error; no fallback.
- **DataLimitExceeded:** Emit NodeFailed; return error. SpillToBlob (Phase 2) is alternative.

### Graceful Degradation

- Telemetry failure: continue execution; events/metrics best-effort.

## Operational Runbook

### Alert Conditions

- actions_failed_total spike
- action_duration_seconds p99 high
- DataLimitExceeded frequency

### Dashboards

- actions_executed_total, actions_failed_total
- action_duration_seconds histogram
- NodeStarted/NodeCompleted/NodeFailed event rate

### Incident Triage Steps

1. Check actions_failed_total; correlate with ActionNotFound vs ActionError
2. Check action_duration_seconds for slow actions
3. Check DataLimitExceeded; consider increasing limits or SpillToBlob
4. Check registry: are handlers registered?

## Capacity Planning

### Load Profile Assumptions

- Throughput = engine concurrency × nodes per execution
- Each execute_action: registry lookup + handler execute + data check + emit

### Scaling Constraints

- ActionRegistry: DashMap; concurrent reads/writes
- No internal queue; engine controls concurrency via semaphore
