---

## name: nebula-api
role: API Gateway
status: frontier
last-reviewed: 2026-07-21
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
`TlsConfig`, `CorsConfig`, `VersioningConfig`,
`PaginationConfig`. Startup fails hard on a missing or short JWT secret
(no `Default` impl).
- `OAuthIdentityRuntime::from_config` / `OAuthRuntimeBuildError` — narrow,
  technical composition exports for constructing the optional opaque Plane-A
  runtime. The runtime exposes no raw HTTP client, provider responses, bearer
  tokens, cache, or admission controls. `nebula-sdk` remains the sole supported
  and branded Rust product surface; direct `nebula-api` use is an implementation
  boundary.
- Technical auth types `OAuthProvider` and `AuthError` are
  `#[non_exhaustive]`; downstream composition code must retain a wildcard arm
  so new providers and failures can be added without another exhaustive-match
  break.
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
`WebhookSecretResolver`, `WebhookActivationContextFactory`. The API owns these
object-safe, credential-neutral ports; the composition root supplies concrete
adapters and invokes bootstrap before `build_app`. Admin reload uses
`collect_*` and `replace_slug_map` for atomic swaps.
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

Per ADR-0085 (design records are maintained in the maintainers' private
design vault, not in this public repository):
operator IdP-client credentials are configured via environment
variables (NOT a database table, NOT the credential store). Declaring
a provider is opt-in; an undeclared provider returns
`AuthError::ProviderNotConfigured` → HTTP 503 from
`GET /api/v1/auth/oauth/{provider}`.

Supported production profiles in 1.0 (`#[non_exhaustive] OAuthProvider` enum):
canonical Google OIDC (`google`) and GitHub.com (`github`). Microsoft,
Auth0/Okta/generic OIDC, GitHub Enterprise Server, custom endpoints, and
operator-supplied JWKS are parked. They require a reviewed application change,
not an environment switch; legacy/profile override variables fail boot with a
secret-free configuration error.

### Configuration shape

Operator configuration is credentials-only:

| Profile | Required env vars | Runtime-owned policy |
|---|---|---|
| Google | `API_AUTH_OAUTH_GOOGLE_CLIENT_ID`, `API_AUTH_OAUTH_GOOGLE_CLIENT_SECRET` | Canonical Google discovery URL, pinned issuer, `openid email profile` scopes |
| GitHub.com | `API_AUTH_OAUTH_GITHUB_CLIENT_ID`, `API_AUTH_OAUTH_GITHUB_CLIENT_SECRET` | Canonical GitHub.com authorize/token/user/emails URLs, `user:email` scope |

Either credential variable declares the profile; an incomplete pair fails boot.
Endpoint, scope, token-auth, JWKS, and insecure-localhost override variables are
rejected even when empty, so stale deployment configuration cannot silently
change the runtime profile.

Every token request uses exactly one client-authentication method. GitHub.com
uses its fixed `client_secret_post` profile. Google selects from
`token_endpoint_auth_methods_supported`: `client_secret_basic` is preferred,
`client_secret_post` is the fallback, and an omitted field uses the OIDC Basic
default. For Basic, each credential component is first
application/x-www-form-urlencoded, then joined with `:` and Base64-encoded per
RFC 6749 section 2.3.1. Client credentials are never also placed in the form.

### `API_PUBLIC_URL`

The Plane-A OAuth flow derives `redirect_uri` from `API_PUBLIC_URL`
(NOT a per-provider config) per ADR-0085 D-3 (recon-4):
`{API_PUBLIC_URL}/api/v1/auth/oauth/{provider}/callback`. The
`/api/v1` prefix matches the actual router mount point in
`crates/api/src/domain/mod.rs`. Operators that need multiple callback
URIs deploy multiple Nebula instances. Boot fails closed if
`API_PUBLIC_URL` is not a canonical external base URL while any provider is
declared: it may contain a normalized reverse-proxy mount prefix such as
`/nebula`, but never credentials, query, fragment, dot segments, encoded path
separators, or ambiguous empty path segments. For example,
`https://app.example.com/nebula` derives
`https://app.example.com/nebula/api/v1/auth/oauth/{provider}/callback`.
Release builds require HTTPS and reject localhost/loopback, while debug builds
additionally permit HTTP localhost.

The start and callback requests must arrive on that configured authority,
including its effective port. Reverse proxies must preserve the public `Host`
instead of rewriting it to an internal upstream name. OAuth transaction
cookies are host-bound (web cookies are not port-bound), so a dedicated,
trusted auth hostname is recommended when unrelated services otherwise share
one hostname.

### Example: Google + GitHub side-by-side

```bash
# Required by all Plane-A OAuth flows.
export API_PUBLIC_URL=https://app.example.com

# Google — canonical OIDC profile; endpoints/scopes are fixed by Nebula.
export API_AUTH_OAUTH_GOOGLE_CLIENT_ID="..."
export API_AUTH_OAUTH_GOOGLE_CLIENT_SECRET="..."

# GitHub.com — canonical endpoints and user:email scope are fixed by Nebula.
export API_AUTH_OAUTH_GITHUB_CLIENT_ID="Iv1.xxxxxxxx"
export API_AUTH_OAUTH_GITHUB_CLIENT_SECRET="..."
```

### Flow

1. A same-site browser client `GET /api/v1/auth/oauth/{provider}` → server
   derives `redirect_uri`, mints a PKCE pair, persists `OAuthStateRow`, sets a
   per-flow `Secure; HttpOnly; SameSite=Lax; Path=/` `__Host-` transaction
   cookie, and returns the IdP `authorize_url` + opaque `state`.
2. Client redirects user's browser to `authorize_url`.
3. IdP redirects back with `state` and exactly one bounded `code` or `error`.
4. The user's browser follows the IdP redirect to
   `GET /api/v1/auth/oauth/{provider}/callback`, carrying that one transaction
   cookie. Missing, duplicate, swapped, or malformed bindings fail before the
   backend sees the state or code.
5. After the exact browser binding is accepted, the server clears its cookie
   on every terminal backend outcome and atomically consumes the live matching
   `(state, provider, redirect_uri)` row. A later upstream failure does not
   resurrect that state. For a code callback, token exchange and primary
   identity verification run without database locks under one 30-second
   callback-network deadline. An existing subject link may be finalized
   immediately; otherwise optional verified-email egress uses the same original
   deadline before a short finalizer atomically commits user/link/session.
6. A valid provider-error callback consumes the matching state without token or
   userinfo egress and returns a fixed 401. Provider `error`,
   `error_description`, and `error_uri` text is ignored and never logged.
7. If the provider identity is valid but a first-link flow has no
   policy-acceptable verified email, the callback returns 403
   `EmailNotVerified` and writes no link/session. Transport failures, non-success
   responses, or malformed provider identity payloads remain fixed 502 failures.
8. If a verified email already belongs to an account but the provider subject
   is not linked, finalization rolls back with 409 `AccountLinkRequired`; email
   possession never auto-links and no session is created.
9. If the authoritative linked user has MFA enabled, finalization atomically
   records an opaque, single-use MFA challenge and returns 202. No session row,
   session cookie, or CSRF cookie is created until the caller completes
   `POST /api/v1/auth/login/mfa`.

Clients that do not use a browser cookie jar must preserve the start response's
`Set-Cookie` value and present it only on the matching callback. A request that
already carries eight names with the Nebula OAuth transaction-cookie prefix
receives 429 before a new server-side state is created. This is a request-local
Cookie-header bound; it is not a globally atomic browser quota. Cross-site
subresource login starts are deliberately unsupported by this Lax cookie
contract; deploy the UI and API same-site or initiate through the canonical
site. Independent flows use independent cookie names and may complete in any
order.

Separately, OAuth-state admission has a hard global bound of 10,000 live rows
per process (Memory) or shared PostgreSQL deployment. Capacity check and insert
are one fail-closed admission operation: a full or contended gate returns 429
without issuing state, PKCE material, or a transaction cookie.

### Security posture (REQ-obs-001)

- **One opaque owner**: `ApiConfig` temporarily owns parsed `SecretString`
  credentials during load-time validation. `apps/server` moves that map into
  `OAuthIdentityRuntime::from_config`; afterward the runtime is the sole
  long-lived owner of credentials, fixed profiles, HTTP/DNS policy, Google
  discovery state, outbound concurrency, and network-deadline policy. The
  router config retains an empty OAuth map, and the selected backend receives
  the same opaque `Arc`, never raw configuration, duplicate secrets, or a
  replaceable client. An empty map constructs neither runtime nor HTTP client.
- **Connect-time SSRF protection**: all server-fetched endpoints use the fixed
  rustls client with HTTPS-only mode and redirects, retries, proxy discovery,
  referer emission, and connection-verbose logging disabled. Literal IPs and
  DNS names share one policy. DNS must return 1–32 addresses and every answer
  must be globally routable; an empty, oversized, private, special-use, or
  mixed global/non-global answer set is rejected. Reqwest receives only the
  exact validated socket addresses, so it cannot perform a second unguarded
  resolution. The two production authorization profiles are HTTPS-only; a
  private test-only constructor can admit an explicit debug localhost fixture
  without creating a production configuration escape hatch.
- **Unambiguous token authentication**: every exchange emits exactly one
  provider client-authentication method, never credentials in both the Basic
  header and form body. GitHub.com uses fixed `client_secret_post`; Google
  prefers discovered `client_secret_basic`, falls back to
  `client_secret_post`, applies the OIDC Basic default when metadata omits the
  field, and fails closed on unsupported-only metadata. The Basic path
  form-encodes each credential component before the colon/Base64 step.
- **Browser-bound state**: PKCE and globally stored state are not treated as a
  browser-session binding. Each start creates a versioned, provider/state-bound
  `__Host-` transaction cookie; callback validation happens before state
  consumption or provider egress, compares the exact cookie value, rejects
  duplicate-name shadowing, and clears an accepted binding on every terminal
  backend outcome. A request carrying eight recognized cookie names cannot
  create another flow; concurrent responses may temporarily exceed that
  request-local browser bound. The separate global state-admission gate is hard:
  at most 10,000 live rows per Memory process or shared PostgreSQL deployment,
  with full/contended admission returning 429 before state creation.
- **Bounded secret lifetime**: every discovery, token, userinfo, and verified-
  email response is read into a preallocated, zeroizing buffer capped at 256
  KiB. Provider JSON is parsed into owned values and the raw body is dropped
  immediately. The access token remains inside a non-cloneable, non-debuggable
  capability until the optional verified-email lookup consumes it; only the
  normalized Nebula user, `(provider, subject) → user_id` link, and Nebula
  session are persisted. Provider access, ID, and refresh tokens are not.
- **Runtime-local discovery**: the fixed Google profile has one
  cache/singleflight slot (one-hour success TTL, five-second failure cooldown).
  Followers wait for the leader before acquiring an outbound permit, so a cold
  profile cannot multiply egress pressure. GitHub.com uses reviewed fixed URLs
  and performs no discovery.
- **One callback-network budget**: one 30-second absolute deadline is created
  after atomic state consumption and reused across Google discovery/admission,
  DNS, token exchange, userinfo, and optional verified-email lookup. Finalizer
  calls are short storage operations outside that timeout and never hold locks
  across provider egress.
- **Secret-free diagnostics**: internal OAuth failures use a closed,
  low-cardinality code set and map to fixed RFC 9457 responses. Global request
  spans record only HTTP method plus the matched route template (or the fixed
  `<unmatched>` marker), never the raw URI/query; inbound W3C parent context is
  preserved. Callback `code` and `state` are redacted from `Debug` output.
- **Google direct-TLS ID-token validation**: Google requires an ID token and
  validates compact/header shape, `RS256`, pinned issuer, exact audience/`azp`,
  bounded `exp`/`iat`, nonce, `at_hash`, and equality between ID-token and
  userinfo subjects. Local cryptographic signature verification against JWKS is
  explicitly deferred: the discovered JWKS URL is policy-validated but not
  fetched, and signature bytes are syntax/size checked only. Thus not all
  ID-token or issuer validation is deferred; TLS to the fixed token endpoint is
  the authenticity boundary for this increment.
- **Verified-email outcome boundary**: once a stable provider subject is valid,
  absence of a policy-acceptable verified email for a first link is a semantic
  403 `EmailNotVerified`, not a provider outage. It creates no link or session.
  Network/non-success responses and malformed provider identity payloads remain
  fixed 502 upstream failures; provider-controlled detail never crosses the
  RFC 9457 boundary.
- **No email auto-link**: an existing `(provider, subject)` link is
  authoritative. An unused verified email may create a new OAuth-only account;
  an email owned by any existing account produces 409 `AccountLinkRequired`,
  rolls back the finalizer, and creates no session. Linking requires a separate
  authenticated capability.
- **MFA-preserving OAuth**: an authoritative linked user with MFA enabled gets
  202 plus an opaque challenge. The finalizer records the challenge and
  MFA-required outcome atomically and creates no session/CSRF material; only
  `/auth/login/mfa` may consume it and mint the session.
- **Provider denial**: a provider-error callback with a valid browser binding
  atomically cancels the state, performs no egress, clears the accepted cookie,
  and returns a fixed 401. Provider-controlled error text is ignored.
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
- `security_schemes_match_adr_0047` — `bearer` / `api_key` /
  `session_cookie` / `csrf`.
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

### `me/*` and Plane-A auth durability (canon §11.6 / §11.5)

The profile, PAT, password, MFA, session, and Plane-A OAuth paths are implemented
for both selectable identity backends. `API_AUTH_BACKEND` defaults to `memory`;
`postgres` is available when `nebula-server` is built with the `postgres` feature
and `DATABASE_URL` is reachable. An explicitly requested Postgres backend fails
closed instead of silently falling back to memory.

| Backend | Restart-survival | Multi-replica share | Intended use |
|---|---|---|---|
| `memory` | **No** | **No** | Local development and tests |
| `postgres` | **Yes** | **Yes**, for replicas using the same database | Durable production identity |

The Postgres implementation persists users, sessions, PATs, verification tokens,
OAuth state, and external identity links. OAuth state is consumed atomically
with provider and expiry predicates; an expired-state cleanup is also attempted
when a new flow starts. This identity
backend does not imply tenant-directory or membership policy: the default server composition
leaves both `WorkspaceResolver` and org membership unwired. Supported operator-supplied
directory and `MembershipStore` paths remain K4 work.

### Credential CRUD durability (canon §11.6 / §12.5)

Every credential operation — CRUD (`create` / `get` / `update` /
`delete` / `list`), lifecycle (`test` / `refresh` / `revoke`), and
acquisition (`resolve` / `resolve/continue`) under
`…/workspaces/{ws}/credentials` — routes through the
API-owned object-safe **`CredentialCommandGateway`**. Middleware supplies a private-field
`AuthenticatedPrincipal` and the resolved request `Scope`; handlers submit public intent only.
The first-party adapter in `apps/server` maps those claims into the credential-owned
`CredentialController`, which obtains exactly one decision from its injected
`CredentialTenantAuthority` before deriving an owner partition or calling `CredentialService`.
That authority re-reads one consistent role snapshot from the same `MembershipStore` used by HTTP
RBAC, maps the command to
`CredentialRead` / `CredentialWrite` / `CredentialDelete`, and fails unavailable when the policy
source is unwired or unreadable. A valid snapshot without organization membership denies the
command; the route's Access Kernel guard separately enforces the authenticated token grant.
Production key, persistence, catalog, refresh, and authority adapters live in `apps/server`.
`ports::credential_service_factory` is compiled only as an unsupported `test-util` fixture. The
service runs the typed validate→resolve pipeline and persists only through owner-bound
`CredentialPersistence`.
Handlers do not run a competing `CredentialSchemaPort` precheck: that port serves catalog/form
schema reads only. Its absence does not block create/update/resolve or produce a mutation 503;
authorized service validation is the single mutation authority.
The universal `resolve` / `resolve/continue` endpoints are the only
credential-acquisition HTTP contract. The former raw Plane-B
`credentials/{id}/oauth2/{auth,callback}` ceremony is parked and returns
404; provider-specific interaction must be represented through the
facade's typed pending interaction before it can become a supported
surface. Accordingly, the default registry/catalog does not register or
advertise `oauth2`; attempts to create or resolve that key fail as an unknown
credential type. When no command gateway is wired, every credential endpoint returns an honest
503 — there is no service/store fallback.

| Aspect | First-party credential storage composition (after membership authority is provisioned) |
|---|---|
| Restart-survival | **Yes for completed credentials** — `NEBULA_CRED_DB` selects the default file-backed SQLite store or PostgreSQL; in-flight pending interactions remain ephemeral |
| Multi-replica share | **Yes with PostgreSQL** — build `nebula-server` with `--features postgres` and set `NEBULA_CRED_DB=postgres://…`; the credential rows and refresh-claim repository share one admitted credential-owned pool. SQLite remains instance-local. |
| Encryption at rest | **Yes** — the facade composes the `EncryptionLayer` adjacent to the backend (AES-256-GCM; key from `NEBULA_CRED_MASTER_KEY`, fail-closed) |
| Cross-workspace isolation | **Yes once policy is provisioned** — authority verifies workspace existence/parentage, revalidates membership/role, reproduces the authenticated scope, and every persistence predicate uses the derived `(owner, credential_id)` selector; cross-workspace IDs collapse to a flat 404. The default server has no workspace-directory or membership source and returns 503 before this path. |
| Lifecycle dispatch | **Live** — `test`/`refresh`/`revoke` dispatch the registered type's capability; a type without it is refused with 400 (capability gate), never a faked success. Provider rejection requiring an integration reconnect is the typed 409 `API:CREDENTIAL_REAUTH_REQUIRED`, not Plane-A 401. The test response is a tagged `status` union: success has no code; failure requires a frozen v1, payload-free code, and future core codes map to `other`. |

> **Operator warning:** completed credentials survive a normal process restart.
> The default SQLite database is not shared across replicas; use the explicit
> PostgreSQL `NEBULA_CRED_DB` profile for multi-replica credential and refresh
> coordination. Pending acquisition state is still process-local and expires
> after at most ten minutes; an interrupted interactive flow must be restarted.
>
> The tenancy path resolver special-cases the literal `resolve`
> sub-route, so `resolve` / `resolve/continue` are **not** shadowed by
> the `{cred}` matcher — they reach the handler; the genuine
> `/credentials/{cred}` position stays strictly ULID-validated.

### Org membership durability (canon §11.6 / §11.5)

The org **member** endpoints (`GET`/`POST`/`DELETE` under
`…/orgs/{org}/members`) and the membership-backed `me/*` reads
(`GET /me/orgs`, `MeResponse.orgs_count`) are **implemented and tested
end-to-end** (`crates/api/tests/org_e2e.rs`) against the in-memory
`MembershipStore` (`nebula_api::domain::org::InMemoryMembershipStore`) —
the **single shared store** `rbac_middleware` also consults, so an
`add_member` is immediately visible to the next RBAC check (no
propagation window). `nebula-storage-port` has a generic row-level membership
store with backend implementations, but no adapter currently satisfies this
API port's consistent authorization snapshot and atomic guarded-mutation
contract. It must not be wired directly as request authority. The in-memory
implementation is the §4.5-honest reference backing, with the same
restart/replica limits as `API_AUTH_BACKEND=memory`; unlike Plane-A identity,
the API policy port has no selectable PostgreSQL implementation yet.

**The default `nebula-server` binary does NOT auto-wire a
`MembershipStore`.** The current in-memory seam is an internal/reference
composition capability, not a supported operator deployment path; that path remains K4 work.
This is the same fail-honest posture as other unavailable capabilities: never silently fake
policy. Rationale: wiring a
`MembershipStore` is required by RBAC on every org/workspace route and by
credential command authority (a caller with no org role is 404'd once the
store is provisioned). The default `AuthBackend` is an *empty*
`InMemoryAuthBackend` (no users; `register_user` mints a **random**
`UserId`), so **no principal could authenticate as any auto-seeded
bootstrap owner** — an auto-seeded store would 404-deadlock every
org/workspace route (a deployment-level §4.5 false capability), and a
hardcoded auto-seeded admin identity would be a default-credential /
privileged-by-default surface (canon §12.5). Both are strictly worse
than honest degradation.

| Aspect | Org membership (in-memory `MembershipStore`) |
|---|---|
| Default binary | **Unwired (`None`)** — every org/workspace route returns an honest **503** before its handler; credential authority also fails unavailable |
| Restart-survival | **No** — memberships are lost on restart (once provisioned) |
| Multi-replica share | **No** — state is process-local |
| Provisioning | Internal tests/reference composition may wire `AppState::with_membership_store(...)` and register the same bootstrap-owner identity in the wired `AuthBackend`. `InMemoryMembershipStore::seeded_bootstrap` is a technical helper, not a supported integration surface. The default binary has no operator configuration; supported deployment composition remains K4 work. |

> **Operator warning:** in the default binary, `GET`/`POST`/`DELETE
> `…/orgs/{org}/members` (and `GET /me/orgs` / `orgs_count`) return
> **503** until composition provides a `MembershipStore`. This is honest
> degradation: it deliberately avoids a default admin credential and never
> treats missing policy state as access. Internal/reference API composition can
> exercise the seam by wiring `with_membership_store(...)` with a
> bootstrap owner that is **also** a registered, authenticatable
> principal in its `AuthBackend`; this is not a supported downstream deployment
> recipe. Once provisioned, RBAC applies role
> enforcement on every
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

State-changing endpoints authenticated by the host-bound session cookie are
gated by `csrf_middleware`
(`crates/api/src/middleware/csrf.rs`). The double-submit-cookie pattern:

- On login the API issues two host-bound cookies:
  `__Host-nebula-session` (HttpOnly) and `__Host-nebula-csrf` (readable by
  the SPA). Both are `Secure`, `SameSite=Lax`, `Path=/`, carry no `Domain`,
  and share the backend's 14-day session lifetime. The fixed policy is
  deliberate: Nebula no longer advertises cookie knobs the runtime ignores.
- Every state-changing request (`POST`/`PUT`/`PATCH`/`DELETE`) must echo
  the `__Host-nebula-csrf` value back in an `X-CSRF-Token` request header.
- `X-CSRF-Token` is part of the canonical CORS allow-header policy, so an
  origin explicitly admitted with credentialed CORS can complete preflight.
- The session-cookie lane is intentionally **schemeful same-site only**.
  Allowing an origin through CORS does not override `SameSite=Lax`; a truly
  cross-site SPA will not receive ambient session authority and must use an
  explicit Bearer credential. Deploy the browser UI and API under the same
  site when cookie sessions are required.
- The middleware rejects the request with `403 Forbidden` when the
  header is missing or does not byte-match the cookie.

**Route table** (verified by `crates/api/tests/{me_e2e,seam_credential_write_path_validation,auth_mfa_csrf}.rs`):

| Route group | Method gate | CSRF | Rationale |
|---|---|---|---|
| `/api/v1/me/*` | `POST`/`PUT`/`PATCH`/`DELETE` | **enforced for session auth** | ambient session cookie + state-changing |
| `/api/v1/orgs/{org}/workspaces/{ws}/credentials/*` | `POST`/`PUT`/`PATCH`/`DELETE` | **enforced for session auth** | ambient session cookie + state-changing |
| `/api/v1/orgs/{org}/*` (tenant-scoped) | `POST`/`PUT`/`PATCH`/`DELETE` | **enforced for session auth** | ambient session cookie + state-changing |
| `/api/v1/auth/mfa/enroll` | `POST` | **enforced** | requires a session created by primary authentication within the 10-minute freshness window, plus matching CSRF proof |
| `/api/v1/auth/mfa/verify` | `POST` | **enforced** | same fresh-session + CSRF authority; confirms the pending enrollment |
| `/api/v1/auth/login/mfa` | `POST` | **exempt by construction** | cookie-less second-factor login completion; `challenge_token` is the sole authority |
| `/api/v1/auth/logout` | `POST` | **exempt by deliberate choice** | revokes `__Host-nebula-session` when present; a CSRF attack can only force a sign-out (annoying, not a confidentiality / integrity breach). Keeps logout reachable when the CSRF cookie has drifted / been cleared. |
| `/api/v1/auth/{signup,login,forgot-password,reset-password,verify-email,oauth/*}` | `POST`/`GET` | **exempt by construction** | these routes never authorize from an ambient session; their explicit input/challenge is the sole authority |
| Any request authenticating via JWT/PAT `Authorization: Bearer …` or `X-API-Key` | any | **exempt by construction** | explicit header authority is not ambient browser authority; verified inside `csrf_middleware` from `AuthContext::auth_method` |

**Middleware order** — `auth_middleware` MUST be layered before
`csrf_middleware`. The latter reads the `AuthContext` extension that the
former installs to know whether to skip the gate for JWT/PAT/API-key callers.
The Plane-B credential CRUD/lifecycle/acquisition routes are part of the
tenant router, which wires the pair in `crates/api/src/domain/mod.rs`
(`auth_middleware` then `csrf_middleware`).

**Header contract** — callers send the matching token as
`X-CSRF-Token: <value>`. The middleware compares header against cookie
byte-for-byte; partial matches and case-normalised matches are rejected.
Missing header **or** missing cookie yields `403 "CSRF token missing"`;
mismatch yields `403 "CSRF token mismatch"`.

**Credential precedence** — explicit credentials are evaluated before the
ambient session cookie: `Authorization: Bearer …`, then `X-API-Key`, then
`__Host-nebula-session`. A present but malformed, expired, unknown, or duplicate
explicit credential fails with 401 and never falls back to a valid cookie. This
makes SDK/CLI behavior deterministic and prevents credential-confusion or
downgrade. Supplying both `Authorization` and `X-API-Key` is ambiguous and also
fails with 401. Duplicate session-cookie names and duplicate/empty CSRF authority
are rejected rather than accepting an arbitrary first value.

OpenAPI publishes the same alternatives: `bearer` (JWT or PAT), `api_key`, and
`session_cookie`; mutating cookie-auth operations require `session_cookie` and
`csrf` together in one security requirement. MFA enrollment/confirmation expose
only the fresh `session_cookie` + `csrf` lane.

### Cache policy for authority-bearing responses

The complete auth and MFA routers, PAT and service-account creation, webhook
registration, and interactive credential `resolve` / `resolve/continue` routes
are wrapped in `no_store_authority_response`. That route-level boundary applies
to successes and errors and overwrites weaker handler policy with
`Cache-Control: no-store`, `Pragma: no-cache`, and
`Referrer-Policy: no-referrer`. Keep the middleware on the whole sensitive
route subtree when adding a new branch; a per-handler header is not an
equivalent invariant because early extractor and routing failures would bypass
it.

This cache policy and the idempotency replay allow-list are independent
defenses. Authority-bearing routes are absent from replay admission, while the
response headers also veto storage if an approved route's response contract
later changes.

### MFA enrollment authority

MFA enrollment is a replacement protocol, not an in-place edit. `enroll`
creates or replaces one candidate with a ten-minute lifetime and returns its QR
material once; it does not change the active seed or `mfa_enabled`, including
when the account already has MFA. A wrong code leaves the live candidate
available for a corrected attempt. `verify` promotes only a live candidate;
success consumes it atomically, while expiry, replay, replacement, and a losing
concurrent confirmation fail closed without modifying the active factor.

The Memory backend serializes candidate installation process-locally. The
PostgreSQL backend persists candidates separately from `users` and owns the
consume-plus-active-update transaction. At the storage boundary candidate
secret material is an opaque envelope, so encryption/key rotation can evolve
without exposing a base32/plaintext contract to persistence.

### Idempotency-Key (M3.4 / ADR-0048)

Idempotent replay is fail-closed: only explicitly reviewed `POST` route
templates participate, and `IdempotencyLayer::new` starts with an empty
allow-list. First-party composition opts in authenticated membership,
resource, credential lifecycle, workflow, and execution-control operations
whose responses carry no one-time authority. Auth/session, PAT,
service-account, webhook-registration, and interactive credential-resolution
routes pass through normally even if a client sends `Idempotency-Key`; a new
route is non-replayable until reviewed and added deliberately.

For an approved route, clients opt in by sending an `Idempotency-Key` header on
a `POST` request. The middleware caches the first response (status + body +
filtered headers) keyed by
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
- Responses carrying `Set-Cookie` or `Cache-Control: no-store` are returned
  without buffering, persistence, or replay even on an approved route.

**Environment variables** (defaults applied by `ApiConfig::from_env`):

| Var | Default | Notes |
|---|---|---|
| `API_IDEMPOTENCY_BACKEND` | `memory` | `memory` \| `postgres`. See backend tradeoffs below. |
| `API_IDEMPOTENCY_TTL_SECS` | `86400` | Cached-entry lifetime (24h matches the IETF draft). |
| `API_IDEMPOTENCY_MAX_ENTRIES` | `10000` | Cap for the in-memory backend; PG honours `expires_at` instead. |
| `API_IDEMPOTENCY_MAX_REQUEST_BODY_BYTES` | `1048576` | Requests beyond this skip caching (forwarded as-is). |
| `API_IDEMPOTENCY_MAX_RESPONSE_BODY_BYTES` | `1048576` | Responses beyond this are returned uncached. |
| `API_IDEMPOTENCY_SWEEP_INTERVAL_SECS` | `300` | PG-only: cadence for the `evict_expired` background sweep. `0` disables. `< 60` triggers a startup `WARN`. |

**Store-backend tradeoffs** (see ADR-0082, ADR-0048 consolidated):

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
- Not a general-purpose outbound HTTP client — the only API-owned egress is the
  private, fixed-policy Plane-A OAuth runtime. Webhook *delivery* and integration
  HTTP clients live in action/resource plugins, not here.
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
│   ├── sub.rs          # TlsConfig, CorsConfig, VersioningConfig, PaginationConfig
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
    ├── credential.rs   # Plane-B facade: CRUD, lifecycle, resolve/continue
    ├── oauth/          # Private Plane-A identity OAuth runtime
    │   ├── mod.rs      # Visibility boundary; narrow root composition exports
    │   ├── egress.rs   # Fixed rustls/DNS/admission/body-limit policy
    │   ├── error.rs    # Closed internal failures + secret-free build error
    │   └── runtime.rs  # Opaque config/cache/singleflight/deadline owner
    └── webhook/        # Inbound trigger transport (§11.3 / §13.4)
        ├── mod.rs
        ├── transport.rs  # WebhookTransport — activate/deactivate/router
        ├── bootstrap.rs  # bootstrap_webhook_activations, WebhookSecretResolver
        ├── signing_secret.rs # private registration-time whsec generation
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
| `POST`   | `/api/v1/auth/login`                                                      | Password login: 200 mints a session; MFA-required returns 202 challenge with no session/CSRF |
| `POST`   | `/api/v1/auth/logout`                                                     | Session logout                                                             |
| `POST`   | `/api/v1/auth/forgot-password`                                            | Initiate password reset                                                    |
| `POST`   | `/api/v1/auth/reset-password`                                             | Complete password reset                                                    |
| `POST`   | `/api/v1/auth/verify-email`                                               | Verify email address                                                       |
| `POST`   | `/api/v1/auth/mfa/enroll`                                                 | Enrol a MFA device (session + CSRF)                                        |
| `POST`   | `/api/v1/auth/mfa/verify`                                                 | Confirm MFA enrollment (session + CSRF)                                    |
| `POST`   | `/api/v1/auth/login/mfa`                                                  | Consume an opaque MFA challenge and mint the session (cookie-less, CSRF-exempt) |
| `GET`    | `/api/v1/auth/oauth/{provider}`                                           | Start OAuth2 login flow                                                    |
| `GET`    | `/api/v1/auth/oauth/{provider}/callback`                                  | OAuth2 callback: 200 session, 202 MFA challenge/no session, or 409 explicit linking required |
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
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/resolve`                  | Start credential acquisition (facade; complete / pending / retry)          |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/resolve/continue`         | Continue multi-step credential acquisition (facade)                        |
| `GET`    | `/api/v1/orgs/{org}/workspaces/{ws}/credentials`                          | List credentials (metadata only)                                           |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials`                          | Create credential (write-only secret)                                      |
| `GET`    | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}`                   | Get credential metadata                                                    |
| `PUT`    | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}`                   | Update credential                                                          |
| `DELETE` | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}`                   | Delete credential                                                          |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/test`              | Test credential (capability-gated dispatch)                                |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/refresh`           | Refresh credential token; integration reconnect is a typed 409, not user-auth 401 |
| `POST`   | `/api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}/revoke`            | Revoke credential (capability-gated dispatch)                              |
| `POST`   | `/webhooks/{trigger_uuid}/{nonce}`                                         | Inbound webhook trigger (mounted when `webhook_transport` is set)          |
| `GET`    | `/api/v1/openapi.json`                                                    | OpenAPI 3.1 specification document                                         |
| `GET`    | `/api/v1/docs/`                                                           | Swagger UI (self-hosted)                                                   |
