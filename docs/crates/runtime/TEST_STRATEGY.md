# Test Strategy

## Test Pyramid

- **Unit:** ActionRegistry register/get/remove/keys; DataPassingPolicy check_output_size; RuntimeError display, is_retryable.
- **Integration:** ActionRuntime with real handlers, sandbox, event bus, metrics; engine integration; resource integration.
- **Contract:** execute_action signature; RuntimeError variants; telemetry event sequence.
- **End-to-end:** Engine runs workflow; runtime executes nodes; events and metrics recorded.

## Critical Invariants

- execute_action returns Ok only when handler succeeds and data within limits.
- NodeStarted emitted before execute; NodeCompleted/NodeFailed after.
- actions_executed_total incremented on every execute (success or failure); actions_failed_total on failure.
- ActionNotFound when key not in registry.
- DataLimitExceeded when output exceeds max_node_output_bytes and Reject.
- RuntimeError::is_retryable() true iff ActionError::is_retryable().

## Scenario Matrix

| Scenario | Coverage |
|----------|----------|
| Happy path | Handler succeeds; NodeCompleted; metrics |
| Unknown action | ActionNotFound |
| Handler fails | ActionError; NodeFailed; actions_failed_total |
| Data limit exceeded | DataLimitExceeded; NodeFailed |
| Telemetry | NodeStarted, NodeCompleted/NodeFailed; counter, histogram |
| Multiple subscribers | Event bus fan-out |
| Cancellation | Context cancel token; action should respect |

## Tooling

- **Property testing:** proptest for DataPassingPolicy check_output_size (various sizes).
- **Fuzzing:** Optional; action input fuzz.
- **Benchmarks:** execute_action latency (with no-op handler).
- **CI quality gates:** `cargo test -p nebula-runtime`; `cargo test -p nebula-engine`.

## Exit Criteria

- **Coverage goals:** Registry, data policy, runtime execute flow, error propagation, telemetry.
- **Flaky test budget:** Zero.
- **Performance regression:** execute_action overhead < 1ms (excluding handler).
