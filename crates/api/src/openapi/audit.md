# `Json<serde_json::Value>` callsite audit (M3.2 Task 3.0)

> Input contract for T3 (typed DTO scope) and T4 (per-handler `#[utoipa::path]`
> annotations). See [ADR-0082](../../../../docs/adr/0082-api-webhooks-idempotency.md)
> for the cross-layer schema strategy and stub endpoint policy (absorbs ADR-0047).
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

…and migrate the three auth handlers to return `Json<AckResponse>`.

**This IS a breaking response-key rename** — any client that unmarshals
the existing `{"queued": true}` / `{"reset": true}` / `{"verified": true}`
keys will fail against the new `{"ok": true}` shape. Acceptable for
M3.2 because the only documented consumer is the upcoming Plane-A
frontend (not yet shipped). External clients should migrate to
`AckResponse` reading `ok` instead of the verb-specific keys; the spec
publishes the new shape as authoritative. If a versioned/compat
transition becomes necessary, return a tagged enum that carries both
the new `ok` field and the legacy verb-specific key.

### `crates/api/src/domain/me/handler.rs`

| Handler | Class | Notes |
|---------|-------|-------|
| `get_me` | **shipped** | Implemented end-to-end via the Plane-A `AuthBackend` port. Returns `MeResponse` 200 + 401/404/503. `tokens_count` is the real count of the caller's active PATs; `orgs_count` is `Option<u32>` and **omitted from the wire** (never a synthesized `0`) until principal→orgs enumeration is wired with the org/membership phase — see `list_my_orgs` (canon §4.5 / §12.2). No longer `#[deprecated]`. Coverage in `tests/me_e2e.rs`. |
| `update_me` | **shipped** | Implemented end-to-end. Request `UpdateMeRequest { display_name?, avatar_url? }` → `AuthBackend::update_user_profile`. Returns `MeResponse` 200 + 400/401/404/503. `UserProfile` gained an `avatar_url` field so the patch is genuinely persisted. No longer `#[deprecated]`. |
| `list_my_orgs` | **(c)** | **Honest 501 (canon §4.5).** Principal→orgs enumeration has no end-to-end backing: `MembershipStore` exposes only point role lookups (`get_org_role(org_id, principal)`), not principal→orgs enumeration, and `nebula_storage` ships no `OrgRepo` impl. Closes with the org/membership phase. Still `#[deprecated]` + 501; tag `me (planned)`. |
| `list_my_tokens` | **shipped** | Implemented end-to-end via `AuthBackend::list_pats`. Returns `MyTokensResponse` 200 + 401/404/503. Metadata only — the secret is never recoverable (only its SHA-256 is stored). No longer `#[deprecated]`. |
| `create_token` | **shipped** | Implemented end-to-end via `AuthBackend::create_pat`. Request `CreateTokenRequest { name, scopes, ttl_seconds }` → `CreateTokenResponse { token, summary }` 201 + 400/401/404/503. The plaintext is exposed exactly once (still `write_only` in the spec) and zeroized from handler memory after the response is built; no secret in logs/errors (asserted by `create_token_plaintext_never_leaks_to_logs`). |
| `delete_token` | **shipped** | Implemented end-to-end via `AuthBackend::revoke_pat`. Returns `AckResponse` 200 + 401/404/503. Owner-scoped: a PAT owned by a different principal is reported as 404 (no cross-user existence disclosure). No longer `#[deprecated]`. |

5 of 6 graduated stub→implemented end-to-end via the Plane-A `AuthBackend`
port (the `deprecated`+501 → 200/201 spec graduation landed with the
handlers, mirroring `terminate_execution`). `list_my_orgs` also
graduated stub→implemented in Phase 3 "Option 1" — real end-to-end via
`MembershipStore::list_orgs_for_principal`; **no `me` class-(c) stub
remains** (canon §4.5).

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
| — | `terminate_execution` | **shipped** | Implemented end-to-end via the durable control queue (canon §12.2): CAS-transition to `Cancelled` + enqueue `ControlCommand::Terminate`, consumed by `EngineControlDispatch::dispatch_terminate` (ADR-0008 A3 / ADR-0016). Returns `ExecutionResponse` 200 + 400/401/403/404/409/503. Mirrors `cancel_execution`; no longer `#[deprecated]`. Parity coverage in `tests/execution_terminate_e2e.rs`. |
| 593 | `restart_execution` | **(c)** | Stub. Planned: `RestartExecutionResponse { new_execution_id }`. Underlying milestone: execution restart semantics. |

`restart_execution` is class **(c)**; `terminate_execution` graduated to shipped. Tag: `workspaces.executions` (no `(planned)` suffix because most executions handlers are shipped — only `restart_execution` is deprecated).

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
| (a) Typed-able shipped | 16 | Base typed-shipped (auth ×3 + credential ×1 + health ×1 + openapi ×1) + the Phase 1/2/4 + Phase-3 "Option 1" graduations: `execution::terminate`; 5 `me/*` (`get_me`/`update_me`/`list_my_tokens`/`create_token`/`delete_token`); `me::list_my_orgs`; the 3 org member endpoints (`list_members`/`add_member`/`remove_member`); 5 credential CRUD. New DTOs added in T3: `AckResponse`, `VersionInfo`. |
| (b) Opaque-shipped | 0 | None observed in the current codebase. |
| (c) Stub (501-equivalent) | 8 (org ×6 + resource ×1 + execution ×1) | Originally 18. Graduated end-to-end and removed from the inventory: `execution::terminate` (Phase 1, ADR-0008 A3 / ADR-0016); 5 `me/*` (`get_me`/`update_me`/`list_my_tokens`/`create_token`/`delete_token`, Phase 2, Plane-A `AuthBackend`); `me::list_my_orgs` + 3 org member endpoints (`list_members`/`add_member`/`remove_member`, Phase 3 "Option 1", `MembershipStore`). Remaining stubs apply Stub Endpoint Policy (`deprecated = true`, 501 response, planned-shape DTOs): org-record `get`/`update`/`delete_org` ×3, service-account `list`/`create`/`delete` ×3, `resource::list_resources` ×1, `execution::restart` ×1. The runtime inventory in `tests/openapi_canon_compliance.rs` enumerates exactly these 8. |
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
- `nebula_core::OrgRole` — appears in `MyOrgsResponse`/`OrgSummary`/`MemberSummary` DTOs. **Wrapped as `OrgRoleDto(String)`** in `crates/api/src/domain/shared.rs` with the canonical bidirectional wire-token mapping (`member`/`billing`/`admin`/`owner`); the org member endpoints + `me/list_my_orgs` are live (Phase 3). *Done.*
- `nebula_core::WorkspaceRole` — **Wrapped as `WorkspaceRoleDto(String)`** in `crates/api/src/domain/shared.rs` (no live endpoint emits it yet — kept for the planned workspace-membership DTOs).
- `nebula_core::OrgId` / `WorkspaceId` — already exposed as `String` (ULID) in shipped DTOs (`WorkflowResponse`, etc.). No new wrapper.
- `crate::middleware::auth::AuthContext` — extension only. No wrapper.
- `crate::auth::dto::SecretString` — kept as-is at the request body level; spec annotation `#[schema(value_type = String, format = "password", write_only = true)]` redacts it. **Verify the runtime redaction test in T3 catches accidental serialization.**

## Open follow-ups (not blocking M3.2)

- `me` — **done** (5 of 6): `get_me`, `update_me`, `list_my_tokens`, `create_token`, `delete_token` implemented end-to-end via the Plane-A `AuthBackend` port; the `deprecated`+501 → 200/201 spec graduation landed with the handlers. Only `list_my_orgs` remains an honest 501 (canon §4.5: principal→orgs enumeration has no wired backing until the org/membership phase — it closes there, not here).
- `org`, `resource` business-logic implementation. Each closes one or more class-(c) entries above. When that PR lands, removing `deprecated` + 501 from the spec is a one-line diff per handler.
- `execution::terminate` — **done**: implemented end-to-end via the durable control queue (ADR-0008 A3 / ADR-0016); the `deprecated`+501 → 200 one-line spec graduation landed with the handler. `execution::restart` semantics — defer to engine team; spec is ready when the handler is.
- `WebSocket` real-time transport — deferred to ROADMAP 1.1 per RESEARCH.md.
