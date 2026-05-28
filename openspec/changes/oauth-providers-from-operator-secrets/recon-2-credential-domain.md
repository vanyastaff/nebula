# Recon-2 — nebula-credential* domain audit

- **Status**: appendix to `design.md`; flags material corrections.
- **Trigger**: user pushback that the design phase locked decisions without verifying credential-domain code.
- **Scope**: targeted reading of `crates/credential/`, `crates/credential-runtime/`, `crates/api/src/transport/oauth/`, ADR-0081, and the existing API OAuth controller stub.
- **Outcome**: **D-2, D-4, REQ-cred-001 must be superseded**. Significant scope reduction. Proposal's PR-chain shrinks from 6 PRs to ~5 PRs / ~1,220 LOC.

---

## 1. What design got wrong

Initial design assumed:

1. The operator's IdP-client credentials (`client_id`, `client_secret`, endpoints) belong in `CredentialService`.
2. The API needs a typed-decode seam (`get_for_oauth_provider`) to read them back.
3. `AppState::credential_service` needs to be wired (dyn-erased per D-2) so `PgAuthBackend` can call it.
4. The API needs to build the OAuth authorize URL + PKCE + state token from scratch.
5. The API needs a new `reqwest` HTTP path for the token endpoint.

**All five of these are wrong.** The codebase already has:

| Reality | Citation |
|---|---|
| The **API owns** the OAuth HTTP transport — this is an established architectural decision called "API-owned OAuth flow". | `crates/api/src/transport/oauth/http.rs:1-4` ("Moved from `nebula-credential` per API-owned OAuth flow incremental split"); `crates/credential/src/credentials/oauth2.rs:314,338,343,541,551,579,591,606,862` (15+ in-code references to the migration). |
| `OAuth2Credential::initiate_authorization_code(values: &FieldValues) -> Result<OAuth2Pending, ...>` already exists and generates PKCE verifier + anti-CSRF state + redirect_uri validation. | `crates/credential/src/credentials/oauth2.rs:650` (impl), tests at lines 1173-1257 cover happy path, missing redirect_uri, and CSRF state unguessability. |
| The PKCE method enum is **S256-only by construction** — `plain` is not representable. | `crates/credential/src/credentials/oauth2_config.rs:48-65` — `PkceMethod` enum has exactly one variant. |
| `OAuth2Pending::expires_in() = 10 min` covers the state TTL my design wanted to invent. | `crates/credential/src/credentials/oauth2.rs:~285` (line near the `PendingState for OAuth2Pending` impl). |
| `crates/api/src/transport/oauth/{flow,http,state}.rs` — the API-owned OAuth scaffolding already exists. `http.rs` is the bounded `reqwest` client for the token endpoint. `state.rs` exports the `OAuthProvider` enum. `flow.rs` orchestrates the ceremony. | Filesystem listing of `crates/api/src/transport/oauth/`. |
| `crates/api/src/domain/credential/oauth.rs` — "OAuth controller endpoints (API-owned OAuth flow rollout slice)" — controller stub already exists. | File header line 1. |
| `AppState::pending_state_store` — "OAuth pending state store (API-owned OAuth flow §4.2 — TTL ≤ 10 min, single-use)" — the pending store slot is already a first-class state field. | `crates/api/src/state.rs:267`. |
| ADR-0081 supersedes the older ADRs I cited (0042–0045, 0051, 0066, 0067) as the unified contract. My design referenced superseded ADRs. | `docs/adr/0081-m6-resource-credential-integration.md` (status: accepted, 2026-05-18). |
| `ValidatedCredentialBinding` pattern (D-4 precedent) is real — `crate-private` `new()` constructor, derived from `TenantFingerprint`. Confirmed but **the pattern is for slot bindings, not OAuth provider lookup** — applying it to OAuth was the wrong analogy. | `crates/credential-runtime/src/binding.rs:1-100`. |

## 2. The right mental model

There are **two distinct OAuth flows** in Nebula, and design conflated them:

### Flow A — Identity OAuth (this change, M3.1 closure)

Used for `POST /auth/oauth/{provider}/start` + `GET /auth/oauth/{provider}/callback`. Purpose: prove "this is Alice" via Google/GitHub/Generic IdP, then mint a Nebula session. **IdP tokens are discarded after callback** (D-7 still correct).

Operator's IdP-client credentials (`client_id`, `client_secret`, endpoint URLs, redirect URIs, scopes) are **infrastructure config**, not user credentials. They belong in `ApiConfig::auth.oauth.providers` next to `ApiConfig::smtp` and `ApiConfig::idempotency` — populated from env vars (`API_AUTH_OAUTH_<PROVIDER>_*`). This matches the SMTP precedent (`SmtpEmailConfig` + `API_SMTP_*` env vars).

`CredentialService` is **NOT involved in Flow A**.

### Flow B — Credential-as-OAuth (out of scope for this change)

A user wants to store their personal OAuth credential (Google Drive, GitHub API, …) so a workflow action can call those APIs on their behalf. The credential domain owns this: `OAuth2Credential::initiate_authorization_code` kicks off a flow, `PendingStateStore` holds the pending row, `Interactive::continue_resolve` finishes the token exchange and persists `OAuth2State` via the encrypted store. This is Plane B in the API layout (`crates/api/src/domain/credential/oauth.rs`).

Flow B is a **1.1 surface** per design D-7. This change does not touch it.

## 3. Required supersedes against `design.md`

### Supersede D-2 (dyn-erase `CredentialServiceErased`)

**Status**: SUPERSEDED. No new trait introduced.

`CredentialService` is not consumed by the M3.1 OAuth login flow. The existing concrete generic on `AppState::credential_service` is fine for the credential-CRUD endpoints (Plane B) that already consume it. M3.1 leaves that field untouched.

If a future change needs `CredentialService` consumed by handler code that does not know `<InMemoryStore, InMemoryPendingStore>` at the type level, that change can introduce dyn-erase then. Not here.

### Supersede D-4 (`OAuthProviderCredentialKey` newtype)

**Status**: SUPERSEDED. No newtype introduced.

No credential lookup happens in Flow A. The operator config carries `client_id` and `client_secret` directly as `SecretString` fields per the SMTP precedent.

### Supersede REQ-cred-001 (typed-decode seam in `nebula-credential-runtime`)

**Status**: SUPERSEDED. No new method on `CredentialService`.

The decode-from-snapshot machinery is moot when the values are already typed in `ApiConfig`. The credential-runtime crate stays untouched.

### Rewrite REQ-compose-001 (`compose.rs` wiring)

**Status**: REWRITTEN.

**Before** (design): `compose.rs` instantiates `CredentialService` and calls `with_credential_service` when OAuth providers are declared.

**After** (recon-2): `compose.rs` validates `ApiConfig::auth.oauth.providers` at boot:
- If non-empty: every provider config must validate (non-empty `client_id`, non-empty `client_secret`, ≥ 1 redirect URI, HTTPS or `localhost`-with-flag). Failures → `TransportInitError::OAuthProviderConfigInvalid { provider, reason }`. Boot fails closed.
- The validated config is threaded into `PgAuthBackend::new` (or the equivalent builder) so the backend can read it per request.
- `AppState::pending_state_store` (already present at `state.rs:267`) is used for OAuth state persistence — no new table needed; the existing `oauth_states` PG table backs the PG-backed implementation of `PendingStateStore`.

`AppState::credential_service` stays as it is today (None in default; populated by Plane B if/when needed).

### Supersede A.8, A.9 in proposal

- **A.8** (compose-root fails closed when OAuth declared without credential service) → reworded to "fails closed when any OAuth provider config is invalid". The credential-service link is gone.
- **A.9** (typed-decode seam in credential-runtime) → **deleted**. Replaced with A.9-NEW: "OAuth flow uses the existing `OAuth2Credential::initiate_authorization_code` helper for PKCE + state token generation; the API HTTP transport at `crates/api/src/transport/oauth/http.rs` performs the token endpoint exchange. No new public surface in `nebula-credential-runtime`."

### Rework R.1 (AppState::credential_service generic shape)

**Status**: DISAPPEARS as an OAuth concern.

The risk is still real for Plane B (credential CRUD), but Plane B is not in scope for M3.1. Drop from the proposal's risk register; Plane B's design will own it when it lands.

### Keep — D-1, D-3, D-5, D-6, D-7, D-8, D-9

These survive intact:

| Decision | Why still valid |
|---|---|
| **D-1** (config-map `[auth.oauth.providers.<name>]`) | The shape of the config-map is right; just the value type changes (inline `client_id`/`client_secret` instead of a `credential_id` reference). The map structure stays. |
| **D-3** (redirect_uris allow-list) | Still right; lives in `OAuthProviderConfig` directly. |
| **D-5** (Generic provider requires endpoints; known providers may override) | Still right; the `OAuthProviderEndpoints` struct lives in the config-row. |
| **D-6** (`ProviderNotConfigured` variant → HTTP 503) | Still right; semantics unchanged. |
| **D-7** (no IdP token persistence) | Even more strongly justified — Flow A is identity-only by construction. |
| **D-8** (`external_identities` table) | Still right; unrelated to CredentialService. |
| **D-9** (`oauth_allow_insecure_localhost` flag) | Still right; lives in `OAuthProviderConfig` or top-level `ApiConfig::auth.oauth`. |

## 4. New design decisions added by this recon

### D-11 — Reuse `OAuth2Credential::initiate_authorization_code`

**Status**: ADDED.

`PgAuthBackend::start_oauth(provider, redirect_uri)` SHALL:
1. Look up the validated `OAuthProviderConfig` from `ApiConfig`.
2. Build a `FieldValues` map matching `OAuth2Properties` (`client_id`, `client_secret`, `auth_url`, `token_url`, `grant_type = "authorization_code"`, `scopes`, `redirect_uri`).
3. Call `OAuth2Credential::initiate_authorization_code(&values)` to get a typed `OAuth2Pending` with PKCE verifier + CSRF state + redirect_uri.
4. Persist `OAuth2Pending` via the PG-backed `PendingStateStore` already wired in `AppState::pending_state_store`. The store's TTL (10 min) is enforced by the existing infrastructure.
5. Build the authorize URL from the pending fields (using a small helper that URL-encodes `client_id`, `redirect_uri`, `response_type=code`, `scope`, `state`, `nonce`, `code_challenge`, `code_challenge_method=S256`).
6. Return `OAuthStart`.

**Rationale**: zero duplication. PKCE generation, CSRF state generation, redirect_uri validation, and pending-state persistence are already implemented and tested. The API only builds the authorize URL on top.

### D-12 — Reuse `crates/api/src/transport/oauth/http.rs`

**Status**: ADDED.

`PgAuthBackend::complete_oauth(...)` SHALL use the bounded `reqwest::Client` already present at `crates/api/src/transport/oauth/http.rs` for both the token endpoint POST and the userinfo GET. The existing client policy (timeout, TLS, header sanitization) applies.

**Rationale**: zero duplication. The HTTP transport layer for OAuth already lives in the API per the established "API-owned OAuth flow" architecture decision. M3.1 should consume it, not reinvent it.

### D-13 — Token exchange step calls `Interactive::continue_resolve` for Flow A?

**Status**: DECIDED — **NO**.

`Interactive::continue_resolve` in `OAuth2Credential` is designed to persist the resolved `OAuth2State` as an encrypted credential row. Flow A discards the tokens after session mint — it must NOT persist them as a credential. So Flow A's token exchange runs the HTTP POST + JSON parse directly (in the API), then drops the parsed `OAuth2Token` after using its `access_token` to fetch userinfo. No `continue_resolve` call.

**Consequence**: PR-4 / PR-5 of the chain do their own JSON deserialization for the token response. Acceptable — the response shape is small (RFC 6749 §5.1) and the API already does this in the existing Flow B code paths under `crates/api/src/domain/credential/oauth.rs` (verify in PR-4 whether helpers can be lifted).

## 5. Revised PR chain (5 PRs, ~1,220 LOC)

| # | PR | Scope summary | LOC |
|---|---|---|---|
| 1 | **ADR** | Documents D-1 / D-3 / D-5 / D-6 / D-7 / D-8 / D-9 / D-11 / D-12 / D-13. Supersedes (note in ADR body) the design's D-2 / D-4 / REQ-cred-001. | ~140 |
| 2 | **Trait + config + redirect_uri** | `AuthBackend::start_oauth(provider, redirect_uri)` signature change; `OAuthProvidersConfig` + `OAuthProviderConfig` + `OAuthProviderEndpoints` types in `crates/api/src/config/oauth.rs`; env binding helpers; handler extracts `redirect_uri` from request; compose-root validates config at boot. | ~350 |
| 3 | **Real authorize URL via `initiate_authorization_code`** | `PgAuthBackend::start_oauth` and `InMemoryAuthBackend::start_oauth` use the credential-domain kickoff helper; pending-state persistence via `AppState::pending_state_store`; tests cover happy path + replay prevention via the existing store. | ~180 |
| 4 | **Token exchange + external_identities + find-or-create** | `PgAuthBackend::complete_oauth` performs the HTTP token exchange via `transport/oauth/http.rs`; userinfo fetch; `id_token` validation against JWKS if present; `external_identities` table migration + `PgExternalIdentityRepo`; first-login / existing-user logic (REQ-oauth-004 / -005 / -007); `InMemoryAuthBackend` symmetric impl. | ~420 |
| 5 | **Integration tests + docs + ROADMAP flip** | `crates/api/tests/oauth_provider_e2e.rs` against `wiremock` IdP; OAuth section in `crates/api/README.md`; ROADMAP §M3.1 checkbox flip. | ~280 |

**Total**: ~1,370 LOC across 5 PRs. Compared to original 6-PR forecast of ~1,400 LOC, the chain compresses by one PR with similar total LOC (the savings from "no credential-runtime work" offset by "no separate authorize-URL PR" — they merge). Each PR ≤ 800 LOC.

## 6. RED test anchors (revised per D-10)

| PR | RED tests committed first |
|---|---|
| 1 (ADR) | n/a |
| 2 (trait + config + redirect_uri) | `start_oauth_handler_extracts_redirect_uri`, `start_oauth_handler_returns_400_when_redirect_uri_missing`, `compose_root_fails_closed_when_oauth_provider_config_invalid`, `oauth_provider_config_rejects_empty_redirect_uris`, `oauth_provider_config_rejects_http_url_in_prod_mode` |
| 3 (authorize URL via initiate_authorization_code) | `start_oauth_emits_real_authorize_url_with_pkce_and_state`, `start_oauth_persists_pending_with_redirect_uri`, `start_oauth_returns_provider_not_configured_when_absent`, `start_oauth_rejects_non_allowlisted_redirect_uri` |
| 4 (token exchange + identities) | `complete_oauth_succeeds_with_valid_code`, `complete_oauth_rejects_replay`, `complete_oauth_rejects_expired_pending`, `complete_oauth_rejects_mismatched_state`, `complete_oauth_rejects_redirect_uri_mismatch`, `complete_oauth_handles_idp_token_endpoint_500`, `complete_oauth_rejects_malformed_token_response`, `complete_oauth_rejects_id_token_signature_invalid`, `complete_oauth_rejects_id_token_nonce_mismatch`, `complete_oauth_token_endpoint_timeout`, `complete_oauth_creates_user_on_first_login_verified_email`, `complete_oauth_rejects_first_login_unverified_email`, `complete_oauth_links_existing_user_on_email_match`, `complete_oauth_rejects_link_for_unverified_nebula_email` |
| 5 (docs) | n/a (prior PRs hold tests; this PR adds README doctest snippets) |

**24 RED tests total** (same count as original D-10; one extra for the new "rejects_non_allowlisted_redirect_uri" case, one removed for the now-irrelevant typed-decode seam).

## 7. What changes in each prior artifact

| Artifact | Required change |
|---|---|
| `proposal.md` §3 (goals) | Goal 4 (typed-decode seam) → reword: "OAuth flow uses existing `OAuth2Credential::initiate_authorization_code` helper; no new public surface in nebula-credential-runtime." |
| `proposal.md` §5 (acceptance criteria) | A.8 reworded (compose fails closed on invalid OAuth config); A.9 reworded (no typed-decode seam — reuse `initiate_authorization_code` + `transport/oauth/http.rs`); rest intact. |
| `proposal.md` §6 (risks) | R.1 deleted (not an OAuth concern anymore). R-D1 / R-D2 (from design) deleted (no newtype, no dyn-erase trait). R.10 (subagent dispatch) stays. |
| `proposal.md` §7 (chain) | 6-PR table → 5-PR table per §5 of this recon-2. |
| `spec.md` REQ-cred-001 | DELETED. |
| `spec.md` REQ-compose-001 | REWRITTEN — boot fails closed on invalid OAuth provider config, no CredentialService wiring requirement. |
| `spec.md` REQ-oauth-002 | Rewrite Scenario 2.2 — PKCE plain rejection is now compile-time (the enum has one variant); config-load can't even parse a plain entry. |
| `design.md` D-2, D-4 | Add SUPERSEDED-BY-RECON-2 block at the top of each section pointing at this recon. |
| `design.md` D-10 | Update per-PR RED test anchors to match this recon's §6 table. |
| `design.md` §"Resulting public-surface diff" | Drop `CredentialServiceErased` trait and `OAuthProviderCredentialKey` newtype from "New types"; drop `get_for_oauth_provider` from "Modified surfaces"; keep all the additions to `crates/api/src/config/oauth.rs` and `crates/api/src/transport/oauth/known.rs`. |

## 8. Confidence check

| Claim | Confidence |
|---|---|
| API owns the OAuth HTTP transport (NOT credential crate) | **High** — 15+ in-code citations + dedicated `crates/api/src/transport/oauth/` subtree |
| `initiate_authorization_code` exists and handles PKCE + state + redirect_uri | **High** — verified at `oauth2.rs:650` with three+ unit tests at lines 1173-1257 |
| `AppState::pending_state_store` is the right home for OAuth pending state (not a custom `oauth_states`-only path) | **Medium-high** — verified the slot exists at `state.rs:267`; have not yet read the concrete PG-backed `PendingStateStore` impl to confirm the `oauth_states` table is its backend, but the wiring is explicit. PR-3 worker should verify by reading `crates/storage/src/pg/pending_state.rs` (or wherever the impl lives) before writing the test. |
| The existing OAuth flow ceremony at `transport/oauth/flow.rs` covers what M3.1 needs | **Medium** — only read the file header. PR-2 worker should read the full file before deciding whether to extend it or add a parallel flow for the identity-login case. |
| Operator IdP-client config belongs in `ApiConfig`, not `CredentialService` | **High** — matches SMTP / idempotency / JWT-secret precedent in `crates/api/src/config/sub.rs`; no in-tree pattern of operator infra creds living in CredentialService. |
| ADR-0081 is the canonical contract; the older ADRs I cited are superseded | **High** — verified ADR-0081 header explicitly lists 0042-0045, 0051, 0066, 0067 as superseded. |
| Token exchange should NOT route through `Interactive::continue_resolve` (D-13) | **Medium** — verified `continue_resolve` is designed to persist `OAuth2State` as a credential row. For the identity flow (D-7 discard), bypassing is correct. PR-4 worker confirms by reviewing whether the existing API OAuth controller stub at `crates/api/src/domain/credential/oauth.rs` already encodes this distinction. |

## 9. Recommendation

1. **Patch the prior artifacts** per §7 in this order: `design.md` (add SUPERSEDED blocks) → `proposal.md` (update goals/criteria/risks/chain) → `spec.md` (delete/rewrite REQ-cred-001 / REQ-compose-001 / REQ-oauth-002 scenario 2.2).
2. **Read the two remaining medium-confidence files** before `sdd-tasks`:
   - `crates/api/src/transport/oauth/flow.rs` (full content)
   - `crates/storage/src/pg/pending_state.rs` (or equivalent `PendingStateStore` PG impl)
3. **Then `sdd-tasks`** decomposes the revised 5-PR chain.

The user's review of these revisions before `sdd-tasks` is mandatory — this recon material is not a minor patch, it's a partial re-architecture of the change.

---

## Result envelope

```yaml
status: recon-appendix-complete
executive_summary: |
  Read crates/credential/src/credentials/oauth2.rs, credential-runtime/src/service.rs,
  binding.rs, ADR-0081, and crates/api/src/transport/oauth/* listings. Material findings:
  (1) the API already owns OAuth HTTP transport per established architecture decision
  ("API-owned OAuth flow"); (2) OAuth2Credential::initiate_authorization_code already
  generates PKCE + CSRF state + redirect_uri validation, tested; (3) PKCE S256-only is
  a compile-time invariant via single-variant enum; (4) AppState::pending_state_store
  is the established slot for OAuth pending state; (5) ADR-0081 supersedes 0042-0045,
  0051, 0066, 0067 — design referenced superseded ADRs. Net impact: D-2 (dyn-erase),
  D-4 (newtype), REQ-cred-001 (typed-decode seam) all SUPERSEDED. Compose-root wiring
  rewritten. Two new decisions: D-11 (reuse initiate_authorization_code) + D-12 (reuse
  transport/oauth/http.rs) + D-13 (do NOT route Flow A through continue_resolve).
  PR chain shrinks from 6 to 5. Two medium-confidence files remain unread; PR-2 and
  PR-4 workers must read them. Recommendation: patch prior artifacts before sdd-tasks.
artifacts:
  - openspec/changes/oauth-providers-from-operator-secrets/recon-2-credential-domain.md
next_recommended: patch-design+proposal+spec, then sdd-tasks
risks:
  - One medium-confidence claim about PendingStateStore PG backend mapping to oauth_states table — worker verifies in PR-3
  - Another medium-confidence claim about transport/oauth/flow.rs sufficiency — worker verifies in PR-2
skill_resolution: none
```
