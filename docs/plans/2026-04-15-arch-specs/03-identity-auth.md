# Spec 03 — Identity and authentication

> **Status:** draft
> **Canon target:** §12.5 (extend), §12.9 (new — telemetry)
> **Depends on:** 02 (tenancy)
> **Depended on by:** 04 (RBAC), 05 (API routing), 11 (triggers — service accounts)

## Problem

One codebase must serve:
- Solo self-host user who wants zero friction (no signup form)
- Cloud customer who needs real accounts, password reset, MFA, eventually SSO
- CI / API consumer who needs non-human identity (service accounts, PAT)
- Enterprise customer who needs SSO (v2+)

Auth is security-critical: any bug is potentially a P0 incident. The solution must balance DX friction against security correctness.

## Decision

**Built-in auth via a new `nebula-auth` crate** using Rust ecosystem primitives (`argon2`, `oauth2`, `openidconnect`, `samael`, `lettre`, `totp-rs`, `tower-sessions`, `governor`). No external identity provider required. Local dev mode `NEBULA_AUTH_MODE=none` gives zero-friction demo. Product analytics is a **separate** concern with its own opt-out telemetry crate `nebula-diagnostics`.

## Data model

### User

```rust
// nebula-core (or nebula-auth if we keep core minimal)
pub struct UserId(Ulid);  // prefix "user_"

pub struct User {
    pub id: UserId,
    pub email: String,              // unique, lowercased
    pub email_verified_at: Option<DateTime<Utc>>,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub password_hash: Option<String>,  // None if OAuth-only
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub locked_until: Option<DateTime<Utc>>,  // rate limit / brute force
    pub failed_login_count: u32,
    pub mfa_enabled: bool,
    pub mfa_secret: Option<Vec<u8>>,    // TOTP secret, zeroized on drop
    pub version: u64,
}

pub struct OAuthLink {
    pub user_id: UserId,
    pub provider: OAuthProvider,
    pub provider_user_id: String,
    pub provider_email: Option<String>,
    pub linked_at: DateTime<Utc>,
}

pub enum OAuthProvider {
    Google,
    GitHub,
    Microsoft,
    // Generic OIDC for v2
    Oidc { issuer: String },
}
```

### Session

```rust
pub struct SessionId(Ulid);  // prefix "sess_"

pub struct Session {
    pub id: SessionId,
    pub user_id: UserId,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub ip_address: IpAddr,
    pub user_agent: String,
    pub revoked_at: Option<DateTime<Utc>>,
}
```

### Personal Access Token (PAT)

```rust
pub struct PatId(Ulid);  // prefix "pat_"

pub struct PersonalAccessToken {
    pub id: PatId,
    pub principal_id: PrincipalId,   // UserId or ServiceAccountId
    pub name: String,                // user-supplied description
    pub prefix: String,              // first 8 chars for display ("pat_01J9...")
    pub hash: Vec<u8>,               // sha256 of full token
    pub scopes: Vec<PatScope>,       // optional restrictions
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

pub enum PrincipalId {
    User(UserId),
    ServiceAccount(ServiceAccountId),
}

pub enum PatScope {
    ReadOnly,
    WorkflowsOnly,
    ExecutionsOnly,
    // Full access is the default when list is empty
}
```

### Service Account

```rust
pub struct ServiceAccountId(Ulid);  // prefix "sa_"

pub struct ServiceAccount {
    pub id: ServiceAccountId,
    pub org_id: OrgId,
    pub slug: String,               // unique per org
    pub display_name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub created_by: UserId,
    pub disabled_at: Option<DateTime<Utc>>,
}
```

### Password reset / email verification

```rust
pub struct VerificationToken {
    pub token_hash: Vec<u8>,   // sha256 of token value
    pub user_id: UserId,
    pub kind: VerificationKind,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

pub enum VerificationKind {
    EmailVerification,
    PasswordReset,
    OrgInvite { org_id: OrgId, role: OrgRole },
    MfaRecovery,
}
```

### SQL schema

```sql
CREATE TABLE users (
    id                 BYTEA PRIMARY KEY,
    email              TEXT NOT NULL,
    email_verified_at  TIMESTAMPTZ,
    display_name       TEXT NOT NULL,
    avatar_url         TEXT,
    password_hash      TEXT,                      -- argon2id encoded
    created_at         TIMESTAMPTZ NOT NULL,
    last_login_at      TIMESTAMPTZ,
    locked_until       TIMESTAMPTZ,
    failed_login_count INT NOT NULL DEFAULT 0,
    mfa_enabled        BOOLEAN NOT NULL DEFAULT FALSE,
    mfa_secret         BYTEA,                     -- encrypted at rest
    version            BIGINT NOT NULL DEFAULT 0,
    deleted_at         TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_users_email_active
    ON users (LOWER(email))
    WHERE deleted_at IS NULL;

CREATE TABLE oauth_links (
    user_id            BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider           TEXT NOT NULL,
    provider_user_id   TEXT NOT NULL,
    provider_email     TEXT,
    linked_at          TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (provider, provider_user_id)
);

CREATE INDEX idx_oauth_links_user ON oauth_links (user_id);

CREATE TABLE sessions (
    id                 BYTEA PRIMARY KEY,
    user_id            BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at         TIMESTAMPTZ NOT NULL,
    last_active_at     TIMESTAMPTZ NOT NULL,
    expires_at         TIMESTAMPTZ NOT NULL,
    ip_address         INET,
    user_agent         TEXT,
    revoked_at         TIMESTAMPTZ
);

CREATE INDEX idx_sessions_user_active
    ON sessions (user_id)
    WHERE revoked_at IS NULL AND expires_at > NOW();

CREATE TABLE personal_access_tokens (
    id                 BYTEA PRIMARY KEY,
    principal_kind     TEXT NOT NULL,       -- 'user' or 'service_account'
    principal_id       BYTEA NOT NULL,
    name               TEXT NOT NULL,
    prefix             TEXT NOT NULL,
    hash               BYTEA NOT NULL,      -- sha256
    scopes             JSONB NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL,
    last_used_at       TIMESTAMPTZ,
    expires_at         TIMESTAMPTZ,
    revoked_at         TIMESTAMPTZ
);

CREATE INDEX idx_pat_hash ON personal_access_tokens (hash) WHERE revoked_at IS NULL;
CREATE INDEX idx_pat_principal ON personal_access_tokens (principal_kind, principal_id);

CREATE TABLE service_accounts (
    id                 BYTEA PRIMARY KEY,
    org_id             BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    slug               TEXT NOT NULL,
    display_name       TEXT NOT NULL,
    description        TEXT,
    created_at         TIMESTAMPTZ NOT NULL,
    created_by         BYTEA NOT NULL REFERENCES users(id),
    disabled_at        TIMESTAMPTZ,
    deleted_at         TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_sa_org_slug
    ON service_accounts (org_id, slug)
    WHERE deleted_at IS NULL;

CREATE TABLE verification_tokens (
    token_hash         BYTEA PRIMARY KEY,
    user_id            BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind               TEXT NOT NULL,
    payload            JSONB,               -- kind-specific data (invite details, etc.)
    created_at         TIMESTAMPTZ NOT NULL,
    expires_at         TIMESTAMPTZ NOT NULL,
    consumed_at        TIMESTAMPTZ
);

CREATE INDEX idx_verification_user_kind
    ON verification_tokens (user_id, kind)
    WHERE consumed_at IS NULL;
```

## Flows

### Self-host first run (no-auth → full auth upgrade)

```
1. Process starts in self-host mode
2. Middleware runs SELECT COUNT(*) FROM users  
   - If count == 0: all requests redirect to /setup (bootstrap mode)
   - If count > 0: normal auth middleware applies
3. /setup renders account creation form
4. User submits email + password (minimum 12 chars, zxcvbn score ≥ 3)
5. Transaction:
   - INSERT user with argon2id(password) hash
   - email_verified_at = NOW() (self-host skips email verification)
   - Create default org + workspace (see spec 02)
   - Create org_member(user, org, OrgOwner)
   - Create session
   - Set cookie
6. Redirect to /default/default dashboard
```

### Cloud signup

```
1. User visits nebula.io, lands on marketing page
2. Click "Get started" → /signup
3. Form: email + password + optional display name
   OR: "Continue with Google/GitHub" buttons (OAuth)
4. On submit:
   - INSERT user with password_hash (email_verified_at = NULL)
   - Send email with verification token (lettre via SMTP)
   - Redirect to /signup/check-email
5. User clicks link in email
6. /verify?token=xxx
   - Look up verification_tokens by sha256(token)
   - Mark user.email_verified_at = NOW()
   - Mark token.consumed_at = NOW()
   - Create session
   - Redirect to /onboarding (create first org)
7. Onboarding: choose org slug, display name, plan (free by default)
8. Create org + default workspace + memberships (spec 02)
9. Land in /{org}/default dashboard
```

### Login

```
1. POST /login with {email, password}
2. Rate limit check: max 5 attempts per IP per minute (governor)
3. Look up user by email
4. If user not found: constant-time fake argon2 verify, return 401 (prevents user enumeration)
5. If user.locked_until > NOW(): return 423 Locked
6. argon2::verify(password, user.password_hash)
7. If mismatch:
   - user.failed_login_count += 1
   - If failed_login_count >= 5: user.locked_until = NOW() + 15 min
   - Return 401
8. If MFA enabled:
   - Return 200 { "mfa_required": true, "mfa_token": <temp token> }
   - Client prompts for TOTP, POSTs to /login/mfa
   - totp_rs::verify(code, user.mfa_secret, time_window=30s)
   - If valid: proceed to session creation
9. Create session, set cookie, return 200
10. Reset user.failed_login_count = 0
```

### OAuth login/link

```
1. User clicks "Continue with Google"
2. Redirect to Google with PKCE state
3. Google redirects back to /oauth/callback/google?code=xxx&state=yyy
4. Verify state matches session
5. Exchange code for tokens (oauth2 crate)
6. Fetch userinfo from Google
7. Look up oauth_links by (google, google_user_id)
   - If exists: log in as that user
   - If not but email matches existing user: link account (requires password confirmation or email verification)
   - If not and email unknown: new user signup via OAuth
```

### Password reset

```
1. POST /forgot-password {email}
2. Rate limit: max 3 per hour per email
3. Look up user, generate token (32 bytes random), store sha256 in verification_tokens
4. Email link: https://nebula.io/reset?token=xxx (expires in 1h)
5. User clicks link → /reset form
6. POST /reset {token, new_password}
7. Verify token in verification_tokens, unused, not expired
8. Update user.password_hash, mark token consumed
9. Revoke all existing sessions for this user
10. Force re-login
```

### PAT creation

```
1. User goes to /settings/tokens → "Create token"
2. Enter name, optional expiry, optional scopes
3. Server generates 32 random bytes, encodes as "pat_" + base32 → token string
4. Compute sha256 of token → hash
5. INSERT personal_access_tokens(prefix=first 12 chars, hash=sha256, ...)
6. Return token ONCE to user (cannot be retrieved later)
7. User copies it, uses in CI / CLI
```

### PAT use (API request)

```
1. Incoming request with Authorization: Bearer pat_01J9XYZ...
2. Middleware extracts token
3. Compute sha256(token)
4. Look up in personal_access_tokens WHERE hash = sha256 AND revoked_at IS NULL
5. Check expires_at
6. Load principal (user or service_account)
7. Update last_used_at (async, non-blocking, throttled to once per minute per PAT)
8. Construct Principal { kind, id }, pass to handler
```

## No-auth mode (local dev)

```rust
// When NEBULA_AUTH_MODE=none, a synthetic middleware returns:
fn fake_auth_middleware() -> Principal {
    Principal::User(UserId::from_slug("user_default"))
}
```

- `user_default` is a synthetic user that always exists in memory
- Belongs to synthetic `org_default` with `WorkspaceAdmin` role
- **Never in production:** explicit env var, warning at startup in bold:
  ```
  ⚠ NEBULA_AUTH_MODE=none — all requests run as default_user, no authentication
  ⚠ This mode is for local development only. Do not expose to network.
  ```
- Binding check: if `NEBULA_AUTH_MODE=none` AND listen address is not loopback → refuse to start

## MFA (TOTP)

```
1. User enables MFA in /settings/security
2. Generate 20 random bytes → totp_secret
3. Encode as base32 → display in QR code (otpauth://totp/Nebula:user@example.com?secret=xxx)
4. User scans with Google Authenticator / 1Password / Authy
5. User enters 6-digit code → verify totp_rs::verify
6. On success: user.mfa_enabled = true, user.mfa_secret = encrypt(totp_secret)
7. Generate 10 backup codes, show once, store sha256 in separate table
```

**MFA secret encryption at rest:** use `nebula-credential` encryption primitives — never store raw secret in DB. Key material comes from `NEBULA_MASTER_KEY` env var (self-host) or KMS (cloud).

## Edge cases

**Duplicate email signup:** DB unique constraint on `LOWER(email)`. Return user-friendly error «email already registered» without disclosing whether account exists (constant-time response).

**Stale OAuth link:** user deletes Google account, OAuth link row orphaned. Not a problem — next login attempt fails gracefully. Cleanup job optional.

**Session fixation:** session ID is regenerated on login (old session revoked, new one created). No cookie reuse across auth boundaries.

**CSRF:** all state-changing endpoints require double-submit cookie pattern OR same-origin via CORS. GET endpoints are safe by convention.

**Password spray attack:** rate limit per IP and per user (whichever hits first). Repeat failures trigger user lock (15 min), then IP lock (1h).

**User enumeration via timing:** constant-time password verification, identical responses for valid/invalid users.

**MFA bypass via password reset:** password reset alone does not disable MFA. Separate flow for MFA recovery using backup codes.

**Lost MFA + lost backup codes:** contact support flow (manual intervention for cloud; `nebula user disable-mfa <email>` CLI for self-host with confirmation).

**Deleted user with active sessions:** user row soft-deleted, sessions revoked in same transaction.

**Deleted user referenced by workflows/executions:** `created_by` columns use SET NULL on delete, not cascade. History retained, user becomes «[deleted user]» in UI.

## Configuration surface

```toml
[auth]
mode = "built-in"              # or "none" for local dev
signup_enabled = true          # cloud: true; self-host: true after first user
email_verification_required = true  # cloud: true; self-host: false

[auth.password]
min_length = 12
require_zxcvbn_score = 3       # 0-4 scale
argon2_memory_kib = 19456      # OWASP 2023 recommendation
argon2_iterations = 2
argon2_parallelism = 1

[auth.session]
cookie_name = "nebula_session"
cookie_secure = true           # false only when development
cookie_same_site = "Lax"       # "Strict" for paranoid
max_lifetime = "7d"
idle_timeout = "24h"           # inactivity logout

[auth.mfa]
enabled = true
totp_window_seconds = 30
backup_codes_count = 10

[auth.oauth]
google.client_id = "..."
google.client_secret_env = "NEBULA_GOOGLE_OAUTH_SECRET"
github.client_id = "..."
github.client_secret_env = "NEBULA_GITHUB_OAUTH_SECRET"

[auth.rate_limit]
login_per_ip_per_minute = 5
login_per_user_per_minute = 5
signup_per_ip_per_hour = 10
password_reset_per_email_per_hour = 3

[email]
from = "noreply@nebula.io"
smtp_host = "smtp.example.com"
smtp_port = 587
smtp_user = "..."
smtp_password_env = "NEBULA_SMTP_PASSWORD"
```

## Testing criteria

**Unit tests:**
- `argon2::hash_password` → `argon2::verify` round-trip
- `generate_pat` produces unique tokens, hash matches
- Token expiry logic
- TOTP verify with time window

**Integration tests:**
- Full signup → email verification → login flow (cloud mode)
- Self-host first run → bootstrap account
- OAuth flow (with mock provider)
- Password reset flow
- Session regeneration on login (fixation prevention)
- PAT creation, usage, revocation
- Service account + PAT lifecycle
- Rate limit triggers on 6th login attempt
- Account lock after 5 failed attempts + unlock after 15 min
- MFA enrollment, login, backup code usage
- No-auth mode: bind to non-loopback refused

**Security tests:**
- Timing attack: user-not-found vs wrong-password responses are constant-time
- CSRF token required on state-changing endpoints
- Session cookie flags: Secure, HttpOnly, SameSite
- Password argon2 parameters meet OWASP 2023
- SQL injection on email / token fields
- OAuth state parameter verified (no CSRF on callback)

**Property tests:**
- PAT hash is one-way (given hash, cannot recover token)
- Tokens have sufficient entropy (min 256 bits)

## Performance targets

- Login endpoint: **< 200 ms p99** (argon2 dominates — intentional)
- PAT verification (API requests): **< 5 ms p99** (sha256 + index lookup)
- Session lookup: **< 2 ms p99**
- Signup: **< 500 ms p99** (argon2 + DB + email queue)

## Module boundaries

| Component | Crate |
|---|---|
| `UserId`, `SessionId`, `PatId`, `ServiceAccountId`, `Principal` | `nebula-core` |
| `User`, `Session`, `OAuthLink` types | `nebula-core` |
| `argon2_hash`, `argon2_verify`, session management | `nebula-auth` (new) |
| OAuth client | `nebula-auth` (new) |
| SSO SAML/OIDC (v2) | `nebula-auth` (new) |
| `AuthBackend` trait (future-proofing) | `nebula-auth` |
| `AuthRepo` (DB access) | `nebula-storage` |
| Login / signup handlers | `nebula-api` |
| Middleware (session/PAT extraction) | `nebula-api` |

## Product telemetry (separate concern)

New crate **`nebula-diagnostics`** — completely separate from auth. Purpose: let us see how the OSS product is used without tracking individual users.

### What it sends

```rust
pub struct DiagnosticsReport {
    pub instance_id: Uuid,          // generated once at first run, persisted
    pub nebula_version: String,
    pub os: String,                  // "linux-x86_64"
    pub rust_version: String,
    pub storage_backend: String,     // "sqlite" / "postgres"
    pub deployment_mode: String,     // "self-host" / "cloud"
    pub uptime_seconds: u64,
    pub n_orgs: u64,
    pub n_workspaces: u64,
    pub n_workflows_active: u64,
    pub n_workflows_total: u64,
    pub n_executions_24h_succeeded: u64,
    pub n_executions_24h_failed: u64,
    pub n_executions_24h_cancelled: u64,
    pub median_execution_duration_ms: u64,
    pub p95_execution_duration_ms: u64,
    pub integration_types_used: HashMap<String, bool>,  // {"slack": true, "postgres": true}
    pub features_used: HashMap<String, bool>,            // {"process_sandbox": true, ...}
}
```

### What it NEVER sends

- Workflow names or definitions
- Credential names or values
- Execution inputs or outputs
- User emails, IPs, hostnames
- External endpoint URLs
- Stack traces (crash reports are a separate opt-in layer)

### User control

```bash
nebula telemetry disable       # opt out permanently
nebula telemetry enable        # opt back in
nebula telemetry preview       # show exact JSON that would be sent
nebula telemetry status        # show current state
```

Environment variable `NEBULA_TELEMETRY=off` overrides config file.

### First-run disclosure

On first start, print to stderr:
```
Nebula v0.1.0 started
  Storage: SQLite (/var/lib/nebula/db.sqlite)

Anonymous usage metrics are enabled by default. They help us understand
what features matter and what breaks in the wild. We never send workflow
content, credentials, or PII.

  See exactly what is sent:  nebula telemetry preview
  Disable:                   nebula telemetry disable
  Learn more:                https://nebula.io/docs/telemetry
```

### Transport

- POST to `https://telemetry.nebula.io/v1/report` once every 24 hours
- Failure tolerant: network error → retry next cycle
- No retries on 4xx (config problem — log and skip)

## Open questions

- **Passkey / WebAuthn** — second factor or replacement for password? Deferred to v1.5.
- **Social login providers beyond Google/GitHub** — Microsoft, Apple, Discord? Add based on user demand.
- **Hardware security keys (FIDO2)** — enterprise requirement? Deferred.
- **Audit log for auth events** — login success/failure, PAT usage, MFA events. Integrate with `nebula-diagnostics`? Or separate audit table? Deferred — likely separate audit table owned by spec #18 (observability).
