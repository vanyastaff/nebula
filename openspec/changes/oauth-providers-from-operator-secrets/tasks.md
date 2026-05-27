# SDD Tasks — OAuth providers from operator secrets

- **Change slug**: `oauth-providers-from-operator-secrets`
- **Status**: tasks-draft (planning lifecycle final phase)
- **Date**: 2026-05-27
- **Predecessors**: explore + proposal + spec + design + recon-2 + recon-3 + recon-4
- **Strict TDD**: active (per `openspec/config.yaml`). Test runner: `cargo nextest run --workspace --no-fail-fast`. Each PR commits RED tests FIRST, then GREEN implementation, then TRIANGULATE, then REFACTOR.
- **Review budget**: 800 LOC per PR (auto-forecast chained — user-confirmed).

> **Task tags**: `RED` (failing test committed first), `GREEN` (minimal impl to pass), `TRIANGULATE` (boundary/edge test), `REFACTOR` (cleanup), `SCAFFOLD` (file/struct/migration creation without tests), `DOC` (markdown/comment only), `GATE` (verification checkpoint, runs `task dev:check`).
>
> **Estimated LOC per task**: counts lines added + removed. Targets are guidance, not gates. The hard gate is the per-PR ≤ 800 LOC budget at squash-merge time.

---

## PR-1 — ADR-only (target ~180 LOC; ARCH owners review)

**Goal**: A single `docs/adr/NNNN-oauth-identity-providers-from-secrets.md` ADR documenting every live decision and the SUPERSEDED chain. No code change. Squash-merges before PR-2 starts.

| # | Tag | Description | LOC |
|---|---|---|---|
| T1.1 | DOC | Pick next ADR number; `ls docs/adr/` shows highest existing is 0084. Use **0085**. | 0 |
| T1.2 | DOC | Write `docs/adr/0085-oauth-identity-providers-from-secrets.md` covering all live D-* decisions (D-1, D-3-RECON4, D-5-RECON4, D-6, D-7, D-8, D-9-RECON3, D-11-RECON3, D-12-RECON3, D-13, D-14, D-15, D-16) and SUPERSEDED notes against D-2, D-4, REQ-cred-001, original D-3 / D-11 / D-12. Includes the R-D7 release-notes blurb verbatim. Cross-links explore/proposal/spec/design and recon-2/3/4. | ~180 |
| T1.3 | DOC | Add ADR-0085 to `docs/adr/README.md` index (if maintained). Verify rustdoc broken-intra-doc-links lint stays clean if the ADR is referenced from a crate's lib.rs. | ~5 |
| T1.4 | GATE | `task dev:check` green. Markdown lint clean. PR description references ROADMAP §M3.1 + explore.md. | 0 |

**PR-1 verification**: `git diff --shortstat main..HEAD` ≤ 200 lines. CODEOWNERS routes to ARCH owners. No code paths touched.

---

## PR-2 — Trait + config + compose validation + test-util feature (target ~330 LOC)

**Goal**: All foundation pieces in place so PR-3 can wire real authorize URLs. After this PR, `start_oauth` still returns the synthetic URL (no behavior change); `complete_oauth` still returns `NotImplemented`. But the config schema, trait signature, and compose-root validation are real.

| # | Tag | Description | LOC |
|---|---|---|---|
| T2.1 | SCAFFOLD | Create `crates/api/src/config/oauth.rs` (new file). Define `OAuthProvidersConfig` (outer map), `OAuthProviderConfig` (per-provider), `OAuthEndpoints` enum (`Oidc { discovery_url }` vs `Manual { authorize_url, token_url, userinfo_url, jwks_url: Option<String>, scopes: Vec<String> }`). Derive Serialize/Deserialize/Debug. `client_id` and `client_secret` are `SecretString`. | ~110 |
| T2.2 | SCAFFOLD | Wire `oauth: Option<OAuthProvidersConfig>` into `ApiConfig::auth` (`crates/api/src/config/mod.rs`). Add env-binding helpers in `crates/api/src/config/env.rs` for `API_AUTH_OAUTH_<PROVIDER>_*` (CLIENT_ID, CLIENT_SECRET, DISCOVERY_URL, AUTHORIZE_URL, TOKEN_URL, USERINFO_URL, SCOPES). | ~80 |
| T2.3 | RED | `oauth_provider_config_validates_oidc_requires_https_discovery_url` (unit test in `crates/api/src/config/oauth.rs`). | ~25 |
| T2.4 | RED | `oauth_provider_config_validates_manual_requires_non_empty_scopes`. | ~20 |
| T2.5 | RED | `oauth_provider_config_rejects_http_endpoint_in_release_mode` (validates `flow::validate_token_endpoint` is invoked). | ~25 |
| T2.6 | RED | `compose_root_fails_closed_when_public_url_unset_with_oauth_declared` (integration test in `crates/api/tests/compose_oauth_smoke.rs`). | ~40 |
| T2.7 | RED | `auth_handler_derives_redirect_uri_from_public_url_for_both_start_and_complete` (handler-level test verifying the `redirect_uri` argument passed to BOTH `backend.start_oauth(provider, redirect_uri)` and `backend.complete_oauth(provider, state, code, redirect_uri)` matches `format!("{}/auth/oauth/{}/callback", public_url, provider.as_str())`). | ~50 |
| T2.8 | GREEN | Implement config validation in `OAuthProvidersConfig::validate_at_load(api_config)`. Touches `apps/server/src/compose.rs` to invoke validation. Failures map to `TransportInitError::OAuthProviderConfigInvalid { provider, reason }`. | ~80 |
| T2.9 | GREEN | Extend `AuthBackend` trait at `crates/api/src/domain/auth/backend/provider.rs` with `redirect_uri: &str` on BOTH methods (spec REQ-oauth-003 requires it for the callback comparison per Scenario 3.10 `public_url` change-mid-flow defense):\<br/>\- `async fn start_oauth(&self, provider, redirect_uri: &str) -> Result<OAuthStart, AuthError>`\<br/>\- `async fn complete_oauth(&self, provider, state: &str, code: &str, redirect_uri: &str) -> Result<Session, AuthError>`\<br/>Update both implementors (`PgAuthBackend`, `InMemoryAuthBackend`) to accept and pass through the new arg (still build the synthetic URL for `start_oauth`; still return `NotImplemented` for `complete_oauth` — no functional change in this PR, just trait surface). Update every mock in `crates/api/tests/*.rs`. | ~90 |
| T2.10 | GREEN | Update both handlers in `crates/api/src/domain/auth/handler.rs` (`oauth_start` AND `oauth_callback`) to derive `redirect_uri = format!("{}/auth/oauth/{}/callback", state.api_config.public_url, provider.as_str())` and pass to the corresponding backend method. The two derivations must use the same formula (a private helper in the handler module or a `OAuthProviderConfig::derived_redirect_uri(public_url, provider)` accessor). | ~40 |
| T2.11 | SCAFFOLD | Add a `test_support` module to `crates/api/src/lib.rs` gated by **custom cfg** `#[cfg(nebula_test_util)]` (NOT a Cargo feature — features are additive and can be transitively activated; custom cfg requires explicit `RUSTFLAGS="--cfg nebula_test_util"` opt-in per the `tokio_unstable` precedent). The module re-exports three test-only bypass helpers: `flow::exchange_code_unchecked` (token POST), a new `oauth_token_http_client_test_unchecked()` (userinfo GET against localhost), and a new `fetch_oidc_discovery_unchecked(url)` (D-15 discovery doc against localhost). Tests opt in via `RUSTFLAGS="--cfg nebula_test_util" cargo nextest run -p nebula-api --test oauth_provider_e2e`. Add `[lints.rust] unexpected_cfgs = { level = "warn", check-cfg = ['cfg(nebula_test_util)'] }` to `crates/api/Cargo.toml`. | ~50 |
| T2.12 | GREEN | Add production-build guard to `crates/api/src/lib.rs`: `#[cfg(all(nebula_test_util, not(debug_assertions)))] compile_error!("nebula_test_util cfg must NOT be active in release builds; remove --cfg nebula_test_util from RUSTFLAGS");`. The `not(debug_assertions)` cfg is the canonical release-profile detection (set by `cargo build --release`). The earlier proposal of `cfg(feature = "release")` was structurally wrong — release is a Cargo profile, NOT a feature. CI parity in `.github/workflows/ci.yml`: (a) one job runs `cargo build --release --workspace` with empty `RUSTFLAGS` and asserts the guard does NOT fire; (b) one negative-test job runs `RUSTFLAGS="--cfg nebula_test_util" cargo build --release --workspace` and asserts a non-zero exit with the compile_error message in stderr. | ~30 |
| T2.13 | TRIANGULATE | `env_binding_round_trips_oidc_provider_config`; `env_binding_round_trips_manual_provider_config_with_scopes`; `provider_key_typo_fails_at_config_load` (e.g. "gooogle"). | ~50 |
| T2.14 | REFACTOR | Inline cleanup. Verify no `#[expect(dead_code)]` introduced. | 0 |
| T2.15 | GATE | `task dev:check` green. `cargo deny check` green. `git diff --shortstat` ≤ 800. PR description references explore/proposal/spec/design/recon-2/recon-3/recon-4 and ADR-0085. | 0 |

**RED test count for PR-2**: 5 (T2.3, T2.4, T2.5, T2.6, T2.7). §T2.7 covers both `start_oauth` and `complete_oauth` trait-arg propagation — the single derivation formula is the test's contract.

---

## PR-3 — Real authorize URL via `flow::build_authorization_uri` + OIDC discovery (target ~150 LOC)

**Goal**: `PgAuthBackend::start_oauth` and `InMemoryAuthBackend::start_oauth` return REAL authorize URLs. `OAuthStateRow.redirect_uri` is populated (was None before). For `Oidc` providers, the discovery doc is fetched and cached.

| # | Tag | Description | LOC |
|---|---|---|---|
| T3.1 | SCAFFOLD | Create `crates/api/src/transport/oauth/discovery.rs`. Define `OidcDiscovery { authorize_url, token_url, userinfo_url, jwks_url: Option<String> }` (serde Deserialize from the well-known doc schema). Add `static DISCOVERY_CACHE: OnceLock<DashMap<String, OidcDiscovery>>`. Implement `async fn fetch_oidc_discovery(url: &str) -> Result<OidcDiscovery, DiscoveryError>` that calls `validate_token_endpoint(url)` first, then uses `oauth_token_http_client()` to GET + parse JSON, caches result. | ~90 |
| T3.2 | RED | `discovery_fetches_and_caches_well_known_doc` (unit in `discovery.rs`). Use a mock server. | ~40 |
| T3.3 | RED | `start_oauth_emits_real_authorize_url_with_pkce_s256_for_oidc_provider` (integration in `crates/api/tests/oauth_provider_e2e.rs` — create file). | ~50 |
| T3.4 | RED | `start_oauth_emits_real_authorize_url_for_manual_provider_with_explicit_endpoints`. | ~40 |
| T3.5 | RED | `start_oauth_persists_redirect_uri_into_oauth_state_row`. | ~35 |
| T3.6 | RED | `start_oauth_returns_provider_not_configured_when_provider_absent` (the new `AuthError::ProviderNotConfigured` variant). | ~25 |
| T3.7 | GREEN | Add `AuthError::ProviderNotConfigured { provider: String }` variant. Update `AuthError → ApiError` mapping → `ApiError::ServiceUnavailable` (HTTP 503). Update the exhaustive-mapping audit per #753. | ~30 |
| T3.8 | GREEN | Rewrite `PgAuthBackend::start_oauth` at `crates/api/src/domain/auth/backend/pg.rs`. Replace synthetic `format!("https://nebula.local/...")` with: (1) look up `OAuthProviderConfig`, (2) resolve endpoints (Oidc → `fetch_oidc_discovery`; Manual → use as-is), (3) call `mint_pkce()`, (4) build `AuthorizationUriRequest`, (5) call `flow::build_authorization_uri(req, &pkce.state, &pkce.code_challenge)`, (6) persist `OAuthStateRow` with `redirect_uri: Some(redirect_uri.to_owned())`, (7) return real `OAuthStart`. | ~50 |
| T3.9 | GREEN | Symmetric impl for `InMemoryAuthBackend::start_oauth`. Uses an in-memory pending store mirror to match Plane A semantics. | ~40 |
| T3.10 | TRIANGULATE | `start_oauth_url_encodes_redirect_uri_correctly`; `start_oauth_includes_scope_param_for_manual_provider`; `start_oauth_omits_scope_param_for_oidc_provider_when_hardcoded_default`. | ~50 |
| T3.11 | REFACTOR | Pull provider-config-lookup helper into `crates/api/src/config/oauth.rs::resolve_for_provider(provider) -> Option<&OAuthProviderConfig>` if used in multiple call sites. | ~20 |
| T3.12 | GATE | `task dev:check` green. `git diff --shortstat main..HEAD` ≤ 800. | 0 |

**RED test count for PR-3**: 5 (T3.2, T3.3, T3.4, T3.5, T3.6).

---

## PR-4 — Real `complete_oauth` + userinfo + `external_identities` + find-or-create (target ~330 LOC)

**Goal**: Removes the `NotImplemented` punt. Real OAuth callback exchanges code for tokens, fetches userinfo, finds or creates the local Nebula user, mints a session. **No id_token JWKS validation in 1.0 (D-16 defer).**

| # | Tag | Description | LOC |
|---|---|---|---|
| T4.1 | SCAFFOLD | Create migration `crates/storage/migrations/postgres/00XX_external_identities.sql` (number TBD per migration sequence at PR time). Table per ADR-0085 D-8: `(user_id BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE, provider TEXT NOT NULL, subject TEXT NOT NULL, email TEXT, linked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), PRIMARY KEY (provider, subject))` + `external_identities_user_id_idx`. **`user_id BYTEA`** matches the existing `0001_users.sql` + `0002_user_auth.sql` convention (`users.id BYTEA PRIMARY KEY` as 16-byte ULID); UUID would fail FK validation at migration apply time. | ~30 |
| T4.2 | SCAFFOLD | Define `ExternalIdentityRow` in `crates/storage/src/rows/external_identity.rs`. Define `ExternalIdentityRepo` trait in `crates/storage/src/repos/external_identity.rs` with `find_user_by_external(provider, subject) -> Option<UserId>` and `link_external(user_id, provider, subject, email) -> Result<(), Duplicate>`. | ~50 |
| T4.3 | SCAFFOLD | Implement `PgExternalIdentityRepo` in `crates/storage/src/pg/external_identity.rs` with `#[tracing::instrument]` + `debug_assert!` on PK invariants. Implement `InMemoryExternalIdentityRepo` parallel. | ~80 |
| T4.4 | RED | `complete_oauth_succeeds_with_valid_code_oidc_provider` (full happy path via wiremock). | ~70 |
| T4.5 | RED | `complete_oauth_succeeds_with_valid_code_manual_provider_github`. | ~50 |
| T4.6 | RED | `complete_oauth_rejects_replay` (same state used twice). | ~30 |
| T4.7 | RED | `complete_oauth_rejects_expired_pending_row`. | ~30 |
| T4.8 | RED | `complete_oauth_rejects_mismatched_state_token`. | ~25 |
| T4.9 | RED | `complete_oauth_rejects_mismatched_provider` (state belongs to google, callback says github). | ~30 |
| T4.10 | RED | `complete_oauth_rejects_public_url_changed_mid_flow` (Scenario 3.10). | ~35 |
| T4.11 | RED | `complete_oauth_handles_idp_token_endpoint_500_with_redacted_log`. | ~40 |
| T4.12 | RED | `complete_oauth_rejects_malformed_token_response_missing_access_token`. | ~25 |
| T4.13 | RED | `complete_oauth_creates_user_on_first_login_with_idp_verified_email` (REQ-oauth-004). | ~55 |
| T4.14 | RED | `complete_oauth_rejects_first_login_when_idp_email_unverified` (REQ-oauth-004). | ~30 |
| T4.15 | RED | `complete_oauth_links_existing_user_when_emails_both_verified` (REQ-oauth-005 5.1). | ~50 |
| T4.16 | RED | `complete_oauth_rejects_link_when_nebula_email_unverified_idp_verified` (REQ-oauth-005 5.2 — account-takeover defense). | ~40 |
| T4.17 | GREEN | Rewrite `PgAuthBackend::complete_oauth` at `pg.rs:1079-1110`. Steps per spec REQ-oauth-003 (RECON-3/4 revised): `oauth_state_repo.consume_by_state_and_provider(state, provider.as_str())` (Plane A `OAuthStateRepo` — NOT `pending_state_store`); verify the row's `redirect_uri` equals the handler-supplied `redirect_uri` arg — if not, return `AuthError::OAuthFailed { cause: "public_url_changed_mid_flow" }` (Scenario 3.10); build `flow::TokenExchangeRequest` from provider config + row.code_verifier + code + redirect_uri; call `flow::exchange_code(req)` (per D-12-RECON3 — uses `oauth_token_http_client` + `validate_token_endpoint`); **log `id_token` field presence only — NO JWKS signature validation per D-16 defer to 1.1**; GET userinfo via `oauth_token_http_client()` with `Authorization: Bearer <access_token>`; parse `email` + `sub` + `email_verified`; find-or-create user via `external_identities` + truth table (REQ-oauth-005); mint session via existing session pipeline; drop tokens (borrow checker enforces). **Remove the `Err(AuthError::NotImplemented(...))` line.** | ~140 |
| T4.18 | GREEN | Symmetric `InMemoryAuthBackend::complete_oauth` impl. Uses `InMemoryExternalIdentityRepo` + `InMemoryUserStore` find-or-create. | ~80 |
| T4.19 | GREEN | Wire `PgExternalIdentityRepo` into `PgAuthBackend` (new constructor arg). Wire `InMemoryExternalIdentityRepo` into `InMemoryAuthBackend`. Update `compose.rs` to instantiate the PG variant. | ~40 |
| T4.20 | TRIANGULATE | `complete_oauth_userinfo_request_includes_bearer_header`; `complete_oauth_drops_oauth_tokens_after_session_mint` (verify via tracing capture that no token-shape value escapes the function); `wiremock_serves_200_with_empty_body_fails_token_response_malformed`. | ~60 |
| T4.21 | REFACTOR | Extract a `find_or_create_user_from_idp(userinfo, provider) -> Result<UserId, AuthError>` helper if used in both backends. | ~30 |
| T4.22 | GATE | `task dev:check` green. **Verify §4.5 grep is clean**: `rg "NotImplemented" crates/api/src/domain/auth/backend/` returns zero OAuth-related hits. `git diff --shortstat main..HEAD` ≤ 800. | 0 |

**RED test count for PR-4**: 13 (T4.4 through T4.16).

---

## PR-5 — Docs + ROADMAP flip + 1.1 follow-up issue (target ~240 LOC)

**Goal**: Operator-facing documentation lands. ROADMAP §M3.1 final checkbox flips. 1.1 follow-up for JWKS validation is filed.

| # | Tag | Description | LOC |
|---|---|---|---|
| T5.1 | DOC | `crates/api/README.md` gets a new "OAuth provider configuration (Plane A — identity login)" section. Show two copy-paste examples: (1) Google via OIDC discovery; (2) GitHub via Manual endpoints. Document the auto-derived `redirect_uri` formula + the `API_PUBLIC_URL` requirement. Cross-link ADR-0085 + recon-4. | ~120 |
| T5.2 | DOC | Add the R-D7 release-notes blurb to `crates/api/README.md` under a "Known limitations" subsection: "OAuth identity login (1.0): `id_token` JWKS signature validation is NOT performed in 1.0. The IdP's userinfo endpoint over TLS is the authoritative source for `email` + `sub`. 1.1 will add JWKS validation via the `openidconnect` crate or equivalent. Operators that require strict OIDC compliance now should track issue #TBD." | ~25 |
| T5.3 | DOC | File a 1.1 follow-up GitHub issue: "Add id_token JWKS signature validation for OAuth identity login (M3.1 D-16 follow-up)". Description references this change's ADR-0085 + recon-4. Update T5.2's `#TBD` with the actual issue number. | ~0 (issue + README #-update) |
| T5.4 | DOC | Update `docs/ROADMAP.md` §M3.1: flip the final OAuth checkbox to `[x]`. Add a one-line evidence reference: `OAuth providers from operator secrets — closed via PR-X1..PR-X5; ADR-0085; see openspec/changes/oauth-providers-from-operator-secrets/`. | ~15 |
| T5.5 | DOC | Add a doctest snippet inside the new README section that compiles (`cargo test --workspace --doc` runs it). The snippet shows constructing `OAuthProvidersConfig` programmatically. | ~40 |
| T5.6 | DOC | Verify no `cargo doc --no-deps --workspace` warnings introduced. Verify `-D rustdoc::broken_intra_doc_links` stays clean if README cross-references introduced any new ADR/recon links. | ~5 |
| T5.7 | GATE | `task dev:check` green; `cargo test --workspace --doc` green; `task examples:check` green if applicable; ROADMAP checkbox visibly flipped. Confirm `cargo deny check` and lefthook pre-push CI parity. `git diff --shortstat main..HEAD` ≤ 800. | 0 |

**RED test count for PR-5**: 0 (DOC-only; the doctest in T5.5 is a TRIANGULATE check that the README example compiles).

---

## Cumulative summary

| PR | LOC target | RED tests | Key deliverable |
|---|---|---|---|
| 1 | ~180 | 0 | ADR-0085 |
| 2 | ~330 | 5 | Trait sig + config types + compose validation + test-util feature |
| 3 | ~150 | 5 | Real authorize URL + OIDC discovery |
| 4 | ~330 | 13 | Real complete_oauth + external_identities + find-or-create user |
| 5 | ~240 | 0 | Docs + ROADMAP flip + 1.1 follow-up |

**Total**: ~1,230 LOC, 23 RED tests (recon-4 said 21; the +2 are T4.4 and T4.5 split into OIDC and Manual happy paths to cover both endpoint shapes — counted as one in recon-4 §6).

## Strict TDD evidence requirements

For each PR (PR-1 and PR-5 exempt as DOC-only):

- **Commit-level discipline**: the FIRST commit of the PR adds RED tests only. They must FAIL on their own (verify via `git commit && cargo nextest run -- <test_name>` → red). The reviewer is expected to confirm RED commit failure before reviewing GREEN.
- **GREEN commits**: minimal implementation to pass each RED. Multiple GREEN commits are allowed; each must turn one or more REDs into PASS without breaking any other test.
- **TRIANGULATE commits**: boundary tests must be authored after GREEN. They sometimes uncover regressions in the GREEN code — that's the point. Fix and re-GREEN.
- **REFACTOR commits**: must NOT change behavior. All tests still pass.
- **PR description**: lists each task ID with its commit SHA, RED→PASS evidence link (CI log URL), and any deviations from this task list (with rationale).

## PR-boundary gates (squash-merge ceremony)

Before each PR squash-merges to `main`:

1. `task dev:check` green locally.
2. CI required jobs green (workflow_call into `.github/workflows/ci.yml`).
3. `cargo deny check` green.
4. `git diff --shortstat main..HEAD` ≤ 800 lines (changed = added + removed combined).
5. PR description references the next-PR dependency chain (PR-N → PR-N+1).
6. CODEOWNERS approval from the routed reviewer set.
7. (PR-1 only) ARCH owner ADR approval.
8. (PR-4 only) `rg "NotImplemented" crates/api/src/domain/auth/backend/` returns zero OAuth-related hits.
9. (PR-5 only) ROADMAP checkbox visibly flipped in the same PR.

## Worker handoff contract

When a worker subagent (or human contributor) takes on a PR, the entry point is this `tasks.md` plus the relevant artifacts. The worker MUST:

1. Read `design.md` + `recon-2` + `recon-3` + `recon-4` (cumulative supersede chain) before writing any code.
2. Read `spec.md` for the requirement(s) this PR closes (per the coverage matrix at the end of `spec.md`).
3. Follow the task list in order. Skip a task only with explicit user approval (via intercom or by re-opening the SDD lifecycle).
4. For PR-2 and PR-3, worker reads of `transport/oauth/flow.rs` (full) and `storage/pg/oauth_state.rs` (full) are NO LONGER required as first tasks — recon-3 did these reads.
5. On RED-test commit, run `cargo nextest run` and screenshot/log the failure into the PR description before pushing the GREEN commit.
6. If the worker discovers a fact that contradicts the design (recon-4 wasn't exhaustive), surface it via `intercom` to the orchestrator BEFORE writing code. Do not silently re-architect.

## Next phase

After this `tasks.md` is approved:

- **`sdd-apply`** starts on PR-1 (ADR-only; ~180 LOC; no code).
- After PR-1 squash-merges to `main`, `sdd-apply` continues with PR-2, then PR-3, then PR-4, then PR-5.
- After PR-5 squash-merges, `sdd-verify` validates the closure against the spec deltas + acceptance criteria, then `sdd-archive` moves the change directory into OpenSpec canonical archive.

---

## Result envelope

```yaml
status: tasks-draft
executive_summary: |
  5-PR chain decomposed into 65 tasks: PR-1 (ADR, 4 tasks), PR-2 (config + trait + test-util, 15 tasks),
  PR-3 (authorize URL + OIDC discovery, 12 tasks), PR-4 (complete_oauth + external_identities, 22 tasks),
  PR-5 (docs + ROADMAP, 7 tasks). 23 RED tests anchored. Each task tagged RED/GREEN/TRIANGULATE/
  REFACTOR/SCAFFOLD/DOC/GATE with LOC estimate. Per-PR ≤ 800 LOC budget. Strict TDD discipline:
  RED commits FIRST, verifiable failure before GREEN. PR-boundary gates enumerated (cargo deny,
  task dev:check, §4.5 grep, ROADMAP visibility). Worker handoff contract requires reading the
  cumulative supersede chain (recon-2 + recon-3 + recon-4) before any code.
artifacts:
  - openspec/changes/oauth-providers-from-operator-secrets/{explore,proposal,spec,design,tasks}.md
  - openspec/changes/oauth-providers-from-operator-secrets/recon-{2,3,4}-*.md
next_recommended: sdd-apply on PR-1 (ADR-only)
risks:
  - R-D7 carried forward: 1.0 ships without id_token JWKS validation; documented in PR-5 README
  - Worker-discovered facts post-recon-4 must surface via intercom (handoff contract §6)
skill_resolution: none
```
