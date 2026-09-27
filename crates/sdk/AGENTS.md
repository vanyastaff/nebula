# nebula-sdk — Agent orientation
> Local guide for `crates/sdk/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** The sole supported and branded Rust façade, organized by persona. External one-dependency proofs cover typed action drafts, `simple_action!`, `WorkflowBuilder`, credential `TestResult`, and representative Action/Credential/Plugin/Resource/Schema/Validator derives; broader manual/prelude workflows, `client`, and `embedded` require workflow-specific proofs or remain explicit gaps. Integration authors do not treat implementation crates as supported substitutes.
**Layer:** API / Surfaces — the sole supported Rust façade; depends only downward.

## Commands

- Features: `default = ["derive", "testing"]`; `testing` gates `src/testing.rs` + `pub use tokio`; optional `http` enables the credential CRUD executor; optional `resource-http` enables the HTTP resource adapter (`integration::resource::http`). Check minimal, default, minimal + http and minimal + resource-http configurations.
- `cargo nextest run -p nebula-sdk --features resource-http --test resource_http` — the adapter against the shared raw TCP server (`tests/support/raw_http.rs`, also used by `credential_http`).
- `cargo nextest run -p nebula-sdk --test derive_external_contract --test public_perimeter_external_contract --test test_result_external_contract` — external consumer proofs; rerun for re-export or companion-macro changes, not just SDK source edits.
- `cargo check -p nebula-sdk --no-default-features` — minimal façade compilation; this does not replace the external consumer proofs.

## Key files

- `src/integration/resource/mod.rs` — curated trusted custom-topology authoring; the public-perimeter fixture compiles non-Clone provider/instance and lifecycle signatures using only SDK plus `async-trait`. This is authoring coverage, not a runtime lifecycle proof. Registration-local store mutation is trusted capability; Manager/Registry/ReleaseQueue remain excluded. Provider calls are authored against the managed call facade (`resource_rate_limit`, `resource_managed_logger` fixtures); the deprecated `Limited` closure family stays re-exported under `#[expect(deprecated)]` until its removal, and `resource_limited_deprecated` proves `wrap` is flagged. `ManagedRow` and the session vocabulary (`SessionProvider`, `SessionSpec`, `SessionCx`, `SessionEnd`, `SessionClosed`, `SessionBinding`, `SessionFuture`) are curated here too, not in the prelude; `resource_session` authors a session provider, `action_managed_row` derives an action whose `#[resource]` fields are rows (the supported route to a row; `ActionContextExt` stays in `__private`), `managed_row_no_deref` and `session_escape` are the negative probes, and `managed_row_has_no_deref` pins the rendered facade. The manager that builds a `ManagedRow` stays excluded. The streaming family (`StreamOperation`, `StreamSink`, `Streaming`, `ConsumerGone`) is curated alongside.
- `src/integration/resource/http/` (feature `resource-http`) — the HTTP resource adapter: `config.rs` (`HttpConfig`, auth-neutral `HttpTransport`), `request.rs` (method markers → `Effect`, `Request`), `auth.rs` (`HttpApi`, `Authorize`), `exchange.rs` (`send`, classification, the `Operation` impl and its re-attempt rule), `stream.rs` (`open_stream`). Never format a `reqwest::Error` (its `Display` carries the URL), never put a `reqwest`/`url` type in a public signature, and never re-attempt a `Write` after it may have been sent.

- `src/lib.rs` — curated persona modules, SDK `Error`, and `params!` / `workflow!` / `simple_action!` / `json!` macros; `__private` exists only for macro hygiene and is not an integration surface.
- `src/resource_contribution.rs` — sealed SDK-owned token and typed bridge for topology-bearing
  resource derives; it must never expose the erased leaf factory or registration authority.
- `src/prelude.rs` — one-stop `use nebula_sdk::prelude::*` set (action traits, schema, credential/OAuth2 types).
- `src/action.rs` — typed action and `ActionMetadataDraft` authoring contracts.
- `src/workflow.rs` — `WorkflowBuilder` (`add_node` / `connect` / `build`).
- `src/runtime.rs` — `TestRuntime`, `RunReport` in-process test harness.
- `src/testing.rs` — test helpers/fixtures (feature `testing`).

## Conventions & never-do

- This is a curated façade, not a crate-topology mirror. Its canonical target covers exactly the five §3.5 integration concepts (Action, Credential, Resource, Schema, Plugin), but maturity is per workflow. Public derives resolve through the hidden macro namespace; `#[doc(hidden)]` is not access control, so that namespace may expose only narrow authoring contracts or SDK-owned opaque bridges, never runtime authority. Do NOT expose implementation crates outside it or add a sixth integration concept without canon revision (§0.2).
- `prelude` / `WorkflowBuilder` / draft metadata are a public open-source contract (§4.4/§7): breaking changes need explicit announcement + migration, not drive-by edits.
- Not the engine/runtime or an expression evaluator. `nebula-resilience` is not currently curated; author demand for it is an SDK gap, not permission to import a Nebula leaf directly. Plugins remain trusted in-process adapters (ADR-0091), while the supported author contract must come through this SDK.
- External fixtures under `tests/fixtures/` pin exact package versions. A workspace version bump must update every affected pin, including renamed leaf dependencies, in the same change. Preserve the SDK-only dependency constraint of the SDK-only fixtures; never add a leaf dependency to make those proofs compile.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Curated exports and hidden namespaces | [public_perimeter_external_contract](tests/public_perimeter_external_contract.rs), [test_result_external_contract](tests/test_result_external_contract.rs); retain negative probes for forbidden imports. |
| Any public path or re-exported resource signature | [public_api_snapshot](tests/public_api_snapshot.rs): review the `.snap` diff, then `task sdk:api:bless`. It is review visibility, not a SemVer freeze; the walker's limits (macro-generated items, blanket/foreign impls) are in `tests/public_api/mod.rs`. |
| HTTP resource adapter (`resource-http`) | unit tests in `src/integration/resource/http/` (config validation, path rules, effect table, classification, re-attempt rule, auth shapes), [resource_http](tests/resource_http.rs) (real `Manager`, raw TCP server: redirects, secrecy, sent-state table, throttles, credentials, streams), perimeter fixtures `resource_http` / `http_no_raw_client`, and [public_api_snapshot](tests/public_api_snapshot.rs) (`the_http_adapter_leaks_no_client_type`). |
| Generated paths | [derive_external_contract](tests/derive_external_contract.rs) compiles and runs both SDK-only and renamed-leaf fixtures, including named resource field schemas and unit/null contracts; [simple_action_macro](tests/simple_action_macro.rs) covers the declarative helper. |

## See also

- `README.md` — full re-export list + maturity notes · canon [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §3.5/§4.4/§7, [docs/INTEGRATION_MODEL.md](../../docs/INTEGRATION_MODEL.md), [docs/MATURITY.md](../../docs/MATURITY.md).
