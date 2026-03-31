# Pitfalls — Read Before Changing Anything

- **parameter stale docs**: `docs/crates/parameter/*.md` is old API — use `src/schema.rs` and `src/providers.rs`.

- **core cascade**: Trait changes in nebula-core cascade to all 25 dependents. New ID types = safe; trait changes require approval.

- **credential↔resource circular dep**: Never import directly between these crates. Use `EventBus<CredentialRotatedEvent>`.

- **resilience CallError**: All patterns return `CallError<E>`. Retries return `CallError::RetriesExhausted { attempts, last }`. Never unwrap — use `into_operation()` or `flat_map_inner()`.

- **resilience compose.rs**: `benches/compose.rs` is an API contract for `ResiliencePipeline`/`PipelineBuilder`. Update it after signature changes; verify with `cargo bench --no-run -p nebula-resilience`.

- **resilience Duration overflow**: `CircuitBreaker::effective_reset_timeout` and `HedgeExecutor` cap f64 before `Duration::from_secs_f64` to prevent panics. Don't remove the `.min()` caps.

- **resilience SlidingWindow**: `acquire()` always evicts expired entries before checking capacity. Don't re-add the "only clean when full" optimization — it breaks the sliding window invariant.

- **InProcessSandbox only**: No OS-process or WASM sandbox — that is Phase 3 (ADR 008).

- **EventBus is best-effort**: In-memory, no persistence (Phase 2). Events lost on overflow. Check `lagged_count()`.

- **MemoryStorage is test-only**: Data is lost on restart. Never use in production.

- **LoggerGuard must live**: Drop only on shutdown. Early drop silences all logging.
