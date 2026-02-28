# Architecture

## Role in the Workspace

`nebula-core` is the canonical vocabulary layer for the whole platform.

Dependency direction should be:

`nebula-*` domain crates -> `nebula-core`

and not:

`nebula-core` -> domain crates

This keeps the graph acyclic and allows independent evolution of engine/runtime/storage/api layers.

## Internal Module Boundaries

- `id.rs`
  - strongly typed UUID wrappers via `domain-key`
  - compile-time separation between ID domains
- `scope.rs`
  - lifecycle and ownership hierarchy (`Global` -> `Organization` -> `Project` -> `Workflow` -> `Execution` -> `Action`)
  - `ScopedId` helper for scope-bound identifiers
- `traits.rs`
  - shared behavior contracts used across crates
  - includes `Scoped`, `HasContext`, `Identifiable`, `Validatable`, `Serializable`, metadata traits
- `types.rs`
  - shared data/value types and utility functions
  - versioning, status, priorities, operation metadata
- `error.rs`
  - `CoreError` classification and conversions from common std/library errors
- `keys.rs`
  - normalized and validated plugin/key types
- `constants.rs`
  - default values, limits, env var names, and reusable platform constants

## Design Constraints

- Foundation APIs must be stable and conservative.
- Types should be serde-friendly for API/storage boundaries.
- IDs should remain cheap-to-copy and misuse-resistant.
- Error surface should stay explicit and machine-classifiable.

## Non-Goals

- No business logic orchestration.
- No storage backend logic.
- No runtime scheduling.
- No transport/API framework concerns.

Those belong to other crates (`engine`, `runtime`, `storage`, `api`, etc.).
