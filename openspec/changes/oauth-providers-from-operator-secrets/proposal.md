# SDD Proposal — OAuth providers from operator secrets

- **Change slug**: `oauth-providers-from-operator-secrets`
- **Origin**: ROADMAP §M3.1 residual; explicitly carved out as its own SDD plan in `docs/plans/2026-05-26-001-feat-api-m3.1-followup-wave-plan.md` §"Out of scope" (PR-C).
- **Status**: proposed (revised 2026-05-27 by recon-2 + recon-3 + recon-4)
- **Date**: 2026-05-27
- **Preflight context**: interactive mode · artifact store both (OpenSpec + Engram) · chained PR strategy auto-forecast · review budget 800 changed lines.
- **Scope (locked)**: **Plane A** — identity OAuth login into the Nebula platform. NOT Plane B (workflow node credentials).
- **Predecessor artifacts**: `explore.md`, `recon-2-credential-domain.md`, `recon-3-flow-and-pending.md`, `recon-4-n8n-and-rust-ecosystem.md`.
- **Recon supersedes (cumulative)**:
  - **recon-2**: A.9 (typed-decode seam) deleted; CredentialService not consumed in Plane A.
  - **recon-3**: chain compresses to 5 PRs; flow.rs helpers (build_authorization_uri, exchange_code) reused; OAuthStateRepo (NOT pending_state_store) is the Plane A surface.
  - **recon-4**: 3 simplifications adopted — (a) auto-derive `redirect_uri` from `ApiConfig::public_url` (drops allow-list); (b) OIDC discovery primary, Manual fallback for OAuth2-only (GitHub); (c) hardcoded `openid email profile` for OIDC. JWKS validation deferred to 1.1.
- Search for "🟥 RECON-2/3/4" markers in this file and in `design.md` / `spec.md` for the revision points.

---

## 1. Problem statement

Nebula advertises an OAuth-backed identity surface (`POST /auth/oauth/{provider}/start`, `GET /auth/oauth/{provider}/callback`) and ships a strongly-typed `OAuthProvider` enum, an `oauth_states` PG table for replay protection, and a session-minting pipeline that already round-trips for password and MFA flows. However:

1. `PgAuthBackend::complete_oauth` (`crates/api/src/domain/auth/backend/pg.rs:1079-1110`) **explicitly returns `AuthError::NotImplemented(...)`**. `InMemoryAuthBackend::complete_oauth` does the same.
2. `PgAuthBackend::start_oauth` mints state + PKCE rows but returns a **synthetic** authorize URL (`https://nebula.local/...`) — there is no real IdP authorize-URL construction.
3. `AppState::credential_service` (`crates/api/src/state.rs:259`) is declared as a slot but `apps/server/src/compose.rs` never calls `with_credential_service`. The slot is always `None` at runtime.
4. `AuthBackend::start_oauth` does not accept `redirect_uri` (the operator's externally-visible callback URL).
5. `CredentialService::get` returns an opaque `CredentialSnapshot` — there is no typed-decode seam to `OAuth2Config`.
6. There is no operator-facing configuration surface for OAuth providers (no `[auth.oauth.providers]` config section, no `API_AUTH_OAUTH_*` env binding).

The combined effect is a **canon §4.5 false-capability violation**: the OpenAPI spec, the route mount, and the trait shape all imply an OAuth flow that simply does not work. Operators cannot wire any IdP. This is the final M3.1 honesty gap.

## 2. Goals

A change that ships **all six** of these in a chained PR series:

1. Real authorization-code + PKCE flow against any compliant OAuth 2.1 / OIDC IdP, mediated by operator-supplied client_id + client_secret + scopes + redirect URI.
2. Production wiring of `CredentialService` into `AppState` via `compose.rs`, with fail-closed posture when the operator declared OAuth providers but the credential store is unprovisioned (no silent fallback to in-memory).
3. Trait-level `redirect_uri` plumb so the handler-supplied callback survives the round trip through `oauth_states` and is verified on completion.
4. **🟥 RECON-2 REVISED**: OAuth flow reuses the existing `OAuth2Credential::initiate_authorization_code` kickoff helper (PKCE + CSRF state generation + redirect_uri validation, already implemented and tested) and the existing `crates/api/src/transport/oauth/http.rs` bounded HTTP client. **No new public surface in `nebula-credential-runtime`.** Operator IdP-client credentials live in `ApiConfig::auth.oauth.providers` as infrastructure config (matching the `SmtpEmailConfig` precedent), NOT in `CredentialService`.
5. End-to-end PG integration tests against a `wiremock` mock IdP, covering: happy-path session mint, replay rejection, state expiry, state mismatch, redirect_uri mismatch, IdP token-endpoint failure, PKCE round-trip, first-login user creation, existing-user linking by verified-email match, provider-not-configured.
6. ROADMAP §M3.1 final checkbox flipped; per-crate observability triple (`thiserror` variant + `tracing` span + invariant check) on every new boundary.

## 3. Non-goals

This change **MUST NOT**:

- Ship vendor-specific provider packs (Google / GitHub / Microsoft / Apple builders). That is M12.3 `nebula-credential-builtin` scope. This change ships only the **generic** authorization-code + PKCE flow that any operator can configure for any IdP.
- Implement OAuth refresh-token rotation as a session-lifecycle concern. Rotation is owned by `nebula-credential-runtime` rotation fan-out (ADR-0067, #688/#690) which already shipped. IdP-issued access/refresh tokens MUST be discarded after the Nebula session is minted (see §6 Risk-4 for the explicit decision).
- Implement device-code or client-credentials grants. Authorization-code with PKCE only.
- Ship a custom claim-policy engine. Standard claim validation (`iss`, `aud`, `exp`, `iat`, `nonce`) only.
- Touch M3.6 (shift-left validation), M3.3 webhook follow-ups, M5 (plugin ABI ADR), or M14.4 (PluginCtx broker). Each is its own SDD plan.
- Block on M12.3 (credential-builtin vendor packs). This change adds at most a generic `Oauth2ProviderCredential` newtype helper inside `nebula-api` or `nebula-credential-runtime`; it does not migrate `nebula-credential-builtin`.
- Persist IdP-side state beyond the existing `oauth_states` replay row + (optional) credential row chosen by the design ADR.

## 4. Alternatives considered (operator-config conventions)

The design phase will close this with an ADR. The proposal documents the three viable shapes so the design surface is bounded.

### Option A — Config-map keyed by `OAuthProvider`

`apps/server` config + env vars carry per-provider rows:

```toml
[auth.oauth.providers.google]
credential_id = "cred_01HX...G7"
scopes        = ["openid", "email"]
redirect_uri  = "https://nebula.example.com/auth/oauth/google/callback"
```

- **Pros**: explicit; one stable name → one credential ID; rotating a credential is a credential-store mutation only (no config edit); `CredentialService::get` stays id-keyed (current public surface); shipping path mirrors `IdempotencyApiConfig`.
- **Cons**: config-map duplicates the `OAuthProvider` enum keys (typo risk: `gooogle`); per-provider scope/redirect divergence pushed into config; adding a `Generic` IdP needs authorize/token/userinfo URLs in the config row too.
- **Touch**: `crates/api/src/config/sub.rs` adds `OAuthProvidersConfig`; `crates/api/src/config/env.rs` adds parsers; `compose.rs` threads it into `PgAuthBackend::new`.

### Option B — Credential name convention (`oauth2/<provider>`)

The backend looks up the credential by a stable derived name (`format!("oauth2/{provider}")`) via a new `CredentialService::get_by_name(scope, name)`.

- **Pros**: zero config-schema growth; rotation is a credential-store mutation; one credential per provider, named consistently.
- **Cons**: extends `CredentialService` public surface with `get_by_name` (needs a `CredentialName` index — additive but non-trivial); naming convention is a magic string that must be enforced; fails silently if operator forgets the prefix; `scopes` and `redirect_uri` still need a home (probably the credential payload itself).
- **Touch**: `nebula-credential-runtime` (additive `get_by_name`); `nebula-credential` or `nebula-api` (convention helper / newtype); no config schema growth; `compose.rs` still wires `CredentialService` but no provider-map.

### Option C — New `CredentialKind::OAuth2Provider { provider }`

Add a kind-tagged credential variant. Lookup is `CredentialService::list_by_kind(OAuth2Provider { provider: "google" })`.

- **Pros**: most type-safe long-term; multiple credentials per provider (dev / staging / prod) with explicit selection; aligns with the §M12.3 generic-credential-core direction (`GenericOAuth2`, `GenericPat`, `GenericApiKey`, `GenericBasicAuth`).
- **Cons**: largest blast radius — extends `CredentialKind` public enum; adds a new query method (`list_by_kind`); needs a migration for existing credentials; couples M3.1 closure to M12.3 sequencing.
- **Touch**: `nebula-credential` (variant + accessor + migration); `nebula-credential-builtin` (`GenericOAuth2` generic builder); `nebula-credential-runtime` (`list_by_kind`); `nebula-api` consumer.

### Recommendation deferred to `sdd-design`

The user chose "open — design phase decides" during the explore review. The design ADR weighs blast radius vs. ergonomics vs. M12.3 coupling and picks one. Proposal stays convention-agnostic — acceptance criteria below are written so they hold under any of A/B/C.

## 5. Acceptance criteria (will move to `spec.md`)

A.1 — `PgAuthBackend::complete_oauth` returns `Ok(Session)` (no longer `AuthError::NotImplemented`) when the operator has provisioned a valid provider configuration and the IdP returns a well-formed token + userinfo response. Verified by `crates/api/tests/oauth_provider_e2e.rs::complete_oauth_succeeds_with_valid_code`.

A.2 — `start_oauth(provider, redirect_uri)` constructs a real IdP authorize URL with PKCE parameters (`code_challenge`, `code_challenge_method=S256`), preserves `redirect_uri` in the `oauth_states` row, and returns `OAuthStart { authorize_url, state, expires_at }`. The synthetic `https://nebula.local/...` URL is gone. Verified by `start_oauth_emits_real_authorize_url`.

A.3 — **🟥 RECON-4 REVISED**: `complete_oauth` rejects all of: replayed code (state row consumed twice) with `AuthError::InvalidToken`; expired state with `AuthError::InvalidToken`; mismatched state with `AuthError::InvalidToken`; **`public_url` change mid-flow** (derived `redirect_uri` differs from row's persisted `redirect_uri`) with `AuthError::OAuthFailed { cause: "public_url_changed_mid_flow" }`; IdP token-endpoint error (any non-2xx) with `AuthError::OAuthFailed` and a redacted body in the error span. id_token JWKS signature validation rejection paths are deferred to 1.1 (see R-D7).

A.4 — `start_oauth` returns `AuthError::OAuthFailed` (or a new typed variant chosen in design — e.g. `ProviderNotConfigured`) when the operator has not configured the requested provider. The mapping in `AuthError → ApiError` lands the response as HTTP 502 `UpstreamError` or HTTP 503 `ServiceUnavailable` per the design choice. Verified by `start_oauth_returns_provider_not_configured`.

A.5 — First-login flow (the IdP userinfo `email` is unknown to Nebula) creates a local user row with `email_verified = true` and the IdP `sub` linked into the `external_identities` table (or equivalent — design-phase). Verified by `complete_oauth_creates_user_on_first_login`.

A.6 — Existing-user flow (the IdP userinfo `email` matches an existing user with `email_verified = true`) links the external identity onto the existing user; no duplicate user row created. Verified by `complete_oauth_links_existing_user_on_email_match`.

A.7 — Existing-user flow with an UN-verified Nebula email rejects the link (returns `AuthError::EmailNotVerified`) to defend against account-takeover via the OAuth path. Verified by `complete_oauth_rejects_link_for_unverified_email`.

A.8 — **🟥 RECON-2 REVISED**: `compose.rs` validates `ApiConfig::auth.oauth.providers` at boot. Every declared provider must have a non-empty `client_id`, non-empty `client_secret`, ≥ 1 redirect URI, HTTPS endpoints (or `localhost` only when `oauth_allow_insecure_localhost = true` AND the binary is not built with the `release` feature). Failures → `TransportInitError::OAuthProviderConfigInvalid { provider, reason }`. Boot fails closed. **No `CredentialService` wiring required.** Verified by `compose_root_fails_closed_when_oauth_provider_config_invalid` test.

A.9 — **🟥 RECON-2 REVISED**: OAuth flow uses (a) `OAuth2Credential::initiate_authorization_code` at `crates/credential/src/credentials/oauth2.rs:650` for PKCE + CSRF state + redirect_uri validation, (b) `AppState::pending_state_store` at `crates/api/src/state.rs:267` for pending-state persistence, and (c) `crates/api/src/transport/oauth/http.rs` for the bounded `reqwest::Client` against the token endpoint. **No new public surface in `nebula-credential-runtime`. No new HTTP client.** Verified by integration tests that observe the round-trip without inspecting credential-runtime internals.

A.10 — `AuthBackend::start_oauth(provider, redirect_uri)` trait signature is updated. All implementors (`PgAuthBackend`, `InMemoryAuthBackend`, every mock in `crates/api/tests/*`) compile. The handler `crates/api/src/domain/auth/handler.rs:390-421` passes `redirect_uri` from the request query string. Verified by `cargo nextest run --workspace --no-fail-fast`.

A.11 — Observability triple on every new boundary:
- Typed `AuthError` variant for every new failure path (no `String` error variants beyond the existing `OAuthFailed(String)` which collapses IdP-side messages); new variants like `ProviderNotConfigured` go through the `AuthError → ApiError` exhaustive-mapping audit added in #753.
- `tracing::Span` with structured fields on `start_oauth` and `complete_oauth` (provider, redirect_uri host-only, state token redacted, IdP HTTP status on error). No `client_secret`, no full `code`, no full `state_token` in any event.
- `debug_assert!` on the PKCE state-row invariants (consumed-once, TTL-respected) inside the PG transaction.

A.12 — ROADMAP §M3.1 final checkbox flipped to `[x]`. `cargo deny check` green. `task dev:check` green. The §4.5 grep over `crates/api/src/domain/auth/` returns zero `NotImplemented` hits referencing OAuth.

A.13 — **🟥 RECON-4 REVISED**: Per-crate README quality: `crates/api/README.md` gets a new "OAuth provider configuration" section showing the operator-facing minimal config with two examples — one `Oidc { discovery_url }` (Google) and one `Manual { ... }` (GitHub) — plus the env-var binding pattern. The release-notes blurb from D-16 about JWKS validation deferral is also included. The chained-PR squash-merged commits are referenced.

A.14 — Chained-PR boundary respected. Each PR in the 6-PR chain (per §7 below) is ≤ 800 changed lines. The last PR (PR-6) is the only one that flips the ROADMAP checkbox.

## 6. Risks

R.1 — **🟥 RECON-2 DROPPED**. `AppState::credential_service` is not consumed by Flow A (identity login). The field stays as it is today. Plane B (credential CRUD) may need this risk in a future change; M3.1 does not.

R.6 — **🟥 RECON-4 DROPPED**. `redirect_uri` shape (single vs allow-list) is moot: recon-4 auto-derives from `ApiConfig::public_url` (`API_PUBLIC_URL` env). No allow-list, no config field. Multi-environment operators register multiple OAuth clients per IdP (one per Nebula instance).

R-D7 — **🟥 RECON-4 NEW**. 1.0 ships **without** id_token JWKS signature validation. The userinfo endpoint over TLS is the source of truth for the user's `email` + `sub`. Mitigation:
- `validate_token_endpoint` enforces HTTPS + non-localhost on the userinfo URL (same SSRF policy as the token URL).
- The token + userinfo endpoints share the same `oauth_token_http_client()` instance with the same TLS posture.
- Release-notes blurb in `crates/api/README.md` documents the deferral.
- 1.1 follow-up plan adds JWKS validation via the `openidconnect` crate or hand-rolled with the existing workspace `jsonwebtoken` dep.
- An operator who requires strict OIDC compliance now should track the 1.1 follow-up issue (filed as part of PR-5).

R.2 — **`wiremock` dev-dep addition**. Not currently in workspace. License (Apache-2.0 / MIT) compatible with `deny.toml`. Added only as `[dev-dependencies]` of `nebula-api` — does not affect production binary size.

R.3 — **PKCE for confidential clients**. Spec mandates `code_challenge` even when `client_secret` is configured (defense in depth). Some IdPs (rare, mostly legacy) reject `code_verifier` for confidential clients. Mitigation: per-provider `pkce_required: bool` config field with default `true`; the design ADR records the rationale.

R.4 — **Whether to persist IdP access/refresh tokens**. Decision: **NO for 1.0**. The OAuth flow mints a Nebula session and discards the IdP tokens. Persisting IdP tokens would tie session lifetime to IdP token lifetime, drag the credential rotation fan-out (#688/#690) into the session path, and re-open the "downstream calls using IdP creds" surface (deferred to 1.1). The 1.0 contract is: OAuth proves identity, then Nebula owns the session.

R.5 — **Session lifetime vs IdP token lifetime divergence**. The IdP's `expires_in` does NOT bound Nebula's session TTL. Mitigation: documented explicitly in the spec (acceptance criterion implicit in A.5/A.6) and in `crates/api/README.md` (A.13). README also explains the trade-off (Nebula's session is independent; if the operator wants stricter IdP-tied sessions, that's a 1.1 surface).

R.6 — **`redirect_uri` host validation and allow-listing**. An operator may need multiple redirect URIs (dev / staging / prod). Decision deferred to design: `redirect_uri: String` (one value) vs. `redirect_uris: Vec<String>` (allow-list with caller-supplied selector). Either is fine; design-ADR records the choice and `start_oauth` enforces membership.

R.7 — **`OAuth2Provider` enum exhaustiveness for `Generic` IdPs**. The enum is finite. A `Generic` provider variant needs authorize_url + token_url + userinfo_url + jwks_url in the config row. Decision: ship `Generic` in 1.0 — the entire point of "operator secrets" is to support any IdP without code changes. The design ADR documents the `Generic` row schema.

R.8 — **CSRF cookie binding on the callback**. PR `b2a59ea8` shipped CSRF enforcement on credential write paths and `/auth/mfa/*`. The OAuth callback must enforce the cookie/state binding (verify the state cookie's HMAC matches the `oauth_states` row) — Mitigation: A.3 covers this via the "mismatched state" test case.

R.9 — **Token-endpoint timeout / retry policy**. The IdP token endpoint is an external HTTP call. Mitigation: configurable `oauth_token_timeout_ms` (default 5000 ms); single attempt (no retry — retries can replay the authorization code, which is single-use). Document explicitly that the code-exchange is non-idempotent and MUST NOT be retried.

R.10 — **Subagent dispatch reliability**. The orchestrator hit two consecutive subagent failures (gemini-3.1-pro-preview empty output; openrouter sonnet-4.5 no API key) during the explore phase. Mitigation: for `design` and `tasks`, prefer the cursor-agent default model on a tighter prompt; fall back to inline if dispatch fails twice. This does not affect the change itself but documents an orchestration risk.

## 7. Chained PR forecast (REVISED per recon-2 + recon-3 + recon-4 — 5 PRs)

| # | PR | Scope summary | LOC | Reviewer focus |
|---|---|---|---|---|
| 1 | **ADR-only** | ADR documenting all live D-* decisions (D-1, D-3-RECON4, D-5-RECON4, D-6, D-7, D-8, D-9-RECON3, D-11-RECON3, D-12-RECON3, D-13, D-14, D-15, D-16) and SUPERSEDED notes against the historical D-2, D-4, REQ-cred-001, D-3 allow-list, D-11/D-12 prior phrasings. Includes the R-D7 release-notes blurb. No code change. | ~180 | ARCH owners; no code review surface. |
| 2 | **Trait + config + compose validation + test-util feature** | `AuthBackend::start_oauth(provider, redirect_uri)` signature change (redirect_uri **auto-derived** from `ApiConfig::public_url` by the handler, NOT user-supplied); `OAuthProvidersConfig` types per recon-4 §4 (Oidc vs Manual tagged union); env binding helpers; compose-root validation; `nebula-api` `test-util` feature flag exposing `exchange_code_unchecked`. | ~330 | api + apps/server CODEOWNERS. |
| 3 | **Real authorize URL via `flow::build_authorization_uri` + OIDC discovery** | `PgAuthBackend::start_oauth` derives redirect_uri from public_url; for `Oidc` endpoints, fetches discovery doc (cached via D-15); for `Manual` uses explicit endpoints; calls `mint_pkce()` + `flow::build_authorization_uri`; persists `OAuthStateRow` via existing `OAuthStateRepo` (the slot is already on `PgAuthBackend`). | ~150 | api CODEOWNERS. |
| 4 | **Real `complete_oauth` + userinfo + external_identities + find-or-create user** | `PgAuthBackend::complete_oauth` uses `oauth_state_repo.consume_by_state_and_provider`; verifies derived `redirect_uri` matches row; uses `flow::exchange_code` for token exchange; userinfo GET via `oauth_token_http_client()` with Bearer header; first-login / existing-user logic (REQ-oauth-004/-005/-007); `external_identities` table migration + `PgExternalIdentityRepo`; `InMemoryAuthBackend` parallel impl. **NO id_token JWKS validation (D-16 defer)**. **Removes `NotImplemented`**. Wiremock-based integration tests. | ~330 | api + storage CODEOWNERS. |
| 5 | **Docs + ROADMAP flip + 1.1 follow-up issue** | `crates/api/README.md` OAuth section (Oidc + Manual examples); release-notes blurb on JWKS deferral; ROADMAP §M3.1 final checkbox flip; file 1.1 follow-up issue for JWKS validation. | ~240 | docs reviewer. |

**Sum**: ~1,230 LOC across 5 PRs. Each PR is ≤ 800 LOC. Dependencies are strict (1 → 2 → 3 → 4 → 5). PR-3 is small (~150 LOC) because `flow::build_authorization_uri` + `mint_pkce()` + `OAuthStateRepo` already exist. PR-4 stays largest because of JWKS-free OAuth flow wire-up + new `external_identities` table + user resolution logic.

## 8. Strict TDD evidence plan

Per `openspec/config.yaml`, strict TDD is active. Test runner: `cargo nextest run --workspace --no-fail-fast`.

For each PR (especially 4 / 5 / 6), the worker must produce:

- **RED**: failing test committed first (one per acceptance criterion touched by that PR).
- **GREEN**: minimal implementation passing the test.
- **TRIANGULATE**: one or more boundary / edge-case tests (e.g. PKCE method `plain` is rejected; userinfo response without `email` is rejected; token response with `expires_in: 0` is rejected).
- **REFACTOR**: cleanup pass under green; documented in the PR description.

Test files:

- New: `crates/api/tests/oauth_provider_e2e.rs` (10 RED tests per §5).
- New: `crates/credential-runtime/tests/oauth_typed_decode.rs` (typed decode + sentinel invariant).
- Extend: `crates/api/tests/auth_pg_e2e.rs` — no, this stays as the lifecycle test; OAuth gets its own file.

Mock IdP via `wiremock = "0.6"` added under `[dev-dependencies]` of `nebula-api` only.

## 9. Out-of-scope follow-ups

These are explicitly NOT in this change but are likely 1.1 surfaces:

- Vendor packs (Google / GitHub / Microsoft / Apple) → M12.3.
- Storing & using IdP-issued tokens for downstream API calls → 1.1 ("OAuth-as-credential" surface).
- Custom claim-policy engine → 1.1.
- Device-code + client-credentials grants → 1.1+.
- Session-IdP token lifetime binding (operator-opt-in) → 1.1.
- Webhook-based OAuth state propagation (e.g. PKCE state stored on a per-tenant signed cookie instead of DB) → not planned.

## 10. Next phase

**`sdd-spec`** — turn §5 acceptance criteria into formal spec deltas (OpenSpec format: one requirement per criterion, BDD-style scenarios). Pass this proposal as input. Expected output: `openspec/changes/oauth-providers-from-operator-secrets/spec.md`.

After spec is approved, `sdd-design` produces the ADR resolving §4 (operator config convention) and §6 R.1 (generic vs dyn) and §6 R.6 (single vs allow-list redirect_uri). Then `sdd-tasks` decomposes the 6-PR chain.

---

## Result envelope

```yaml
status: proposed
executive_summary: |
  Close M3.1 final §4.5 honesty gap: ship real OAuth authorization-code + PKCE flow
  against any IdP, wired by operator config + secrets. 14 acceptance criteria
  (A.1–A.14), 10 risks (R.1–R.10) with mitigations. Operator-config convention
  (A/B/C) left to design ADR per user choice. 6-PR chain confirmed (ADR →
  trait+redirect_uri → config+compose → authorize URL → token exchange →
  tests+docs), each ≤ 800 LOC, total ~1,400 LOC. Strict TDD with wiremock mock IdP.
  Non-goals explicit: no vendor packs (M12.3), no IdP token persistence, no
  refresh-rotation in session path, no device-code/client-credentials grants.
artifacts:
  - openspec/changes/oauth-providers-from-operator-secrets/explore.md
  - openspec/changes/oauth-providers-from-operator-secrets/proposal.md
next_recommended: sdd-spec
risks:
  - AppState::credential_service generic shape (R.1) — design must pick
  - wiremock dev-dep addition (R.2)
  - PKCE for confidential clients (R.3)
  - decision to NOT persist IdP tokens (R.4) — documented + locked
  - redirect_uri allow-list shape (R.6) — design picks
  - subagent dispatch reliability (R.10) — orchestration risk
skill_resolution: none
```
