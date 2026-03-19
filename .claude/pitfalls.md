# Pitfalls — Read Before Changing Anything

- **parameter stale docs**: `docs/crates/parameter/*.md` is old API — use `src/schema.rs` and `src/providers.rs`.

- **core cascade**: Trait changes in nebula-core cascade to all 25 dependents. New ID types = safe; trait changes require approval.

- **credential↔resource circular dep**: Never import directly between these crates. Use `EventBus<CredentialRotatedEvent>`.

- **resilience RetryFailure**: Inside `CircuitBreaker::execute()`, retry errors are `RetryFailure<E>`. Unwrap: `.map_err(|f| f.error)`.

- **resilience compose.rs**: `benches/compose.rs` is an API contract for `LayerStack`/`ResilienceLayer`. Update it after signature changes; verify with `cargo bench --no-run -p nebula-resilience`.

- **InProcessSandbox only**: No OS-process or WASM sandbox — that is Phase 3 (ADR 008).

- **EventBus is best-effort**: In-memory, no persistence (Phase 2). Events lost on overflow. Check `lagged_count()`.

- **MemoryStorage is test-only**: Data is lost on restart. Never use in production.

- **LoggerGuard must live**: Drop only on shutdown. Early drop silences all logging.
