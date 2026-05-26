# PR2 commit 2 (EmailPort + AuthBackendKind) — Reviewer Report

**Commit reviewed:** `c0888808`
**Parent reviewed against:** `ed39fcc7`
**Branch:** `feat/api-pg-auth-backend`
**Verdict:** **LGTM with nits**

## Verdict justification

The delta is exactly the API-side preparation §PR2 commit 2 calls for and
nothing more. The new `EmailPort` trait is small (one async method,
typed two-variant error, dev `EchoSink` only), object-safe via
`#[async_trait]` to match the existing `AuthBackend` shape, and
`#[non_exhaustive]` on both `EmailKind` and `EmailError` keeps the
public seam evolvable. `AuthBackendKind` / `AuthApiConfig` mirror the
`IdempotencyBackend` / `IdempotencyApiConfig` precedent down to the
`API_*` (not `NEBULA_API_*`) env prefix, the case-insensitive parse,
the `ParseEnum` rejection on unknown values, and the `clear_env()`
test fixture. The `InMemoryAuthBackend` refactor preserves the
existing `emails()` snapshot surface through a clever double-`Arc`
pattern (`email_port: Arc<dyn EmailPort>` + `default_echo:
Option<Arc<EchoSink>>`, both cloned from the same `Arc<EchoSink>`),
so all 12 pre-existing in-memory tests and all 36 e2e tests
(`me_e2e` + `access_e2e` + `auth_mfa_csrf`) keep passing without
touching their bodies. The `From<EmailError> for AuthError` impl is
the right call to keep `?`-propagation uniform once `PgAuthBackend`
lands. Verification commands all pass on this worktree (fmt, clippy
with `-D warnings`, nextest 427/427, doc with `-D warnings`). No
`unwrap`/`expect`/`panic!` outside `#[cfg(test)]` blocks in the
scope files. Scope is honored: 8 files in `crates/api/` only, no
`crates/storage/` touch, no `apps/server/` touch, no `Cargo.toml`
churn. Nits are all genuine future-PR cleanups, not commit-2 blockers.

## Blocker findings (must fix before commit 3 starts)

None.

## Nit findings (nice-to-fix; not blocking)

1. **No positive test that a custom `EmailPort` injected via
   `InMemoryAuthBackend::with_email_port` is actually used.** The
   builder + `default_echo = None` drop are documented inline and the
   `password_reset_round_trips` test exercises the default-port path
   through the side handle, but there is no test like
   `with_custom_port_routes_signup_email_through_injected_port` that
   asserts `register_user → signup` lands in a caller-controlled
   `Arc<EchoSink>` instead of the default echo. Easy ~25-LOC add;
   forward-compat insurance for the SMTP transport follow-up.
2. **`From<EmailError> for AuthError` carries the recipient address
   into the `Internal` error string** when `EchoSink` rejects an
   `@`-less `to`. Specifically:
   `EmailError::InvalidAddress(msg.to)` → `Display`:
   `"invalid email address: <to>"` → `format!("email: {e}")` →
   `AuthError::Internal("email: invalid email address: <to>")` →
   `ApiError::Internal(...)`. In practice this never fires on the
   signup path because `register_user` validates the email shape
   before calling `record_email`, and the password-reset path
   swallows + logs — so the leak is theoretical. Worth scrubbing
   `to` from the `InvalidAddress` formatter or wrapping it in
   `[REDACTED]`-style display in commit 3 alongside any richer
   transient/permanent split. The worker's doc on the `From` impl
   already flags PR2 commit 3 as the natural place to revisit.
3. **`AuthBackendKind` uses `#[derive(Default)] + #[default]
   Memory`** while the immediately-preceding `IdempotencyBackend`
   hand-writes its `Default`. Both compile to the same machine code;
   pick one and apply it consistently. The derive pattern is the
   modern idiom (Rust 1.62+), so I would align `IdempotencyBackend`
   forward rather than regress `AuthBackendKind` — but doing so is
   out-of-scope for commit 2 and belongs in a tiny follow-up.
4. **`EchoSink::send` returns `EmailError::InvalidAddress(msg.to)`
   with the *original* untrimmed `to`** rather than the trimmed
   value used for the actual check. Intentional (operator forensics)
   and stated as much in the doc, but a one-liner clarification in
   the field-level rustdoc that "the error carries the value the
   caller submitted, not the trimmed form" would prevent a future
   reader from filing a bug.
5. **`AppState::email_port` is currently wired but unread** — see
   Decision §4. Not a footgun (zero panic sites; default `None`
   means any handler that wants email must `Option`-unwrap
   explicitly and report 503 on its own). Worth a single grep-pin
   when commit 3 wires the composition selector so the slot does
   not stay an unconsumed builder forever.
6. **No `cleanup`-style sanity grep for `parking_lot::RwLock` vs
   `tokio::sync::RwLock`.** `EchoSink::send` is `async fn` but
   acquires a synchronous `parking_lot::RwLock` write guard. Fine
   for the dev sink — sub-millisecond critical sections under any
   realistic test load — but a production SMTP impl should not
   inherit this and the trait doc should mention that holding a
   sync lock across an `.await` is the impl's responsibility to
   avoid. Doc nit.

## Re-run evidence

All commands executed from
`C:/Users/vanya/RustroverProjects/nebula/.worktrees/pg-auth-backend`:

- `cargo fmt -p nebula-api -- --check`: **pass** (exit 0, no output).
- `cargo clippy -p nebula-api --all-targets -- -D warnings`:
  **pass** (clean `Finished` line, no warnings).
- `cargo nextest run -p nebula-api`:
  **427 passed, 1 skipped, 0 failed** in 32.305s. Matches the
  worker claim of "427 = PR1's 418 + the 9 new tests" exactly.
- `cargo nextest run -p nebula-api --tests config::sub`:
  **13 passed** (10 pre-existing idempotency + 3 pre-existing for_test +
  5 new auth backend; one of the "13" overlaps the for_test bucket).
  All 5 new `from_env_auth_backend_*` / `for_test_auth_backend_*`
  tests pass independently.
- `cargo nextest run -p nebula-api --tests ports::email`:
  **4 passed** (all 4 new echo-sink tests).
- `cargo nextest run -p nebula-api -E 'test(/domain::auth::backend::in_memory/)'`:
  **12 passed, 0 failed.** All pre-existing in-memory backend tests
  survive the refactor (the `password_reset_round_trips` migration
  from `email_sink.write().clear()` to
  `default_echo.as_ref().expect(...).drain()` works correctly via
  the shared inner `Arc<RwLock<Vec<EmailMessage>>>`).
- `cargo nextest run -p nebula-api -E 'binary(me_e2e) + binary(access_e2e) + binary(auth_mfa_csrf)'`:
  **36 passed, 0 failed.** Confirms backward compat across the
  three flagged integration suites.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-api --no-deps`:
  **pass** (`Generated target/doc/nebula_api/index.html`).
- `rg "unwrap\(\)|expect\(|panic!" crates/api/src/ports/email.rs
  crates/api/src/domain/auth/backend/in_memory.rs` outside
  `#[cfg(test)]` blocks: **0 hits**. All matches in production
  scope are `unwrap_or_default()` (infallible recovery on
  `SystemTime::duration_since(UNIX_EPOCH)` /
  `chrono::Duration::from_std(LOCKOUT_TTL)`) and are pre-existing
  (untouched by this commit).
- `rg "#\[(tokio::)?test\]" crates/api/src/config/sub.rs
  crates/api/src/ports/email.rs`:
  **17 total** (13 in `config/sub.rs` — 8 pre-existing + 5 new;
  4 in `ports/email.rs` — all new).
- **New tests counted: 9** (5 config + 4 echo sink). Matches the
  worker claim exactly.
- `git show c0888808 --stat`: 8 files, all under `crates/api/`.
  `git diff --stat ed39fcc7..c0888808 -- crates/storage/ apps/`:
  empty. No scope creep.

## Decisions on open questions (with rationale)

1. **`with_email_port` drops the default `EchoSink` side handle —
   accept strict semantics.** Mirroring sends to both the injected
   port and a hidden in-process echo would surprise callers testing
   against, e.g., a wiremock-backed SMTP impl ("why is my prod
   transport double-billed?"). The strict path keeps "the injected
   port is the single source of truth" obvious; callers that need
   introspection can wire the `EchoSink` themselves and keep their
   own `Arc` clone — exactly the contract the doc spells out.
2. **`EmailKind` is `#[non_exhaustive]` — accept.** Future kinds
   (`MfaChallenge`, `OrgInvitation`, `BillingNotice`) can land
   without a breaking change; external matchers are forced to write
   a `_` arm which is the correct migration path for an evolving
   public enum. Internal callers (this crate) can still exhaustively
   match (the attribute only affects external crates), so the dev
   ergonomics cost is zero. Same reasoning applies to the
   `#[non_exhaustive]` on `EmailError`.
3. **`EchoSink` "missing-`@`" check — accept minimal.** This is a
   dev/test port; real address validation belongs in the production
   transport (lettre's `Mailbox::from_str` for SMTP, AWS SES
   rejection codes, etc.). Stricter validation would also create
   false-positive risk in dev (e.g. rejecting `root@localhost`
   which is legitimately routable in test fixtures). The minimal
   check is enough to exercise the `EmailError::InvalidAddress`
   path via one test (`echo_sink_rejects_invalid_address`) without
   becoming a half-validator anyone might rely on.
4. **`AppState::email_port` wired but unconsumed — accept
   forward-compat hole.** Verified by grep
   (`rg "\.email_port" crates/api/src/ apps/`) that the only
   write sites are `AppState::with_email_port` and
   `InMemoryAuthBackend::with_email_port`, and the only read site
   is `InMemoryAuthBackend::email_port.send(...)` — which is the
   backend's *own* field, not `AppState::email_port`. So
   `AppState::email_port` has zero consumer paths today; no
   `.unwrap()` / `.expect()` reads it, and the default is `None`.
   Nothing can panic on it. The slot is forward-compat scaffolding
   for commit 3's composition selector (pass one `Arc<dyn
   EmailPort>` to both `AppState::with_email_port` and
   `PgAuthBackend::new`) and for future non-auth email consumers
   (org invitations, billing). Gating it behind a feature flag
   would add noise without preventing any failure mode the current
   shape already prevents.

## Per-file audit

### `crates/api/src/ports/email.rs` (NEW, 224 LOC)

- `EmailPort: Send + Sync` with `#[async_trait]` — matches the
  `AuthBackend` shape (line 100 of `provider.rs`), so a single
  `Arc<dyn EmailPort>` works across every handler and storage-backed
  impl. Object-safe.
- `EmailMessage { to, subject, body, kind }` matches the call site in
  `in_memory.rs:253-260` 1:1 (verified by grep).
- `EmailKind { Verification, PasswordReset, Generic }` is
  `#[non_exhaustive]`. `as_str()` is `const fn` and returns the
  legacy `"EmailVerify"` / `"PasswordReset"` / `"Generic"` labels;
  the inline test `email_kind_as_str_matches_legacy_labels` pins
  this so the `EmailEnvelope.kind` back-compat shim cannot drift
  silently.
- `EmailError { Transport, InvalidAddress }` is `thiserror`-derived,
  small, and `#[non_exhaustive]`. No `Box<dyn Error>` escape
  hatch. The `Display` strings carry the operator-facing detail —
  see Nit §2 for the recipient-in-message observation.
- `EchoSink` field is private (`inbox: Arc<RwLock<Vec<EmailMessage>>>`),
  cloneable, `Default`. `peek()` returns `Vec<EmailMessage>` (a
  clone — does not leak the inner `Arc<RwLock>`). `drain()` uses
  `std::mem::take(&mut *self.inbox.write())` which atomically
  swaps in `Vec::default()` and returns the old contents — the
  inbox is genuinely cleared in one critical section.
- `EmailPort for EchoSink::send` trims `to`, rejects empty or
  `@`-less, otherwise pushes. The error preserves the *original*
  `msg.to` for operator forensics (see Nit §4).
- Observability: `#[tracing::instrument(level = "debug", …)]` on
  `send` (mandatory per CLAUDE.md DoD) plus `peek` / `drain`
  (worker deviation §5). The `peek` / `drain` instrumentation is
  cheap (`skip(self)`, no payload field) and uniform with `send`
  — accept.
- 4 inline tests cover: happy-path buffer, drain-clears-inbox,
  invalid-address rejection (verifies `EmailError::InvalidAddress`
  is the variant returned), and the legacy-label compat. All pass
  independently.

### `crates/api/src/state.rs` (+22 LOC)

- `pub email_port: Option<Arc<dyn EmailPort>>` field added at
  state.rs:299, mirrors the `auth_backend: Option<Arc<dyn AuthBackend>>`
  shape at state.rs:283. Default is `None` (initialized in
  `AppState::new` at the matching site).
- `with_email_port(self, port: Arc<dyn EmailPort>) -> Self` builder
  at state.rs:1085, follows the existing `with_*` convention
  (`#[must_use = "builder methods must be chained or built"]`).
- Doc comment correctly states "the in-memory `InMemoryAuthBackend`
  keeps its own default `EchoSink` inbox when no port is provided"
  — accurate observation about the current decoupling between
  `AppState::email_port` (forward-compat slot) and
  `InMemoryAuthBackend::email_port` (the backend's own field).
- Forward-compat hole: see Decision §4. The slot is reserved but
  no read path exists yet; commit 3 will consume it via
  composition.

### `crates/api/src/config/sub.rs` + `config/mod.rs` + `config/env.rs`

- `AuthBackendKind { Memory, Postgres }` at sub.rs:180 with
  `#[derive(Default)] + #[default] Memory` (worker deviation §6 —
  modern Rust 1.62+ idiom; minor inconsistency with the hand-written
  `IdempotencyBackend::default` impl in the same file, see Nit §3).
  `#[serde(rename_all = "kebab-case")]` matches `IdempotencyBackend`.
- `AuthApiConfig { backend: AuthBackendKind }` at sub.rs:200 with
  `#[derive(Default)]` — mirrors `IdempotencyApiConfig` shape
  (single-field struct). Extension points (lockout knobs, session
  TTL overrides, MFA enforcement) can land here without changing
  the env-binding shape.
- `ApiConfig::auth: AuthApiConfig` at config/mod.rs:141 with
  `#[serde(default)]` for back-compat with existing JSON config
  files. Wired into the `Debug` impl at mod.rs:179 (with
  `.field("auth", &self.auth)`), into `from_env` at mod.rs:301
  (via `auth_from_env()`), and into `for_test` at mod.rs:422
  (via `AuthApiConfig::default()`).
- `auth_from_env()` at mod.rs:328 — parses `API_AUTH_BACKEND`
  (correct prefix: matches `API_IDEMPOTENCY_BACKEND`, NOT
  `NEBULA_API_*`). Case-insensitive `memory` / `postgres`; unknown
  values rejected with `ApiConfigError::ParseEnum { var:
  "AUTH_BACKEND", raw }` — same error shape as the idempotency
  path.
- `config/env.rs` clear_env list extended with `API_AUTH_BACKEND`
  at line 91. Necessary so the env-binding tests don't leak state
  across parallel nextest runs.
- 5 new tests at sub.rs:397-475: defaults-to-memory,
  accepts-postgres, case-insensitive, rejects-unknown, and
  for_test-defaults-to-memory. All gated by `env_lock()` +
  `clear_env()`. All pass independently.
- **Missing test (nit):** no equivalent of
  `from_env_idempotency_rejects_invalid_ttl` for the auth path
  because `AuthApiConfig` currently has no numeric knobs. Add one
  when the lockout/TTL knobs land.

### `crates/api/src/domain/auth/backend/error.rs` (+14 LOC)

- `From<EmailError> for AuthError` impl at error.rs:73-83 collapses
  both `EmailError::Transport(_)` and `EmailError::InvalidAddress(_)`
  into `AuthError::Internal(format!("email: {e}"))`. The mapping
  preserves the original error detail string in the `Internal`
  variant via the `Display` interpolation.
- Doc comment correctly flags PR2 commit 3 as the natural place to
  revisit (richer mapping: transient vs hard reject) — accept the
  scope decision.
- See Nit §2 for the small recipient-in-message observation. Not a
  blocker because (a) it only matters on signup, where the user
  knows their own address, and (b) the `request_password_reset`
  path swallows the error before it surfaces to the client.
- Worker deviation §4 — having the `From` impl in `error.rs` rather
  than inline at the call site is the right call for `?`-ergonomics
  and lets `PgAuthBackend` reuse the same conversion for free.

### `crates/api/src/domain/auth/backend/in_memory.rs` (+142 LOC)

- Field swap at in_memory.rs:107-113:
  - `email_port: Arc<dyn EmailPort>` (was
    `email_sink: Arc<RwLock<Vec<EmailEnvelope>>>`).
  - `default_echo: Option<Arc<EchoSink>>` — side handle for the
    `emails()` introspection shim.
  - **Clever pattern verified:** the `Default` impl at lines
    116-130 constructs *one* `Arc<EchoSink>`, then `Arc::clone`s
    it into both `email_port` (upcast to `Arc<dyn EmailPort>`) and
    `default_echo`. Because `EchoSink` itself contains an
    `Arc<RwLock<Vec<EmailMessage>>>`, the two outer `Arc`s share
    the *same* inner inbox — `email_port.send()` pushes into the
    same Vec that `default_echo.drain()` clears. This is the
    cleanest available pattern without bolting `Any` onto the
    trait surface (worker deviation §2 — accept).
- `InMemoryAuthBackend::new()` delegates to `Default::default()`
  (line 167-170), so a fresh backend always wires the default
  `EchoSink` into both slots.
- `with_email_port(self, port: Arc<dyn EmailPort>) -> Self` at
  lines 188-191 sets `email_port = port` AND clears `default_echo
  = None`. Strict semantics — see Decision §1.
- `emails()` at lines 200-208 is `#[must_use]`, returns
  `Vec<EmailEnvelope>` (legacy shape via `EmailEnvelope::from` shim).
  Returns empty `Vec` when `default_echo == None` (i.e. after a
  `with_email_port` swap) — the doc explicitly states this contract.
  `#[allow(deprecated, …)]` is scoped to the shim closure.
- `EmailEnvelope` legacy type at lines 142-152 marked
  `#[deprecated(since = "0.2.0", note = "...")]`. NOT re-exported
  from `domain/auth/backend/mod.rs` (verified by grep) — external
  crates cannot see it via the trimmed re-export surface, only via
  the deep path `crate::domain::auth::backend::in_memory::EmailEnvelope`.
  Minimal blast radius.
- `record_email` at lines 240-265: now `async`, builds
  `EmailMessage { to, subject, body: token, kind }` and calls
  `self.email_port.send(msg).await?`. The `body == token` dev
  convention is documented inline (worker deviation §3 — accept,
  the shim contract is closed by the
  `EmailEnvelope::from(EmailMessage)` impl).
- `register_user` at lines 287-329: `record_email` failures
  propagate via `?` (signup fails closed if email is broken —
  exactly the contract).
- `request_password_reset` at lines 503-533: `record_email`
  failure logged via `tracing::error!` + swallowed; returns
  `Ok(())` unconditionally. **Enumeration-safety preserved** — the
  caller cannot distinguish "no such email" from "email send
  failed", which is the documented contract for this path. Worker
  also correctly handles the *token mint* failure path with the
  same swallow-and-log pattern, which is stricter than the previous
  code (which would have panicked on RNG failure).
- Internal test `password_reset_round_trips` at line 873 migrated
  from `email_sink.write().clear()` to
  `default_echo.as_ref().expect("default echo sink is wired when
  no custom port is injected").drain()`. The `expect()` is
  test-only and the message is informative; the test runs in the
  default-port branch where the assertion holds.
- All 12 in-memory tests pass (verified independently). All 36
  e2e tests in the flagged suites pass (verified independently).

## Forward-compat assessment for commit 3

Clean. The `EmailPort` shape lets `PgAuthBackend` build itself with
its own `email_port: Arc<dyn EmailPort>` field (mirroring the
in-memory backend) plus the five repo `Arc`s; the
`register_user` flow becomes
`UserRepo::create → VerificationTokenRepo::create →
self.email_port.send(EmailMessage { ... }).await?`, with the `?`
propagating cleanly via `From<EmailError> for AuthError`. The
composition root in commit 3 will construct one `Arc<dyn EmailPort>`
(initially the dev `EchoSink`, later an SMTP impl) and pass it to
both `AppState::with_email_port` (filling the forward-compat slot
this commit reserved) and `PgAuthBackend::new`. Holding the email
port on the `PgAuthBackend` struct itself — rather than reading
`state.email_port` lazily — matches the existing pattern (the
in-memory backend already owns its own port; `auth_backend` is
already on both `AppState` and the request path) and keeps the
storage backend self-contained for tests. No change needed to
`EmailPort`, `EmailMessage`, `EmailError`, or `EchoSink` for commit
3 to land.

## Plan adherence

- **Scope creep:** None.
  `git diff --stat ed39fcc7..c0888808 -- crates/storage/ apps/`
  returns empty. All 8 changed files are under `crates/api/`. No
  new workspace dependencies (no `Cargo.toml` change).
- **Deliverables A-F (plan §PR2 commit 2):** all present.
  - A. `EmailPort` trait + `EchoSink` impl
    (`crates/api/src/ports/email.rs`, NEW). ✓
  - B. `AppState::email_port` slot + `with_email_port` builder
    (`crates/api/src/state.rs`). ✓
  - C. `AuthBackendKind { Memory, Postgres }` enum
    (`crates/api/src/config/sub.rs`). ✓
  - D. `AuthApiConfig` struct + `ApiConfig::auth` wiring + env
    binding `API_AUTH_BACKEND`
    (`crates/api/src/config/sub.rs` + `mod.rs` + `env.rs`). ✓
  - E. `From<EmailError> for AuthError` impl
    (`crates/api/src/domain/auth/backend/error.rs`). ✓
  - F. `InMemoryAuthBackend` refactor to consume `EmailPort` with
    backward-compat `emails()` shim
    (`crates/api/src/domain/auth/backend/in_memory.rs`). ✓
- **Commit message accuracy:** matches the diff. The "no behaviour
  change for existing `InMemoryAuthBackend` callers" claim is
  verified by the 12-in-memory + 36-e2e green re-run.

## Recommendation

**LGTM → proceed to `oracle` consult → commit 3 (`PgAuthBackend`
+ composition selector).**

The two nits worth folding into commit 3 (since they touch the
same area):

- Add the `with_email_port` positive test (Nit §1) — ~25 LOC,
  forward-compat insurance for the SMTP transport follow-up.
- Decide on the recipient-in-Internal-msg policy (Nit §2) when the
  richer `EmailError` mapping lands.

Other nits (§3 `Default` consistency, §4 `EchoSink::send` doc
clarification, §5 `state.email_port` consumer pin, §6
sync-lock-across-`.await` doc) can land in commit 3 or as a small
follow-up commit on the same branch; none block the
`PgAuthBackend` worker from picking up.
