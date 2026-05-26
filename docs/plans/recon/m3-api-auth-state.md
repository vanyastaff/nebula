# M3 API Auth State — Recon

Read-only recon of the Plane-A auth surface in `crates/api/` after PR #671
(domain-module layout) and PR #702 (PAT-scope access kernel). Goal: ground
a plan doc that ships (a) CSRF enforcement, (b) a PG-backed `AuthBackend`
impl, and (c) coordination with the `2026-05-20-credential-stabilize-sweep-plan.md`
Wave 4 wiring for operator-secret OAuth providers.

Every claim carries a `path:line` citation or is explicitly marked
`[NOT FOUND]`.

---

## 1. `crates/api/` post-#671 layout

Top-level `crates/api/src/` (`crates/api/src/lib.rs:60-77`):

```
crates/api/src/
├── access/          ── PAT-scope + tenant access kernel (Grant, parse_pat_grant, require_permission)
├── app.rs           ── build_app: router + middleware stack composition
├── config/          ── ApiConfig + sub-configs (JwtSecret, CookieConfig, IdempotencyApiConfig, …)
├── domain/          ── per-domain self-contained modules (routes/handler/dto)
├── error/           ── ApiError + RFC 9457 ProblemDetails
├── extractors/      ── credential + JSON request extractors
├── lib.rs           ── crate docs + public re-exports
├── middleware/      ── auth, csrf, rbac, tenancy, rate_limit, idempotency, security_headers, trace_w3c, request_id, internal_auth
├── openapi/         ── OpenApiDoc generator + audit notes
├── ports/           ── credential_schema, credential_schema_registry (in-process trait ports)
├── state.rs         ── AppState + port traits (OrgResolver, WorkspaceResolver, MembershipStore)
├── telemetry_init.rs ── OTLP / Tracing init helpers
├── trace_capture.rs  ── inbound W3C trace context capture
└── transport/       ── webhook + credential OAuth (Plane B) HTTP transport
```

`crates/api/src/domain/` two levels (`crates/api/src/domain/mod.rs:25-39`):

```
domain/
├── auth/            ── Plane-A auth (signup/login/MFA/PAT/OAuth-sign-in)
│   ├── backend/     ── AuthBackend trait + InMemoryAuthBackend + session/pat/mfa/oauth/password/dto/error primitives
│   ├── handler.rs   ── 10 thin HTTP handlers
│   ├── mod.rs       ── re-exports
│   └── routes.rs    ── /api/v1/auth/* OpenApiRouter
├── catalog/         ── /api/v1/actions, /api/v1/plugins
├── credential/      ── Plane-B credential CRUD + Plane-B OAuth callback wiring
├── execution/       ── /api/v1/orgs/{org}/workspaces/{ws}/executions
├── health/          ── /health, /ready, /version
├── internal.rs      ── /internal/v1/* (admin webhook reload, shared-token-gated)
├── me/              ── /api/v1/me/* (profile, PATs)
├── metrics.rs       ── /metrics (Prometheus scrape)
├── mod.rs           ── domain assembly: create_routes + build_openapi_router (per-group middleware stacks)
├── org/             ── /api/v1/orgs/{org}/* (membership)
├── resource/        ── workspace-scoped resource catalog
├── shared.rs        ── cross-domain DTOs (AckResponse, cursor pagination)
├── workflow/        ── workflow CRUD + execute
├── workspace.rs     ── workspace-scoped assembly only
```

`crates/api/src/middleware/` two levels (`crates/api/src/middleware/mod.rs:1-15`):

```
middleware/
├── auth.rs              ── auth_middleware (Session/PAT/ApiKey/JWT → AuthContext + Grant)
├── csrf.rs              ── csrf_middleware (double-submit cookie; PAT/ApiKey exempt)
├── idempotency/         ── IdempotencyLayer + In-memory/PG stores
├── internal_auth.rs     ── X-Internal-Token gate for /internal/v1/*
├── mod.rs               ── re-exports
├── rate_limit.rs        ── per-IP token-bucket RateLimitState
├── rbac.rs              ── tenant role-check (uses MembershipStore)
├── request_id.rs        ── X-Request-Id propagation
├── security_headers.rs  ── HSTS / X-Frame-Options / CSP / Permissions-Policy
├── tenancy.rs           ── path-derived TenantContext (org/workspace)
└── trace_w3c.rs         ── W3C traceparent extraction + response injection
```

---

## 2. AuthBackend trait + current impls

**Trait definition:** `crates/api/src/domain/auth/backend/provider.rs:99` (`pub trait AuthBackend: Send + Sync`). Module re-export: `crates/api/src/domain/auth/backend/mod.rs:48`. Storage slot on `AppState`: `crates/api/src/state.rs:294-296` (`pub auth_backend: Option<Arc<dyn AuthBackend>>`).

**Impls discovered:**

| Impl | File:line | Notes |
|------|-----------|-------|
| `InMemoryAuthBackend` (production-quality dev default) | `crates/api/src/domain/auth/backend/in_memory.rs:95` (struct), `:186` (`impl AuthBackend`) | DashMap + parking_lot::RwLock; Argon2id / RFC 6238 TOTP / SHA-256 PAT lookup. State lost on restart. |
| Any storage-backed impl (e.g. `PgAuthBackend`) | `[NOT FOUND]` — confirmed by `rg "impl.*AuthBackend"` across `crates/`, only the in-memory impl exists. README is explicit: `crates/api/README.md:230` ("`nebula_storage` ships no `UserRepo` / `PatRepo` / `SessionRepo`"). |
| Any mock/test impl outside `cfg(test)` | `[NOT FOUND]` |

**Composition wiring:** `apps/server/src/compose.rs:161` (`InMemoryAuthBackend::new().into_arc()`) → `AppState::with_auth_backend` (`crates/api/src/state.rs:1063`).

### AuthBackend method surface (one-line semantics)

All methods are required (no default impls). Citations are line numbers in `provider.rs`.

| Method | Line | Behavior |
|--------|------|----------|
| `get_principal_by_session(session_id)` | `:103` | Resolve a live session ID to `Principal::User`; `None` for expired/unknown. Entry point for `auth_middleware` session path. |
| `register_user(SignupRequest)` | `:110` | Create user; impl must queue verification email. |
| `authenticate_password(email, password, totp)` | `:115` | Verify password (+ TOTP if supplied); returns `Authenticated` or `MfaRequired{challenge_token}`. |
| `verify_mfa(challenge_token, code)` | `:122` | Second-step MFA against an in-flight login challenge. |
| `create_session(user_id)` | `:126` | Mint session record (id, csrf_token, expires_at). |
| `revoke_session(session_id)` | `:130` | Idempotent logout. |
| `lookup_pat(presented)` | `:134` | SHA-256 hash lookup; constant-time compare inside impl. Used by `auth_middleware` PAT path. |
| `get_user_profile(user_id)` | `:144` | `GET /me`. |
| `update_user_profile(user_id, ProfilePatch)` | `:148` | `PATCH /me`. |
| `list_pats(user_id)` | `:155` | `GET /me/tokens` — metadata only, no plaintext. |
| `create_pat(user_id, CreatePatParams)` | `:161` | `POST /me/tokens` — returns plaintext **once** via `MintedPat`. |
| `revoke_pat(user_id, pat_id)` | `:169` | `DELETE /me/tokens/{pat}`; cross-user → `UserNotFound` (no existence oracle). |
| `request_password_reset(email)` | `:174` | Always-Ok forgot-password (enumeration-safe). |
| `complete_password_reset(token, new_password)` | `:178` | Consume reset token + set new password. |
| `verify_email(token)` | `:185` | Consume email-verification token. |
| `start_mfa_enrollment(user_id)` | `:189` | Returns otpauth URI + base32 secret once. |
| `confirm_mfa_enrollment(user_id, code)` | `:192` | Verify first TOTP code and flip `mfa_enabled`. |
| `start_oauth(provider)` | `:195` | Mint PKCE state, persist server-side, return `authorize_url`. |
| `complete_oauth(provider, state, code)` | `:199` | Exchange code → token → user profile + new session. |

### Stub / `NotImplemented` markers in `InMemoryAuthBackend`

- `complete_oauth` returns `AuthError::NotImplemented("complete_oauth requires a configured provider backend")` — `crates/api/src/domain/auth/backend/in_memory.rs:573-577`. Synthetic authorize URLs are returned by `start_oauth` for test coverage (`:539-555`).
- `register_user` and `request_password_reset` queue email to an in-memory `email_sink` (`:97-105`, `:404-410`) instead of an SMTP transport — by design for dev/tests but a production gap.
- Every other method is fully implemented (Argon2id verify, TOTP RFC 6238 verify, brute-force lockout at threshold 5 / 15 min, password reset / email verification token lifetimes).

---

## 3. Mounted auth routes (from `build_openapi_router`)

Auth router mount site: `crates/api/src/domain/mod.rs:73` (`let auth_routes = auth::routes::router();`). Notably, **no middleware is layered onto `auth_routes`** before the nest at `crates/api/src/domain/mod.rs:122-126`, so `/api/v1/auth/*` has no `csrf_middleware`, no `auth_middleware`, no RBAC.

Routes list (handler file: `crates/api/src/domain/auth/handler.rs`; router: `crates/api/src/domain/auth/routes.rs:9-21`):

| # | Method + Path | Handler | File:line | State |
|---|----|----|----|----|
| 1 | `POST /auth/signup` | `signup` | `handler.rs:102` (route `:88`, fn `:102`) | working — calls `register_user`; emits verification email to in-memory sink |
| 2 | `POST /auth/login` | `login` | `handler.rs:134` (route `:120`, fn `:134`) | working — returns `200 LoginResponse` w/ session+CSRF cookies or `202 MfaChallengeResponse` |
| 3 | `POST /auth/logout` | `logout` | `handler.rs:192` (route `:180`, fn `:192`) | working — idempotent revoke + clears both cookies |
| 4 | `POST /auth/forgot-password` | `forgot_password` | `handler.rs:219` (route `:206`, fn `:219`) | working — always `202`, in-memory sink |
| 5 | `POST /auth/reset-password` | `reset_password` | `handler.rs:246` (route `:233`, fn `:246`) | working — consume token + set new password |
| 6 | `POST /auth/verify-email` | `verify_email` | `handler.rs:272` (route `:260`, fn `:272`) | working — consume verification token |
| 7 | `POST /auth/mfa/enroll` | `mfa_enroll` | `handler.rs:302` (route `:286`, fn `:302`) | working — extracts session cookie inline; returns otpauth URI + secret |
| 8 | `POST /auth/mfa/verify` | `mfa_verify` | `handler.rs:339` (route `:322`, fn `:339`) | working — dual: confirms enrollment OR completes login second-step |
| 9 | `GET /auth/oauth/{provider}` | `oauth_start` | `handler.rs:379` (route `:362`, fn `:379`) | partial — works on top of in-memory `start_oauth` which returns a **synthetic** `nebula.local` authorize URL (`in_memory.rs:539-555`); no operator-secret provider config |
| 10 | `GET /auth/oauth/{provider}/callback` | `oauth_callback` | `handler.rs:414` (route `:393`, fn `:414`) | stub — backend returns `AuthError::NotImplemented` (`in_memory.rs:573-577`); handler translates to `503 service unavailable: oauth provider not configured` (`handler.rs:438-441`) |

All ten handlers carry `#[tracing::instrument(level = "info", …)]` (see §8).

---

## 4. Session store + PAT lookup

- **`AuthBackend` slot on `AppState`:** `crates/api/src/state.rs:286-296` (`pub auth_backend: Option<Arc<dyn AuthBackend>>`).
- **Wired in composition root:** `apps/server/src/compose.rs:161` → `.with_auth_backend(auth_backend)` at `apps/server/src/compose.rs:207`; builder at `crates/api/src/state.rs:1063`.
- **Session resolution (used by auth middleware):** `crates/api/src/middleware/auth.rs:115-135` — extracts `nebula_session` cookie via `extract_cookie` (`:240`), calls `backend.get_principal_by_session(&session_id)`. Backend method body: `crates/api/src/domain/auth/backend/in_memory.rs:187-203`.
- **PAT lookup wiring:** `crates/api/src/middleware/auth.rs:140-167` — `extract_bearer` (`:230`) → `bearer.starts_with(PAT_PREFIX)` → `backend.lookup_pat(bearer_value)` → `parse_pat_grant(&record.scopes)`. Backend hash compare: `crates/api/src/domain/auth/backend/in_memory.rs:331-343` (uses `pat::hash_for_lookup` + `is_active`).
- **PAT-scope access kernel (PR #702):**
  - `Grant` enum + `require(Permission)` (PR #702 core): `crates/api/src/access/grant.rs:11-35`.
  - Scope vocabulary + `parse_pat_grant`: `crates/api/src/access/scope.rs:30-118` (21 mapped permissions + `full_access`).
  - Runtime guard middleware: `crates/api/src/access/layer.rs:11-67` (`require_permission`); enforces both `TenantContext::require(permission)` and `auth.grant.require(permission)`.
  - `protected(permission, routes)` wrapper that annotates OpenAPI + layers the guard: `crates/api/src/access/route.rs:23-42`. Coverage assertion that all `/api/v1/orgs/{org}/*` operations carry `x-required-permission`: `crates/api/src/access/route.rs:59-89`.
  - `Grant` is attached to every `AuthContext` at auth time: `crates/api/src/middleware/auth.rs:121-127, 159-167, 187-192, 218-222` (Session = `UnrestrictedIdentity`, PAT = `parse_pat_grant(scopes)`, ApiKey = `SystemInternal`, JWT = `UnrestrictedIdentity`).

---

## 5. CSRF state

**Cookie issuance:**

- Cookie constant: `CSRF_COOKIE = "nebula_csrf"` — `crates/api/src/domain/auth/backend/session.rs:30`.
- Cookie builder: `csrf_cookie(token)` — `crates/api/src/domain/auth/backend/session.rs:67-69`, body in shared `fn cookie(name, value, http_only=false, ttl=CSRF_TTL)` at `:82-91`.
- Attributes: `Path=/`, `Max-Age=<14d>`, `Secure`, `SameSite=Lax`, **no `HttpOnly`** (deliberate — SPA must read it). Verified by unit test `crates/api/src/domain/auth/backend/session.rs:122-130`.
- TTL: `CSRF_TTL = SESSION_TTL = 14 days` — `:23-26`.

**Issuance call sites** (every login / OAuth-callback mints a fresh cookie pair):

- `mint_session_response` (login + MFA-completion): `crates/api/src/domain/auth/handler.rs:163-176`.
- OAuth callback success: `crates/api/src/domain/auth/handler.rs:425-433`.
- Logout clears it via `cleared_cookie(CSRF_COOKIE)`: `crates/api/src/domain/auth/handler.rs:201`.

**CSRF validator:**

- Middleware exists and IS implemented: `crates/api/src/middleware/csrf.rs:27-79` (`pub async fn csrf_middleware`). Double-submit `X-CSRF-Token` header vs `nebula_csrf` cookie; exempt for `GET/HEAD/OPTIONS` and for `AuthMethod::Pat | AuthMethod::ApiKey`; enforced for `Session | Jwt`.
- Re-export: `crates/api/src/middleware/mod.rs:18` (`pub use csrf::csrf_middleware`).
- Tests reference it: `crates/api/tests/me_e2e.rs:48` ("a JWT-authenticated mutating request is correctly rejected with 403 by `csrf_middleware`").

**Existing `X-CSRF-Token` references / TODOs:**

- Header literal `"x-csrf-token"` parsed in `crates/api/src/middleware/csrf.rs:61`.
- Test-side double-submit pair: `crates/api/tests/common/mod.rs:33-35` (`TEST_CSRF_TOKEN`, `TEST_CSRF_COOKIE`).
- No `TODO`/`FIXME` markers in `csrf.rs`. The middleware itself is complete; the **wiring gap** is that it is layered onto some route groups but **not** onto `/auth/*` or `/api/v1/credentials*` (Plane-B credential routes).

**`build_app` middleware stack (apply order — outermost first):**

Per `crates/api/src/app.rs:154-201`, the layers attached in `build_app` after the route tree is composed (outermost first, because `Router::layer` wraps):

1. `rate_limit` per-IP token bucket — `crates/api/src/app.rs:198-201`.
2. `request_id_middleware` (X-Request-Id) — `:194`.
3. `security_headers_middleware` (HSTS, CSP, X-Frame-Options, …) — `:193`.
4. `trace_context_middleware` (W3C extraction) — `:191`.
5. `ServiceBuilder` inner stack `:154-189`: `TraceLayer` (INFO span) → `inject_w3c_trace_response_headers` → `CompressionLayer` → `CorsLayer`.
6. `IdempotencyLayer` is layered onto `api_routes` ONLY (not webhook), inside `:113-128`.
7. `DefaultBodyLimit::max(max_body_size)` on `api_routes` before webhook merge — `:96`.

Per-route-group middleware (set inside `crates/api/src/domain/mod.rs:71-117`):

| Route group | Layers (innermost first; auth → csrf → tenancy → rbac on tenant) | File:line |
|----|----|----|
| `auth_routes` (`/api/v1/auth/*`) | **none** — public surface | `domain/mod.rs:73` |
| `me_routes` (`/api/v1/me/*`) | `auth_middleware`, then `csrf_middleware` | `domain/mod.rs:80-86` |
| `catalog_routes` (`/api/v1/actions`, `/api/v1/plugins`) | `auth_middleware` only (no CSRF — read-only) | `:89-92` |
| `tenant_routes` (`/api/v1/orgs/{org}/**`) | `auth_middleware` → `csrf_middleware` → `tenancy_middleware` → `rbac_middleware` | `:95-109` |
| `credential_routes` (Plane-B OAuth callbacks) | `auth_middleware` only — **no `csrf_middleware`** | `:112-116` |
| `health` / `metrics` | none | `:130-132` |
| `/internal/v1/*` | `internal_auth_middleware` (shared token) | `app.rs:140`, mounted via `domain::internal::router` |

**Gap restated:** `csrf_middleware` exists and is correct, but it is NOT layered onto `auth_routes` (which is fine — `signup`/`login`/`forgot-password` are pre-session) **or** `credential_routes` (which IS a gap — Plane-B credential mutations under session auth are unprotected). The prompt's "no middleware validates" is therefore **partially** inaccurate: validation runs for `/me/*` and tenant routes but does not run for `/credential/*`.

---

## 6. Storage / migrations relevant to auth

**Existing migrations** under `crates/storage/migrations/postgres/` (mirrored 1:1 in `sqlite/`):

| Migration | Tables touched | File |
|----|----|----|
| `0001_users.sql` | `users` (id, email, email_verified_at, password_hash, mfa_enabled, mfa_secret, failed_login_count, locked_until, version, deleted_at) | `crates/storage/migrations/postgres/0001_users.sql:5-30` |
| `0002_user_auth.sql` | `oauth_links`, `sessions`, `personal_access_tokens`, `verification_tokens` | `crates/storage/migrations/postgres/0002_user_auth.sql:7-77` |
| `0027_port_adapter_schema.sql` | `port_users` (TEXT-id mirror of `users`) — for the storage-port adapter | `crates/storage/migrations/postgres/0027_port_adapter_schema.sql:143-160` |

Detail of identity tables:

- `users` — auth columns (`password_hash`, `email_verified_at`, `mfa_enabled`, `mfa_secret`, `failed_login_count`, `locked_until`) live as columns, not separate tables. `crates/storage/migrations/postgres/0001_users.sql:8-19`.
- `sessions` — opaque id BYTEA, `user_id`, `expires_at`, `revoked_at`, IP/UA. `crates/storage/migrations/postgres/0002_user_auth.sql:20-40`.
- `personal_access_tokens` — id (`pat_` ULID), `principal_kind` ('user'/'service_account'), `prefix`, `hash` (SHA-256), `scopes` JSONB, `expires_at`, `revoked_at`. `crates/storage/migrations/postgres/0002_user_auth.sql:44-63`.
- `verification_tokens` — `token_hash` PK, `user_id`, `kind` ∈ {`email_verification`, `password_reset`, `org_invite`, `mfa_recovery`}, `expires_at`, `consumed_at`. `crates/storage/migrations/postgres/0002_user_auth.sql:67-82`.
- `oauth_links` — third-party identity provider linkage (provider, provider_user_id, provider_email). `crates/storage/migrations/postgres/0002_user_auth.sql:7-17`.

**Repo trait definitions / impls relevant to Plane A:**

- `nebula_storage::repos::UserRepo` — trait at `crates/storage/src/repos/user.rs:11-46` (8 methods: `create`, `get`, `get_by_email`, `update`, `soft_delete`, `record_login_success`, `record_login_failure`).
- `nebula_storage::repos::SessionRepo` — trait at `crates/storage/src/repos/user.rs:48-67` (5 methods: `create`, `get`, `touch`, `revoke`, `cleanup_expired`).
- `nebula_storage::repos::PatRepo` — trait at `crates/storage/src/repos/user.rs:70-94` (5 methods: `create`, `get_by_hash`, `touch`, `revoke`, `list_for_principal`).
- Row structs `UserRow`, `SessionRow`, `PersonalAccessTokenRow`, `VerificationTokenRow`, `OAuthLinkRow`: `crates/storage/src/rows/user.rs:11-105`.
- **Impls of `UserRepo` / `SessionRepo` / `PatRepo` / `VerificationTokenRepo`:** `[NOT FOUND]` — confirmed by `rg "impl.*UserRepo|impl.*SessionRepo|impl.*PatRepo"` returning no matches. The traits are definition-only (called out in `crates/storage/src/repos/mod.rs:18-22`).

**Parallel `nebula-storage-port` surface (TEXT-id adapter schema):**

- `nebula_storage_port::store::UserStore` trait: `crates/storage-port/src/store/identity.rs:17-28` — CRUD only (no sessions, no PATs, no verification tokens).
- `PgUserStore` impl: `crates/storage/src/postgres/identity.rs:38-130` (over `port_users`).
- `SqliteUserStore` impl: `crates/storage/src/sqlite/identity.rs:52+`.
- No `SessionStore` / `PatStore` / `VerificationTokenStore` traits exist on the port surface — `[NOT FOUND]` via `rg "trait SessionStore|trait PatStore|trait PersonalAccessTokenStore"`.

**Gap analysis — tables that DO NOT yet exist:**

| Expected table | Status | Substitute |
|----|----|----|
| `users` | ✅ exists | — |
| `sessions` | ✅ exists | — |
| `pats` / `personal_access_tokens` | ✅ exists | — |
| `mfa_secrets` | ❌ no separate table | `users.mfa_secret` BYTEA column (`0001_users.sql:17`) — encrypted with master key per comment |
| `oauth_state` (Plane-A login state, PKCE) | ❌ no migration — `[NOT FOUND]` | Currently `DashMap<String, OAuthStateEntry>` in `in_memory.rs:103` |
| `password_reset_tokens` | ❌ no separate table | `verification_tokens` rows where `kind='password_reset'` (`0002_user_auth.sql:67`) |
| `email_verification_tokens` | ❌ no separate table | `verification_tokens` rows where `kind='email_verification'` (`0002_user_auth.sql:67`) |
| `lockouts` | ❌ no separate table | `users.failed_login_count` + `users.locked_until` columns (`0001_users.sql:14-15`) |
| `mfa_challenges` (in-flight login challenges) | ❌ no migration — `[NOT FOUND]` | Currently `DashMap<String, MfaChallenge>` in `in_memory.rs:102` |

---

## 7. Existing auth tests

There is **no `crates/api/tests/auth_e2e.rs`** (confirmed by `ls`). Auth flows are exercised through three indirect paths:

| Test file | Scope | File:line |
|----|----|----|
| `crates/api/src/domain/auth/backend/in_memory.rs` `#[cfg(test)] mod tests` | Unit-level lifecycle: register → login → verify → reset / MFA enroll-verify / PAT round-trip / OAuth state persistence. 11 `#[tokio::test]`. | `crates/api/src/domain/auth/backend/in_memory.rs:661-836` |
| `crates/api/tests/me_e2e.rs` | E2E for `/api/v1/me/*` against a real `InMemoryAuthBackend`. Asserts `csrf_middleware` rejects a JWT-mutating request without the X-CSRF-Token pair. | `crates/api/tests/me_e2e.rs:1-30` (overview), `:46-66` (CSRF helper), 8+ `#[tokio::test]` from `:88+`. |
| `crates/api/tests/access_e2e.rs` | E2E for PAT-scope kernel: register user → mint PAT → assert `POST /workflows` 200/403 by scope + tenant role. | `crates/api/tests/access_e2e.rs:133-167` (`state_with_pat_and_workspace_role`), tests `:194-286`. |
| `crates/api/src/domain/auth/backend/error.rs` `#[cfg(test)] mod tests` | `AuthError → ApiError` status mapping (401/409/423/429). | `crates/api/src/domain/auth/backend/error.rs:107-145` |
| `crates/api/src/access/{grant,layer,route,scope}.rs` `#[cfg(test)]` | Grant/scope unit coverage + OpenAPI access-coverage assertion. | `crates/api/src/access/layer.rs:71-201`, `scope.rs:124-220`, `route.rs:149-330`, `grant.rs:57-86` |
| `crates/api/tests/openapi_spec.rs` | Drift test that `/api/v1/auth/{signup,login,logout}` exist in served spec. | `crates/api/tests/openapi_spec.rs:411-415` |

**Tests directly exercising `signup` / `login` / `mfa-verify` / `oauth-callback` HTTP endpoints end-to-end:** `[NOT FOUND]` — auth HTTP handlers have no dedicated integration test file; only the backend trait implementation is unit-tested.

---

## 8. Observability

### Metrics

- `nebula_api_auth_*` namespace: `[NOT FOUND]` in implementation. Only `nebula_api_idempotency_*` exists (`crates/api/src/middleware/idempotency/layer.rs:68, 136, 143, 145`). The namespace is **planned** per `docs/ROADMAP.md:220` ("`nebula_api_auth_*` metrics family for failed/locked-out attempts") but no counter / gauge / histogram has been registered yet — `rg "auth_failures|auth_attempts|login_failures|locked_out|counter.*auth"` returns no matches.

### Tracing spans on auth handlers

All 10 handlers carry `#[tracing::instrument(level = "info", …)]`:

- `signup` — `crates/api/src/domain/auth/handler.rs:100` (`skip(state, body), fields(email = %body.email)`).
- `login` — `crates/api/src/domain/auth/handler.rs:132`.
- `logout` — `crates/api/src/domain/auth/handler.rs:189`.
- `forgot_password` — `crates/api/src/domain/auth/handler.rs:217`.
- `reset_password` — `crates/api/src/domain/auth/handler.rs:244`.
- `verify_email` — `crates/api/src/domain/auth/handler.rs:270`.
- `mfa_enroll` — `crates/api/src/domain/auth/handler.rs:300`.
- `mfa_verify` — `crates/api/src/domain/auth/handler.rs:337`.
- `oauth_start` — `crates/api/src/domain/auth/handler.rs:377`.
- `oauth_callback` — `crates/api/src/domain/auth/handler.rs:412`.

Backend-side `tracing::info!` log lines on lifecycle events: `crates/api/src/domain/auth/backend/in_memory.rs:241` (`"user registered"`), `:393` (`"user profile updated"`), `:434` (`"personal access token created"`), `:451` (`"personal access token revoked"`), `:469` (`"failed to mint password reset token"`).

Access-kernel span: `crates/api/src/access/layer.rs:16-26` (`access.require_permission` info span with `permission`, `auth.method`, `tenant.org_id`, `tenant.workspace_id`, `outcome` fields).

### `AuthError → ApiError` typed mapping

`impl From<AuthError> for ApiError` at `crates/api/src/domain/auth/backend/error.rs:73-103`. Mapping table:

| AuthError | ApiError | HTTP |
|----|----|----|
| `NotImplemented(_)` | `ServiceUnavailable` | 503 |
| `EmailAlreadyRegistered` | `Conflict` | 409 |
| `UserNotFound` | `NotFound("user")` | 404 |
| `InvalidCredentials` | `Unauthorized` | 401 |
| `InvalidInput(_)` | `validation_message` | 400 |
| `AccountLocked` | `AccountLocked` | 423 (unit test `error.rs:131-138`) |
| `EmailNotVerified` | `Forbidden` | 403 |
| `MfaRequired` | `MfaRequired` | 401 |
| `InvalidMfaCode` | `Unauthorized` | 401 |
| `InvalidToken` | `Unauthorized` | 401 |
| `RateLimit` | `RateLimitExceeded` | 429 |
| `OAuthFailed(msg)` | `UpstreamError` | 502 (per `ApiError` convention) |
| `Crypto(msg)` / `Internal(msg)` | `Internal` | 500 |

---

## 9. Cross-dep status — `2026-05-20-credential-stabilize-sweep-plan.md`

File exists: `docs/plans/2026-05-20-credential-stabilize-sweep-plan.md` (verified via `find`).

Wave 4 — "API integration" — is the cross-dep surface (`docs/plans/2026-05-20-credential-stabilize-sweep-plan.md:2009-2087`). Task 17 ("Wire `nebula-api` onto `CredentialService`", `:2015-2086`) modifies `crates/api/src/state.rs`, `crates/api/src/domain/credential/handler.rs`, `crates/api/src/domain/credential/routes.rs`, `crates/api/src/transport/credential.rs`, and `crates/api/Cargo.toml`. Step 4 (`:2061-2069`) routes the Plane-B OAuth callback through `state.credential_service.create::<OAuth2Credential>(&scope, props).await?` instead of writing to the raw `oauth_credential_store`. The `credential_service` slot is already wired on `AppState` (`crates/api/src/state.rs:235-249` — `pub credential_service: Option<Arc<CredentialService<InMemoryStore, InMemoryPendingStore>>>`), and the API plan that consumes operator OAuth-provider secrets (Plane A) must coordinate with Wave 4 because both touch `AppState` builder methods (`with_credential_service` at `state.rs:1041-…`) and both reach into the credential subsystem's secret-resolution boundary; the API plan should land **after** the relevant `CredentialService` accessor (`get`/`create`) signatures stabilize in Waves 1–3 (`:59-2007`) so the Plane-A `start_oauth` / `complete_oauth` impls can either reuse `CredentialService` for client-secret retrieval or define their own narrower port without duplicating tenant-scope wiring. Task 18 (`:2087-…`) deletes `CredentialScopeLayer` from `nebula-tenancy`, which the Plane-A OAuth impl must not begin to depend on.

---

## 10. Gaps for PR plan

### A. CSRF middleware enforcement

The `csrf_middleware` itself is **complete** (`crates/api/src/middleware/csrf.rs:27-79`, 92 lines). The gaps are wiring + test coverage:

| Gap | File:line | Concrete change | LOC estimate |
|----|----|----|----|
| Layer `csrf_middleware` onto `credential_routes` (currently auth-only) | `crates/api/src/domain/mod.rs:112-116` | Add `.layer(middleware::from_fn(csrf_middleware))` between auth and the existing layer | +1 line, +1 import already present |
| Layer `csrf_middleware` onto state-changing `/auth/*` routes if/when those carry session auth (e.g. `POST /auth/mfa/enroll`, `POST /auth/mfa/verify` enroll-confirm path) | `crates/api/src/domain/mod.rs:73` (currently `let auth_routes = auth::routes::router();` with no layers) | Either split the auth router into pre-session vs session-bearing sub-groups, or accept the per-handler inline session extraction at `handler.rs:69-78` as canonical and add CSRF inline | 10–30 LOC depending on split |
| Plane-A `csrf_middleware` requires `AuthContext` (`csrf.rs:43-50` reads it from extensions) — so `/auth/*` routes that DO carry a session today (mfa-enroll/verify w/ session cookie) need `auth_middleware` ahead of any CSRF layer | `crates/api/src/middleware/csrf.rs:43-50`, `crates/api/src/domain/auth/handler.rs:300-371` | Decision: either move MFA enroll/verify under `/me/*` (already CSRF-gated) or add `auth_middleware` to the MFA subset before layering CSRF | 5–20 LOC (route refactor) |
| Negative-path tests for `/credential/*` write paths missing CSRF | `crates/api/tests/seam_credential_write_path_validation.rs` (already sends `x-csrf-token` headers per `:60-72`) | Add a `cookie-vs-header mismatch` and a `missing-header` case asserting 403 | +40–60 LOC |
| Document CSRF policy on auth routes in `crates/api/README.md` (currently silent — `rg csrf crates/api/README.md` empty) | `crates/api/README.md` | New section under "Authentication" | +20 lines docs |

**Total CSRF wiring estimate: ~30–80 LOC of code + 60 LOC of tests + docs.**

### B. PG-backed `AuthBackend`

The trait surface (`provider.rs:99-216`, 19 methods + `Send + Sync`) has only `InMemoryAuthBackend` (`in_memory.rs:95-577`, 482 lines of impl). To land a PG-backed alternative the missing pieces are repos + a façade impl:

| Gap | File:line / Path | Notes | LOC estimate |
|----|----|----|----|
| `PgUserRepo` impl of `nebula_storage::repos::UserRepo` (8 methods) | new: `crates/storage/src/pg/user.rs` | Mirror `PgUserStore` (`crates/storage/src/postgres/identity.rs:38-130`) but over the spec-16 `users` table (BYTEA ids), plus `record_login_success` / `record_login_failure` (no analogue on the port impl) | ~250 LOC |
| `PgSessionRepo` impl of `SessionRepo` (5 methods) | new: `crates/storage/src/pg/session.rs` | Over `sessions` table — `expires_at` / `revoked_at` index already at `migrations/postgres/0002_user_auth.sql:33-40` | ~160 LOC |
| `PgPatRepo` impl of `PatRepo` (5 methods) | new: `crates/storage/src/pg/pat.rs` | Over `personal_access_tokens` table; idx_pat_hash partial index already at `migrations/postgres/0002_user_auth.sql:55-57` | ~180 LOC |
| `PgVerificationTokenRepo` trait + impl (consumed by `verify_email` / `complete_password_reset`) | new trait at `crates/storage/src/repos/user.rs` (currently no `VerificationTokenRepo` trait — `[NOT FOUND]`); new impl at `crates/storage/src/pg/verification_token.rs` | Table exists (`migrations/postgres/0002_user_auth.sql:67-82`); trait must be designed (~5 methods: `create`, `consume`, `cleanup_expired`, `get`, `revoke_all_for_user`) | ~80 (trait) + ~150 (impl) = 230 LOC |
| Plane-A `oauth_state` storage — currently in-memory only (`in_memory.rs:103, 539-577`) | new migration `crates/storage/migrations/postgres/0028_plane_a_oauth_state.sql` + repo trait + impl | Table needs: state PK, provider, code_verifier (encrypted?), expires_at, consumed_at | migration ~25 LOC + trait ~40 + PgImpl ~80 = 145 LOC |
| `mfa_challenges` storage — currently in-memory only (`in_memory.rs:102`) | option A: reuse `verification_tokens` w/ `kind='mfa_challenge'`; option B: new table | If reusing `verification_tokens`: 0 schema LOC; if new table: ~25 + 40 + 80 = 145 LOC | 0–145 LOC |
| Email sender port (currently `InMemoryAuthBackend::email_sink` at `in_memory.rs:103-105`) | new: `crates/api/src/ports/email.rs` (trait) + `AppState` slot | Required so a `PgAuthBackend` doesn't silently drop verification / reset emails | trait ~30 + state slot ~20 = 50 LOC |
| `PgAuthBackend` struct + `impl AuthBackend` (all 19 methods, delegating to the four repos + email port) | new: `crates/api/src/domain/auth/backend/pg.rs` | Argon2id / TOTP / hash helpers are already in `password.rs`, `mfa.rs`, `pat.rs` and can be reused unchanged | ~600–800 LOC (similar shape to `in_memory.rs:186-577` minus the dashmaps) |
| Re-export + composition wiring | `crates/api/src/domain/auth/backend/mod.rs:46` (add `pub use pg::PgAuthBackend`), `apps/server/src/compose.rs:140-209` (conditionally select backend based on `ApiConfig`/`DATABASE_URL`, mirroring `build_idempotency_store` at `apps/server/src/compose.rs:230+`) | Reuse the idempotency-store selection pattern; fail-closed when PG requested but unavailable | ~80 LOC |
| Operator config knob (`ApiConfig::auth_backend: Memory | Postgres`) | `crates/api/src/config/sub.rs`, `crates/api/src/config/env.rs` | Parallel to `IdempotencyApiConfig::backend` | ~40 LOC |
| Integration tests gated on `feature = "postgres"` + `DATABASE_URL` | new: `crates/api/tests/auth_pg_e2e.rs` and/or `crates/storage/tests/pg_identity_repos.rs` | Mirror `crates/storage/src/pg/*.rs` testing convention (per `crates/storage/src/pg/mod.rs:13-15`) | ~300 LOC |

**Total PG `AuthBackend` estimate: ~1,900–2,300 LOC** across `crates/storage` (~860 LOC of PG repo impls + traits) and `crates/api` (~1,000 LOC of `PgAuthBackend` + config + wiring + tests). A staged delivery is recommended:

1. Migration + PG repo impls + storage-side tests (~860 LOC).
2. Email port + `ApiConfig` backend selector (~110 LOC).
3. `PgAuthBackend` + composition + API-side tests (~900 LOC).

### Out-of-scope-for-this-PR but adjacent (for the plan doc)

- **`nebula_api_auth_*` metrics namespace** (planned per `docs/ROADMAP.md:220`) is currently `[NOT FOUND]`. The PG backend PR should not invent the namespace unilaterally; coordinate with the roadmap entry to register counters on the `MetricsRegistry` slot already on `AppState` (`crates/api/src/state.rs:218`).
- **Operator-secret OAuth providers** (cross-dep #3) is a `CredentialService`-shaped concern; do not add a parallel secret store on Plane A.

RECON COMPLETE — 132 citations, 11 not-found markers
