# nebula-sdk — Agent orientation
> Agent quick-map for `crates/sdk/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** The sole supported and branded Rust façade, organized by persona. The current external one-dependency proof covers `ActionBuilder`, `WorkflowBuilder`, and credential `TestResult`; broader manual/prelude workflows, `client`, `embedded`, and SDK-hygienic procedural derives require workflow-specific proofs or remain explicit gaps. Integration authors do not treat implementation crates as supported substitutes.
**Layer:** API/Public — depends only downward (root AGENTS.md → Layered Dependency Map).

## Commands
- `cargo check -p nebula-sdk`
- `cargo nextest run -p nebula-sdk`  ·  doctests: `cargo test -p nebula-sdk --doc`
- Features: `default = ["derive", "testing"]`; `testing` gates `src/testing.rs` + `pub use tokio`.

## Key files
- `src/lib.rs` — curated persona modules, SDK `Error`, and `params!` / `workflow!` / `simple_action!` / `json!` macros; `__private` exists only for macro hygiene and is not an integration surface.
- `src/prelude.rs` — one-stop `use nebula_sdk::prelude::*` set (action traits, schema, credential/OAuth2 types).
- `src/action.rs` — `ActionBuilder` (programmatic action metadata).
- `src/workflow.rs` — `WorkflowBuilder` (`add_node` / `connect` / `build`).
- `src/runtime.rs` — `TestRuntime`, `RunReport` in-process test harness.
- `src/testing.rs` — test helpers/fixtures (feature `testing`).

## Conventions & never-do
- This is a curated façade, not a crate-topology mirror. Its canonical target covers exactly the five §3.5 integration concepts (Action, Credential, Resource, Schema, Plugin), but maturity is per workflow: the current external proof covers builders/manual contracts, not every derive-based workflow. Do NOT expose implementation crates or add a sixth integration concept without canon revision (§0.2).
- `prelude` / `WorkflowBuilder` / `ActionBuilder` are a public open-source contract (§4.4/§7): breaking changes need explicit announcement + migration, not drive-by edits.
- Not the engine/runtime or an expression evaluator. `nebula-resilience` is not currently curated; author demand for it is an SDK gap, not permission to import a Nebula leaf directly. Plugins remain trusted in-process adapters (ADR-0091), while the supported author contract must come through this SDK.
- `anyhow` ergonomics are allowed for author scripts, but first-party lib code uses typed `thiserror`/`NebulaError`; no unwrap/expect/panic in lib code.
- Direct downward domain/port dependencies follow the root layer map; durable cross-crate commands/facts use persisted state or explicit outbox/inbox ports; nebula-eventbus carries only lossy observation and wake hints.

## See also
- `README.md` — full re-export list + maturity notes · canon `docs/PRODUCT_CANON.md` §3.5/§4.4/§7, `docs/INTEGRATION_MODEL.md`, `docs/MATURITY.md`.
