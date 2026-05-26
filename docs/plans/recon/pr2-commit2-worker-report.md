# PR2 commit 2 (EmailPort + AuthBackendKind) — Worker Report

**Branch:** `feat/api-pg-auth-backend`
**Commit added:** `c0888808` — `feat(api): add EmailPort trait + AuthBackendKind config selector`
**Parent commit:** `ed39fcc7` — `feat(storage): add VerificationTokenRepo + OAuthStateRepo traits + 5 PG identity repos`

## Summary

Adds the API-owned `EmailPort` trait (+ dev `EchoSink` impl) and the
`AuthBackendKind` / `AuthApiConfig` selector with `API_AUTH_BACKEND`
env binding. Refactors `InMemoryAuthBackend` to send verification /
password-reset emails through `Arc<dyn EmailPort>` while preserving
the legacy `EmailEnvelope` + `InMemoryAuthBackend::emails()` surface
as a `#[deprecated]` back-compat shim. No `PgAuthBackend` and no
`crates/storage/` touches in this commit — those land in commit 3.
All existing tests pass against the default `EchoSink` port; the new
backend + email plumbing adds 9 new tests (5 config + 4 echo sink).

## Files changed (`git diff --stat HEAD~1..HEAD`)

```
 crates/api/src/config/env.rs                    |   1 +
 crates/api/src/config/mod.rs                    |  35 +++-
 crates/api/src/config/sub.rs                    | 123 +++++++++++++
 crates/api/src/domain/auth/backend/error.rs     |  14 +-
 crates/api/src/domain/auth/backend/in_memory.rs | 142 ++++++++++++---
 crates/api/src/ports/email.rs                   | 224 ++++++++++++++++++++++++
 crates/api/src/ports/mod.rs                     |   1 +
 crates/api/src/state.rs                         |  22 +++
 8 files changed, 539 insertions(+), 23 deletions(-)
```

Highlights:

- `crates/api/src/ports/email.rs` (new, 224 LOC) — `EmailPort` trait,
  `EmailMessage`, `EmailKind`, `EmailError`, `EchoSink` impl + 4
  inline tests. `#[tracing::instrument(level = "debug", …)]` on
  `send` / `peek` / `drain` per the CLAUDE.md observability triple.
- `crates/api/src/ports/mod.rs` — `pub mod email;`.
- `crates/api/src/state.rs` — `pub email_port: Option<Arc<dyn EmailPort>>`
  field + `with_email_port` builder (mirrors the existing
  `with_auth_backend` / `with_credential_service` pattern).
- `crates/api/src/config/sub.rs` — `AuthBackendKind { Memory, Postgres }`
  enum (default = `Memory`), `AuthApiConfig { backend }` struct,
  + 5 tests covering default / accepts-postgres / case-insensitive /
  rejects-unknown / `for_test`-defaults-to-memory.
- `crates/api/src/config/mod.rs` — `auth` field on `ApiConfig`,
  `auth_from_env()` parser (mirrors `idempotency_from_env`),
  re-export, Debug impl, `for_test` wiring.
- `crates/api/src/config/env.rs` — `API_AUTH_BACKEND` added to
  `clear_env()`.
- `crates/api/src/domain/auth/backend/error.rs` — `impl From<EmailError>
  for AuthError` collapsing transport faults into
  `AuthError::Internal`.
- `crates/api/src/domain/auth/backend/in_memory.rs` — refactor: field
  replaced with `email_port: Arc<dyn EmailPort>` + side handle
  `default_echo: Option<Arc<EchoSink>>` so the legacy `emails()`
  snapshot keeps working without a dyn downcast. Added
  `with_email_port` builder. `record_email` is now async and goes
  through the port; on signup it propagates failures, on
  `request_password_reset` it logs and swallows for enumeration
  safety (preserves the existing contract). `EmailEnvelope` marked
  `#[deprecated(...)]`. Internal test `password_reset_round_trips`
  updated to call `default_echo.drain()` instead of the removed
  `email_sink` field.

## Verification

All commands executed from
`C:/Users/vanya/RustroverProjects/nebula/.worktrees/pg-auth-backend`:

- `cargo fmt -p nebula-api -- --check`: **pass** (exit 0).
- `cargo clippy -p nebula-api --all-targets -- -D warnings`: **pass**
  (clean build, no warnings).
- `cargo nextest run -p nebula-api`: **427 passed, 1 skipped, 0
  failed** in 32.82s. New tests confirmed in run:
  - `config::sub::tests::from_env_auth_backend_defaults_to_memory`
  - `config::sub::tests::from_env_auth_backend_accepts_postgres`
  - `config::sub::tests::from_env_auth_backend_is_case_insensitive`
  - `config::sub::tests::from_env_auth_backend_rejects_unknown`
  - `config::sub::tests::for_test_auth_backend_defaults_to_memory`
  - `ports::email::tests::echo_sink_buffers_sent_message`
  - `ports::email::tests::echo_sink_drain_clears_inbox`
  - `ports::email::tests::echo_sink_rejects_invalid_address`
  - `ports::email::tests::email_kind_as_str_matches_legacy_labels`

  All 12 existing `in_memory::tests` pass (the refactor preserves
  backward compatibility through the `EchoSink` default + `emails()`
  shim). The 1 skipped test is pre-existing (unrelated to this
  commit). 427 = PR1's 418 + the 9 new tests above.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-api --no-deps`:
  **pass**.

## Deviations from plan

1. **Public method name.** The task description references
   `email_envelopes()`; the actual existing method on
   `InMemoryAuthBackend` is `emails()`. Kept the real name and
   preserved its `Vec<EmailEnvelope>` return shape via a small
   `From<EmailMessage> for EmailEnvelope` shim. Every existing test
   that called `emails()` keeps working unchanged.
2. **`InMemoryAuthBackend` field name.** Renamed
   `email_sink: Arc<RwLock<Vec<EmailEnvelope>>>` to
   `email_port: Arc<dyn EmailPort>` + added a side handle
   `default_echo: Option<Arc<EchoSink>>`. The side handle is needed
   because `Arc<dyn EmailPort>` is not safely downcastable to
   `Arc<EchoSink>` without an `Any` bound on the trait, and adding
   `Any` would clutter the public seam. Single internal test that
   reached into `email_sink.write().clear()` was retargeted to
   `default_echo.as_ref().expect(...).drain()`; the choice is
   documented inline so PR2 commit 3 (which won't touch this) knows
   why the shape exists.
3. **`EmailMessage.body` carries the raw token in the dev path.**
   The task spec defines `EmailMessage { to, subject, body, kind }`
   but the legacy `EmailEnvelope` exposed a `token` field. To keep
   the shim 1:1, the dev `EchoSink` convention is `body == token`;
   production `EmailPort` impls will replace this with a rendered
   template (and the `EmailEnvelope` legacy shim becomes irrelevant
   once tests migrate to `EchoSink::peek` directly). Documented
   inline on `EmailEnvelope::from(EmailMessage)`.
4. **`From<EmailError> for AuthError` lives in `auth/backend/error.rs`,
   not inline at the call site.** The task says "translate
   `EmailError → AuthError::Internal(...)` for now"; adding the `From`
   impl keeps `?`-propagation idiomatic and PR2 commit 3
   (`PgAuthBackend`) gets to reuse the same conversion for free
   without re-deriving the mapping at every call site. Not a behaviour
   change — every `?` collapses to the same `AuthError::Internal`
   string the inline map would produce.
5. **`#[tracing::instrument]` on `EchoSink::peek` / `drain` as well as
   `send`.** The task only requires it on `send`, but adding the
   same level on `peek` / `drain` costs nothing and keeps the
   observability surface uniform for the dev sink. Removed if a
   reviewer prefers minimal instrumentation.
6. **`AuthBackendKind` uses `#[derive(Default)]` with `#[default]
   Memory`** (Rust 1.62+ pattern) instead of a hand-written `Default`
   impl like `IdempotencyBackend`. Functionally identical; the
   derived shape is shorter and matches more modern enum-default
   idiom. If the reviewer prefers consistency with `IdempotencyBackend`,
   easy to switch to a hand-written `impl Default`.

## Open questions for reviewer

1. **Should `with_email_port` on `InMemoryAuthBackend` keep the
   default `EchoSink` side handle alive, so `emails()` still returns
   what was sent through the wrapped port?** Current behaviour
   forcibly drops `default_echo` when a custom port is wired, which
   matches the documented "callers wanting introspection must keep
   their own `EchoSink` reference" contract but is a small
   ergonomics tax. I picked the strict path because the alternative
   (mirror every send to both the custom port and the side echo)
   has surprising semantics for callers that test against, e.g.,
   an SMTP transport.
2. **`EmailKind` is `#[non_exhaustive]`** so future kinds
   (`MfaChallenge`, `OrgInvitation`) can land without a breaking
   change. Confirm this is the right call given the kinds are
   trait-visible.
3. **The `EchoSink` "missing-`@`" check** is a minimal smoke check,
   not a real address validator. Production transports will replace
   it. Fine to leave, or should the dev sink accept any non-empty
   string?
4. **`AppState::email_port` is wired but never consumed yet** — the
   field is reserved for `PgAuthBackend` in commit 3. Acceptable
   forward-compat hole (mirrors the pattern of pre-wiring slots
   before the consumer lands) or should I gate the slot behind a
   feature flag?

## Next

Ready for fresh reviewer on commit `c0888808`. No coordination
needed before commit 3 — the next worker can pick up the storage
traits from commit 1 + `EmailPort`/`AuthBackendKind` from this
commit and start on `PgAuthBackend` + composition selector.
