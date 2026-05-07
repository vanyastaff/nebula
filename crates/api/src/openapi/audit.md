# `Json<serde_json::Value>` callsite audit (M3.2 Task 3.0)

> Input contract for T3 (typed DTO scope) and T4 (per-handler `#[utoipa::path]`
> annotations). See [ADR-0047](../../../../docs/adr/0047-openapi-31-generator.md)
> for the cross-layer schema strategy and stub endpoint policy.
>
> Run on: 2026-05-06 against worktree `feat/api-openapi-spec` at HEAD.
> Discovery: `Grep "Json<.*Value>"` across `crates/api/src/`.

## Classification scheme

| Class | Definition | Treatment in spec |
|-------|------------|-------------------|
| **(a) Typed-able shipped** | Handler returns shipped behaviour but uses ad-hoc `Json<Value>` for the response body. Body shape is well-defined and stable. | Introduce typed DTO; switch handler to `Json<T>`; `responses(body = T)` in `#[utoipa::path]`. |
| **(b) Opaque-shipped** | Handler returns shipped behaviour but the body shape is genuinely caller-defined (workflow definitions, credential payloads). | Keep `Json<Value>`; declare schema with `additionalProperties = true`; document as JSON object with no fixed shape. |
| **(c) Stub (501-equivalent)** | Handler currently returns `ApiError::Internal("not implemented")`; underlying business logic is in a separate milestone. | Apply Stub Endpoint Policy: `deprecated = true`, `responses((status = 501, ...))`, planned-shape note. |
| **(d) Out of scope** | Handler exists but route is NOT mounted in `routes::create_routes` (separate optional transport, dev-only, etc.). | Not in spec; no `#[utoipa::path]` annotation. |

## Audit

### `crates/api/src/handlers/auth.rs`

| Line | Handler | Class | Notes |
|------|---------|-------|-------|
| 167 | `forgot_password` | **(a)** | Returns `(StatusCode::ACCEPTED, Json(json!({"queued": true})))`. Typed DTO: `AsyncAcceptedResponse { queued: bool }` (or reuse a shared `MessageResponse { ok: bool }` if the boolean field name doesn't matter). Shipped under PR #638. |
| 181 | `reset_password` | **(a)** | Returns `Json(json!({"reset": true}))`. Same DTO pattern as `forgot_password`. Shipped. |
| 195 | `verify_email` | **(a)** | Returns `Json(json!({"verified": true}))`. Same DTO pattern. Shipped. |

**Recommended DTO consolidation:** introduce one `crates/api/src/models/system.rs` (or extend `health.rs`) with:

```rust
#[derive(Serialize, Deserialize, ToSchema)]
pub struct AckResponse {
    /// Operation outcome — always `true` on the success path.
    pub ok: bool,
}
```

…and migrate the three auth handlers to return `Json<AckResponse>`. The existing JSON keys (`queued`, `reset`, `verified`) are not load-bearing — they were ad-hoc. Migrating to `ok` is a non-breaking simplification (or use a tagged enum if we want to preserve the verb-specific keys; `AckResponse` is recommended for spec clarity).

### `crates/api/src/handlers/me.rs`

| Line | Handler | Class | Notes |
|------|---------|-------|-------|
| 19 | `get_me` | **(c)** | Stub. Planned shape: `MeResponse { user: UserProfile, orgs_count: u32, tokens_count: u32 }`. Underlying milestone: Plane-A "me" endpoint extension. |
| 29 | `update_me` | **(c)** | Stub. Request body also `Json<Value>` — planned shape `UpdateMeRequest { display_name: Option<String>, avatar_url: Option<String> }`. |
| 38 | `list_my_orgs` | **(c)** | Stub. Planned shape: `MyOrgsResponse { orgs: Vec<OrgSummary> }` where `OrgSummary { id: String, slug: String, role: OrgRoleDto }` (per ADR-0047 cross-layer rule). |
| 47 | `list_my_tokens` | **(c)** | Stub. Planned shape: `MyTokensResponse { tokens: Vec<TokenSummary> }` where `TokenSummary { id, name, scopes, created_at, last_used_at, expires_at }`. **Never** the secret value. |
| 57 | `create_token` | **(c)** | Stub. Request: `CreateTokenRequest { name, scopes, ttl_seconds }`. Response: `CreateTokenResponse { token: String, summary: TokenSummary }` — token shown ONCE; flagged `write_only` in spec. |
| 67 | `delete_token` | **(c)** | Stub. Response: `AckResponse`. |

All 6 are class **(c)**. Tag in spec: `me (planned)`. Group all under `deprecated: true` until the underlying milestone closes.

### `crates/api/src/handlers/org.rs`

| Line | Handler | Class | Notes |
|------|---------|-------|-------|
| 19 | `get_org` | **(c)** | Stub. Planned shape: `OrgResponse { id, slug, name, plan, created_at }`. |
| 29 | `update_org` | **(c)** | Stub. Both request and response `Json<Value>`. Planned: `UpdateOrgRequest { name, settings }` → `OrgResponse`. Already calls `tenant.require(Permission::OrgUpdate)?` — RBAC gate is real, body handling is not. |
| 39 | `delete_org` | **(c)** | Stub. Planned: `AckResponse`. RBAC gate (`Permission::OrgDelete`) is real. |
| 49 | `list_members` | **(c)** | Stub. Planned: `MembersResponse { members: Vec<MemberSummary> }`. |
| 59 | `invite_member` | **(c)** | Stub. Request/response `Json<Value>`. Planned: `InviteMemberRequest { email, role }` → `InviteMemberResponse { invitation_id, expires_at }`. |
| 70 | `remove_member` | **(c)** | Stub. Planned: `AckResponse`. |
| 80 | `list_service_accounts` | **(c)** | Stub. Planned: `ServiceAccountsResponse { accounts: Vec<ServiceAccountSummary> }`. |
| 90 | `create_service_account` | **(c)** | Stub. Planned: `CreateServiceAccountRequest { name, scopes }` → `CreateServiceAccountResponse { account: ServiceAccountSummary, key: String }` (key shown once, write_only). |
| 101 | `delete_service_account` | **(c)** | Stub. Planned: `AckResponse`. |

All 9 are class **(c)**. Tag in spec: `orgs (planned)`. RBAC `tenant.require(...)` gates ARE real on update/delete/create — the spec should still reflect that 403 is a possible outcome alongside 501.

### `crates/api/src/handlers/resource.rs`

| Line | Handler | Class | Notes |
|------|---------|-------|-------|
| 15 | `list_resources` | **(c)** | Stub. Planned: `ListResourcesResponse { resources: Vec<ResourceSummary> }` where `ResourceSummary { id, name, kind, version, attached_to_workflows }`. Underlying milestone: resource catalog endpoint backend. |

Class **(c)**. Tag: `workspaces.resources (planned)`.

### `crates/api/src/handlers/execution.rs`

| Line | Handler | Class | Notes |
|------|---------|-------|-------|
| 582 | `terminate_execution` | **(c)** | Stub. Planned: `AckResponse` plus 404 on missing exec, 409 on already-terminal. Underlying milestone: terminate-action wiring (ROADMAP §M2 follow-up if not already shipped). |
| 593 | `restart_execution` | **(c)** | Stub. Planned: `RestartExecutionResponse { new_execution_id }`. Underlying milestone: execution restart semantics. |

Both are class **(c)**. Tag: `workspaces.executions` (no `(planned)` suffix because most executions handlers are shipped — only these two are deprecated).

### `crates/api/src/handlers/credential.rs`

| Line | Handler | Class | Notes |
|------|---------|-------|-------|
| 136 | `delete_credential` | **(a)** | Returns `Json(json!({"deleted": true}))`. Typed DTO: `AckResponse`. Shipped. |

### `crates/api/src/handlers/health.rs`

| Line | Handler | Class | Notes |
|------|---------|-------|-------|
| 44 | `version_info` | **(a)** | Returns `Json(json!({"version": ..., "name": "nebula"}))`. Typed DTO: `VersionInfo { version: String, name: String }`. Shipped. Add to `crates/api/src/models/health.rs`. |

### `crates/api/src/handlers/openapi.rs`

| Line | Handler | Class | Notes |
|------|---------|-------|-------|
| 8 | `openapi_spec` | **(a)** | Stub today, but T6 wires it to return the cached `Arc<OpenApi>`. `responses(body = serde_json::Value, content_type = "application/json")` — body IS `serde_json::Value` because `OpenApi` serializes through `serde_json`. Schema description: "OpenAPI 3.1 specification document for this API." |

### `crates/api/src/server/websocket.rs`

| Line | Handler | Class | Notes |
|------|---------|-------|-------|
| 32 | `ws_not_implemented` | **(d)** | Mounted only when `RealtimeTransport` is attached. `app.rs::build_app` does NOT merge `RealtimeTransport` today (only `WebhookTransport` is merged at `app.rs:35-38`). Out of M3.2 scope; deferred to ROADMAP 1.1 per RESEARCH.md. **Do not annotate.** |

## Summary

| Class | Count | Treatment |
|-------|-------|-----------|
| (a) Typed-able shipped | 6 (auth ×3 + credential ×1 + health ×1 + openapi ×1) | New DTOs added in T3: `AckResponse`, `VersionInfo`. Existing handlers migrated to typed return. |
| (b) Opaque-shipped | 0 | None observed in the current codebase. |
| (c) Stub (501-equivalent) | 18 (me ×6 + org ×9 + resource ×1 + execution ×2) | Stub Endpoint Policy: `deprecated = true`, 501 response, planned-shape DTOs declared in `models/me.rs` / `models/org.rs` / `models/resource.rs`. |
| (d) Out of scope | 1 (websocket) | Not in spec. |
| **Total** | **25** | |

The earlier rough count of "~24 callsites" missed `health.rs:44` and the websocket case; the verified total is 25.

## Implementation order for T3 / T4

1. **T3 first**, in this order:
   - `crates/api/src/models/system.rs` (or extend `health.rs`): `AckResponse`, `VersionInfo`.
   - `crates/api/src/models/me.rs`: planned DTOs for the 6 stub handlers + tokens.
   - `crates/api/src/models/org.rs`: planned DTOs for the 9 stub handlers + invitations.
   - `crates/api/src/models/resource.rs`: `ListResourcesResponse`, `ResourceSummary`.
   - Existing `models/{auth,catalog,credential,execution,health,workflow}.rs`: add `#[derive(ToSchema)]`.
   - `models/pagination.rs` extracted module.
   - Migrate class-(a) handlers to return their typed DTOs.

2. **T4 next**: `#[utoipa::path]` on every handler. Class-(c) handlers use the planned DTOs from step 1 in their `responses(...)` annotation alongside the `(status = 501, ...)` marker, so once the underlying milestone closes, removing `deprecated` and the 501 response is the only diff.

## Cross-layer types observed

The following cross-layer types appear in current handler signatures and MUST be wrapped per ADR-0047:

- `nebula_core::TenantContext` — extension only, not on the wire. No wrapper needed.
- `nebula_core::Permission` — used in `tenant.require(...)` only, not on the wire. No wrapper needed.
- `nebula_core::OrgRole` — appears in planned `MyOrgsResponse / OrgSummary` DTOs. **Wrap as `OrgRoleDto(String)`** in `crates/api/src/models/me.rs` (or shared `models/role.rs`).
- `nebula_core::WorkspaceRole` — appears in planned `MyOrgsResponse` (workspace-level). **Wrap as `WorkspaceRoleDto(String)`**.
- `nebula_core::OrgId` / `WorkspaceId` — already exposed as `String` (ULID) in shipped DTOs (`WorkflowResponse`, etc.). No new wrapper.
- `crate::middleware::auth::AuthContext` — extension only. No wrapper.
- `crate::auth::dto::SecretString` — kept as-is at the request body level; spec annotation `#[schema(value_type = String, format = "password", write_only = true)]` redacts it. **Verify the runtime redaction test in T3 catches accidental serialization.**

## Open follow-ups (not blocking M3.2)

- `me`, `org`, `resource` business-logic implementation. Each closes one or more class-(c) entries above. When that PR lands, removing `deprecated` + 501 from the spec is a one-line diff per handler.
- `execution::terminate` / `execution::restart` semantics — defer to engine team; spec is ready when handlers are.
- `WebSocket` real-time transport — deferred to ROADMAP 1.1 per RESEARCH.md.
