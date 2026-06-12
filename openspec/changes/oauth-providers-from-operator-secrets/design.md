# SDD Design — OAuth providers from operator secrets

- **Change slug**: `oauth-providers-from-operator-secrets`
- **Status**: design-draft (revised 2026-05-27 by recon-2, recon-3, recon-4)
- **Date**: 2026-05-27
- **Predecessor artifacts**: `explore.md`, `proposal.md`, `spec.md`.
- **Recon supersedes (recon-2 + recon-3 + recon-4)**:
  - **recon-2** invalidates **D-2, D-4** and **REQ-cred-001** (CredentialService not consumed by Plane A; operator IdP-client config is infra config).
  - **recon-3** invalidates parts of D-9 (`validate_token_endpoint` is non-relaxable for token URLs), D-11 (Plane A uses OAuthStateRepo + mint_pkce, NOT initiate_authorization_code + pending_state_store), D-12 (use `flow::exchange_code` not raw http.rs).
  - **recon-4** invalidates **D-3** (auto-derive `redirect_uri` from `ApiConfig::public_url`; no allow-list), refines **D-5** (tagged union `OAuthEndpoints { Oidc { discovery_url } | Manual { ... } }`), adds D-15 (OIDC discovery doc fetch + process-lifetime cache), D-16 (defer id_token JWKS signature validation to 1.1; userinfo authoritative). User confirmed all three recon-4 ADOPTs + JWKS DEFER.
  - **wave-6** (PR-757 Codex review on `6ad880d4` at 22:51:42Z) closes a **P1 SSRF hole**: `validate_token_endpoint` was only run on the token endpoint URL itself; userinfo GET + OIDC discovery GET + discovery-returned child URLs bypassed it. New **D-9-WAVE6** generalizes the gate to ALL server-side OAuth outbound URLs and renames `validate_token_endpoint` → `validate_oauth_outbound_url`. Plus **D-15-WAVE6** validates discovery-returned URLs before cache insert. Plus Manual variant gains `verified_emails_url: Option<String>` (P2: GitHub's `/user` lacks `email_verified`, requires `/user/emails` lookup).
- **New decisions added across recons**: D-11 (reuse `mint_pkce` + `flow::build_authorization_uri` + `OAuthStateRepo`), D-12 (use `flow::exchange_code`), D-13 (do NOT route Plane A through `Interactive::continue_resolve`), D-14 (`nebula-api` `test_support` module gated by **custom cfg `nebula_test_util`** — NOT a Cargo feature — exposing bypass helpers for all three IdP HTTP call sites; revised in PR-757 wave-5 to fix two Codex P2 findings), D-15 (OIDC discovery fetch+cache), D-16 (JWKS validation deferred to 1.1).
- **Format**: this design document acts as an ADR. Each decision below has Context / Decision / Rationale / Consequences / Alternatives. The final §"Resulting public-surface diff" stitches the decisions into a coherent code surface for `sdd-tasks` to decompose.

---

## D-1 — Operator-config convention

> **🟥 RESHAPED by `recon-2-credential-domain.md` §3 and `recon-4-n8n-and-rust-ecosystem.md` §3 (ADOPT (a)+(b)+(c)).**
> The original Option A (config-map referencing `credential_id` in `CredentialService` + per-row `redirect_uris` allow-list + per-row `scopes`) was superseded:
> - recon-2: operator IdP-client config is **infra config**, not a credential row. Drop `credential_id`; store `client_id`/`client_secret` directly as `SecretString` (matches `SmtpEmailConfig` precedent).
> - recon-4 ADOPT (a): drop `redirect_uris` allow-list. `redirect_uri` is **auto-derived** as `format!("{}/auth/oauth/{}/callback", api_config.public_url, provider.as_str())` from `ApiConfig::public_url` (`API_PUBLIC_URL` env).
> - recon-4 ADOPT (b): replace flat `authorize_url`/`token_url`/`userinfo_url`/`jwks_url`/`scopes` fields with a tagged `OAuthEndpoints` union — `Oidc { discovery_url }` (endpoints fetched from `.well-known/openid-configuration`; scopes hardcoded `"openid email profile"`) vs `Manual { authorize_url, token_url, userinfo_url, jwks_url?, scopes }` (per-provider scopes; `jwks_url` ignored in 1.0 per D-16).
> - recon-4 ADOPT (c): drop the `pkce_required` override knob (PKCE S256 is compile-time mandatory per the `PkceMethod` enum).
>
> The historical Decision/Rationale/Consequences blocks below remain for audit. The live shape lives in **D-3 (recon-4)**, **D-5 (recon-4)**, and the `OAuthProvidersConfig` types described in `tasks.md` T2.1.

### Context (historical — superseded by recon-4)

Three viable shapes (proposal §4): Option A config-map; Option B credential name convention; Option C new `CredentialKind::OAuth2Provider`.

### Decision (historical — superseded by recon-4)

**Option A — Config-map keyed by `OAuthProvider` variant.**

```toml
[auth.oauth.providers.google]
credential_id = "cred_01HX...G7"
scopes        = ["openid", "email", "profile"]
redirect_uris = ["https://app.example.com/cb"]
pkce_required = true                                # optional override; default true

[auth.oauth.providers.generic_corp_sso]
credential_id = "cred_01HX...XX"
authorize_url = "https://sso.corp.example.com/authorize"
token_url     = "https://sso.corp.example.com/token"
userinfo_url  = "https://sso.corp.example.com/userinfo"
jwks_url      = "https://sso.corp.example.com/jwks"
scopes        = ["openid", "email"]
redirect_uris = ["https://app.example.com/cb"]
```

Env-binding mirror: `API_AUTH_OAUTH_GOOGLE_CREDENTIAL_ID`, `API_AUTH_OAUTH_GOOGLE_SCOPES` (comma-separated), `API_AUTH_OAUTH_GOOGLE_REDIRECT_URIS` (comma-separated), etc.

### Rationale (historical — partially superseded)

1. **Smallest blast radius** — still true under the recon-4 reshaping (still no `CredentialService::get_by_name`, no `list_by_kind`).
2. **Reversibility** — still true; Option B/C remain layerable in 1.1.
3. **Operator ergonomics** — reshaped: the config row now co-locates `client_id`/`client_secret` + `endpoints` + (for Manual) `scopes` instead of `credential_id` + `scopes` + `redirect_uris`.
4. **M12.3 sequencing decoupled** — still true.
5. **Matches existing pattern** — still true (`SmtpEmailConfig` is the right precedent now, not `IdempotencyApiConfig`).

### Consequences (historical — superseded by recon-4 / tasks.md T2.1)

- New file: `crates/api/src/config/oauth.rs` defines `OAuthProvidersConfig`, `OAuthProviderConfig`, `OAuthEndpoints` (tagged union), and the `Manual` arm's nested fields. No `OAuthProviderEndpoints` separate type.
- `ApiConfig::auth.oauth` becomes `Option<OAuthProvidersConfig>`; `None` keeps current behavior (no OAuth wired).
- Env parsing follows the `parse_*_env` pattern in `crates/api/src/config/env.rs`. Env vars: `API_AUTH_OAUTH_<PROVIDER>_CLIENT_ID`, `_CLIENT_SECRET`, `_DISCOVERY_URL` (Oidc), `_AUTHORIZE_URL`/`_TOKEN_URL`/`_USERINFO_URL`/`_SCOPES` (Manual).
- Typo risk on the provider key is mitigated by typed deserialization (TOML key MUST parse into the `OAuthProvider` enum at `crates/api/src/domain/auth/backend/oauth.rs:28-47` via `FromStr`).

### Alternatives rejected

- **Option B (name convention)**: extends `CredentialService` public surface; magic-string fragility; scopes/redirect still need a home outside the credential.
- **Option C (`CredentialKind` variant)**: largest blast radius; couples M3.1 to M12.3; requires data migration for existing credentials.

---

## D-2 — `AppState::credential_service` shape

> **🟥 SUPERSEDED by `recon-2-credential-domain.md` §3 (Supersede D-2).**
>
> `CredentialService` is NOT consumed by the M3.1 OAuth login flow. Operator IdP-client credentials are infrastructure config (matching `SmtpEmailConfig` precedent), not credential rows. **No new `CredentialServiceErased` trait. No dyn-erase. `AppState::credential_service` stays as it is today.** The text below is kept for audit; ignore for implementation.
>
> The actual M3.1 wiring is: validate `ApiConfig::auth.oauth.providers` at boot, thread the validated config into `PgAuthBackend::new`. See recon-2 §3 "Rewrite REQ-compose-001" and new decision D-11.

### Context (historical — superseded)

Today: `pub credential_service: Option<Arc<CredentialService<InMemoryStore, InMemoryPendingStore>>>` (concrete generic). PG-backed OAuth needs a different concrete generic at runtime. Three shapes considered:

- **D-2.a**: Widen `AppState` generics (`AppState<CS, ...>`) so the credential-store type ripples through.
- **D-2.b**: Dyn-erase via an object-safe trait (`Arc<dyn CredentialServiceErased>`).
- **D-2.c**: New repository seam (`Arc<dyn OAuthProviderRepository>`) wrapping `CredentialService`, isolating the auth domain from the credential surface.

### Decision

**D-2.b — Dyn-erase via a new `CredentialServiceErased` trait** living in `nebula-credential-runtime`.

```rust
// nebula-credential-runtime
#[async_trait]
pub trait CredentialServiceErased: Send + Sync {
    async fn get(&self, scope: &TenantScope, id: &CredentialId)
        -> Result<CredentialSnapshot, CredentialServiceError>;

    async fn get_for_oauth_provider(&self, scope: &TenantScope, key: &OAuthProviderCredentialKey)
        -> Result<OAuth2Config, CredentialServiceError>;

    // Other CredentialService methods needed by api consumers; minimal surface, additive.
}

impl<B, PS> CredentialServiceErased for CredentialService<B, PS>
where
    B: CredentialStore + Send + Sync + 'static,
    PS: PendingStore + Send + Sync + 'static,
{ /* delegate */ }
```

`AppState` becomes:

```rust
pub credential_service: Option<Arc<dyn CredentialServiceErased>>,
```

### Rationale

1. **Isolation**. Widening `AppState` generics (D-2.a) ripples into every handler signature, every test, every middleware that holds `State<AppState>`. The change touches ~40 files for zero behavioral benefit.
2. **Performance is irrelevant here**. The OAuth callback path is not hot — one HTTP call to the IdP dominates. The vtable dispatch on `dyn CredentialServiceErased::get_for_oauth_provider` adds nanoseconds against milliseconds of network. The cost of dyn-erase is invisible.
3. **Repository seam (D-2.c) is over-engineering**. A new `OAuthProviderRepository` trait that wraps `CredentialService` adds one more layer of indirection without solving anything Option D-2.b doesn't already solve. The "isolate the auth domain from the credential surface" framing is real, but the boundary already exists via the typed-decode seam (REQ-cred-001). A second seam would be redundant.
4. **Matches existing patterns**. The codebase already uses `Arc<dyn EmailPort>`, `Arc<dyn ControlQueueRepo>`, `Arc<dyn AuthBackend>` for analogous slots — dyn-erase is the established convention for ports.

### Consequences

- `CredentialServiceErased` trait is new public surface in `nebula-credential-runtime` (additive, non-breaking for existing concrete-generic consumers).
- `AppState::credential_service` field type changes — breaking the type signature but with zero consumers today (no production code reads `state.credential_service` outside this proposal).
- The `with_credential_service` builder method on `AppState` accepts `Arc<dyn CredentialServiceErased>` instead of the concrete generic.
- Trybuild probes verify the trait is object-safe.

### Alternatives rejected

- **D-2.a (widen generics)**: blast radius across the entire handler tree.
- **D-2.c (repository seam)**: redundant given the typed-decode seam already in REQ-cred-001.

---

## D-3 — `redirect_uri` shape

> **🟥 SUPERSEDED by `recon-4-n8n-and-rust-ecosystem.md` §3 (ADOPT (a)).**
>
> `redirect_uri` is **auto-derived** from the existing `ApiConfig::public_url` field (`API_PUBLIC_URL` env, at `crates/api/src/config/mod.rs:107`). Formula: `format!("{}/api/v1/auth/oauth/{}/callback", api_config.public_url, provider.as_str())` (PR-5 wave-1 correction: the original formula omitted the `/api/v1` prefix; the Plane-A router is nested under `/api/v1` in `crates/api/src/domain/mod.rs:170` so the prefix is mandatory or the IdP redirects to a 404). The `redirect_uris: Vec<String>` config field is dropped from `OAuthProviderConfig`. The allow-list semantics is moot. Multi-environment deployments (dev / staging / prod) each have their own `API_PUBLIC_URL` and their own OAuth client registration at the IdP. Matches n8n's `{instanceBaseUrl}/rest/sso/oidc/callback` pattern.
>
> The text below is kept for audit only; ignore for implementation.

### Context (historical — superseded)

OAuth callbacks need a registered redirect URI. Operators commonly deploy dev / staging / prod environments and want one OAuth client registration covering all.

### Decision

**Allow-list `redirect_uris: Vec<String>` (non-empty, validated at config-load).**

The handler-supplied `redirect_uri` MUST be a member of the allow-list (exact-string match, no wildcards) for `start_oauth` to succeed.

### Rationale

1. **Operator reality**. Most IdPs (Google, Auth0, Okta, generic OIDC) allow multiple redirect URIs per client registration. Forcing one URI per Nebula deployment means three separate OAuth client registrations for dev/staging/prod, each with its own `client_secret`. That's operationally worse.
2. **Security parity**. Exact-string match (no wildcards, no host-suffix match) is the same security posture as RFC 6749 §3.1.2.4 prescribes. The allow-list does not weaken the model — it just lets the operator declare multiple acceptable values.
3. **Validation at config-load**. The allow-list is parsed at server boot. Empty allow-list → boot fails with `OAuthProviderConfigInvalid { reason: "redirect_uris_empty" }`. Non-HTTPS entries (except `http://localhost:*` in test mode) → boot fails. Wildcards in entries → boot fails.

### Consequences

- `OAuthProviderConfig::redirect_uris: Vec<String>` (must be non-empty, each entry HTTPS or `http://localhost:*`).
- `start_oauth(provider, redirect_uri)` validates membership before persisting the state row.
- `complete_oauth` re-validates by checking the row's stored `redirect_uri` matches the call's `redirect_uri` (spec REQ-oauth-003 step 4).

### Alternatives rejected

- **Single `redirect_uri: String`**: blocks the common multi-env case; forces operator into N OAuth client registrations.
- **Host-suffix or wildcard match**: weakens security; rejected by RFC 6749.

---

## D-4 — `OAuthProviderCredentialKey` newtype

> **🟥 SUPERSEDED by `recon-2-credential-domain.md` §3 (Supersede D-4).**
>
> No credential lookup happens in Flow A (identity login). Operator's `client_id` / `client_secret` live directly as `SecretString` fields in `OAuthProviderConfig` (per the `API_SMTP_PASSWORD` precedent in `SmtpEmailConfig`). **No newtype is introduced.** The text below is kept for audit; ignore for implementation.

### Context (historical — superseded)

REQ-cred-001 introduces `CredentialService::get_for_oauth_provider(scope, key)`. The `key` type depends on D-1 (Option A is locked).

### Decision

**`OAuthProviderCredentialKey` is a newtype over `CredentialId`** with a typed constructor that requires going through the `OAuthProvidersConfig` lookup:

```rust
// nebula-credential-runtime
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OAuthProviderCredentialKey(CredentialId);

impl OAuthProviderCredentialKey {
    /// Constructor reserved for code paths that have already validated the
    /// provider exists in the operator config map.
    pub(crate) fn from_validated_id(id: CredentialId) -> Self {
        Self(id)
    }

    pub fn as_credential_id(&self) -> &CredentialId {
        &self.0
    }
}
```

The constructor is `pub(crate)` so callers cannot synthesize a key without going through the validated config map (similar pattern to `ValidatedCredentialBinding` shipped in #732).

A public `OAuthProvidersConfig::credential_key_for(provider) -> Option<OAuthProviderCredentialKey>` is the only legitimate way to obtain a key — the `Option` reflects "provider not configured by operator" and feeds REQ-oauth-001 Invariant 2.

### Rationale

1. **Type-level enforcement**. The `pub(crate)` constructor prevents a confused-deputy pattern where some other code synthesizes a key for an arbitrary credential id and bypasses the operator-config validation. Matches the `ValidatedCredentialBinding` pattern landed in #732.
2. **Encapsulation**. The auth backend never sees raw `CredentialId` for OAuth — only the validated newtype. This makes the API self-documenting at the call site.
3. **Flexibility for future**. If 1.1 layers Option B (name convention) on top, the newtype can carry a `kind: Resolved(CredentialId) | ConventionLookup(Name)` discriminant without breaking Option A consumers.

### Consequences

- New public type in `nebula-credential-runtime`.
- `OAuthProvidersConfig::credential_key_for(provider)` is the only public constructor — keeps the validation invariant local to the config crate.
- `CredentialService::get_for_oauth_provider` consumes `&OAuthProviderCredentialKey` (not `&CredentialId`).

### Alternatives rejected

- **Raw `CredentialId`**: opens the confused-deputy pattern.
- **`(provider, CredentialId)` tuple**: redundant since the config map already binds provider → id; carrying both is double-bookkeeping.

---

## D-5 — `Generic` provider config-row schema

> **🟥 REFINED by `recon-4-n8n-and-rust-ecosystem.md` §3 (ADOPT (b) + (c)).**
>
> `OAuthProviderConfig::endpoints` becomes a **tagged union** instead of a flat struct:
>
> ```rust
> enum OAuthEndpoints {
>     /// OIDC provider — endpoints discovered at runtime from `.well-known/openid-configuration`.
>     /// Scopes are hardcoded `"openid email profile"`.
>     Oidc { discovery_url: String },
>     /// OAuth2-only provider (e.g. GitHub) — explicit endpoints + per-provider scopes.
>     Manual {
>         authorize_url: String,
>         token_url: String,
>         userinfo_url: String,
>         /// Required only for IdPs that issue id_token AND we want JWKS signature validation
>         /// (deferred to 1.1 per D-16). For 1.0, ignored even if supplied.
>         jwks_url: Option<String>,
>         /// Required for Manual; OIDC providers do not need this (scopes hardcoded).
>         scopes: Vec<String>,
>     },
> }
> ```
>
> Known providers (in 1.0: Google + Microsoft) ship as `Oidc` defaults. GitHub ships as `Manual` default. Auth0 / Okta / generic OIDC / custom OAuth2 require extending the `OAuthProvider` enum at `crates/api/src/domain/auth/backend/oauth.rs:28-47` and are tracked as a 1.1 follow-up (see ADR-0085 D-5 "Extending the enum"). The historical text below described the flat struct; ignore for implementation.

### Context (historical — refined)

The `OAuthProvider` enum has known-provider variants (Google, GitHub, Microsoft, …) and a `Generic` variant for arbitrary IdPs. Known-provider variants ship with hardcoded `authorize_url` / `token_url` / `userinfo_url` / `jwks_url` (the IdP's well-known endpoints). `Generic` needs operator-supplied endpoints.

### Decision

**`Generic` requires explicit `endpoints` block; known providers MAY override.**

```toml
[auth.oauth.providers.generic_corp_sso]
credential_id = "cred_01HX...XX"
scopes        = ["openid", "email"]
redirect_uris = ["https://app.example.com/cb"]
endpoints = { authorize_url = "https://sso.corp.example.com/authorize",
              token_url     = "https://sso.corp.example.com/token",
              userinfo_url  = "https://sso.corp.example.com/userinfo",
              jwks_url      = "https://sso.corp.example.com/jwks" }
```

For known providers (e.g. Google), `endpoints` is optional. If absent, the hardcoded defaults from `crates/api/src/transport/oauth/known.rs` (new file) are used. If present, the operator-supplied values OVERRIDE the defaults — allowing operators to point to a private OIDC mirror for testing.

Validation at config-load:
- `Generic` provider MUST carry `endpoints`. Missing → boot fails with `OAuthProviderConfigInvalid { provider: "generic_*", reason: "endpoints_required_for_generic" }`.
- All `endpoints.*_url` MUST be absolute HTTPS (or `http://localhost:*` in test mode).
- `jwks_url` is required ONLY if the IdP issues `id_token` and the test suite needs signature verification (see Risk R.5 below — design records the decision).

### Rationale

1. **Operator flexibility**. Corporate SSO setups vary wildly; hardcoding "Generic" without operator-supplied endpoints is a footgun.
2. **Known-provider override**. Letting operators override known-provider endpoints supports staging environments that point at IdP mirrors / mock IdPs.
3. **Boundary clarity**. The `endpoints` block is its own type — operators see immediately which fields belong to "where do I talk to the IdP" vs. "what is my client identity".

### Consequences

- New `OAuthProviderEndpoints` struct with the four URL fields.
- New file `crates/api/src/transport/oauth/known.rs` defines the hardcoded endpoint maps for each known `OAuthProvider` variant. Each variant gets a `default_endpoints() -> OAuthProviderEndpoints` impl.
- Config-load applies known defaults THEN overrides with operator-supplied values. The merge is shallow (per-field override).
- `Generic` provider config MUST carry the full block; partial `Generic` configs fail at load.

### Alternatives rejected

- **`Generic` only — every provider needs explicit endpoints**: operator burden for the 80% case of "I just want Google".
- **No `endpoints` override for known providers**: blocks IdP mirror use cases (staging, mock testing).

---

## D-6 — Error variant: `ProviderNotConfigured` vs reuse of `OAuthFailed`

### Context

When a caller hits `start_oauth(provider, ...)` and the operator has NOT configured `provider` in the config map, the backend needs to return an error. Two shapes:

- **D-6.a**: Reuse `AuthError::OAuthFailed(String)` with a structured cause like `"provider_not_configured"`. Maps to HTTP 502 `UpstreamError`.
- **D-6.b**: Add `AuthError::ProviderNotConfigured { provider: String }`. Maps to HTTP 503 `ServiceUnavailable` (or 404 NotFound).

### Decision

**D-6.b — Add `AuthError::ProviderNotConfigured { provider: String }` and map to HTTP 503 `ServiceUnavailable`.**

### Rationale

1. **Semantic correctness**. HTTP 502 (`UpstreamError`) implies the IdP failed. But the IdP never got a chance to respond — the operator simply has not provisioned the provider yet. 503 `ServiceUnavailable` is the honest mapping ("this capability is not currently provisioned by the operator").
2. **Discoverability**. A typed variant gives the API caller a structured response shape they can branch on (`provider_unavailable` vs `idp_failed`). Stuffing both into `OAuthFailed(String)` loses that signal.
3. **Observability**. The `AuthError → outcome` mapping audit added in #753 requires exhaustive coverage. Adding the variant keeps the mapping exhaustive and self-documenting.
4. **Documentation**. The OpenAPI 3.1 spec gets a separate response example for `provider_not_configured` distinguishable from `oauth_failed` — clearer integrator UX.

### Consequences

- `AuthError` gets a new variant: `ProviderNotConfigured { provider: String }`.
- `AuthError → ApiError` mapping gets a new arm → `ApiError::ServiceUnavailable`.
- The `AuthError → outcome` exhaustive audit (#753) is extended; one new `outcome = "provider_not_configured"` label value lands in `auth_outcome`.
- The `nebula_api_auth_*` metrics family gets one new closed-set outcome label.

### Alternatives rejected

- **Reuse `OAuthFailed`**: collapses two semantically distinct conditions into one error; loses discoverability for integrators.
- **`NotFound (404)`**: implies the URL is unknown; the URL is mounted, the provider is just not provisioned. 503 is more honest.

---

## D-7 — IdP token discard policy (lock the proposal Risk R.4)

### Context

Proposal R.4 locked the decision: IdP access/refresh tokens are NOT persisted. Design records the ADR-grade rationale for traceability.

### Decision

**IdP-issued `access_token`, `refresh_token`, and `id_token` are discarded after Nebula session is minted. None are persisted in any store, log, or metric.**

### Rationale

1. **Identity vs. delegated access**. OAuth in this change serves identity proof only ("this is Alice, signed by Google"). Nebula does NOT make downstream calls on Alice's behalf using the IdP credentials. Persisting them would expand the security surface without enabling any 1.0 feature.
2. **Session lifecycle independence**. Nebula's session has its own TTL governed by `ApiConfig::auth.session_ttl`. Persisting IdP tokens would invite the question "what if the IdP's `expires_in` < Nebula's session TTL?" — the answer is "we don't care, because we don't use the IdP token after the callback." Making that the explicit contract is cheaper than the alternative.
3. **Defer "OAuth as credential" to 1.1**. The 1.1 surface for using IdP tokens to call downstream APIs (Google Drive, GitHub API, …) lives in a separate change. That change can introduce a `nebula-credential` row of kind `OAuth2Token` populated by the callback. It is out of scope for 1.0.
4. **Reduces logging risk**. With no IdP tokens persisted, the observability rules (REQ-obs-001 forbidden fields) are easier to enforce statically.

### Consequences

- `complete_oauth` consumes `OAuth2Token` for the duration of the function (long enough to validate `id_token` claims if present, fetch userinfo, and find/create the local user) — then drops it. Rust's borrow checker enforces the discard.
- No new persistence migration.
- 1.1 follow-up explicitly carved out: "OAuth-as-downstream-credential" — design TBD then.

### Alternatives rejected

- **Persist as credential row**: expands 1.0 scope beyond identity; introduces rotation, refresh, and revocation concerns; couples M3.1 closure to #688/#690 rotation fan-out (already shipped, but ties OAuth lifecycle to it).
- **Persist in session row**: bypasses the credential-runtime rotation surface; reinvents key management.

---

## D-8 — `external_identities` table shape (spec REQ-oauth-004/005 storage seam)

### Context

REQ-oauth-004 and REQ-oauth-005 mention `external_identities` table linking `(provider, sub) → user_id`. The spec did not freeze the schema — design does.

### Decision

**New migration `0010_external_identities.sql`** (number TBD per actual migration sequence at PR time):

```sql
CREATE TABLE external_identities (
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider    TEXT NOT NULL,        -- snake_case OAuthProvider variant ("google", "github", "generic_<name>")
    subject     TEXT NOT NULL,        -- IdP-side stable subject identifier (sub claim)
    email       TEXT,                 -- snapshot of IdP-side email at link-time (for audit)
    linked_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, subject)
);

CREATE INDEX external_identities_user_id_idx ON external_identities (user_id);
```

`PgExternalIdentityRepo` lives in `crates/storage/src/pg/external_identity.rs` with two methods:

```rust
pub async fn find_user_by_external(
    &self,
    pool: &PgPool,
    provider: &str,
    subject: &str,
) -> Result<Option<UserId>, StorageError>;

pub async fn link_external(
    &self,
    pool: &PgPool,
    user_id: UserId,
    provider: &str,
    subject: &str,
    email: Option<&str>,
) -> Result<(), StorageError>; // returns Duplicate{entity:"external_identity"} on PK conflict
```

### Rationale

1. **Primary key on `(provider, subject)`**. The IdP guarantees `sub` is stable per user inside its tenant; `(provider, sub)` is therefore globally unique in Nebula's view.
2. **`email` is a snapshot**. The IdP email at link-time is recorded for audit but not authoritative — the source of truth for "user's email" stays `users.email`.
3. **`ON DELETE CASCADE`**. If a Nebula user is deleted, their external identities are deleted too. Inverse direction (IdP-side deletion) is handled by Nebula returning a fresh `find_or_create` outcome on next login.
4. **No `tenant_scope` column**. External identity is global per `(provider, sub)` because IdPs do not respect Nebula's tenancy. The user row carries the tenant link via `users.tenant_id` (existing column).

### Consequences

- New migration file under `crates/storage/migrations/postgres/`.
- New repo file `crates/storage/src/pg/external_identity.rs`.
- `PgAuthBackend` gets a new field `external_identities: Arc<PgExternalIdentityRepo>`.
- `InMemoryAuthBackend` gets a parallel `InMemoryExternalIdentityRepo` for symmetry.

### Alternatives rejected

- **Embed in `users.external_id` column**: blocks N:1 (one user multiple IdPs) which is a 1.1 ergonomic ask (link Google + GitHub to same user).
- **`(user_id, provider)` primary key with `subject` unique-constraint**: harder to enforce uniqueness across (provider, sub) at the schema level.

---

## D-9 — Test mode HTTPS bypass (spec Scenario cred-001.3 + D-3 risk)

### Context

Spec Scenario cred-001.3 says HTTP-only URLs are rejected in non-test mode. D-3 says `http://localhost:*` is allowed in test mode for `redirect_uris`. We need a single, auditable "test mode" flag.

### Decision

**`#[cfg(test)]` is NOT used.** Instead, introduce a config-load flag `oauth_allow_insecure_localhost: bool` (default `false`).

- Production builds: flag MUST be `false`. Boot fails with `OAuthProviderConfigInvalid { reason: "insecure_localhost_only_allowed_when_explicitly_enabled" }` if the operator sets it to `true` AND the binary is built with the production feature gate (`feat(release)`).
- Test/dev builds: flag is honored as-is. Tests that need localhost URLs set this flag to `true` in their fixture.
- A `tracing::warn!` event is emitted at boot when the flag is `true`, mirroring the SMTP TLS-disabled warning.

### Rationale

1. **No `#[cfg(test)]` leak to runtime behavior**. The harness rule (AGENTS.md) forbids `#[cfg(test)]` paths that change production behavior. A runtime flag is testable end-to-end via the same code path operators hit.
2. **Auditable posture**. An operator who genuinely wants a localhost-only deployment for in-cluster dev can explicitly opt in. The `warn!` event makes the opt-in visible in observability dashboards.
3. **Matches existing SMTP precedent**. `SmtpTlsMode::None` follows the same pattern (allowed but warns at boot).

### Consequences

- One new config field; one new boot-time warn event.
- `OAuth2Config` URL validation accepts `http://localhost:*` (any port) when the flag is `true`.
- Production builds (feature `release`) reject the flag.

### Alternatives rejected

- **`#[cfg(test)]` only**: harness rule violation.
- **No flag, only HTTPS**: blocks in-cluster localhost dev (some operators run the IdP in the same pod for hermetic testing).

---

## Resulting public-surface diff (REVISED per recon-2)

### New types (additive)

- `crates/api/src/config/oauth.rs`:
  - `OAuthProvidersConfig` — outer map keyed by `OAuthProvider`.
  - `OAuthProviderConfig` — per-provider row holding `client_id: SecretString`, `client_secret: SecretString`, `scopes: Vec<String>`, `redirect_uris: Vec<String>`, optional `endpoints: OAuthProviderEndpoints`, optional `pkce_required: bool` (default `true`).
  - `OAuthProviderEndpoints` — `authorize_url` / `token_url` / `userinfo_url` / `jwks_url`. Required for `Generic` provider; optional override for known providers.
- `crates/api/src/transport/oauth/known.rs` — hardcoded known-provider default endpoints (Google, GitHub, …).
- `crates/api/src/domain/auth/backend/error.rs`:
  - `AuthError::ProviderNotConfigured { provider: String }` (D-6).
- `crates/storage/src/pg/external_identity.rs`:
  - `PgExternalIdentityRepo` (D-8).
- `crates/storage/migrations/postgres/00XX_external_identities.sql`.

> Removed from this list per recon-2: `crates/credential-runtime/src/oauth_key.rs` (no newtype, D-4 superseded); `CredentialServiceErased` trait in `crates/credential-runtime/src/service.rs` (no dyn-erase, D-2 superseded); `CredentialService::get_for_oauth_provider` method (REQ-cred-001 superseded).

### Modified surfaces (breaking)

- `AuthBackend::start_oauth` accepts `redirect_uri: &str` (spec REQ-auth-backend-001).

> Removed from this list per recon-2: `AppState::credential_service` type change. The field stays as it is today; OAuth does not consume it.

### Reused (existing) surfaces — NOT modified

- `OAuth2Credential::initiate_authorization_code(&FieldValues) -> Result<OAuth2Pending, _>` at `crates/credential/src/credentials/oauth2.rs:650` — already generates PKCE verifier + anti-CSRF state + redirect_uri validation. **PR-3 uses this verbatim** (D-11).
- `OAuthStateRepo` over the `plane_a_oauth_states` table — already wired into `PgAuthBackend::self.oauth_state_repo` at `crates/api/src/domain/auth/backend/pg.rs`. PR-3 persists `OAuthStateRow { state, provider, code_verifier, redirect_uri: Some(...), ... }` here. **`AppState::pending_state_store` is Plane B (credential-OAuth) and is NOT consumed by this change.**
- `crates/api/src/transport/oauth/http.rs` — bounded `reqwest::Client` for token endpoint. **PR-4 uses this verbatim** (D-12).
- `crates/api/src/transport/oauth/state.rs` — `OAuthProvider` enum.
- `crates/api/src/transport/oauth/flow.rs` — OAuth flow ceremony (PR-2 worker reads full file before deciding whether to extend or wrap).

### Removed surfaces

- `Err(AuthError::NotImplemented(...))` early return in both `PgAuthBackend::complete_oauth` and `InMemoryAuthBackend::complete_oauth` (spec REQ-auth-backend-002).
- The synthetic `https://nebula.local/...` authorize URL construction.

### Deps added

- `wiremock = "0.6"` under `nebula-api` `[dev-dependencies]` only.
- `reqwest` direct dep of `nebula-api` — but **the existing `transport/oauth/http.rs` already pulls it in**. Verify no new top-level `Cargo.toml` change is needed for PR-4 beyond what's already wired.
- PKCE encoding (`base64` + `sha2`) — verified transitively present via existing crypto; no new top-level deps.

### CODEOWNERS coverage

The change touches:
- `crates/api/**` → `@vanyastaff` (auth domain).
- `crates/credential-runtime/**` → likely `@vanyastaff` (single-owner repo today).
- `crates/storage/**` → `@vanyastaff`.
- `apps/server/**` → `@vanyastaff`.

No cross-org review routing needed.

---

## D-15 — OIDC discovery doc fetch + process-lifetime cache

### Context

ADOPT (b) from recon-4: OIDC providers ship as `Oidc { discovery_url }` and Nebula fetches `.well-known/openid-configuration` at runtime to learn `authorize_url`, `token_url`, `userinfo_url`, `jwks_url`. This adds a network dependency at first OAuth-start call per provider.

### Decision

`crates/api/src/transport/oauth/discovery.rs` (new file) exposes:

```rust
pub struct OidcDiscovery {
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub jwks_url: Option<String>,
}

static DISCOVERY_CACHE: OnceLock<DashMap<String, OidcDiscovery>> = OnceLock::new();

pub async fn fetch_oidc_discovery(url: &str) -> Result<OidcDiscovery, DiscoveryError>;
```

The cache key is the `discovery_url` itself. Cache lifetime is process-wide (no TTL; discovery docs are stable per provider; restart Nebula to refresh). Anti-SSRF: `discovery_url` MUST pass `flow::validate_token_endpoint` (same HTTPS + non-localhost policy). HTTP via `oauth_token_http_client()`.

Typos and 404s on the discovery URL surface at `start_oauth` time as `AuthError::OAuthFailed { cause: "oidc_discovery_failed" }`, NOT at boot. This is acceptable because the operator might add a provider while the server is running (env-managed mode triggers a SIGHUP-style reload — outside this change's scope; for 1.0 the operator restarts).

### Consequences

- New file ~80 LOC.
- One additional HTTP round-trip on the FIRST OAuth-start call per provider per process lifetime.
- `OAuthEndpoints::Oidc` carries `discovery_url`; runtime resolves to a concrete `OidcDiscovery` via the cache.

## D-16 — Defer id_token JWKS signature validation to 1.1

### Context

ADOPT-DEFER from recon-4: `jsonwebtoken` crate is already in workspace deps with `rust_crypto` feature, so JWKS verification is *technically* ~80 LOC. But proper handling (fetch JWKS, cache with key rotation, handle multiple keys, validate `iss`/`aud`/`exp`/`iat`/`nonce`) is ~200 LOC + HTTP discipline. For 1.0:

### Decision

**The userinfo endpoint is authoritative** for `email` + `sub`. `complete_oauth` does NOT validate the `id_token` signature in 1.0. The presence of `id_token` in the IdP's token response is ignored beyond logging.

The security argument:
- Token endpoint is over TLS, validated by `validate_token_endpoint`.
- Userinfo endpoint is over TLS using the same HTTP client policy.
- A compromise of TLS to the IdP would also compromise id_token signature validation (since JWKS is fetched over TLS too).
- The `state` (anti-CSRF) + PKCE `code_verifier` defenses are still intact — these are pre-token-endpoint defenses.
- The marginal risk from skipping id_token signature is: a malicious IdP could swap the userinfo response. But the operator chose the IdP; if they don't trust their IdP's TLS chain, they have bigger problems.

1.1 ships the `openidconnect` crate (or hand-rolled JWKS via `jsonwebtoken`) and adds signature + claims validation as a hardening step.

### Release-notes blurb (required in PR-5)

> **OAuth identity login (1.0)**: Nebula ships authorization-code with PKCE for the three OAuth providers in the live `OAuthProvider` enum: Google + Microsoft (`Oidc` discovery-based) and GitHub (`Manual` explicit endpoints). The IdP's userinfo endpoint is the authoritative source for the user's verified email and stable subject identifier. `id_token` signature validation against the IdP's JWKS is **not** performed in 1.0 — a 1.1 hardening pass will add it. Adding Auth0 / Okta / generic OIDC / custom OAuth2 providers requires extending the enum (small mechanical change, 1.1 follow-up). Operators that require strict OIDC compliance now should track issue #TBD.

### Consequences

- PR-4 saves ~100 LOC + the JWKS HTTP discipline complexity.
- Spec REQ-oauth-003 steps 7-8 deleted; Scenarios 3.7 (id_token signature invalid) and 3.8 (id_token nonce mismatch) deleted.
- `OAuthEndpoints::Manual::jwks_url` becomes `Option<String>` accepted for forward compat but ignored in 1.0.
- One new risk R-D7 documented in the proposal.

## D-11 — Reuse `mint_pkce` + `flow::build_authorization_uri` + `OAuthStateRepo` (recon-3 / recon-4)

> **🟥 ORIGINAL D-11 SUPERSEDED by `recon-3-flow-and-pending.md` §6.**
> The original D-11 said to use `OAuth2Credential::initiate_authorization_code` + persist `OAuth2Pending` via `AppState::pending_state_store`. **Wrong** — those are Plane B (credential-OAuth) surfaces; this change is Plane A (identity login). Plane A has its own dedicated store: `OAuthStateRepo` over the `plane_a_oauth_states` PG table, already wired into `PgAuthBackend` (`self.oauth_state_repo`).

### Context (recon-3 grounded)

- `mint_pkce()` at `crates/api/src/domain/auth/backend/oauth.rs:90+` already generates `state` + `code_verifier` + `code_challenge` (SHA-256 / base64url-no-pad).
- `flow::build_authorization_uri(req, state, code_challenge)` at `crates/api/src/transport/oauth/flow.rs:38-60` already builds an RFC-compliant authorize URL with `code_challenge_method=S256` + optional scope. Tested at `flow.rs:188+`.
- `PgAuthBackend::start_oauth` at `pg.rs:1028-1075` already calls `mint_pkce()` and persists an `OAuthStateRow` via `self.oauth_state_repo.create(...)`. The two gaps are: synthetic `https://nebula.local/...` authorize URL (no `flow::build_authorization_uri` call), and `redirect_uri: None` (the trait signature lacks the param).
- The `OAuthStateRepo` API supports atomic single-statement consume-by-state-and-provider (`UPDATE ... WHERE consumed_at IS NULL AND expires_at > NOW() RETURNING ...`).

### Decision

`PgAuthBackend::start_oauth(provider, redirect_uri)` and `InMemoryAuthBackend::start_oauth(...)` SHALL:

1. Look up the validated `OAuthProviderConfig` from `ApiConfig::auth.oauth.providers[provider]`. If absent → `AuthError::ProviderNotConfigured { provider }`.
2. Derive the canonical `redirect_uri` from `api_config.public_url` per D-3 (`format!("{}/api/v1/auth/oauth/{}/callback", public_url, provider.as_str())` — PR-5 wave-1 corrected). The handler-supplied `redirect_uri` arg MUST already equal this value (the handler derived it the same way before calling `start_oauth`); a mismatch is a debug-assert at the trait boundary.
3. For `Oidc { discovery_url }`: call `fetch_oidc_discovery(discovery_url)` (D-15) to obtain `authorize_url`/`token_url`/`userinfo_url`. For `Manual { authorize_url, ... }`: use the operator-supplied URLs directly. Construct `flow::AuthorizationUriRequest` (client_id, token_url, client_secret, redirect_uri, scopes, auth_style).
4. Call `mint_pkce()` to get `(state, code_verifier, code_challenge)`.
5. Call `flow::build_authorization_uri(&req, &state, &code_challenge)` to construct the real authorize URL.
6. Persist `OAuthStateRow { state, provider, code_verifier, redirect_uri: Some(redirect_uri.to_owned()), created_at, expires_at, consumed_at: None }` via `self.oauth_state_repo.create(...)`. TTL = `OAUTH_STATE_TTL` (10 min).
7. Return `OAuthStart { authorize_url, state }`.

### Rationale

1. **Plane-A storage seam is `OAuthStateRepo`, not `pending_state_store`.** `plane_a_oauth_states` is the dedicated PG table; `pending_credentials` is Plane B.
2. **`OAuth2Credential::initiate_authorization_code` is not reachable from Plane A.** It produces `OAuth2Pending` (Plane B shape with `client_secret: SecretString`, `grant_type`, `device_code`, `interval`, etc.) and is persisted via `PendingStateStore`. The Plane-A flow needs only `(state, provider, code_verifier, redirect_uri)` — a strict subset.
3. **Zero duplication**. `mint_pkce` + `flow::build_authorization_uri` + `OAuthStateRepo` already exist, tested, and wired into `PgAuthBackend`. PR-3 only replaces the synthetic URL construction + plumbs the `redirect_uri` parameter through the existing skeleton.

### Consequences

- No new PKCE / CSRF code in `nebula-api`.
- No new pending-state surface; `AppState::pending_state_store` (Plane B slot) is **not** consumed by Plane A.
- PR-3 of the 5-PR chain becomes "plumb `flow::build_authorization_uri` into `start_oauth` + populate `OAuthStateRow.redirect_uri`" — much smaller than even the recon-2 estimate.

## D-12 — Reuse `crates/api/src/transport/oauth/http.rs`

### Context

Recon-2 revealed `crates/api/src/transport/oauth/http.rs` already exposes a bounded `reqwest::Client` configured with the API's OAuth-flow policy (timeout, TLS, header sanitization). It was moved from `nebula-credential` per the established "API-owned OAuth flow" architecture decision.

### Decision

`PgAuthBackend::complete_oauth(...)` SHALL use the shared `reqwest::Client` from `transport/oauth/http.rs` for both the token endpoint POST and the userinfo GET. PR-4 does NOT instantiate its own HTTP client.

The `oauth_token_timeout_ms` config field (5000 ms default) is read by the http.rs client builder — PR-3 wires the value through.

### Rationale

1. **Zero duplication**. The OAuth HTTP transport already exists and is policy-configured per the canonical architecture decision.
2. **Audit reach**. A single HTTP client surface means a single point of audit for TLS posture, header sanitization, and observability (`#[instrument]` spans live there).

### Consequences

- PR-4 imports from `crates/api/src/transport/oauth/http.rs` directly; no new HTTP code.
- The OAuth HTTP client may need a small extension (a `get_userinfo(url, access_token)` helper if not already there) — PR-4 worker checks and either uses the existing surface or adds one method.

## D-13 — Do NOT route Flow A through `Interactive::continue_resolve`

### Context

`OAuth2Credential` implements `Interactive::continue_resolve(token: PendingToken, user_input: UserInput) -> ResolveResult<OAuth2State, ()>`. The natural-looking move is to call this from `complete_oauth` to validate the IdP response.

But `continue_resolve` is designed to **persist** the resolved `OAuth2State` as an encrypted credential row (via `CredentialService` → `CredentialStore` → `EncryptionLayer`). That contradicts D-7 (do NOT persist IdP tokens for Flow A).

### Decision

`PgAuthBackend::complete_oauth` performs the token exchange + userinfo fetch **without** invoking `Interactive::continue_resolve`. Per D-16 (recon-4), id_token JWKS signature validation is **deferred to 1.1**; the function only logs `id_token` presence and treats the userinfo endpoint as authoritative. The function:
1. Loads the `OAuthStateRow` (NOT `OAuth2Pending` — that's Plane B) via `self.oauth_state_repo.consume_by_state_and_provider(state, provider.as_str())` — single-statement atomic UPDATE-RETURNING on `plane_a_oauth_states` (covers absent, already-consumed, expired, and wrong-provider in one query).
2. POSTs to the token endpoint via `transport/oauth/http.rs`.
3. Parses `OAuth2Token` (or a private equivalent) from the response.
4. **🟥 RECON-4 / D-16 REVISED**: If `id_token` is present in the token response, log its presence (`tracing::debug!(provider, id_token_present = true)`) but **do NOT validate the signature against JWKS in 1.0** — D-16 defers JWKS signature validation to 1.1. The presence of `id_token` does NOT change subsequent flow steps; userinfo (next step) is authoritative.
5. GETs userinfo via the same HTTP client.
6. Applies REQ-oauth-004 / -005 / -007 user resolution.
7. Mints session via the existing session pipeline.
8. Drops `OAuth2Token` (and any intermediate access/refresh tokens) at the end of the function. Borrow checker enforces no leak.

No credential row is created. The PG `external_identities` row links the IdP `sub` to the Nebula user (D-8).

### Rationale

1. **D-7 enforcement at the type level**. Bypassing `continue_resolve` means no `OAuth2State` is ever constructed, encrypted, or persisted. The discard happens because there is no persistence path, not because we remember to call `drop()`.
2. **Flow A / Flow B separation**. Flow B (credential-as-OAuth) is the right consumer of `continue_resolve`. Routing Flow A through it conflates identity proof with delegated-access storage.
3. **Smaller blast radius**. `continue_resolve` invokes `CredentialService` machinery (encryption layer, audit layer, cache layer) that Flow A does not need. Bypassing skips that entire stack at runtime.

### Consequences

- PR-4 implements the token-exchange + userinfo + JWKS validation logic directly in the auth backend. Estimated ~200 LOC of net-new code on top of the existing transport/oauth/http.rs.
- A future change that wants "OAuth-as-stored-credential" for Flow B will use `continue_resolve` independently — both paths can coexist without conflict.

### Alternatives rejected

- **Route through `continue_resolve` then immediately delete the credential row**: a credential-create-then-delete round trip is a security and performance regression vs. simply not creating the row.
- **Add a `Credential::resolve_no_persist` variant**: bloats the credential trait surface; the right architectural answer is that Flow A is just not a credential operation.

---

## D-10 — Strict TDD orchestration

### Context

`openspec/config.yaml` declares strict TDD active. The 5-PR chain (compressed from 6 per recon-3 §5) must reflect RED → GREEN → TRIANGULATE → REFACTOR evidence per PR.

### Decision

Per-PR TDD anchors:

| PR | RED tests committed first | GREEN scope | TRIANGULATE | REFACTOR |
|---|---|---|---|---|
| 1 (ADR) | n/a (markdown only) | n/a | n/a | n/a |
| 2 (trait + config + compose validation + test_support module) | `auth_handler_derives_redirect_uri_from_public_url_for_both_start_and_complete`, `compose_root_fails_closed_when_public_url_unset_with_oauth_declared`, `oauth_provider_config_validates_oidc_requires_https_discovery_url`, `oauth_provider_config_validates_manual_requires_non_empty_scopes`, `oauth_provider_config_rejects_http_endpoint_in_release_mode` | trait sig extension on BOTH `start_oauth(provider, redirect_uri)` AND `complete_oauth(provider, state, code, redirect_uri)` + handler derives redirect_uri from `ApiConfig::public_url` for both endpoints + config types + env binding + compose-root validation + `crates/api/src/test_support.rs` gated by `#[cfg(nebula_test_util)]` exposing bypass helpers for token POST / userinfo GET / OIDC discovery GET + `not(debug_assertions)` compile_error guard against release-with-cfg | env-binding round-trip; Manual missing scopes rejected; provider key typo (`gooogle`) fails at config-load; CI gate proves `RUSTFLAGS="--cfg nebula_test_util" cargo build --release` fails | inline cleanup |
| 3 (authorize URL via flow::build_authorization_uri) | `start_oauth_emits_real_authorize_url_with_pkce_and_state`, `start_oauth_persists_redirect_uri_into_oauth_state_row`, `start_oauth_returns_provider_not_configured_when_absent` | both backends call `mint_pkce` + `flow::build_authorization_uri(req, state, code_challenge)`; persist `OAuthStateRow` (with `redirect_uri: Some(...)`) via existing `OAuthStateRepo`; metrics emitted | URL-encode invariants; PKCE method=S256 in query string; OIDC discovery cache hit on second call | inline cleanup |
| 4 (token exchange + external_identities + find-or-create) | `complete_oauth_succeeds_with_valid_code_oidc_provider_first_login`, `complete_oauth_short_circuits_via_external_identity_on_repeated_login` (🟥 wave-7 REQ-oauth-006), `complete_oauth_external_identity_link_survives_idp_email_change` (🟥 wave-7 REQ-oauth-006 5.2), `complete_oauth_succeeds_with_valid_code_manual_provider_github`, `complete_oauth_rejects_replay`, `complete_oauth_rejects_expired_pending_row`, `complete_oauth_rejects_mismatched_state_token`, `complete_oauth_rejects_mismatched_provider`, `complete_oauth_rejects_public_url_changed_mid_flow` (Scenario 3.10), `complete_oauth_handles_idp_token_endpoint_500_with_redacted_log`, `complete_oauth_rejects_malformed_token_response_missing_access_token`, `complete_oauth_creates_user_on_first_login_with_idp_verified_email`, `complete_oauth_creates_user_on_first_login_with_github_verified_email_via_second_endpoint` (🟥 wave-6), `complete_oauth_rejects_first_login_when_idp_email_unverified`, `complete_oauth_links_existing_user_when_emails_both_verified`, `complete_oauth_rejects_link_when_nebula_email_unverified_idp_verified` (account-takeover defense), `complete_oauth_rejects_github_userinfo_when_verified_emails_url_returns_internal_ip` (🟥 wave-6 SSRF defense) | full complete_oauth implementation in both backends — `oauth_state_repo.consume_by_state_and_provider` (Plane A); verify row's redirect_uri matches handler-supplied derived value; `flow::exchange_code` for token POST; userinfo GET via `oauth_token_http_client()` with Bearer header; **log id_token presence only — NO JWKS signature validation per D-16 (1.1 follow-up)**; **REQ-oauth-006 short-circuit via `find_user_by_external` before email truth-table** (🟥 wave-7); find-or-create user via `external_identities` (BYTEA user_id per ADR-0085 D-8) + truth table (REQ-oauth-004/005); for Manual `verified_emails_url: Some(_)` providers (GitHub) fetch second endpoint and pick `primary && verified`; mint session; drop tokens; PG migration + repo | wiremock chaos: 200 with empty body, 200 with garbage JSON; userinfo-only flow (no id_token); CASCADE delete invariant on user-row delete | inline cleanup |
| 5 (tests + docs + roadmap) | n/a (prior PRs hold tests; PR-5 adds README doctest snippets) | docs prose + README OAuth section + ROADMAP checkbox | section-by-section README doctest | n/a |

### Rationale

(🟥 WAVE-7/WAVE-8 REVISED) PR-4 carries the largest test burden (**17 RED tests** after wave-6/-7 additions) because it implements all of REQ-oauth-003 / -004 / -005 / **-006** (🟥 wave-7 added; -007 was an orphan reference, fixed) against the wiremock IdP plus the `external_identities` table migration. PR-3 carries 5 RED tests covering the authorize-URL emission and OIDC-discovery cache. PR-2 carries 5 RED tests covering the trait signature change, two-validator config validation (strict + flag-aware per F.2 wave-7), and compose-root fail-closed posture. Splitting the test load this way keeps each PR independently reviewable under the 800-LOC budget.

### Consequences

- `sdd-tasks` decomposes each PR's RED tests into individual tasks, each tagged `RED:`, `GREEN:`, `TRIANGULATE:`, `REFACTOR:`.
- The worker for each PR commits RED tests in the FIRST commit of that PR; reviewer verifies the RED test commit is failing on its own before subsequent GREEN commits.

---

## Risks recap (REVISED per recon-2)

| Risk | Status |
|---|---|
| R.1 — `AppState::credential_service` generic shape | **DROPPED** — not an OAuth concern; recon-2 §3 |
| R.2 — `wiremock` dev-dep addition | already shipped — in `crates/api/Cargo.toml` dev-deps |
| R.3 — PKCE for confidential clients | mitigated; PKCE is compile-time S256-only |
| R.4 — IdP token persistence | re-locked by D-7 + D-13 (Flow A cannot persist by construction) |
| R.5 — Session/IdP token lifetime | unchanged |
| R.6 — `redirect_uri` shape | **DROPPED** by recon-4 ADOPT (a) — auto-derived from `public_url`, allow-list moot |
| R.7 — `Generic` provider config-row schema | closed by D-5 (refined to tagged union per recon-4) |
| R.8 — CSRF cookie binding on callback | unchanged |
| R.9 — Token-endpoint timeout / no retries | unchanged |
| R.10 — Subagent dispatch reliability | unchanged |
| **R-D7 (NEW)** — 1.0 ships without id_token JWKS signature validation | **mitigated** by D-16 release-notes blurb + 1.1 follow-up issue; userinfo over TLS is authoritative |

> Removed: **R-D1** (newtype constructor leak — newtype is gone), **R-D2** (CredentialServiceErased object-safety — trait is gone).

Retained / re-stated risks introduced by design decisions:

- **R-D3** — `oauth_allow_insecure_localhost` flag prod leak. Mitigation: D-9's release-feature gate rejects the flag.

New risks introduced by recon-2:

- **R-D4** — PR-3 worker MUST verify `crates/storage/src/pg/pending_state.rs` (or wherever the PG-backed `PendingStateStore` lives) before writing RED tests for `start_oauth_persists_pending_with_redirect_uri`. The existing `PendingStateStore` may have a different signature than what `OAuth2Credential::initiate_authorization_code` returns. Mitigation: PR-3 first task is the read; if signature mismatch, file a small adapter task before the RED test.
- **R-D5** — `crates/api/src/transport/oauth/flow.rs` may already implement Flow B's ceremony in a way that constrains Flow A. PR-2 worker reads full file before writing the trait change. Mitigation: PR-2 first task is the read; if Flow B's ceremony imposes shape constraints, design records the constraint as a new D-* decision before PR-2 writes code.

---

## Next phase

**`sdd-tasks`** — decompose this design + spec into an ordered task list:
- One task per RED test commit (per D-10).
- One task per GREEN implementation chunk.
- One task per TRIANGULATE pass.
- One task per REFACTOR pass.
- One task per PR-boundary handoff (squash-merge to `main`).
- Strict TDD evidence anchors per task.
- Estimated changed LOC per task to verify the 800-LOC PR ceiling holds.

---

## Result envelope (REVISED per recon-2 + recon-3 + recon-4)

```yaml
status: design-draft
executive_summary: |
  13 live design decisions after three recon waves: D-1 Option A reshaped (env-managed
  config-map with client_id/client_secret in ApiConfig, no credential_id); D-3 (recon-4)
  redirect_uri auto-derived from public_url, no allow-list; D-5 (recon-4) OAuthEndpoints
  tagged union Oidc { discovery_url } vs Manual { authorize_url, token_url, userinfo_url,
  jwks_url?, scopes }; D-6 new ProviderNotConfigured variant → HTTP 503; D-7 IdP tokens
  discarded after session mint; D-8 external_identities table BYTEA user_id matching
  existing 0001_users.sql convention; D-9 (recon-3) oauth_allow_insecure_localhost scope
  narrowed (token_url stays strict); D-11 (recon-3) reuse mint_pkce + flow::build_authorization_uri
  + OAuthStateRepo for Plane A (NOT initiate_authorization_code + pending_state_store);
  D-12 (recon-3) use flow::exchange_code; D-13 do NOT route Plane A through
  Interactive::continue_resolve; D-14 nebula-api test_support module gated by custom cfg `nebula_test_util` (NOT a Cargo feature — wave-5 revision); D-15 OIDC discovery
  cache; D-16 JWKS id_token validation deferred to 1.1. Superseded: D-2 dyn-erase
  (recon-2), D-4 OAuthProviderCredentialKey (recon-2), D-3 original allow-list (recon-4),
  D-11/D-12 original phrasings (recon-3). 5-PR chain (was 6) per recon-3 §5.
artifacts:
  - openspec/changes/oauth-providers-from-operator-secrets/explore.md
  - openspec/changes/oauth-providers-from-operator-secrets/proposal.md
  - openspec/changes/oauth-providers-from-operator-secrets/spec.md
  - openspec/changes/oauth-providers-from-operator-secrets/design.md
  - openspec/changes/oauth-providers-from-operator-secrets/recon-{2,3,4}-*.md
  - openspec/changes/oauth-providers-from-operator-secrets/tasks.md
next_recommended: sdd-tasks (already produced; sdd-apply PR-1 is this ADR)
risks:
  - R-D3 oauth_allow_insecure_localhost prod leak (mitigated by release feature gate)
  - R-D6 nebula-api test_support module gated by `--cfg nebula_test_util` must not leak into release builds (mitigated by `cfg(all(nebula_test_util, not(debug_assertions))) compile_error!` + CI negative-test job per wave-5 revision)
  - R-D7 1.0 ships without id_token JWKS signature validation; userinfo authoritative
skill_resolution: none
```
