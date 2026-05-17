# ADR-0047: OpenAPI 3.1 spec generation — adopt `utoipa` for `nebula-api`

**Status:** Accepted
**Date:** 2026-05-06
**Supersedes:** —
**Superseded by:** —
**ROADMAP:** §M3.2 — OpenAPI 3.1 spec generation
**Issues:** Closes the `crates/api/src/handlers/openapi.rs:8-17` "not implemented" stub. Tracks ecosystem maintenance risks `juhaku/utoipa#1536` (query-param inference), `#1546` (`IntoParams` generics), `#841` (`#[serde(flatten)]` ignored in `IntoParams`).

> **Note on prior state.** `crates/api/src/handlers/openapi.rs` ships two stub handlers — `openapi_spec()` and `docs_ui()` — that return `ApiError::Internal("not implemented")` for `GET /api/v1/openapi.json` and `GET /api/v1/docs`. The routes are mounted in `crates/api/src/routes/openapi.rs` so external consumers see 500 today. ROADMAP §M3.2 is one of four remaining 1.0 feature gaps in the API layer (alongside §M3.4 idempotency wiring, §M3.5 tracing-context propagation, §M3.6 shift-left `validate_workflow` audit). Auth (§M3.1) and webhook dispatch (§M3.3) closed via PR #638.

## Context

`nebula-api` is the public HTTP surface of the Nebula workflow engine. Third-party integrators consume it directly; the spec at `/api/v1/openapi.json` is the machine-readable contract that drives client SDK generation, doc tooling, and contract-testing pipelines. Today there is no spec — the endpoint returns 500.

The DoD for ROADMAP §M3.2 (`.ai-factory/ROADMAP.md:282-294`) requires:

1. A real OpenAPI 3.1 generator wired into the router.
2. Every handler annotated with request / response schemas.
3. The served spec must match the actual route table at runtime — drift detection is non-negotiable per canon §4.5 ("no false capability").
4. Swagger UI mounted at `/api/v1/docs`.
5. Integration test validating the spec against the OpenAPI 3.1 schema.
6. Doc test or CI lint catching divergence between router-mounted routes and spec entries.

The Rust ecosystem offers three serious candidates:

- **`utoipa`** + `utoipa-axum` + `utoipa-swagger-ui` (juhaku) — derive-macro driven, axum-native companion crate.
- **`aide`** (tamasfe) — `schemars`-based, axum-native, 3.1-only.
- **`okapi`** / **`rocket-okapi`** — Rocket-coupled, no axum integration.

`poem-openapi` and `salvo-oapi` are framework-coupled to non-axum stacks and are not options.

A first-principles ecosystem audit (run on 2026-05-06 via WebFetch against docs.rs, GitHub releases, and open-issue trackers) collected the evidence summarized below.

### Evidence collected

| Crate | OpenAPI 3.1 | axum 0.8 | License | Last release | Active issue/PR signal | Drift-detection mechanism |
|---|---|---|---|---|---|---|
| `utoipa` 5.5.0 / `utoipa-axum` 0.2.0 / `utoipa-swagger-ui` 9.x | Yes (README feature #1) | Yes (`utoipa-axum 0.2.0` pins `axum ^0.8.0`) | MIT OR Apache-2.0 | utoipa 5.5.0 → **2026-05-04** | 201 open issues, fresh activity through May 2026; `utoipa-axum` companion crate maintained in lockstep | `OpenApiRouter` + `.split_for_parts()` — same router becomes both spec source and served `axum::Router`; handlers without `#[utoipa::path]` cannot be mounted through `routes!(...)` |
| `aide` 0.15.1 | Yes ("Currently only Open API version 3.1.0 is supported") | Yes (`axum ^0.8.1`) | MIT OR Apache-2.0 | **2025-08-19** | 35 open issues; latest activity Mar 2026 (#295); 0.16.0 tracking issue (#270) open since Dec 2025 — borderline on a "maintained in last 6 months" threshold | `ApiRouter::api_route()` is opt-in; plain `.route()` mounts handlers WITHOUT documenting them — silently bypasses the spec |
| `okapi` 0.7.0 | Unverified (module named `openapi3`, no 3.1 claim found) | No (Rocket-coupled via `rocket-okapi`) | MIT | Unverified | — | Out on hard requirement |

Source URLs (fetched 2026-05-06):

- https://docs.rs/utoipa/latest/utoipa/
- https://docs.rs/utoipa-axum/latest/utoipa_axum/
- https://github.com/juhaku/utoipa
- https://github.com/juhaku/utoipa/releases
- https://github.com/juhaku/utoipa/issues
- https://docs.rs/aide/latest/aide/
- https://docs.rs/aide/latest/aide/axum/index.html
- https://github.com/tamasfe/aide
- https://github.com/tamasfe/aide/releases
- https://docs.rs/okapi/latest/okapi/

### Why drift detection is the load-bearing criterion

Canon §4.5 ("no false capability") is enforced by `feedback_observability_as_completion.md` and `feedback_review_verify_claims.md`: every shipped claim must be backed by exercised code. A spec that documents an endpoint not mounted (or omits a mounted endpoint) IS a false capability claim. Two of the three §M3.2 acceptance items (#3 spec ≡ route table, #6 doc test or CI lint) are explicitly drift-detection gates.

Aide's drift model is opt-in: `ApiRouter::api_route()` documents, plain `.route()` does not. A future contributor copy-pasting an existing route block can silently land an endpoint in production without it appearing in the spec. The bug class is exactly the "discipline rule that drifts" pattern called out in `docs/pitfalls.md` (re-entry into the evaluator) — and `feedback_type_enforce_not_discipline.md` mandates structural fixes over discipline.

`utoipa-axum` inverts this: `OpenApiRouter::routes(routes!(handler1, handler2))` is the documenting mounting path — handlers without `#[utoipa::path]` cannot be passed to `routes!`, so adding a route via `routes!` AND forgetting to annotate it is a compile error, not a review-time catch.

**Caveat (not a total guarantee):** `OpenApiRouter` also exposes `route(path, MethodRouter)` (mirroring `axum::Router::route`) as a pass-through that mounts an axum route without contributing to the spec. This is the right escape hatch for endpoints we deliberately keep out of the public contract (Prometheus `/metrics`, Tower-served `SwaggerUi`). It does **not** raise a compile error when a contributor uses it for a regular handler that should be in the spec — the runtime drift tests in `crates/api/tests/openapi_spec.rs` (path inventory + smoke list) are the second line of defence for that case. Reviewers must call out any new `OpenApiRouter::route(...)` invocation that documents an application endpoint.

## Decision

**Adopt `utoipa` 5.5 + `utoipa-axum` 0.2 + `utoipa-swagger-ui` 9 as the OpenAPI 3.1 generator stack for `nebula-api`.**

### Pinned versions

```toml
# root Cargo.toml [workspace.dependencies]
utoipa             = { version = "5.5", features = ["axum_extras", "macros"] }
utoipa-axum        = "0.2"
utoipa-swagger-ui  = { version = "9", features = ["axum"] }
oas3               = "0.16"  # dev-dependency only — typed 3.1 parser for spec validation tests
```

The `axum_extras` feature unlocks utoipa's axum extractor inference (`Path<T>`, `Query<T>`, `Json<T>`) so `#[utoipa::path]` annotations don't have to restate parameter sources. `macros` enables `#[derive(ToSchema, IntoParams)]`.

### Compatibility check at task start

Before landing the deps, the contributor MUST run `cargo tree -p nebula-api` and confirm:

- exactly one `axum 0.8.*` resolution
- exactly one `utoipa 5.5.*` resolution
- `utoipa-axum 0.2.*` and `utoipa-swagger-ui 9.*` align with the above

If any of these resolve to multiple versions, this ADR is amended before merging the deps.

### Spec validation in tests

Use `oas3` 0.16 (Rust-native typed OpenAPI 3.1 parser) for round-trip validation:

```rust
let served: serde_json::Value = serde_json::from_str(&body)?;
let _spec: oas3::OpenApi = oas3::OpenApi::deserialize(&served)?;
```

A successful deserialize is strong validation — `oas3` enforces required fields, type discriminators, and `$ref` shape. Fixture-free, hermetic, and avoids meta-schema URL maintenance.

If a future spec exercises a 3.1 corner case `oas3` does not represent (strict required-field enforcement, vendor extensions, etc.), fall back to `jsonschema` against the pinned 3.1 meta-schema URL `https://spec.openapis.org/oas/3.1/schema-base/2022-10-07` vendored under `crates/api/tests/fixtures/openapi-3.1.json`. Choice is documented in the test file's top-of-file comment.

### Mounting model

```rust
// crates/api/src/routes/<group>.rs
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::auth::signup, handlers::auth::login, ...))
}
```

```rust
// crates/api/src/app.rs (internal; signature unchanged externally)
let (router, openapi): (axum::Router, utoipa::openapi::OpenApi) =
    OpenApiRouter::with_openapi(OpenApiDoc::openapi())
        .merge(routes::auth::router())
        // ... merge every group ...
        .split_for_parts();

let state = state.with_openapi_doc(Arc::new(openapi));
let router = router.with_state(state).layer(/* middleware stack */);
```

The served `axum::Router` and the materialized `OpenApi` are produced from the same `OpenApiRouter` value — physical proof of route-table ≡ spec parity.

### Stub handler policy

Endpoints whose handler currently returns `ApiError::Internal("not implemented")` (today: every handler in `me.rs`, `org.rs`, `resource.rs`) MUST be documented honestly:

- `#[utoipa::path(deprecated)]` set to `true`.
- `responses` includes `(status = 501, description = "Not yet implemented; tracked in M-X")`.
- Tag suffixed with ` (planned)` so Swagger UI groups stubs visibly.
- Description text explicitly says "Returns 501 today; payload schema is the planned shape once the underlying milestone closes."

This satisfies canon §4.5 while keeping drift detection green (route table ≡ spec). When the underlying milestone lands, the `deprecated` flag and 501-response are removed in a one-line follow-up — no additional refactor.

### Idempotency-Key header

§M3.4 ships the `IdempotencyLayer` middleware but does not mount it (`crates/api/src/middleware/idempotency.rs:48-50`). This ADR does NOT document `Idempotency-Key` as an accepted header until §M3.4 closes. Once mounted, every mutating-handler `#[utoipa::path]` annotation gains `params(("Idempotency-Key" = String, Header, ...))` — single-line follow-up tracked alongside §M3.4.

### Swagger UI vs RapiDoc

Swagger UI is the default. RapiDoc is not adopted at this time — Swagger UI's "Try it out" round-trip + auth-header injection covers the integrator workflow today; switching is a single-line change in `routes/openapi.rs` if needed later.

### Tracing-context (`traceparent`) headers

Out of scope. Tracking under §M3.5. Not documented in spec until that milestone closes — same pattern as Idempotency-Key.

## Cross-Layer Schema Strategy

This section is load-bearing for §M3.2 implementation and forms the contract for the T2.5 commitment in the M3.2 plan.

### Rule

**API DTOs MUST NOT embed types from `nebula-core`, `nebula-storage`, `nebula-engine`, or `nebula-credential` — except via simple string aliases or wrapped enums.**

### Why

- **Layer hygiene.** The Nebula architecture (per `.ai-factory/ARCHITECTURE.md`) enforces one-way layer dependencies via `cargo deny check [bans.wrappers]`. `ToSchema` derives propagating into `nebula-core` / `nebula-storage` / `nebula-engine` would either pull `utoipa` into those crates (a leaf API-layer concern leaking down the stack) or force feature-gated derive plumbing to keep upper-layer concerns out of lower layers. Both are layering violations.
- **Public-contract stability.** The OpenAPI spec is part of the public contract. Coupling it to internal types means every internal-type rename ships as an API-spec change, undermining the contract's stability promise.
- **Implementation hiding.** `nebula-storage::repos::*`, `nebula-engine::ActionRegistry`, `nebula-credential::PendingToken` carry storage-shape and execution-shape internals that must not leak to integrators. The API layer's job is to translate, not forward.
- **Boundary discipline matches `feedback_boundary_erosion.md`.** "Just one helper in the wrong crate" compounds; the same applies to "just one type re-export." The rule is structural, not discipline-based.

### Wrapping checklist

Every cross-layer type that appears in current handler signatures gets wrapped at `crates/api/src/models/`:

| Cross-layer source type | API-layer wrapper | Notes |
|-------------------------|-------------------|-------|
| `nebula_core::OrgId` | `String` (ULID) | Direct stringification at handler boundary; no DTO wrapper needed. |
| `nebula_core::WorkspaceId` | `String` (ULID) | Same. |
| `nebula_core::OrgRole` | `OrgRoleDto(String)` newtype with `#[schema(value_type = String, format = "enum")]` | Enum-as-string preserves serde repr without leaking the type. |
| `nebula_core::WorkspaceRole` | `WorkspaceRoleDto(String)` | Same pattern. |
| `nebula_core::scope::Principal` | `PrincipalDto { kind: String, id: String }` | Discriminator-as-string flat shape. |
| `nebula_credential::PendingToken` | `PendingTokenDto { token: String }` | Wrapper struct; secret material redacted via `#[schema(format = "password", write_only)]`. |
| `nebula_storage::*` repository types | not exposed | Handlers consume repos via `State<AppState>`; only DTOs go on the wire. |
| `nebula_engine::ActionRegistry` | not exposed | Already wrapped by `ActionSummary` / `ActionDetailResponse` in `models/catalog.rs`. |
| `nebula_plugin::PluginRegistry` | not exposed | Already wrapped by `PluginSummary` / `PluginDetailResponse`. |
| `serde_json::Value` (request/response bodies) | OK at request body for genuinely-opaque payloads (workflow definition JSON, credential `data`); for response bodies, prefer typed DTOs unless the shape is genuinely caller-defined. Tag as `additionalProperties = true`. | The T3.0 audit classifies each existing `Value` callsite. |

### Enforcement

- T3 (DTO ToSchema task) runs a `cargo tree`-style audit at completion: `cargo metadata` confirms `utoipa::ToSchema` derives appear only inside `crates/api/`.
- T4 (handler annotation task) fails compile if a handler `responses(body = X)` references an unwrapped cross-layer type — the type would not derive `ToSchema` and `routes!()` would reject it.
- The T7 drift-detection test cannot pass if a handler is mounted without a derive-able body type, providing a third gate.

### Out of scope for this ADR

- Adding `ToSchema` derives to `nebula-sdk` or `nebula-plugin-sdk` for plugin/SDK consumers. Those are public surfaces but not on the HTTP API path; if needed, that's a separate ADR.
- Re-deriving wrapper types from declarative macros. Hand-rolled `From<OrgRole> for OrgRoleDto` is fine; if wrapper boilerplate exceeds 300 LOC, revisit with a `derive_more::From` consideration.

## Alternatives Considered

### Alternative 1: `aide` (tamasfe)

`aide` is the closest competitor — 3.1-only, schemars-based, axum-native. Rejected for two reasons:

1. **Drift model is opt-in.** `ApiRouter::api_route()` documents; plain `Router::route()` doesn't. A future PR can silently bypass the spec. Canon §4.5 + `feedback_type_enforce_not_discipline.md` mandate structural drift detection, not review-time catches.
2. **Maintenance velocity is slowing.** Last release 2025-08-19; 0.16.0 tracking issue open since Dec 2025 with no merge. utoipa shipped a release on 2026-05-04. For a 1.0 dependency we ship to integrators, "actively maintained" is a hard requirement.

Aide's schemars integration is genuinely cleaner conceptually (one schema source) — but the drift-model failure dominates.

### Alternative 2: Hand-written OpenAPI YAML / JSON

Rejected. Hand-written specs drift from the code by construction; canon §4.5 requires structural prevention of drift. Maintenance burden is also linear in route count (50+ today), so the breakeven against utoipa's macro overhead is immediate.

### Alternative 3: `okapi` / `rocket-okapi`

Rejected on hard requirement: no axum integration. `okapi` is also a 3.0 generator — would require down-conversion of any 3.1 features.

### Alternative 4: Defer M3.2 to 1.1

Rejected. M3.2 is one of four 1.0 API blockers. Third-party integrators cannot start their SDK / contract-testing work without a machine-readable spec. Deferring blocks downstream consumers and the 1.0 release narrative.

## Consequences

### Positive

- Drift between router and spec becomes a compile error, not a review catch.
- Stub endpoints are documented honestly via `deprecated: true` + 501, satisfying canon §4.5 without blocking M3.2 closure.
- Spec is hermetically validated via `oas3` round-trip — no external network, no fixture maintenance.
- Maintenance burden tracks `utoipa` upstream, not Nebula-internal hand-spec churn.
- Integrators get a 3.1 spec + Swagger UI on day one of 1.0.

### Negative

- `utoipa-axum` 0.2 is a 0.x crate; backwards-incompatible changes are possible at 0.3. Mitigation: pin minor; review utoipa-axum changelog at every workspace dep refresh; the migration path is mechanical (rename `routes!` macro shape, etc.).
- `utoipa` derive-macro overhead adds compile time. Estimated +5-10s on `cargo build -p nebula-api` (clean build); negligible on incremental. Acceptable.
- Three known utoipa upstream issues affect annotation ergonomics:
  - `#1536` (Apr 2026): query-param extractor inference gap → use explicit `params(...)`.
  - `#1546` (May 2026): `IntoParams` does not support generic types → keep `PaginationParams` non-generic.
  - `#841` (long-standing): `IntoParams` ignores `#[serde(flatten)]` → flatten only at request-body level.
  None block M3.2; all have documented workarounds.

### Neutral

- Swagger UI vs RapiDoc choice can be revisited without re-doing the generator work.
- Spec evolution is continuous: every new handler ships its annotation in the same PR; spec auto-regenerates.

## Open Questions / Follow-ups

- **Idempotency-Key documentation:** ship in §M3.4 closure (one-line follow-up per mutating handler).
- **`traceparent` header documentation:** ship in §M3.5 closure.
- **Auto-generated client SDKs (TypeScript, Python):** separate effort post-1.0; the spec being correct is the prerequisite, generators consume it later.
- **OpenAPI 3.0 fallback for older tooling:** not planned. 3.1 is the standard; tooling that can't consume 3.1 should upgrade.

## Pointers

- ROADMAP §M3.2: `.ai-factory/ROADMAP.md:282-294`
- Implementation plan: `.ai-factory/plans/m3-2-openapi-spec.md`
- Stub today: `crates/api/src/handlers/openapi.rs:8-17`
- Route declaration: `crates/api/src/routes/openapi.rs`
- App composition: `crates/api/src/app.rs:19-69`
- AppState: `crates/api/src/state.rs`
- Pitfalls — type-enforced boundaries: `docs/pitfalls.md` (`nebula-expression` builtin re-entry case)
- Feedback memories applied: `feedback_adr_ecosystem_evidence.md`, `feedback_type_enforce_not_discipline.md`, `feedback_observability_as_completion.md`, `feedback_boundary_erosion.md`

## Amendment (2026-05-17) — ADR-0052 P4 credential-schema seam

ADR-0052 P4 populates `CredentialTypeInfo.schema` from
`ValidSchema::json_schema()` (produced behind an api-owned
`CredentialSchemaPort`), and validates the credential `data` request body
against the resolved `ValidSchema` before persist; the `data` request
body stays `serde_json::Value` (this ADR's "Cross-Layer Schema Strategy"
table already sanctions `Value` for credential `data`). A
`nebula-api`-owned mapper strips `x-nebula-root-rules` + predicate
operands from the public catalog schema (cross-field predicate logic must
not leak to unauthenticated clients). **Clarification to the Cross-Layer
Schema Strategy:** that section's binding rule is *"API DTOs MUST NOT
embed types from `nebula-core`/`-storage`/`-engine`/`-credential`"* — a
**DTO-type** rule, which P4 fully honors (the port and every catalog DTO
carry only `serde_json::Value` / api-owned structs; **no `ValidSchema`
type appears in any DTO**). To keep ADR-0052's harder "zero `deny.toml`
change" constraint feasible post-PR #671 (composition root is now a
separate `nebula-server` crate not in `nebula-credential`'s wrapper
allowlist), the concrete port impl lives in `nebula-api`, which therefore
takes a `nebula-schema` **production** dependency + the `schemars`
feature. `nebula-schema` is Core (freely importable; no `deny.toml`
change) and `schemars` is already `ignored` in `deny.toml`. The informal
"`nebula-api` never imports `nebula-schema`" prose preference is
**relaxed** to "no `nebula-schema` *type* crosses into a DTO" — the
load-bearing layer guarantee (DTO purity, public-contract stability,
implementation hiding) is unchanged. Seam: `crates/api/tests/seam_credential_*`.
