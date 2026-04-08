# nebula-engine
Workflow execution orchestrator — frontier-based DAG scheduler, node dispatch.

## Invariants
- Delegates action execution to `ActionRuntime` — never runs actions directly.
- `EngineCredentialAccessor`: empty `allowed_keys` = **allow all** (passthrough). Non-empty = strict allowlist. `has()` checks allowlist AND resolver. Carries `action_id` for `SandboxViolation` attribution.
- `ResolverCredentialAccessor` deleted — all credential access goes through `EngineCredentialAccessor`.
- `EngineResourceAccessor::exists()` delegates to `acquire()` (both use `ScopeLevel::Global`).
- `credential_resolver: None` → noop. `resource_manager: None` → noop.
- `with_credential_resolver()` type-erases the resolver fn — engine stays non-generic.

## Key Decisions
- Frontier-based: nodes spawn when all incoming edges resolve (not level-by-level).
- No EventBus — metrics only. Events deferred until engine stabilizes.
- Budget enforced in `check_budget()`: `max_duration`, `max_output_bytes`, `max_total_retries`. Retry counter is infrastructure-only — no retry-loop mechanism yet.
- Error strategy via `handle_node_failure()`: FailFast cancels, ContinueOnError skips dependents, IgnoreErrors treats failure as null success.
- Disabled nodes (`enabled = false`): `mark_node_skipped()` + `process_outgoing_edges(…, None, None, …)` — successors run with null input. Do NOT use `propagate_skip` (that dead-activates edges, skipping successors too).

## Traps
- `pub(crate) resolver` — internal, don't expose.
- `ExecutionResult` (engine return) vs `ExecutionState` (persistent) — different types.
- `nebula-credential` is a direct dep (for `CredentialSnapshot`).

<!-- updated: 2026-04-07 — PR#229: remove ResolverCredentialAccessor, fix allowlist semantics, action_id attribution, exists() scope fix -->
