# nebula-sdk — Claude Code orientation
> Agent quick-map for `crates/sdk/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Single-crate façade so integration authors `use nebula_sdk::prelude::*` and get the action/credential/resource/schema/workflow/plugin surface plus builders and a test harness — no hunting the dependency graph.
**Layer:** API/Public — depends only downward (root CLAUDE.md → Layered Dependency Map).

## Commands
- `cargo check -p nebula-sdk`
- `cargo nextest run -p nebula-sdk`  ·  doctests: `cargo test -p nebula-sdk --doc`
- Features: `default = ["derive", "testing"]`; `testing` gates `src/testing.rs` + `pub use tokio`.

## Key files
- `src/lib.rs` — full-crate re-exports (`nebula_action`, `nebula_credential`, …), SDK `Error`, and `params!` / `workflow!` / `simple_action!` / `json!` macros.
- `src/prelude.rs` — one-stop `use nebula_sdk::prelude::*` set (action traits, schema, credential/OAuth2 types).
- `src/action.rs` — `ActionBuilder` (programmatic action metadata).
- `src/workflow.rs` — `WorkflowBuilder` (`add_node` / `connect` / `build`).
- `src/runtime.rs` — `TestRuntime`, `RunReport` in-process test harness.
- `src/testing.rs` — test helpers/fixtures (feature `testing`).

## Conventions & never-do
- This is a re-export façade only: it covers exactly the five §3.5 concepts (Action, Credential, Resource, Schema, Plugin). Do NOT add a sixth integration concept or a parallel OAuth/credential type alias — those track `nebula-credential`; a new concept needs canon revision (§0.2).
- `prelude` / `WorkflowBuilder` / `ActionBuilder` are a public open-source contract (§4.4/§7): breaking changes need explicit announcement + migration, not drive-by edits.
- Not the engine/runtime, not an expression evaluator, not the plugin process entry point, and it does NOT re-export `nebula-resilience` — see `nebula-engine` / `nebula-expression` / `nebula-plugin-sdk`.
- `anyhow` ergonomics are allowed for author scripts, but first-party lib code uses typed `thiserror`/`NebulaError`; no unwrap/expect/panic in lib code.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.

## See also
- `README.md` — full re-export list + maturity notes · canon `docs/PRODUCT_CANON.md` §3.5/§4.4/§7, `docs/INTEGRATION_MODEL.md`, `docs/MATURITY.md`.
