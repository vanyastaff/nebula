# Recon-4 — n8n prior art + Rust ecosystem patterns

- **Status**: research appendix to design.md.
- **Trigger**: user asked to study how n8n implements identity OAuth + operator-secret provider config.
- **Sources**: deepwiki on `n8n-io/n8n` (`OidcService`, `SamlService`, `IdentityResolutionService`, env-vs-DB config); exa research on Rust ecosystem (`openidconnect` crate, `jsonwebtoken` crate, JWKS verification).
- **Outcome**: three n8n patterns worth adopting (env-vs-DB config gate, OIDC discovery endpoint, derived redirect_uri); one Rust ecosystem finding (`jsonwebtoken` already a workspace dep — JWKS verification cheap to add); one defer decision (id_token signature validation → 1.1).

---

## 1. n8n architectural patterns

### Two-planes OAuth distinction is universal

n8n separates "OAuth for workflow node credentials" (their Plane B) from "OAuth identity login" (their Plane A) the same way Nebula does. The two planes have completely separate storage, services, and code paths in n8n:

- **Plane B**: `OAuth2Api` credential type stores `clientId`/`clientSecret`/`authUrl`/`accessTokenUrl` per-credential in DB; `OAuth2CredentialController` + `OauthService` orchestrate; tokens encrypted via `encryptAndSaveData`.
- **Plane A**: `OidcService` / `SamlService` for SSO providers; `IdentityResolutionService` handles callback → local user resolution.

This validates Nebula's two-plane design (recon-3 §1). No change needed.

### n8n's OIDC config dual-mode: env-managed vs DB-managed

n8n exposes a top-level toggle `N8N_SSO_MANAGED_BY_ENV={true|false}`:

- `true` (env-managed): config read-only from env vars on every startup; admin UI locked; API writes rejected.
- `false` (default; DB-managed): operator configures via admin UI; stored encrypted in DB under `OIDC_PREFERENCES_DB_KEY`; `clientSecret` encrypted via `Cipher.encryptV2()`, redacted as `OIDC_CLIENT_SECRET_REDACTED_VALUE` when read back.

**Nebula 1.0 has no admin UI for SSO config** — DB-managed mode requires UI work that is M3.6+ scope. So 1.0 ships **env-managed only**, matches our D-1.

### n8n's OIDC config schema is MINIMAL

The n8n `OidcConfig` DTO carries only:
- `clientId` (string)
- `clientSecret` (encrypted string)
- `discoveryEndpoint` (URL — OIDC `.well-known/openid-configuration`)
- `loginEnabled` (bool)
- `prompt` ('select_account' | 'login' | 'consent' | 'none')
- `authenticationContextClassReference` (string[] — ACR values)

**That's six fields**. No `authorize_url`, `token_url`, `userinfo_url`, `jwks_url`, `scopes`, `redirect_uri`, or `pkce_*` flags.

**How n8n handles the missing fields**:
- `authorize_url` / `token_url` / `userinfo_url` / `jwks_url` → fetched at runtime from the `discoveryEndpoint` (`.well-known/openid-configuration`). Standard OIDC discovery.
- `scopes` → **hardcoded** to `'openid email profile'` + optional provisioning scopes via separate env vars (`N8N_SSO_SCOPES_PROVISION_INSTANCE_ROLE`, etc.). Operator does NOT pick scopes per provider.
- `redirect_uri` → **auto-derived** as `{instanceBaseUrl}/rest/sso/oidc/callback`. Operator does NOT supply.
- `pkce_*` → **always-on S256** via `openid-client` library. Operator does NOT toggle.

### n8n's HTTPS posture is LOOSER than Nebula

n8n delegates to `openid-client` (npm) which accepts any valid URL including `http://localhost`. No explicit `validate_token_endpoint` SSRF defense in n8n's identity code path.

Nebula's `validate_token_endpoint` is stricter (recon-3 §4) and we should keep it. The asymmetry is intentional — Nebula's posture is more security-conscious.

### n8n's user resolution matches REQ-oauth-004/005

n8n's `IdentityResolutionService` order:
1. AuthIdentity lookup by `(provider, sub)` → use linked user.
2. Email fallback → existing user with same email → link external identity.
3. JIT provisioning → new user + personal project; requires email claim.

This matches Nebula's REQ-oauth-004 (first-login) + REQ-oauth-005 (existing-user link). **No change to spec needed.**

n8n does NOT have an equivalent of REQ-oauth-005 Scenario 5.2 (account-takeover defense via "Nebula email unverified rejects IdP-verified link"). Nebula's defense is stricter than n8n's. Keep ours.

## 2. Rust ecosystem findings

### `openidconnect` crate is the canonical Rust path

- `openidconnect = "4.0.1"` — strongly-typed OIDC + OAuth2; handles discovery, JWKS auto-fetch + cache with key rotation, signature verification.
- Best path for production Rust OIDC code.
- For 1.0: adopting this crate would mean replacing `flow::build_authorization_uri` + `flow::exchange_code` with `openidconnect` calls — out of scope (large refactor; flow.rs already shipped).

### `jsonwebtoken` is ALREADY a workspace dep

`Cargo.toml` carries `jsonwebtoken = { version = "10", features = ["rust_crypto"] }`. No new dep needed for id_token JWKS verification.

### GitHub is OAuth2-only (no OIDC discovery)

GitHub.com does NOT expose `.well-known/openid-configuration`. For GitHub identity login, the flow is:
1. Authorize URL: `https://github.com/login/oauth/authorize`.
2. Token endpoint: `https://github.com/login/oauth/access_token`.
3. Userinfo: `GET https://api.github.com/user` with `Authorization: Bearer <access_token>`.
4. No `id_token`, no JWKS, no nonce — just trust the userinfo response.

So Nebula's 1.0 schema MUST support both OIDC-discoverable providers AND OAuth2-only providers. Two shapes coexist.

## 3. Adopt / skip decisions

### ADOPT — Auto-derive `redirect_uri` from `ApiConfig::public_url`

**Existing**: `crates/api/src/config/mod.rs:107` already declares `pub public_url: String`, bound to `API_PUBLIC_URL` env var with default `http://{bind_address}`.

**Change**: drop `redirect_uris: Vec<String>` from `OAuthProviderConfig`. Derive `redirect_uri = format!("{}/auth/oauth/{}/callback", api_config.public_url, provider.as_str())`. Single source of truth.

**Impact on prior decisions**:
- **D-3 (allow-list redirect_uris) → SUPERSEDED by recon-4**. Per provider config has no redirect_uris field. The instance has one public URL; each provider has one derived callback.
- **R.6 (single vs allow-list) → CLOSED** — implicitly resolved by auto-derivation.
- Multi-environment deploys (dev/staging/prod) each have their own `API_PUBLIC_URL` and their own OAuth client registration with the IdP. Operators that want multi-callback-URI register multiple Nebula instances OR set up an external IdP-proxy. This matches n8n's posture.

**Impact on spec**:
- REQ-oauth-002 Scenario 2.3 (redirect_uri membership check) → DELETED. There is no allow-list to check membership against.
- REQ-oauth-003 Step 4 (redirect_uri mismatch) → REWRITTEN: server verifies the `OAuthStateRow.redirect_uri` matches the freshly-derived `format!(...)` value. If they differ, the row was created under a different `public_url` (operator changed it mid-flow) → reject with `AuthError::OAuthFailed { cause: "public_url_changed_mid_flow" }`. Edge case but auditable.

### ADOPT — Prefer OIDC discovery endpoint when available

**Change**: `OAuthProviderConfig::endpoints` becomes a tagged union:

```rust
enum OAuthEndpoints {
    /// OIDC provider — endpoints discovered at runtime from `.well-known/openid-configuration`.
    Oidc { discovery_url: String },
    /// OAuth2-only provider — explicit endpoint URLs.
    Manual {
        authorize_url: String,
        token_url: String,
        userinfo_url: String,
        jwks_url: Option<String>, // None for providers without id_token (e.g. GitHub)
    },
}
```

**Impact on prior decisions**:
- **D-5 (Generic requires endpoints; known providers may override) → REFINED**. Known providers (Google, Microsoft) ship as `Oidc { discovery_url: "https://accounts.google.com/.well-known/openid-configuration" }` defaults. GitHub ships as `Manual { ... }` defaults. Operator can override either.
- **D-12-RECON3 (use flow::exchange_code) → STAYS** but PR-4 adds a small discovery-document fetcher (`fetch_oidc_discovery(url) -> OidcDiscovery`) that caches the discovery document for the process lifetime (since it's stable per provider). ~50 LOC.

### ADOPT — Hardcoded scopes `openid email profile` for OIDC; per-provider for OAuth2-only

**Change**: drop `scopes: Vec<String>` from `OAuthProviderConfig` for OIDC providers. Hardcode `openid email profile`. For OAuth2-only providers (GitHub), keep `scopes: Vec<String>` (default `["user:email"]`).

**Impact**:
- Smaller config surface for the 80% case (OIDC).
- OAuth2-only providers (GitHub) keep scope flexibility because they need provider-specific scopes (`user:email`, `repo`, etc.).
- Matches n8n's posture.

### DEFER — id_token JWKS signature validation → 1.1

**Reason**:
- `jsonwebtoken` is in workspace deps — JWKS verification is technically ~80 LOC of code (fetch JWKS, parse, validate signature + claims).
- BUT: JWKS fetching needs HTTP + caching + key rotation handling. Doing this correctly is ~200 LOC if we don't use `openidconnect` crate.
- The userinfo endpoint is authoritative for email + sub. If userinfo is over TLS to a trusted IdP, the security gap from skipping id_token validation is small.
- 1.0 ships honest: "id_token signature is NOT validated in 1.0; userinfo endpoint is the source of truth. 1.1 will add JWKS verification via the `openidconnect` crate or equivalent."

**Impact on prior decisions**:
- **REQ-oauth-003 Steps 7-8 (id_token validation)** → DEFERRED to 1.1. Step 8 (id_token nonce match) too — without signature verification, nonce is unverifiable.
- **Scenario 3.7 (id_token signature invalid)** → DELETED from 1.0 spec; moved to 1.1 follow-up note.
- **Scenario 3.8 (id_token nonce mismatch)** → DELETED from 1.0 spec; moved to 1.1.
- **Sub-impact**: the `state` in `OAuthStateRow` is still anti-CSRF; PKCE `code_verifier` is still validated by the IdP. The CSRF + PKCE defenses are intact; only id_token signature is deferred.
- **Risk**: a malicious IdP could swap the userinfo response (if the operator misconfigured a public WiFi proxy or similar). Mitigation: token endpoint MUST be HTTPS (already enforced by `validate_token_endpoint`); userinfo endpoint reuses the same HTTP client policy.

### SKIP — env-managed-vs-DB-managed dual mode (admin UI is M3.6+)

**Reason**: Nebula 1.0 has no admin UI for SSO config. DB-managed mode is a 1.1+ ergonomic extension. 1.0 ships env-managed only (matches D-1).

### SKIP — Replace `flow::build_authorization_uri` + `flow::exchange_code` with `openidconnect` crate

**Reason**: large refactor; current hand-rolled flow.rs is already shipped + tested. 1.0 stays with the in-tree helpers. A follow-up plan (post-1.0) can swap to `openidconnect` if the maintenance burden grows.

## 4. Final config schema (REVISED)

```toml
# Env-managed config (1.0 default and only supported mode):
[auth.oauth.providers.google]
client_id     = "${API_AUTH_OAUTH_GOOGLE_CLIENT_ID}"
client_secret = "${API_AUTH_OAUTH_GOOGLE_CLIENT_SECRET}"
endpoints     = { kind = "oidc", discovery_url = "https://accounts.google.com/.well-known/openid-configuration" }
# scopes omitted — hardcoded "openid email profile" for OIDC providers

[auth.oauth.providers.github]
client_id     = "${API_AUTH_OAUTH_GITHUB_CLIENT_ID}"
client_secret = "${API_AUTH_OAUTH_GITHUB_CLIENT_SECRET}"
endpoints     = { kind = "manual", authorize_url = "https://github.com/login/oauth/authorize", token_url = "https://github.com/login/oauth/access_token", userinfo_url = "https://api.github.com/user" }
scopes        = ["user:email"]  # required for OAuth2-only providers

[auth.oauth.providers.acme_corp]
client_id     = "${API_AUTH_OAUTH_ACME_CLIENT_ID}"
client_secret = "${API_AUTH_OAUTH_ACME_CLIENT_SECRET}"
endpoints     = { kind = "oidc", discovery_url = "https://sso.acme.corp/.well-known/openid-configuration" }
```

Env-binding mirror per provider:
- `API_AUTH_OAUTH_<PROVIDER>_CLIENT_ID`
- `API_AUTH_OAUTH_<PROVIDER>_CLIENT_SECRET`
- `API_AUTH_OAUTH_<PROVIDER>_DISCOVERY_URL` (for `kind=oidc`)
- `API_AUTH_OAUTH_<PROVIDER>_AUTHORIZE_URL` / `_TOKEN_URL` / `_USERINFO_URL` (for `kind=manual`)
- `API_AUTH_OAUTH_<PROVIDER>_SCOPES` (comma-separated; required for `kind=manual`)

`redirect_uri` is derived: `{api_config.public_url}/auth/oauth/<provider>/callback`. NOT configurable.

## 5. Recompute PR sizes (REVISED PER RECON-4)

| # | PR | Scope | LOC |
|---|---|---|---|
| 1 | **ADR** | Documents all recon-2 + recon-3 + recon-4 SUPERSEDED decisions. The single source of architectural truth. | ~180 |
| 2 | **Trait + config + redirect_uri-from-public_url + compose validation + test-util feature** | `start_oauth(provider, redirect_uri)` sig (redirect_uri derived from public_url, NOT user-supplied); `OAuthProvidersConfig` types per recon-4 §4; env-binding; compose-root validation; `test-util` feature exposing `exchange_code_unchecked`. | ~330 |
| 3 | **Real authorize URL via `flow::build_authorization_uri` + OIDC discovery fetch** | `PgAuthBackend::start_oauth` derives redirect_uri from public_url; for `Oidc` endpoints, fetches discovery doc (cached); calls `mint_pkce` + `flow::build_authorization_uri`; persists `OAuthStateRow`. | ~150 |
| 4 | **Real complete_oauth + userinfo + external_identities + find-or-create** | Token exchange via `flow::exchange_code`; userinfo GET via `oauth_token_http_client()` with Bearer header; first-login / existing-user logic; `external_identities` table migration + repo; `InMemoryAuthBackend` parallel. **NO id_token JWKS validation (deferred to 1.1).** | ~330 |
| 5 | **Integration tests + docs + ROADMAP flip** | Wiremock integration tests (revised counts — JWKS-related scenarios deleted); README; ROADMAP §M3.1 final checkbox. | ~240 |

**Total**: ~1,230 LOC, 5 PRs. ~40 LOC less than recon-3 because id_token JWKS validation deferred. PR-3 grows by ~70 LOC because of OIDC discovery doc fetch + cache.

## 6. RED test anchors (REVISED per recon-4)

Counted PRs:
- PR-2: 5 RED tests (handler-extracts-redirect-uri-from-public-url, handler-rejects-missing-redirect-uri-when-public-url-unset, oauth-config-validation-rejects-non-https-discovery-url, oauth-config-validation-rejects-empty-scopes-for-manual, compose-fails-closed-on-invalid-provider-config).
- PR-3: 5 RED tests (start_oauth-derives-redirect-uri-from-public-url, start_oauth-fetches-oidc-discovery-and-uses-its-authorize_url, start_oauth-uses-manual-endpoints-for-oauth2-only-provider, start_oauth-emits-pkce-s256, start_oauth-rejects-unknown-provider).
- PR-4: 11 RED tests (complete_oauth-happy-path-oidc, complete_oauth-happy-path-oauth2-only, complete_oauth-rejects-replay, complete_oauth-rejects-expired-state, complete_oauth-rejects-mismatched-state, complete_oauth-rejects-mismatched-provider, complete_oauth-rejects-public-url-change-mid-flow, complete_oauth-handles-idp-token-endpoint-500, complete_oauth-rejects-malformed-token-response, complete_oauth-creates-user-first-login-verified-email, complete_oauth-rejects-link-unverified-nebula-email).
- PR-5: docs (no RED tests, just doctest snippets).

**Net 21 RED tests** (was 24 in recon-3; 3 JWKS-related tests deleted, 2 new tests added for OIDC vs Manual provider distinction).

## 7. Decision summary for the user

| Decision | Vote |
|---|---|
| Adopt `public_url`-derived `redirect_uri` (drop allow-list) | **Yes — simplifies config, matches n8n, eliminates redirect_uri membership scenarios** |
| Adopt OIDC discovery endpoint as primary path | **Yes — smaller config surface for OIDC providers; Manual fallback for OAuth2-only** |
| Adopt hardcoded `openid email profile` scopes for OIDC | **Yes — drops a config field; per-provider override only for `Manual` endpoints** |
| Defer id_token JWKS signature validation to 1.1 | **Yes — userinfo endpoint is authoritative; saves ~100 LOC + dep on `openidconnect` crate** |
| Skip DB-managed admin UI mode | **Yes — 1.1+ scope; 1.0 is env-managed only** |
| Skip wholesale replacement with `openidconnect` crate | **Yes — too large a refactor for M3.1** |

## 8. Required artifact patches (recon-4 = third patch wave)

| Artifact | Change |
|---|---|
| `design.md` D-3 | SUPERSEDED-BY-RECON-4: redirect_uri auto-derived from `ApiConfig::public_url`; no allow-list. |
| `design.md` D-5 | REFINED-BY-RECON-4: `OAuthEndpoints` tagged union (`Oidc { discovery_url }` vs `Manual { ... }`). Known providers ship as defaults (Google = Oidc, GitHub = Manual). |
| `design.md` (new) D-15 | OIDC discovery doc fetch + process-lifetime cache. |
| `design.md` (new) D-16 | Defer id_token JWKS signature validation to 1.1; userinfo is authoritative for `email` + `sub`. |
| `design.md` D-10 LOC table | Update per recon-4 §5: PR-3 ~150 LOC (was 80 in recon-3, growth from discovery fetch), PR-4 ~330 LOC (was ~400 in recon-3, savings from JWKS defer). |
| `spec.md` REQ-oauth-001 | Update Invariant 1 + Scenarios to reflect new config shape (Oidc vs Manual tagged union). |
| `spec.md` REQ-oauth-002 | Scenario 2.3 (redirect_uri membership) DELETED; auto-derivation makes it moot. Step 2 updated: lookup OAuthProviderConfig; resolve endpoints via discovery doc if Oidc; build AuthorizationUriRequest. |
| `spec.md` REQ-oauth-003 | Steps 7-8 (id_token validation) DELETED; Scenarios 3.7 + 3.8 (id_token sig + nonce) DELETED. New Scenario 3.10: rejects `public_url` change mid-flow. |
| `spec.md` REQ-compose-001 | Validation scenarios updated: Oidc kind requires `discovery_url`, Manual kind requires all four manual fields + `scopes`. |
| `proposal.md` §5 acceptance criteria | A.3 update (delete JWKS-related rejection sub-criteria); A.13 (README) update for new config shape. |
| `proposal.md` §6 risks | R.6 dropped (auto-derived solves it). New R-D7: 1.0 ships without id_token signature validation; userinfo is authoritative. Release-notes blurb required. |
| `proposal.md` §7 chain | LOC re-table per recon-4 §5. |

---

## Result envelope

```yaml
status: recon-4-complete
executive_summary: |
  Studied n8n via deepwiki + Rust ecosystem via exa. Three patterns to ADOPT: (a) auto-derive
  redirect_uri from existing ApiConfig::public_url (drops D-3 allow-list); (b) OIDC discovery
  endpoint as primary, Manual endpoints as fallback for OAuth2-only providers like GitHub
  (refines D-5 to tagged union); (c) hardcoded "openid email profile" scopes for OIDC (per-provider
  scopes only for Manual). One DEFER: id_token JWKS signature validation → 1.1 (userinfo is
  authoritative for 1.0; saves ~100 LOC + complex dep). Two SKIPs: DB-managed admin UI mode
  (1.1+); wholesale replace flow.rs with openidconnect crate (too large refactor). jsonwebtoken
  already in workspace deps; openidconnect crate would be the long-term path but not for 1.0.
  Net: 6 patches to design.md (D-3 superseded; D-5 refined; D-15/D-16 new; D-10 LOC table revised),
  4 patches to spec.md (config schema, REQ-oauth-002/003 simplified, REQ-compose-001 expanded),
  3 patches to proposal.md (acceptance criteria, risks, chain LOC). Net 21 RED tests (was 24).
  Total ~1,230 LOC, 5 PRs. n8n's IdentityResolutionService matches our REQ-oauth-004/005;
  account-takeover defense (Scenario 5.2) stays as Nebula-specific stricter posture.
artifacts:
  - openspec/changes/oauth-providers-from-operator-secrets/recon-4-n8n-and-rust-ecosystem.md
next_recommended: user decides which recon-4 ADOPTs to apply (default: all); then patch design+spec+proposal; then sdd-tasks
risks:
  - R-D7 (NEW): 1.0 ships without id_token JWKS signature validation. Documented as known limitation; userinfo endpoint is authoritative. 1.1 follow-up plan tracks the gap.
skill_resolution: none
```
