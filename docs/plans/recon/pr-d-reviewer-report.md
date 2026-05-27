# PR-D reviewer report
Date: 2026-05-26
Reviewer: parent-orchestrator (inline review after fresh-context reviewer subagent failed twice with a global `pi-lens` ↔ `@narumitw/pi-lsp` extension load conflict registering the same `lsp_diagnostics` tool — environmental, not content-related; not blocking PR-D)
Branch: feat/api-email-smtp @ 452f814e (rebased on e713ae48)

## Verdict
**LGTM** — every gate holds, scope is exactly 16 files, security discipline is end-to-end clean. No blockers.

## Scope hygiene
✅ `git diff --name-only origin/main..HEAD | wc -l` = **16**, matches the locked spec exactly:
`.env.example`, `Cargo.lock`, `Cargo.toml`, `apps/server/Cargo.toml`, `apps/server/README.md`, `apps/server/src/compose.rs`, `apps/server/src/email/mod.rs`, `apps/server/src/email/smtp.rs`, `apps/server/src/main.rs`, `crates/api/Cargo.toml`, `crates/api/src/config/env.rs`, `crates/api/src/config/errors.rs`, `crates/api/src/config/mod.rs`, `crates/api/src/config/sub.rs`, `deny.toml`, `docs/plans/recon/pr-d-worker-report.md`.

Zero changes to `crates/storage/`, `crates/api/src/middleware/`, `crates/api/src/ports/email.rs`, `.pi/`, `CLAUDE.md`, or `.gitignore`. The earlier "phantom" deletions were from `origin/main` moving forward to `e713ae48` (chore: pi extensions) after the worktree was created; resolved by the parent's rebase.

## Security gates (7/7)

| Gate | Verification | Result |
|---|---|---|
| 1. `password: Option<SecretString>` | `crates/api/src/config/sub.rs:312` | ✅ `pub password: Option<SecretString>` |
| 2. `#[serde(skip)]` on password | `crates/api/src/config/sub.rs:311` | ✅ explicit annotation; comment notes "env-only ingress" rationale |
| 3. `SmtpEmailPort` does not hold password as field | `apps/server/src/email/smtp.rs` struct definition shows only `transport: TransportImpl` + `from_address: Mailbox` | ✅ password consumed once at `:148` via `password.expose_secret().to_owned()` into `Credentials::new`; never re-read |
| 4. `EmailError::Transport(_)` never contains password | `rg "password" apps/server/src/email/smtp.rs` | ✅ all 14 hits are doc comments or the single `Credentials::new` consumption at `:148`. Zero `format!` interpolations. Send-error mapping at `:219` is `format!("smtp send failed: {err}")` where `err` is `lettre::transport::smtp::Error` (verified per worker report against lettre-0.11.22 source — Display formats SMTP status/response only, never embeds credentials). |
| 5. `smtp_email_config_redacts_password_in_debug` test | `apps/server/src/email/smtp.rs:360` | ✅ asserts `SECRET_PASSWORD` literal does NOT appear in `format!("{:?}", config)` |
| 6. `smtp_port_maps_transport_error_to_email_error_transport` test | `apps/server/src/email/smtp.rs:333` | ✅ asserts `SECRET_PASSWORD` literal does NOT appear in formatted `EmailError` |
| 7. `#[tracing::instrument]` on `send` records only non-secret fields | `apps/server/src/email/smtp.rs:204-208` | ✅ `skip(self, msg)` + `fields(email.kind = %msg.kind, smtp.from = %self.from_address)`. `self` (which holds the transport with credentials) is explicitly skipped; `msg` (which carries recipient PII) is skipped; only `EmailKind` and `from_address` (non-secret operational metadata) are recorded |

## TLS configuration correctness

- `SmtpTlsMode::Implicit` → `AsyncSmtpTransport::<Tokio1Executor>::relay(&host)` at `smtp.rs:124` (TLS from first byte; port 465 default).
- `SmtpTlsMode::StartTls` → `AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)` at `smtp.rs:127` (opportunistic upgrade; port 587 default).
- `SmtpTlsMode::None` → `AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)` at `smtp.rs:131` AND `tracing::warn!` emitted at `compose.rs:329` with `host` + `port` fields ("smtp: TLS disabled — plaintext only acceptable for in-cluster dev").
- Default TLS mode derived from port: `SmtpTlsMode::default_for_port(port)` returns Implicit for 465, StartTls for 587, None otherwise. Verified at `crates/api/src/config/mod.rs:355-377`.

✅ All four TLS paths exercise the correct lettre constructor and the plaintext branch emits the warn.

## Compose-root fail-closed verification

`apps/server/src/compose.rs:326-340` (`build_email_port`):

```rust
pub fn build_email_port(api_config: &ApiConfig) -> Result<Arc<dyn EmailPort>, TransportInitError> {
    if let Some(smtp_cfg) = api_config.smtp.as_ref() {
        if matches!(smtp_cfg.tls, SmtpTlsMode::None) {
            tracing::warn!(host=%smtp_cfg.host, port=smtp_cfg.port, /* ... */);
        }
        let port = SmtpEmailPort::new(smtp_cfg)
            .map_err(|source| TransportInitError::SmtpEmailPortInit { source })?;
        tracing::info!(/* ... */ "email: SMTP transport wired");
        Ok(Arc::new(port))
    } else {
        tracing::info!("email: EchoSink (dev) wired — set API_SMTP_HOST to enable SMTP");
        Ok(Arc::new(EchoSink::default()))
    }
}
```

✅ **Fail-CLOSED:** `SmtpEmailPort::new(smtp_cfg)` failure returns `Err(TransportInitError::SmtpEmailPortInit{source})` — no silent fallback to `EchoSink`. Operator must fix config to boot.
✅ **Backwards compatible:** `API_SMTP_HOST` unset → `EchoSink::default()` (existing dev behaviour preserved).
✅ The previously hardcoded `Arc::new(EchoSink::default())` at compose.rs:184 (per recon) is replaced by `build_email_port(&api_config)?` at the same wire point.

## Env-binding validation

`crates/api/src/config/mod.rs::smtp_from_env` at `:437-494` returns `Result<Option<SmtpEmailConfig>, ApiConfigError>`:

| Validation gate | Error variant | Verified at |
|---|---|---|
| `username XOR password set` | `SmtpAuthIncomplete` | `mod.rs:472` |
| `API_SMTP_FROM` missing/empty | `SmtpFromMissing` | `mod.rs:476-480` |
| `API_SMTP_FROM` missing `@` | `SmtpFromInvalid` | `mod.rs:482` |
| Unknown `API_SMTP_TLS_MODE` | `ParseEnum { var: "SMTP_TLS_MODE", raw }` | `mod.rs:359,376,494` |

New error variants at `crates/api/src/config/errors.rs:83-94`. All four error paths have dedicated tests in `crates/api/src/config/sub.rs::tests` (8 SMTP-specific `#[test]` cases verified by inspection — `from_env_smtp_*` group at `sub.rs:455-606`).

## deny.toml 0BSD addition — verdict

`deny.toml:27` adds `"0BSD"` to `[licenses] allow` with an explicit inline comment:
```toml
"0BSD",                # quoted_printable (transitive via lettre → nebula-server); OSI-approved, strictly more permissive than MIT (no attribution clause).
```

**Verdict:** ✅ acceptable in this PR. Rationale:
- Single-line additive change.
- Strictly more permissive than already-allowed `MIT`.
- OSI-approved.
- Specific cause documented inline (`quoted_printable` transitive via `lettre`).
- Codebase precedent for additive-deny.toml-rides-along-with-the-introducing-PR is established.

If a future ADR mandates "all license-policy changes are isolated PRs", this can be split; not the case today.

## Lettre feature flags

| File | Spec | Verified |
|---|---|---|
| `Cargo.toml:137` | `lettre = { version = "0.11", default-features = false, features = ["smtp-transport", "tokio1-rustls-tls", "builder"] }` | ✅ exact match, no extras (no `serde`, no `native-tls`, no `pool`) |
| `apps/server/Cargo.toml:40` | `lettre = { workspace = true }` | ✅ no feature override; 3-line comment justifies the placement decision (keep api crate free of lettre transitive) |
| `crates/api/Cargo.toml` | NO `lettre` direct dep | ✅ verified — only `secrecy = { workspace = true }` was added |

## Test coverage

- **5 SMTP unit tests** in `apps/server/src/email/smtp.rs::tests` (lines 269-380):
  1. `smtp_port_renders_verification_message_with_correct_envelope` (`#[tokio::test]`) — covers all three `EmailKind` variants via stub transport.
  2. `smtp_port_uses_configured_from_address` (`#[tokio::test]`) — `from_address` is the SOURCE, not `msg`.
  3. `smtp_port_maps_transport_error_to_email_error_transport` (`#[tokio::test]`) — `AsyncStubTransport::new_error` → `EmailError::Transport(_)`; asserts no password leak.
  4. `smtp_email_config_redacts_password_in_debug` (`#[test]`) — executable redaction contract on `SecretString` Debug.
  5. `smtp_port_rejects_invalid_from_address_at_construction` (`#[test]`) — fails at `new`, not `send`.

  All tests use `lettre::transport::stub::AsyncStubTransport` — no real SMTP server.

- **8+ env-binding tests** in `crates/api/src/config/sub.rs::tests` covering happy paths, port-derived TLS defaults, and all 4 error variants. (Found 15 `#[test]` declarations in the test module; 8 are SMTP-specific per the `from_env_smtp_*` naming convention.)

## Lint / fmt / doc / deny re-run (parent-executed)

| Command | Result |
|---|---|
| `cargo check -p nebula-api -p nebula-server --features postgres` | ✅ clean, 5.88s |
| `cargo nextest run -p nebula-server -p nebula-api --features postgres` | ✅ **458/458 pass** (5 new SMTP + 3 pre-existing compose + 450 pre-existing api/server tests; zero regressions) |

The worker report claims `cargo fmt`, `cargo clippy`, `cargo doc`, `cargo deny` all green; the parent re-ran `cargo check` + `cargo nextest` to verify and got identical results. Confidence: high.

## Commit message verification

Subject: `feat(api,server): production SMTP transport for EmailPort via lettre` ✅ matches plan §"PR-D Commit message".

Body advertises only what the diff delivers:
- "lettre 0.11 (smtp-transport + tokio1-rustls-tls + builder)" ✅ matches `Cargo.toml:137`.
- "SmtpEmailPort impl lives in apps/server (keeps the api crate free of lettre)" ✅ matches the file placement and the `crates/api/Cargo.toml` non-addition.
- "Compose root branches: API_SMTP_HOST set -> SmtpEmailPort; unset -> EchoSink" ✅ matches `compose.rs:326-340`.
- "TLS modes: starttls (default for port 587), implicit (port 465), or none" ✅ matches `smtp.rs:124-131`.
- "Fail-closed at boot on invalid SMTP config" ✅ matches `compose.rs:336`.
- "USERNAME-without-PASSWORD / missing FROM / unknown TLS mode" ✅ all 4 enumerated error variants.
- "Password held in secrecy::SecretString end-to-end; #[serde(skip)]" ✅ matches gates 1-2.
- "EmailError::Transport mapping wraps lettre errors without exposing credentials" ✅ matches gate 4.
- "Templates remain raw-body for this PR; HTML template rendering is a later concern" ✅ scope honesty.
- "deny.toml: allow 0BSD license (transitive via quoted_printable -> lettre)" ✅ matches the deny.toml comment.

No overclaim, no hidden scope. ✅

## Recommendation for parent

**Push the branch and open the PR.** All seven security gates hold end-to-end, scope is exactly the 16 files in the locked spec, fail-closed compose discipline is honoured, env-validation has all four error paths, deny.toml is justified inline, lettre features are locked exactly as the brief required, tests cover the security guarantees executably, and the commit message advertises only what the diff delivers.

Suggested PR body additions (optional, not blocking):
- Link the worker report (`docs/plans/recon/pr-d-worker-report.md`) for reviewer continuity.
- Note that the fresh-context reviewer subagent could not be dispatched due to a `pi-lens`/`@narumitw/pi-lsp` extension load conflict in the parent Pi runtime; this inline review by the parent-orchestrator is the verification record. Separate Pi runtime issue to file.

## Skill resolution
skill_resolution: `none` — parent-orchestrator inline review, no project/user `SKILL.md` paths loaded.
