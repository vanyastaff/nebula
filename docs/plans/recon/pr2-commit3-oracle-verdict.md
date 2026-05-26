# PR2 commit 3 — Oracle Verdict

**Consulted before:** worker dispatch for commit 3 (`PgAuthBackend` façade
+ composition-root selector).

**Scope of this verdict:** design-only pre-flight against the 7 risk
surfaces in the dispatch prompt, plus discoveries from a fresh read of
commits 1 + 2 (`a8a949ea`, `c0888808`), the trait surface
(`provider.rs`), the in-memory backend (`in_memory.rs`), the trait
definitions and the `0001_users.sql` / `0002_user_auth.sql` /
`0028_plane_a_oauth_state.sql` migrations.

**Posture:** opinionated. Worker should follow the verdicts unless a
later reviewer surfaces a concrete contradiction.

---

## Risk verdicts

### Risk 1 — `register_user` transactional boundary

**Verdict:** **transaction wrap (user-create + token-create);
email send outside the tx.**

**Rationale.** The three failure modes the prompt lists are real and
not symmetrical. Mode (a) "user exists, token exists, email send
failed" is recoverable — the user can `request_password_reset` to
regain access (the in-memory backend already supports
forgot-password against `email_verified=false` accounts; PG must
match). Modes (b) and (c) — orphaned user with no verification token,
or `email_verified_at IS NULL` user that the next signup attempt
hits as `EmailAlreadyRegistered` — are the dangerous ones because
the user has no recovery path that does not require operator
intervention. Wrapping the two INSERTs in one `sqlx::Transaction`
eliminates (b) and (c) without inheriting the impossible "roll back
a sent email" requirement (which would force a saga). Email send
stays after `tx.commit()`; on email failure we log + return
`AuthError::Internal` (mode a), and the user can still recover via
password-reset.

The trait surface (`UserRepo::create(&Pool)`, `VerificationTokenRepo
::create(&Pool)`) is pool-bound, so the cleanest path inside the
~900-LOC commit-3 budget is to **bypass the repo abstraction for
the register_user happy path** — issue two `sqlx::query(...)`
INSERTs against an `&mut Transaction<'_, Postgres>` directly, with
an inline doc comment marking this as a deliberate seam and a TODO
to retrofit the repos with an `Executor`-generic shape when a
second multi-step flow appears. This avoids adding `Executor`
generics across five trait methods just to satisfy one caller,
and keeps the repo trait contract (single-call atomicity) intact.

**Concrete worker instruction.** Inside `PgAuthBackend::register_user`,
`pool.begin().await?`, INSERT `users` row, INSERT `verification_tokens`
row (kind=`email_verification`), `tx.commit().await?`, THEN call
`self.email_port.send(...).await?`. Comment: "two-step tx bypasses
the repo abstraction by design — see oracle verdict §Risk 1; convert
to Executor-generic repos when a second multi-step flow lands."

---

### Risk 2 — Lockout race + `record_login_failure` semantics

**Verdict:** **accept as-is.** No additional measure needed.

**Rationale.** Commit 1's reviewer correctly debunked the worker's
"REPEATABLE READ best-effort" framing. PG's default is `READ
COMMITTED`; the single-statement `UPDATE … SET failed_login_count
= failed_login_count + 1, locked_until = CASE … END WHERE id = $1`
serializes concurrent same-row writers via row-level locking and
re-reads the post-commit row. The threshold-arming `CASE` evaluates
against the post-increment value the racer actually writes, so the
lockout arms exactly once per threshold-crossing. The `WHERE
locked_until > NOW()` gate the `PgAuthBackend::authenticate_password`
flow reads from the freshly-loaded UserRow is sound: any racing
caller that crossed the threshold has already committed the
`locked_until` write before the next caller's `get_by_email`
returns.

The only remaining race window is the TOCTOU between
`get_by_email → password::verify_password → record_login_failure`
and a concurrent `record_login_success`. Scenario: T1 loads with
count=4, T2 loads with count=4, T1 verifies password OK → success
clears (count=0, locked_until=NULL), T2 verifies password FAIL →
records failure (count=0+1=1). Net result: 1 failed attempt on
record after a 4-fail-then-1-success-then-1-fail sequence.
This **matches in-memory semantics** (in-memory has the same
load-then-mutate window) and is the contract callers already
expect.

The PG path additionally inherits the "every failed attempt while
count >= threshold extends the lockout window" behaviour from the
inline CASE, which is exactly what in-memory does
(`if u.failed_login_count >= LOCKOUT_THRESHOLD { u.locked_until =
Some(until); }`). Parity preserved.

No additional `SELECT … FOR UPDATE` needed. No race window the
in-memory backend does not already accept.

---

### Risk 3 — MFA challenge state machine via `VerificationTokenRepo`
(kind = `mfa_challenge`)

**Verdict:** **reuse `verification_tokens`; `kind = 'mfa_challenge'`;
TTL = 5 minutes (parity with in-memory `MFA_CHALLENGE_TTL =
Duration::from_mins(5)`); do NOT introduce a config knob in commit 3.**

**Rationale.** The plan and dispatch prompt cite "10 min" but the
**actual in-memory const is 5 minutes** (`in_memory.rs:42` —
`MFA_CHALLENGE_TTL: Duration = Duration::from_mins(5)`). The plan
prompt is wrong; the worker must match the existing const for
behavioural parity, not the docstring. Concurrent `verify_mfa`
calls are handled by `PgVerificationTokenRepo::consume_by_hash`,
which is an atomic `UPDATE … WHERE token_hash = $1 AND
consumed_at IS NULL AND expires_at > NOW() RETURNING …`
(reviewer verified in commit 1). Single-shot consume is the same
guarantee in-memory provides via `mfa_challenges.remove(...)`.

**Doc-only caveat to acknowledge in the worker prompt.** The
`0002_user_auth.sql` `kind` column comment lists `'email_verification'
/ 'password_reset' / 'org_invite' / 'mfa_recovery'` and does NOT
mention `'mfa_challenge'`. The column is plain `TEXT` with no CHECK
constraint, so storing `'mfa_challenge'` works correctly today, but
the migration comment is incomplete. **Worker should NOT add a new
migration to fix the comment** (that would push commit 3 over its
budget for cosmetic reasons); instead leave a `// NOTE:` comment in
`pg.rs` next to the kind-literal usage and file a follow-up cleanup
ticket. The behavioural conflict between `'mfa_recovery'` (backup
codes — long-lived, not implemented today) and `'mfa_challenge'`
(in-flight 2FA gate — 5-min single-shot) is purely a naming concern
because `mfa_recovery` is unused.

**Concrete instruction.** In `PgAuthBackend::authenticate_password`'s
MFA branch: mint a 24-byte URL-safe random challenge token via
`session::random_token(24)`, SHA-256 it, INSERT
`VerificationTokenRow { token_hash, user_id, kind: "mfa_challenge",
payload: None, created_at: now, expires_at: now + 5 min,
consumed_at: None }`, return the plaintext challenge token. In
`PgAuthBackend::verify_mfa`: SHA-256 the presented challenge token,
call `consume_by_hash`, assert `row.kind == "mfa_challenge"`,
`get(row.user_id)` the user, decode `mfa_secret` to base32 string,
call `mfa::verify_code(secret, code)`. Use a `const MFA_CHALLENGE_TTL:
Duration = Duration::from_mins(5);` at the top of `pg.rs` (not
imported from `in_memory.rs` — keep backend modules independent).

---

### Risk 4 — OAuth `complete_oauth` implementation

**Verdict:** **`AuthError::NotImplemented("oauth code exchange
requires provider config follow-up")` AFTER consuming + validating
the state row.** Mirrors in-memory's posture, with the critical
difference that the PG path performs the replay defence even when
the code exchange is absent.

**Rationale.** The plan explicitly defers operator-secret OAuth
configs to a follow-up cross-dep on
`2026-05-20-credential-stabilize-sweep-plan.md` Wave 4. Commit 3
must NOT route through `CredentialService` and must NOT introduce
ad-hoc env-driven `GOOGLE_CLIENT_ID`/`SECRET` plumbing (that would
predate Wave 4 and create exactly the kind of parallel-secret-path
the credential plan is trying to eliminate). At the same time, the
in-memory backend's current shape (`in_memory.rs:573-577`) returns
`NotImplemented` AFTER inspecting (but not consuming) the state
row, which leaves the row replayable. The PG path can do better
cheaply: `consume_by_state` is atomic, so we get the replay defence
"for free" even though the actual token exchange is still
not-implemented. This is honest and forward-compatible — when the
Wave-4 follow-up wires `CredentialService::get::<OAuth2Credential>`,
it replaces the `NotImplemented` return with the real exchange
without changing any storage semantics.

**Concrete instruction.** `PgAuthBackend::complete_oauth(provider,
state, _code)`:
1. `let row = self.oauth_state_repo.consume_by_state(state).await?
   .ok_or(AuthError::InvalidToken)?;`
2. `if row.provider != provider.as_str() { return
   Err(AuthError::InvalidToken); }`
3. `return Err(AuthError::NotImplemented("oauth code exchange
   requires provider config follow-up (see
   docs/plans/2026-05-20-credential-stabilize-sweep-plan.md
   Wave 4)"));`

Doc-comment the cross-dep on the method body so the next maintainer
sees the upgrade path inline. `start_oauth` is fully implementable
today: mint PKCE state with the existing `oauth::mint_pkce`
helper, INSERT into `plane_a_oauth_states`, return synthetic
authorize URL (same shape in-memory uses) — no provider config
needed.

---

### Risk 5 — `complete_password_reset` atomicity

**Verdict:** **transaction wrap all three operations: consume_by_hash
+ user update + revoke_all_for_user.** Bypass the repo abstraction
the same way Risk 1 does.

**Rationale.** Two correctness concerns stack here. First, the
prompt's atomicity question: if `consume_by_hash` succeeds but
`UserRepo::update` fails (CAS conflict or storage error), the
token is burned without the password actually changing. The user
has to request a new reset token. That is poor UX and avoidable.
Second, the prompt's revoke-siblings question: the in-memory
backend keeps reset tokens in a dashmap with a 1-hour TTL and a
realistic scenario of one outstanding token per user, so it does
not bother revoking siblings. PG retains state durably and at low
cost; with rate limits on `request_password_reset` not yet wired
(the prompt accepts this as a separate concern), an attacker could
in principle accumulate multiple unconsumed reset tokens. Calling
`revoke_all_for_user(user_id, "password_reset")` after the password
update closes the replay window cheaply. The trait method exists
exactly for this.

CAS conflict on `UserRepo::update` is rare in practice (the user is
mid-reset, not concurrently editing their profile) but the tx wrap
is cheap insurance and the semantics are clean: if anything fails,
roll back, surface a typed error, let the caller retry with the
same reset token. The tx ensures the token stays unconsumed if the
user update fails, so retry actually works.

**Concrete instruction.** `PgAuthBackend::complete_password_reset(token,
new_password)`:
1. Validate `new_password.len() >= 8` (mirror in-memory) BEFORE
   touching storage. Return `AuthError::InvalidCredentials` on
   failure so the token is not burned for a malformed input.
2. Compute `password::hash_password(new_password)` BEFORE the tx
   so the Argon2id work happens outside the lock window.
3. `let mut tx = self.pool.begin().await?;`
4. UPDATE `verification_tokens SET consumed_at = NOW() WHERE
   token_hash = $1 AND kind = 'password_reset' AND consumed_at IS
   NULL AND expires_at > NOW() RETURNING user_id` — if 0 rows,
   `return Err(AuthError::InvalidToken);`.
5. UPDATE `users SET password_hash = $2, failed_login_count = 0,
   locked_until = NULL, version = version + 1 WHERE id = $1 AND
   deleted_at IS NULL` (no CAS guard inside the tx — the
   consume-by-hash gate is the serialization point). If 0 rows,
   `return Err(AuthError::UserNotFound);`.
6. UPDATE `verification_tokens SET consumed_at = NOW() WHERE
   user_id = $1 AND kind = 'password_reset' AND consumed_at IS
   NULL`.
7. `tx.commit().await?; Ok(())`.

Note: step 5 deliberately does NOT use the repo's CAS-protected
`update` because the serialization gate is already the consumed-
by-hash row. Inside the tx, the user row is already protected by
the row-lock the `RETURNING` UPDATE in step 5 acquires.

---

### Risk 6 — Composition root fail-closed

**Verdict:** **confirm fail-closed pattern; mirror
`build_idempotency_store` exactly with one extra hop for the shared
`Arc<dyn EmailPort>`.**

**Concrete instruction.** Add `async fn build_auth_backend(api_config:
&ApiConfig, email_port: Arc<dyn EmailPort>) -> Result<Arc<dyn
AuthBackend>, TransportInitError>` to `apps/server/src/compose.rs`.
Shape parallels `build_idempotency_store`:

- `AuthBackendKind::Memory` arm: return `InMemoryAuthBackend::new()
  .with_email_port(email_port).into_arc()`. No env reads, no async
  IO needed but the function stays `async fn` for symmetry.
- `AuthBackendKind::Postgres` arm: cfg-gated. Without
  `feature = "postgres"`, return
  `TransportInitError::IdempotencyBackendUnavailable`-style typed
  error (add a sibling variant
  `TransportInitError::AuthBackendUnavailable { requested,
  requirement }`). With `feature = "postgres"`, read
  `DATABASE_URL` (return typed error if missing — same shape as
  the existing idempotency arm), build a fresh
  `PgPoolOptions::new().max_connections(8).connect(&url).await`
  pool, instantiate the five repos, instantiate
  `PgAuthBackend::new(user_repo, session_repo, pat_repo,
  verification_token_repo, oauth_state_repo, email_port)`, wrap
  `Arc::new(...)`.

In `ServerRuntime::run_transport`, after the existing
`build_idempotency_store` call:
- Build ONE `Arc<dyn EmailPort>` (today `Arc::new(EchoSink::default())
  as Arc<dyn EmailPort>`; future SMTP impl).
- Call `let auth_backend = build_auth_backend(&api_config,
  email_port.clone()).await?;`
- `state = state.with_auth_backend(auth_backend).with_email_port(
  email_port);`

**Important:** `default_state` should STOP unconditionally wiring
`InMemoryAuthBackend`. The auth backend is now selected
conditionally by `build_auth_backend`, parallel to the idempotency
store. Remove the
`.with_auth_backend(InMemoryAuthBackend::new().into_arc())` call
from `default_state` to avoid double-wiring (the second
`.with_auth_backend` call would overwrite anyway, but the dead
constructor is misleading). Update the §"Plane-A identity backend"
comment in `default_state` to point at `build_auth_backend`.

Two parallel sqlx pools (one for idempotency, one for auth) is
acceptable for commit 3 — consolidation is a follow-up. Document
this in the `build_auth_backend` doc comment.

---

### Risk 7 — `email_port` wiring per backend

**Verdict:** **always set `state.email_port = Some(shared_port)`
for BOTH backends.** Both backends additionally hold their own
`Arc<dyn EmailPort>` internally. No special-case for `Memory`.

**Rationale.** The prompt offers two views — let `state.email_port`
stay `None` for Memory until a non-auth consumer appears, OR
always populate it. The "always populate" path is strictly more
forward-compatible: future non-auth email consumers (org
invitations, billing notices) read from `state.email_port` and
work uniformly regardless of which backend is wired. The cost is
zero (an Arc clone on startup). The "stay None for Memory" path
introduces a special-case that the next developer has to discover
and reason about; it offers no benefit because `state.email_port`
is already wired-but-unread today (commit 2 reviewer Decision §4
explicitly accepts this).

This also harmonizes commit 2's `InMemoryAuthBackend::with_email_port`
contract: the backend is the *current* email consumer, but
`state.email_port` is the *future* email consumer slot. Both should
point at the same Arc when an explicit port is wired.

**Concrete instruction.** In `run_transport`, build the shared port
ONCE (`let email_port: Arc<dyn EmailPort> =
Arc::new(EchoSink::default());`) before the backend-selector call,
pass `email_port.clone()` into `build_auth_backend`, and chain
`.with_email_port(email_port)` on `AppState` regardless of
backend choice. Drop the worker's commit-2 §Decision-§4
"unconsumed slot" framing — the slot becomes consumed by future
non-auth flows; for commit 3 the consumer is just both backends
internally.

---

## Additional risks the prompt missed

### Risk 8 — `UserId` / opaque-ID conversion seam

`UserId` is a prefixed ULID (`usr_01J9...`) provided by the
`domain_key::define_ulid!` macro. The API trait surface uses
`UserId` as a typed value, and `AuthBackend` methods take `&str`
for `user_id` (parsed back to `UserId` inside each method —
e.g. `let parsed: UserId = user_id.parse()?`). The PG schema
expects `BYTEA(16)` raw ULID bytes for `users.id`. The worker
needs a stable conversion at the seam.

**Concrete instruction.** Use `UserId::ulid().to_bytes()` (or the
equivalent accessor on `domain_key` ULIDs) to extract the 16-byte
representation for INSERTs/UPDATES/lookups; store the BYTEA, not
the prefixed string. On read-back, reconstruct the prefixed
`UserId` via the `domain_key`-provided `from_bytes`/`from_ulid`.
Verify the exact method names from `domain_key` 0.5.2 docs
during implementation. If `domain_key` does not expose direct
ULID-byte accessors, fall back to storing `user_id_string.as_bytes()
.to_vec()` as opaque bytes — but the prefixed-string-as-bytes
fallback breaks the BYTEA(16) contract documented in
`crates/storage/src/rows/user.rs:10` ("`user_` ULID, 16-byte
BYTEA"), so this is a last resort and warrants a doc TODO if used.

### Risk 9 — `users.mfa_secret` BYTEA encoding

`UserRow.mfa_secret: Option<Vec<u8>>` with the doc claim
"Encrypted with master key." The encryption is aspirational —
no encryption is wired today (parallels the commit-1 §3 decision
about `code_verifier` plaintext). In-memory stores
`mfa_secret: Option<String>` as base32. The PG path needs a
consistent encoding.

**Concrete instruction.** Store the base32 secret as raw UTF-8
bytes (`base32_string.as_bytes().to_vec()`); decode via
`String::from_utf8(bytes)?` before passing to
`mfa::verify_code(secret, code)`. Add a `// TODO: encrypt with
master key per ADR (cross-dep on credential-stabilize plan)`
comment at the encode site. Do NOT pull `nebula-credential`
master-key access into `nebula-api` for commit 3 — same posture
as commit-1 §3.

### Risk 10 — `session.id` BYTEA vs base64url-string shape

`session::random_token(32)` returns a 43-char URL-safe base64
string. The schema declares `sessions.id BYTEA PRIMARY KEY --
sess_ ULID`. There is a shape mismatch (43 chars of base64url ≠
16-byte ULID). The migration comment is aspirational and there is
no length constraint on BYTEA.

**Concrete instruction.** Store `session_id_str.as_bytes().to_vec()`
into `sessions.id`; decode via `String::from_utf8` on read. The
schema BYTEA PK accepts arbitrary-length keys, and the API
contract is that session_id is opaque to callers. Do NOT change
`session::random_token` to mint a 16-byte ULID — that would
diverge from the in-memory backend and break
`crates/api/tests/me_e2e.rs` and friends. Add a one-line doc
comment noting the deliberate divergence from the
"sess_ ULID" docstring in `0002_user_auth.sql`.

Same pattern applies to PAT id and OAuth state where opaque
strings are stored as BYTEA — match the existing helper return
shapes, do not refactor primitives in commit 3.

### Risk 11 — `record_login_success` is the right call after successful password verify, NOT `UserRepo::update`

The PG repo decision (commit 1 Deviation §2, reviewer Decision §2):
`record_login_success` / `record_login_failure` do NOT bump
`users.version`. This is intentional. The worker must use
`record_login_success`/`failure` on the password-verify path
exclusively; do NOT additionally call `update` to refresh
`last_login_at` because that would race against concurrent
profile patches via CAS conflict and would spuriously bump version
on every login.

**Concrete instruction.** `PgAuthBackend::authenticate_password`
calls `record_login_success(id)` on success and
`record_login_failure(id)` on hash mismatch — that is all the
storage work the password path does. No `update` calls in this
method.

### Risk 12 — PAT hash uniqueness

`idx_pat_hash` is a non-unique partial index on
`personal_access_tokens(hash) WHERE revoked_at IS NULL`. SHA-256
collision on a 256-bit random token is vanishingly unlikely
(birthday bound ≫ practical), so this is a forward-compat
trap rather than a commit-3 blocker. Worker should NOT change
the migration in commit 3; file a follow-up to consider
upgrading the index to UNIQUE when the next storage migration
window opens.

### Risk 13 — Background sweepers are out of scope

`SessionRepo::cleanup_expired`, `VerificationTokenRepo::cleanup_expired`,
`OAuthStateRepo::cleanup_expired` all exist but
nothing calls them yet. PG tables will grow unbounded under
production use. **Do not wire a sweeper task in commit 3** (it
would inflate scope and tangle with the runtime shutdown
contract). Add a single `// TODO: wire sweeper job — see M3.1
follow-up` comment somewhere visible (e.g. the `PgAuthBackend`
doc-comment) and file a small plan addendum.

### Risk 14 — `tracing::instrument` parity on the trait surface

The 19 `AuthBackend` trait methods should carry
`#[tracing::instrument(level = "info", skip(self, …), fields(…))]`
spans on the PgAuthBackend impl, mirroring the 10 handlers that
already do this on the HTTP boundary. The PG repos already
carry `level = "debug"` spans; the AuthBackend-trait-level spans
are the layer the reviewer will check. Pin this in the worker
prompt so it does not get omitted under deadline pressure.

### Risk 15 — `OAuthStateRow.redirect_uri` plumbing

Commit 1 reviewer Nit §3 flagged that `redirect_uri` is present in
the row + migration but unused by `OAuthStateEntry`. Commit 3 is
the natural place to plumb it through `start_oauth`/`complete_oauth`,
but the current `AuthBackend` trait signature for `start_oauth` does
NOT accept a `redirect_uri` parameter. **Do not change the trait
signature in commit 3** (that ripples into in-memory backend +
handler + DTO + tests, far outside the budget). Store `redirect_uri =
None` in PG for now and leave a TODO. The field stays correctly
nullable; future trait-signature change picks it up.

---

## Forward-compat traps

1. **Two parallel sqlx pools (idempotency + auth).** Acceptable for
   commit 3 — each binary holds 8 connections × 2 = 16 — but in
   six months when the credential service and a sessions sweeper
   also need pools, the count grows. File a follow-up to introduce
   a shared `Arc<PgPool>` on `AppState` and have each subsystem
   borrow it. Not a commit-3 concern.

2. **`Executor`-generic repos.** The transaction-wrap decisions on
   Risk 1 + Risk 5 bypass the repo abstraction by calling
   `sqlx::query` directly inside `PgAuthBackend`. When the third
   multi-step flow appears (likely: a "rotate refresh token"
   operation when OAuth completes properly post-Wave-4), the
   pressure to retrofit `Executor`-generic repo methods will be
   real. Today's bypass is a known seam; doc-comment it inline so
   the next worker does not assume the repo abstraction is
   inviolable.

3. **Migration `0002_user_auth.sql` `kind` column lacks a CHECK
   constraint.** Storing `'mfa_challenge'` works today because PG
   does not enforce the docstring enum. If a future migration adds
   a CHECK constraint, it must include `'mfa_challenge'`. File a
   tiny ADR note or add the kind to the `0002` comment in a
   sweep-up migration.

4. **`mfa_secret` and `code_verifier` plaintext storage.** Both are
   marked TODO-encrypt-with-master-key. When the credential plan
   Wave 4 lands, both should migrate to encrypted-at-rest. This is
   recorded in commit 1 §3 and Risk 9 here.

5. **Session ID, PAT ID, OAuth state are stored as opaque
   `string.as_bytes()` rather than the "ULID" the migration comments
   advertise.** If any future tooling parses these BYTEA columns
   as raw ULIDs (e.g. a CLI or external read replica), it will
   break. Pre-commit-3 design call: accept the divergence from the
   docstring or refactor the primitives. **Verdict: accept the
   divergence for commit 3** (Risk 10), with a doc note. Refactor
   the primitives in a separate "make session/PAT/oauth-state IDs
   real ULIDs" PR if and when external consumers materialize.

6. **No metrics emitted from the auth backend.** `nebula_api_auth_*`
   is roadmap §M3.1 box 6 (deferred follow-up per the plan). Worker
   must NOT add ad-hoc counters in commit 3.

---

## Sign-off

**Cleared to dispatch the commit 3 worker** with the verdicts and
concrete instructions below folded into the dispatch prompt. The
design is sound, the trait surface is stable after commits 1 + 2,
and the two transactional decisions (Risks 1 + 5) are the only
non-mechanical design calls. No blockers; no further oracle pass
needed before the reviewer audits the worker's diff.

Two soft expectations the parent should bake into the dispatch
prompt to keep the reviewer audit short:

1. The worker MUST fold the two commit-2 nits the prompt already
   names — `with_email_port_routes_through_injected_port` positive
   test, and scrubbing the recipient address from
   `EmailError::InvalidAddress` Display. The second one ships as
   a one-line change in `crates/api/src/ports/email.rs`'s
   `EmailError` variant Display impl (replace `{0}` with
   `[redacted]` for the InvalidAddress variant; preserve the
   original `to` in the variant payload for operator-side
   inspection via the typed value, just not in `Display`).

2. The reviewer prompt should explicitly include "verify the two
   transactional boundaries (register_user, complete_password_reset)
   match this oracle verdict §Risk 1 + §Risk 5" so the design
   intent does not drift into in-band scope of the diff.

---

## Concrete worker instructions to add to the dispatch prompt

Copy these bullets into the worker dispatch as a **§Design contract
(non-negotiable)** block. They are distilled from the verdicts
above.

- **register_user (Risk 1):** wrap user-create + verification-token-create
  in `sqlx::Transaction`; bypass the repo abstraction
  inside the tx (two direct `sqlx::query` INSERTs). Email send
  AFTER `tx.commit().await?`; on email failure return
  `AuthError::Internal` and log. Inline comment: "two-step tx
  bypasses repos — see oracle verdict §Risk 1."
- **authenticate_password (Risk 2 + Risk 11):** load via
  `get_by_email`, check `locked_until > NOW()` (return
  `AccountLocked`), `password::verify_password`; on success call
  `record_login_success(id)` ONLY (no `update` call), then enter
  the MFA branch if `mfa_enabled`; on failure call
  `record_login_failure(id)` and return `InvalidCredentials`. No
  `SELECT … FOR UPDATE`.
- **MFA challenge (Risk 3):** declare `const MFA_CHALLENGE_TTL:
  Duration = Duration::from_mins(5);` (NOT 10 — the prompt was
  wrong; parity with in-memory). Mint a 24-byte random challenge
  via `session::random_token(24)`, SHA-256 it, INSERT into
  `verification_tokens` with `kind = "mfa_challenge"`. Add a
  `// NOTE: kind 'mfa_challenge' is not in the 0002 migration
  docstring; column is plain TEXT with no CHECK.` comment at the
  literal. `verify_mfa` hashes the presented token, calls
  `consume_by_hash`, asserts `row.kind == "mfa_challenge"`, looks
  up the user, decodes `mfa_secret` UTF-8 to base32, calls
  `mfa::verify_code`.
- **complete_oauth (Risk 4):** call `consume_by_state(state)`
  (atomic replay defence), validate `row.provider == provider.as_str()`,
  then return `AuthError::NotImplemented("oauth code
  exchange requires provider config follow-up (see docs/plans/
  2026-05-20-credential-stabilize-sweep-plan.md Wave 4)")`. Do
  NOT route through `CredentialService`. Do NOT read OAuth
  client_id/secret from env.
- **start_oauth:** mint PKCE via existing `oauth::mint_pkce`,
  INSERT into `plane_a_oauth_states` (10-min TTL via
  `oauth::OAUTH_STATE_TTL`), return the synthetic
  `https://nebula.local/oauth/<provider>/authorize?...` URL same
  shape as in-memory. `redirect_uri = None` per Risk 15.
- **complete_password_reset (Risk 5):** validate
  `new_password.len() >= 8` first, hash the new password BEFORE
  the tx (Argon2id is slow). Then `pool.begin().await?`, atomic
  consume of the reset token (UPDATE … RETURNING user_id), UPDATE
  `users SET password_hash = $2, failed_login_count = 0,
  locked_until = NULL, version = version + 1 WHERE id = $1 AND
  deleted_at IS NULL`, UPDATE `verification_tokens SET consumed_at
  = NOW() WHERE user_id = $1 AND kind = 'password_reset' AND
  consumed_at IS NULL` (revoke siblings), `tx.commit().await?`.
- **Composition root (Risk 6):** add `async fn
  build_auth_backend(api_config, email_port) -> Result<Arc<dyn
  AuthBackend>, TransportInitError>` mirroring
  `build_idempotency_store`. Add
  `TransportInitError::AuthBackendUnavailable { requested,
  requirement }` variant. Postgres arm: cfg-gate
  `feature = "postgres"`; require `DATABASE_URL`; fail closed with
  typed error if either missing. Remove the
  `InMemoryAuthBackend::new().into_arc()` call from `default_state`
  (the conditional builder owns this now). Update the comment in
  `default_state` to point at `build_auth_backend`.
- **email_port (Risk 7):** in `run_transport`, build ONE
  `Arc<dyn EmailPort>` (today `Arc::new(EchoSink::default())`),
  pass `port.clone()` into `build_auth_backend`, chain
  `.with_email_port(port)` on `AppState`. Both backends share the
  SAME Arc; both branches set `state.email_port` to `Some(port)`.
- **UserId/BYTEA seam (Risk 8):** use the raw 16-byte ULID
  representation from `domain_key` (likely `UserId::ulid().to_bytes()`
  or equivalent) for INSERTs/lookups against `users.id`,
  `sessions.user_id`, `personal_access_tokens.principal_id`,
  `verification_tokens.user_id`. Verify the exact `domain_key`
  0.5.2 API at implementation time. Last resort fallback:
  `user_id_string.as_bytes().to_vec()` with a doc TODO.
- **mfa_secret encoding (Risk 9):** store base32 string as
  `as_bytes().to_vec()`; decode `String::from_utf8` on read.
  `// TODO: encrypt with master key (cross-dep on
  credential-stabilize plan)` comment at the encode site.
- **session id / pat id / oauth state encoding (Risk 10):** store
  the existing `session::random_token` / `pat::mint_pat` / etc.
  string outputs as `as_bytes().to_vec()`. Doc-comment the
  divergence from the "ULID" hint in migration comments.
- **Tracing parity (Risk 14):** every `impl AuthBackend for
  PgAuthBackend` method carries
  `#[tracing::instrument(level = "info", skip(self, …), fields(…))]`
  with sensible identifying fields. Match the existing handler
  span pattern (`user_id`, never `email` in clear text on the
  password path, never `token` / `code` / `password`).
- **`EmailError` recipient leak fix (commit-2 nit fold-in):** in
  `crates/api/src/ports/email.rs`, change the
  `EmailError::InvalidAddress(String)` variant's `Display` impl
  to print `[redacted]` (or a fingerprint hash) instead of the
  raw recipient. Keep the original `to` value in the variant
  payload for typed inspection. Update the inline test
  `echo_sink_rejects_invalid_address` accordingly.
- **`with_email_port_routes_through_injected_port` test (commit-2
  nit fold-in):** add a ~25-LOC `#[tokio::test]` in
  `crates/api/src/domain/auth/backend/in_memory.rs` that
  constructs a custom `Arc<EchoSink>`, wraps it with
  `InMemoryAuthBackend::with_email_port(...)`, performs
  `register_user`, and asserts the custom sink received the
  verification email while `default_echo` is dropped (i.e.
  `backend.emails()` returns empty because the back-compat shim
  only reads the default echo).
- **`PgAuthBackend` module structure:** `pub struct
  PgAuthBackend { user_repo: Arc<PgUserRepo>, session_repo:
  Arc<PgSessionRepo>, pat_repo: Arc<PgPatRepo>,
  verification_token_repo: Arc<PgVerificationTokenRepo>,
  oauth_state_repo: Arc<PgOAuthStateRepo>, pool: Pool<Postgres>,
  email_port: Arc<dyn EmailPort> }`. The `pool: Pool<Postgres>`
  is held alongside the repos because the two transactional
  flows (register_user, complete_password_reset) call
  `pool.begin().await?` directly. `PgAuthBackend::new(...)`
  takes the pool and the email port, constructs the five repo
  Arcs internally. Re-export at `crates/api/src/domain/auth/
  backend/mod.rs`.
- **E2E test (`crates/api/tests/auth_pg_e2e.rs`):** DATABASE_URL-
  gated, mirrors the `crates/storage/src/pg/*::tests` shape with
  `let Some(pool) = pool().await else { return };`. Covers:
  signup → verify-email → login → MFA enroll → MFA login →
  PAT mint → PAT-authenticated request → PAT revoke → forgot-
  password → reset-password → re-login with new password →
  start_oauth persists row → complete_oauth returns
  NotImplemented after consuming state. Asserts the email
  port receives expected messages (use a caller-owned
  `Arc<EchoSink>`).
- **DO NOT touch:** the in-memory backend method bodies (other
  than the new positive test); the trait definitions in
  `provider.rs` or `repos/user.rs`; `apps/server` outside the
  identified `default_state` + `run_transport` edits;
  `Cargo.toml` workspace deps (sqlx + the postgres feature
  already cascade through `nebula-storage`).
- **STRICT TDD per `openspec/config.yaml`:** if that file
  declares a strict TDD test runner, follow RED → GREEN →
  TRIANGULATE → REFACTOR per change. Otherwise: write tests
  alongside impls and ensure `task dev:check` (or
  `cargo nextest run -p nebula-api -p nebula-storage --features
  postgres`) passes locally before commit.
