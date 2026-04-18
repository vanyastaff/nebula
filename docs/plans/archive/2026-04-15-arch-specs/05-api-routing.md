# Spec 05 — API routing and versioning

> **Status:** draft
> **Canon target:** §12.10 (new)
> **Depends on:** 02 (tenancy), 03 (auth), 04 (RBAC), 06 (IDs), 07 (slugs)
> **Depended on by:** 11 (triggers — webhook paths)

## Problem

The API URL structure is the most user-visible choice in the whole product. It affects:
- Can self-host run on `localhost:8080` without DNS magic
- Can deep links work (share URL = opens to right place)
- Can one user have two browser tabs in different orgs
- Does API URL match UI URL (consistent mental model)
- Can `curl` work without magic headers
- Is migration from self-host to cloud painful

Getting this wrong requires breaking URL changes, SEO hit, customer pain. Getting it right means this is the one thing developers don't complain about.

## Decision

**Path-based nested routing with slug OR ULID accepted, one domain, self-host and cloud identical structure.** API version in path (`/api/v1/`). Session cookies on the domain; tenant extracted from path, not from cookie.

## URL structure

### API endpoints

```
POST    /api/v1/orgs/{org}/workspaces/{ws}/workflows
GET     /api/v1/orgs/{org}/workspaces/{ws}/workflows
GET     /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}
PUT     /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}
DELETE  /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}

POST    /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/executions
GET     /api/v1/orgs/{org}/workspaces/{ws}/executions
GET     /api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}
DELETE  /api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}          # cancel
POST    /api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}/terminate
POST    /api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}/restart

GET     /api/v1/orgs/{org}/workspaces/{ws}/credentials
POST    /api/v1/orgs/{org}/workspaces/{ws}/credentials
GET     /api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}
PUT     /api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}
DELETE  /api/v1/orgs/{org}/workspaces/{ws}/credentials/{cred}

GET     /api/v1/orgs/{org}/workspaces/{ws}/resources
GET     /api/v1/orgs/{org}/members
POST    /api/v1/orgs/{org}/members                          # invite
DELETE  /api/v1/orgs/{org}/members/{principal}

GET     /api/v1/orgs/{org}/service-accounts
POST    /api/v1/orgs/{org}/service-accounts
DELETE  /api/v1/orgs/{org}/service-accounts/{sa}

# Org-level
GET     /api/v1/orgs/{org}
PATCH   /api/v1/orgs/{org}
DELETE  /api/v1/orgs/{org}

# User-level (global, no org scope)
GET     /api/v1/me
PATCH   /api/v1/me
GET     /api/v1/me/orgs                                     # list orgs user belongs to
GET     /api/v1/me/tokens                                   # PATs
POST    /api/v1/me/tokens
DELETE  /api/v1/me/tokens/{pat}

# Auth endpoints (no tenant scope)
POST    /api/v1/auth/signup
POST    /api/v1/auth/login
POST    /api/v1/auth/logout
POST    /api/v1/auth/forgot-password
POST    /api/v1/auth/reset-password
POST    /api/v1/auth/verify-email
POST    /api/v1/auth/mfa/enroll
POST    /api/v1/auth/mfa/verify
GET     /api/v1/auth/oauth/{provider}                       # start OAuth flow
GET     /api/v1/auth/oauth/{provider}/callback

# Webhook triggers (special — no auth, separate routing)
POST    /api/v1/hooks/{org}/{ws}/{trigger_slug}
GET     /api/v1/hooks/{org}/{ws}/{trigger_slug}             # some webhooks use GET
```

### UI routes (same domain, parallel structure)

```
/                                                           # landing or dashboard redirect
/login
/signup
/verify-email
/reset-password
/setup                                                      # self-host first-run
/onboarding                                                 # cloud first org

/{org}/{ws}                                                 # workspace dashboard
/{org}/{ws}/workflows
/{org}/{ws}/workflows/{wf}
/{org}/{ws}/workflows/{wf}/edit
/{org}/{ws}/executions
/{org}/{ws}/executions/{exec}
/{org}/{ws}/credentials
/{org}/{ws}/resources
/{org}/settings
/{org}/members
/{org}/billing                                              # cloud only
/me/settings
/me/tokens
```

**Relationship:** UI route `/acme/production/workflows/onboard-user` maps to API `/api/v1/orgs/acme/workspaces/production/workflows/onboard-user`. One mental model.

### Health / internal

```
GET  /health       # liveness — is process alive and storage reachable
GET  /ready        # readiness — is process accepting work (not draining)
GET  /metrics      # Prometheus scrape target (behind auth in cloud, open in self-host)
GET  /version      # version info, also used for update check
```

## Identifier resolution — slug OR ULID

Any path segment that identifies a resource accepts **either** the slug **or** the ULID:

```
/api/v1/orgs/acme/...                           # slug
/api/v1/orgs/org_01J9XYZ.../...                 # ULID

/api/v1/orgs/acme/workspaces/production/...     # both slugs
/api/v1/orgs/acme/workspaces/ws_01J9XYZ.../...  # slug + ULID

/api/v1/orgs/org_01J9.../workspaces/ws_01J.../workflows/wf_01J.../...  # all ULIDs
```

### Resolution middleware

```rust
// Before handler runs
async fn resolve_path_ids(
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let path = req.uri().path();
    
    // Parse path segments
    if let Some(segments) = parse_api_path(path) {
        // Resolve org
        let org_id = match segments.org.starts_with("org_") {
            true => OrgId::from_str(segments.org)?,
            false => org_repo.find_by_slug(segments.org).await?.id,
        };
        
        // Resolve workspace within org
        let ws_id = match segments.workspace.starts_with("ws_") {
            true => WorkspaceId::from_str(segments.workspace)?,
            false => ws_repo.find_by_org_slug(org_id, segments.workspace).await?.id,
        };
        
        // Resolve workflow within workspace (optional — depends on path)
        // ... etc
        
        req.extensions_mut().insert(ResolvedIds { org_id, ws_id, /* ... */ });
    }
    
    next.run(req).await
}
```

### Canonical form in response

API responses always include **both** forms:

```json
{
  "id": "wf_01J9XYZABCDEF0123456789XYZA",
  "slug": "onboard-user",
  "display_name": "Customer Onboarding",
  "workspace": {
    "id": "ws_01J9XYZ...",
    "slug": "production"
  },
  "org": {
    "id": "org_01J9XYZ...",
    "slug": "acme"
  },
  "canonical_url": "/api/v1/orgs/acme/workspaces/production/workflows/onboard-user",
  ...
}
```

Client can store slug form for UI, ULID form for long-term references, switch between them freely. If slug changes later, ULID still works; clients that bookmarked ULID form are insulated from slug rename.

## Session handling

### Cookies

```
Set-Cookie: nebula_session=<session_id>; Domain=nebula.io; Path=/;
            Secure; HttpOnly; SameSite=Lax; Max-Age=604800
```

- **Domain** — single domain, not subdomain-scoped (so one session works across tabs in different orgs)
- **`SameSite=Lax`** — protects against CSRF for state-changing ops, allows normal cross-site GET links
- **`Secure`** — always true in cloud, configurable in self-host (false only for `http://localhost`)
- **`HttpOnly`** — no JS access, prevents XSS token theft
- **`Max-Age=604800`** — 7 days default, configurable

### CSRF protection

Double-submit cookie pattern:

```
1. On login, server sets nebula_csrf cookie with random value (not HttpOnly — client JS needs it)
2. Client JS reads nebula_csrf cookie, sends as X-CSRF-Token header on every state-changing request
3. Server verifies header matches cookie
4. GET requests are CSRF-safe by convention (no side effects)
```

Alternatively, for API-only clients (CLI, CI): PAT authentication skips CSRF since there is no cookie.

### Multi-tab in different orgs

**Key property:** session cookie is NOT tenant-scoped. Tenant comes from URL path. So:

- Tab 1: `nebula.io/acme/production` — session cookie active, tenant = acme/production
- Tab 2: `nebula.io/example/staging` — same session cookie, different tenant
- Both work simultaneously, handler constructs TenantContext from path, not from session

**Switching org** = navigate to different URL, no API call needed beyond normal navigation.

## API versioning

### Strategy: path-based major versions

```
/api/v1/...       # stable v1 API
/api/v2/...       # future, when breaking changes accumulate
```

**Within a major version, semver applies internally:**

- **Patch** (v1.0.0 → v1.0.1) — bug fixes, performance, zero API change
- **Minor** (v1.0.0 → v1.1.0) — new fields, new endpoints, new optional parameters, **no removals or renames**
- **Major** (v1 → v2) — breaking changes, separate path, parallel serving during deprecation period

### Backward compatibility rules for v1

**Allowed within v1 (minor bumps):**
- Adding new fields to response objects (clients ignore unknown)
- Adding new optional query parameters
- Adding new endpoints
- Adding new enum variants (with `#[non_exhaustive]` clients)
- Relaxing validation (previously rejected input now accepted)

**Forbidden within v1:**
- Removing fields from response objects
- Renaming fields
- Changing field types
- Removing endpoints
- Tightening validation (input previously accepted now rejected)
- Changing error codes
- Changing HTTP status codes for existing responses

**Breaking = v2.** If we accumulate enough «should remove X» items, we plan a v2 release with deprecation schedule.

### Deprecation

Fields or endpoints marked deprecated via response headers:

```
Deprecation: true
Sunset: Sat, 31 Dec 2026 23:59:59 GMT
Link: <https://nebula.io/docs/deprecations#field-foo>; rel="deprecation"
```

Clients can monitor these headers to find usages of deprecated features.

## Error responses — RFC 9457 `problem+json`

All error responses follow RFC 9457:

```http
HTTP/1.1 403 Forbidden
Content-Type: application/problem+json

{
  "type": "https://nebula.io/errors/insufficient-role",
  "title": "Insufficient role",
  "status": 403,
  "detail": "WorkspaceEditor required, current role WorkspaceViewer",
  "instance": "/api/v1/orgs/acme/workspaces/production/workflows/wf_01J9.../edit",
  "error_code": "INSUFFICIENT_ROLE",
  "required_role": "WorkspaceEditor",
  "current_role": "WorkspaceViewer"
}
```

**Every error variant** in `ApiError` has:
- HTTP status code
- Machine-readable `error_code` string (stable contract — never renamed)
- Human-readable `title` and `detail`
- Problem `type` URI pointing to docs
- Optional structured fields per variant

Canon §12.4 already requires this; this spec ensures it's applied uniformly.

### Common error codes

| HTTP | error_code | When |
|---|---|---|
| 400 | `INVALID_REQUEST` | Malformed JSON, missing required field |
| 400 | `VALIDATION_FAILED` | Field fails validation rule |
| 401 | `NOT_AUTHENTICATED` | Missing or invalid credentials |
| 401 | `SESSION_EXPIRED` | Session present but expired |
| 401 | `MFA_REQUIRED` | Password accepted, MFA step required |
| 403 | `INSUFFICIENT_ROLE` | Authenticated but permission denied |
| 403 | `QUOTA_EXCEEDED` | Within plan, but hit specific quota |
| 404 | `NOT_FOUND` | Resource doesn't exist, or user lacks access (enumeration prevention) |
| 409 | `CONFLICT` | Resource already exists (e.g., slug taken) |
| 409 | `VERSION_MISMATCH` | Optimistic concurrency (`If-Match` header stale) |
| 410 | `GONE` | Resource was deleted |
| 422 | `UNPROCESSABLE` | Valid format, semantic error (e.g., workflow invalid) |
| 423 | `LOCKED` | Account locked due to failed logins |
| 429 | `RATE_LIMITED` | Too many requests |
| 500 | `INTERNAL_ERROR` | Unexpected server error (no leak of internals) |
| 502 | `UPSTREAM_ERROR` | External dependency failure |
| 503 | `SERVICE_UNAVAILABLE` | Draining, starting, overloaded |
| 507 | `STORAGE_FULL` | Storage quota exceeded |

### 404 vs 403 policy

As noted in spec 04: return **404 when user has no access to tenant**, **403 when user has tenant access but insufficient role**. This is GitHub's convention, prevents enumeration.

## Request / response content type

```
Content-Type: application/json               # for request and response bodies
Content-Type: application/problem+json       # for error bodies only
Accept: application/json                     # client indicates preference
```

JSON is the only body format in v1. Binary payloads (blobs, files) go through `/blobs/` or similar dedicated endpoints, not nested in workflow definitions.

## Rate limiting

Per spec 10, applied as middleware before auth middleware (to prevent login brute force even before we look up credentials):

```
1. Rate limiter keyed on (identity_or_ip, endpoint_class)
2. 429 if exceeded, with Retry-After header
3. Separate buckets for auth endpoints vs general API endpoints
```

See spec 10 for the taxonomy and bucket sizes.

## CORS

Cloud: `Access-Control-Allow-Origin: https://nebula.io` (own domain only).

Self-host: configurable, default permissive for `localhost:*` only.

Webhook endpoints (`/api/v1/hooks/...`): no CORS (not meant for browsers).

```toml
[api.cors]
allowed_origins = ["https://nebula.io"]
allow_credentials = true
max_age = 86400
```

## Pagination

### Cursor-based pagination

All list endpoints use cursor pagination, not offset:

```
GET /api/v1/orgs/{org}/workspaces/{ws}/executions?limit=50&cursor=abc...
```

Response:

```json
{
  "items": [...],
  "next_cursor": "def...",
  "has_more": true
}
```

**Why cursor not offset:**
- Offset is O(n) on every page, breaks at scale
- Offset has consistency issues on insert (same item appears on two pages)
- Cursor is O(1), stable across inserts

**Cursor format:** opaque base64 of `(last_id, last_sort_key)`. Never exposed to user, never parsed by client.

### Limits

```toml
[api.pagination]
default_limit = 50
max_limit = 500        # hard cap to prevent expensive queries
```

## Webhook trigger routing (special case)

Webhook endpoints are different from normal API:

```
POST /api/v1/hooks/{org}/{ws}/{trigger_slug}
```

**Differences:**
- **No authentication middleware** — trigger has its own auth (spec 11), per-trigger
- **No CSRF** — this is incoming from third-party systems, not browser
- **No rate limit per user** — rate limit per trigger, configured on trigger
- **Path parameters still tenant-scoped** — org and ws visible in URL for routing
- **Content type flexible** — depends on source (Stripe sends `application/json`, some legacy APIs send `application/x-www-form-urlencoded`)
- **Response is `202 Accepted` by default** — fire and forget, see spec 11

## OpenAPI specification

API is documented via OpenAPI 3.1 generated from Rust types (using `utoipa` or `aide` crate, author chooses).

```
GET /api/v1/openapi.json       # OpenAPI spec
GET /api/v1/docs               # Swagger UI or similar
```

Generated at compile time from handler annotations — **cannot diverge from actual API**.

## Configuration surface

```toml
[api]
bind = "0.0.0.0:8080"
public_url = "https://nebula.io"     # used for email links, OAuth callbacks
request_timeout = "30s"
max_body_size = "10MB"
request_id_header = "X-Request-ID"   # generate if missing, propagate to logs

[api.tls]
enabled = true                       # default true in cloud, false in self-host
cert_path = "/etc/nebula/cert.pem"
key_path = "/etc/nebula/key.pem"
# ACME auto-provisioning for cloud
acme_enabled = false
acme_directory = "https://acme-v02.api.letsencrypt.org/directory"
acme_email = "ops@nebula.io"

[api.cookies]
domain = ".nebula.io"                # leading dot for cloud, localhost for self-host
secure = true
same_site = "lax"
session_max_age = "7d"

[api.cors]
allowed_origins = ["https://nebula.io"]
allow_credentials = true

[api.versioning]
supported_versions = ["v1"]
deprecated_versions = []             # e.g., ["v1"] when v2 launches
```

## Flows

### Deep link resolution

```
1. User clicks link: https://nebula.io/acme/production/workflows/onboard-user
2. Browser loads UI SPA (which can handle any path)
3. SPA parses path: org=acme, ws=production, wf=onboard-user
4. SPA calls API: GET /api/v1/orgs/acme/workspaces/production/workflows/onboard-user
5. API middleware resolves slugs to IDs, constructs TenantContext
6. RBAC middleware checks permissions
7. Handler returns workflow data (or 404 / 403)
8. SPA renders page
```

### Switching orgs (multi-tab scenario)

```
1. Tab 1 viewing nebula.io/acme/production (logged in, session cookie set)
2. User clicks link to example org workflow
3. Opens in Tab 2: nebula.io/example/staging/workflows/foo
4. Same session cookie sent (SameSite=Lax, same domain)
5. API middleware resolves tenant from path = example/staging
6. RBAC middleware checks user's roles in example/staging (not acme/production)
7. If user is member of example → sees the workflow
8. If user is not member → 404

No «current org» state in session. Tenant is always from URL.
```

### Slug rename effect on API

```
1. Workflow slug was "onboard-user"
2. User renames to "customer-onboarding"
3. Old slug "onboard-user" goes into slug_history with 7-day grace period
4. API calls to GET .../workflows/onboard-user:
   - Middleware resolves slug, finds it in slug_history
   - Responds with 301 Moved Permanently + Location header to new URL
   - Clients using slug automatically follow
5. API calls using ULID (wf_01J9...):
   - Always work, never affected by rename
6. After 7 days, slug_history entry GC'd
7. Old slug now 404 (or available for new workflow)
```

## Testing criteria

**Unit tests:**
- Path parser handles all endpoint patterns
- Slug vs ULID detection by prefix
- Canonical URL generation from IDs

**Integration tests:**
- Every endpoint: happy path, auth required, permission check
- 404 vs 403 consistency (enumeration prevention)
- CSRF rejection on missing token
- CORS preflight
- Slug resolution + 301 redirect on rename
- Cursor pagination stability under insert
- Deep link + authentication flow
- Multi-tab same session different org
- OpenAPI spec validates

**Load tests:**
- 1000 req/sec sustained on list endpoints
- Path parsing overhead < 100 µs
- Slug-to-ID resolution cached, < 1 ms cold / < 100 µs warm

**Security tests:**
- Path traversal attempts (`../`, URL-encoded)
- SQL injection via path params
- Header injection via slug names
- Session fixation prevention
- Cookie flag enforcement
- CORS origin validation

## Performance targets

- Request parsing: **< 100 µs**
- Auth + RBAC middleware: **< 5 ms p99** (DB lookups)
- Slug resolution (with cache): **< 1 ms p99 cold**, **< 100 µs warm**
- Full request overhead (middleware stack): **< 10 ms p99** before handler

## Module boundaries

| Component | Crate |
|---|---|
| `ApiError` enum, RFC 9457 response builder | `nebula-api` |
| Route definitions (axum routers) | `nebula-api` |
| Handlers | `nebula-api` |
| Middleware (auth, tenancy, RBAC, CSRF, rate limit) | `nebula-api` |
| OpenAPI generation | `nebula-api` (derive macros) |
| Webhook trigger routing | `nebula-api` (`webhook` module) |
| Path parser utilities | `nebula-api` |
| HTTP server (axum) | `nebula-api` |

## Migration path

**v1 is greenfield** — no prior API to migrate.

**Future v2 release plan:**
1. Announce v2 with at least 6 months deprecation notice
2. Both v1 and v2 serve in parallel
3. Migration guide per breaking change
4. Client library auto-selects v2 if supported, falls back to v1
5. After 6 months, v1 marked `Deprecation: true` header
6. After 12 months, v1 removed entirely

## Open questions

- **GraphQL endpoint** — alongside REST? Deferred. REST is enough for v1.
- **WebSocket for live updates** — confirmed needed (see spec 18 observability + execution detail views). Endpoint: `ws://.../api/v1/orgs/{org}/workspaces/{ws}/live` with subscription messages. Not fully designed here.
- **gRPC for service-to-service** — not in v1. If cloud needs internal RPC, we add gRPC for that layer, not for public API.
- **API client libraries** — Rust, TypeScript, Python? Generated from OpenAPI? Deferred to v1.1.
- **Subdomain routing for cloud** (`acme.nebula.io/...`) — not in v1, would need wildcard TLS + routing overlay. Consider for cloud branding in v2.
