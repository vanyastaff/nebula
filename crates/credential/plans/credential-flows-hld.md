# nebula-credential Integration Flows — High-Level Design

> **Companion to:** credential-hld-v6-final.md (core types), credential-api-hld.md (HTTP endpoints),
> credential-storage-hld.md (backends).
> **Scope:** End-to-end flow for every interactive credential pattern.
> How the framework orchestrates resolve → redirect → callback → store for real integrations.

---

## Overview

This document traces the complete journey from "user clicks Connect" to
"credential stored and usable" for each authentication pattern. It shows
how framework components (API layer, PendingStateStore, CredentialStore,
resolve executor) coordinate across HTTP boundaries.

### Components in play

```
┌─ Browser ──────────────────────────────┐
│  Credential setup form                  │
│  OAuth2 consent screen (Google/GitHub)  │
│  SAML IdP login page                    │
│  Device code entry page                 │
└────────────────┬───────────────────────┘
                 │ HTTP
                 ▼
┌─ Nebula API Layer ─────────────────────┐
│  POST /api/v1/credentials              │
│  GET  /api/v1/credentials/callback     │
│  POST /api/v1/credentials/callback/saml│
│  POST /api/v1/credentials/{id}/poll    │
└────────────────┬───────────────────────┘
                 │
                 ▼
┌─ Framework Executor ───────────────────┐
│  execute_resolve<C>()                  │
│  execute_continue<C>()                 │
│  30s timeout on all credential methods │
└───┬──────────────┬─────────────────────┘
    │              │
    ▼              ▼
┌─────────┐  ┌──────────────┐
│ Pending │  │  Credential  │
│ State   │  │  Store       │
│ Store   │  │  (layered)   │
└─────────┘  └──────────────┘
```

---

## Flow 1: OAuth2 Authorization Code + PKCE

**Used by:** GitHub, Google (Sheets, Drive, Calendar), Microsoft (OneDrive, Teams),
Slack, Spotify, Dropbox, Salesforce, HubSpot, Notion, Linear, Figma, etc.

### Sequence

```
User                    Nebula API              Framework              Provider (Google)
 │                         │                       │                       │
 │ 1. Click "Connect       │                       │                       │
 │    Google Sheets"       │                       │                       │
 │ ────────────────────►   │                       │                       │
 │    POST /credentials    │                       │                       │
 │    { type: "google-     │                       │                       │
 │      sheets-oauth2",   │                       │                       │
 │      values: {          │                       │                       │
 │        client_id,       │                       │                       │
 │        client_secret,   │                       │                       │
 │        scopes           │                       │                       │
 │      }}                 │                       │                       │
 │                         │                       │                       │
 │                         │ 2. authenticate       │                       │
 │                         │    request             │                       │
 │                         │ 3. build ctx           │                       │
 │                         │ ──────────────────►    │                       │
 │                         │   execute_resolve()    │                       │
 │                         │                        │                       │
 │                         │                        │ 4. C::resolve()      │
 │                         │                        │    generates PKCE     │
 │                         │                        │    builds auth URL    │
 │                         │                        │    returns Pending {  │
 │                         │                        │      state: OAuth2    │
 │                         │                        │        Pending,      │
 │                         │                        │      interaction:    │
 │                         │                        │        Redirect{url} │
 │                         │                        │    }                  │
 │                         │                        │                       │
 │                         │                        │ 5. Framework stores   │
 │                         │                        │    OAuth2Pending in   │
 │                         │                        │    PendingStateStore  │
 │                         │                        │    key=(credential_   │
 │                         │                        │     kind, owner_id,  │
 │                         │                        │     session_id,      │
 │                         │                        │     token_id)        │
 │                         │                        │                       │
 │                         │ ◄──────────────────    │                       │
 │                         │   PendingToken +       │                       │
 │                         │   redirect URL         │                       │
 │                         │                        │                       │
 │  6. 202 Accepted        │                       │                       │
 │  Set-Cookie:            │                       │                       │
 │   nebula_pending=       │                       │                       │
 │   <token>; HttpOnly;    │                       │                       │
 │   SameSite=Lax          │                       │                       │
 │  { interaction: {       │                       │                       │
 │      type: "redirect",  │                       │                       │
 │      url: "https://     │                       │                       │
 │       accounts.google   │                       │                       │
 │       .com/o/oauth2/    │                       │                       │
 │       v2/auth?..."      │                       │                       │
 │  }}                     │                       │                       │
 │ ◄───────────────────    │                       │                       │
 │                         │                       │                       │
 │ 7. Browser redirects ──────────────────────────────────────────────►    │
 │    to Google consent    │                       │                       │
 │    screen               │                       │                       │
 │                         │                       │                       │
 │ 8. User authorizes ◄───────────────────────────────────────────────    │
 │    Google redirects     │                       │                       │
 │    back to Nebula       │                       │                       │
 │                         │                       │                       │
 │ 9. GET /callback?       │                       │                       │
 │    code=AUTH_CODE&       │                       │                       │
 │    state=CSRF_STATE      │                       │                       │
 │    Cookie: nebula_       │                       │                       │
 │    pending=<token>       │                       │                       │
 │ ────────────────────►   │                       │                       │
 │                         │                       │                       │
 │                         │ 10. Read pending       │                       │
 │                         │     cookie             │                       │
 │                         │ 11. Validate CSRF      │                       │
 │                         │     state param        │                       │
 │                         │ ──────────────────►    │                       │
 │                         │   execute_continue()   │                       │
 │                         │                        │                       │
 │                         │                        │ 12. PendingStateStore │
 │                         │                        │     .consume() with   │
 │                         │                        │     4D validation     │
 │                         │                        │                       │
 │                         │                        │ 13. C::continue_      │
 │                         │                        │     resolve(          │
 │                         │                        │       &pending,       │
 │                         │                        │       &UserInput::    │
 │                         │                        │        Callback{code},│
 │                         │                        │       &ctx            │
 │                         │                        │     )                 │
 │                         │                        │                       │
 │                         │                        │ 14. Exchange code ────────────────►
 │                         │                        │     for tokens        │ POST /token
 │                         │                        │     (using PKCE       │ {code, verifier,
 │                         │                        │      verifier from    │  client_secret}
 │                         │                        │      pending state)   │
 │                         │                        │                       │
 │                         │                        │ 15. ◄─────────────────────────────
 │                         │                        │     { access_token,   │
 │                         │                        │       refresh_token,  │
 │                         │                        │       expires_in }    │
 │                         │                        │                       │
 │                         │                        │ 16. Return            │
 │                         │                        │     Complete(         │
 │                         │                        │       OAuth2State)    │
 │                         │                        │                       │
 │                         │                        │ 17. Framework encrypts│
 │                         │                        │     + stores in       │
 │                         │                        │     CredentialStore   │
 │                         │                        │     (PutMode::Create) │
 │                         │                        │                       │
 │                         │ ◄──────────────────    │                       │
 │                         │   success              │                       │
 │                         │                        │                       │
 │ 18. 302 Redirect        │                       │                       │
 │     to /credentials/    │                       │                       │
 │     cred-123?status=    │                       │                       │
 │     connected            │                       │                       │
 │ ◄───────────────────    │                       │                       │
```

### What's stored where

| Data | Where | When deleted |
|------|-------|-------------|
| OAuth2Pending (PKCE verifier, client_secret, csrf_state) | PendingStateStore | Step 12: consumed on callback |
| OAuth2State (access_token, refresh_token, client_id, client_secret) | CredentialStore (encrypted) | When user deletes credential |
| PendingToken | HttpOnly cookie | Step 18: Max-Age=0 (cleared) |
| Authorization code | Google → Nebula URL | One-time use by Google |
| CSRF state | OAuth2Pending + URL state param | Validated step 11, deleted step 12 |

### Security checkpoints

| Step | Check | Prevents |
|------|-------|----------|
| 6 | PendingToken in HttpOnly cookie, not URL | Token leakage in logs/Referer/history |
| 9 | Cookie SameSite=Lax | CSRF on callback endpoint |
| 11 | state param == stored csrf_state | Login CSRF (attacker initiates, victim completes) |
| 12 | 4D validation: credential_kind + owner + session + token | Type confusion, cross-user, session fixation |
| 14 | PKCE verifier from pending state (not URL) | Authorization code interception |
| 14 | 30s framework timeout | Hung provider / DoS |

### Provider-specific quirks

| Provider | Quirk | Handling |
|----------|-------|---------|
| GitHub | auth_style = PostBody (not BasicAuth for token exchange) | OAuth2Flow.auth_style(AuthStyle::PostBody) |
| Google | Offline access requires `access_type=offline` prompt=consent | Extra parameter in auth URL |
| Microsoft | Tenant-specific endpoints (`/v2.0/authorize`) | tenant_id in config |
| Slack | Scopes are comma-separated (not space-separated) | Custom scope formatter |
| Salesforce | Custom domains (`login.salesforce.com` vs `test.salesforce.com`) | Configurable auth/token URLs |

---

## Flow 2: OAuth2 Client Credentials (server-to-server, no redirect)

**Used by:** Service accounts without user context. Machine-to-machine auth.
Twilio, SendGrid, some internal APIs.

### Sequence

```
User                    Nebula API              Framework              Provider
 │                         │                       │                       │
 │ POST /credentials       │                       │                       │
 │ { type: "twilio",      │                       │                       │
 │   values: {             │                       │                       │
 │     account_sid,        │                       │                       │
 │     auth_token          │                       │                       │
 │   }}                    │                       │                       │
 │ ────────────────────►   │                       │                       │
 │                         │ ──────────────────►    │                       │
 │                         │   execute_resolve()    │                       │
 │                         │                        │                       │
 │                         │                        │ C::resolve()          │
 │                         │                        │ POST /oauth/token ────────────►
 │                         │                        │ {grant_type:          │
 │                         │                        │  client_credentials}  │
 │                         │                        │                       │
 │                         │                        │ ◄──────────────────────────────
 │                         │                        │ { access_token,       │
 │                         │                        │   expires_in }        │
 │                         │                        │                       │
 │                         │                        │ Complete(State)       │
 │                         │                        │ Store encrypted       │
 │                         │                        │                       │
 │ 201 Created             │                       │                       │
 │ { id: "cred-456" }     │                       │                       │
 │ ◄───────────────────    │                       │                       │
```

**Key difference from OAuth2 Authorization Code:**
- No redirect, no PendingState, no callback
- resolve() makes network call directly (POST to token endpoint)
- Returns `ResolveResult::Complete` immediately
- `type Pending = NoPendingState`
- But resolve() is async — framework timeout still applies

---

## Flow 3: Google Service Account (JWT grant, no redirect)

**Used by:** Google Cloud service accounts, Firebase, GCP APIs.
User uploads JSON key file → framework mints JWT → exchanges for access token.

### Sequence

```
User                    Nebula API              Framework              Google
 │                         │                       │                       │
 │ POST /credentials       │                       │                       │
 │ { type: "google-       │                       │                       │
 │   service-account",    │                       │                       │
 │   values: {             │                       │                       │
 │     service_account_    │                       │                       │
 │       json: "{...}",   │                       │                       │
 │     scopes: [...]       │                       │                       │
 │   }}                    │                       │                       │
 │ ────────────────────►   │                       │                       │
 │                         │ ──────────────────►    │                       │
 │                         │   execute_resolve()    │                       │
 │                         │                        │                       │
 │                         │                        │ C::resolve():         │
 │                         │                        │ 1. Parse JSON key     │
 │                         │                        │ 2. Build JWT claims   │
 │                         │                        │ 3. Sign with RS256   │
 │                         │                        │ 4. POST /token ───────────────►
 │                         │                        │    {grant_type:       │
 │                         │                        │     jwt-bearer,      │
 │                         │                        │     assertion: JWT}   │
 │                         │                        │                       │
 │                         │                        │ 5. ◄─────────────────────────
 │                         │                        │    { access_token }   │
 │                         │                        │                       │
 │                         │                        │ Complete(State)       │
 │                         │                        │ State = key + token   │
 │                         │ ◄──────────────────    │                       │
 │                         │                        │                       │
 │ 201 Created             │                       │                       │
 │ ◄───────────────────    │                       │                       │
```

**Refresh:** re-mint JWT + exchange. No user interaction. Framework calls
refresh() automatically before token expires.

**Protocol transparency:** Google Sheets resource accepts `OAuth2Token`.
Both GoogleSheetsOAuth2 (user consent) and GoogleServiceAccount (JWT)
produce `OAuth2Token`. Resource doesn't care which.

---

## Flow 4: SAML 2.0 (POST binding, assertion-based)

**Used by:** Enterprise APIs protected by ADFS, Okta, OneLogin.
Workflow needs to access SharePoint, internal REST APIs behind SAML SSO.

### Sequence

```
User                    Nebula API              Framework              IdP (ADFS)
 │                         │                       │                       │
 │ POST /credentials       │                       │                       │
 │ { type: "saml",        │                       │                       │
 │   values: {             │                       │                       │
 │     idp_sso_url,        │                       │                       │
 │     idp_entity_id,      │                       │                       │
 │     idp_certificate,    │                       │                       │
 │     ...                 │                       │                       │
 │   }}                    │                       │                       │
 │ ────────────────────►   │                       │                       │
 │                         │ ──────────────────►    │                       │
 │                         │   execute_resolve()    │                       │
 │                         │                        │                       │
 │                         │                        │ C::resolve():         │
 │                         │                        │ Build AuthnRequest    │
 │                         │                        │ Return Pending {      │
 │                         │                        │   state: SamlPending, │
 │                         │                        │   interaction:        │
 │                         │                        │     FormPost {        │
 │                         │                        │       url: idp_sso,   │
 │                         │                        │       fields: [       │
 │                         │                        │         SAMLRequest,  │
 │                         │                        │         RelayState    │
 │                         │                        │       ]               │
 │                         │                        │     }                 │
 │                         │                        │ }                     │
 │                         │                        │                       │
 │ 202 + cookie +          │                       │                       │
 │ FormPost data           │                       │                       │
 │ ◄───────────────────    │                       │                       │
 │                         │                       │                       │
 │ Browser auto-submits ──────────────────────────────────────────────►    │
 │ POST to IdP with        │                       │                       │
 │ SAMLRequest + RelayState│                       │                       │
 │                         │                       │                       │
 │ User authenticates ◄───────────────────────────────────────────────    │
 │ at IdP (AD login)       │                       │                       │
 │                         │                       │                       │
 │ IdP POSTs back: ─────────────────────────────►  │                       │
 │ POST /callback/saml     │                       │                       │
 │ SAMLResponse=...        │                       │                       │
 │ RelayState=...          │                       │                       │
 │ Cookie: nebula_pending  │                       │                       │
 │ ────────────────────►   │                       │                       │
 │                         │ ──────────────────►    │                       │
 │                         │  execute_continue()    │                       │
 │                         │                        │                       │
 │                         │                        │ consume PendingState  │
 │                         │                        │ C::continue_resolve() │
 │                         │                        │  parse SAMLResponse   │
 │                         │                        │  verify signature     │
 │                         │                        │  extract attributes   │
 │                         │                        │  Complete(SamlState)  │
 │                         │                        │                       │
 │ 302 → success           │                       │                       │
 │ ◄───────────────────    │                       │                       │
```

### Key differences from OAuth2

| Aspect | OAuth2 | SAML |
|--------|--------|------|
| Callback method | GET (code in query) | POST (assertion in body) |
| InteractionRequest | Redirect { url } | FormPost { url, fields } |
| UserInput | Callback { params } | FormData { params } |
| Auth material | access_token + refresh_token | assertion + attributes |
| Refresh | Token refresh via refresh_token | Re-authenticate (ReauthRequired) |
| Expiry | Token expires_in (hours) | Assertion NotOnOrAfter (minutes) |

### SAML-specific security

- Assertion signature validated against IdP certificate
- Audience restriction validated against SP entity ID
- InResponseTo validated against stored AuthnRequest ID
- Clock skew tolerance: ±5 minutes
- RelayState = session correlation (matches session_id binding)

---

## Flow 5: OAuth2 Device Code (CLI / headless)

**Used by:** CLI tools, smart TVs, IoT devices. User authorizes on another device.
GitHub CLI (`gh auth login`), Azure CLI, Google Cloud CLI.

### Sequence

```
User (CLI)              Nebula API              Framework              Provider
 │                         │                       │                       │
 │ POST /credentials       │                       │                       │
 │ { type: "cli-oauth2",  │                       │                       │
 │   values: {             │                       │                       │
 │     client_id,          │                       │                       │
 │     device_auth_url,    │                       │                       │
 │     token_url           │                       │                       │
 │   }}                    │                       │                       │
 │ ────────────────────►   │                       │                       │
 │                         │ ──────────────────►    │                       │
 │                         │   execute_resolve()    │                       │
 │                         │                        │                       │
 │                         │                        │ C::resolve():         │
 │                         │                        │ POST /device/code ───────────►
 │                         │                        │ { client_id, scope }  │
 │                         │                        │                       │
 │                         │                        │ ◄───────────────────────────
 │                         │                        │ { device_code,        │
 │                         │                        │   user_code: "ABCD",  │
 │                         │                        │   verification_uri,   │
 │                         │                        │   interval: 5,        │
 │                         │                        │   expires_in: 300 }   │
 │                         │                        │                       │
 │                         │                        │ Pending {             │
 │                         │                        │   state: DevicePend,  │
 │                         │                        │   interaction:        │
 │                         │                        │     DisplayInfo {     │
 │                         │                        │       code: "ABCD",   │
 │                         │                        │       uri: "https://  │
 │                         │                        │        device.login"  │
 │                         │                        │     }                 │
 │                         │                        │ }                     │
 │                         │                        │                       │
 │ 202 + display info      │                       │                       │
 │ { user_code: "ABCD",   │                       │                       │
 │   verification_uri,     │                       │                       │
 │   poll_url, interval }  │                       │                       │
 │ ◄───────────────────    │                       │                       │
 │                         │                       │                       │
 │ Display to user:        │                       │                       │
 │ "Go to device.login     │                       │                       │
 │  and enter code ABCD"   │                       │                       │
 │                         │                       │                       │
 ┆ (user goes to browser,  │                       │                       │
 ┆  enters code, authorizes)                       │                       │
 │                         │                       │                       │
 │ POST /{id}/poll         │                       │                       │
 │ Cookie: nebula_pending  │                       │                       │
 │ ────────────────────►   │                       │                       │
 │                         │ ──────────────────►    │                       │
 │                         │   execute_continue()   │                       │
 │                         │                        │                       │
 │                         │                        │ PendingStore.get()    │
 │                         │                        │ (not consume — poll!) │
 │                         │                        │                       │
 │                         │                        │ C::continue_resolve() │
 │                         │                        │ POST /token ──────────────────►
 │                         │                        │ { grant_type:         │
 │                         │                        │   device_code,        │
 │                         │                        │   device_code: "..." }│
 │                         │                        │                       │
 │                         │                        │ ◄─────────────────────────────
 │                         │                        │ { error:              │
 │                         │                        │   "authorization_     │
 │                         │                        │    pending" }         │
 │                         │                        │                       │
 │                         │                        │ Retry { after: 5s }   │
 │                         │                        │                       │
 │ 202 { status: pending,  │                       │                       │
 │   retry_after: 5 }      │                       │                       │
 │ ◄───────────────────    │                       │                       │
 │                         │                       │                       │
 ┆ ... repeat poll ...     │                       │                       │
 │                         │                       │                       │
 │ POST /{id}/poll         │                       │                       │
 │ ────────────────────►   │                       │                       │
 │                         │ ──────────────────►    │                       │
 │                         │                        │                       │
 │                         │                        │ C::continue_resolve() │
 │                         │                        │ POST /token ──────────────────►
 │                         │                        │                       │
 │                         │                        │ ◄─────────────────────────────
 │                         │                        │ { access_token,       │
 │                         │                        │   refresh_token }     │
 │                         │                        │                       │
 │                         │                        │ PendingStore.consume()│
 │                         │                        │ Complete(OAuth2State) │
 │                         │                        │ Store encrypted       │
 │                         │                        │                       │
 │ 200 { status: active }  │                       │                       │
 │ ◄───────────────────    │                       │                       │
```

### Key difference: get() vs consume()

Device code flow **polls** — pending state must survive multiple reads.
Framework uses `PendingStateStore.get()` (non-destructive) during polling.
Only on final success: `consume()` (destructive, single-use).

This is why PendingStateStore has both `get()` and `consume()`.

### ResolveResult::Retry

continue_resolve() returns `Retry { after: Duration }` when provider says
"authorization_pending". Framework:
1. Does NOT consume pending state
2. Returns 202 + retry_after to client
3. Client polls again after interval

On success: returns `Complete(State)`. Framework consumes pending state.

---

## Flow 6: Static Credential with Test (API Key)

**Used by:** OpenAI, Stripe, Twilio (API key mode), Telegram Bot, SendGrid, etc.

### Sequence

```
User                    Nebula API              Framework              Provider
 │                         │                       │                       │
 │ POST /credentials       │                       │                       │
 │ { type: "openai",      │                       │                       │
 │   values: {             │                       │                       │
 │     api_key: "sk-..."   │                       │                       │
 │   }}                    │                       │                       │
 │ ────────────────────►   │                       │                       │
 │                         │ ──────────────────►    │                       │
 │                         │   execute_resolve()    │                       │
 │                         │                        │                       │
 │                         │                        │ C::resolve():         │
 │                         │                        │ Complete(BearerToken) │
 │                         │                        │ (immediate, no net)   │
 │                         │                        │                       │
 │                         │                        │ If TESTABLE:          │
 │                         │                        │ C::test(scheme, ctx)  │
 │                         │                        │ GET /v1/models ───────────────►
 │                         │                        │ Authorization: Bearer │
 │                         │                        │ sk-...                │
 │                         │                        │                       │
 │                         │                        │ ◄─────────────────────────────
 │                         │                        │ 200 OK                │
 │                         │                        │ TestResult::Success   │
 │                         │                        │                       │
 │                         │                        │ Store encrypted       │
 │                         │                        │                       │
 │ 201 { id, test: "ok" }  │                       │                       │
 │ ◄───────────────────    │                       │                       │
```

**Test-before-save:** If `TESTABLE = true`, framework calls `test()` after
resolve() completes but BEFORE storing. If test fails → credential not saved,
error returned to user with test failure reason.

**No PendingState, no callback, no redirect.** Simplest flow.

---

## Flow 7: Credential Refresh (automatic, background)

**Applies to:** OAuth2 (all providers), Kerberos, AWS STS, any credential
with `REFRESHABLE = true` and `expires_at()`.

### Sequence

```
(background)            CredentialResolver      RefreshCoordinator      Provider
                               │                       │                    │
 1. Workflow requests          │                       │                    │
    credential resolution      │                       │                    │
 ──────────────────────────►   │                       │                    │
                               │                       │                    │
 2. Load from store            │                       │                    │
    Decrypt + deserialize      │                       │                    │
    project() → Scheme         │                       │                    │
                               │                       │                    │
 3. Check expires_at()         │                       │                    │
    Token expires in 3 min     │                       │                    │
    REFRESH_POLICY.early_      │                       │                    │
    refresh = 5 min            │                       │                    │
    3 min < 5 min → REFRESH    │                       │                    │
                               │                       │                    │
                               │ ──────────────────►   │                    │
                               │   refresh_if_needed() │                    │
                               │                        │                    │
                               │                        │ DashMap entry()    │
                               │                        │ Idle → Refreshing  │
                               │                        │ scopeguard(notify) │
                               │                        │                    │
                               │                        │ timeout(30s,       │
                               │                        │   C::refresh(      │
                               │                        │     &mut state,    │
                               │                        │     &ctx           │
                               │                        │   )                │
                               │                        │ )                  │
                               │                        │                    │
                               │                        │ POST /token ───────────────►
                               │                        │ { grant_type:      │
                               │                        │   refresh_token }  │
                               │                        │                    │
                               │                        │ ◄──────────────────────────
                               │                        │ { access_token,    │
                               │                        │   expires_in }     │
                               │                        │                    │
                               │                        │ Mutate state:      │
                               │                        │   access_token =   │
                               │                        │     new_token      │
                               │                        │   expires_at =     │
                               │                        │     now + 3600s    │
                               │                        │                    │
                               │                        │ CAS write:         │
                               │                        │ store.put(id,      │
                               │                        │   state,           │
                               │                        │   CompareAndSwap)  │
                               │                        │                    │
                               │                        │ notify_waiters()   │
                               │                        │ (scopeguard drop)  │
                               │                        │                    │
                               │ ◄──────────────────    │                    │
                               │   refreshed scheme     │                    │
                               │                        │                    │
 4. Return fresh scheme        │                       │                    │
 ◄─────────────────────────    │                       │                    │
```

### Concurrent refresh (same credential, multiple workers)

```
Worker A: refresh_if_needed() → DashMap: Idle → Refreshing(notify)
Worker B: refresh_if_needed() → DashMap: Refreshing → wait on notify (timeout 60s)
Worker C: refresh_if_needed() → DashMap: Refreshing → wait on notify (timeout 60s)

Worker A: POST /token → success → CAS write → notify_waiters()
Worker B: notified → re-read from store → return fresh scheme
Worker C: notified → re-read from store → return fresh scheme
```

Only Worker A makes the network call. B and C wait and re-read.

### Multi-node refresh (CAS coordination)

```
Node 1: refresh → POST /token → success → CAS write(version=7, expected=6) → OK
Node 2: refresh → POST /token → success → CAS write(version=7, expected=6) → CONFLICT
Node 2: CAS conflict → re-read → check lifecycle → Active → return fresh scheme
```

Both nodes call the provider (unavoidable without distributed lock), but
only one wins the CAS write. Loser re-reads the winner's result.

---

## Credential Composition (AWS Assume Role pattern)

Some credentials depend on other credentials. AWS Assume Role needs a base
AWS credential (access key) to call STS. LDAP GSSAPI might need a Kerberos
credential. This is **credential composition** via `ctx.resolve_credential()`.

### Sequence (happy path)

```
Workflow                Framework              Base Credential         STS
 │                         │                       │                    │
 │ resolve AssumeRole      │                       │                    │
 │ ────────────────────►   │                       │                    │
 │                         │ resolve_credential()  │                    │
 │                         │ ─────────────────►    │                    │
 │                         │                       │ decrypt, project   │
 │                         │ ◄─────────────────    │                    │
 │                         │   AwsAuth             │                    │
 │                         │                       │                    │
 │                         │ call STS ──────────────────────────────►   │
 │                         │ assume_role()          │                    │
 │                         │                       │                    │
 │                         │ ◄─────────────────────────────────────    │
 │                         │   temp credentials    │                    │
 │                         │                       │                    │
 │                         │ Complete(AssumeState)  │                    │
 │ ◄───────────────────    │                       │                    │
```

### Error path — CompositionFailed

When `ctx.resolve_credential()` fails (base credential not found, scope
violation, refresh failed), the framework maps `ResolutionError` to
`CredentialError::CompositionFailed`:

```rust
// Inside credential author code:
let base: AwsAuth = ctx.resolve_credential(&state.base_credential_id).await?;
// If fails → CredentialError::CompositionFailed { source: Box<ResolutionError> }

// Framework then wraps the outer error:
// ResolutionError {
//     credential_id: "assume-role-cred",
//     stage: Resolve,
//     source: CredentialError::CompositionFailed {
//         source: ResolutionError {
//             credential_id: "base-aws-key",
//             stage: Decrypt,
//             source: ...
//         }
//     }
// }
```

**Error chain:** outer ResolutionError → CompositionFailed → inner ResolutionError.
API layer normalizes to `"credential unavailable"`. Operator logs show full chain.

### Error helpers

Credential authors construct errors with `error_source()` helper for string messages:

```rust
// String → Box<dyn StdError + Send> doesn't compile directly.
// Use error_source() helper:
Err(CredentialError::RefreshFailed {
    kind: RefreshErrorKind::TokenExpired,
    retry: RetryAdvice::Never,
    source: error_source("no refresh token available"),
})

// Or use the convenience method:
Err(CredentialError::refresh(
    RefreshErrorKind::TokenExpired,
    RetryAdvice::Never,
    "no refresh token available",
))
```

---

## Flow Summary

| Flow | Interactive? | PendingState? | Callback? | Redirect? | Refresh? |
|------|-------------|---------------|-----------|-----------|----------|
| OAuth2 Auth Code | Yes | OAuth2Pending | GET callback | Yes | Yes (refresh_token) |
| OAuth2 Client Creds | No | NoPendingState | No | No | Yes (re-mint) |
| Service Account JWT | No | NoPendingState | No | No | Yes (re-mint) |
| SAML 2.0 | Yes | SamlPending | POST callback | FormPost | No (ReauthRequired) |
| Device Code | Yes | DeviceCodePending | Poll | No | Yes (refresh_token) |
| Static API Key | No | NoPendingState | No | No | No |
| AWS Assume Role | No | NoPendingState | No | No | Yes (re-assume) |

### ResolveResult mapping

| Flow | resolve() returns | continue_resolve() returns |
|------|-------------------|----------------------------|
| OAuth2 Auth Code | Pending { state, Redirect } | Complete(OAuth2State) |
| OAuth2 Client Creds | Complete(OAuth2State) | n/a |
| Service Account JWT | Complete(ServiceAccountState) | n/a |
| SAML 2.0 | Pending { state, FormPost } | Complete(SamlState) |
| Device Code | Pending { state, DisplayInfo } | Retry / Complete |
| Static API Key | Complete(BearerToken) | n/a |
