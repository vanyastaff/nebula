# SDD Spec — OAuth providers from operator secrets

- **Change slug**: `oauth-providers-from-operator-secrets`
- **Status**: spec-draft (revised 2026-05-27 by recon-2 + recon-3 + recon-4)
- **Date**: 2026-05-27
- **Predecessor artifacts**: `explore.md`, `proposal.md`, `recon-2-credential-domain.md`, `recon-3-flow-and-pending.md`, `recon-4-n8n-and-rust-ecosystem.md` (this directory).
- **Affected specs**: `auth-backend` (modified), `oauth-flow` (added), `credential-service` (**REMOVED — recon-2**), `app-state-composition` (modified), `observability` (added), `chained-pr-boundary` (added). **Recon-4 simplifications applied**: `redirect_uri` auto-derived from `ApiConfig::public_url` (no allow-list); `OAuthEndpoints` is tagged union `Oidc { discovery_url } | Manual { ... }`; OIDC scopes hardcoded; id_token JWKS validation deferred to 1.1.

> Format: each requirement uses `REQ-{spec}-{nnn}`. Each requirement carries one or more BDD scenarios (`Given / When / Then`). Where a requirement modifies an existing public surface, the diff is marked `MODIFIED:`. Net-new behavior is marked `ADDED:`. Behavior removed from production is `REMOVED:`.

---

## Spec: `oauth-flow` — ADDED

### REQ-oauth-001 — Operator declares OAuth providers via configuration (🟥 RECON-4 REVISED)

**Status**: ADDED.

The server SHALL accept an operator-supplied configuration that maps each supported `OAuthProvider` enum value to:
- `client_id: SecretString` (env-bound from `API_AUTH_OAUTH_<PROVIDER>_CLIENT_ID`)
- `client_secret: SecretString` (env-bound from `API_AUTH_OAUTH_<PROVIDER>_CLIENT_SECRET`)
- `endpoints: OAuthEndpoints` — tagged union:
  - `Oidc { discovery_url: String }` for OIDC-compliant providers — in 1.0 this means Google and Microsoft (the only `Oidc`-shaped variants in the live `OAuthProvider` enum at `crates/api/src/domain/auth/backend/oauth.rs:28-47`). Auth0 / Okta / generic OIDC require extending the enum (1.1 follow-up per ADR-0085 D-5). Endpoints fetched at runtime from `.well-known/openid-configuration` (D-15). **Scopes hardcoded `"openid email profile"` for `Oidc`.**
  - `Manual { authorize_url, token_url, userinfo_url, jwks_url: Option<String>, scopes: Vec<String> }` for OAuth2-only providers (GitHub) or operator-customized OIDC. `jwks_url` accepted for forward compat but ignored in 1.0 (D-16). `scopes` MUST be non-empty for `Manual`.

`redirect_uri` is **NOT a configuration field**. It is auto-derived at runtime as `format!("{}/auth/oauth/{}/callback", api_config.public_url, provider.as_str())` from the existing `ApiConfig::public_url` (`API_PUBLIC_URL` env). Operators that need multiple callback URIs deploy multiple Nebula instances (each with its own `API_PUBLIC_URL` and IdP client registration).

**Invariant 1** (🟥 WAVE-6 anti-SSRF hardening per D-9-WAVE6): Each provider config MUST validate at boot via the generalized **`validate_oauth_outbound_url`** function (renamed from `validate_token_endpoint` in wave-6 to signal coverage of ALL server-side OAuth fetches, not just the token endpoint):
- `client_id` non-empty; `client_secret` non-empty.
- `Oidc.discovery_url` passes `validate_oauth_outbound_url` (HTTPS + no localhost/private/loopback/link-local/multicast).
- `Manual.authorize_url` passes the same gate (operator-config controlled, cheap insurance even though browser fetches it).
- `Manual.token_url`, `Manual.userinfo_url`, and (when `Some`) `Manual.verified_emails_url` + `Manual.jwks_url` each pass `validate_oauth_outbound_url` (server-side fetches — strict policy non-negotiable).
- `Manual.scopes` non-empty.
- `ApiConfig::public_url` set AND absolute (with scheme). Empty/relative `public_url` is a boot-time error.
- **Dynamic OIDC URLs** (validated at first `start_oauth` per D-15-WAVE6): the URLs RETURNED in the `.well-known/openid-configuration` JSON (`authorize_url`, `token_url`, `userinfo_url`, `jwks_url`) MUST each pass `validate_oauth_outbound_url` BEFORE the `OidcDiscovery` is cached. A hostile discovery doc with internal-IP child URLs fails with `DiscoveryError::EndpointSsrfRejected { field, url_host }` and the cache stays empty (no partial entries).

**Invariant 2**: Declaring an OAuth provider is sufficient — there is no separate credential row. Boot validates the config; first OAuth-start call resolves endpoints (via `fetch_oidc_discovery` if `Oidc`) and may surface `AuthError::OAuthFailed { cause: "oidc_discovery_failed" }` if the discovery URL is unreachable. Caching is process-wide per discovery URL.

**Scenarios**:

- **Scenario 1.1 — Successful boot with declared Oidc provider** (🟥 RECON-4)
  - **Given** `[auth.oauth.providers.google]` has valid `client_id`, `client_secret`, `endpoints = { kind = "oidc", discovery_url = "https://accounts.google.com/.well-known/openid-configuration" }`
  - **And** `API_PUBLIC_URL=https://nebula.example.com` is set
  - **When** the server boots
  - **Then** boot succeeds; `auth: oauth providers wired` log emitted with `count=1`, `provider="google"`, `kind="oidc"`

- **Scenario 1.2 — Successful boot with declared Manual provider** (🟥 RECON-4)
  - **Given** `[auth.oauth.providers.github]` has valid `client_id`, `client_secret`, `endpoints = { kind = "manual", authorize_url, token_url, userinfo_url }` and `scopes = ["user:email"]`
  - **When** the server boots
  - **Then** boot succeeds; per-provider log line `kind="manual"`, `scope_count=1`

- **Scenario 1.3 — Boot fails closed when `public_url` unset but OAuth declared** (🟥 RECON-4)
  - **Given** any `[auth.oauth.providers.*]` entry exists
  - **And** `API_PUBLIC_URL` is empty or unset
  - **When** the server attempts to boot
  - **Then** boot fails with `TransportInitError::OAuthProviderConfigInvalid { provider, reason: "public_url_required" }`

- **Scenario 1.4 — Lazy detection of unreachable OIDC discovery** (🟥 RECON-4)
  - **Given** `[auth.oauth.providers.google]` has `discovery_url = "https://unreachable.example.com/.well-known/openid-configuration"`
  - **When** the server boots
  - **Then** boot succeeds (no network call at boot)
  - **When** the first caller hits `POST /auth/oauth/google/start`
  - **Then** the response is HTTP 502 `UpstreamError`
  - **And** the structured error log includes `provider = "google"`, `cause = "oidc_discovery_failed"`, with no secret material

---

### REQ-oauth-002 — `start_oauth` constructs a real authorize URL with PKCE

**Status**: MODIFIED (replaces the synthetic `https://nebula.local/...` URL behavior).

`PgAuthBackend::start_oauth(provider, redirect_uri)` and `InMemoryAuthBackend::start_oauth(provider, redirect_uri)` SHALL (🟥 RECON-3 + RECON-4 REVISED steps; supersedes the earlier RECON-2 revision):

1. Look up the validated `OAuthProviderConfig` from `ApiConfig::auth.oauth.providers[provider]`. If absent → `AuthError::ProviderNotConfigured { provider }`.
2. Derive the canonical `redirect_uri` from `api_config.public_url` per D-3 (recon-4): `format!("{}/auth/oauth/{}/callback", api_config.public_url, provider.as_str())`. The handler-supplied `redirect_uri` argument MUST already equal this derived value (the handler derived it the same way before calling the trait method); a mismatch is a `debug_assert!` at the trait boundary, not a runtime branch.
3. Resolve the IdP endpoints from `provider_config.endpoints`:
   - `OAuthEndpoints::Oidc { discovery_url }` — call `fetch_oidc_discovery(discovery_url)` (D-15) to obtain `OidcDiscovery { authorize_url, token_url, userinfo_url, jwks_url? }` (cached process-wide). Hardcoded scopes `"openid email profile"`.
   - `OAuthEndpoints::Manual { authorize_url, token_url, userinfo_url, jwks_url?, scopes }` — use as-is; operator-supplied `scopes`.
4. Build the `flow::AuthorizationUriRequest { auth_url, token_url, client_id, client_secret, redirect_uri, scopes, auth_style }` from the resolved endpoints + provider config.
5. Call `mint_pkce()` at `crates/api/src/domain/auth/backend/oauth.rs:~90` to obtain `(state, code_verifier, code_challenge)` (already wired in `PgAuthBackend::start_oauth`).
6. Call `flow::build_authorization_uri(&req, &state, &code_challenge)` at `crates/api/src/transport/oauth/flow.rs:38-60` to construct the real authorize URL (PKCE S256 mandatory by the `PkceMethod` enum).
7. Persist `OAuthStateRow { state, provider, code_verifier, redirect_uri: Some(redirect_uri.to_owned()), created_at, expires_at: now + OAUTH_STATE_TTL, consumed_at: None }` via `self.oauth_state_repo.create(...)` (Plane A `OAuthStateRepo` over the `plane_a_oauth_states` table — NOT `pending_state_store`, which is Plane B).
8. Return `OAuthStart { authorize_url, state }`.

PKCE plain is structurally impossible per Scenario 2.2 (the `PkceMethod` enum has one variant).

> Why these steps differ from the earlier RECON-2 revision: recon-3 audit (`recon-3-flow-and-pending.md` §6) revealed that `OAuth2Credential::initiate_authorization_code` + `AppState::pending_state_store` are Plane B (credential-OAuth) surfaces; Plane A uses the distinct `OAuthStateRepo` already wired into `PgAuthBackend`. Recon-4 (`recon-4-n8n-and-rust-ecosystem.md`) replaced the `redirect_uris` allow-list with `public_url`-derivation.

**Scenarios**:

- **Scenario 2.1 — Real authorize URL emitted (🟥 RECON-3 + RECON-4 REVISED)**
  - **Given** `[auth.oauth.providers.google]` has `client_id = "google-client-1"`, `client_secret = "..."`, `endpoints = { kind = "oidc", discovery_url = "https://accounts.google.com/.well-known/openid-configuration" }`
  - **And** `API_PUBLIC_URL = "https://app.example.com"`
  - **And** the OIDC discovery doc resolves `authorize_url = "https://accounts.google.com/o/oauth2/v2/auth"`
  - **When** the handler derives `redirect_uri = "https://app.example.com/auth/oauth/google/callback"` and calls `start_oauth(OAuthProvider::Google, &redirect_uri)`
  - **Then** the returned `authorize_url` starts with `https://accounts.google.com/o/oauth2/v2/auth?`
  - **And** the query string contains `client_id=google-client-1`
  - **And** the query string contains `redirect_uri=https%3A%2F%2Fapp.example.com%2Fauth%2Foauth%2Fgoogle%2Fcallback` (URL-encoded)
  - **And** the query string contains `code_challenge_method=S256` and a non-empty `code_challenge`
  - **And** the `plane_a_oauth_states` row contains the unencoded `code_verifier` (NOT the challenge) and `redirect_uri = "https://app.example.com/auth/oauth/google/callback"`

- **Scenario 2.2 — PKCE plain is structurally impossible** (🟥 RECON-2)
  - **Given** the `PkceMethod` enum at `crates/credential/src/credentials/oauth2_config.rs:48-65` has exactly one variant (`S256`)
  - **Then** there is no representable `pkce_method = "plain"` value in the codebase
  - **And** `OAuth2Credential::initiate_authorization_code` unconditionally derives `code_challenge = BASE64URL(SHA256(code_verifier))` and emits `code_challenge_method=S256` (no runtime branch)
  - **And** the `OAuthProviderConfig` schema has NO `pkce_method` field (its absence is enforced by the absence of an enum variant)

- **Scenario 2.3 — 🟥 RECON-4 DELETED**
  - `redirect_uri` is auto-derived from `ApiConfig::public_url` per REQ-oauth-001 — there is no allow-list to validate against. The defense equivalent for "`public_url` change mid-flow" lives in REQ-oauth-003 Scenario 3.10.

---

### REQ-oauth-003 — `complete_oauth` performs real token exchange against IdP

**Status**: MODIFIED (replaces the `NotImplemented` return).

`PgAuthBackend::complete_oauth(provider, state, code, redirect_uri)` and `InMemoryAuthBackend::complete_oauth(...)` SHALL (🟥 RECON-2 REVISED steps):

1. Atomically consume the row for `(state, provider)` via `self.oauth_state_repo.consume_by_state_and_provider(state, provider.as_str())` — single-statement `UPDATE ... WHERE consumed_at IS NULL AND expires_at > NOW() RETURNING ...` over the Plane A `plane_a_oauth_states` table (NOT `pending_state_store`, which is Plane B). If `None` → `AuthError::InvalidToken` (covers absent, already-consumed, expired, or wrong-provider cases).
2. Verify the `redirect_uri` argument (handler-supplied; the handler derived it from `api_config.public_url` per D-3 — same formula used at `start_oauth` time, ensured by a private helper or `OAuthProviderConfig::derived_redirect_uri(public_url, provider)` accessor consumed by both endpoints) equals the row's `redirect_uri` (from step 1). If they differ → `AuthError::OAuthFailed { cause: "public_url_changed_mid_flow" }` (Scenario 3.10). The operator changed `API_PUBLIC_URL` between `start_oauth` and the callback; the operator must restart the flow.
3. Look up the `OAuthProviderConfig` for `provider` (from `ApiConfig::auth.oauth.providers` — the same config thread used by `start_oauth`). If absent (operator removed it mid-flow) → `AuthError::ProviderNotConfigured`.
4. Resolve IdP endpoints (cached `OidcDiscovery` for `Oidc` providers; `Manual` block as-is) to get `token_url` and `userinfo_url`.
5. Build `flow::TokenExchangeRequest { token_url, client_id, client_secret, code, redirect_uri, code_verifier (from consumed row), auth_style }`.
6. Call `flow::exchange_code(&req)` (per D-12-RECON3) at `crates/api/src/transport/oauth/flow.rs:79-118`. This wraps `oauth_token_http_client()` + form-encoded body + `validate_token_endpoint` (anti-SSRF) + bounded response reading. Returns `serde_json::Value` on 2xx; `Err(String)` on any failure. Map errors to `AuthError::OAuthFailed { cause: "token_endpoint_<reason>" }`. Timeout follows the bounded client's default (30s); no retries (the authorization code is single-use).
7. Parse the JSON response into a local typed shape (or `OAuth2Token` if convenient). Any non-2xx HTTP status, malformed JSON, or missing required fields → `AuthError::OAuthFailed { cause: "token_endpoint_<reason>" }` with the IdP body redacted in the structured log.
8. **🟥 RECON-4 REVISED**: If the provider supplies an `id_token`, the field is logged (`tracing::debug!("id_token present in token response", ...)`) but NOT signature-validated in 1.0 (D-16 defer to 1.1). The presence of `id_token` does NOT affect the rest of the flow.
9. GET userinfo via `provider_config.endpoints.userinfo_url` (or the OIDC-discovered userinfo endpoint) using the same `oauth_token_http_client()` with `Authorization: Bearer <access_token>`. **The userinfo response is the authoritative source for `email` + `sub`.** Failure (non-2xx, malformed JSON, missing `email` or `sub`) → `AuthError::OAuthFailed { cause: "userinfo_<reason>" }`.
10. Apply REQ-oauth-004 / REQ-oauth-005 / REQ-oauth-007 to resolve the local user via the `external_identities` table (D-8).
11. Mint a Nebula session via the same path used by password auth. Return the session.
12. **Per D-13**: the function does NOT route through `Interactive::continue_resolve`. IdP-issued tokens are local variables that Rust's borrow checker drops at function exit. No credential row is created.

**Scenarios**:

- **Scenario 3.1 — Happy path mints a session (🟥 RECON-3 + RECON-4 REVISED)**
  - **Given** `start_oauth` returned `state = "abc"` and persisted the `plane_a_oauth_states` row with `redirect_uri = "https://app.example.com/auth/oauth/google/callback"`
  - **And** the IdP redirects with `?state=abc&code=xyz` to the callback URL
  - **And** `wiremock` (reached via `nebula_api::test_support::*` bypass helpers under `--cfg nebula_test_util` — D-14 wave-5 revision) is configured to return a 200 token response and a userinfo response containing `email = "alice@example.com"` (verified) and `sub = "google-1"`
  - **When** the handler derives the same `redirect_uri` and calls `complete_oauth(OAuthProvider::Google, "abc", "xyz", &redirect_uri)`
  - **Then** the call returns `Ok(Session { ... })`
  - **And** `consume_by_state_and_provider("abc", "google")` returned the row with `consumed_at` now set; a replay would return `None`
  - **And** the `tracing::Span` carries `provider = "google"`, `userinfo_email_hash = <stable-hash>`, and NO raw email, NO raw code, NO state token, NO client secret, NO access/refresh/id token

- **Scenario 3.2 — Replay rejection**
  - **Given** Scenario 3.1 just completed successfully
  - **When** the handler is called a second time with the same `(state, code)`
  - **Then** the call returns `AuthError::InvalidToken`
  - **And** no token endpoint POST is made (state row already consumed)

- **Scenario 3.3 — Expired state rejection**
  - **Given** an `oauth_states` row whose `expires_at` is in the past
  - **When** `complete_oauth` is called with that row's state
  - **Then** the row is consumed (DELETE) and the call returns `AuthError::InvalidToken`

- **Scenario 3.4 — Redirect_uri mismatch rejection**
  - **Given** the `oauth_states` row carries `redirect_uri = "https://a.example.com/cb"`
  - **When** `complete_oauth` is called with `redirect_uri = "https://b.example.com/cb"`
  - **Then** the call returns `AuthError::OAuthFailed` with `cause = "redirect_uri_mismatch"`
  - **And** no token endpoint POST is made

- **Scenario 3.5 — IdP token endpoint 500**
  - **Given** `wiremock` is configured to return HTTP 500 from the token endpoint
  - **When** `complete_oauth` is called
  - **Then** the call returns `AuthError::OAuthFailed` with `cause = "token_endpoint_5xx"`
  - **And** the structured log includes `idp_status = 500` and a redacted body excerpt (max 256 chars, no secrets)

- **Scenario 3.6 — Token response missing `access_token`**
  - **Given** the IdP returns a 200 with body `{"token_type":"Bearer"}` (no `access_token`)
  - **When** `complete_oauth` is called
  - **Then** the call returns `AuthError::OAuthFailed` with `cause = "token_response_malformed"`

- **Scenario 3.7 — 🟥 RECON-4 DELETED (id_token JWKS signature validation deferred to 1.1)**

- **Scenario 3.8 — 🟥 RECON-4 DELETED (id_token nonce match requires signature validation; deferred to 1.1)**

- **Scenario 3.9 — Token endpoint timeout**
  - **Given** `wiremock` is configured to delay the token endpoint response beyond the configured `oauth_token_timeout_ms` (default 5000 ms)
  - **When** `complete_oauth` is called
  - **Then** the call returns `AuthError::OAuthFailed` with `cause = "token_endpoint_timeout"` within `oauth_token_timeout_ms + 500 ms`

- **Scenario 3.10 — `public_url` change mid-flow rejected** (🟥 RECON-4 NEW)
  - **Given** `start_oauth` was called when `API_PUBLIC_URL=https://a.example.com`, persisting `redirect_uri="https://a.example.com/auth/oauth/google/callback"` in `OAuthStateRow`
  - **And** the operator changed `API_PUBLIC_URL=https://b.example.com` and restarted the server
  - **When** the callback arrives and `complete_oauth` derives `redirect_uri="https://b.example.com/auth/oauth/google/callback"`
  - **Then** the row's `redirect_uri` does not match the derived one
  - **And** the call returns `AuthError::OAuthFailed` with `cause = "public_url_changed_mid_flow"`
  - **And** no token endpoint POST is made

---

### REQ-oauth-004 — First-login flow creates a local user

**Status**: ADDED.

When `complete_oauth` validates the IdP response and the userinfo `email` is NOT present in the `users` table (or equivalent) AND the IdP-provided email is marked as `email_verified = true` by the IdP, the backend SHALL:

1. INSERT a new user row with the IdP `email`, `email_verified = true`, no password hash, and link the IdP `sub` into the `external_identities` table (exact table name + schema is a design-phase artifact).
2. Mint a session for the new user.

If the IdP-provided email is NOT marked `email_verified` by the IdP, the backend SHALL return `AuthError::OAuthFailed { cause: "idp_email_unverified" }`. No user row is created.

**Scenarios**:

- **Scenario 4.1 — First login with verified email creates user**
  - **Given** no user exists with `email = "alice@example.com"`
  - **And** IdP userinfo returns `{ email: "alice@example.com", email_verified: true, sub: "google-1" }`
  - **When** `complete_oauth` succeeds
  - **Then** a new user row exists with `email = "alice@example.com"` and `email_verified = true`
  - **And** an `external_identities` row links `(provider = google, sub = google-1) -> user_id`

- **Scenario 4.2 — IdP-unverified email rejects first login**
  - **Given** no user exists with `email = "bob@example.com"`
  - **And** IdP userinfo returns `{ email: "bob@example.com", email_verified: false, sub: "google-2" }`
  - **When** `complete_oauth` is called
  - **Then** the call returns `AuthError::OAuthFailed` with `cause = "idp_email_unverified"`
  - **And** no user row is created

---

### REQ-oauth-005 — Existing-user flow links IdP identity by verified email match

**Status**: ADDED.

When `complete_oauth` validates the IdP response and the userinfo `email` matches an EXISTING user row, the backend SHALL behave per the following truth table:

| Existing user `email_verified` | IdP `email_verified` | Outcome |
|---|---|---|
| `true` | `true` | Link `external_identities`, mint session (Scenario 5.1). |
| `true` | `false` | Reject with `AuthError::OAuthFailed { cause: "idp_email_unverified" }`. |
| `false` | `true` | **Reject** with `AuthError::EmailNotVerified` (Scenario 5.2 — account-takeover defense). |
| `false` | `false` | Reject with `AuthError::EmailNotVerified`. |

**Scenarios**:

- **Scenario 5.1 — Verified-on-verified links and mints session**
  - **Given** a user exists with `email = "alice@example.com"` and `email_verified = true`
  - **And** no `external_identities` row links `(google, google-1)` yet
  - **And** IdP userinfo returns `{ email: "alice@example.com", email_verified: true, sub: "google-1" }`
  - **When** `complete_oauth` succeeds
  - **Then** the call returns `Ok(Session { user_id = <alice's id>, ... })`
  - **And** an `external_identities` row links `(google, google-1) -> alice's user_id`
  - **And** NO duplicate user row is created

- **Scenario 5.2 — Unverified Nebula email rejects OAuth link (account-takeover defense)**
  - **Given** a user exists with `email = "attacker@example.com"` and `email_verified = false`
  - **And** IdP userinfo returns `{ email: "attacker@example.com", email_verified: true, sub: "google-evil" }`
  - **When** `complete_oauth` is called
  - **Then** the call returns `AuthError::EmailNotVerified`
  - **And** no `external_identities` row is created
  - **And** no session is minted
  - **And** the structured log records `cause = "nebula_email_unverified_oauth_link_blocked"`

---

## Spec: `auth-backend` — MODIFIED

### REQ-auth-backend-001 — `AuthBackend::start_oauth` signature accepts `redirect_uri`

**Status**: MODIFIED (breaking change to the trait surface).

**Before**:

```rust
async fn start_oauth(&self, provider: OAuthProvider) -> Result<OAuthStart, AuthError>;
```

**After**:

```rust
async fn start_oauth(
    &self,
    provider: OAuthProvider,
    redirect_uri: &str,
) -> Result<OAuthStart, AuthError>;
```

Every implementor of `AuthBackend` MUST update. Today's implementors:
- `PgAuthBackend` (`crates/api/src/domain/auth/backend/pg.rs`)
- `InMemoryAuthBackend` (`crates/api/src/domain/auth/backend/in_memory.rs`)
- All test-only mocks under `crates/api/tests/`

The handler `crates/api/src/domain/auth/handler.rs::oauth_start` (and `oauth_callback`) SHALL **derive** `redirect_uri` from `ApiConfig::public_url` per D-3 recon-4 — `format!("{}/auth/oauth/{}/callback", api_config.public_url, provider.as_str())` — and pass the derived value to the backend method. The handler does NOT accept `redirect_uri` as a request parameter (query string, form, or otherwise); the operator does not get to override it per-request. The same derivation formula MUST be used at both endpoints (a shared private helper in the handler module or a `OAuthProviderConfig::derived_redirect_uri(public_url, provider)` accessor) so the value round-trips identically through `OAuthStateRow.redirect_uri` and is re-verified on `complete_oauth` (Scenario 3.10).

**Scenarios**:

- **Scenario auth-backend-001.1 — Handler derives `redirect_uri` from `ApiConfig::public_url` (🟥 RECON-4 REVISED)**
  - **Given** `API_PUBLIC_URL = "https://app.example.com"`
  - **And** a request `POST /auth/oauth/google/start` (NO `redirect_uri` query parameter — the handler does not accept one)
  - **When** the handler computes `redirect_uri = format!("{}/auth/oauth/google/callback", api_config.public_url)` and invokes `backend.start_oauth(OAuthProvider::Google, &redirect_uri)`
  - **Then** the resulting `plane_a_oauth_states` row carries `redirect_uri = "https://app.example.com/auth/oauth/google/callback"`
  - **And** the returned `OAuthStart.authorize_url` query string contains `redirect_uri=https%3A%2F%2Fapp.example.com%2Fauth%2Foauth%2Fgoogle%2Fcallback`

- **Scenario auth-backend-001.2 — 🟥 RECON-4 DELETED**
  - The original "handler returns 400 when request-supplied `redirect_uri` is missing" scenario is moot: per D-3 recon-4 the handler does not accept a `redirect_uri` parameter; it derives the value unconditionally from `ApiConfig::public_url`. The fail-closed posture for an unset `public_url` lives at boot time per REQ-compose-001 (rejected before any handler runs).

---

### REQ-auth-backend-002 — `AuthBackend::complete_oauth` no longer returns `NotImplemented`

**Status**: REMOVED-from-production-paths (the `Err(AuthError::NotImplemented(...))` early return is deleted in both impls).

Behavior is fully governed by REQ-oauth-003 through REQ-oauth-005.

**Scenarios**:

- **Scenario auth-backend-002.1 — §4.5 grep is clean after this change**
  - **Given** the worktree at the closing PR of the 5-PR chain (per `tasks.md`; was 6-PR pre-recon-3)
  - **When** the gate runs `rg "NotImplemented" crates/api/src/domain/auth/backend/`
  - **Then** the output contains zero matches that reference OAuth (matches for unrelated features, if any, are allowed)

---

## Spec: `credential-service` — **REMOVED (🟥 RECON-2)**

> The entire `credential-service` spec delta is dropped per `recon-2-credential-domain.md` §3:
> - `CredentialService` is NOT consumed by Flow A (identity login).
> - Operator IdP-client credentials live in `ApiConfig::auth.oauth.providers` as infra config, not credential rows.
> - `OAuth2Credential::initiate_authorization_code` already exists in `nebula-credential` and is reused via D-11.
> - **No new public surface in `nebula-credential-runtime`.** No new methods on `CredentialService`. No new types in that crate.
>
> The original REQ-cred-001 and its three scenarios are deleted. The proposal acceptance criterion A.9 has been reworded; see proposal §5 A.9 RECON-2 block.

---

## Spec: `app-state-composition` — MODIFIED

### REQ-compose-001 — `compose.rs` validates `ApiConfig::auth.oauth.providers` at boot (🟥 RECON-2 REWRITTEN)

**Status**: MODIFIED (rewritten per recon-2).

`apps/server/src/compose.rs` SHALL:

1. If `api_config.auth.oauth.providers` is empty (default): no validation work, boot continues. `AppState::credential_service` stays as it is today (independent of OAuth).
2. If `api_config.auth.oauth.providers` is non-empty: every provider config is validated synchronously at boot:
   - `client_id` non-empty.
   - `client_secret` non-empty (`SecretString`).
   - `ApiConfig::public_url` set AND absolute (with scheme) — required for the auto-derived `redirect_uri` per D-3 recon-4. Empty/relative `public_url` is a boot-time error.
   - For `Oidc { discovery_url }` endpoints: `discovery_url` absolute HTTPS (no localhost; `validate_token_endpoint` policy). Endpoints fetched at runtime via `fetch_oidc_discovery` (D-15); cache is process-wide.
   - For `Manual { authorize_url, token_url, userinfo_url, jwks_url?, scopes }` endpoints: each URL absolute HTTPS; `scopes` non-empty; `jwks_url` accepted but ignored in 1.0 (D-16).
   - Known providers ship as defaults in `crates/api/src/transport/oauth/known.rs` for every `OAuthProvider` enum variant (Google + Microsoft as `Oidc`, GitHub as `Manual` in 1.0); operator config overrides per-provider when present.
3. Any validation failure → `TransportInitError::OAuthProviderConfigInvalid { provider, reason }` and the process exits non-zero. No silent fallback to dev posture.
4. The validated config is threaded into `PgAuthBackend::new` (or the equivalent builder) so the backend reads it per request.
5. The Plane A OAuth state storage seam is `OAuthStateRepo` over the `plane_a_oauth_states` table — already wired into `PgAuthBackend` via `self.oauth_state_repo` (`crates/api/src/domain/auth/backend/pg.rs`). `AppState::pending_state_store` is the **Plane B** (credential-OAuth) seam over `pending_credentials` and is NOT consumed by this change. `CredentialService` is NOT instantiated for OAuth purposes.

**Scenarios**:

- **Scenario compose-001.1 — Boot succeeds with no OAuth declared**
  - **Given** `api_config.auth.oauth.providers` is empty (default)
  - **When** the server boots with any backend
  - **Then** boot succeeds, no OAuth-related validation occurs

- **Scenario compose-001.2 — Boot succeeds with valid OAuth config (🟥 RECON-4 REVISED)**
  - **Given** `api_config.auth.oauth.providers` declares one Google provider with valid `client_id`, `client_secret`, and `endpoints = { kind = "oidc", discovery_url = "https://accounts.google.com/.well-known/openid-configuration" }`
  - **And** `API_PUBLIC_URL = "https://app.example.com"` is set (required for `redirect_uri` auto-derivation)
  - **And** `API_AUTH_BACKEND=postgres` and `DATABASE_URL` reachable
  - **When** the server boots
  - **Then** boot succeeds and the server opens its listening port (no `.well-known/openid-configuration` fetch at boot — lazy per D-15)

- **Scenario compose-001.3 — Boot fails closed on empty `client_secret`**
  - **Given** an OAuth provider config has `client_secret = ""`
  - **When** the server attempts to boot
  - **Then** boot fails with `TransportInitError::OAuthProviderConfigInvalid { provider: "google", reason: "client_secret_required" }`

- **Scenario compose-001.4 — Boot fails closed on HTTP endpoint URL in release build (🟥 RECON-4 REVISED)**
  - **Given** an OAuth provider config has `endpoints = { kind = "manual", authorize_url = "http://github.com/login/oauth/authorize", ... }` (HTTP instead of HTTPS)
  - **And** the binary is built with the `release` feature
  - **When** the server attempts to boot
  - **Then** boot fails with `TransportInitError::OAuthProviderConfigInvalid { provider, reason: "endpoint_url_must_be_https" }`

- **Scenario compose-001.5 — Boot fails closed on `Manual` provider missing required endpoint URL (🟥 RECON-4 REVISED, was "Generic provider")**
  - **Given** a `Manual` OAuth provider config with `userinfo_url` empty (missing)
  - **When** the server attempts to boot
  - **Then** boot fails with `TransportInitError::OAuthProviderConfigInvalid { provider, reason: "manual_userinfo_url_required" }`

> Note: a "Generic" provider variant is **out of 1.0 scope** — the live `OAuthProvider` enum at `crates/api/src/domain/auth/backend/oauth.rs:28-47` has only Google/Microsoft/GitHub. Adding a Generic variant (or `Generic { name: String }` for arbitrary operator-named providers) is a 1.1 enum-extension follow-up per ADR-0085 D-5. Within 1.0, any IdP not in the enum cannot be configured at all (the TOML key fails `FromStr` at config-load), so a "Generic provider config missing endpoints" scenario is unrepresentable.

---

## Spec: `observability` — ADDED

### REQ-obs-001 — Observability triple on OAuth boundaries

**Status**: ADDED.

Per CLAUDE.md "Observability is part of Definition of Done", every new state, error, or hot path in this change MUST carry a typed `thiserror` variant, a `tracing` span or event, and an invariant check.

**Required spans/events**:

| Boundary | Span / Event | Required structured fields |
|---|---|---|
| `start_oauth` entry | `#[instrument]` span | `provider`, `redirect_uri_host` (host-only, never full URL) |
| `start_oauth` row inserted | `tracing::info!` | `provider`, `state_token_prefix` (first 8 chars, redacted) |
| `complete_oauth` entry | `#[instrument]` span | `provider`, `state_token_prefix` (first 8 chars) |
| `complete_oauth` state row consumed | `tracing::debug!` | `provider`, `row_age_ms` |
| Token endpoint POST | `tracing::debug!` | `provider`, `idp_token_url_host`, `attempt_ms` |
| Token endpoint error | `tracing::warn!` | `provider`, `idp_status`, `body_redacted_excerpt` (≤ 256 chars), `cause` |
| `id_token` field presence | `tracing::debug!` | `provider`, `id_token_present` (bool) — 🟥 RECON-4: NO signature validation in 1.0 (D-16) |
| Userinfo fetch | `tracing::debug!` | `provider`, `userinfo_url_host`, `status` |
| User created (first login) | `tracing::info!` | `provider`, `user_id`, `email_hash` |
| User linked (existing) | `tracing::info!` | `provider`, `user_id` |
| Account-takeover block (Scenario 5.2) | `tracing::warn!` | `provider`, `user_id`, `cause = "nebula_email_unverified_oauth_link_blocked"` |

**Forbidden fields** (must never appear in any span/event/log/error from this change):
- `client_secret` (raw or any portion)
- Full `code` (authorization code)
- Full `state_token` (use the 8-char prefix only)
- Full `access_token` / `refresh_token` / `id_token` (none of these are persisted; logs must not capture them either)
- Raw user email (use a stable hash; the `email_hash` field is the agreed redaction)

**Invariant checks**:

```rust
debug_assert!(state_row.expires_at > Utc::now(), "expired state row reached token exchange");
debug_assert!(state_row.provider == provider, "provider mismatch reached token exchange");
debug_assert!(!code_verifier.is_empty(), "empty code_verifier reached token exchange");
```

**Scenarios**:

- **Scenario obs-001.1 — Secrets are never logged**
  - **Given** any of the §3 scenarios runs
  - **When** the test captures `tracing_test::traced_test` output
  - **Then** the captured output contains zero substrings matching: the raw `client_secret`; the full `code`; the full `state_token` (only the 8-char prefix is allowed); any access/refresh/id token

- **Scenario obs-001.2 — Account-takeover defense emits warn-level event**
  - **Given** Scenario 5.2 runs
  - **Then** the captured tracing output includes exactly one `WARN` level event with `cause = "nebula_email_unverified_oauth_link_blocked"`

---

## Spec: `chained-pr-boundary` — ADDED

### REQ-chain-001 — Each PR in the 5-PR chain stays ≤ 800 changed lines (🟥 RECON-3 REVISED — was 6 PRs)

**Status**: ADDED (enforces review workload guard).

Every PR in the chain (PR-1 through PR-5 per `proposal.md` §7; chain compressed from 6→5 PRs per recon-3 §5) SHALL:

1. Have `git diff --shortstat main..HEAD` total ≤ 800 changed lines (added + removed combined).
2. Squash-merge to `main` cleanly with the conventional-commits prefix matching its scope (`docs(adr)`, `refactor(api)`, `feat(api)`, etc.).
3. Pass `task dev:check` locally.
4. Pass the full CI required-jobs matrix.
5. Reference the previous PR in its description and link the spec requirements it covers.

**Scenarios**:

- **Scenario chain-001.1 — Over-budget PR is blocked**
  - **Given** a PR in the chain has `git diff --shortstat main..HEAD` totaling 1,050 lines
  - **When** the worker prepares the PR
  - **Then** the worker MUST stop, surface the over-budget condition to the orchestrator, and split before opening the PR
  - **And** the orchestrator MUST escalate to the user before any split decision (per harness Review Workload Guard)

---

## Coverage matrix (REVISED per recon-2)

| Proposal acceptance criterion | Spec requirement(s) |
|---|---|
| A.1 (real token exchange) | REQ-oauth-003 |
| A.2 (real authorize URL + PKCE) | REQ-oauth-002 |
| A.3 (replay / expiry / mismatch / redirect_uri / IdP error) | REQ-oauth-003 scenarios 3.2–3.6, 3.9 |
| A.4 (provider not configured) | REQ-oauth-001 Invariant 2 + Scenario 1.3 |
| A.5 (first-login creates user) | REQ-oauth-004 |
| A.6 (existing-user links by email) | REQ-oauth-005 |
| A.7 (unverified Nebula email rejects link) | REQ-oauth-005 Scenario 5.2 |
| A.8 (🟥 RECON-2 — compose-root fails closed on invalid OAuth config) | REQ-compose-001 Scenarios compose-001.3 / 001.4 / 001.5 |
| A.9 (🟥 RECON-3 — reuse `mint_pkce` + `flow::build_authorization_uri` + `OAuthStateRepo` (NOT `initiate_authorization_code` + `pending_state_store` — those are Plane B); use `flow::exchange_code` for token endpoint; no new credential-runtime surface) | REQ-oauth-002 + REQ-oauth-003 (the existing surfaces are used, not extended); REQ-cred-001 DELETED |
| A.10 (trait signature update) | REQ-auth-backend-001 |
| A.11 (observability triple) | REQ-obs-001 |
| A.12 (ROADMAP flip + §4.5 grep clean) | REQ-auth-backend-002 |
| A.13 (README OAuth section) | covered by PR-5 scope in revised proposal §7 (no separate spec requirement) |
| A.14 (chained-PR boundary) | REQ-chain-001 |

---

## Next phase

**`sdd-design`** — produce the ADR resolving the open decisions:
- Operator-config convention (A / B / C).
- `AppState::credential_service` shape (dyn-erase vs widen generics vs new repository seam).
- `redirect_uri` shape (single string vs allow-list `Vec<String>`).
- `OAuthProviderCredentialKey` newtype shape (depends on convention).
- `Generic` provider config-row schema.
- `ProviderNotConfigured` vs reuse of `OAuthFailed`.
- `OAuth2Token` discard policy lock (Risk R.4 already locked in proposal; design records the ADR for traceability).

After design, `sdd-tasks` decomposes the **5 PRs** into ordered tasks with strict-TDD anchors (chain was compressed from 6 → 5 by recon-3 §5).

---

## Result envelope

```yaml
status: spec-draft
executive_summary: |
  Spec deltas across 5 specs: oauth-flow (ADDED 5 requirements + 17 scenarios),
  auth-backend (MODIFIED start_oauth signature + REMOVED NotImplemented), credential-service
  (REMOVED per recon-2 — no typed-decode seam), app-state-composition (MODIFIED
  compose root wiring with fail-closed posture), observability (ADDED OAuth boundary
  triple), chained-pr-boundary (ADDED 800-LOC enforcement). 27 BDD scenarios total.
  Coverage matrix maps every proposal A.1–A.14 to a spec REQ. Design ADR open
  decisions enumerated for sdd-design handoff. Strict TDD anchors: 10 RED tests in
  oauth_provider_e2e.rs, 3 in oauth_typed_decode.rs.
artifacts:
  - openspec/changes/oauth-providers-from-operator-secrets/explore.md
  - openspec/changes/oauth-providers-from-operator-secrets/proposal.md
  - openspec/changes/oauth-providers-from-operator-secrets/spec.md
next_recommended: sdd-design
risks:
  - Operator-config convention still open (3 ADR sub-decisions ride on it)
  - Generic provider config-row schema (R.7 in proposal) — design must enumerate
  - `external_identities` table shape — design must define schema
  - Test mode HTTPS-bypass for `localhost` token URLs in cred-001.3 must not leak to prod
skill_resolution: none
```
