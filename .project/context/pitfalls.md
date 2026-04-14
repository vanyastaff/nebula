# Pitfalls

- **core cascade**: Trait changes cascade to 13+ crates. New ID types = safe; trait changes require approval.
- **credential↔resource**: Never import directly. Use `EventBus<CredentialRotatedEvent>`.
- **resilience CallError**: Never unwrap — use `into_operation()` or `flat_map_inner()`.
- **resilience Duration**: Don't remove `.min()` caps before `from_secs_f64`.
- **resilience SlidingWindow**: Always evicts expired first. Don't re-add "only clean when full".
- **InProcessSandbox only**: No OS/WASM sandbox — Phase 3.
- **EventBus**: Best-effort, in-memory, events lost on overflow.
- **MemoryStorage**: Test-only. Never use in production.
- **LoggerGuard**: Must live until shutdown. Early drop silences logging.
- **PostgresStorage**: Not implemented. Only MemoryStorage works.
- **nebula-auth**: RFC phase — do not depend on it.
- **API JWT**: Placeholder — header presence-check only.
- **Gone crates**: nebula-app, nebula-value, nebula-memory — all removed, references stale.
- **`cargo deny` layers are convention-only**: `[bans.deny]=[]`. CI runs deny but only for advisories/licenses. Cross-layer deps won't fail CI — review must catch.
