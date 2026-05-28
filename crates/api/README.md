---

## name: nebula-api
role: API Gateway
status: frontier
last-reviewed: 2026-05-15
canon-invariants: [L2-§4.5, L2-§12.3, L2-§12.4, L2-§13]
related: [nebula-storage, nebula-runtime, nebula-engine, nebula-plugin, nebula-metrics, nebula-credential, nebula-core]

# nebula-api

## Purpose

Provides the HTTP entry point for the Nebula workflow engine. Translates REST
requests into calls against typed port traits (`WorkflowRepo`, `ExecutionRepo`,
`ControlQueueRepo`, `OrgResolver`, `WorkspaceResolver`, `SessionStore`,
`MembershipStore`), then delegates all business logic to the crates below it.
The crate also hosts the `transport::webhook` subsystem, which handles inbound trigger
delivery and per-endpoint lifecycle management.

All routes are tenant-scoped under `/api/v1/orgs/{org}/workspaces/{ws}/…`
(per spec 05-api-routing). Slugs and ULIDs are accepted interchangeably
via `nebula-core::Slug`.

No SQL driver or storage schema knowledge lives here — those details are owned
by `nebula-storage` and injected through ports at startup (§12.3 local-first:
in-memory repos work without Docker or Redis).

## Role

API Gateway (EIP "Message Endpoint" at the system boundary). Thin HTTP shell
with no business logic of its own; all decisions are delegated to the engine
layer via port traits injected into `AppState`.

## Public API

- `AppState` — shared state struct holding all port references; built via
constructor + builder methods (`with_api_keys`, `with_metrics_registry`,
`with_webhook_transport`, …).
- `build_app` — assembles the axum `Router` with all middleware (tracing,
  CORS, compression, security headers, auth, tenancy, RBAC, CSRF).
  W3C Trace Context (`traceparent` / optional `tracestate`) is parsed in
  `middleware::trace_w3c`, linked into the per-request `TraceLayer` span, echoed on
  responses where applicable, and stamped onto durable `execution_control_queue` rows
  for `Start` / `Cancel` (see ADR-0050).
- `ApiConfig` / `ApiConfig::from_env` — runtime configuration with sub-configs:
`TlsConfig`, `CookieConfig`, `CorsConfig`, `VersioningConfig`,
`PaginationConfig`. Startup fails hard on a missing or short JWT secret
(no `Default` impl).
- `JwtSecret` — newtype that enforces a 32-byte minimum length at construction;
any value in hand is valid.
- `ApiError` / `ProblemDetails` — RFC 9457 error envelope; seam for §12.4.
Every failure path maps to a typed `ApiError` variant with an explicit HTTP
status — no new ad-hoc 500s for business-logic errors. Includes variants:
`SessionExpired`, `MfaRequired`, `InsufficientRole`, `OrgNotFound`,
`WorkspaceNotFound`, `SlugConflict`, `CsrfRejected`, `PaginationInvalid`,
`RateLimited`, `TenantMismatch`.
- `CursorParams` / `PaginatedResponse<T>` — cursor-based pagination
infrastructure (opaque base64-encoded cursors).
- Port traits: `OrgResolver`, `WorkspaceResolver`, `SessionStore`,
`MembershipStore` — tenant resolution and session management ports.
- `AuthContext` — authenticated request context extracted by `middleware::auth`.
- `transport::webhook::WebhookTransport` — activate/activate_slug/deactivate/router
for inbound webhook triggers; mounted on `/webhooks/*` (programmatic) and
`/api/v1/hooks/*` (slug-routed) when the transport is attached to
`AppState`.
- `transport::webhook::EndpointProviderImpl` — implements `nebula_action::WebhookEndpointProvider`
so action code can read `ctx.webhook.endpoint_url()` without knowing the HTTP
layer.
- `transport::webhook::WebhookRateLimiter` / `RateLimitExceeded` — per-key rate
limiting for inbound webhook requests.

## Contract

- **[L2-§12.4]** All error responses use RFC 9457 `application/problem+json`.
No new ad-hoc 500 for a business-logic failure; map new failure modes into
a typed `ApiError` variant. Seam: `crates/api/src/error/mod.rs`.
- **[L2-§13 step 1]** Workflow creation (`POST /api/v1/workflows`) delegates
to `WorkflowRepo::create`. Seam: `crates/api/src/domain/workflow/handler.rs` —
`create_workflow`.
- **[L2-§13 step 2]** Workflow activation (`POST /api/v1/workflows/:id/activate`)
runs `nebula_workflow::validate_workflow` and rejects invalid definitions
with structured RFC 9457 errors — it does not silently flip a flag. Seam:
`crates/api/src/domain/workflow/handler.rs` — `activate_workflow`.
- **[L2-§13 step 3]** Execution start (`POST /api/v1/workflows/:id/executions`)
returns 202 Accepted and enqueues; it does not block on engine completion.
Seam: `crates/api/src/domain/execution/handler.rs` — `start_execution`.
- **[L2-§13 step 5]** Cancel (`POST /api/v1/executions/:id/cancel`) writes a
durable signal to `ControlQueueRepo` in the same logical operation as the
state transition — not only a DB-row flip. Seam:
`crates/api/src/domain/execution/handler.rs` — `cancel_execution`.
- **[L2-§12.3]** Local-first: the server starts with in-memory repos (no
Docker/Redis required). The `AppState::new` constructor accepts any
`WorkflowRepo + ExecutionRepo + ControlQueueRepo` impl.
- **[L2-§4.5]** No capability is advertised that the engine does not honor
end-to-end. Planned features (JWT validation, rate limiting) are hidden
until the engine side is wired.
- **[L2-§12.2]** Cancel signals share the outbox transaction — the
`control_queue_repo` field in `AppState` is the durable outbox (§12.2).
A second in-memory control channel is forbidden (see §12.2 prohibition on
unreconciled second channels).

## Webhook subsystem

`crates/api/src/transport/webhook/` is the **single converged** HTTP transport
for inbound webhook triggers (M3.3 / ADR-0049). Both URL shapes funnel through
one `dispatch_inner` pipeline:

- **Programmatic** — `POST /webhooks/{trigger_uuid}/{nonce}`, minted by
`WebhookTransport::activate(...)` from the typed `nebula_action::WebhookAction`
runtime path.
- **Slug-routed** — `POST|GET /api/v1/hooks/{org}/{ws}/{trigger_slug}`,
loaded from storage at startup via `bootstrap_webhook_activations` and
mutated by `TriggerLifecycleEvent` consumers / the admin reload endpoint.

Responsibility split:

- `transport::webhook::WebhookTransport` — activate / deactivate / activate_slug /
replace_slug_map / axum router. Owns the routing map, rate limiter, signature
enforcement, replay-window check, and `pre_handle` short-circuit.
- `bootstrap` — `bootstrap_webhook_activations` / `collect_webhook_activations`,
`WebhookSecretResolver`, `WebhookContextFactory`. The composition root
invokes the bootstrap before `build_app`; admin reload uses `collect_*` and
`replace_slug_map` for atomic swaps.
- `events` — `TriggerLifecycleEvent` { Created / Updated / Deleted } +
`TriggerLifecycleSubscriber`. M3.3 ships the consumer; producer-side
wiring is deferred (ADR-0049 § "Out of scope").
- `transport::webhook::provider::EndpointProviderImpl` — implements
`nebula_action::WebhookEndpointProvider` so plugins read the public URL
without knowing the HTTP layer.
- `transport::webhook::key::WebhookKey` — `Programmatic { uuid, nonce }` | `Slug(TriggerCoordinates)`.
- `transport::webhook::routing` — private `RoutingMap` (DashMap) keyed by `WebhookKey`.
- `transport::webhook::ratelimit::WebhookRateLimiter` — per-key sliding-window guard with LRU-capped
path table (#271 mitigation).

Provider catalog (Slack `url_verification`, Stripe `pending_webhook` ping,
Generic `?challenge=…`) lives in `crates/action/src/webhook/providers/`;
each implements `WebhookAction::pre_handle`.

Operator-configured webhook URLs are documented here (and ADR-0049),
not in the OpenAPI spec — promoting the slug surface back into
`/api/v1/openapi.json` is a 1.0 follow-up that requires typed schemas
for every provider's request envelope.

Internal admin reload: `POST /internal/v1/webhooks/reload` (gated by
`X-Internal-Token`) atomically swaps the slug map after consulting
`WebhookActivationRepo::list_active`.

Delivery semantics: at-least-once (§11.3 / §13.4). Duplicate delivery is the
caller's responsibility via stable event identity + idempotency keys.

Signature enforcement (ADR-0022): the transport consults
`entry.config.signature_policy()` between `WebhookRequest::try_new` and
oneshot setup. Outcomes:

- `SignaturePolicy::OptionalAcceptUnsigned` → pass through to `handle_event`.
- `Required` with an empty secret → `500 application/problem+json`
(`https://nebula.dev/problems/webhook-signature-misconfigured`). Handler is NOT invoked.
- `Required` / `Custom` producing `SignatureOutcome::Missing` or `Invalid` → `401 application/problem+json` (`https://nebula.dev/problems/webhook-signature`). Handler is NOT invoked.
- `Valid` → pass through to `handle_event`.

Every rejection increments `nebula_webhook_signature_failures_total` with
`reason ∈ { missing, invalid, missing_secret }` when a metrics registry is
attached via `WebhookTransport::with_metrics`. The counter is low cardinality
by design (three static reason labels, no per-trigger dimension).

## OAuth identity providers (Plane A)

Per [ADR-0085](../../docs/adr/0085-oauth-identity-providers-from-secrets.md):
operator IdP-client credentials are configured via environment
variables (NOT a database table, NOT the credential store). Declaring
a provider is opt-in; an undeclared provider returns
`AuthError::ProviderNotConfigured` → HTTP 503 from
`/auth/oauth/{provider}/start`.

Supported providers in 1.0 (`OAuthProvider` enum):
`google`, `microsoft`, `github`. Auth0 / Okta / generic OIDC require
extending the enum (tracked as a 1.1 follow-up).

### Configuration shape

Each provider exposes one of two endpoint shapes:

| Shape | When to use | Required env vars |
|---|---|---|
| **OIDC** (`Google`, `Microsoft`) | IdP publishes `.well-known/openid-configuration` | `CLIENT_ID`, `CLIENT_SECRET`, `DISCOVERY_URL` |
| **Manual** (`GitHub`) | No discovery doc OR operator pins endpoints | `CLIENT_ID`, `CLIENT_SECRET`, `AUTHORIZE_URL`, `TOKEN_URL`, `USERINFO_URL`, `SCOPES` (+ optional `VERIFIED_EMAILS_URL`, `JWKS_URL`) |

All env vars are prefixed `API_AUTH_OAUTH_<UPPERCASE_PROVIDER>_*`.
A provider is "declared" if its `CLIENT_ID` is set; if `DISCOVERY_URL`
is also set the OIDC arm is chosen, otherwise Manual.

### `API_PUBLIC_URL`

The Plane-A OAuth flow derives `redirect_uri` from `API_PUBLIC_URL`
(NOT a per-provider config) per ADR-0085 D-3 (recon-4):
`{API_PUBLIC_URL}/auth/oauth/{provider}/callback`. Operators that need
multiple callback URIs deploy multiple Nebula instances. Boot fails
closed if `API_PUBLIC_URL` is empty / relative / scheme-less while
any provider is declared (REQ-compose-001 Invariant 1).

### Example: Google + GitHub side-by-side

```bash
# Required by all Plane-A OAuth flows.
export API_PUBLIC_URL=https://app.example.com

# Google — OIDC arm.
export API_AUTH_OAUTH_GOOGLE_CLIENT_ID="..."
export API_AUTH_OAUTH_GOOGLE_CLIENT_SECRET="..."
export API_AUTH_OAUTH_GOOGLE_DISCOVERY_URL="https://accounts.google.com/.well-known/openid-configuration"

# GitHub — Manual arm (no discovery doc; userinfo lacks email_verified
# so verified_emails_url is required for the truth-table to pass).
export API_AUTH_OAUTH_GITHUB_CLIENT_ID="Iv1.xxxxxxxx"
export API_AUTH_OAUTH_GITHUB_CLIENT_SECRET="..."
export API_AUTH_OAUTH_GITHUB_AUTHORIZE_URL="https://github.com/login/oauth/authorize"
export API_AUTH_OAUTH_GITHUB_TOKEN_URL="https://github.com/login/oauth/access_token"
export API_AUTH_OAUTH_GITHUB_USERINFO_URL="https://api.github.com/user"
export API_AUTH_OAUTH_GITHUB_VERIFIED_EMAILS_URL="https://api.github.com/user/emails"
export API_AUTH_OAUTH_GITHUB_SCOPES="user:email"
```

### Flow

1. Client `GET /api/v1/auth/oauth/{provider}` → server derives
   `redirect_uri`, mints PKCE pair, persists `OAuthStateRow`,
   returns the IdP `authorize_url` + opaque `state`.
2. Client redirects user's browser to `authorize_url`.
3. IdP redirects back to `{redirect_uri}?state=...&code=...`.
4. Client posts to `GET /api/v1/auth/oauth/{provider}/callback`.
5. Server consumes the state row atomically, verifies `redirect_uri`
   match, exchanges code at the IdP token endpoint, fetches
   userinfo (+ verified emails for GitHub), applies the
   REQ-oauth-004/-005/-006 truth table, mints a Nebula session.

### Security posture (REQ-obs-001)

- **Anti-SSRF**: every server-side OAuth HTTP URL passes
  `validate_oauth_outbound_url` (HTTPS-only; rejects loopback /
  private / link-local / multicast). Browser-fetched authorize URLs
  use the flag-aware `validate_oauth_authorize_url` which accepts
  `http://localhost` only when `oauth_allow_insecure_localhost` is
  set AND the binary is built with debug assertions (dev mode).
- **Body caps**: token endpoint and userinfo / verified-emails GETs
  are capped at 256 KiB so a hostile IdP cannot DoS via unbounded
  responses.
- **Discovery cache**: process-wide; key includes the
  `oauth_allow_insecure_localhost` flag so flag=true and flag=false
  callers never share a cache entry.
- **Tokens discarded**: per D-7, access / id / refresh tokens are
  dropped after the userinfo lookup; only the
  `(provider, subject) → user_id` link and the minted Nebula session
  are persisted.
- **id_token JWKS signature validation**: deferred to 1.1 per D-16.
  Userinfo response is authoritative for `(email, sub)`. The TLS
  trust chain to the IdP is the integrity boundary.
- **Account-takeover defense**: a user whose Nebula `email_verified`
  is `false` cannot link an OAuth identity even if the IdP attests
  `email_verified = true` (Scenario 5.2). The unverified user must
  complete email verification through the password flow first.
- **Observability**: `nebula_api_auth_oauth_attempts_total` carries
  closed-set `outcome` + `provider` labels;
  `nebula_api_auth_duration_seconds` histogram keyed by `outcome`
  only (provider stripped to keep histogram cardinality at the
  floor of `len(auth_outcome::*)` series).

## Machine-Readable API

`nebula-api` publishes its full route surface as an **OpenAPI 3.1**
specification (ADR-0047). The document is regenerated on every startup
from the same source as the served `axum::Router`, so the published
contract and the runtime cannot drift apart.

| Endpoint                  | Purpose |
|---------------------------|---------|
| `GET /api/v1/openapi.json` | OpenAPI 3.1 specification document — fetched by client generators (openapi-generator-cli, oapi-codegen, …). Unauthenticated. |
| `GET /api/v1/docs/`        | Swagger UI rendering of the served spec. Unauthenticated. **Self-hosted** via `utoipa_swagger_ui::SwaggerUi` — every static asset (HTML, CSS, JS) ships embedded in the server binary; no third-party CDN is reached at request time. |

### Drift-detection guarantee

The router is built through `utoipa_axum::router::OpenApiRouter::routes(routes!(handler))`,
which is the **only** mounting path that ties the served `axum::Router`
to the generated `OpenApi` value. Handlers without `#[utoipa::path]`
cannot pass through `routes!()` — drift is a compile error rather than
a review-time catch.

`crates/api/tests/openapi_spec.rs` adds runtime guards on top:

- `served_spec_pins_openapi_3_1_0` — `openapi == "3.1.0"`.
- `operation_ids_are_unique` — every handler function name is observed
  exactly once across the spec.
- `all_refs_resolve_to_declared_components` — recursive `$ref` walk.
- `security_schemes_match_adr_0047` — `bearer` / `api_key` / `csrf`.
- `drift_smoke_known_paths_are_present` — load-bearing paths inventory.
- `served_spec_round_trips_through_oas3_parser` — strict 3.1 parse.

### Stub Endpoint Policy

Endpoints whose handler is not yet wired end-to-end return
`ApiError::NotImplemented` (501) and are documented honestly per
ADR-0047 §4. The remaining stubs are the 3 org-record endpoints
(`GET`/`PATCH`/`DELETE /orgs/{org}` — no org-record store), the 3
service-account endpoints (no end-to-end `Principal::ServiceAccount`
auth path), `resource/list`, and `execution/restart` (canon §4.5: a
stub stays a 501 until its downstream genuinely honors it end-to-end).
**Graduated** stub→implemented end-to-end: `execution::terminate`; all
six `me/*` endpoints (`get_me`, `update_me`, `list_my_tokens`,
`create_token`, `delete_token`, and — Phase 3 — `list_my_orgs` via the
shared `MembershipStore`); and the 3 org **member** endpoints
(`list_members`, `add_member`, `remove_member`) — Phase 3, "Option 1"
honest contract (direct add-by-principal; the fake email-invitation
shape was dropped).

For each remaining stub:

- `#[deprecated]` on the handler so utoipa flags the operation in spec.
- `responses((status = 501, …))` carries the **planned** payload shape.
- Tag suffix ` (planned)` groups the stubs visibly in Swagger UI.

`crates/api/tests/openapi_canon_compliance.rs` enforces the policy in
both directions (every deprecated operation has a 501 response; every
stub module reaches the handler at runtime returning 500/501) so a
silently-shipped endpoint cannot pass review.

### `me/*` durability (canon §11.6 / §11.5)

The `me/*` profile + PAT endpoints (`get_me`, `update_me`,
`list_my_tokens`, `create_token`, `delete_token`) are **implemented and
work end-to-end**, but the only wired `AuthBackend` is the in-memory
one (`InMemoryAuthBackend`) — there is no storage-backed `AuthBackend`
impl, and (unlike idempotency) no feature-gated PG path, because
`nebula_storage` ships no `UserRepo` / `PatRepo` / `SessionRepo`.

| Aspect | `me/*` (in-memory `AuthBackend`) |
|---|---|
| Restart-survival | **No** — profiles, PATs, sessions are lost on restart |
| Multi-replica share | **No** — state is process-local; a PAT minted on one instance is invisible to others |

> **Operator warning:** a personal access token created via
> `POST /api/v1/me/tokens` stops authenticating the moment the process
> exits and is not shared across replicas. This is the same local-first
> caveat the `memory` idempotency backend carries (see *Store-backend
> tradeoffs* below) — the gap is strictly persistence, not capability.
> It closes when a storage-backed `AuthBackend` lands.

### Credential CRUD durability (canon §11.6 / §12.5)

The credential CRUD endpoints (`create` / `get` / `update` / `delete`
/ `list` under `…/workspaces/{ws}/credentials`) are **implemented and
work end-to-end** over the wired in-memory credential store
(`AppState::oauth_credential_store` —
`nebula_storage::credential::InMemoryStore`, the same real
`nebula_credential::CredentialStore` impl the OAuth2 callback writes
through). The type-specific `data` is persisted **write-only**: it is
wrapped in `nebula_credential::SecretString` for its in-process
lifetime and stored as an opaque blob; the metadata-only response
types have no `data` field, so `get` / `list` cannot echo the secret.

| Aspect | Credential CRUD (in-memory store) |
|---|---|
| Restart-survival | **No** — credentials are lost on restart |
| Multi-replica share | **No** — state is process-local |
| Encryption at rest | **No `EncryptionLayer` wired** — the blob is plaintext-at-rest in the in-memory store |
| Cross-workspace isolation | **None today** — the in-memory credential store is global and the `{org}`/`{ws}` path segments are not bound to credential ownership; any authenticated caller with a valid `cred_<ULID>` resolves/mutates it regardless of workspace. Pre-existing crate-wide local-first gap (same as `workflow`/`execution`); closes when the owner-scoped `nebula_storage::credential::ScopeLayer` is composed in the composition root. |

> **Operator warning:** a credential created via
> `POST …/credentials` stops resolving the moment the process exits and
> is not shared across replicas, its secret blob is **not encrypted
> at rest** in this build (the production `EncryptionLayer` from
> `nebula_storage`, ADR-0032, is not composed here), and there is **no
> cross-workspace isolation** — any authenticated caller holding a valid
> `cred_<ULID>` can resolve or mutate it regardless of the `{org}`/`{ws}`
> in the path, because no owner-scoped `ScopeLayer` / credential→workspace
> ownership binding is wired (the same crate-wide local-first gap that
> `workflow`/`execution` carry). Same local-first caveat as `me/*` and
> the `memory` idempotency backend — the gap is persistence + at-rest
> encryption + tenant-isolation *wiring* (un-composed cross-cutting
> layers), not the CRUD capability itself. It closes when a
> storage-backed, `EncryptionLayer`- and `ScopeLayer`-wrapped credential
> store is composed in the composition root.
>
> `test` / `refresh` / `revoke` / generic `resolve` /
> `resolve/continue` / credential-type discovery remain **honest 503**
> (canon §4.5): they require engine-owned dispatch
> (`nebula-engine::credential`, ADR-0030/ADR-0041) and/or a
> `CredentialRegistry` that is not wired into this build, so they
> deliberately refuse rather than fake a credential capability. The
> tenancy path resolver special-cases the literal `resolve` sub-route,
> so `resolve` / `resolve/continue` are **not** shadowed by the
> `{cred}` matcher — they reach the handler and return the honest 503
> above (not a pre-handler 404); the genuine `/credentials/{cred}`
> position stays strictly ULID-validated.

### Org membership durability (canon §11.6 / §11.5)

The org **member** endpoints (`GET`/`POST`/`DELETE` under
`…/orgs/{org}/members`) and the membership-backed `me/*` reads
(`GET /me/orgs`, `MeResponse.orgs_count`) are **implemented and tested
end-to-end** (`crates/api/tests/org_e2e.rs`) against the in-memory
`MembershipStore` (`nebula_api::domain::org::InMemoryMembershipStore`) —
the **single shared store** `rbac_middleware` also consults, so an
`add_member` is immediately visible to the next RBAC check (no
propagation window). There is no storage-backed alternative
(`nebula_storage` ships no membership repo); the in-memory impl *is*
the §4.5-honest backing, exactly as `InMemoryAuthBackend` is for
`me/*` identity and `InMemoryControlQueueRepo` is for the durable
control plane.

**The default `nebula-server` binary does NOT auto-wire a
`MembershipStore`.** It is an **explicitly-provisioned** feature — the
same posture as Postgres-for-durable-idempotency (provision the
production path; never silently fake it). Rationale: wiring a
`MembershipStore` activates RBAC enforcement on every org/workspace
route (`rbac_middleware`'s `is_some()` guard → a caller with no org
role is 404'd). The default `AuthBackend` is an *empty*
`InMemoryAuthBackend` (no users; `register_user` mints a **random**
`UserId`), so **no principal could authenticate as any auto-seeded
bootstrap owner** — an auto-seeded store would 404-deadlock every
org/workspace route (a deployment-level §4.5 false capability), and a
hardcoded auto-seeded admin identity would be a default-credential /
privileged-by-default surface (canon §12.5). Both are strictly worse
than honest degradation.

| Aspect | Org membership (in-memory `MembershipStore`) |
|---|---|
| Default binary | **Unwired (`None`)** — org member endpoints return an honest **503** (port-absent), RBAC stays inert (no spurious 404 on any route) |
| Restart-survival | **No** — memberships are lost on restart (once provisioned) |
| Multi-replica share | **No** — state is process-local |
| Provisioning | An operator/integrator wires `AppState::with_membership_store(...)` **and** registers the same bootstrap-owner identity in the wired `AuthBackend` so it can authenticate. `nebula_api::domain::org::InMemoryMembershipStore::seeded_bootstrap(org_id, owner_id)` is the documented constructor (fail-closed on a malformed id) |

> **Operator warning:** in the default binary, `GET`/`POST`/`DELETE
> `…/orgs/{org}/members` (and `GET /me/orgs` / `orgs_count`) return
> **503** until you provision a `MembershipStore`. This is honest
> degradation, **not** a bug — it deliberately avoids both an RBAC
> deadlock (404 on every org/workspace route) and a default admin
> credential. To enable: wire `with_membership_store(...)` with a
> bootstrap owner that is **also** a registered, authenticatable
> principal in your `AuthBackend`. Once provisioned, wiring a
> `MembershipStore` **activates RBAC enforcement** on every
> `/orgs/{org}/...` and `/orgs/{org}/workspaces/{ws}/...` route — a
> caller with no role in the resolved org is `404`'d *before* the
> handler (enumeration prevention); the bootstrap owner grants further
> access via `POST /orgs/{org}/members` (org-admin only, abuse-safe:
> role-clamp, last-admin/demote lockout guard at the atomic store seam,
> role-precedence, IDOR-404). Memberships are **process-local** and lost
> on restart — same local-first caveat as `me/*` and the `memory`
> idempotency backend. The org-record (`GET`/`PATCH`/`DELETE /orgs/{org}`)
> and service-account endpoints remain **honest 501** (no org-record
> store; no end-to-end `Principal::ServiceAccount` auth path).

### Regeneration

The spec is materialised inside `build_app` via
`OpenApiRouter::split_for_parts()` and handed straight to
`utoipa_swagger_ui::SwaggerUi`, which serves both `/api/v1/openapi.json`
and `/api/v1/docs/` as a Tower service merged into the application
router. `build_app` writes a stable
`tracing::info!(spec.version=%, paths=N, "openapi: spec compiled")`
line at startup so production logs can pin against it. To regenerate
locally, run any test that calls `nebula_api::build_app` — for example
`cargo nextest run -p nebula-api --test openapi_spec`.

### CSRF (M3.1)

State-changing endpoints reached over a cookie-bearing authentication
method (session cookie, JWT) are gated by `csrf_middleware`
(`crates/api/src/middleware/csrf.rs`). The double-submit-cookie pattern:

- On login the API issues two cookies: `nebula_session` (HttpOnly) and
  `nebula_csrf` (readable by the SPA, `SameSite=Lax`, `Secure`).
- Every state-changing request (`POST`/`PUT`/`PATCH`/`DELETE`) must echo
  the `nebula_csrf` value back in an `X-CSRF-Token` request header.
- The middleware rejects the request with `403 Forbidden` when the
  header is missing or does not byte-match the cookie.

**Route table** (verified by `crates/api/tests/{me_e2e,seam_credential_write_path_validation,auth_mfa_csrf}.rs`):

| Route group | Method gate | CSRF | Rationale |
|---|---|---|---|
| `/api/v1/me/*` | `POST`/`PUT`/`PATCH`/`DELETE` | **enforced** | session/JWT auth + state-changing |
| `/api/v1/orgs/{org}/workspaces/{ws}/credentials/*` | `POST`/`PUT`/`PATCH`/`DELETE` | **enforced** | session/JWT auth + state-changing |
| `/api/v1/orgs/{org}/*` (tenant-scoped) | `POST`/`PUT`/`PATCH`/`DELETE` | **enforced** | session/JWT auth + state-changing |
| `/api/v1/auth/mfa/enroll` | `POST` | **enforced** | session-bearing |
| `/api/v1/auth/mfa/verify` | `POST` | **enforced** | session-bearing (enrollment confirm) |
| `/api/v1/auth/login/mfa` | `POST` | **exempt by construction** | cookie-less second-factor login completion; `challenge_token` is the sole authority |
| `/api/v1/auth/logout` | `POST` | **exempt by deliberate choice** | revokes `nebula_session` when present; a CSRF attack can only force a sign-out (annoying, not a confidentiality / integrity breach). Keeps logout reachable when the CSRF cookie has drifted / been cleared. |
| `/api/v1/auth/{signup,login,forgot-password,reset-password,verify-email,oauth/*}` | `POST`/`GET` | **exempt by construction** | request does not carry a pre-existing session cookie |
| Any request authenticating via PAT (`pat_…`) or `X-API-Key` | any | **exempt by construction** | no cookie ⇒ no CSRF risk; verified inside `csrf_middleware` by reading the `AuthContext::auth_method` extension |

**Middleware order** — `auth_middleware` MUST be layered before
`csrf_middleware`. The latter reads the `AuthContext` extension that the
former installs to know whether to skip the gate for PAT/ApiKey callers.
The Plane-B credential routes wire the pair explicitly in
`crates/api/src/domain/mod.rs` (`auth_middleware` then `csrf_middleware`).

**Header contract** — callers send the matching token as
`X-CSRF-Token: <value>`. The middleware compares header against cookie
byte-for-byte; partial matches and case-normalised matches are rejected.
Missing header **or** missing cookie yields `403 "CSRF token missing"`;
mismatch yields `403 "CSRF token mismatch"`.

### Idempotency-Key (M3.4 / ADR-0048)

Every state-changing endpoint reachable from `build_app` is replay-protected
through the `IdempotencyLayer` middleware. Clients opt in by sending an
`Idempotency-Key` header on a `POST` request — the middleware caches the
first response (status + body + filtered headers) keyed by
`(method, path, key, identity-fingerprint, body-fingerprint)` and replays
it byte-for-byte on subsequent requests within the configured TTL.

**Protocol contract** — the IETF draft
[`draft-ietf-httpapi-idempotency-key`](https://datatracker.ietf.org/doc/draft-ietf-httpapi-idempotency-key/)
governs header semantics. Highlights:

- Same `Idempotency-Key` + same body within the TTL → cached replay
  (response carries `Idempotent-Replay: true`).
- Same key + different body → **422 Unprocessable Entity** with
  `application/problem+json`.
- `5xx` responses are passed through uncached so transient backend failures
  do not pin a permanent error for the TTL window.

**Environment variables** (defaults applied by `ApiConfig::from_env`):

| Var | Default | Notes |
|---|---|---|
| `API_IDEMPOTENCY_BACKEND` | `memory` | `memory` \| `postgres`. See backend tradeoffs below. |
| `API_IDEMPOTENCY_TTL_SECS` | `86400` | Cached-entry lifetime (24h matches the IETF draft). |
| `API_IDEMPOTENCY_MAX_ENTRIES` | `10000` | Cap for the in-memory backend; PG honours `expires_at` instead. |
| `API_IDEMPOTENCY_MAX_REQUEST_BODY_BYTES` | `1048576` | Requests beyond this skip caching (forwarded as-is). |
| `API_IDEMPOTENCY_MAX_RESPONSE_BODY_BYTES` | `1048576` | Responses beyond this are returned uncached. |
| `API_IDEMPOTENCY_SWEEP_INTERVAL_SECS` | `300` | PG-only: cadence for the `evict_expired` background sweep. `0` disables. `< 60` triggers a startup `WARN`. |

**Store-backend tradeoffs** (see `docs/adr/0082-api-webhooks-idempotency.md`, ADR-0048 consolidated):

| Backend | When | Restart-survival | Multi-replica share |
|---|---|---|---|
| `memory` | Dev / single-process tests / one-replica deployments | No — state lost on restart | No — process-local |
| `postgres` | Production deployments (≥ 2 replicas, or restart-tolerance required) | Yes — table survives restart | Yes — same `DATABASE_URL` shared |

> **Operator warning:** selecting `memory` outside `NEBULA_ENV=development`
> emits a startup `tracing::warn!` — dedup state is lost on restart and
> across runners. The §M3 1.0 closure criterion requires `postgres` for
> production.

The `postgres` backend is gated behind the `nebula-api/postgres` cargo
feature so default builds remain lightweight. Selecting
`API_IDEMPOTENCY_BACKEND=postgres` without that feature compiled in fails
closed at startup (per ADR-0048; no silent fallback).

### Cross-layer schema strategy

Per ADR-0047 §3, API DTOs MUST NOT embed types from `nebula-core`,
`nebula-storage`, `nebula-engine`, or `nebula-credential`. Cross-layer
types (`OrgRole`, `WorkspaceRole`) are wrapped at the API boundary as
`OrgRoleDto` / `WorkspaceRoleDto` so the spec is decoupled from internal
type evolution.

## Non-goals

- Not a storage driver — no SQL or schema knowledge; see `nebula-storage`.
- Not a workflow engine — no execution logic; see `nebula-engine` / `nebula-runtime`.
- Not an expression evaluator — see `nebula-expression`.
- Not an outbound HTTP client — webhook *delivery* outbound lives in action
plugins, not here. This module handles inbound receipt only.
- WebSocket / SSE for real-time execution updates — not yet wired end-to-end
(§4.5: hidden until the engine side exists).

## Maturity

See `docs/MATURITY.md` row for `nebula-api`. Short summary:

- API stability: frontier
- JWT validation middleware, rate limiting, and pagination are implemented;
some route handlers contain TODO stubs pending full business logic (§4.5).
- Webhook transport is functional but delivery guarantee is at-least-once;
exactly-once is not claimed.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.5 (operational honesty), §12.2 (durable
outbox), §12.3 (local-first), §12.4 (RFC 9457 errors), §13 (knife scenario
steps 1–3 and 5).
- Satellite docs: `docs/INTEGRATION_MODEL.md`, `docs/OBSERVABILITY.md`.
- Siblings: `nebula-storage` (port impls), `nebula-engine` (execution logic),
`nebula-runtime` (action registry), `nebula-plugin` (plugin registry),
`nebula-credential` (secret store), `nebula-metrics` (metric primitives,
naming policy, and Prometheus export).

## Appendix

### Source layout

```
src/
├── lib.rs              # Crate root, public re-exports
├── app.rs              # build_app: OpenApiRouter merge + split_for_parts + middleware stack + serve()
├── state.rs            # AppState (builder) + API-tier port traits (OrgResolver/WorkspaceResolver/
│                       # MembershipStore/SessionStore/AuthBackend etc.)
├── openapi/
│   └── mod.rs          # OpenApiDoc + spec assembly
├── telemetry_init.rs   # init_api_telemetry()
├── trace_capture.rs
├── config/             # Was 1123-line config.rs — split into:
│   ├── mod.rs          # ApiConfig re-exports
│   ├── jwt.rs          # JwtSecret (32-byte min enforcement)
│   ├── errors.rs       # ConfigError
│   ├── sub.rs          # TlsConfig, CookieConfig, CorsConfig, VersioningConfig, PaginationConfig
│   └── env.rs          # ApiConfig::from_env loader
├── error/              # Was 759-line errors.rs — split into:
│   ├── mod.rs          # ApiError (§12.4 seam, #[non_exhaustive])
│   ├── problem.rs      # ProblemDetails (RFC 9457 envelope)
│   └── classify.rs     # HTTP-status mapping helpers
├── extractors/
│   ├── mod.rs
│   ├── json_extractor.rs  # ValidatedJson
│   └── credential.rs
├── middleware/
│   ├── mod.rs
│   ├── auth.rs            # JWT + API-key auth → AuthContext
│   ├── tenancy.rs         # Tenant resolution from path (org/workspace)
│   ├── rbac.rs            # Role-based access control checks
│   ├── csrf.rs            # CSRF token validation
│   ├── rate_limit.rs      # Rate limiting
│   ├── request_id.rs      # Unique request ID propagation
│   ├── security_headers.rs
│   ├── trace_w3c.rs       # W3C Trace Context (traceparent/tracestate) — ADR-0050
│   ├── internal_auth.rs   # X-Internal-Token gate
│   └── idempotency/       # Was 1224-line idempotency.rs — split into:
│       ├── mod.rs
│       ├── layer.rs       # IdempotencyLayer Tower middleware
│       ├── store.rs       # IdempotencyStore trait
│       ├── memory.rs      # InMemoryIdempotencyStore
│       └── key.rs         # IdempotencyKey construction
├── domain/             # Per-domain handlers + DTOs + routes (§12.7 knife seam)
│   ├── mod.rs          # create_routes + build_openapi_router assembly
│   ├── shared.rs       # CursorParams, PaginatedResponse, PaginationParams,
│   │                   # AckResponse, OrgRoleDto, WorkspaceRoleDto
│   ├── workspace.rs    # Tenant-prefix nesting: merges workflow/execution/resource/credential routers
│   ├── internal.rs     # /internal/v1/* ops (plain axum Router; X-Internal-Token)
│   ├── metrics.rs      # Prometheus scrape endpoint
│   ├── workflow/
│   │   ├── mod.rs
│   │   ├── handler.rs  # §13 seam: create_workflow, activate_workflow, start_execution
│   │   └── dto.rs
│   ├── execution/
│   │   ├── mod.rs
│   │   ├── handler.rs  # §13 seam: cancel_execution
│   │   └── dto.rs
│   ├── credential/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   ├── dto.rs
│   │   └── oauth.rs
│   ├── catalog/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   └── dto.rs
│   ├── auth/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   └── backend/    # AuthBackend impls (in-memory, session, password, MFA, OAuth, PAT)
│   ├── org/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   └── dto.rs
│   ├── me/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   └── dto.rs
│   ├── health/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   └── dto.rs
│   └── resource/
│       ├── mod.rs
│       ├── handler.rs
│       └── dto.rs
└── transport/          # Protocol transports (was services/ — NOT business services)
    ├── mod.rs
    ├── credential.rs   # Plane-B credential CRUD stubs (Phase 4 will implement)
    ├── oauth/          # OAuth2 flow transport
    │   ├── mod.rs
    │   ├── flow.rs
    │   ├── http.rs
    │   └── state.rs
    └── webhook/        # Inbound trigger transport (§11.3 / §13.4)
        ├── mod.rs
        ├── transport.rs  # WebhookTransport — activate/deactivate/router
        ├── bootstrap.rs  # bootstrap_webhook_activations, WebhookSecretResolver
        ├── dispatch.rs   # dispatch_inner pipeline
        ├── events.rs     # TriggerLifecycleEvent + subscriber
        ├── provider.rs   # EndpointProviderImpl
        ├── routing.rs    # RoutingMap (private, DashMap)
        ├── signature.rs  # Signature enforcement (ADR-0022)
        ├── replay.rs     # Replay-window check
        ├── ratelimit.rs  # WebhookRateLimiter — per-key sliding-window + LRU cap
        └── key.rs        # WebhookKey: Programmatic { uuid, nonce } | Slug(TriggerCoordinates)
```

### Startup example

`nebula-api` is a **pure library** — it ships no composition root and no
binary. The canonical wiring lives outside this crate:

- `examples/examples/api_simple_server.rs` — the minimal runnable startup
  (run: `cargo run -p nebula-examples --example api_simple_server`).
- `apps/server` — the production composition root (the single
  `nebula-server` binary; see the **Transport binaries** section below).

`AppState::new` takes the **spec-16 storage-port** handles
(`WorkflowStore` + `WorkflowVersionStore` + `ExecutionStore` +
`NodeResultStore` + `ExecutionJournalReader` + `ControlQueue`), each
wrapped in the `nebula-tenancy` scope-enforcing decorator by the
composition root before it reaches `AppState` — never a raw legacy
`ExecutionRepo` / `WorkflowRepo`. Consult `apps/server/src/compose.rs`
for the authoritative, current shape rather than duplicating it here
(this README intentionally does not inline a startup snippet that would
drift from the composition root — ADR-0072).

### Transport binaries

`nebula-api` is a **pure library** — it ships no binaries. The composition root
and the single `nebula-server` binary live in the `apps/server` workspace member.

`nebula-server` selects the active transport(s) via `--transport` (or `NEBULA_TRANSPORT`
env var). Valid values: `api`, `webhook`, `realtime`, `all` (default).

Run locally:

```bash
# All transports (default)
cargo run -p nebula-server

# REST API transport only
cargo run -p nebula-server -- --transport=api

# Webhook ingress only
cargo run -p nebula-server -- --transport=webhook

# Realtime scaffold only (/ws currently returns 501 — Phase 5)
cargo run -p nebula-server -- --transport=realtime
```

### Error format (RFC 9457)

```json
{
  "type": "https://nebula.dev/problems/not-found",
  "title": "Not Found",
  "status": 404,
  "detail": "Workflow abc123 not found"
}
```

### Endpoint reference

Every row in this table corresponds to a real mounted route. Rows marked
`(honest 501)` are mounted, return 501 per ADR-0047 Stub Endpoint Policy,
and carry `#[deprecated]` + ` (planned)` tag in the OpenAPI spec. Rows
marked `(honest 503)` are mounted but refuse rather than fake a capability
that requires an unwired subsystem. See the Stub Endpoint Policy section
above for the enforcement guarantee.

| Method   | Path                                                                      | Description                                                                |
| -------- | ------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `GET`    | `/health`                                                                 | Liveness check (always available)                                          |
| `GET`    | `/ready`                                                                  | Readiness check (verifies dependencies)                                    |
| `GET`    | `/version`                                                                | Version info                                                               |
| `POST`   | `/api/v1/auth/signup`                                                     | Account registration                                                       |
| `POST`   | `/api/v1/auth/login`                                                      | Session login                                                              |
| `POST`   | `/api/v1/auth/logout`                                                     | Session logout                                                             |
| `POST`   | `/api/v1/auth/forgot-password`                                            | Initiate password reset                                                    |
| `POST`   | `/api/v1/auth/reset-password`                                             | Complete password reset                                                    |
| `POST`   | `/api/v1/auth/verify-email`                                               | Verify email address                                                       |
| `POST`   | `/api/v1/auth/mfa/enroll`                                                 | Enrol a MFA device (session + CSRF)                                        |
| `POST`   | `/api/v1/auth/mfa/verify`                                                 | Confirm MFA enrollment (session + CSRF)                                    |
| `POST`   | `/api/v1/auth/login/mfa`                                                  | Complete second-factor login (cookie-less, CSRF-exempt)                    |
| `GET`    | `/api/v1/auth/oauth/{provider}`                                           | Start OAuth2 login flow                                                    |
| `GET`    | `/api/v1/auth/oauth/{provider}/callback`                                  | OAuth2 login callback                                                      |
| `GET`    | `/api/v1/me`                                                              | Current user profile                                                       |
| `PUT`    | `/api/v1/me`                                                              | Update current user profile                                                |
| `GET`    | `/api/v1/me/orgs`                                                         | List orgs the caller belongs to (real — shared `MembershipStore`)          |
| `GET`    | `/api/v1/me/tokens`                                                       | List personal access tokens                                                |
| `POST`   | `/api/v1/me/tokens`                                                       | Create personal access token                                               |
| `DELETE` | `/api/v1/me/tokens/{token_id}`                                            | Revoke a personal access token                                             |
| `GET`    | `/api/v1/actions`                                                         | List action catalog                                                        |
| `GET`    | `/api/v1/plugins`                                                         | List plugin catalog                                                        |
| `GET`    | `/api/v1/orgs/{org}`                                                      | Get org by slug or ID `(honest 501)`                                       |
| `PATCH`  | `/api/v1/orgs/{org}`                                                      | Update org settings `(honest 501)`                                         |
| `DELETE` | `/api/v1/orgs/{org}`                                                      | Delete org `(honest 501)`                                                  |
| `GET`    | `/api/v1/orgs/{org}/members`                                              | List org members (real — shared `MembershipStore`)                         |
| `POST`   | `/api/v1/orgs/{org}/members`                                              | Add member by principal id (real — Option 1 honest contract, org-admin)    |
| `DELETE` | `/api/v1/orgs/{org}/members/{principal}`                                  | Remove member (real — abuse-safe: last-admin/role-precedence/IDOR guards)  |
| `GET`    | `/api/v1/orgs/{org}/service-accounts`                                     | List service accounts `(honest 501)`                                       |
| `POST`   | `/api/v1/orgs/{org}/service-accounts`                                     | Create service account `(honest 501)`                                      |
| `DELETE` | `/api/v1/orgs/{org}/service-accounts/{sa}`                                | Delete service account `(honest 501)`                                      |
| `GET`    | `/api/v1/orgs/{org}/workspaces/{ws}/workflows`                            | List workflows (tenant-scoped)                                             |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/workflows`                            | Create workflow (§13 step 1)                                               |
| `GET`    | `/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}`                       | Get workflow by ID                                                         |
| `PUT`    | `/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}`                       | Update workflow                                                            |
| `DELETE` | `/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}`                       | Delete workflow                                                            |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/activate`              | Activate workflow — runs validation (§13 step 2)                           |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/execute`               | Trigger workflow execution — 202 Accepted (§13 step 3)                     |
| `GET`    | `/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/executions`            | List executions for a workflow                                             |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/executions`            | Start execution — 202 Accepted (§13 step 3)                                |
| `GET`    | `/api/v1/orgs/{org}/workspaces/{ws}/executions`                           | List all executions in workspace                                           |
| `GET`    | `/api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}`                    | Get execution status                                                       |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}/cancel`             | Cancel execution — durable signal (§13 step 5)                             |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}/terminate`          | Terminate execution — durable signal (§12.2)                               |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}/restart`            | Restart execution `(honest 501)`                                           |
| `GET`    | `/api/v1/orgs/{org}/workspaces/{ws}/resources`                            | List resources `(honest 501)`                                              |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/resolve`                  | Start generic credential resolve `(honest 503)`                            |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/resolve/continue`         | Continue multi-step credential resolve `(honest 503)`                      |
| `GET`    | `/api/v1/orgs/{org}/workspaces/{ws}/credentials`                          | List credentials (metadata only)                                           |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials`                          | Create credential (write-only secret)                                      |
| `GET`    | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}`                   | Get credential metadata                                                    |
| `PUT`    | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}`                   | Update credential                                                          |
| `DELETE` | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}`                   | Delete credential                                                          |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/test`              | Test credential `(honest 503)`                                             |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/refresh`           | Refresh credential token `(honest 503)`                                    |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/revoke`            | Revoke credential `(honest 503)`                                           |
| `POST`   | `/webhooks/{trigger_uuid}/{nonce}`                                         | Inbound webhook trigger (mounted when `webhook_transport` is set)          |
| `GET`    | `/api/v1/openapi.json`                                                    | OpenAPI 3.1 specification document                                         |
| `GET`    | `/api/v1/docs/`                                                           | Swagger UI (self-hosted)                                                   |


