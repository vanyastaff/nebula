# nebula-engine
Workflow execution orchestrator — frontier-based DAG scheduler, node dispatch.

## Invariants
- Delegates action execution to `ActionRuntime` — never runs actions directly.
- Blocked: credential/resource DI not wired yet.

## Key Decisions
- Frontier-based execution: nodes spawn when all incoming edges resolve (not level-by-level).
- No EventBus — metrics only. Events redesigned when engine stabilizes.
- Budget: `max_duration` and `max_output_bytes` checked before each dispatch. `max_total_retries` not enforced yet.
- Error strategy via `handle_node_failure()`: FailFast cancels, ContinueOnError skips dependents, IgnoreErrors treats failure as null success.

## Traps
- Blocked on nebula-resource stabilization — don't invest heavily until DI is stable.
- `pub(crate) resolver` is internal — don't expose without design review.
- `ExecutionResult` (engine return) vs `ExecutionState` (persistent state) — different types.

<!-- reviewed: 2026-04-06 -->
