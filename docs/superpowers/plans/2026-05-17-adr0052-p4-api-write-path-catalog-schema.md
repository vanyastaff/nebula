# ADR-0052 P4 — API write-path validation + catalog json_schema() + public projection (FINAL cascade phase) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the ADR-0052 cascade: the credential write path validates `data` against the credential type's resolved `ValidSchema` before persist (V2, closing a verified live fail-open); catalog endpoints expose the type's `json_schema()` (V3); a `nebula-api`-owned mapper strips cross-field predicate logic from the public schema (#6); ADR-0047 + ADR-0052 P4 amendments.

**Architecture:** A `nebula-api`-owned object-safe port trait `CredentialSchemaPort` (api-safe types only) is added to `AppState` as `Option<Arc<dyn CredentialSchemaPort>>`, mirroring the verified `action_registry` precedent (None → honest 503, §4.5). The concrete impl lives in the composition root (`apps/server` + the `examples` server + the api test harness) holding a `nebula-credential` `CredentialRegistry`; it resolves `credential_key → CredentialMetadata.base.schema (ValidSchema)`, runs `FieldValues::from_json` + `ValidSchema::validate` (authority = validator; INTEGRATION_MODEL §29/§33 signatures unchanged), and produces `json_schema()` (`schemars` feature enabled on the composition crate's `nebula-schema` dep). `nebula-api` never gains a `nebula-schema` *production* dep nor a `ValidSchema` in any DTO; only `serde_json::Value` / api-owned structs cross the seam. Public projection strips `x-nebula-root-rules` + predicate-bearing `x-nebula-*` mode operands; standard JSON-Schema keywords stay (public contract).

**Tech Stack:** Rust 1.95 (edition 2024), axum 0.8 + `utoipa` (ADR-0047 OpenAPI 3.1), `nebula-credential` `CredentialRegistry`/`AnyCredential`, `nebula-schema` `ValidSchema::validate`/`json_schema()` (`schemars`), `nebula-validator` `ValidationErrors`, `cargo nextest`, lefthook full-workspace-clippy-per-commit gate.

---

## Authority & corrected facts (read before any step — the design spec is pre-#671/pre-P1-P3 and is FACTUALLY WRONG on 5 points; build on the corrected facts, exactly as P1's plan did)

Scope source of truth: `docs/superpowers/specs/2026-05-15-nebula-schema-finalization-design.md` §Q2 (lines 213-242) + §"Public projection" + Phasing P4 (lines 339-341); `docs/adr/0052-schema-validator-condition-seam.md` Amendment 2026-05-17 (P3) closing paragraph (lines 183-185); P3 plan "P4 backlog" (`docs/superpowers/plans/2026-05-17-adr0052-p3-hasschema-convergence.md:583`). P4 = exactly: (a) write-path validates `data` vs resolved `ValidSchema` before persist [V2]; (b) catalog populates schema from `ValidSchema::json_schema()` [V3]; (c) public OpenAPI DTO strips `x-nebula-root-rules` + predicate operands [#6]; (d) one-paragraph in-place ADR-0047 amendment + ADR-0052 P4 amendment + seam test, same PR (canon §0.1/§17). **NOT P4:** spec §"#3 JSON-Schema export hardening" / §"#4 ReDoS" / §"the one real panic" were **P2** scope (Phasing P2, line 335-336) — out of P4. **Non-goal, stays OPEN:** `slot_bindings` confused-deputy (credential resolution stays confused-deputy-exposed after P4).

**Corrected facts (verified against `main`@da3e0a13; the spec's pre-#671 line refs are dead):**

1. **There is a verified LIVE fail-open / false-capability (not a benign TODO).** `crates/api/src/transport/credential.rs:278-325` `create_credential` does `encode_secret_data(&req.data)` → `StoredCredential{ data: secret_bytes }` → `oauth_credential_store.put(...)` with **zero schema validation**. `crates/api/src/domain/credential/handler.rs:76` docstring claims "The `data` field is validated against the credential type's schema before encryption and storage." The doc asserts a security property that does not hold (§4.5 [L1] + §10 [L2]). V2 must make the doc true; the docstring is fixed in the same PR.
2. **`schemars` is enabled by NO crate** (`crates/schema/Cargo.toml:16` `schemars = ["dep:schemars"]` optional; lines 69/74 `required-features` are schema's own tests). `ValidSchema::json_schema()` is `#![cfg(feature="schemars")]` (`crates/schema/src/json_schema.rs:6`) — compiles into no workspace build today. V3 requires enabling `schemars` on the composition crate's `nebula-schema` dep. `deny.toml:104` already `ignored = ["schemars"]` — this is a Cargo feature-enable, **not a new crate and not a `deny.toml` change**.
3. **`crates/api/Cargo.toml` line ~104 has `nebula-schema = { path = "../schema" }` as a DEV-dependency only** (in `[dev-dependencies]`; zero `use nebula_schema` in `crates/api/src/**`). The spec's "nebula-api never imports nebula-schema" is an ADR-0047 *design preference*, not a deny.toml rule (`nebula-schema` is Core, freely importable; `deny.toml:139-149` already allows `nebula-api → nebula-credential`). Keep api free of a `nebula-schema` *production* dep and of `ValidSchema` in DTOs — the port trait carries only `serde_json::Value`/api-owned types.
4. **No engine credential registry, no `AppState` credential port.** `list_credential_types`/`get_credential_type` are honest 503s (`crates/api/src/transport/credential.rs:607-625`). The contract-side `nebula_credential::CredentialRegistry` (`crates/credential/src/contract/registry.rs`) holds `Box<dyn AnyCredential>`; `AnyCredential::metadata() -> CredentialMetadata` (`crates/credential/src/contract/any.rs:27`) and post-P3 `CredentialMetadata.base.schema: ValidSchema` (`crates/credential/src/metadata.rs` + `crates/metadata/src/base.rs:39`). The registry is not wired into `AppState`.
5. **No `json_schema()`-over-a-port precedent** — the action catalog uses hand-wrapped `utoipa::ToSchema` DTOs. V3 is net-new plumbing.

**Verified signatures the plan codes against (confirm the few flagged name-items against source at the step that needs them — name confirmation, not design freedom):**

- `crates/api/src/state.rs:205` `pub struct AppState`; `:239` `pub action_registry: Option<Arc<ActionRegistry>>`; ctor (`:346`) sets `action_registry: None`; `:384` `pub fn with_action_registry(mut self, registry: Arc<ActionRegistry>) -> Self`. **Mirror this exactly for `credential_schema`.**
- Composition root: `apps/server/src/compose.rs:103` `let mut state = default_state(&api_config)?;` then chained `state = state.with_idempotency_store(...)` (line 113). `apps/server/src/transport.rs:18` `use nebula_api::{ApiConfig, AppState, build_app};`.
- `crates/api/src/error/classify.rs:8` `use nebula_validator::foundation::{ValidationError, ValidationErrors};`; `:123` `impl From<ValidationError> for ApiError`; `:137` `impl From<ValidationErrors> for ApiError`; `ApiError::Validation { detail, errors: Vec<ValidationFieldError> }` (`crates/api/src/error/problem.rs:41-48`, `classify.rs:133/144`). **api already depends on `nebula-validator` (Core) and already maps validator errors to a secret-safe 422** (P2 made validator errors path+code+message, no values).
- `crates/schema/src/validated.rs:332` `pub fn validate(&self, values: &FieldValues) -> Result<ValidValues, ValidationReport>`. `crates/schema/src/value.rs:~325` `pub fn from_json(value: serde_json::Value) -> Result<FieldValues, crate::error::ValidationError>`. `crates/schema/src/json_schema.rs:81` `pub fn json_schema(&self) -> Result<schemars::Schema, JsonSchemaExportError>`; `:115` emits `root.insert("x-nebula-root-rules".to_owned(), Value::Array(serialized))`; `:454-524` emits `x-nebula-{field-kind,expression-mode,required-mode,visibility-mode,…}`; `:352-397` `apply_value_rules` emits standard JSON-Schema keywords (`minLength`/`maxLength`/`pattern`/`enum`/…).
- `crates/credential/src/contract/any.rs:23-48` `pub trait AnyCredential { fn credential_key(&self)->&str; fn metadata(&self)->CredentialMetadata; fn as_any(&self)->&dyn Any; }`. `crates/credential/src/contract/registry.rs` `CredentialRegistry` with `resolve_any(key)->Option<&dyn AnyCredential>` + an iterator over entries (confirm exact iterator/accessor names at Task 5).
- `crates/api/src/domain/credential/dto.rs:31-45` `CreateCredentialRequest { credential_key:String, name:String, description:Option<String>, data:serde_json::Value, tags:Option<HashMap<String,String>> }`; `:251-272` `CredentialTypeInfo { key, name, description, auth_pattern:String, capabilities:CredentialCapabilities, schema:serde_json::Value, icon:Option<String>, documentation_url:Option<String> }`.
- OpenAPI honesty tests: `crates/api/tests/openapi_canon_compliance.rs` (§4.5 stub-honesty: deprecated⇒501; probes honest 503/501 stubs), `crates/api/tests/openapi_spec.rs` (3.1.0, operationId uniqueness, $ref resolution — no committed JSON snapshot; spec generated in-process), `crates/api/tests/openapi_secret_redaction.rs`. Confirm at Task 9 whether credential-type endpoints are in the honest-503 inventory (their 503→200 flip must be reflected there).

> **Open name-confirmations the implementer MUST verify against source at the flagged step (P1/P2/P3 discipline — confirm, do not invent):** (i) exact `ValidationReport` public accessor for per-error `{path, code, message}` (P3 used `report.errors()` with `e.code.as_ref()` / `e.path.to_string()` in `crates/credential/tests/properties_pipeline.rs` — confirm there); (ii) exact `CredentialRegistry` enumeration accessor (iter of `&dyn AnyCredential` or `(key, entry)`) in `crates/credential/src/contract/registry.rs`; (iii) whether an `update_credential` write path exists in `transport/credential.rs` that also needs the V2 gate; (iv) exact `ApiError::Validation` construction path / `ValidationFieldError` fields (`crates/api/src/error/problem.rs:46-48`); (v) `default_state`/`build_app` location for the api test harness wiring; (vi) `crates/schema/src/value.rs` exact `from_json` line + error type name.

---

## File structure / change map

| Area | File | Responsibility |
|---|---|---|
| Port trait (api-owned) | `crates/api/src/ports/credential_schema.rs` (new; add `pub mod ports;`/entry per existing module convention — confirm api module layout) | `pub trait CredentialSchemaPort: Send + Sync` — object-safe; `validate_data(&self, credential_key:&str, data:&serde_json::Value) -> Result<(), Vec<CredentialFieldError>>`; `list_types(&self) -> Vec<CredentialTypeDescriptor>`; `get_type(&self, key:&str) -> Option<CredentialTypeDescriptor>`. `CredentialFieldError { path:String, code:String, message:String }` + `CredentialTypeDescriptor { key,name,description,auth_pattern,capabilities,icon,documentation_url, schema_json: serde_json::Value }` — all api-safe. |
| AppState | `crates/api/src/state.rs` | add `pub credential_schema: Option<Arc<dyn CredentialSchemaPort>>` (ctor `None`, mirror line 239/346); `with_credential_schema(self, Arc<dyn CredentialSchemaPort>) -> Self` (mirror line 384) |
| V2 write path | `crates/api/src/transport/credential.rs` (`create_credential` + any `update_credential`), `crates/api/src/domain/credential/handler.rs:76` docstring | validate `data` via port before persist; map `Vec<CredentialFieldError>` → `ApiError::Validation` (422, secret-safe); `None` → 503 "credential data validation unavailable" (no silent unvalidated persist); fix the docstring to state validation occurs when configured |
| V3 catalog | `crates/api/src/transport/credential.rs:607-625` (`list_credential_types`/`get_credential_type`) | replace 503 bodies: port `Some` → populated `CredentialTypeInfo` (schema = projected `schema_json`); `None` → unchanged honest 503 |
| #6 projection | `crates/api/src/ports/credential_schema.rs` or a sibling `crates/api/src/domain/credential/schema_projection.rs` (api-owned) | strip `x-nebula-root-rules` + predicate-bearing `x-nebula-{required,visibility}-mode` "when" operands from the `serde_json::Value`; keep standard JSON-Schema keywords |
| Composition impl | `apps/server/src/compose.rs` (+ `examples/` server + api test harness) ; `apps/server/Cargo.toml` (or wherever the impl lives) `nebula-schema = { features=["schemars"] }` | concrete `CredentialSchemaPort` over a `CredentialRegistry`: resolve key → `metadata().base.schema` → `FieldValues::from_json` + `ValidSchema::validate` (V2) and `json_schema()` (V3); `state.with_credential_schema(...)` |
| Seam tests | `crates/api/tests/seam_credential_write_path_validation.rs` (new), `crates/api/tests/seam_credential_catalog_schema_projection.rs` (new) | V2: invalid `data` → 422 with path/code, response body contains NO submitted value; valid → persists; port `None` → 503. V3/#6: catalog `schema` populated, contains NO `x-nebula-root-rules`/predicate operands; OpenAPI honesty updated |
| ADR / docs | `docs/adr/0047-openapi-31-generator.md` (one-paragraph in-place amendment), `docs/adr/0052-schema-validator-condition-seam.md` (P4 amendment + cascade close-out), `crates/api/README.md`/rustdoc, this plan | canon §0.1/§17; record cascade complete + `slot_bindings` Non-goal still OPEN |

**Commit boundaries (lefthook runs full-workspace `cargo clippy --workspace --all-targets -q -- -D warnings` every commit ⇒ commit only at workspace-green; coarse atomic commits; per-crate `cargo fmt -p <crate>` — NEVER `cargo fmt --all`/`task fmt` on this worktree path, Windows os error 206):**
- C0 plan doc.
- C1 port trait + DTOs + `AppState` field/builder (api-only addition; workspace green).
- C2 V2 write-path gate through the port + docstring fix + seam test (api test supplies a test port impl; `None`→503; workspace green).
- C3 V3 catalog via port (replace 503 bodies; `None`→503 unchanged; workspace green).
- C4 #6 public-projection mapper + its unit/seam test (api-owned; workspace green).
- C5 composition-root concrete impl + `schemars` enable + `with_credential_schema` wiring (apps/server + examples + api test harness; workspace green; flips relevant OpenAPI-honesty expectations).
- C6 ADR-0047 in-place amendment + ADR-0052 P4 amendment + cascade close-out + README/rustdoc.
All commits via `bash scripts/worktree.sh commit feat api "<summary>"` (convco `feat(api): …` / `docs(api): …`); PR title `feat(api): …` (no `!` — additive: a previously-503 endpoint becoming 200 and a previously-unvalidated path becoming validated are not breaking removals; confirm at PR time).

---

## Task 0: Commit this plan

**Files:** Create `docs/superpowers/plans/2026-05-17-adr0052-p4-api-write-path-catalog-schema.md` (this file).

- [ ] **Step 1: Stage and commit**

```bash
cd C:/Users/vanya/RustroverProjects/nebula/.worktrees/adr0052-p4
git add docs/superpowers/plans/2026-05-17-adr0052-p4-api-write-path-catalog-schema.md
bash scripts/worktree.sh commit docs api "ADR-0052 P4 API write-path + catalog schema plan"
```
Expected: convco accepts `docs(api): …`; lefthook pre-commit passes (no code → clippy/fmt skip).

---

## Task 1: `CredentialSchemaPort` trait + api-safe DTOs + `AppState` wiring (api-only addition)

**Files:** Create `crates/api/src/ports/credential_schema.rs`; Modify `crates/api/src/lib.rs` (or `crates/api/src/ports/mod.rs` — confirm api module layout: `rg -n "pub mod " crates/api/src/lib.rs`); Modify `crates/api/src/state.rs:~205-395`; Test inline `#[cfg(test)]` in `state.rs`.

- [ ] **Step 1: Write the failing test** (append to `crates/api/src/state.rs` `#[cfg(test)] mod tests`, or create one mirroring the existing AppState test pattern — `rg -n "mod tests" crates/api/src/state.rs`):

```rust
#[test]
fn appstate_credential_schema_defaults_none_and_builder_sets_it() {
    use std::sync::Arc;
    use crate::ports::credential_schema::{CredentialSchemaPort, CredentialFieldError, CredentialTypeDescriptor};
    struct StubPort;
    impl CredentialSchemaPort for StubPort {
        fn validate_data(&self, _k: &str, _d: &serde_json::Value) -> Result<(), Vec<CredentialFieldError>> { Ok(()) }
        fn list_types(&self) -> Vec<CredentialTypeDescriptor> { Vec::new() }
        fn get_type(&self, _k: &str) -> Option<CredentialTypeDescriptor> { None }
    }
    let st = AppState::for_test(); // confirm the existing test-ctor name via `rg -n "fn for_test|fn test_state|default_state" crates/api/src/state.rs`
    assert!(st.credential_schema.is_none());
    let st = st.with_credential_schema(Arc::new(StubPort));
    assert!(st.credential_schema.is_some());
}
```

- [ ] **Step 2: Run — expect FAIL** `cd C:/Users/vanya/RustroverProjects/nebula/.worktrees/adr0052-p4 && cargo test -p nebula-api --lib appstate_credential_schema_defaults_none_and_builder_sets_it 2>&1 | tail -15` → Expected: cannot find module `ports`/`credential_schema` / field `credential_schema`.

- [ ] **Step 3: Create the port module** `crates/api/src/ports/credential_schema.rs`:

```rust
//! API-owned credential-schema port (ADR-0052 P4).
//!
//! The api never imports `nebula-schema`/`nebula-validator` *types* into
//! DTOs; this object-safe port carries only api-safe values. The concrete
//! impl lives in the composition root (which legally depends on
//! `nebula-credential`/`nebula-schema`) and runs `ValidSchema::validate` /
//! `json_schema()` — authority sits with the validator
//! (INTEGRATION_MODEL §29/§33 unchanged).

use serde::Serialize;
use utoipa::ToSchema;

/// One field-level validation failure, secret-safe (RFC-6901 path +
/// validator code + static message — never the submitted value).
#[derive(Debug, Clone)]
pub struct CredentialFieldError {
    pub path: String,
    pub code: String,
    pub message: String,
}

/// Catalog descriptor for one credential type. `schema_json` is the
/// public-projected JSON Schema (`x-nebula-root-rules` + predicate
/// operands already stripped).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CredentialTypeDescriptor {
    pub key: String,
    pub name: String,
    pub description: String,
    pub auth_pattern: String,
    pub icon: Option<String>,
    pub documentation_url: Option<String>,
    pub schema_json: serde_json::Value,
}

/// Resolve a credential type's schema for the write-path gate (V2) and the
/// catalog read-model (V3). `None` wiring ⇒ handlers return honest 503
/// (mirrors `AppState::action_registry`, canon §4.5).
pub trait CredentialSchemaPort: Send + Sync + 'static {
    /// Validate `data` against the type's resolved schema before persist.
    /// `Err` is a secret-safe field-error list.
    fn validate_data(
        &self,
        credential_key: &str,
        data: &serde_json::Value,
    ) -> Result<(), Vec<CredentialFieldError>>;

    /// All known credential types with public-projected schema.
    fn list_types(&self) -> Vec<CredentialTypeDescriptor>;

    /// One credential type by key.
    fn get_type(&self, credential_key: &str) -> Option<CredentialTypeDescriptor>;
}
```

Add the module: in `crates/api/src/lib.rs` add `pub mod ports;` (and create `crates/api/src/ports/mod.rs` with `pub mod credential_schema;`) — match the existing module declaration style (confirm: `rg -n "^pub mod|^mod " crates/api/src/lib.rs | head`).

- [ ] **Step 4: Wire `AppState`** in `crates/api/src/state.rs`: add field beside `action_registry` (after line ~243):

```rust
    /// Optional credential-schema port (ADR-0052 P4). When `None`, the
    /// credential write path and credential-type catalog return 503.
    pub credential_schema: Option<std::sync::Arc<dyn crate::ports::credential_schema::CredentialSchemaPort>>,
```
In the ctor (the block at ~line 346 with `action_registry: None,`) add `credential_schema: None,`. Add the builder beside `with_action_registry` (~line 384):

```rust
    /// Attach the credential-schema port (ADR-0052 P4).
    #[must_use]
    pub fn with_credential_schema(
        mut self,
        port: std::sync::Arc<dyn crate::ports::credential_schema::CredentialSchemaPort>,
    ) -> Self {
        self.credential_schema = Some(port);
        self
    }
```

- [ ] **Step 5: Run — expect PASS** `cargo test -p nebula-api --lib appstate_credential_schema_defaults_none_and_builder_sets_it 2>&1 | tail -8` → PASS.

- [ ] **Step 6: Workspace-green + commit (C1)**

```bash
cargo clippy --workspace --all-targets -q -- -D warnings 2>&1 | tail -5
cargo fmt -p nebula-api
git add crates/api
bash scripts/worktree.sh commit feat api "add CredentialSchemaPort + AppState wiring (ADR-0052 P4)"
```
Expected: clippy clean workspace-wide (pure api addition). `CredentialFieldError` may trip a dead-code lint until Task 2 uses it — if clippy flags it, proceed to Task 2 in the SAME commit (do not `#[allow]`); restructure C1/C2 into one commit if needed to stay green.

---

## Task 2: V2 — write-path validation through the port + close the fail-open

**Files:** Modify `crates/api/src/transport/credential.rs` (`create_credential` ~278-325, and `update_credential` if present — confirm `rg -n "pub async fn (create|update)_credential" crates/api/src/transport/credential.rs`); Modify `crates/api/src/domain/credential/handler.rs:~76` (docstring); `crates/api/src/error/` (reuse `ApiError::Validation`); Test `crates/api/tests/seam_credential_write_path_validation.rs` (new).

- [ ] **Step 1: Write the failing seam test** `crates/api/tests/seam_credential_write_path_validation.rs` (mirror an existing api integration test harness — `rg -n "build_app|default_state|AppState::for_test|router\(\)" crates/api/tests/*.rs | head`; use a test `CredentialSchemaPort` impl that rejects when `data` lacks key `"api_key"`):

```rust
//! ADR-0052 P4 seam (V2): credential `data` is validated against the type's
//! schema before persist; rejection is secret-safe (no submitted value in
//! the response); an unconfigured port yields 503, never silent
//! unvalidated persist.
// ... build AppState with a stub CredentialSchemaPort whose validate_data
// returns Err([CredentialFieldError{path:"/api_key",code:"required",message:"required"}])
// for data missing "api_key", Ok otherwise ...

#[tokio::test]
async fn create_credential_rejects_invalid_data_secret_safe() {
    // POST /…/credentials with data = {"server":"x","secret":"SUPERSECRET"} (no api_key)
    // assert: 422; body JSON contains "/api_key" and "required";
    // assert: body string does NOT contain "SUPERSECRET" (no value echo).
}

#[tokio::test]
async fn create_credential_persists_valid_data() {
    // data = {"api_key":"k"} → 201/200; subsequent GET returns metadata.
}

#[tokio::test]
async fn create_credential_503_when_port_unconfigured() {
    // AppState without with_credential_schema → POST → 503 (NOT 200/persisted).
}
```
(Fill the harness from the confirmed existing pattern; keep assertions exact.)

- [ ] **Step 2: Run — expect FAIL** `cargo test -p nebula-api --test seam_credential_write_path_validation 2>&1 | tail -15` → FAIL (validation not wired; currently persists unvalidated).

- [ ] **Step 3: Gate the write path.** In `crates/api/src/transport/credential.rs` `create_credential`, BEFORE `encode_secret_data(&req.data)`:

```rust
    match state.credential_schema.as_ref() {
        Some(port) => {
            if let Err(field_errors) = port.validate_data(&req.credential_key, &req.data) {
                return Err(api_error_from_credential_field_errors(field_errors));
            }
        }
        None => {
            return Err(ApiError::ServiceUnavailable(
                "credential data validation unavailable: no credential-schema port configured".into(),
            ));
        }
    }
```
Add a small mapper near the api error module (secret-safe — only path/code/message):

```rust
fn api_error_from_credential_field_errors(
    errs: Vec<crate::ports::credential_schema::CredentialFieldError>,
) -> ApiError {
    let errors = errs
        .into_iter()
        .map(|e| crate::error::problem::ValidationFieldError {
            // confirm exact ValidationFieldError fields at this step
            field: e.path,
            code: Some(e.code),
            message: e.message,
        })
        .collect();
    ApiError::Validation { detail: "credential data failed schema validation".into(), errors }
}
```
(Confirm `ValidationFieldError` field names + `ApiError::Validation` shape against `crates/api/src/error/problem.rs:46-48` / `classify.rs:133` at this step; adjust to the real constructor — there may be an existing `ApiError::validation(...)` helper to reuse instead of constructing the variant directly.) Apply the identical gate to `update_credential` if it exists. Fix `crates/api/src/domain/credential/handler.rs:~76` docstring to: "When a credential-schema port is configured, `data` is validated against the credential type's schema before encryption and storage; if no validator is configured the request is rejected with 503 (data is never persisted unvalidated)."

- [ ] **Step 4: Run — expect PASS** `cargo test -p nebula-api --test seam_credential_write_path_validation 2>&1 | tail -10` → all 3 PASS.

- [ ] **Step 5: Workspace-green + commit (C2)**

```bash
cargo clippy --workspace --all-targets -q -- -D warnings 2>&1 | tail -5
cargo nextest run -p nebula-api 2>&1 | tail -8
cargo fmt -p nebula-api
git add crates/api
bash scripts/worktree.sh commit feat api "validate credential data before persist; close fail-open (ADR-0052 P4 V2)"
```
Expected: clippy clean; api nextest green (the 503-on-None change may flip existing create_credential tests that asserted 200 without a port — update those tests to wire a stub port or assert the new honest 503; that is a correct consequence of closing the fail-open, not a regression — note each changed test).

---

## Task 3: V3 — catalog endpoints via the port

**Files:** Modify `crates/api/src/transport/credential.rs:607-625` (`list_credential_types`/`get_credential_type`); Test in `crates/api/tests/seam_credential_catalog_schema_projection.rs` (new — V3 part; #6 part added in Task 4).

- [ ] **Step 1: Write the failing test** (catalog populated when port present, honest 503 when absent):

```rust
#[tokio::test]
async fn list_credential_types_populated_when_port_present() {
    // AppState.with_credential_schema(stub yielding one descriptor key="api_key", schema_json={"type":"object",...})
    // GET /credential-types → 200; body[0].key=="api_key"; body[0].schema is the object.
}
#[tokio::test]
async fn get_credential_type_503_when_port_absent() {
    // no port → GET /credential-types/api_key → 503 (unchanged honest stub).
}
```
(Confirm the exact route paths from `crates/api/src/domain/credential/routes.rs:16-17` + handler param names.)

- [ ] **Step 2: Run — expect FAIL** `cargo test -p nebula-api --test seam_credential_catalog_schema_projection 2>&1 | tail -12` → FAIL (still 503 unconditionally).

- [ ] **Step 3: Implement.** Replace the two 503 bodies in `crates/api/src/transport/credential.rs`:

```rust
pub async fn list_credential_types(state: &AppState) -> ApiResult<ListCredentialTypesResponse> {
    let port = state.credential_schema.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "credential type discovery requires a credential-schema port (not configured)".into(),
        )
    })?;
    let types = port.list_types().into_iter().map(credential_type_info_from_descriptor).collect();
    Ok(ListCredentialTypesResponse { credential_types: types }) // confirm response field name vs dto.rs
}

pub async fn get_credential_type(state: &AppState, key: &str) -> ApiResult<CredentialTypeInfo> {
    let port = state.credential_schema.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "credential type discovery requires a credential-schema port (not configured)".into(),
        )
    })?;
    port.get_type(key)
        .map(credential_type_info_from_descriptor)
        .ok_or_else(|| ApiError::NotFound(format!("unknown credential type: {key}")))
}
```
Add `fn credential_type_info_from_descriptor(d: CredentialTypeDescriptor) -> CredentialTypeInfo` mapping descriptor → the existing `CredentialTypeInfo` DTO (`dto.rs:251-272`); `schema` ← `d.schema_json`; `capabilities` ← derive/Default (confirm `CredentialCapabilities` source — the descriptor may need a `capabilities` field; if so add it to `CredentialTypeDescriptor` in Task 1's struct and the Task 1 test — keep types consistent across tasks).

- [ ] **Step 4: Run — expect PASS**; **Step 5: Workspace-green + commit (C3)** `bash scripts/worktree.sh commit feat api "populate credential-type catalog via port (ADR-0052 P4 V3)"`.

---

## Task 4: #6 — public-projection mapper (api-owned) + seam

**Files:** Create `crates/api/src/domain/credential/schema_projection.rs` (api-owned); Modify the port-descriptor consumer so the catalog `schema` is projected; Test `crates/api/tests/seam_credential_catalog_schema_projection.rs` (extend).

- [ ] **Step 1: Failing unit test** for `project_public_schema(serde_json::Value) -> serde_json::Value`:

```rust
#[test]
fn project_strips_root_rules_and_predicate_operands_keeps_standard_keywords() {
    let raw = serde_json::json!({
        "type":"object",
        "properties": { "api_key": { "type":"string", "minLength": 2,
            "x-nebula-required-mode": {"when": {"predicate": "…"}},
            "x-nebula-field-kind":"secret" } },
        "x-nebula-root-rules": [ {"predicate":"…"} ],
        "additionalProperties": false
    });
    let p = super::project_public_schema(raw);
    assert!(p.get("x-nebula-root-rules").is_none());
    let ak = &p["properties"]["api_key"];
    assert!(ak.get("x-nebula-required-mode").is_none(), "predicate-bearing mode operand stripped");
    assert_eq!(ak["minLength"], 2, "standard JSON-Schema keyword kept");
    assert_eq!(ak["type"], "string");
}
```

- [ ] **Step 2: Run — expect FAIL** (function absent).

- [ ] **Step 3: Implement** `project_public_schema`: recursively remove the top-level `"x-nebula-root-rules"` key and every `"x-nebula-required-mode"`/`"x-nebula-visibility-mode"` whose value is an object containing a `"when"`/predicate (the predicate-bearing variants — confirm exact emitted shapes against `crates/schema/src/json_schema.rs:454-524` semantics); KEEP standard JSON-Schema keywords and non-predicate `x-nebula-*` structural hints (`x-nebula-field-kind`, `x-nebula-expression-mode`, etc. — these are not cross-field predicate logic). Pin the precise strip set in a top-of-file doc comment citing `json_schema.rs:115` + the mode-operand lines. Route every `CredentialTypeDescriptor.schema_json` through this before it reaches the wire (in `credential_type_info_from_descriptor`, or have the port impl pre-project — decision: api-owned per spec, so project in api at the DTO boundary).

- [ ] **Step 4: Run — expect PASS**; add an integration assertion in the catalog seam test that a wired port whose raw `json_schema()` contains `x-nebula-root-rules` yields a catalog `schema` with **no** `x-nebula-root-rules` and no predicate operands.

- [ ] **Step 5: Workspace-green + commit (C4)** `bash scripts/worktree.sh commit feat api "api-owned public schema projection strips predicate logic (ADR-0052 P4 #6)"`.

---

## Task 5: Composition-root concrete `CredentialSchemaPort` impl + `schemars` enable

**Files:** Create the concrete impl (recommended: `apps/server/src/credential_schema.rs` — confirm apps/server module layout) holding a `nebula_credential::CredentialRegistry`; Modify `apps/server/src/compose.rs:~103-113` (chain `.with_credential_schema(...)`); Modify `apps/server/Cargo.toml` to enable `nebula-schema = { path="../../crates/schema", features=["schemars"] }` (confirm exact dep spec/path); mirror for the `examples` server + the api test harness composition; Test: extend the api seam tests to use the REAL impl over a tiny in-test `CredentialRegistry` if feasible, else keep stub + add an `apps/server` (or `examples`) integration test exercising the real impl.

- [ ] **Step 1: Failing test** — an integration test that builds the real impl over a `CredentialRegistry` containing one registered first-party credential (e.g. `ApiKeyCredential`) and asserts: `validate_data("api_key", {"api_key":"k"})` → Ok; `validate_data("api_key", {})` → Err with path/code; `get_type("api_key").schema_json` is a non-empty object with `"properties"`. Place where the deps are legal (apps/server or examples test, which may depend on nebula-credential + nebula-schema+schemars + nebula-validator).

- [ ] **Step 2: Run — expect FAIL** (impl absent).

- [ ] **Step 3: Implement the concrete port** — struct `RegistryCredentialSchema { registry: Arc<CredentialRegistry> }`:
  - `validate_data`: `registry.resolve_any(key)` → `&dyn AnyCredential` (404-equivalent → return an Err code `unknown_credential_type` mapped to a field error, OR a distinct path; confirm desired UX) → `any.metadata().base.schema` (`ValidSchema`) → `nebula_schema::FieldValues::from_json(data.clone())` (map its `ValidationError` → one `CredentialFieldError`) → `valid_schema.validate(&field_values)` → on `Err(ValidationReport)` map each `report.errors()` item to `CredentialFieldError { path: e.path.to_string(), code: e.code.as_ref().to_owned(), message: e.message.clone() }` (CONFIRM `ValidationReport`/error accessor names against `crates/credential/tests/properties_pipeline.rs` usage + `crates/schema/src/validated.rs`); never include any submitted value.
  - `list_types`/`get_type`: enumerate the registry (confirm iterator accessor) → per `&dyn AnyCredential`: `m = any.metadata()`; `schema_json = m.base.schema.json_schema().map(|s| s.to_value())?` (confirm `schemars::Schema` → `serde_json::Value`: `serde_json::to_value(&schema)` or `schema.to_value()`); build `CredentialTypeDescriptor` (key=`any.credential_key()`, name/description from `m.base`, `auth_pattern` from `m.pattern` Display/Into<String>, icon/doc_url from `m.base` if present). Public projection is applied api-side (Task 4) — the impl returns raw `json_schema()` Value (spec: "not a raw passthrough" refers to the api wire, satisfied by Task 4).
  - DoD: typed errors (no `unwrap`/`expect`/`panic!` in non-test code — map every `Result`), a `#[tracing::instrument]` span with `credential_key` + outcome enum (NEVER `data`/values), an invariant check (e.g. `debug_assert` the projected schema has no `x-nebula-root-rules` — or assert in the seam test).
- [ ] **Step 4: Enable `schemars`** on the impl crate's `nebula-schema` dep; `cargo tree -p apps-server` (or the impl crate) to confirm one `schemars` resolution; `cargo deny check` to confirm zero deny.toml change needed (it is `ignored=["schemars"]`).
- [ ] **Step 5: Wire** `apps/server/src/compose.rs`: after `default_state`, build the registry (register the first-party credentials available — confirm the registration entrypoint; if the server has no credential registration yet, wire an empty-or-builtin registry and document that catalog is populated as types are registered) and `state = state.with_credential_schema(Arc::new(RegistryCredentialSchema::new(registry)));`. Mirror in the `examples` server + the api test harness/`default_state` used by api integration tests so api tests exercise a real (or representative) port.
- [ ] **Step 6: Run — expect PASS**; **Step 7: full per-crate gate + commit (C5)**:

```bash
cargo nextest run -p nebula-api -p nebula-server 2>&1 | tail -12   # confirm crate name (apps/server package)
cargo clippy --workspace --all-targets -q -- -D warnings 2>&1 | tail -5
cargo deny check 2>&1 | tail -4   # zero deny.toml change; pre-existing nebula-credential-vault wrapper warning is NOT ours
cargo fmt -p nebula-api -p nebula-server
git add crates apps Cargo.lock
bash scripts/worktree.sh commit feat api "wire RegistryCredentialSchema in composition root + enable schemars (ADR-0052 P4)"
```
(Per `feedback_lockfile_rebase`: a dep/feature change touches root `Cargo.lock` — `git add` it too.)

---

## Task 6: OpenAPI honesty + drift reconciliation

**Files:** Modify `crates/api/tests/openapi_canon_compliance.rs` (the credential-type endpoints flip 503→200 when a port is wired — update the honest-stub inventory/expectations so the test reflects reality, NOT to silence it); confirm `crates/api/tests/openapi_spec.rs` still green (operationId/$ref/3.1.0 — `CredentialTypeInfo`/`CredentialTypeDescriptor` ToSchema additions must resolve).

- [ ] **Step 1:** Run `cargo nextest run -p nebula-api -E 'test(/openapi/)' 2>&1 | tail -20`. Read failures.
- [ ] **Step 2:** For each failure, VERIFY it is the legitimate 503→200 honesty flip / new-DTO `$ref` (not a real drift). Update `openapi_canon_compliance.rs` expectations to match the now-honest 200 (the endpoint is no longer a stub when the port is wired; if the api default test harness wires the port, the canon-compliance probe must expect 200, and any "deprecated⇒501" inventory must drop the credential-type entries). Document each expectation change inline with a comment citing ADR-0052 P4. Do NOT add `#[allow]` or weaken assertions to pass.
- [ ] **Step 3:** Re-run until green; **Step 4: commit (folds into C5 or a small C5b)** `bash scripts/worktree.sh commit test api "reconcile OpenAPI honesty tests for populated credential-type endpoints (ADR-0052 P4)"`.

---

## Task 7: ADR-0047 + ADR-0052 P4 amendments + cascade close-out + docs

**Files:** Modify `docs/adr/0047-openapi-31-generator.md` (one paragraph, in-place); `docs/adr/0052-schema-validator-condition-seam.md` (P4 amendment + cascade-complete + Non-goal-open); `crates/api/README.md`/`lib.rs` rustdoc if public surface described there.

- [ ] **Step 1: ADR-0047 in-place amendment.** Append to its "Cross-Layer Schema Strategy" (or a new dated `## Amendment (2026-05-17) — ADR-0052 P4`) ONE paragraph: catalog `CredentialTypeInfo.schema` is populated from `ValidSchema::json_schema()` produced in the composition layer and crosses to `nebula-api` as `serde_json::Value` (api takes no `nebula-schema` production dep); the credential `data` request body stays `serde_json::Value`; the write path validates `data` against the resolved `ValidSchema` before persist; a `nebula-api`-owned mapper strips `x-nebula-root-rules` + predicate operands from the public schema. Cite the seam tests.
- [ ] **Step 2: ADR-0052 P4 amendment** (`## Amendment (2026-05-17) — P4: API write-path validation + catalog json_schema() + public projection`), same register as P1/P2/P3 amendments: what V2/V3/#6 did, the corrected pre-#671 facts (5 points), the `CredentialSchemaPort` seam (api-owned, Option→503 §4.5, authority=validator, INTEGRATION_MODEL §29/§33 unchanged), `schemars` feature-enable (no new crate / no deny.toml change), seam-anchor test paths. **Cascade close-out:** state explicitly "the ADR-0052 cascade (P1 #670 / P2 #672 / P3 #676 / P4 #<this>) is complete; **no P5**." And: "**Non-goal still OPEN:** the `slot_bindings` confused-deputy — credential resolution remains confused-deputy-exposed (no owner/tenant/workspace authz); 'cascade complete' is NOT 'that is closed'; tracked separately."
- [ ] **Step 3: commit (C6)** `git add docs crates && bash scripts/worktree.sh commit docs api "ADR-0047 + ADR-0052 P4 amendments; cascade close-out (ADR-0052 P4)"`.

---

## Task 8: Full pre-PR gate + PR + bot triage + merge + cascade close

- [ ] **Step 1: Full gate** (read tails — never assert green from exit code):

```bash
cd C:/Users/vanya/RustroverProjects/nebula/.worktrees/adr0052-p4
cargo nextest run --workspace 2>&1 | tail -15
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -8
cargo test --workspace --doc 2>&1 | tail -8
cargo deny check 2>&1 | tail -6   # zero deny.toml change; pre-existing nebula-credential-vault note NOT ours
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps 2>&1 | tail -8
for c in nebula-api nebula-server nebula-schema nebula-credential; do cargo fmt -p $c -- --check; done
```
If the OpenAPI 3.1 operationIds change, confirm `openapi_spec.rs` drift assertions stay green (intended additions only).

- [ ] **Step 2: Two-stage review** (spec-compliance, then code-quality) via subagents: always-on personas + `ce-api-contract-reviewer` (DTO/route/serialization + 503→200), `ce-security-reviewer` (untrusted write path + projection: run abuse-case — SSRF/exfiltration via crafted `data`, oversized-payload DoS, expression-injection-in-`data`, confused-deputy note, secret echo in errors), `ce-kieran-rust-reviewer` (.rs), `ce-data-integrity` (persistence gate). Fix-loop until clean; security findings verify-first, never dismiss.

- [ ] **Step 3: Push + PR** `git push -u origin feat/api-adr0052-p4`; `gh pr create --repo vanyastaff/nebula --base main` per `.github/PULL_REQUEST_TEMPLATE.md`. Title `feat(api): ADR-0052 P4 — credential write-path validation + catalog json_schema() + public projection` (confirm no public-contract *removal* ⇒ no `!`; the 503→200 + new validation are additive/§4.5-corrective). Tick "L2 invariant changed → ADR + seam test in this PR". Breaking changes: "None (additive: a previously-503 catalog endpoint becomes 200 when configured; a previously-unvalidated write path becomes validated/explicit-503 — both close a §4.5 fail-open, no API removal)". Docs checklist: ADR-0047 + ADR-0052 P4 amendments; MATURITY.md NOT git-tracked here (external L1; note tiers unchanged). Safety section: closes the verified credential-data fail-open; secret-safe errors (no value echo); proof-token custody INTEGRATION_MODEL §29/§33 unchanged. Notes: cascade complete; `slot_bindings` Non-goal OPEN; `schemars` feature-enable (no new crate / no deny.toml change). End body with the Claude Code trailer.

- [ ] **Step 4: Triage ALL bot reviews verify-first** (CodeRabbit/Copilot/Codex): reproduce each claim against source; rebut false positives with exact-line + green-gate evidence; implement real fixes as new commits + re-run the affected gate; reply + resolve every thread by id. Never blind-merge on green CI + CLEAN alone (P1–P3 each had a real bug past green CI; P3 Copilot posted 4 confidently-wrong comments).

- [ ] **Step 5: Squash-merge** per AGENTS.md ONLY when CI fully green + confirmed stage-by-stage + all threads resolved. Post-merge: `cd C:/Users/vanya/RustroverProjects/nebula && bash scripts/worktree.sh finish adr0052-p4`. Update the user-memory cascade note: P1–P4 all MERGED, cascade COMPLETE, no P5, `slot_bindings` Non-goal still OPEN. **Do NOT spawn a P5** — P4 closes the cascade.

---

## P4 backlog / explicitly deferred (record so honest)

- `slot_bindings` confused-deputy (design-spec Non-goal): credential/resource resolution still has no owner/tenant/workspace authz after P4. Tracked separately; NOT closed by the cascade.
- If the composition root has no first-party credential-registration entrypoint yet, the catalog is correctly empty until types are registered (honest, not a stub-lie — the port is wired; it simply has zero entries). Note this in the PR if so.

## Self-Review

**1. Spec coverage** (§Q2 + Phasing P4 + ADR-0052 P3-amendment P4 paragraph):
- V2 write-path validates `data` vs resolved `ValidSchema` before persist → Task 2 (+ Task 5 real impl). ✓ (also closes the verified live fail-open — corrected fact #1)
- V3 catalog populated from `json_schema()` → Task 3 (+ Task 5). ✓
- #6 public projection strips `x-nebula-root-rules` + predicate operands, api-owned → Task 4. ✓
- ADR-0047 one-paragraph in-place amendment + ADR-0052 P4 amendment + seam tests same PR (canon §0.1/§17) → Task 7 + seam tests in Tasks 2/3/4/5. ✓
- Zero new crate / zero deny.toml change / api free of nebula-schema production dep / authority with validator → Architecture + Task 1 (port api-safe) + Task 5 (schemars feature-enable only; `cargo deny` check Task 5/8). ✓
- Cascade close-out + `slot_bindings` Non-goal OPEN, no P5 → Task 7 Step 2 + Task 8 Step 5. ✓
- #3/#4/panic correctly ABSENT (P2 scope) → not in any task. ✓

**2. Placeholder scan:** No "TBD"/"add error handling"/"similar to Task N". The six flagged name-confirmations are explicit `rg`/source-check callouts at the exact step (P1/P2/P3 discipline) — not design gaps; every type/method (`AppState`, `with_action_registry`, `ValidSchema::validate`, `FieldValues::from_json`, `json_schema`, `AnyCredential::metadata`, `ApiError::Validation`, `CredentialRegistry::resolve_any`) is read from verified source at da3e0a13.

**3. Type consistency:** `CredentialSchemaPort` / `CredentialFieldError{path,code,message}` / `CredentialTypeDescriptor{key,name,description,auth_pattern,icon,documentation_url,schema_json(,capabilities?)}` identical across Tasks 1/2/3/4/5; `AppState.credential_schema: Option<Arc<dyn CredentialSchemaPort>>` + `with_credential_schema` consistent; commit scope `api`, type `feat`/`docs`/`test`, every commit via `scripts/worktree.sh commit`. If Task 3 needs `capabilities` on the descriptor, it is added to Task 1's struct + test (flagged inline) to keep types consistent.

**4. Scope/granularity:** 9 tasks, atomic-commit boundaries aligned to lefthook full-workspace-clippy-per-commit (C1 api-addition green, C2 V2 green, C3 V3 green, C4 #6 green, C5 composition+schemars green, C6 docs). No #3/#4/panic. No P5. Non-goal explicitly deferred.
