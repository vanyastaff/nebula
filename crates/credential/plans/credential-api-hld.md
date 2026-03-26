# nebula-credential HTTP API — High-Level Design

> **Companion to:** credential-hld-v6-final.md (core types), credential-storage-hld.md (storage)
> **Scope:** REST endpoints for credential CRUD, OAuth2/SAML/device-code callback handling,
> error responses, authentication, request/response schemas.

---

## Overview

The HTTP API is the boundary between **untrusted callers** (UI, external tools)
and the **framework zone** (CredentialResolver, storage, refresh). This is
where error normalization, authentication, and callback security are enforced.

```
┌─ UI / CLI / External ──────────────────────┐
│  REST API calls + OAuth2/SAML callbacks     │
└──────────────────┬─────────────────────────┘
                   │ HTTP
                   ▼
┌─ API Layer ────────────────────────────────┐
│  Authentication (session / API key)         │
│  Request validation                         │
│  Error normalization (ResolutionError →     │
│    generic public error)                    │
│  Callback routing (PendingToken ↔ session)  │
└──────────────────┬─────────────────────────┘
                   │ Rust API
                   ▼
┌─ Framework Zone ───────────────────────────┐
│  CredentialResolver                         │
│  PendingStateStore                          │
│  CredentialStore (layered)                  │
│  RefreshCoordinator                         │
└────────────────────────────────────────────┘
```

---

## Authentication

All API endpoints require authentication. The API layer extracts `owner_id`
and `session_id` from the authenticated session — these are passed to
CredentialContext and PendingStateStore.

| Method | Use case |
|--------|----------|
| Session cookie (HttpOnly, SameSite=Lax) | Web UI |
| API key (header `X-Api-Key`) | CLI tools, automation |
| Bearer token (header `Authorization: Bearer ...`) | Service-to-service |

The API layer resolves the identity → `(owner_id, session_id, scope)`.
These are injected into framework calls. Credential operations are
always scoped to the authenticated user.

---

## REST Endpoints

### Base path: `/api/v1/credentials`

### List credentials

```
GET /api/v1/credentials
Query: ?scheme_kind=bearer&lifecycle=Active
```

**Response 200:**
```json
{
  "credentials": [
    {
      "id": "cred-abc-123",
      "name": "GitHub PAT",
      "credential_type": "github-pat",
      "scheme_kind": "bearer",
      "lifecycle": "Active",
      "created_at": "2026-01-15T10:00:00Z",
      "updated_at": "2026-03-25T14:30:00Z"
    }
  ]
}
```

**Note:** Response contains metadata only. No secret data. No `data` field.
Filtered by ScopeLayer — user sees only their own credentials.

### Get credential metadata

```
GET /api/v1/credentials/{id}
```

**Response 200:**
```json
{
  "id": "cred-abc-123",
  "name": "GitHub PAT",
  "credential_type": "github-pat",
  "scheme_kind": "bearer",
  "lifecycle": "Active",
  "created_at": "2026-01-15T10:00:00Z",
  "updated_at": "2026-03-25T14:30:00Z",
  "capabilities": {
    "interactive": false,
    "refreshable": false,
    "revocable": false,
    "testable": true
  }
}
```

**No secret data returned.** Ever. Frontend uses this for display + buttons
(show "Test" button if `testable: true`, show "Refresh" if `refreshable: true`).

### Get credential setup form schema

```
GET /api/v1/credential-types/{type_key}/parameters
```

**Response 200:**
```json
{
  "credential_type": "google-sheets-oauth2",
  "name": "Google Sheets (OAuth2)",
  "icon": "google-sheets",
  "parameters": [
    {
      "key": "client_id",
      "type": "string",
      "label": "Client ID",
      "required": true
    },
    {
      "key": "client_secret",
      "type": "string",
      "label": "Client Secret",
      "secret": true,
      "required": true
    },
    {
      "key": "scopes",
      "type": "select",
      "label": "Scopes",
      "multiple": true,
      "options": [
        {"value": "spreadsheets", "label": "Read/Write Spreadsheets"},
        {"value": "spreadsheets.readonly", "label": "Read Only"}
      ],
      "default": ["spreadsheets"]
    }
  ],
  "capabilities": {
    "interactive": true,
    "refreshable": true,
    "revocable": true,
    "testable": false
  }
}
```

This is the JSON serialization of `ParameterCollection`. Frontend renders
it as a form.

### Create credential (begin resolve)

```
POST /api/v1/credentials
Content-Type: application/json

{
  "type": "google-sheets-oauth2",
  "name": "My Google Sheets",
  "values": {
    "client_id": "123456.apps.googleusercontent.com",
    "client_secret": "GOCSPX-...",
    "scopes": ["spreadsheets"]
  }
}
```

**Framework flow:**
1. Authenticate request → extract `owner_id`, `session_id`
2. Look up credential type → `GoogleSheetsOAuth2`
3. Build `CredentialContext` (read-only, with callback_url)
4. Call `execute_resolve::<GoogleSheetsOAuth2>(values, ctx, pending_store)` with 30s timeout
5. Handle result:

**Response: Static credential (Complete)**
```
201 Created

{
  "id": "cred-new-456",
  "status": "active",
  "lifecycle": "Active"
}
```

**Response: Interactive credential (Pending)**
```
202 Accepted

{
  "id": "cred-new-456",
  "status": "pending_interaction",
  "interaction": {
    "type": "redirect",
    "url": "https://accounts.google.com/o/oauth2/v2/auth?client_id=...&state=..."
  }
}
```

The `PendingToken` is NOT returned to the client. It is stored in a
server-side HttpOnly SameSite=Lax cookie:

```
Set-Cookie: nebula_pending=<pending_token>; HttpOnly; SameSite=Lax; Secure; Path=/api/v1/credentials/callback; Max-Age=600
```

Frontend redirects user to `interaction.url`.

**Response: Polling (device code)**
```
202 Accepted

{
  "id": "cred-new-456",
  "status": "pending_poll",
  "interaction": {
    "type": "display_info",
    "title": "Enter Code",
    "message": "Go to https://device.login.com and enter code: ABCD-1234",
    "data": {
      "user_code": "ABCD-1234",
      "verification_uri": "https://device.login.com"
    },
    "expires_in": 300
  },
  "poll_url": "/api/v1/credentials/cred-new-456/poll",
  "poll_interval": 5
}
```

### Update credential values (re-resolve)

```
PUT /api/v1/credentials/{id}
Content-Type: application/json

{
  "values": {
    "client_id": "new-client-id",
    "client_secret": "new-secret",
    "scopes": ["spreadsheets", "spreadsheets.readonly"]
  }
}
```

Same flow as create. May return 200 (Complete) or 202 (Pending interaction).

### Delete credential

```
DELETE /api/v1/credentials/{id}
```

**Framework flow:**
1. Check lifecycle — if Active + revocable, call `revoke()` first
2. Delete from CredentialStore
3. Emit `CredentialRotatedEvent` (for resource re-auth / cleanup)

**Response 204:** No content.

### Test credential

```
POST /api/v1/credentials/{id}/test
```

**Framework flow:**
1. Load credential from store → decrypt → project to Scheme
2. Call `test(scheme, ctx)` with 30s timeout
3. Return result

**Response 200:**
```json
{
  "status": "success"
}
```

```json
{
  "status": "failed",
  "reason": "HTTP 401 Unauthorized"
}
```

```json
{
  "status": "untestable"
}
```

**Rate limit:** 1 test per credential per minute.

### Refresh credential (manual trigger)

```
POST /api/v1/credentials/{id}/refresh
```

**Framework flow:**
1. Load credential → check REFRESHABLE capability
2. Call `refresh(state, ctx)` with 30s timeout
3. CAS-write updated state
4. Return result

**Response 200:**
```json
{
  "status": "refreshed",
  "expires_at": "2026-03-25T15:30:00Z"
}
```

**Response 409:** Conflict (another refresh in progress).
**Response 422:** Credential not refreshable.

---

## OAuth2 Callback Handling

### Flow

```
1. User creates credential → POST /api/v1/credentials
2. Server returns 202 + redirect URL + sets pending cookie
3. User's browser redirects to Google/GitHub/etc.
4. User authorizes
5. Provider redirects to: GET /api/v1/credentials/callback?code=...&state=...
6. Server reads pending cookie → PendingToken
7. Server calls execute_continue<C>(token, UserInput::Callback{params}, ctx, pending_store)
8. PendingStateStore validates 4 dimensions (credential_kind, owner_id, session_id, token_id)
9. PendingState consumed (deleted)
10. continue_resolve() exchanges code for tokens
11. Credential state stored in CredentialStore
12. Server redirects user to success page
```

### Callback endpoint

```
GET /api/v1/credentials/callback?code=AUTH_CODE&state=CSRF_STATE
Cookie: nebula_pending=<pending_token>
```

**Security:**
- PendingToken comes from **HttpOnly cookie**, NOT from URL
- `state` parameter = csrf_state from OAuth2Pending — validated against stored value
- 4-dimensional validation on consume (credential_kind, owner_id, session_id, token_id)
- PendingToken NOT in URL — no proxy log / browser history / Referer leakage

**CSRF protection:**
1. `state` parameter in URL matches `csrf_state` in OAuth2Pending
2. SameSite=Lax cookie → browser only sends cookie on same-site navigations
3. session_id in PendingToken binding → session fixation prevented

**Error handling:**
```
GET /api/v1/credentials/callback?error=access_denied&error_description=User+denied+access
```
→ Delete pending state, redirect to error page with generic message.

**Success response:**
```
302 Found
Location: /credentials/cred-new-456?status=connected
Set-Cookie: nebula_pending=; Max-Age=0  (clear cookie)
```

### OAuth2 callback — provider error handling

| Provider response | Framework action |
|-------------------|-----------------|
| `?code=...&state=...` | Exchange code → store credential |
| `?error=access_denied` | Clean up pending state, show "access denied" |
| `?error=server_error` | Clean up pending state, show "provider error" |
| Missing `state` parameter | Reject — possible CSRF |
| Invalid `state` (doesn't match stored csrf_state) | Reject — CSRF attempt |
| Missing/invalid pending cookie | Reject — session mismatch |

---

## SAML Callback Handling

### Flow (POST binding)

```
1. User creates SAML credential → POST /api/v1/credentials
2. Server returns 202 + FormPost interaction
3. Frontend auto-submits POST form to IdP SSO URL
4. User authenticates at IdP
5. IdP POSTs to: POST /api/v1/credentials/callback/saml
   Body: SAMLResponse=BASE64...&RelayState=SESSION_TOKEN
6. Server reads pending cookie → PendingToken
7. Server calls execute_continue<SamlCredential>(token, UserInput::FormData{params}, ...)
8. PendingState consumed, SAML assertion validated
9. Credential state stored
10. Server redirects user to success page
```

### SAML callback endpoint

```
POST /api/v1/credentials/callback/saml
Content-Type: application/x-www-form-urlencoded

SAMLResponse=PHNhbWw...&RelayState=session_abc
Cookie: nebula_pending=<pending_token>
```

**Security:**
- Same HttpOnly cookie approach as OAuth2
- `RelayState` = session correlation (matches session_id in PendingToken binding)
- SAML Response signature validated inside `continue_resolve()`
- Assertion audience validated against SP entity ID

---

## Device Code Polling

### Flow

```
1. User creates device code credential → POST /api/v1/credentials
2. Server returns 202 + display_info (user code + verification URI)
3. User enters code on another device
4. Frontend polls: POST /api/v1/credentials/{id}/poll
5. Framework calls continue_resolve with UserInput::Poll
6. If not yet authorized → 202 + Retry
7. If authorized → credential stored → 200
```

### Poll endpoint

```
POST /api/v1/credentials/{id}/poll
Cookie: nebula_pending=<pending_token>
```

**Response: Not yet authorized**
```
202 Accepted
Retry-After: 5

{
  "status": "pending",
  "retry_after": 5
}
```

**Response: Authorized**
```
200 OK

{
  "id": "cred-new-456",
  "status": "active"
}
```

**Response: Expired**
```
410 Gone

{
  "status": "expired",
  "message": "Device code expired. Please restart."
}
```

**Note:** For device code, `get()` (not `consume()`) is used on PendingStateStore
during polling — state is not consumed until authorization succeeds.
On success, `consume()` is called to delete the pending state.

---

## Error Responses

### Two-layer error model

Errors follow the split model from credential-hld-v6:

- **CredentialError** — author-facing, no credential_id. Returned by credential
  implementations (resolve, refresh, revoke, test). Variants: InvalidInput,
  ValidationFailed, RefreshFailed, RevokeFailed, NotInteractive,
  CompositionNotAvailable, CompositionFailed, Provider.

- **ResolutionError** — framework-facing, with credential_id + ResolutionStage.
  Wraps CredentialError with context. Constructed by framework executor, never
  by credential authors.

The API layer receives `ResolutionError` and normalizes it for external callers.

### Normalized public errors

API layer transforms `ResolutionError` → generic public error.
Internal details (stage, scheme_kind, credential_id) go to operator logs only.

```json
{
  "error": {
    "code": "credential_unavailable",
    "message": "Credential is not available."
  }
}
```

### Error code mapping

| Internal error | HTTP status | Public code | Public message |
|---------------|-------------|-------------|----------------|
| ResolutionError(NotFound) | 404 | `not_found` | "Credential not found." |
| ResolutionError(ScopeViolation) | 403 | `access_denied` | "Access denied." |
| ResolutionError(SchemeMismatch) | 422 | `credential_unavailable` | "Credential is not available." |
| ResolutionError(Credential(InvalidInput)) | 400 | `invalid_request` | "Invalid request: {sanitized message}" |
| ResolutionError(Credential(RefreshFailed)) | 502 | `credential_unavailable` | "Credential is not available." |
| ResolutionError(Credential(NotInteractive)) | 400 | `invalid_request` | "This credential type does not support this operation." |
| ResolutionError(Credential(CompositionFailed)) | 502 | `credential_unavailable` | "Credential dependency unavailable." |
| ResolutionError(CompositionDepthExceeded) | 422 | `credential_unavailable` | "Credential configuration error." |
| ResolutionError(Store(Conflict)) | 409 | `conflict` | "Concurrent modification. Please retry." |
| StoreError(Backend) | 500 | `internal_error` | "Internal server error." |

**Error sanitization rules:**
- User-facing messages NEVER contain credential IDs, scheme kinds, stage info,
  or provider error details
- Provider errors (e.g., `"Client secret 'ABC' is invalid"`) are stripped before
  returning to user — they may contain secrets
- Operator logs contain everything: full ResolutionError with credential_id,
  stage, source chain. Sent to audit sink only.

**Constructing errors in credential code:**
```rust
// Credential authors use CredentialError (no credential_id):
Err(CredentialError::InvalidInput("missing code".into()))
Err(CredentialError::refresh(RefreshErrorKind::TokenExpired, RetryAdvice::Never, "token expired"))
Err(CredentialError::Provider(Box::new(e)))

// Framework wraps automatically:
// ResolutionError { credential_id: "cred-123", stage: Resolve, source: CredentialError::... }
```

---

## Request Validation

### Parameter sanitization

All credential `values` from POST/PUT are validated against `ParameterCollection`
before passing to `resolve()`:

1. Required fields present
2. Types match (string, integer, boolean, select)
3. Rules applied (regex, min/max, options)
4. Secret fields → `SecretString` (never logged, never echoed back)

If validation fails → 400 with `invalid_request` and field-level errors:

```json
{
  "error": {
    "code": "invalid_request",
    "message": "Validation failed.",
    "details": {
      "client_id": "required",
      "scopes": "at least one scope required"
    }
  }
}
```

### Credential type validation

POST body `type` field must match a registered credential type. Unknown
type → 400 `invalid_request`.

---

## Credential Picker API

For credential composition (AWS Assume Role needs base AWS credential),
the UI needs a filtered list of compatible credentials.

```
GET /api/v1/credentials?scheme_kind=aws&lifecycle=Active
```

Returns credentials filtered by:
- `scheme_kind` matching the required AuthScheme
- `lifecycle = Active` (not terminal or re-auth required)
- ScopeLayer (user's own credentials only)

Frontend renders this as a dropdown in the credential setup form.

---

## Concurrency & Idempotency

### Create credential

`POST /api/v1/credentials` is NOT idempotent. Repeated calls create
multiple credentials. Client should check for existing credential before
creating.

### Update / Refresh

Use CAS via `If-Match` header:

```
PUT /api/v1/credentials/{id}
If-Match: "7"    ← current version
```

If version mismatch → 409 Conflict. Client re-reads and retries.

### Delete

Idempotent. Deleting non-existent credential → 204 (not 404).

### Callback

OAuth2/SAML callbacks are single-use (PendingStateStore consume semantics).
Replaying a callback → 400 (pending state already consumed).

---

## Rate Limits

| Endpoint | Limit | Window |
|----------|-------|--------|
| `POST /credentials` | 10 | per minute per user |
| `POST /credentials/{id}/test` | 1 | per minute per credential |
| `POST /credentials/{id}/refresh` | 5 | per minute per credential |
| `GET /credentials/callback` | 20 | per minute per user |
| `POST /credentials/{id}/poll` | 12 | per minute per credential |

Rate limit exceeded → 429 Too Many Requests with `Retry-After` header.
