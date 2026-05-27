# Recon-3 — `transport/oauth/flow.rs` + Plane-A pending store audit

- **Status**: appendix to `design.md` + `recon-2-credential-domain.md`; flags **material correction to recon-2 itself**.
- **Trigger**: user requested medium-confidence files read before `sdd-tasks`.
- **Scope**: `crates/api/src/transport/oauth/{flow,http}.rs` full files, `crates/storage/src/credential/pending.rs`, `crates/storage/src/pg/oauth_state.rs`, `crates/storage/src/repos/user.rs` (`OAuthStateRepo` trait), `crates/api/Cargo.toml`, and the existing `PgAuthBackend::start_oauth` body at `pg.rs:1028-1075`.
- **Outcome**: **recon-2's D-11 was directionally wrong** (used Plane-B store for a Plane-A flow). Recon-3 corrects: there is a dedicated **Plane-A `OAuthStateRepo`** (table `plane_a_oauth_states`), already wired into `PgAuthBackend`, distinct from Plane-B's `PendingStateStore`. Plus `flow::build_authorization_uri` + `flow::exchange_code` already implement the OAuth ceremony end-to-end. PR sizes shrink further; D-9 conflicts with `validate_token_endpoint`.

---

## 1. The two-planes design is real and shipped

Nebula's OAuth surface is sliced into two **non-overlapping** planes, and the storage layer reflects this:

| Plane | Purpose | Pending store trait | Storage row | Table |
|---|---|---|---|---|
| **Plane A** — identity OAuth login (this M3.1 change) | "Sign in with Google" → mint Nebula session, discard IdP tokens | `OAuthStateRepo` (in `crates/storage/src/repos/user.rs`) | `OAuthStateRow` (slim: state, provider, code_verifier, redirect_uri, timestamps) | `plane_a_oauth_states` |
| **Plane B** — credential OAuth (1.1 surface) | User stores an OAuth credential for workflow actions to call APIs on their behalf | `PendingStateStore` (in `nebula-credential::pending_store`) | `OAuth2Pending` (fat: full config, client creds, PKCE verifier, device code, state, redirect_uri) | `pending_credentials` |

`crates/storage/src/pg/oauth_state.rs:5-8` is explicit:

> "Holds Plane-A sign-in-with-OAuth PKCE state — distinct from the Plane-B credential OAuth pending surface (`pending_credentials`)."

Recon-2 mistakenly recommended using `AppState::pending_state_store` (Plane B) for Flow A. **That was wrong.** The right surface is `OAuthStateRepo` / `OAuthStateRow`, which `PgAuthBackend` already holds at field-level (`self.oauth_state_repo`).

## 2. `PgAuthBackend::start_oauth` is mostly already implemented

The current `start_oauth` body at `crates/api/src/domain/auth/backend/pg.rs:1028-1075` is **80% done**:

```rust
async fn start_oauth(&self, provider: OAuthProvider) -> Result<OAuthStart, AuthError> {
    let pkce = mint_pkce()?;                                  // ✓ PKCE generated
    let authorize_url = format!(                              // ✗ Synthetic URL (the gap)
        "https://nebula.local/oauth/{}/authorize?state={}...",
        provider.as_str(), pkce.state, pkce.code_challenge,
    );
    let now = Utc::now();
    let expires_at = now + chrono_duration(OAUTH_STATE_TTL)?;
    self.oauth_state_repo.create(&OAuthStateRow {            // ✓ State persisted
        state: pkce.state.clone(),
        provider: provider.as_str().to_owned(),
        code_verifier: pkce.code_verifier,
        redirect_uri: None,                                   // ✗ trait signature gap
        created_at: now,
        expires_at,
        consumed_at: None,
    }).await?;
    Ok(OAuthStart { authorize_url, state: pkce.state })
}
```

What's missing:
- Real `authorize_url` (use `flow::build_authorization_uri`, see §3).
- `redirect_uri: Some(...)` (depends on trait signature change in PR-2).
- Provider-config lookup + `ProviderNotConfigured` branch (depends on config types in PR-2).

What's already present and reusable:
- ✓ `mint_pkce()` generates `state` + `code_verifier` + `code_challenge` (random 43-128 char URL-safe alphabet + SHA256 base64url).
- ✓ `OAUTH_STATE_TTL` constant (already enforces ≤ 10 min).
- ✓ `OAuthStateRepo::create` persists the row.
- ✓ `#[tracing::instrument(level = "info", skip(self), fields(provider = %provider.as_str()))]` already wraps the function — the observability triple is in place.
- ✓ `metrics_emit::run_with_metrics(NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL, ...)` already wired with outcome labels (SUCCESS / OAUTH_FAILED / INTERNAL).

## 3. `flow::build_authorization_uri` and `flow::exchange_code` are full-featured

`crates/api/src/transport/oauth/flow.rs:38-60` provides:

```rust
pub fn build_authorization_uri(
    req: &AuthorizationUriRequest,
    state: &str,
    code_challenge: &str,
) -> Result<Url, url::ParseError>
```

The function appends `response_type=code`, `client_id`, `redirect_uri`, `state`, `code_challenge`, `code_challenge_method=S256`, and optional `scope` — exactly the OAuth 2.1 authorize URL shape. `AuthorizationUriRequest` carries `auth_url`, `token_url`, `client_id`, `client_secret`, `redirect_uri`, `scopes`, `auth_style`. PR-3 builds this from the operator config and calls the helper.

`crates/api/src/transport/oauth/flow.rs:79-118` provides:

```rust
pub async fn exchange_code(req: &TokenExchangeRequest) -> Result<serde_json::Value, String>
```

The function:
1. Calls `validate_token_endpoint(&req.token_url)` (HTTPS + no localhost / private / loopback IPs).
2. POSTs `grant_type=authorization_code` + `code` + `redirect_uri` + `code_verifier` to the token endpoint with the bounded `reqwest::Client`.
3. Sends client creds via `AuthStyle::Header` (HTTP Basic) or `AuthStyle::PostBody` per request.
4. Returns parsed `serde_json::Value` on 2xx; `Err(String)` on any failure.

PR-4 builds `TokenExchangeRequest` from operator config + `OAuthStateRow.code_verifier` and calls the helper. Then parses `access_token` / `id_token` / `expires_in` from the JSON.

## 4. `validate_token_endpoint` is strict — conflicts with D-9

`crates/api/src/transport/oauth/flow.rs:118-185` rejects **token URLs** that:
- Are not HTTPS (any HTTP rejected).
- Target `localhost` (domain match).
- Target IPv4 private (`10.0.0.0/8`, `192.168.0.0/16`, `172.16.0.0/12`), loopback (`127.0.0.0/8`), link-local (`169.254.0.0/16`), unspecified, broadcast.
- Target IPv6 loopback, unspecified, multicast, ULA, link-local, site-local, and IPv4-mapped equivalents of the above.

This is an **anti-SSRF defense**. Recon-3 confirms with tests:
- `token_exchange_rejects_loopback_token_url` (line 248).
- `token_endpoint_rejects_ipv4_mapped_ipv6_private_addresses` (line 261).

**Impact on D-9** (`oauth_allow_insecure_localhost`):
- D-9 as written said localhost should be allowed in dev/test for OAuth endpoint URLs (token_url + authorize_url + redirect_uris) when the flag is on.
- **For `token_url`**: this contradicts `validate_token_endpoint`. The anti-SSRF defense MUST stay. D-9 cannot relax it.
- **For `authorize_url`**: the server never fetches the authorize URL (the browser does). No SSRF surface. D-9's relaxation is fine here.
- **For `redirect_uris`**: the server only validates membership; the callback URL is the same one the browser already loaded. No SSRF surface. D-9's relaxation is fine here.

**D-9 revised**: applies ONLY to `authorize_url` and `redirect_uris`. `token_url` MUST always be HTTPS + non-localhost. Period.

### Sub-problem: how do integration tests POST to `wiremock`?

`wiremock` (already a dev-dep on `nebula-api`) defaults to HTTP listener on 127.0.0.1. `validate_token_endpoint` blocks both. Existing in-tree precedent at `flow.rs:330-454` solves this by calling `exchange_code_unchecked` (private bypass) from the same crate's own tests.

Integration tests (`crates/api/tests/oauth_provider_e2e.rs`) cannot reach `exchange_code_unchecked` (private). Options for PR-4:

| Option | Description | Cost |
|---|---|---|
| **(a)** Expose `exchange_code_unchecked` via a `#[cfg(any(test, feature = "test-util"))]` gate (matching the `nebula-credential-runtime::test_fixtures` precedent at `lib.rs:31-35`) | The unchecked variant becomes available only when the `test-util` feature is enabled. Production binaries do not include it. | Low — matches existing pattern in the workspace. |
| **(b)** Refactor `exchange_code` to accept an injectable `&dyn TokenEndpointValidator` parameter (default to the strict one) | More flexible long-term; tests inject a no-op validator. | Medium — touches the function signature; needs design ADR commitment. |
| **(c)** Use HTTPS wiremock with self-signed cert + reqwest `danger_accept_invalid_certs(true)` in a test-only client builder | Skips the validator (which only checks scheme + host, not cert validity). Test-only `reqwest::Client` lives behind a test-util gate. | Medium — needs wiremock HTTPS setup; cert handling is fragile. |

**Recommendation**: **option (a)**. It's the lightest, matches the workspace's existing test-util gate pattern, keeps the strict validator on the production path, and integration tests just enable the feature.

This is a **new design decision required from PR-1's ADR**.

## 5. Workspace deps audit (final)

`crates/api/Cargo.toml` already declares:
- ✓ `reqwest = { workspace = true, default-features = false, features = [...] }` (direct dep, not transitive)
- ✓ `nebula-credential` (path dep)
- ✓ `nebula-credential-runtime` (path dep)
- ✓ `wiremock = { workspace = true }` (dev-dep, used by `tests/wiremock_smoke.rs`)
- ✓ "OAuth ceremony deps (nebula-credential, reqwest, hmac, sha2, zeroize)" comment confirms hmac + sha2 + zeroize already wired

**Nothing new to add to `Cargo.toml`** for PR-4 — every dep already exists. This collapses one whole sub-task that was in the original chain.

## 6. Net impact on prior artifacts

### Supersede design D-11 (reuse `initiate_authorization_code`)

**Status**: SUPERSEDED-BY-RECON-3.

**Wrong**: `OAuth2Credential::initiate_authorization_code` returns `OAuth2Pending` — a Plane-B shape. Persisting it via `pending_state_store` mixes planes.

**Right (D-11-RECON3)**: `PgAuthBackend::start_oauth` uses the EXISTING in-crate `mint_pkce()` helper (already generates state + code_verifier + code_challenge) + builds `flow::AuthorizationUriRequest` from the operator config + calls `flow::build_authorization_uri(req, state, code_challenge)` to construct the real URL + persists `OAuthStateRow` via `self.oauth_state_repo.create(...)` (already wired).

Net code change in `start_oauth`: ~30 LOC (replace synthetic format string + pipe redirect_uri through).

### Supersede design D-12 (reuse `transport/oauth/http.rs`)

**Status**: REFINED-BY-RECON-3 (not superseded — clarified).

**Was**: "use `crates/api/src/transport/oauth/http.rs` for HTTP POST".

**Now (D-12-RECON3)**: use `flow::exchange_code(TokenExchangeRequest)` — the higher-level helper that wraps `oauth_token_http_client()` + body-shaping + bounded response reading + endpoint validation. Direct use of `http.rs` is for the userinfo GET only (no equivalent flow.rs helper exists for userinfo).

### Supersede design D-9 (`oauth_allow_insecure_localhost` flag)

**Status**: SCOPE-NARROWED-BY-RECON-3.

**Was**: flag relaxes HTTPS-only / no-localhost for all OAuth URLs.

**Now (D-9-RECON3)**:
- Flag applies to `authorize_url` and `redirect_uris` only.
- `token_url` (and any future `userinfo_url` / `jwks_url` fetched by the server) MUST remain HTTPS + non-localhost regardless of the flag. The anti-SSRF policy in `validate_token_endpoint` is non-negotiable for production.
- Integration test access to localhost token endpoints goes through a NEW `test-util` feature gate on `nebula-api` (matching the `nebula-credential-runtime::test_fixtures` precedent), which exposes `exchange_code_unchecked` for test consumers. Production builds do NOT include this feature.

### Add D-14 — `nebula-api` `test-util` feature

**Status**: NEW.

**Decision**: `nebula-api/Cargo.toml` declares a `test-util` feature that:
- Exposes `flow::exchange_code_unchecked` (currently `async fn`, lift to `pub`).
- Exposes any other helpers the integration tests need (none others identified yet).

PR-1 ADR records the feature; PR-2 adds the feature flag to `Cargo.toml`; PR-3 uses it in the integration test crate via `[dev-dependencies] nebula-api = { path = "...", features = ["test-util"] }`.

### Recompute PR sizes (REVISED PER RECON-3)

| # | PR | Scope summary | LOC |
|---|---|---|---|
| 1 | **ADR** | D-1, D-3, D-5, D-6, D-7, D-8, D-9-RECON3, D-11-RECON3, D-12-RECON3, D-13, D-14. SUPERSEDED notes on D-2, D-4, REQ-cred-001 (per recon-2), D-9 / D-11 / D-12 (per recon-3). | ~150 |
| 2 | **Trait + config + redirect_uri + compose validation + test-util feature** | `start_oauth(provider, redirect_uri)` sig change + handler propagation; `OAuthProvidersConfig` types in `crates/api/src/config/oauth.rs`; env binding; compose-root validation; `nebula-api` `test-util` feature flag exposing `exchange_code_unchecked`. | ~380 |
| 3 | **Real authorize URL via `flow::build_authorization_uri`** | `PgAuthBackend::start_oauth` replaces synthetic URL with real one (builds `AuthorizationUriRequest`, calls helper, persists `redirect_uri: Some(...)`). `InMemoryAuthBackend::start_oauth` parallel impl. | **~80** (DRAMATICALLY smaller than recon-2's ~180) |
| 4 | **Real `complete_oauth` + JWKS + userinfo + external_identities + find-or-create** | Token exchange via `flow::exchange_code`; JWKS verification (~80 LOC of new code — no in-tree helper); userinfo GET via `oauth_token_http_client()` directly; `external_identities` table migration + `PgExternalIdentityRepo`; find-or-create user; account-takeover defense for unverified Nebula email; `InMemoryAuthBackend` parallel impl. | ~400 |
| 5 | **Integration tests + docs + ROADMAP flip** | `crates/api/tests/oauth_provider_e2e.rs` against wiremock IdP (14 RED tests per design D-10 revised table); `crates/api/README.md` OAuth section; ROADMAP §M3.1 final checkbox. | ~260 |

**Total**: ~1,270 LOC across 5 PRs. PR-3 collapsed by ~60% vs recon-2 because the authorize-URL builder already exists. PR-4 stays large because JWKS verification + external_identities + user resolution are the actual net-new work.

## 7. Final confidence check

| Claim | Confidence | Citation |
|---|---|---|
| Plane A and Plane B are distinct in storage + trait surfaces | **High** | `crates/storage/src/pg/oauth_state.rs:5-8` |
| `PgAuthBackend::start_oauth` already uses `OAuthStateRepo` | **High** | `crates/api/src/domain/auth/backend/pg.rs:1028-1075` |
| `mint_pkce` already generates state + code_verifier + code_challenge | **High** | Same body — `let pkce = mint_pkce()?;` |
| `flow::build_authorization_uri` builds real authorize URL with PKCE S256 | **High** | `crates/api/src/transport/oauth/flow.rs:38-60` + test at line 188 |
| `flow::exchange_code` does token exchange end-to-end | **High** | `crates/api/src/transport/oauth/flow.rs:79-118` |
| `validate_token_endpoint` rejects all localhost / private / loopback for `token_url` | **High** | `crates/api/src/transport/oauth/flow.rs:118-185` + tests |
| `nebula-api` already direct-deps `reqwest`, `nebula-credential`, `nebula-credential-runtime`, `wiremock` (dev) | **High** | `crates/api/Cargo.toml` |
| `OAuth2Pending` (Plane B) is the WRONG shape for Plane A persistence | **High** | Plane-B comment + OAuth2Pending field set vs OAuthStateRow field set |
| Integration tests need a test-util feature gate to bypass `validate_token_endpoint` | **High** | `validate_token_endpoint` is `pub fn`, called unconditionally by `exchange_code`. The in-crate test workaround at `flow.rs:330+` uses private `exchange_code_unchecked` which integration tests cannot reach. |
| No new `Cargo.toml` deps needed for PR-4 | **High** | reqwest + nebula-credential + nebula-credential-runtime + wiremock all already present |

All recon-3 claims are **High** confidence. No remaining unread medium-confidence files. **Recon is done.**

## 8. Required artifact patches (after recon-3)

| Artifact | Change |
|---|---|
| `design.md` D-9 | Replace with D-9-RECON3 (scope narrowing to authorize_url / redirect_uris only; token_url stays strict) |
| `design.md` D-11 | Replace with D-11-RECON3 (use `mint_pkce` + `flow::build_authorization_uri` + `OAuthStateRepo`; NO `OAuth2Credential::initiate_authorization_code` for Plane A) |
| `design.md` D-12 | Replace with D-12-RECON3 (use `flow::exchange_code` as the wrapper; direct `oauth_token_http_client()` only for userinfo GET) |
| `design.md` (new) D-14 | Add: `nebula-api` `test-util` feature gate exposing `exchange_code_unchecked` for integration tests |
| `design.md` "Resulting public-surface diff" | Remove `crates/api/src/transport/oauth/known.rs` as net-new file (it's referenced by D-5 but the known-provider defaults can live anywhere; D-5 is still right). Add `test-util` feature row to "Deps added" — actually it's not a dep, it's an exposure. Note: no new top-level `Cargo.toml` deps required. |
| `design.md` D-10 table | Update LOC estimates: PR-3 drops from ~180 to ~80; PR-2 grows from ~350 to ~380 (test-util feature wiring) |
| `spec.md` REQ-oauth-002 steps | Step 4 now says "`PgAuthBackend::start_oauth` calls `mint_pkce()` + builds `flow::AuthorizationUriRequest` + calls `flow::build_authorization_uri`". Step 5 says "persist `OAuthStateRow` via `OAuthStateRepo::create`" (NOT `pending_state_store`). |
| `spec.md` REQ-oauth-003 steps | Step 1 now says "`oauth_state_repo.consume_by_state_and_provider(state, provider)` returns Option<OAuthStateRow> (atomic UPDATE-RETURNING; existing replay defence)". Step 6 says "call `flow::exchange_code(TokenExchangeRequest)`". |
| `spec.md` REQ-compose-001 step 5 | Remove the reference to `AppState::pending_state_store`. Plane A uses `OAuthStateRepo` (the field is already on `PgAuthBackend`); no compose-root change needed for the store seam. |
| `spec.md` (new) REQ-test-util-001 | Add: `nebula-api` exposes `flow::exchange_code_unchecked` via a `test-util` feature gate. Production builds (no `test-util`) do NOT expose this surface. |
| `proposal.md` §7 chain | Update PR table: PR-3 LOC drops to ~80; total ~1,270 LOC; add test-util feature scope to PR-2 |
| `proposal.md` §6 risks | R-D4 (PR-3 worker reads pending_state.rs) DROPPED — recon-3 already did the read. R-D5 (PR-2 worker reads transport/oauth/flow.rs) DROPPED — same. Add **R-D6**: `test-util` feature gate must not silently leak into production builds. Mitigation: `release` feature build asserts `cfg!(not(feature = "test-util"))` at boot. |

These patches are smaller than the recon-2 ones because they only touch decision-level text, not whole sections.

## 9. Recommendation

Apply patches per §8 in order: `design.md` → `spec.md` → `proposal.md`. Then proceed to `sdd-tasks` with confidence — all medium-confidence claims are now resolved.

The change is meaningfully smaller and **more honest** than the original design suggested:
- We're NOT inventing PKCE generation (it's `mint_pkce()`).
- We're NOT inventing an authorize-URL builder (it's `flow::build_authorization_uri`).
- We're NOT inventing an OAuth state replay table (it's `plane_a_oauth_states` / `OAuthStateRepo`).
- We're NOT inventing a bounded token-endpoint HTTP client (it's `flow::exchange_code`).
- We're NOT inventing PKCE S256-only enforcement (it's compile-time via the enum).
- We're NOT inventing anti-SSRF token endpoint validation (it's `validate_token_endpoint`).

What we ARE inventing:
- `OAuthProvidersConfig` types + env binding (genuine new operator surface).
- `nebula-api` `test-util` feature gate (small, matches existing pattern).
- JWKS verification helper for `id_token` (~80 LOC, no in-tree precedent).
- `external_identities` table + repo + find-or-create user logic (genuine new data layer).
- Account-takeover defense for unverified Nebula email matching IdP-verified email (REQ-oauth-005 Scenario 5.2).
- `AuthError::ProviderNotConfigured` variant + 503 mapping (small additive).
- 14 RED integration tests against wiremock (this IS the bulk of PR-5).

---

## Result envelope

```yaml
status: recon-3-complete
executive_summary: |
  Read transport/oauth/{flow,http}.rs, storage/credential/pending.rs, storage/pg/oauth_state.rs,
  OAuthStateRepo trait, existing PgAuthBackend::start_oauth body, and crates/api/Cargo.toml.
  Major findings: (1) Plane A and Plane B are distinct in storage and trait surfaces; recon-2's
  D-11 used the WRONG store for Plane A. The right surface is OAuthStateRepo / OAuthStateRow,
  already wired into PgAuthBackend. (2) flow::build_authorization_uri and flow::exchange_code
  implement the full OAuth ceremony — PR-3 collapses to ~80 LOC. (3) mint_pkce() already exists
  and generates state + verifier + challenge. (4) validate_token_endpoint anti-SSRF defense
  rejects localhost + private + loopback for token URLs — D-9 must scope-narrow to authorize_url
  + redirect_uris only. (5) Integration tests need a `test-util` feature gate (D-14) exposing
  exchange_code_unchecked. (6) ALL workspace deps already in Cargo.toml — nothing to add.
  Net: 6 supersede/refine actions against design.md, plus D-14. Chain stays 5 PRs but PR-3
  drops to ~80 LOC and PR-2 grows to ~380 LOC. Total ~1,270 LOC. All medium-confidence
  claims resolved; recon is done.
artifacts:
  - openspec/changes/oauth-providers-from-operator-secrets/recon-3-flow-and-pending.md
next_recommended: apply recon-3 patches to design+spec+proposal, then sdd-tasks
risks:
  - R-D6 (NEW): nebula-api `test-util` feature must not leak into production builds. Mitigation: release-feature build-time assertion or CI gate that rejects test-util in release builds.
skill_resolution: none
```
