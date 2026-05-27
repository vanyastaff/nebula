# SDD Explore — OAuth providers from operator secrets

- **Change slug**: `oauth-providers-from-operator-secrets`
- **Origin**: ROADMAP §M3.1 residual; explicitly carved out as its own SDD plan in `docs/plans/2026-05-26-001-feat-api-m3.1-followup-wave-plan.md` §"Out of scope" (PR-C).
- **Date**: 2026-05-27
- **Author**: el Gentleman (orchestrator) + inline file recon (subagent dispatch retries failed: gemini-3.1-pro-preview returned empty; openrouter sonnet-4.5 had no API key).
- **Phase**: explore → handoff to `sdd-proposal`.

---

## 1. Change idea

Close the last open M3.1 checkbox: **wire the production OAuth code-exchange path** so a server operator can declare OAuth providers (Google, GitHub, generic OIDC, …) via configuration + secrets and have `POST /auth/oauth/{provider}/start` + `GET /auth/oauth/{provider}/callback` perform a real authorization-code flow that mints a Nebula session.

Today both endpoints are mounted (`crates/api/src/domain/auth/handler.rs:390-465`), the `OAuthStateRepo` row exists in PG (`crates/storage/src/pg/oauth_state.rs`), and the `PgAuthBackend::complete_oauth` implementation **explicitly returns `AuthError::NotImplemented(...)`** at `crates/api/src/domain/auth/backend/pg.rs:1109`. This is the last §4.5 honesty gap in M3.1.

## 2. Code map — touch-points (file:line → role)

| Touch-point | File | Role / current state |
|---|---|---|
| OAuth start handler | `crates/api/src/domain/auth/handler.rs:390-421` | Calls `backend.start_oauth(provider)`; no `redirect_uri` arg today. Mounted by the auth router. |
| OAuth callback handler | `crates/api/src/domain/auth/handler.rs:425-465` | Calls `backend.complete_oauth(provider, state, code)`. Returns a `SessionResponse` on success. |
| `AuthBackend` trait | `crates/api/src/domain/auth/backend/provider.rs:~207-220` | `start_oauth(provider) -> OAuthStart` and `complete_oauth(provider, state, code) -> Session`. **`start_oauth` lacks `redirect_uri`** (flagged in pg.rs:35 doc comment as deferred). |
| `PgAuthBackend::start_oauth` | `crates/api/src/domain/auth/backend/pg.rs:1028-~1075` | Mints state + nonce, persists into `oauth_states` row, returns synthetic authorize URL hardcoded to `https://nebula.local/...`. Never calls `CredentialService::get`. |
| `PgAuthBackend::complete_oauth` | `crates/api/src/domain/auth/backend/pg.rs:1079-1110` | Consumes state row, validates expiry/replay, then `return Err(AuthError::NotImplemented("oauth provider code exchange is not yet wired; complete_oauth requires …"))`. |
| `InMemoryAuthBackend` OAuth methods | `crates/api/src/domain/auth/backend/in_memory.rs:~720-770` | Same stubs (synthetic URL on start; `NotImplemented` on complete). Trait signature mirror — any breaking change must update this too. |
| `AppState::credential_service` slot | `crates/api/src/state.rs:259` | `pub credential_service: Option<Arc<CredentialService<InMemoryStore, InMemoryPendingStore>>>`. **Concrete generic — not `dyn`**. Set via `with_credential_service` (line 1036). |
| Compose root | `apps/server/src/compose.rs` | **NEVER calls `with_credential_service`** — `credential_service` is always `None` in the running server. Only `with_credential_schema` is wired (line ~300 — that's ADR-0052 P4 catalog schema, unrelated). |
| `CredentialService::get` | `crates/credential-runtime/src/service.rs:361-371` | Returns `CredentialSnapshot`, **not** generic `get::<C>()`. No typed-decode helper from snapshot to `OAuth2Credential` exists. |
| `OAuth2Credential` types | `crates/credential/src/credentials/oauth2.rs` | `OAuth2Config` with `GrantType` (`AuthCode`, `ClientCredentials`, `DeviceCode`), `AuthCodeBuilder`, `PkceMethod`, `OAuth2State`, `OAuth2Token`. Pre-existing — no changes needed. |
| Credential-builtin scaffold | `crates/credential-builtin/src/{bearer_token,shared_key,signing_key,registry}.rs` | Recon said "scaffold" but the crate already ships 4 concrete types. **No `oauth2_provider.rs` yet** — this change may add one OR live entirely inside `nebula-api`. |
| OAuth provider enum | `crates/api/src/transport/oauth/state.rs::OAuthProvider` | The provider enum (Google/GitHub/Generic/…). Strongly typed; not a free-string. |
| HTTP client | workspace `Cargo.toml` | **`reqwest = "0.13"` IS already in workspace deps** (recon was stale on this). Currently a transitive dep; `nebula-api/Cargo.toml` does not import it directly. |
| OAuth helper crates | workspace `Cargo.toml` | No `oauth2`, no `openidconnect` crate. Decision: hand-rolled with `reqwest` vs. add `oauth2` crate (~12 LOC saved per provider × N providers, but ~+1 dep). |

## 3. Operator config surface — decision space (NOT a decision)

The "how does the API know which credential ID holds the Google OAuth2 config?" question has three viable shapes. All three must integrate with the spec-16 `CredentialService` facade.

### Option A — Config-map keyed by `OAuthProvider` variant

```toml
# apps/server config (TOML / env-var binding)
[auth.oauth.providers]
google = { credential_id = "cred_01HX...G7", scopes = ["openid", "email"], redirect_uri = "https://nebula.example.com/auth/oauth/google/callback" }
github = { credential_id = "cred_01HX...GH", scopes = ["read:user"] }
```

- **Pros**: explicit; one stable name → one credential ID; operators add/remove providers via config without touching credentials; the credential itself stays a generic `OAuth2Credential` (no Nebula-specific tagging). Aligns with how `IdempotencyApiConfig` is shaped today (`crates/api/src/config/sub.rs:91-190`).
- **Cons**: config-map duplicates the `OAuthProvider` enum keys (operator can typo `"gooogle"`); rotating a credential ID requires a config edit; cross-provider scopes/redirect divergence pushed into config.
- **Touch**: new `OAuthProvidersConfig` struct in `crates/api/src/config/sub.rs`; new `API_AUTH_OAUTH_<PROVIDER>_*` env vars; compose root threads it into `PgAuthBackend::new`.

### Option B — Credential name convention (e.g. `oauth2/<provider>`)

The `PgAuthBackend` looks up the credential by a derived stable name (`format!("oauth2/{provider}")`) via `CredentialService::get` keyed on `(tenant_scope, name)`.

- **Pros**: zero config-schema growth; one credential per provider, named consistently; rotation is a credential-store mutation, not a config edit.
- **Cons**: `CredentialService::get` today is keyed on `(scope, id)`, not `(scope, name)` — needs a `get_by_name` accessor or a `CredentialName` index, both new public surface; name convention is a magic string that must be enforced somewhere (probably a typed `OAuth2ProviderCredentialName` newtype); fails closed if operator forgets the convention.
- **Touch**: extend `CredentialService` with name-keyed lookup; convention helper in `nebula-credential` or `nebula-api`; no config schema growth.

### Option C — New `CredentialKind::OAuth2Provider` discriminant

Add a tagged credential variant whose `kind = OAuth2Provider { provider: String }`. `PgAuthBackend` discovers via `CredentialService::list_by_kind(OAuth2Provider { provider: "google" })`.

- **Pros**: kind-tagged data model is the most type-safe; supports multiple credentials per provider (e.g. dev/staging/prod) with an explicit selector; aligns with the §M12.3 generic-credential-core direction (one of `GenericOAuth2`, `GenericPat`, `GenericApiKey`, `GenericBasicAuth`).
- **Cons**: largest blast radius — extends the public `CredentialKind` enum, adds new query path (`list_by_kind`), needs migration for existing credentials; couples M3.1 closure to M12.3 sequencing.
- **Touch**: `nebula-credential` (variant + accessor + maybe migration); `nebula-credential-builtin` (the `GenericOAuth2` type); `nebula-credential-runtime` (`list_by_kind`); `nebula-api` consumer.

### Recommendation for design phase

**Lean Option A** for 1.0 because it (1) avoids extending the credential public surface, (2) ships in one PR boundary, (3) keeps `CredentialService::get` honest (id-keyed), and (4) is reversible — Option B/C can layer on top in 1.1 without an Option-A migration. The actual decision belongs to `sdd-design`, not `sdd-explore`.

## 4. Breaking-change surface

| Public signature | Why | Compile-fail call sites |
|---|---|---|
| `AuthBackend::start_oauth(provider) → ...` becomes `start_oauth(provider, redirect_uri) → ...` | Operator-supplied redirect URL must round-trip to the IdP authorize URL and back to `complete_oauth` for verification. | `PgAuthBackend::start_oauth`, `InMemoryAuthBackend::start_oauth`, the auth router handler `crates/api/src/domain/auth/handler.rs:390-421`, every mock backend in `crates/api/tests/*` (search `impl AuthBackend for`). |
| `AppState::credential_service: Option<Arc<CredentialService<InMemoryStore, InMemoryPendingStore>>>` — concrete generic | Production path needs PG-backed `CredentialService`. Today's concrete `<InMemoryStore, _>` cannot wire a `PgCredentialStore`. Either erase to `dyn CredentialServiceErased` or widen generics. | All `AppState` consumers using `state.credential_service.as_ref()` — none today, so blast radius is minimal IF we change shape now. |
| `CredentialService::get_for_oauth_provider(scope, id) → Result<OAuth2Config, _>` (new typed helper) | `get()` returns opaque `CredentialSnapshot`; the OAuth path needs the typed `OAuth2Config`. The decode seam must live in `nebula-credential-runtime` or `nebula-credential` (not in `nebula-api`) to keep the typed-decode logic where the snapshot semantics are. | None — additive method. |

Total breaking surface is **bounded**: one trait method, one `AppState` field shape, one new typed-decode method. No callers outside the auth domain depend on these today.

## 5. HTTP client / new deps

- `reqwest` (workspace dep, `0.13`, `json` feature) — **already present** as a transitive dep; making `nebula-api` a direct consumer is a 1-line `Cargo.toml` addition. Recon claim "first direct consumer" stands — the workspace pin already covers TLS via `tokio1-rustls-tls` (paired with `lettre`).
- Decision: hand-rolled token exchange with `reqwest` vs. adding the `oauth2 = "5.x"` crate.
  - Hand-rolled: ~50 LOC of `POST` to the token endpoint with `Content-Type: application/x-www-form-urlencoded`, parse JSON, validate `expires_in`. Full control over error mapping to `AuthError::OAuthFailed(_)` (which exists).
  - `oauth2` crate: handles PKCE, state, refresh tokens, JWT decoding; ~+1 dep; introduces its own error taxonomy that must map back to `AuthError`. The crate is BSD-style licence — fine for our `deny.toml`.
- **Recommendation for design phase**: hand-rolled `reqwest` for 1.0 (the `OAuth2Config` and `PkceMethod` types we already own in `crates/credential/src/credentials/oauth2.rs` cover the input shape; the `OAuth2Token` type covers the output). Reconsider for 1.1 if multi-grant-type expansion lands.

## 6. Scope estimate

| Slice | Touch (rough LOC) |
|---|---|
| ADR (operator config convention; `CredentialService` generic vs. `dyn`; hand-rolled vs `oauth2` crate) | ~120 LOC (markdown only) |
| Trait extension: `start_oauth(provider, redirect_uri)` + every impl/test mock | ~80 LOC |
| Config schema: `OAuthProvidersConfig` + env binding + `Default` + tests | ~150 LOC |
| Compose-root wiring: instantiate `CredentialService` (PG-backed in prod, in-memory in dev), call `with_credential_service` | ~100 LOC |
| `CredentialService::get_for_oauth_provider` typed-decode seam (in `credential-runtime`) | ~80 LOC |
| `PgAuthBackend::start_oauth` real authorize-URL construction (read credential, build query string with PKCE, persist state row) | ~120 LOC |
| `PgAuthBackend::complete_oauth` real token exchange (`reqwest::post` to token endpoint, parse `OAuth2Token`, fetch user-info, find-or-create local user, mint session) | ~250 LOC |
| `InMemoryAuthBackend` symmetric implementation (same behavior modulo storage) | ~120 LOC |
| Integration tests: PG e2e (`auth_pg_e2e.rs` extension) + mock OAuth server with `wiremock` + replay-protection + signature-mismatch + provider-not-configured | ~300 LOC |
| Docs: `crates/api/README.md` OAuth section + ROADMAP §M3.1 checkbox flip | ~80 LOC |
| **Total estimate** | **~1,400 LOC** |

Recon said "~600-900 LOC" — that was an undercount. Real estimate is **~1,400 LOC** with integration tests, ADR, and docs. **Exceeds the 800-LOC review budget**, so chained PRs are mandatory per session preflight (`auto-forecast`).

## 7. Chained PR forecast (auto-forecast strategy)

| PR | Scope | LOC | Depends on |
|---|---|---|---|
| **PR-1: ADR** | Operator-config-convention ADR (pick A/B/C); `CredentialService` generic-vs-dyn ADR; hand-rolled vs `oauth2`-crate ADR. Single ADR file with three sub-decisions. | ~120 | None (decision-only, no apply phase) |
| **PR-2: Trait surface + redirect_uri plumb** | Extend `AuthBackend::start_oauth(provider, redirect_uri)`; update both impls + every mock; handler passes `redirect_uri` from request. No behavior change yet (URL still synthetic in PR-2). | ~150 | PR-1 |
| **PR-3: Config + compose-root wiring** | `OAuthProvidersConfig` + env binding; `CredentialService` instantiation in `compose.rs` (PG-backed in prod, fail-closed if not provisioned). No OAuth logic yet — slot is now populated. | ~250 | PR-2 |
| **PR-4: `get_for_oauth_provider` + `PgAuthBackend::start_oauth` real authorize URL** | Typed-decode seam in `credential-runtime`; PG backend builds real authorize URL with PKCE; state row carries `redirect_uri`. Still `NotImplemented` on complete. | ~250 | PR-3 |
| **PR-5: `complete_oauth` real token exchange** | `reqwest`-based token endpoint POST; parse `OAuth2Token`; user-info fetch; find-or-create local user; mint session. `InMemoryAuthBackend` symmetric impl. Removes `NotImplemented`. | ~350 | PR-4 |
| **PR-6: Integration tests + docs** | PG e2e with `wiremock` IdP; replay/expiry/signature-mismatch/provider-not-configured cases; README update; ROADMAP checkbox flip. | ~280 | PR-5 |

**6 PRs total.** Each fits well inside the 800-LOC review budget. PR-1 (ADR-only) is the smallest natural unblock. PR-2/3 are reversible plumbing. PR-4/5 are the real implementation. PR-6 is the verification gate.

**Alternative chain**: merge PR-2 + PR-3 into one (still ~400 LOC, safe), and merge PR-4 + PR-5 into one (~600 LOC, near budget ceiling). That gives a 4-PR chain. Recommend the 6-PR chain for review-load smoothness — the ADR has independent reviewers (ARCH owners) and the typed-decode seam belongs to credential CODEOWNERS, while the rest is api CODEOWNERS.

## 8. Strict TDD test anchors

- **Test runner**: `cargo nextest run --workspace --no-fail-fast` (per `openspec/config.yaml`).
- **Existing harness to extend**: `crates/api/tests/auth_pg_e2e.rs` (542 lines today; lifecycle test pattern at lines 100-542).
- **New file**: `crates/api/tests/oauth_provider_e2e.rs` for the OAuth-specific scenarios — keeps the giant lifecycle test undisturbed.
- **Mock IdP**: use `wiremock = "0.6"` (NOT currently a workspace dep — would be added as a `[dev-dependencies]` entry for `nebula-api` only). `wiremock` exposes a `MockServer` that can stub the `/authorize`, `/token`, `/userinfo` endpoints with controlled latency, status, and replay.
- **Dependency surface**: `task db:up` already provides PG. `task obs:up` is NOT needed for OAuth tests. No new infra requirement.
- **RED tests to write first** (one per acceptance criterion in the spec):
  1. `complete_oauth_succeeds_with_valid_code` — full happy path.
  2. `complete_oauth_rejects_replay` — same code twice → 2nd attempt returns `AuthError::InvalidToken` or `AuthError::OAuthFailed`.
  3. `complete_oauth_rejects_expired_state` — state row past TTL → reject.
  4. `complete_oauth_rejects_mismatched_state` — state cookie ≠ DB row → reject.
  5. `start_oauth_returns_provider_not_configured` — config has no entry for `provider` → `AuthError::OAuthFailed` (mapped to `ApiError::UpstreamError` 502, or a new `ProviderNotConfigured` variant if the design phase picks differently).
  6. `complete_oauth_handles_idp_token_endpoint_error` — `wiremock` returns 500 → `AuthError::OAuthFailed` with redacted body.
  7. `complete_oauth_creates_user_on_first_login` — unknown email → user row created with verified email.
  8. `complete_oauth_links_existing_user_on_email_match` — existing email → links external_id; no duplicate user.
  9. `pkce_code_verifier_round_trips` — code_verifier stored on start, validated on complete.
  10. `redirect_uri_mismatch_rejected` — handler-supplied redirect_uri differs from state row → reject.

## 9. Non-goals

This change MUST NOT do any of the following:

- **Ship vendor packs** (Google/GitHub/Microsoft/Apple-specific provider builders). That is M12.3 `nebula-credential-builtin` scope. This change ships the **generic** OAuth2 authorization-code flow that any operator can configure for any IdP.
- **Implement OAuth refresh-token rotation** as part of session lifecycle. Token rotation belongs to `nebula-credential-runtime` rotation fan-out (ADR-0067 / #688 / #690) which already shipped. This change uses access tokens only to mint a Nebula session, then discards the OAuth tokens (or stores them as a credential — TBD in design).
- **Ship the M14.4 PluginCtx broker** or any other Plugin-Proto layer change.
- **Resolve the M5 ABI ADR** — orthogonal.
- **Ship JWT-secured ID-token verification beyond signature** (no claim-policy engine). 1.0 ships HS/RS256 signature verification + standard claim validation (iss / aud / exp / iat / nonce). Custom claim policies are 1.1.
- **Implement device-code or client-credentials grant**. Authorization-code with PKCE only.
- **Touch any other M3.x sub-bullet** (M3.6 shift-left validation, M3.3 webhook follow-ups). Each is its own SDD plan.

## 10. Risks / unknowns to escalate in the proposal

1. **`AppState::credential_service` generic shape**: switching from `CredentialService<InMemoryStore, InMemoryPendingStore>` to either a `dyn`-erased type or a wider generic is a public-API break. Need to confirm in the design phase whether existing handler code outside `auth/oauth` consumes this slot — if yes, the breaking-change blast radius is larger than the touch table suggests.
2. **`CredentialService::get` keyed on id, not name**: Option B (name convention) needs a `get_by_name` method. If design picks Option B, expect an additive `nebula-credential-runtime` API change.
3. **Wiremock as a `[dev-dependencies]`-only addition**: confirm no policy in `deny.toml` blocks it (it should be fine — it is BSD-style).
4. **Operator credential rotation**: if PR-5 stores the IdP access/refresh tokens as a credential, the credential rotation fan-out (#688/#690) becomes a per-session concern. Recommend NOT storing IdP tokens in 1.0 — mint a Nebula session, discard the OAuth tokens. Defer "use IdP tokens for downstream calls" to 1.1.
5. **Session lifetime vs IdP token lifetime**: the IdP's `expires_in` does not bound Nebula's session lifetime. Document this explicitly in the spec to avoid §4.5 confusion.
6. **PKCE for confidential clients**: spec mandates PKCE even when `client_secret` is configured (defense in depth). Confirm this is acceptable; some IdPs reject `code_verifier` for confidential clients.
7. **redirect_uri host validation**: an operator might allow-list multiple redirect URIs (dev / staging / prod). Decide whether the config schema is `redirect_uri: String` (one value) or `redirect_uris: Vec<String>` (allow-list).
8. **`OAuth2Provider` enum exhaustiveness**: the enum is finite (Google/GitHub/Generic). For a `Generic` provider, the config-map needs authorize-URL + token-URL + user-info-URL fields. Confirm `Generic` ships in 1.0 vs. being deferred.
9. **CSRF & cookie-bound state**: the existing `oauth_states` row carries `state_token` + nonce; CSRF cookie binding (PR `b2a59ea8` / CSRF enforcement on credential write paths) — verify the OAuth callback handler enforces the cookie/state binding pre-existing.

## 11. Skill resolution

`skill_resolution: none` — no `.atl/skill-registry.md` in this repo; parent did not inject project-specific `SKILL.md` paths. Standard `sdd-explore` behavior used. Inline file recon substituted for the failed subagent dispatch.

## 12. Next recommended phase

**`sdd-proposal`** — turn this exploration into a proposal-grade document with: problem statement (operator declares OAuth providers → real session minting; today blocked by `NotImplemented`), goals/non-goals, alternatives considered (the three operator-config conventions), explicit acceptance criteria, risk register, and chained-PR forecast confirmation.

Pass the same artifact path (`openspec/changes/oauth-providers-from-operator-secrets/explore.md`) as input. Expected proposal output path: `openspec/changes/oauth-providers-from-operator-secrets/proposal.md`.

---

## Result envelope

```yaml
status: completed
executive_summary: |
  M3.1 OAuth code-exchange is the last open §4.5 honesty gap in M3.1. complete_oauth at
  pg.rs:1109 returns NotImplemented. AppState.credential_service is always None
  (compose.rs never wires it). AuthBackend::start_oauth lacks redirect_uri.
  CredentialService::get returns CredentialSnapshot (no typed OAuth2Credential decode).
  Workspace already pins reqwest 0.13 + lettre 0.11 (PR-D shipped SMTP).
  credential-builtin already ships bearer/shared-key/signing-key but no OAuth2Provider.
  Three operator-config conventions surfaced (A: config-map; B: name convention;
  C: CredentialKind variant) — Option A leans best for 1.0. Scope estimate ~1,400 LOC →
  6-PR chain (ADR / trait+redirect_uri / config+compose / authorize URL / token exchange /
  tests+docs). 10 RED tests anchored on a wiremock IdP. Non-goals: vendor packs (M12.3),
  refresh-token rotation, plugin-ctx broker, M5 ABI ADR.
artifacts:
  - openspec/changes/oauth-providers-from-operator-secrets/explore.md
next_recommended: sdd-proposal
risks:
  - AppState::credential_service generic shape is a public-API break
  - operator config convention (A/B/C) requires design-phase decision
  - wiremock dev-dep addition
  - whether to persist IdP tokens (recommend NOT for 1.0)
  - session/IdP token lifetime divergence (document explicitly)
  - Generic OAuth2 provider in 1.0 vs deferred
skill_resolution: none
```
