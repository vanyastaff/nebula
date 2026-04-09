# nebula-action
Action trait hierarchy and execution contract — Ports & Drivers architecture.

## Invariants
- Defines **what** actions are, not how the engine runs them. Concrete execution environments are in nebula-runtime.
- Actions may run concurrently. No mutable state in action struct fields — use `StatefulAction` for stateful loops.
- `Context` is always injected — never construct `ActionContext` directly inside an action.

## Key Decisions
- Action subtypes: `StatelessAction` (one-shot), `StatefulAction` (Continue/Break loop), `TriggerAction` (starts workflow), `ResourceAction` (branch-scoped DI setup/cleanup).
- `ActionDependencies` has two complementary pairs: `credential`/`resources` (trait-object, runtime injection) and `credential_keys`/`resource_keys` (typed keys, engine validation at registration). All four default to empty — no migration needed.
- `ActionRegistry` (`registry.rs`): keyed by `ActionKey`, supports multiple versions per key. `get()` → latest, `get_versioned(&InterfaceVersion)` → specific. `Send + Sync` — use `Arc<ActionRegistry>` for read-only sharing, `Arc<RwLock<_>>` for mutation after sharing.
- `credential_typed::<S>()` on `ActionContext`/`TriggerContext` consumes snapshot via `into_project::<S>()`, maps `SnapshotError` → `ActionError::Fatal`. Primary typed credential access path.
- `ErrorCode` enum (8 variants, `#[non_exhaustive]`) on `ActionError::Retryable` and `Fatal` — machine-readable classification for engine retry decisions (RateLimited, AuthExpired, UpstreamTimeout, etc.).
- `ActionResultExt` trait — `.retryable()?` and `.fatal()?` ergonomic conversion on any `Result<T, E>`. Also `_with_code()` variants for ErrorCode attachment.
- Error field: `Arc<anyhow::Error>` — preserves full error chain, Clone via Arc. Factory methods accept `impl Display + Debug + Send + Sync + 'static`.

## Traps
- `#[derive(Action)]` requires **unit structs** (no fields). Config goes in a separate injected type.
- `ActionError::retryable(...)` vs `ActionError::fatal(...)` — engine uses this to decide retry.
- `FnStatelessAction` / `stateless_fn()` for closure-based actions (testing and one-off use).

## Relations
- Depends on nebula-core, nebula-parameter. Used by nebula-engine, nebula-runtime, nebula-sdk.

<!-- reviewed: 2026-04-09 — Phase 10: ErrorCode, Arc<anyhow::Error>, ActionResultExt. Docs cleanup: stale files removed, consolidated into docs/plans/2026-04-08-action-v2-{spec,roadmap,examples}.md -->