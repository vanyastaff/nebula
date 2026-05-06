# nebula-resilience Rules

> Area-specific conventions for `nebula-resilience` (Stability Patterns Pipeline).
> Loaded after `rules/base.md`. Applies to call sites that compose `ResiliencePipeline`,
> standalone patterns, and observability hooks. See `crates/resilience/README.md`
> and `crates/resilience/docs/` for the full contract.

## Rules

- `nebula-resilience` is the only retry surface in the workflow stack (canon §11.2) — the engine does not re-execute nodes, so retry, circuit breaking, and timeout for outbound calls live in `ResiliencePipeline` composed inside an action.
- Compose pipelines in the recommended order `load_shed → rate_limiter → timeout → retry → circuit_breaker → bulkhead` (first added = outermost).
- Place `timeout` outside `retry` so all attempts share one deadline; treat the `build()` warning about timeout-inside-retry as a bug, not a hint.
- Place `rate_limiter` outside `retry` so quota checks are not multiplied per attempt; treat the `build()` warning about rate-limiter-inside-retry as a bug.
- For config-driven pipelines call `build_checked()` (rejects unsafe order); use `build_recommended_order()` only when auto-sorting different policy kinds is explicitly acceptable; reserve raw `build()` for tests and one-off composition where order is statically obvious.
- Drive retry filtering through `nebula_error::Classify::retry_hint()` (or `RetryConfig::retry_if` / `PipelineBuilder::classifier` / `PipelineBuilder::classify_errors`) — never re-implement transient/permanent flags inside an action body.
- Operation errors are permanent by default in the retry step — to replay them configure `retry_if` / `with_classifier` / `classify_errors` explicitly at the call site.
- `CallError::CircuitOpen`, `LoadShed`, and `Cancelled` are permanent by default — do not mark them retryable.
- `CallError::RateLimited::retry_after` and operation `RetryHint::after` are delay floors — never sleep less than the requested wait, and never strip them when constructing custom backoff.
- Workflow runtime call sites must use `call_with_policy_context()` / `call_with_policy_context_and_fallback()` with a shared `PolicyContext`; `call_with_context` is only for cancellation-only paths outside the engine.
- Share `CircuitBreaker` and `Bulkhead` instances via `Arc` — one instance per logical downstream / resource, never one per call site or per task.
- Enable hedging only for operations marked `HedgeSafety::Idempotent`; never fan out non-idempotent calls and never enable duplicate hedge requests by default.
- Use the safe `fallback()` wrapper (or `FallbackOperation`) for graceful degradation — do not silently recover `Cancelled` or overload-class errors in custom `FallbackStrategy` implementations.
- Wire `MetricsSink` at the pattern level (`CircuitBreaker`, `Bulkhead`, `RetryConfig`, `HedgeExecutor`, `AdaptiveHedgeExecutor`, `TimeoutExecutor`, `FallbackOperation`) and at the pipeline level via `PipelineBuilder::with_sink`, combined with a `PolicyScope` attached through `.scope()` so `PipelineCompleted` carries low-cardinality scope.
- `MetricsSink::record` is invoked synchronously on the hot path — keep implementations allocation-light and offload heavy I/O to a background channel.
- Use `Gate::close_with_timeout()` for cooperative shutdown to get a typed timeout with active-guard count; never `Gate::close().await` in production paths without a bounded budget.
- Do not embed third-party rate limiters, retry libraries, or executors inside this crate — keep specialized adapters at integration boundaries (`nebula-action`, `nebula-sandbox`, `nebula-engine`).
- Boundary config and event types stay behind `feature = "serde"` (default-on); runtime executors, guards, sinks, callbacks, and caller error types stay outside serde because they carry live process state, not stable config/event data.
- Loom-checked invariants stay behind the `loom` feature and require `RUSTFLAGS="--cfg loom"` (`cargo test -p nebula-resilience --features loom --lib loom`); do not enable `loom` in regular CI runs.
- Preserve the caller's error type with `CallError<E>` — never map it into a foreign enum just to flatten variants; use `CallError::flat_map_inner()` when you need access to `E`.
