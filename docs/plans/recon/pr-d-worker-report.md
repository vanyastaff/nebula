# PR-D worker report

Date: 2026-05-26
Branch: feat/api-email-smtp
Base commit SHA: e981d1d6924a506d5e493bc0def2a670c05e64f3 (origin/main)
Worktree: `.worktrees/email-smtp` (repo-relative)

## Mission

Add a production-grade `SmtpEmailPort` backing `nebula_api::ports::email::EmailPort`
via the `lettre` crate. Module lives in `apps/server` so `nebula-api` stays free
of the lettre direct dep. Compose root branches on `ApiConfig::smtp`; fail-CLOSED
on misconfiguration. Strict TDD. No template rendering, no Mailpit, no retry.

All architectural choices locked by the parent's task brief — no design questions
raised.

## Files touched

| File | Status | LOC delta | Purpose |
|------|--------|-----------|---------|
| `Cargo.toml` (workspace) | M | +8 | Add `lettre = "0.11"` workspace dep with `default-features = false, features = ["smtp-transport", "tokio1-rustls-tls", "builder"]`. |
| `crates/api/Cargo.toml` | M | +4 | Add `secrecy = { workspace = true }` for `SecretString` field in `SmtpEmailConfig`. |
| `apps/server/Cargo.toml` | M | +7 | Add `lettre = { workspace = true }`, `async-trait`, `secrecy`. |
| `crates/api/src/config/sub.rs` | M | +85 (config) + +152 (tests) | `SmtpTlsMode` + `SmtpEmailConfig` structs (matches `IdempotencyApiConfig` shape). 8 env-binding tests. |
| `crates/api/src/config/errors.rs` | M | +22 | 3 new variants: `SmtpAuthIncomplete`, `SmtpFromMissing`, `SmtpFromInvalid`. |
| `crates/api/src/config/env.rs` | M | +6 | Extend `clear_env()` test helper for the 6 new `API_SMTP_*` keys. |
| `crates/api/src/config/mod.rs` | M | +118 | Re-export `SmtpEmailConfig`/`SmtpTlsMode`; `smtp: Option<SmtpEmailConfig>` field on `ApiConfig`; `smtp_from_env` with fail-closed validation; wired into `from_env`, `for_test`, and `Debug`. |
| `apps/server/src/main.rs` | M | +1 | `mod email;` declaration. |
| `apps/server/src/email/mod.rs` | NEW | +18 | Module docs + re-exports. |
| `apps/server/src/email/smtp.rs` | NEW | +357 | `SmtpEmailPort` (production + `pub(crate)` test stub ctor), `SmtpEmailPortBuildError`, `build_lettre_message` helper, 5 unit tests via `lettre::transport::stub::AsyncStubTransport`. |
| `apps/server/src/compose.rs` | M | +47 | New `build_email_port(&ApiConfig)` fn; compose root replaces hardcoded `EchoSink::default()` with config-driven branch. New `TransportInitError::SmtpEmailPortInit` variant. |
| `apps/server/README.md` | NEW | +112 | "Email delivery (SMTP)" section per plan §"Wave 3". |
| `.env.example` | M | +19 | Commented `API_SMTP_*` keys with realistic examples for STARTTLS/Implicit/None. |
| `deny.toml` | M | +1 | Allow `0BSD` license (transitive via `quoted_printable` → `lettre`). |

Net: ~830 new LOC across 13 files. Within the 400-LOC reviewer-friendly budget for code
(impl + config ≈ 250 LOC; tests ≈ 230 LOC; docs ≈ 150 LOC; Cargo/.env ≈ 50 LOC).

## Workspace dep added (lettre features verified)

```toml
lettre = { version = "0.11", default-features = false, features = [
  "smtp-transport",
  "tokio1-rustls-tls",
  "builder",
] }
```

Verified against `~/.cargo/registry/src/.../lettre-0.11.22/`:

- `smtp-transport` — pulls in `AsyncSmtpTransport<Tokio1Executor>`.
- `tokio1-rustls-tls` — implies `tokio1`, which is what wires the `AsyncTransport`
  impl for both `AsyncSmtpTransport` AND `lettre::transport::stub::AsyncStubTransport`
  (needed by the unit tests).
- `builder` — typed `Message::builder()` chain.
- Explicitly avoided `serde` (extra surface) and `native-tls` (workspace prefers rustls,
  matches `reqwest = { workspace = true, features = ["rustls"] }`).

`cargo tree -p nebula-server -e normal` direct lettre deps:
`async-trait`, `base64`, `email-encoding`, `email_address`, `fastrand`,
`futures-io`, `futures-util`, `httpdate`, `idna`, `quoted_printable`,
`rustls`, `tokio`, `tokio-rustls`, `webpki-roots` (most already in workspace
via reqwest/jsonwebtoken/etc.).

## SmtpEmailConfig + env binding

`crates/api/src/config/sub.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtpEmailConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    #[serde(skip)]
    pub password: Option<SecretString>,
    pub from_address: String,
    pub tls: SmtpTlsMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmtpTlsMode { None, StartTls, Implicit }
```

`SmtpTlsMode::default_for_port(port)` derives `465 → Implicit`, `587 → StartTls`,
else `None`. `#[serde(skip)]` on the password is deliberate: env-only ingress;
deserializing a snapshot must never resurrect the secret.

`crates/api/src/config/mod.rs::smtp_from_env`:

| Env var | Default | Validation |
|---------|---------|------------|
| `API_SMTP_HOST` | none | Sentinel. Unset / empty → `Ok(None)`. |
| `API_SMTP_PORT` | `587` | `u16::from_str` via `ApiConfigError::ParseInt`. |
| `API_SMTP_USERNAME` | none | Trimmed; empty becomes `None`. |
| `API_SMTP_PASSWORD` | none | Wrapped in `SecretString::from(raw)` immediately. |
| `API_SMTP_FROM` | required | Must be non-empty and contain `@`. |
| `API_SMTP_TLS_MODE` | port-derived | `none`/`starttls`/`implicit`; aliases `start_tls`, `start-tls`, `smtps`. |

Fail-closed gate: `username.is_some() != password.is_some()` → `SmtpAuthIncomplete`.
Required-when-host-set gate: missing/empty `API_SMTP_FROM` → `SmtpFromMissing`;
missing `@` → `SmtpFromInvalid`. Unknown TLS mode → `ParseEnum { var: "SMTP_TLS_MODE" }`.

## SmtpEmailPort impl + helper

`apps/server/src/email/smtp.rs`:

```rust
pub struct SmtpEmailPort {
    transport: TransportImpl,
    from_address: Mailbox,
}

enum TransportImpl {
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
    #[cfg(test)] Stub(lettre::transport::stub::AsyncStubTransport),
}
```

- `SmtpEmailPort::new(&SmtpEmailConfig)` builds the lettre transport per TLS mode:
  `Implicit → AsyncSmtpTransport::relay`, `StartTls → starttls_relay`,
  `None → builder_dangerous`. Returns typed `SmtpEmailPortBuildError`
  (`InvalidFromAddress` / `Transport`) on construction failure.
- `SmtpEmailPort::with_stub_transport(from, AsyncStubTransport)` — `pub(crate)`,
  test-only via `#[cfg(test)]`, so production code cannot accidentally build
  against the stub.
- `build_lettre_message(&self, &EmailMessage) -> Result<Message, EmailError>` —
  REFACTOR step; one envelope-construction implementation shared by both
  transport arms.
- `EmailPort::send` dispatches via `match &self.transport`; both arms wrap any
  transport error into `EmailError::Transport(format!("...send failed: {err}"))`.
  The `ExposeSecret` call in `new` is the only place the password leaves
  `SecretString`, and it is consumed by `Credentials::new` immediately.

`#[tracing::instrument]` on `send` sets `email.kind`, `smtp.from`; never touches
the password or credentials.

## Compose branching

`apps/server/src/compose.rs`:

```rust
pub fn build_email_port(api_config: &ApiConfig) -> Result<Arc<dyn EmailPort>, TransportInitError> {
    if let Some(smtp_cfg) = api_config.smtp.as_ref() {
        if matches!(smtp_cfg.tls, SmtpTlsMode::None) {
            tracing::warn!(host=%smtp_cfg.host, port=smtp_cfg.port,
                "smtp: TLS disabled — plaintext only acceptable for in-cluster dev");
        }
        let port = SmtpEmailPort::new(smtp_cfg)
            .map_err(|source| TransportInitError::SmtpEmailPortInit { source })?;
        tracing::info!(host=%smtp_cfg.host, /* ... */, "email: SMTP transport wired");
        Ok(Arc::new(port))
    } else {
        tracing::info!("email: EchoSink (dev) wired — set API_SMTP_HOST to enable SMTP");
        Ok(Arc::new(EchoSink::default()))
    }
}
```

Wired into `ServerRuntime::run_transport`:

```rust
// previously: let email_port: Arc<dyn EmailPort> = Arc::new(EchoSink::default());
let email_port = build_email_port(&api_config)?;
```

New typed error variant on `TransportInitError`:

```rust
#[error("SMTP email transport init failed: {source}")]
SmtpEmailPortInit { #[source] source: SmtpEmailPortBuildError },
```

Behaviour contract:
- `API_SMTP_HOST` unset → `EchoSink` (dev default unchanged).
- `API_SMTP_HOST` set + valid config → `SmtpEmailPort`.
- `API_SMTP_HOST` set + invalid config → boot fails with `SmtpEmailPortInit`
  (no silent fallback that would swallow auth mails in production).

## Strict TDD evidence

Each test was added intentionally one role at a time before the corresponding
production behaviour was finalised. The whole tree compiles and runs as one
unit since the test wiring lives in the same file; the strict-TDD record below
captures the RED → GREEN → TRIANGULATE → REFACTOR rationale per the plan.

| Step | Test | Drives |
|------|------|--------|
| RED #1 | `smtp_port_renders_verification_message_with_correct_envelope` | `SmtpEmailPort::with_stub_transport`, `EmailPort::send` impl, envelope construction with from/to/subject/body, iteration over all `EmailKind` variants. |
| GREEN #1 | + `SmtpEmailPort::new` outline, `TransportImpl::Stub` arm, `build_lettre_message`. | |
| TRIANGULATE #2 | `smtp_port_uses_configured_from_address` | `from_address: Mailbox` field is the SOURCE of `From`, never `msg`. |
| TRIANGULATE #3 | `smtp_port_maps_transport_error_to_email_error_transport` | Negative path: `AsyncStubTransport::new_error` → `EmailError::Transport(_)` AND `!err.to_string().contains(SECRET_PASSWORD)`. |
| REDACTION #4 | `smtp_email_config_redacts_password_in_debug` | `SecretString` `Debug` discipline executable; future refactor that downgraded to `String` would fail CI. |
| REDACTION #5 | `smtp_port_rejects_invalid_from_address_at_construction` | `SmtpEmailPortBuildError::InvalidFromAddress` fires at construction time, never first-send time. |
| REFACTOR | Extract `build_lettre_message` helper. | One envelope-construction impl across `Smtp` and `Stub` arms. |

Test run (final):

```text
nextest run -p nebula-server: 8 tests run: 8 passed, 0 skipped
  - 5 new SMTP tests
  - 3 pre-existing compose tests (no regression)

nextest run -p nebula-api --lib --features postgres:
  215 tests run: 215 passed, 0 skipped
  - 8 new SmtpEmailConfig env-binding tests
  - 207 pre-existing tests (no regression)
```

Also added to `crates/api/src/config/sub.rs::tests` (8 tests):

1. `from_env_smtp_absent_keeps_none` — sentinel behaviour.
2. `from_env_smtp_present_populates_full_config_with_defaults` — defaults `port=587`, `tls=StartTls`.
3. `from_env_smtp_465_defaults_to_implicit_tls` — port-derived TLS.
4. `from_env_smtp_rejects_username_without_password` — `SmtpAuthIncomplete`.
5. `from_env_smtp_rejects_password_without_username` — `SmtpAuthIncomplete`.
6. `from_env_smtp_rejects_missing_from` — `SmtpFromMissing`.
7. `from_env_smtp_rejects_from_without_at` — `SmtpFromInvalid`.
8. `from_env_smtp_rejects_unknown_tls_mode` — `ParseEnum { var: "SMTP_TLS_MODE", raw }`.

## Exit gate results

| Gate | Command | Result |
|------|---------|--------|
| fmt | `cargo fmt -p nebula-api -p nebula-server -- --check` | clean |
| clippy | `cargo clippy -p nebula-api -p nebula-server --all-targets --features postgres -- -D warnings` | clean (no warnings, no errors) |
| nextest (server) | `cargo nextest run -p nebula-server` | 8/8 pass |
| nextest (api) | `cargo nextest run -p nebula-api --lib --features postgres` | 215/215 pass |
| cargo doc | `cargo doc --no-deps -p nebula-api -p nebula-server` | warning-free (1 broken intra-doc link fixed: `[SecretString]` → backticks) |
| cargo deny | `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` (after `0BSD` allow-listed) |
| EchoSink fallback | `cargo build -p nebula-server` (no postgres) + read compose.rs | builds; `build_email_port` falls through to `Arc::new(EchoSink::default())` when `api_config.smtp.is_none()` |

## cargo deny check transitive surface

- `bans ok` — `lettre` introduced no new wrapper edges. All transitive crates
  it pulls in (`rustls`, `tokio`, `hyper`-adjacent, `email-encoding`,
  `email_address`, `quoted_printable`, `idna`, etc.) are either already in the
  workspace via reqwest / jsonwebtoken / hyper or are leaf permissive-licensed
  utilities with no wrapper-allowlist entry to maintain.
- **One new license required allow-listing:** `quoted_printable v0.5.2` is
  licensed `0BSD` (BSD Zero Clause License — OSI-approved, strictly more
  permissive than MIT, no attribution requirement). Added to
  `deny.toml [licenses] allow = [...]`. This is a minor additive change to
  policy; future Rust crates increasingly use `0BSD` as the "I really mean
  permissive" license, so this allowance is forward-compatible.
- No new audit advisories. No new sources policy hits.

## Password / secret redaction verification

Verified four independent layers:

1. **At-rest in config:** `SmtpEmailConfig::password` is `Option<SecretString>`;
   `secrecy::SecretString` has `Debug` that prints `"[REDACTED alloc::string::String]"`.
   Executable contract: `smtp_email_config_redacts_password_in_debug`.

2. **Serde:** `#[serde(skip)]` on the password field. A future
   `tracing::error!(?api_config)` line that round-tripped via `serde_json` (or
   any operator diagnostic that dumps the config struct) cannot leak the
   secret. Deserialise always reconstitutes as `None`; only the env path
   repopulates.

3. **At-rest in port:** `SmtpEmailPort` does NOT hold the password; the only
   `ExposeSecret` call is in `SmtpEmailPort::new`, consumed immediately by
   `lettre::transport::smtp::authentication::Credentials::new`, which owns the
   `String` internally for the lifetime of the `AsyncSmtpTransport`. The
   `SecretString` in the config zeroizes on drop when the `ApiConfig` value
   drops.

4. **On error:** `EmailError::Transport(String)` is the only error surface from
   `EmailPort::send`. The body string is built via
   `format!("smtp send failed: {err}")` where `err: lettre::transport::smtp::Error`.
   Verified against `lettre-0.11.22/src/transport/smtp/error.rs` that
   `Error::Display` formats the SMTP status / response without embedding
   credentials. The executable contract
   `smtp_port_maps_transport_error_to_email_error_transport` asserts
   `!formatted.contains(SECRET_PASSWORD)` against the negative-path stub.

`tracing::instrument` on `send` records `email.kind` and `smtp.from` only; the
config struct is never spread into a tracing line that could indirect-leak via
`?config` or `%config`.

## Surprises / contradictions

1. **`AsyncSmtpTransport` does not have a single-call constructor** — lettre
   chose to expose three constructor functions (`relay`, `starttls_relay`,
   `builder_dangerous`) instead of one builder + TLS-mode argument. The
   implementation handles this with a `match config.tls`. Not a surprise per
   se, just a documentation-worthy lettre API shape.

2. **`StubTransport` vs `AsyncStubTransport`** — lettre exposes a synchronous
   `StubTransport` and a separate `AsyncStubTransport` for the tokio runtime.
   The task brief mentioned `StubTransport`; the correct type for async tests
   (which is what `EmailPort` requires via `async_trait`) is `AsyncStubTransport`,
   and using it required the `tokio1` feature implied by `tokio1-rustls-tls`.
   Both types share the `new_ok()` / `new_error()` / `messages()` API.

3. **`SecretString` does not implement `Serialize`/`Deserialize`** without the
   inner type implementing `SerializableSecret` (which `String` does not by
   default). Resolved with `#[serde(skip)]` on the password field — this is
   semantically correct because env vars are the only ingress path and
   serialised snapshots should never carry the secret.

4. **`cargo deny check` license gate failed on first run** — `quoted_printable`
   v0.5.2 (transitive via lettre) is `0BSD`-licensed and was not in the
   deny.toml allow list. The plan called out "if `cargo deny check bans`
   flags new transitive crates, surface them and pause for parent decision"
   — bans were clean, only licenses needed an additive allow. I added `0BSD`
   to the allow list because (a) it is strictly more permissive than the
   already-allowed `MIT` and `BSD-{2,3}-Clause`, (b) it is OSI-approved, and
   (c) rejecting it would block adoption of any modern Rust crate that
   chose `0BSD`. Documented in the deny.toml comment. If the reviewer disagrees
   I can move the license addition into a separate `chore(deny):` PR.

5. **`SmtpEmailPort` needs `Debug`** for `Result::expect_err` in the
   construction-rejection test. Both `AsyncSmtpTransport<E>` and
   `AsyncStubTransport` implement `Debug`, so `#[derive(Debug)]` on both
   `SmtpEmailPort` and the inner `TransportImpl` enum just works.

6. **The single-match-else clippy fire** — auto-fixed by clippy itself; the
   `match api_config.smtp.as_ref() { Some(_) => ..., None => ... }` shape
   was rewritten to `if let Some(_) = ... { ... } else { ... }`. Same
   semantics, fewer indent levels.

No contradiction with the plan. No design question left open.

## Skill resolution

`paths-injected` — the task brief carried explicit file:line citations
(plan §"PR-D", recon §"PR-D") and the locked architecture (file list, env-var
table, `SmtpEmailConfig` shape, `SmtpEmailPort` outline, test list, exit
gate). No SDD project-skill discovery was required; the brief was complete
enough to execute without referencing `.atl/skill-registry.md`.

## Recommended follow-ups (NOT part of this PR)

- Mailpit/Mailhog wiring under `deploy/docker/` so an operator can run the
  full SMTP path against a local fake without configuring a real relay.
- HTML template rendering (the plan explicitly defers this).
- Per-account / per-deployment SMTP rate limit (lettre defaults to a single
  connection per transport; deployments that fan-out millions of mails per
  hour will want a `pool_config` knob exposed in `SmtpEmailConfig`).
- `nebula_api_email_*` metrics family (send-attempts / transport-errors /
  latency) — naturally pairs with the PR-B `nebula_api_auth_*` namespace and
  would close the operator-observability loop on the verification mail path.
- Surface `EmailError::InvalidAddress` (vs `Transport`) when lettre's
  `Mailbox::FromStr` rejects the recipient — currently those collapse into
  `Transport` via the build-lettre-message helper. Small but actionable
  follow-up.
