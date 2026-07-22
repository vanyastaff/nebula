# nebula-api — Agent orientation
> Agent quick-map for `crates/api/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Thin axum HTTP gateway translating REST into typed port-trait calls; all business logic delegates downward, plus inbound webhook + OAuth transports.
**Layer:** API/Public — depends only downward (root AGENTS.md -> Layered Dependency Map).

## Common Tasks

| Task | Steps |
|------|-------|
| Add a new API endpoint | 1. Add handler in `src/domain/<x>/handler.rs` 2. Register it in `src/domain/<x>/routes.rs` and the relevant `src/domain/mod.rs` assembly 3. Run `cargo nextest run -p nebula-api --test openapi_spec` to verify spec sync |
| Add a new middleware | Add to the stack in `src/app.rs` — **order is load-bearing** (auth before csrf). See existing stack. |
| Add a new DTO | Create in `src/domain/<x>/dto.rs`. DTOs MUST NOT embed `nebula-core`/`-storage`/`-engine` types (ADR-0047). Use `serde_json::Value` or wrappers. |
| Add a new error variant | Extend `ApiError` in `src/error/mod.rs` — all errors are RFC 9457 `application/problem+json`. Never a new ad-hoc 500. |
| Test Plane-A OAuth | Run `cargo nextest run -p nebula-api` and `cargo nextest run -p nebula-api --features postgres`; the private egress suite uses a generated TLS CA/server and the production client policy without a release bypass. |
| Check if API compiles | `cargo check -p nebula-api` |

## Commands
- `cargo check -p nebula-api`
- `cargo nextest run -p nebula-api`  ·  doctests: `cargo test -p nebula-api --doc`
- OpenAPI/spec guards: `cargo nextest run -p nebula-api --test openapi_spec` (regenerates spec from the router)
- Feature flags: `postgres` (PG idempotency + `PgAuthBackend`), `test-util` (`ApiConfig::for_test`, bypasses JWT gate — never in prod)

## Key files
- `src/lib.rs` — crate root, public re-exports (`build_app`, `AppState`, `ApiConfig`, `ApiError`)
- `src/app.rs` — `build_app`: OpenApiRouter merge + `split_for_parts` + full middleware stack + `serve()`
- `src/state.rs` — `AppState` builder + API-tier port traits (`OrgResolver`/`WorkspaceResolver`/`MembershipStore`/`SessionStore`/`AuthBackend`)
- `src/error/mod.rs` — `ApiError` (§12.4 RFC 9457 seam, `#[non_exhaustive]`)
- `src/transport/oauth/{egress,error,runtime}.rs` — private Plane-A HTTP policy,
  closed internal failures, and the opaque `OAuthIdentityRuntime`; only the runtime and its
  secret-free build error are re-exported at the crate root for composition.
- `src/middleware/` — auth → tenancy → rbac → csrf → idempotency stack (order is load-bearing: auth before csrf)
- `src/domain/<x>/handler.rs` — §13 knife seams (`create_workflow`, `activate_workflow`, `start_execution`, `cancel_execution`)
- `src/transport/webhook/` — single converged inbound webhook transport (programmatic + slug-routed)

## Conventions & never-do
- Pure library — ships NO binary/composition root; wiring lives in `apps/server` + `examples/examples/api_simple_server.rs`. Do not add a `main`.
- No SQL driver / storage-schema knowledge here — inject spec-16 storage ports via `AppState::new` (`nebula-storage` owns adapters).
- DTOs MUST NOT embed `nebula-core`/`-storage`/`-engine`/`-credential` types (ADR-0047 §3); wrap cross-layer types (`OrgRoleDto`/`WorkspaceRoleDto`). DTOs carry only `serde_json::Value`/wrappers.
- All errors are RFC 9457 `application/problem+json` via a typed `ApiError` variant — never a new ad-hoc 500 for business failures.
- §4.5 operational honesty: an unwired capability returns honest 501/503, never a faked success. Drift between router and OpenAPI spec is a compile error (`OpenApiRouter::routes(routes!(...))`).
- Cancel/terminate signals share the durable `control_queue_repo` outbox (§12.2) — no second in-memory control channel.
- Direct downward domain/port dependencies follow the root layer map; durable cross-crate commands/facts use persisted state or explicit outbox/inbox ports; nebula-eventbus carries only lossy observation and wake hints.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## Invariants

- `transport/oauth` is exclusively Plane-A identity OAuth. Plane-B credential acquisition exposes
  only the universal `resolve` / `resolve/continue` public HTTP contract; raw provider ceremony
  routes, state, and DTOs remain absent.
- Plane-A 1.0 admits only the fixed Google and GitHub.com profiles. Operator configuration is
  credentials-only: `API_AUTH_OAUTH_{GOOGLE,GITHUB}_{CLIENT_ID,CLIENT_SECRET}`. Microsoft,
  generic OIDC, GitHub Enterprise Server, endpoint/scope/auth overrides, and operator JWKS are
  parked and must fail boot rather than becoming environment-selected profiles.
- `ApiConfig` temporarily owns parsed OAuth secrets for validation; `apps/server` moves the map
  into `OAuthIdentityRuntime`. The runtime is then the sole long-lived owner of credentials,
  egress, Google discovery state, concurrency, and deadline policy. The router config retains an
  empty map; backends never receive raw clients, duplicate secrets, or independent config.
- OAuth-state admission is globally capped at 10,000 live entries per Memory process or shared
  PostgreSQL deployment. Capacity check plus insert is atomic/fail-closed; full or contended
  admission returns 429 before issuing state, PKCE material, or a browser cookie.
- OAuth completion has separate boundaries: consume live state atomically; run provider egress
  without database locks under one original callback-network deadline; finalize local state in a
  short Memory critical section or PostgreSQL transaction. Never describe the whole callback as
  one transaction or run provider I/O inside the finalizer.
- Existing `(provider, subject)` links are authoritative. Email collision without such a link is
  `AccountLinkRequired` (409), performs no writes, and never auto-links. For an MFA-enabled linked
  user, the finalizer atomically records an opaque challenge plus MFA-required outcome and creates
  no session/CSRF material; `/auth/login/mfa` is the only completion path.
- MFA enrollment and replacement require a CSRF-protected user session created by primary
  authentication within ten minutes; JWT, PAT, and API-key authority is insufficient. Start
  writes only an expiring candidate and never changes active MFA. Confirm consumes and installs
  the exact live candidate atomically; expiry, replay, replacement, and concurrent losers leave
  the active factor unchanged.
- A valid new provider identity with no policy-acceptable verified email is 403
  `EmailNotVerified` and writes no link/session. Provider transport/non-success or malformed
  identity responses are 502. Do not collapse these semantic and upstream failure lanes.
- Server-fetched OAuth endpoints use rustls HTTPS with redirects, retries, and proxies disabled.
  Literal hosts and every DNS answer must be globally routable before reqwest receives the exact
  validated socket addresses. Provider bodies are bounded and zeroized; bearer tokens stay inside
  a non-cloneable runtime capability.
- Google validates ID-token claims on the direct-TLS token-endpoint path (RS256/header, pinned
  issuer, audience/azp, times, nonce, at_hash, and subject equality). Only local cryptographic
  signature verification against JWKS is deferred; claim and issuer validation are live.
- Request spans record method plus the matched route template (or fixed `<unmatched>`), never a raw
  URI/query. OAuth failures cross the public boundary only through fixed RFC 9457 messages.
- `OAuthProvider` and `AuthError` are `#[non_exhaustive]`; downstream technical composition code
  must use wildcard matches. Keep provider/failure additions semver-additive.
- `nebula-api` is a technical HTTP/composition boundary. `nebula-sdk` remains the sole supported,
  branded Rust surface; do not promote private OAuth machinery into a second integration API.
- Credential handlers submit only middleware-created `AuthenticatedPrincipal`, resolved scope,
  and API-owned intent through `CredentialCommandGateway`; they never receive a credential
  service, owner key, selector, authority proof, or raw persistence handle.
- Credential mutations do not prevalidate through `CredentialSchemaPort`. That port is a
  catalog/form-schema read model only; after authority, the credential service is the single
  validate→resolve authority. An unwired schema port must not turn create/update/resolve into 503.
- `MembershipStore::get_tenant_membership` returns org/workspace roles from one logical snapshot
  (one lock guard or database read snapshot). RBAC and bounded-context authorities must use it;
  do not reconstruct an authorization decision from two independent role reads. An unwired or
  failed workspace resolver or membership source disables tenant routes with 503; a valid snapshot
  without organization membership denies access (enumeration-safe 404 where the route contract
  requires it). Neither case implies administrator access.

## See also
- `README.md` — full design (endpoint table, CSRF route table, OAuth/idempotency env vars, durability caveats)
- ADRs: 0047 (OpenAPI), 0048/0082 (idempotency), 0049 (webhook), 0050 (W3C trace), 0072 (storage port), 0085 (OAuth IdP)
