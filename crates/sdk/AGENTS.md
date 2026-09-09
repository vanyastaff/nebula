# nebula-sdk — Agent orientation
> Local guide for `crates/sdk/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** The sole supported and branded Rust façade, organized by persona. External one-dependency proofs cover `ActionBuilder`, `WorkflowBuilder`, credential `TestResult`, and representative Action/Credential/Plugin/Resource/Schema/Validator derives; broader manual/prelude workflows, `client`, and `embedded` require workflow-specific proofs or remain explicit gaps. Integration authors do not treat implementation crates as supported substitutes.
**Layer:** API / Surfaces — the sole supported Rust façade; depends only downward.

## Commands

- Features: `default = ["derive", "testing"]`; `testing` gates `src/testing.rs` + `pub use tokio`.
- `cargo nextest run -p nebula-sdk --test derive_external_contract --test public_perimeter_external_contract --test test_result_external_contract` — external consumer proofs; rerun for re-export or companion-macro changes, not just SDK source edits.
- `cargo check -p nebula-sdk --no-default-features` — minimal façade compilation; this does not replace the external consumer proofs.

## Key files

- `src/lib.rs` — curated persona modules, SDK `Error`, and `params!` / `workflow!` / `simple_action!` / `json!` macros; `__private` exists only for macro hygiene and is not an integration surface.
- `src/prelude.rs` — one-stop `use nebula_sdk::prelude::*` set (action traits, schema, credential/OAuth2 types).
- `src/action.rs` — `ActionBuilder` (programmatic action metadata).
- `src/workflow.rs` — `WorkflowBuilder` (`add_node` / `connect` / `build`).
- `src/runtime.rs` — `TestRuntime`, `RunReport` in-process test harness.
- `src/testing.rs` — test helpers/fixtures (feature `testing`).

## Conventions & never-do

- This is a curated façade, not a crate-topology mirror. Its canonical target covers exactly the five §3.5 integration concepts (Action, Credential, Resource, Schema, Plugin), but maturity is per workflow. Public derives resolve through the hidden macro namespace; that namespace is implementation plumbing, not a supported persona. Do NOT expose implementation crates outside it or add a sixth integration concept without canon revision (§0.2).
- `prelude` / `WorkflowBuilder` / `ActionBuilder` are a public open-source contract (§4.4/§7): breaking changes need explicit announcement + migration, not drive-by edits.
- Not the engine/runtime or an expression evaluator. `nebula-resilience` is not currently curated; author demand for it is an SDK gap, not permission to import a Nebula leaf directly. Plugins remain trusted in-process adapters (ADR-0091), while the supported author contract must come through this SDK.
- External fixtures under `tests/fixtures/` pin exact package versions. A workspace version bump must update every affected pin, including renamed leaf dependencies, in the same change. Preserve the SDK-only dependency constraint of the SDK-only fixtures; never add a leaf dependency to make those proofs compile.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Curated exports and hidden namespaces | [public_perimeter_external_contract](tests/public_perimeter_external_contract.rs), [test_result_external_contract](tests/test_result_external_contract.rs); retain negative probes for forbidden imports. |
| Generated paths | [derive_external_contract](tests/derive_external_contract.rs) checks both SDK-only and renamed-leaf fixtures; [simple_action_macro](tests/simple_action_macro.rs) covers the declarative helper. |

## See also

- `README.md` — full re-export list + maturity notes · canon [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §3.5/§4.4/§7, [docs/INTEGRATION_MODEL.md](../../docs/INTEGRATION_MODEL.md), [docs/MATURITY.md](../../docs/MATURITY.md).
