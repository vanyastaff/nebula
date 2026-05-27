# ADR-0085 — OAuth identity providers from operator secrets

- **Status:** Accepted
- **Date:** 2026-05-27
- **Supersedes:** N/A
- **Superseded by:** N/A
- **Related:** ROADMAP §M3.1; ADR-0081 (m6-resource-credential-integration); ADR-0084 (pre-expiry credential refresh deferred); `openspec/changes/oauth-providers-from-operator-secrets/`

## Context

ROADMAP §M3.1 closure left one residual canon §4.5 honesty gap: the OAuth
identity-login path (`POST /auth/oauth/{provider}/start`,
`GET /auth/oauth/{provider}/callback`) is mounted, advertised in OpenAPI 3.1,
and partially wired through `PgAuthBackend`/`InMemoryAuthBackend`, but
`complete_oauth` at `crates/api/src/domain/auth/backend/pg.rs:1079-1110`
explicitly returns `Err(AuthError::NotImplemented(...))`. The synthetic
`https://nebula.local/...` authorize URL emitted by `start_oauth` is
likewise placeholder.

This ADR resolves the design questions raised by closing that gap. The
full SDD planning trail (explore → proposal → spec → design → tasks plus
three recon waves) lives under
`openspec/changes/oauth-providers-from-operator-secrets/`. The directory
is part of this PR series so the audit chain is reachable from git
history.

**Scope distinction.** Nebula has two non-overlapping OAuth surfaces:

- **Plane A — identity login** (this ADR): operator-supplied OAuth client
  proves "this is Alice" via Google/GitHub/Microsoft/Generic-OIDC, then
  Nebula mints its own session and discards IdP tokens.
- **Plane B — credential OAuth** (out of scope; 1.1): user-stored OAuth
  credential lets a workflow node call Google Drive / GitHub API / etc.
  on the user's behalf. Owned by `nebula-credential` + `Interactive::continue_resolve`.

This ADR governs Plane A only.

## Decision

The 13 decisions below resolve the open questions in the SDD design
phase. Each decision identifier matches its label in
`openspec/changes/oauth-providers-from-operator-secrets/design.md` and
its recon supersede chain (recon-2 / recon-3 / recon-4).

### D-1 — Operator config lives in `ApiConfig::auth.oauth.providers`

Operator IdP-client credentials (`client_id`, `client_secret`, endpoints,
scopes) are **infrastructure config**, not credential rows. They live in
`ApiConfig::auth.oauth.providers` next to `ApiConfig::smtp` and
`ApiConfig::idempotency`, populated from `API_AUTH_OAUTH_<PROVIDER>_*`
env vars. This matches the SMTP precedent (`SmtpEmailConfig` +
`API_SMTP_*`).

**Rejected alternatives:**

- **Name-convention credential lookup** (`CredentialService::get_by_name("oauth2/<provider>")`):
  extends `CredentialService` public surface; magic-string fragility;
  scopes/redirect still need a separate home.
- **New `CredentialKind::OAuth2Provider` variant:** largest blast
  radius; couples M3.1 closure to M12.3 sequencing; requires data
  migration for existing credentials.

Env-managed mode is the only supported config path in 1.0. DB-managed
admin UI (matching n8n's `N8N_SSO_MANAGED_BY_ENV=false` mode) is a 1.1+
extension; Nebula has no admin UI for SSO config yet.

### D-3 (recon-4 revised) — `redirect_uri` auto-derived from `ApiConfig::public_url`

`redirect_uri` is **NOT a configuration field**. It is auto-derived at
runtime as
`format!("{}/auth/oauth/{}/callback", api_config.public_url, provider.as_str())`
from the existing `ApiConfig::public_url` (`API_PUBLIC_URL` env, declared
at `crates/api/src/config/mod.rs:107`).

Operators that need multiple callback URIs deploy multiple Nebula
instances (each with its own `API_PUBLIC_URL` and IdP client
registration). Matches n8n's `{instanceBaseUrl}/rest/sso/oidc/callback`
pattern.

**Supersedes the prior design's allow-list `redirect_uris: Vec<String>`
shape.** No redirect-URI allow-list. The original Risk R.6 (single vs.
allow-list) is moot.

### D-5 (recon-4 refined) — `OAuthEndpoints` is a tagged union

```rust
enum OAuthEndpoints {
    /// OIDC provider — endpoints discovered at runtime from
    /// `.well-known/openid-configuration`. Scopes hardcoded
    /// `"openid email profile"`.
    Oidc { discovery_url: String },
    /// OAuth2-only provider (e.g. GitHub) or operator-customized OIDC.
    /// `jwks_url` accepted for forward compat but ignored in 1.0 (see D-16).
    /// `scopes` MUST be non-empty for Manual.
    Manual {
        authorize_url: String,
        token_url: String,
        userinfo_url: String,
        jwks_url: Option<String>,
        scopes: Vec<String>,
    },
}
```

Known providers (Google, Microsoft, Auth0, Okta) ship as `Oidc` defaults
in `crates/api/src/transport/oauth/known.rs`. GitHub ships as `Manual`
default (it does not expose `.well-known/openid-configuration`).
Operator config overrides per-provider.

### D-6 — `AuthError::ProviderNotConfigured { provider }` → HTTP 503

A new typed variant for "operator has not configured this provider".
Maps to HTTP 503 `ServiceUnavailable` (the capability is not currently
provisioned). Distinct from `AuthError::OAuthFailed(_)` which maps to
HTTP 502 `UpstreamError` (the IdP failed). Closes a discoverability
gap for API integrators.

The variant lands in the `AuthError → outcome` exhaustive audit added
in #753; a new closed-set `outcome = "provider_not_configured"` label
value joins the `nebula_api_auth_*` metrics family.

### D-7 — IdP tokens discarded after Nebula session is minted

IdP-issued `access_token`, `refresh_token`, and `id_token` are
**not persisted** anywhere — no store, no log, no metric. The OAuth
flow proves identity; Nebula owns the session. Persisting IdP tokens
would tie session lifetime to IdP token lifetime, drag the credential
rotation fan-out (#688/#690) into the session path, and re-open the
"downstream calls using IdP creds" surface (deferred to 1.1).

Rust's borrow checker enforces the discard: `OAuth2Token` (or its local
typed equivalent) is a function-local value in `complete_oauth` that
drops at function exit.

### D-8 — `external_identities` table links `(provider, sub) → user_id`

New PG migration:

```sql
CREATE TABLE external_identities (
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider    TEXT NOT NULL,
    subject     TEXT NOT NULL,
    email       TEXT,
    linked_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, subject)
);
CREATE INDEX external_identities_user_id_idx ON external_identities (user_id);
```

Primary key `(provider, subject)` because the IdP guarantees `sub` is
stable per user inside its tenant. `email` is a snapshot at link-time
for audit; `users.email` is the authoritative source for "user's email".
`ON DELETE CASCADE` so deleting a Nebula user removes the IdP linkage.
No `tenant_scope` column — external identity is global per
`(provider, sub)` because IdPs do not respect Nebula's tenancy.

### D-9 (recon-3 narrowed) — `oauth_allow_insecure_localhost` flag scope

The flag scope is narrower than the original design proposed. Existing
in-tree code at `crates/api/src/transport/oauth/flow.rs::validate_token_endpoint`
strictly rejects `http`, `localhost`, private/loopback IPv4, ULA/link-local
IPv6, and IPv4-mapped equivalents — for **token URLs**. This is an
anti-SSRF defense that MUST stay in 1.0.

The `oauth_allow_insecure_localhost` flag therefore applies ONLY to
`authorize_url` (which the browser fetches; no server-side SSRF surface)
and the now-defunct `redirect_uris` (eliminated by D-3 recon-4). For
`token_url` / `userinfo_url`, the strict policy is non-negotiable. The
flag is rejected at boot when the binary is built with the `release`
feature.

Integration tests against `wiremock` on `127.0.0.1` work via the
`test-util` feature gate documented in D-14.

### D-11 (recon-3 revised) — Reuse `mint_pkce` + `flow::build_authorization_uri` + `OAuthStateRepo`

`PgAuthBackend::start_oauth` uses the existing in-crate `mint_pkce()`
helper (already generates state + code_verifier + code_challenge),
builds `flow::AuthorizationUriRequest` from the operator config, calls
`flow::build_authorization_uri(req, state, code_challenge)`, and
persists `OAuthStateRow` via the existing `self.oauth_state_repo.create(...)`
(slot already on `PgAuthBackend`).

**This does NOT use `OAuth2Credential::initiate_authorization_code`** —
that helper produces `OAuth2Pending`, which is a Plane B shape persisted
to `pending_credentials` via `PendingStateStore`. Plane A uses the
distinct `OAuthStateRepo` / `plane_a_oauth_states` table.

### D-12 (recon-3 revised) — Use `flow::exchange_code` for token endpoint

`PgAuthBackend::complete_oauth` uses the existing `flow::exchange_code(TokenExchangeRequest)`
helper at `crates/api/src/transport/oauth/flow.rs:79-118`. It wraps
`oauth_token_http_client()` + form-encoded body + `AuthStyle::Header`/`PostBody`
selection + bounded response reading + `validate_token_endpoint`. PR-4
does NOT introduce its own HTTP client.

The userinfo GET reuses `oauth_token_http_client()` directly with a
`Bearer` header.

### D-13 — Plane A does NOT route through `Interactive::continue_resolve`

`Interactive::continue_resolve` on `OAuth2Credential` is designed to
persist the resolved `OAuth2State` as an encrypted credential row.
Plane A (per D-7) discards tokens. Bypassing `continue_resolve` makes
the discard a type-level property: no `OAuth2State` is ever constructed,
encrypted, or persisted.

A future change (1.1 "OAuth-as-stored-credential" surface) will use
`continue_resolve` independently for Plane B. Both paths coexist without
conflict.

### D-14 — `nebula-api` `test-util` feature exposes `exchange_code_unchecked`

Integration tests against `wiremock` on `127.0.0.1` cannot pass
`validate_token_endpoint`'s strict policy (D-9). A new `test-util`
feature on `nebula-api`'s `Cargo.toml` exposes
`flow::exchange_code_unchecked` (the existing private bypass at
`flow.rs:330+`) to integration test consumers via `[dev-dependencies]
nebula-api = { path = "...", features = ["test-util"] }`.

The `release` feature build rejects the `test-util` feature via a
compile-time assertion (or CI gate in `lefthook.yml`). Production
binaries do NOT include `test-util`. This matches the existing
`nebula-credential-runtime::test_fixtures` precedent at
`crates/credential-runtime/src/lib.rs:31-35`.

### D-15 — OIDC discovery doc fetch + process-lifetime cache

`crates/api/src/transport/oauth/discovery.rs` (new file) exposes
`fetch_oidc_discovery(url) -> Result<OidcDiscovery, DiscoveryError>`
which validates the URL via `validate_token_endpoint`, GETs `.well-known/openid-configuration`
via `oauth_token_http_client()`, parses, and caches the result for the
process lifetime (no TTL; discovery docs are stable per provider;
restart Nebula to refresh).

Discovery failures surface at `start_oauth` call time as
`AuthError::OAuthFailed { cause: "oidc_discovery_failed" }`, not at
boot. The operator may add a provider config that points at an
unreachable discovery URL; boot succeeds, the first OAuth attempt
fails closed with a typed cause.

### D-16 — id_token JWKS signature validation deferred to 1.1

The 1.0 OAuth identity flow does **not** validate the `id_token`
signature against the IdP's JWKS. The userinfo endpoint over TLS is
treated as authoritative for the user's `email` + `sub`. Presence of
`id_token` in the token response is logged (`tracing::debug!(id_token_present=true)`)
but otherwise ignored.

**Why defer:** `jsonwebtoken` is already in workspace deps with the
`rust_crypto` feature, so signature validation alone is ~80 LOC. But
proper handling — JWKS fetch, cache with key rotation, multi-key
selection by `kid`, claim validation (`iss`/`aud`/`exp`/`iat`/`nonce`),
clock-skew tolerance — is ~200 LOC + ongoing maintenance. The
`openidconnect` crate (v4.0.1) is the canonical Rust solution but is
a heavy dep to introduce mid-cycle; 1.1 picks the right shape.

**Why this is safe enough for 1.0:**

- The token endpoint is over TLS, validated by `validate_token_endpoint`
  (anti-SSRF defense in D-9).
- The userinfo endpoint reuses the same HTTP client policy.
- A TLS-chain compromise to the IdP would also break JWKS fetching, so
  signature validation does not provide a defense JWKS-fetch-over-TLS
  doesn't itself rely on.
- The anti-CSRF `state` and PKCE `code_verifier` defenses are intact —
  they're pre-token-endpoint defenses, not id_token concerns.
- The marginal residual risk is a malicious operator-chosen IdP swapping
  the userinfo response; an operator who does not trust their IdP's
  TLS chain has bigger problems.

**Release-notes blurb** (must appear in `crates/api/README.md`
"Known limitations" section per PR-5):

> **OAuth identity login (1.0)**: Nebula ships authorization-code with
> PKCE for OIDC providers (Google, Microsoft, Auth0, Okta) and OAuth2-only
> providers (GitHub). The IdP's userinfo endpoint over TLS is the
> authoritative source for the user's verified email and stable subject
> identifier. `id_token` signature validation against the IdP's JWKS is
> **not** performed in 1.0 — a 1.1 hardening pass will add it via the
> `openidconnect` crate or equivalent. Operators that require strict OIDC
> compliance now should track issue #TBD (filed alongside the closing
> PR).

## Superseded sub-decisions (audit trail)

The recon waves invalidated three sub-decisions from the original
design. Documented here so the audit chain is reachable without reading
the recon appendices:

| Sub-decision (historical) | Recon | Reason superseded |
|---|---|---|
| **D-2** — dyn-erase `CredentialServiceErased` for `AppState::credential_service` | recon-2 | `CredentialService` is not consumed by Plane A. Operator IdP-client config is infra config (D-1), not credential rows. No dyn-erase trait introduced. |
| **D-4** — `OAuthProviderCredentialKey` newtype with `pub(crate)` constructor | recon-2 | No credential lookup happens in Plane A. The newtype was scaffolding for a typed-decode seam that is itself superseded (see REQ-cred-001 below). |
| **REQ-cred-001** — `CredentialService::get_for_oauth_provider` typed-decode seam | recon-2 | The operator config carries `client_id`/`client_secret` directly. Decoding from `CredentialSnapshot` is moot when the values are already typed in `ApiConfig`. |
| **D-3 (original)** — `redirect_uris: Vec<String>` allow-list | recon-4 | Replaced by auto-derivation from `ApiConfig::public_url` (D-3 revised above). |
| **D-11 (original)** — Use `OAuth2Credential::initiate_authorization_code` + `pending_state_store` | recon-3 | Wrong store: that helper produces Plane B `OAuth2Pending`. Plane A uses the distinct `OAuthStateRepo` already wired into `PgAuthBackend`. |
| **D-12 (original)** — Generic "use `transport/oauth/http.rs`" | recon-3 | Refined to "use `flow::exchange_code` wrapper specifically" (more precise about the right call site). |

The recon appendices (`recon-2-credential-domain.md`,
`recon-3-flow-and-pending.md`, `recon-4-n8n-and-rust-ecosystem.md`)
preserve the full reasoning. Subsequent ADRs that touch these surfaces
should reference both this ADR and the relevant recon.

## Consequences

**Code paths landing across PR-2 through PR-5:**

- New file `crates/api/src/config/oauth.rs` — `OAuthProvidersConfig`,
  `OAuthProviderConfig`, `OAuthEndpoints` enum (`Oidc` / `Manual`),
  validation, env binding.
- New file `crates/api/src/transport/oauth/discovery.rs` —
  `fetch_oidc_discovery` + process-lifetime cache.
- New file `crates/api/src/transport/oauth/known.rs` — hardcoded
  known-provider defaults.
- New variant `AuthError::ProviderNotConfigured { provider }` and
  `AuthError → ApiError → HTTP 503` mapping.
- New migration `crates/storage/migrations/postgres/00XX_external_identities.sql`
  + `PgExternalIdentityRepo` + `InMemoryExternalIdentityRepo` parallel.
- Rewritten `PgAuthBackend::start_oauth` (real authorize URL via
  `mint_pkce` + `flow::build_authorization_uri` + `OAuthStateRepo`).
- Rewritten `PgAuthBackend::complete_oauth` (real token exchange via
  `flow::exchange_code` + userinfo GET + find-or-create user with
  account-takeover defense + session mint). Removes the `NotImplemented`
  return.
- Symmetric `InMemoryAuthBackend` impl for both methods.
- Trait signature change: `AuthBackend::start_oauth(provider, redirect_uri)`
  (redirect_uri auto-derived by the handler from `ApiConfig::public_url`).
- New `nebula-api` `test-util` feature exposing
  `flow::exchange_code_unchecked` for integration tests; rejected in
  `release` builds.

**Code paths NOT changing:**

- `CredentialService` and `nebula-credential-runtime` public surface —
  no new method, no dyn-erase trait, no new credential type.
- `OAuth2Credential` and its `Interactive`/`Refreshable`/`Revocable`/`Testable`
  trait impls — Plane B unchanged.
- `validate_token_endpoint` — anti-SSRF policy preserved.
- `PendingStateStore` (Plane B store) — untouched by this change.
- Workspace `Cargo.toml` — every required dep (`reqwest`, `nebula-credential`,
  `nebula-credential-runtime`, `wiremock` (dev), `jsonwebtoken`, `hmac`,
  `sha2`, `zeroize`) is already present.

**Strict TDD evidence** (per `openspec/config.yaml`): 23 RED tests
anchored across PR-2 (5 tests), PR-3 (5 tests), PR-4 (13 tests). PR-5
is doc-only; PR-1 (this ADR) is markdown-only.

**Review workload:** 5-PR chain, total ~1,230 LOC, each PR ≤ 800 LOC.
Squash-merge order: 1 → 2 → 3 → 4 → 5. No parallelization. PR-1 ADR
gates the chain.

**1.1 follow-ups carved out:**

- id_token JWKS signature validation (D-16). Tracked via the issue
  filed alongside PR-5.
- Plane B credential-OAuth maturity work (out of scope for this ADR).
- DB-managed admin UI for OAuth provider config (n8n's
  `N8N_SSO_MANAGED_BY_ENV=false` equivalent).
- Wholesale replacement of `flow::build_authorization_uri` +
  `flow::exchange_code` with the `openidconnect` crate (if maintenance
  burden grows).
