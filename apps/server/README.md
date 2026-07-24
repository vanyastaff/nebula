# nebula-server

Composition-root binary for the Nebula API. Wires the `nebula-api` HTTP
surface to one of three ingress transports (`api`, `webhook`,
`realtime`, or `all`) and instantiates the currently configured runtime ports
(storage adapters, idempotency store, identity backend, email transport,
metrics + telemetry exporters). Tenant membership policy is deliberately not
wired by the default binary yet; this is an explicit K4 composition gap.

Run the default profile locally:

```bash
cargo run -p nebula-server                       # NEBULA_TRANSPORT=all
cargo run -p nebula-server -- --transport api    # REST only
cargo run -p nebula-server -- --transport webhook
cargo run -p nebula-server -- --transport realtime
```

All operator-facing configuration lives in environment variables; the
canonical registry is `crates/api/src/config/env.rs`. The composition
root in `apps/server/src/compose.rs` is the only place those values
turn into concrete `Arc<dyn …>` ports.

## Tenant membership and credential authority

The default server has no operator-configurable `MembershipStore`. Every
org/workspace route, including Plane-B credential management, therefore returns
an honest 503 before performing tenant work. An unwired or failed policy source
is unavailable; once a future supported source is wired, a valid snapshot with
no organization membership is denied rather than treated as administrator.
The composition root also leaves the credential command gateway unmounted in
that state, so it does not construct an unusable management surface behind a
guaranteed authority failure.

The in-memory membership adapter and `AppState::with_membership_store` are
internal/reference composition seams. They do not make direct `nebula-api`
embedding a supported deployment surface. K4 must ship the apps-owned durable
bridge/operator configuration and the curated SDK composition façade.

## Webhook credential bridge

`nebula-api` exposes only the object-safe, credential-neutral
`WebhookSecretResolver` bootstrap port. The first-party adapter lives in
`apps/server/src/webhook_credential_resolver.rs`: it resolves a credential
through the shared `CredentialService`, enforces the activation row's tenant
scope, and converts stored `whsec_` material to raw HMAC bytes. Its failures
cross the API port as a closed, secret-free classification; credential or
provider error text is never forwarded to API logs or problem responses.

Webhook registration still generates and returns the one-time `whsec_` value
inside the HTTP boundary. That generator is API-private and is not a public
credential-runtime or integration surface.

## Execution-store backend

The server's execution engine (workflow execution rows, control queue, journal)
is backed by one of three selectable stores. Choose based on your deployment needs.

### Behaviour contract

| `API_EXECUTION_BACKEND` | Store | When to use |
|-------------------------|-------|-------------|
| **unset** / `memory`    | In-memory (dev default) | Local development. Execution state is lost on restart. Cannot be shared across processes. |
| `sqlite`                | WAL-mode SQLite file | Single-process production. State survives restarts. Not shareable across hosts or concurrent writers. |
| `postgres`              | PostgreSQL (build with `--features postgres`) | Multi-process or multi-host production. State is shared across all replicas that point at the same database. |

Without an explicit `API_EXECUTION_BACKEND`, the server uses in-memory adapters
and emits a `tracing::warn!` at startup when `NEBULA_ENV` is not `dev` / `development`
/ `local`. This matches the idempotency-store convention.

### Env vars

| Variable | Type | Default | Notes |
|----------|------|---------|-------|
| `API_EXECUTION_BACKEND` | enum | `memory` | Case-insensitive: `memory`, `sqlite`, `postgres`. |
| `API_EXECUTION_DB_PATH` | string | `nebula-server-execution.db` | SQLite only. Path relative to the working directory. |
| `DATABASE_URL` | string | — | Postgres only. Standard sqlx DSN (`postgres://user:pass@host/db`). Required when `API_EXECUTION_BACKEND=postgres`. |

### NodeResult and Checkpoint stores

`NodeResultStore` and `CheckpointStore` always use in-memory adapters regardless
of `API_EXECUTION_BACKEND`. These stores hold transient per-execution data (node
output slots and stateful action checkpoints) that are written and read within a
single execution lifetime. Durability is provided by the `ExecutionStore` state
machine (one JSON blob per execution row); on a crash the reclaim sweep re-delivers
the job and the engine re-executes affected nodes from the last persisted state.

### Example: SQLite single-process production

```bash
API_EXECUTION_BACKEND=sqlite
API_EXECUTION_DB_PATH=/var/lib/nebula/execution.db
```

### Example: Postgres multi-process production

```bash
API_EXECUTION_BACKEND=postgres
DATABASE_URL=postgres://nebula:secret@db.internal/nebula_prod
```

Build with the `postgres` feature:

```bash
cargo build --release -p nebula-server --features postgres
```

## Identity backend and Plane-A OAuth

`API_AUTH_BACKEND` selects the Plane-A identity store independently from the
execution and idempotency stores. The selection is fail-closed: requesting
Postgres without the feature, `DATABASE_URL`, or a reachable database aborts
startup instead of silently losing users, sessions, or PATs into memory.

| `API_AUTH_BACKEND` | Identity backend | Durability |
|--------------------|------------------|------------|
| **unset** / `memory` | `InMemoryAuthBackend` | Process-local; lost on restart and not shared across replicas |
| `postgres` | `PgAuthBackend` (build with `--features postgres`) | Users, sessions, PATs, verification/OAuth state, and external identity links survive restart and are shared through `DATABASE_URL` |

### PostgreSQL identity-authority upgrade runbook

The release containing Postgres migration `0038` is an intentional coordinated
cutover; mixed old/new auth nodes are unsupported.

1. Stop or drain every old auth writer. Take and inventory the required
   pre-upgrade backup, then treat that backup and its WAL as plaintext-MFA
   sensitive material.
2. Configure one stable base64 AES-256 `NEBULA_CRED_MASTER_KEY` (or explicitly
   opt into the insecure local-only `NEBULA_CRED_DEV_KEY=1` policy). Run
   `task db:migrate`; all schema changes come from numbered migrations.
3. Start the new server. Before `PgAuthBackend` is exposed, the startup
   migrator serializes replicas with an advisory lock, converts canonical
   historical TOTP seeds in bounded CAS batches, authenticates active and
   pending envelopes with user/purpose-bound AAD, and fails closed on a safe
   reason plus truncated owner correlation.
4. Expect every pre-upgrade browser session to be invalidated. Migration
   `0038` discards raw stored cookie bearers and the new runtime persists only
   domain-separated SHA-256 lookup digests; users must authenticate again.
5. After convergence, test backup restore into an isolated environment. Expire
   or quarantine pre-migration backups, WAL archives, snapshots, and replicas
   under the incident-retention policy. Live-row encryption does not erase
   plaintext from those historical media.

Do not retire an old encryption key while any retained backup contains an
envelope produced by it. The library exposes explicit decrypt-only legacy keys
for controlled rotation, but the first-party server currently resolves only
the current `NEBULA_CRED_MASTER_KEY`; retain the old key and use a reviewed
explicit composition, or re-enroll MFA in strict environments. Never rotate the
environment key in place and assume old backups or rows remain recoverable.

The composition root also owns the only supported Plane-A OAuth runtime
lifecycle:

1. It loads and validates the credentials-only Google/GitHub.com provider set
   and canonical `API_PUBLIC_URL`. `ApiConfig` owns those `SecretString`s only
   during this boot phase.
2. `OAuthIdentityRuntime::from_config` returns `None` for an empty provider set;
   no OAuth HTTP client or egress capability exists in that process.
3. A non-empty set is moved out of `ApiConfig` into exactly one opaque runtime
   before the Memory/Postgres branch. The router config retains an empty OAuth
   map; the selected backend receives the same `Arc`, and neither backend
   constructs a client or retains raw/duplicate provider secrets.

The runtime fixes the production egress policy: rustls HTTPS only; redirects,
retries, and proxies disabled; every literal/DNS address must be globally
routable; reqwest receives only the exact validated answers. It also owns the
Google discovery cache/singleflight, outbound semaphore, and 30-second
per-operation network deadlines; every callback stage reuses its one original
deadline. It also owns bounded zeroizing provider buffers and the opaque
bearer-token capability. There is no public raw-client or custom-cfg escape
hatch.

Credential OAuth refresh uses the same credential-owned endpoint and
global-unicast address policy through a separate private server adapter.
Its reqwest client is likewise rustls/HTTPS-only with redirects, retries,
referer propagation, Hickory fallback, and implicit system proxies disabled.
The custom resolver rejects empty, excessive, non-global, and mixed DNS
answers, then returns that exact validated set to reqwest's connect path.
Provider-required endpoint queries are retained but the endpoint, response
body, request/response DTOs, and transport errors have constant redacted
diagnostics. The application-owned accumulated response buffer is zeroized on
every success and failure path; reqwest/rustls necessarily retain their own
short-lived transport buffers outside this type-level guarantee.

Provider configuration is opt-in only through
`API_AUTH_OAUTH_{GOOGLE,GITHUB}_{CLIENT_ID,CLIENT_SECRET}`. Either variable
declares the profile; an incomplete pair aborts startup. Google discovery URL,
issuer and scopes plus GitHub.com endpoints/scopes are fixed by the runtime.
Microsoft, generic OIDC, GitHub Enterprise Server, endpoint/scope/auth overrides,
and operator JWKS abort startup with a secret-free error. GitHub.com uses fixed
`client_secret_post`; Google prefers discovered `client_secret_basic`, falls
back to Post, and uses the OIDC Basic default when metadata omits the field.
Basic credentials are form-encoded component-wise before the colon/Base64 step.
An undeclared admitted provider remains an honest 503.
OAuth start and callback traffic must use the authority configured by
`API_PUBLIC_URL`; proxies must preserve that public `Host`. Start sets an
opaque per-flow `__Host-` transaction cookie and callback requires the exact
cookie before state consumption or provider egress. Browser clients therefore
need a same-site, cookie-preserving start request; non-browser clients must
retain and replay the matching `Set-Cookie`. A start request carrying eight
Nebula transaction-cookie names is rejected with 429 before state creation;
this is a request-local Cookie-header bound, not a globally atomic browser
quota. The independent hard admission gate permits at most 10,000 live OAuth
states per Memory process or shared PostgreSQL deployment; full or contended
admission returns 429 without state, PKCE, or cookie creation.

Callback persistence and network work are deliberately separated: matching
state is consumed atomically first, provider egress runs without database locks,
then the finalizer atomically decides local identity/session state. Email never
authorizes an implicit account link; collision returns 409 and no session. A
valid provider-error callback consumes state without egress and returns a fixed
401. A valid new identity without a policy-acceptable verified email returns
403 and writes no link/session; provider transport/non-success or malformed
identity payloads remain 502. If the authoritative linked user has MFA enabled,
the finalizer atomically stores an opaque challenge plus MFA-required outcome
and the callback returns
202 without session/CSRF material; `/auth/login/mfa` completes the login.
See `crates/api/README.md` for the full provider matrix and redirect URI shape.
This is identity OAuth (Plane A); integration credential acquisition (Plane B)
continues through the universal `resolve` / `resolve/continue` contract once
tenant membership authority is provisioned. In the default binary those tenant
routes currently return 503, as described above.

## Credential persistence backend

`NEBULA_CRED_DB` independently selects the Plane-B credential database. It
does not inherit `DATABASE_URL` and never reuses the execution/auth pool:
credential schema admission, migrations, rows, refresh claims, and sentinel
evidence have one credential-owned readiness lifecycle.

| `NEBULA_CRED_DB` | Backend | Deployment |
|------------------|---------|------------|
| **unset** | `sqlite://nebula-credentials.db?mode=rwc` | Durable single-process default |
| `sqlite://…`, `sqlite::memory:`, or a bare relative/absolute/Windows path | SQLite | Single process; the memory form is test/development only |
| `postgres://…` or `postgresql://…` | PostgreSQL (build with `--features postgres`) | Shared multi-replica production |

An explicit unsupported URL scheme aborts startup. A malformed PostgreSQL
locator such as `postgres:…` / `postgresql:…` also aborts instead of being
interpreted as a SQLite path. Requesting PostgreSQL from a build without the
`postgres` feature likewise aborts; none of these cases falls back to SQLite or
memory. Startup diagnostics expose only the backend class and closed error
taxonomy—database URLs, credentials, and tenant-specific paths are never
logged.

Plane-B credential persistence and refresh coordination share the same admitted
private pool for either supported backend. The server creates a unique
`nebula-server:<uuid>` replica identity on each process start and retains one
periodic reclaim-sweep guard until `serve` exits. Expired pre-provider claims
may be reclaimed; expired `RefreshInFlight` claims remain durable
`OutcomeUnknown` poison, are accounted exactly once, and never become
replayable merely because TTL elapsed. There is no in-memory claim fallback in
the production composition.

## Email delivery (SMTP)

The API needs an `EmailPort` to ship sign-up verification and
password-reset links. `nebula-api` ships the trait + a dev-only
`EchoSink` that buffers messages in-process so the local-first boot
path requires no SMTP server. The production transport
(`apps/server::email::SmtpEmailPort`, backed by `lettre = "0.11"`) is
wired here.

### Behaviour contract

| `API_SMTP_HOST` | Resulting port | When to use |
|-----------------|----------------|-------------|
| **unset**       | `EchoSink` (dev) | Local development, in-process tests. Messages are visible via the in-memory backend's `emails()` accessor. |
| **set + valid** | `SmtpEmailPort` | Production. Verification mails actually leave the process. |
| **set + invalid** | startup error (`TransportInitError::SmtpEmailPortInit`) | Fail-CLOSED. An operator who asked for SMTP and got the config wrong sees the error at boot, not as a silent fallback to `EchoSink` that would swallow auth mails. |

The same `Arc<dyn EmailPort>` is shared between `AppState::email_port`
and the selected `AuthBackend`, so forward-compat non-auth consumers
(org invitations, billing notices) inherit the transport without extra
wiring.

### Env vars

All keys are prefixed with `API_SMTP_`. `API_SMTP_HOST` is the
sentinel: present means the operator wants SMTP, absent means
`EchoSink`. Validation is fail-closed; the table below lists the
strict-on-startup checks.

| Variable | Type | Default | Validation |
|----------|------|---------|------------|
| `API_SMTP_HOST` | string | — (none → EchoSink) | Non-empty when present. |
| `API_SMTP_PORT` | u16    | `587` | Parsed by `u16::from_str`. |
| `API_SMTP_FROM` | string | — | Required when `API_SMTP_HOST` is set. Must contain `@`. Becomes the canonical `From` header for every outbound mail (a handler cannot smuggle a different sender). |
| `API_SMTP_USERNAME` | string | — | Optional. If set, `API_SMTP_PASSWORD` MUST also be set, or boot fails with `ApiConfigError::SmtpAuthIncomplete`. |
| `API_SMTP_PASSWORD` | string | — | Wrapped in `secrecy::SecretString` immediately on parse; `Debug` redacts and the buffer zeroizes on drop. Never logged. Never round-tripped through `serde` (`#[serde(skip)]` on the field). |
| `API_SMTP_TLS_MODE` | enum   | port-derived | `none` / `starttls` / `implicit`. If unset: `465` → `implicit`, `587` → `starttls`, anything else → `none`. `none` emits a `tracing::warn!` at startup because plaintext SMTP is dev-only. |

Recognized enum values for `API_SMTP_TLS_MODE` (case-insensitive):

- `starttls` (also `start_tls`, `start-tls`) — STARTTLS upgrade.
- `implicit` (also `smtps`) — TLS from the first byte.
- `none` — plaintext (dev only; warns at startup).

### Example: production gmail-style relay

```bash
API_SMTP_HOST=smtp.example.com
API_SMTP_PORT=587
API_SMTP_USERNAME=noreply@example.com
API_SMTP_PASSWORD=...           # generate; do not commit
API_SMTP_FROM=noreply@example.com
# API_SMTP_TLS_MODE=starttls    # inferred from port 587
```

### Example: implicit-TLS submission

```bash
API_SMTP_HOST=smtp.example.com
API_SMTP_PORT=465
API_SMTP_USERNAME=noreply@example.com
API_SMTP_PASSWORD=...
API_SMTP_FROM=noreply@example.com
# API_SMTP_TLS_MODE=implicit    # inferred from port 465
```

### Example: in-cluster dev relay (plaintext, no auth)

```bash
API_SMTP_HOST=mailhog
API_SMTP_PORT=1025
API_SMTP_FROM=dev@nebula.local
API_SMTP_TLS_MODE=none          # composition root warns at startup
```

### Security

- Password is held in `SecretString` everywhere: env parser → config
  struct → `lettre::Credentials::new` (the only `ExposeSecret` call).
- `EmailError::Transport(String)` and `EmailError::InvalidAddress(_)`
  never contain the SMTP password — lettre's `Error::Display` does
  not embed credentials (verified against `lettre-0.11.22`), and the
  mapping in `apps/server/src/email/smtp.rs` wraps lettre's error
  rather than propagating it raw.
- `SmtpEmailConfig::password` is `#[serde(skip)]`, so a
  `tracing::error!(?config)` line cannot leak the secret through
  `serde_json::to_string` on an `ApiConfig` snapshot.

### Out of scope (today)

- HTML template rendering — `EmailMessage::body` stays a raw token; a
  future PR introduces `templates/`.
- Mailpit/Mailhog in `deploy/docker/` — local dev stays on `EchoSink`
  unless the operator explicitly wires `API_SMTP_HOST`.
- Bounce / retry logic — handled at the auth-backend caller layer via
  idempotency, not here.
- Multipart / attachments — `text/plain` only.
