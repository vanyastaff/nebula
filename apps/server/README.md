# nebula-server

Composition-root binary for the Nebula API. Wires the `nebula-api` HTTP
surface to one of three ingress transports (`api`, `webhook`,
`realtime`, or `all`) and instantiates every port the runtime needs
(storage adapters, idempotency store, identity backend, email transport,
metrics + telemetry exporters).

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
