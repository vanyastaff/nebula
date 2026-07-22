# nebula-api — current design

| Field | Value |
|---|---|
| Status | Pre-1.0 technical HTTP boundary |
| Reviewed | 2026-07-22 |
| Layer | API / transport |

## Responsibility

`nebula-api` is a pure axum library. It owns routing, OpenAPI, middleware, HTTP DTOs, RFC 9457
errors, and API-facing object-safe ports. It does not own a binary, deployment lifecycle, SQL
schema, workflow engine, credential authority policy, or the branded Rust integration API.
First-party construction and process policy live in `apps/server`; `nebula-sdk` is the sole
supported Rust product surface.

## Request flow

```text
request
  -> request id / trace
  -> authentication
  -> tenant resolution
  -> RBAC
  -> CSRF (session-cookie authority only)
  -> idempotency / rate limits
  -> domain handler
  -> API-owned port
  -> RFC 9457 response
```

Middleware order is load-bearing. OpenAPI routes are mounted through `OpenApiRouter`; route/spec
drift and stub honesty are executable contracts.

## Credential command boundary

Authentication middleware inserts `AuthenticatedPrincipal`, whose fields and constructor are not
public and which has no serde contract. Credential handlers pass a reference to that principal,
the already resolved `Scope`, and a `CredentialGatewayCommand` to the object-safe
`CredentialCommandGateway`.

The API port exposes no owner key, selector, repository, raw writer, service, or authority proof.
The production implementation is an apps-owned trust bridge that maps authenticated claims to the
credential-owned authority/controller. The authority obtains org/workspace roles through one
consistent `MembershipStore` snapshot, applies the operation's credential permission, and asks the
tenancy resolver to reproduce the scope. An unwired or failed membership source returns
unavailable; a valid snapshot without organization membership denies. A missing gateway returns an
honest 503; there is no raw store or direct-service fallback.

Credential validation failures cross the port as structural path/code issues only. The API owns
static, value-free user-facing messages and renders canonical RFC 6901 pointers. Provider text,
validator messages/params, and submitted credential values never reach `ProblemDetails`.

`CredentialSchemaPort` is a catalog/form-schema read model. The credential service remains the
single mutation validation authority after the credential-authority decision and before
persistence. Mutation handlers never precheck through the schema port, and an unwired schema port
does not turn create/update/resolve into 503. Production construction and concrete adapters live
in `apps/server`; the API-side factory/builder is an unsupported `test-util` fixture.

## Plane-A identity boundary

Plane-A user sign-in is separate from Plane-B integration credentials. The identity OAuth runtime
admits only fixed Google and GitHub.com profiles, owns long-lived operator credentials and network
policy, uses rustls HTTPS with redirects/retries/proxies disabled, validates the actual connect
addresses, bounds and zeroizes response bodies, and exposes closed errors.

OAuth state admission/consume/finalization, browser transaction binding, MFA authority, account
link policy, no-store authority responses, and trace redaction are pinned by the invariants in
`crates/api/AGENTS.md` and integration tests. Provider-specific Plane-B credential OAuth routes are
absent; only universal `resolve` / `resolve/continue` remains.

## AppState and ports

`AppState` holds object-safe capabilities. Optional capabilities fail honestly when unwired.
Storage adapters are injected through ports or curated technical seams; handlers do not know SQL
schemas. Durable control commands use the control-queue/outbox path rather than an in-memory side
channel.

The HTTP DTO layer does not expose lower-layer domain/storage objects. Conversion happens at the
handler/port adapter boundary, and public errors are always `application/problem+json`.

## Invariants

- Authentication facts are middleware-produced capabilities, not request-deserializable DTOs.
- Supported authenticated HTTP credential management reaches persistence only through one gateway
  and one credential-authority decision. K3 still owns sole-semantic-writer/operation-ledger
  closure for technical service paths.
- Cross-tenant absence and wrong-tenant access are indistinguishable.
- Credential validation, persistence diagnostics, and public HTTP errors cross
  their boundaries only as value-free platform-owned codes/messages. Internal
  technical diagnostics are outside that guarantee and must be sanitized before
  tracing or response rendering.
- Missing capabilities return 501/503, never fabricated success.
- Cancel/terminate and other durable commands use persisted seams.
- `nebula-api` remains a technical boundary; do not turn it into a parallel SDK.

## Known gaps

- Production credential composition and concrete adapters live in `apps/server`; the API-side
  factory/builder exists only behind unsupported `test-util` for hermetic integration tests.
- Google ID-token claim checks are live, but local rotating-JWKS signature verification remains an
  explicit security follow-up.
- Some legacy resource repository/error mapping remains to be moved onto the canonical port model.
- Production workspace-directory and membership wiring remains incomplete: lower-level storage is
  not a substitute for the API policy port's one-snapshot authorization and atomic lockout guards;
  an apps-owned durable directory/policy bridge and supported operator configuration are K4 work.
  The default server therefore returns 503 for tenant routes. Several deliberately advertised 501
  surfaces also remain.
- Live PostgreSQL suites are release evidence; skip-clean local tests do not constitute that proof.
