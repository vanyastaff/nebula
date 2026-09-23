# nebula-server

Composition-root binary for the Nebula API. Wires the `nebula-api` HTTP
surface to one of three ingress transports (`api`, `webhook`,
`realtime`, or `all`) and instantiates the currently configured runtime ports
(storage adapters, idempotency store, identity backend, tenant directory,
email transport, metrics + telemetry exporters). The tenant directory uses
the same selected memory, SQLite, or PostgreSQL backend as execution storage.
Startup does not create an organization, workspace, or privileged member;
durable tenants are provisioned explicitly through the operator bootstrap path.

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

Credential composition admits the complete registry and exports its catalog before
starting runtime maintenance. Registration or schema-export failure propagates as
a typed bootstrap error; no partial catalog or permissive schema is served.

## Runtime-repair RED profile

The non-default `runtime-repair-red` feature opens the app-owned
**first-party RED conformance profile (evidence-only; non-deployment; non-SDK)**.
It is an internal evidence fixture in the existing `nebula-server` package,
not a supported deployment root or SDK/embedding surface. Ordinary server
launch remains environment-driven; the profile uses only its closed explicit
preset and never mutates process-global environment, installs telemetry, or
handles OS signals.

The profile pre-binds loopback port zero, composes API and the core-flavor
worker from views over the exact same in-memory core, SQLite pool, or
PostgreSQL pool, and supervises the HTTP and durable worker siblings under
one cancellation token. A third supervised, authority-free observer maps only
typed execution/node lifecycle facts into a state-carrying registry. Scenario
code can await `NodeStarted`, durable `NodeParked`, `NodeWaitCompleted`, and
terminal success/failure with a bounded timeout and no polling or sleeps. The
registry has a fixed distinct-fact ceiling; a new fact beyond it fails the
observer and cancels the supervised profile instead of growing without bound.
The same `ManualClock` exposed through evidence controls is injected into the
sealed workflow engine. Its opaque lifecycle still exposes only address,
readiness, shutdown, and join; observation waits remain on the retained
harness controls. Current product behavior is deliberately preserved: REST
start writes the `ControlQueue`, while the worker claims exact-flavor commands from it. The
profile does not bridge that gap or invent a failing sentinel. Genuine
first-party durable-wait scenarios therefore reach a bounded lifecycle observation and
fail with `durable-wait-control-path-disconnected`; keyed acceptance independently exposes duplicate
same-fingerprint starts, while cancellation reachability exposes immediate API
terminalization. The same-processor claim-generation fixture is deliberately labeled
component/storage evidence rather than product-root proof.

File-SQLite configuration retains one opaque path across repeated
`harness.launch()` calls. A recovery scenario must shut down and join the first
handle before launching the same retained harness again. Join drains every
supervised component and explicitly closes the actual shared SQLite pool (or
selected PostgreSQL pool) before it completes. The real integrity test performs
launch → shutdown/join → relaunch on one retained file-SQLite harness. The
standalone marker method remains only a low-level file/pool persistence check;
it does not claim worker crash recovery. InMemory is intentionally reconstructed
per launch and is not a restart-durability lane. Selecting PostgreSQL first
requires a live connection/version probe and fails instead of skipping or
substituting another backend.

Evidence artifacts accept only a closed enum of structured references, counts,
and fixed-width digests. Secret-bearing and raw-business-payload
classifications are rejected before an artifact entry can be constructed; no
caller-provided text or arbitrary payload is retained or printed by `Debug`.

The worker runtime currently spawns its durable timer scanner internally and
drops that nested `JoinHandle`. Profile cancellation and pool close stop its
work, but a scanner-only panic is not yet join-visible to the app supervisor.
HTTP, the worker pull loop, and lifecycle observer are owned and joined. This
residual must be closed in the worker runtime before the profile can claim
complete nested structured-concurrency evidence.

Run the passing infrastructure-integrity slice independently:

```bash
cargo nextest run -p nebula-server --features runtime-repair-red \
  -E 'test(/integrity_tests/)'
```

## Tenant membership and credential authority

The default server wires organization lookup, workspace lookup, and membership
authorization to one apps-owned adapter over the selected storage backend. The
SQLite and PostgreSQL projections reuse the execution database pool; the memory
profile uses one shared `InMemoryIdentityDirectory`. Tenant changes are visible
to RBAC and the credential authority through the same durable source.

Composition never creates an implicit owner or tenant. A fresh database has an
empty directory until the operator bootstrap path provisions stable organization,
workspace, and owner IDs for an already-authenticatable user. Missing membership
denies access, while storage and malformed-data failures remain redacted as 503.

Bootstrap is opt-in and runs before the HTTP listener starts. Set all eight
variables or none of them:

| Variable | Meaning |
|----------|---------|
| `NEBULA_BOOTSTRAP_ORG_ID` | Stable `org_<ULID>` identifier. |
| `NEBULA_BOOTSTRAP_ORG_SLUG` | Valid organization slug. |
| `NEBULA_BOOTSTRAP_ORG_NAME` | Organization display name. |
| `NEBULA_BOOTSTRAP_ORG_PLAN` | Initial plan identifier. |
| `NEBULA_BOOTSTRAP_WORKSPACE_ID` | Stable `ws_<ULID>` identifier. |
| `NEBULA_BOOTSTRAP_WORKSPACE_SLUG` | Valid workspace slug. |
| `NEBULA_BOOTSTRAP_WORKSPACE_NAME` | Default workspace display name. |
| `NEBULA_BOOTSTRAP_OWNER_USER_ID` | Existing `usr_<ULID>` identity with verified email. |

Bootstrap requires `API_AUTH_BACKEND=postgres`. The process-local memory
backend starts empty and cannot contain a pre-existing verified owner before
the listener starts, so enabling bootstrap with it fails during startup.

Create and verify the owner in the selected `API_AUTH_BACKEND` first. Startup
then writes the organization, default workspace, and `OrgOwner` membership in
one storage transaction. Restarting with exactly the same values is a safe
replay. Partial configuration, changed values, pre-existing partial state, an
unknown owner, or an unverified owner aborts startup. A replay never restores a
membership that an operator removed or downgraded.

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
   `task db:migrate`; the server-owned operator first attempts catalog-only
   admission, then falls back only from a typed configuration rejection to
   credential-owner deep admission. It closes the general pool before that
   fallback, and every failure is closed and redacted. All schema changes come
   from immutable numbered migrations.
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

### Credential master-key rotation runbook

`NEBULA_CRED_MASTER_KEY` is the current base64 AES-256 key. Every new or
updated identity secret and credential payload is encrypted with this key.
`NEBULA_CRED_LEGACY_MASTER_KEYS` is an optional comma-separated list of at
most eight base64 AES-256 keys. Legacy entries are decrypt-only: they can open
matching historical envelopes, but every replacement or ordinary write uses
`NEBULA_CRED_MASTER_KEY`. Startup rejects malformed, duplicate, current-key,
or over-limit entries instead of silently dropping them.

Credentials written by the historical pre-guard envelope format can carry an
empty key ID. Set `NEBULA_CRED_LEGACY_EMPTY_ID_MASTER_KEY` to the one base64
AES-256 key that produced those rows. This explicit decrypt-only alias applies
only to credential envelopes; Plane-A identity envelopes always require a
non-empty key ID. Remove the alias after those credential rows and retained
recovery media have converged.

Use a two-stage rolling deployment so old and new replicas can read each
other's writes during the cutover:

1. Generate the new key and retain the old key. First roll out the old key as
   `NEBULA_CRED_MASTER_KEY` while adding the new key to
   `NEBULA_CRED_LEGACY_MASTER_KEYS`, together with any older keys still needed.
   Complete this bridge rollout on every replica before changing the writer
   key.
2. Roll out the new key as `NEBULA_CRED_MASTER_KEY` and move the old key into
   `NEBULA_CRED_LEGACY_MASTER_KEYS`. During this mixed-key window, every
   replica can decrypt both generations while each replica writes only with
   its configured current key. The server and first-party worker must receive
   the same current and legacy configuration.
3. Drain every bridge replica that still writes with the old current key, then
   run one final new-key server startup convergence before verification. This
   ordering prevents an old Plane-A writer from reintroducing an old-key MFA
   envelope after another replica's startup migrator has passed it. Migrate
   and verify all live rows under the new key. Plane-A startup
   convergence re-encrypts admitted identity envelopes. Plane-B credential
   reads are side-effect free; a supported credential mutation writes the
   replacement with the current key. There is no automatic credential
   re-encryption scanner, so operators must plan and verify this convergence.
4. Test restores of every retained backup, snapshot, replica, and WAL recovery
   set that can contain old-key envelopes. Keep the old key available to those
   restore environments until their rows are migrated, or until the media is
   expired or quarantined under the retention policy.
5. Remove the old key from `NEBULA_CRED_LEGACY_MASTER_KEYS` only after no live
   row and no retained recovery material requires it, then complete that
   configuration rollout on every replica. Never delete old key material
   earlier: removing it makes any remaining envelope permanently unreadable.

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
tenant membership authority is provisioned. The default binary resolves that
authority through the selected execution storage backend described above.

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
credential lifecycle runtime until `serve` exits. It immediately reclaims stale
claims at startup and scans backend-clock expiry pages for refresh work using
bounded pagination and concurrency. Every due candidate is rechecked against
current state and passes through the existing durable cross-replica claim before
provider egress. Expired pre-provider claims may be reclaimed; expired
`RefreshInFlight` claims remain durable
`OutcomeUnknown` poison, are accounted exactly once, and never become
replayable merely because TTL elapsed. There is no in-memory claim fallback in
the production composition. The lifecycle runtime binds scheduler and recovery
counters to the server's shared metrics registry. Their labels are closed
outcome classes only: no tenant, credential, provider, or replica identifiers
enter the metrics cardinality boundary.

Interactive credential pending state uses that same admitted credential pool
and the same current/decrypt-only keyring as credential material. SQLite flows
therefore survive a server restart, while PostgreSQL flows can continue on any
replica connected to the credential database. Pending bearer tokens are stored
only as digests; the state envelope is encrypted and bound to credential kind,
owner, session, and expiry. A binding failure leaves the row available for the
matching callback, while a successful callback consumes it atomically.

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
