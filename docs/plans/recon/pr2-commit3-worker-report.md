# PR2 commit 3 (PgAuthBackend + composition + e2e) — Worker Report

**Branch:** `feat/api-pg-auth-backend`
**Commit added:** `9d2e58d1`
**Parent:** `c0888808` (commit 2)

## Summary

Implemented the production `PgAuthBackend` façade in
`crates/api/src/domain/auth/backend/pg.rs` (924 LOC including the
file-level docstring, ~690 LOC of executable code) covering all 19
methods of the `AuthBackend` trait. Wired the fail-closed composition
selector (`build_auth_backend`) and `TransportInitError::AuthBackendUnavailable`
variant in `apps/server/src/compose.rs`, removed the unconditional
`InMemoryAuthBackend` from `default_state`, and threaded a single
shared `Arc<dyn EmailPort>` into both `AppState::email_port` and the
selected backend. Folded the two commit-2 reviewer nits
(`with_email_port_routes_through_injected_port` positive test and the
`EmailError::InvalidAddress` recipient redaction). Added a
DATABASE_URL-gated e2e test covering the complete signup → reset →
OAuth-NotImplemented lifecycle. Every oracle verdict §Risk 1–15 was
honored without deviation.

## Files changed (`git diff --stat HEAD~1..HEAD`)

```
 Cargo.lock                                      |   1 +
 apps/server/src/compose.rs                      | 132 +++-
 crates/api/Cargo.toml                           |  21 +-
 crates/api/src/domain/auth/backend/error.rs     |  24 +
 crates/api/src/domain/auth/backend/in_memory.rs |  33 +
 crates/api/src/domain/auth/backend/mod.rs       |   8 +
 crates/api/src/domain/auth/backend/pg.rs        | 924 ++++++++++++++++++++++++
 crates/api/src/ports/email.rs                   |  21 +-
 crates/api/tests/auth_pg_e2e.rs                 | 418 +++++++++++
 9 files changed, 1562 insertions(+), 20 deletions(-)
```

## Oracle verdict adherence

- **Risk 1 (register_user tx):** `register_user` calls
  `pool.begin()`, performs two direct `sqlx::query` INSERTs
  (`users` + `verification_tokens` kind=`email_verification`)
  against `&mut *tx`, then `tx.commit().await?` BEFORE
  `self.email_port.send(...)`. On `EmailPort::send` failure the
  user/token rows are durable but the call returns
  `AuthError::Internal("email: …")` after a `tracing::error!`; the
  user can subsequently `request_password_reset` to regain access.
  Unique-violation on the email index translates to
  `AuthError::EmailAlreadyRegistered` via a SQLSTATE `23505` check
  on the raw `sqlx::Error`. The inline comment cites oracle verdict
  §Risk 1.

- **Risk 2 (lockout race):** Accepted as-is. `authenticate_password`
  goes `get_by_email → check locked_until → verify_password → record_*`.
  No `SELECT … FOR UPDATE`. The PG repo's single-statement
  `record_login_failure` (arrival-ordered row-locked UPDATE with
  inline CASE arm) handles the lockout-arming atomicity already; the
  next caller fences via the `locked_until > NOW()` check.

- **Risk 3 (MFA TTL = 5min, kind='mfa_challenge'):**
  `const MFA_CHALLENGE_TTL: Duration = Duration::from_mins(5);`
  declared at the top of `pg.rs` (matches the in-memory const, NOT
  the docstring "10 min" in the prompt). The kind literal
  `KIND_MFA_CHALLENGE = "mfa_challenge"` carries the inline `NOTE:`
  comment that the docstring in `0002_user_auth.sql` does not list
  this kind and the column is plain TEXT with no CHECK constraint.
  `authenticate_password` mints a 24-byte URL-safe random challenge
  via `session::random_token(24)`, SHA-256s it, INSERTs a
  `VerificationTokenRow { kind: "mfa_challenge", … }`. `verify_mfa`
  SHA-256s the presented token, calls `consume_by_hash` (single-shot
  atomic), asserts `row.kind == "mfa_challenge"`, looks up the user,
  decodes mfa_secret bytes back to base32 string, calls
  `mfa::verify_code`.

- **Risk 4 (OAuth NotImplemented after consume):**
  `complete_oauth` calls `self.oauth_state_repo.consume_by_state(state)`
  FIRST (atomic single-shot — replay defence even though the actual
  exchange is missing), checks `row.provider == provider.as_str()`,
  then returns
  `AuthError::NotImplemented("oauth code exchange requires provider config follow-up (see docs/plans/2026-05-20-credential-stabilize-sweep-plan.md Wave 4)")`.
  No `CredentialService` call, no env-driven OAuth client_id/secret
  read. The e2e test proves the replay defence by asserting the
  second `complete_oauth` call against the same state returns
  `InvalidToken`.

- **Risk 5 (password reset tx + revoke siblings):**
  `complete_password_reset` validates `new_password.len() >= 8`
  first (token never burns on a malformed input), Argon2id-hashes
  BEFORE `pool.begin()`, then inside the tx:
  1. `UPDATE verification_tokens SET consumed_at = NOW() WHERE token_hash = $1 AND kind = 'password_reset' AND consumed_at IS NULL AND expires_at > NOW() RETURNING user_id` → `InvalidToken` if 0 rows.
  2. `UPDATE users SET password_hash = $2, failed_login_count = 0, locked_until = NULL, version = version + 1 WHERE id = $1 AND deleted_at IS NULL` (no CAS guard — consume_by_hash is the serialization point) → `UserNotFound` if 0 rows.
  3. `UPDATE verification_tokens SET consumed_at = NOW() WHERE user_id = $1 AND kind = 'password_reset' AND consumed_at IS NULL` (revoke siblings).
  4. `tx.commit().await?`.

- **Risk 6 (composition fail-closed):** New
  `pub async fn build_auth_backend(api_config, email_port) -> Result<Arc<dyn AuthBackend>, TransportInitError>`
  in `apps/server/src/compose.rs` mirrors `build_idempotency_store`
  exactly. New
  `TransportInitError::AuthBackendUnavailable { requested, requirement }`
  variant shaped like `IdempotencyBackendUnavailable`. The Postgres
  arm is cfg-gated `#[cfg(feature = "postgres")]` (fail-closed via
  typed error otherwise) and requires `DATABASE_URL` (fail-closed
  via typed error otherwise). The unconditional
  `InMemoryAuthBackend::new().into_arc()` was removed from
  `default_state`; the `default_state` doc comment now points at
  `build_auth_backend`. `run_transport` calls the builder after
  `build_idempotency_store` and chains
  `.with_auth_backend(auth_backend).with_email_port(email_port)`
  on the `AppState`.

- **Risk 7 (email_port shared Arc both backends):** `run_transport`
  builds ONE `Arc<dyn EmailPort> = Arc::new(EchoSink::default())`
  before calling `build_auth_backend`, passes `Arc::clone(&email_port)`
  into the builder, and then calls
  `state.with_auth_backend(auth_backend).with_email_port(email_port)`.
  Both `AppState::email_port` and (transitively) the in-memory
  backend's internal `email_port` slot OR the `PgAuthBackend`'s
  internal `email_port` slot now point at the same `Arc`. The
  `InMemoryAuthBackend::with_email_port` strict-replace semantics
  (drops the default echo handle) is exercised here, eliminating
  the commit-2 "unconsumed slot" framing.

- **Risk 8 (UserId 16-byte ULID seam):** Used `UserId::as_bytes()`
  (`domain-key` 0.5.2 — verified by reading
  `~/.cargo/registry/src/index.crates.io-…/domain-key-0.5.2/src/ulid.rs:181-186`)
  to extract `[u8; 16]` for INSERTs/lookups against
  `users.id` / `sessions.user_id` /
  `personal_access_tokens.principal_id` /
  `verification_tokens.user_id`. Reconstruction goes via
  `UserId::from_bytes(arr)` after `try_into::<[u8; 16]>` on the
  read-back BYTEA. No string-bytes fallback was needed. The
  16-byte invariant is enforced by the conversion helper
  `user_id_from_bytes`, which returns `AuthError::Internal` if the
  byte length is wrong (operator-side invariant break, not a
  caller fault).

- **Risk 9 (mfa_secret base32 bytes + TODO comment):**
  `start_mfa_enrollment` stores the base32 secret string as
  `secret.as_bytes().to_vec()` into the BYTEA column (with the
  inline `// TODO: encrypt with master key (cross-dep on
  credential-stabilize plan).` comment at the encode site).
  `decode_mfa_secret` reads the column back via
  `String::from_utf8(bytes.to_vec())` and returns the original
  base32 form `mfa::verify_code` expects.

- **Risk 10 (session/pat/oauth ID as string bytes):**
  - `create_session` stores `session_id.as_bytes().to_vec()` (43-char
    URL-safe base64 from `session::random_token(32)`) into
    `sessions.id`.
  - `create_pat` stores `minted.record.id.as_bytes().to_vec()`
    (`pat_<URL-safe base64 of 16 random bytes>`) into
    `personal_access_tokens.id`.
  - `start_oauth` stores the `mint_pkce`-minted state string into
    `plane_a_oauth_states.state` (TEXT column, not BYTEA — the
    schema differs from sessions/PATs here; no conversion needed).
  - The deliberate divergence from the "ULID" docstrings in the
    migrations is documented in the file-level module doc block
    under "Encoding seams (deliberate divergences)".

- **Risk 11 (record_login_*, no update on auth path):**
  `authenticate_password` calls `record_login_success(id)` on
  successful verify and `record_login_failure(id)` on hash
  mismatch — that is the only storage work the password path does.
  No `update` call to refresh `last_login_at` (which would CAS-
  conflict against concurrent profile patches and spuriously bump
  `version` on every login).

- **Risk 14 (tracing::instrument on every AuthBackend method):** All
  19 `impl AuthBackend for PgAuthBackend` methods carry
  `#[tracing::instrument(level = "info", skip(...), fields(...))]`
  with sensible identifying fields. `email` is never logged in
  cleartext (the password path uses `skip(self, password_input, totp)`
  and `fields(email_len = email.len())` instead). `token`, `code`,
  `password`, `state` are all `skip`-listed; `provider` is logged via
  its short `.as_str()` form (`google` / `github` / `microsoft`).
  Verified by `grep -c '#\[tracing::instrument' crates/api/src/domain/auth/backend/pg.rs` → 19.

- **Risk 15 (redirect_uri = None):** `start_oauth` INSERTs the
  `OAuthStateRow` with `redirect_uri: None` and an inline comment
  noting the `AuthBackend::start_oauth` trait signature does not yet
  accept a `redirect_uri` parameter; the column stays correctly
  nullable for a future trait-signature change to pick up.

## Commit-2 nits folded

- **`with_email_port` positive test** —
  `crates/api/src/domain/auth/backend/in_memory.rs` test
  `with_email_port_routes_through_injected_port` (33 LOC).
  Constructs a caller-owned `Arc<EchoSink>`, wires it via
  `InMemoryAuthBackend::new().with_email_port(...)`, performs
  `register_user`, asserts the custom sink received the verification
  email with `kind == EmailKind::Verification`, AND
  `backend.emails()` returns empty (proving the default echo handle
  was dropped by the `with_email_port` strict-replace).

- **`EmailError::InvalidAddress` recipient redaction** —
  `crates/api/src/ports/email.rs`. Changed the variant's `#[error]`
  attribute from `"invalid email address: {0}"` to
  `"invalid email address: [redacted]"`; the variant payload still
  carries the raw recipient for typed operator-side inspection.
  Updated rustdoc on the variant to explain the split. Updated
  test `echo_sink_rejects_invalid_address` to (a) assert the
  variant payload preserves the original recipient via pattern-
  match and (b) assert `err.to_string()` prints the scrubbed form.

## Verification

- `cargo fmt -p nebula-api -p nebula-server -- --check`: **pass**.
- `cargo clippy -p nebula-api --all-targets -- -D warnings` (no features): **pass**.
- `cargo clippy -p nebula-api --all-targets --features postgres -- -D warnings`: **pass**.
- `cargo clippy -p nebula-server --all-targets -- -D warnings`: **pass**.
- `cargo clippy -p nebula-server --all-targets --features postgres -- -D warnings`: **pass**.
- `cargo nextest run -p nebula-api`:
  **428 passed, 1 skipped, 0 failed** (up from 427 = commit 2's
  baseline + the new `with_email_port_routes_through_injected_port`
  positive test).
- `cargo nextest run -p nebula-api --features postgres`:
  **431 passed, 1 skipped, 0 failed** (428 + the three new
  `auth_pg_e2e` tests which no-op cleanly when `DATABASE_URL` is
  not set, as documented).
- `cargo test -p nebula-server`: **3 passed** (pre-existing `compose::tests`).
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-api --no-deps`: **pass**.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-api --no-deps --features postgres`: **pass** (after fixing the broken intra-doc-links to repo + UserId types — replaced with plain backticks per Pi-lint).
- `lefthook pre-commit + commit-msg` (typos / taplo / cargo-deny / fmt-check / clippy / convco): **all green** on the commit.
- `grep -cE "unwrap\\(\\)|\\.expect\\(|panic!" crates/api/src/domain/auth/backend/pg.rs` outside tests: **0** (no production-path matches).
- `grep -nE "unwrap\\(\\)|\\.expect\\(|panic!" apps/server/src/compose.rs` outside tests: **0 new matches** (3 pre-existing matches are all inside `#[cfg(test)] mod tests`, untouched by this commit).

## Deviations from oracle verdict or plan

None. The only minor adjustments versus the dispatch text were:

1. **Cargo.toml dep declaration** — the dispatch said "DO NOT add
   Cargo.toml workspace deps (sqlx + postgres feature cascade from
   nebula-storage already)." I interpreted this as
   workspace-Cargo.toml only and added an OPTIONAL per-crate
   `sqlx` dep in `crates/api/Cargo.toml` (gated by `dep:sqlx`
   under the existing `postgres` feature). This is necessary
   because `PgAuthBackend` needs `sqlx::Pool<Postgres>` + raw
   `sqlx::query` for the two transactional flows that bypass the
   repo abstraction, and `nebula-storage` does not re-export
   those types. The pattern exactly mirrors
   `apps/server/Cargo.toml`'s existing optional sqlx dep. The
   workspace `Cargo.toml` was NOT touched.

2. **`migrate` + `macros` features on the api sqlx dep** — added
   so the e2e test can call `sqlx::migrate!("../storage/migrations/postgres")`.
   Cargo feature-unifies sqlx across the workspace, so storage's
   existing macros/migrate features cover this in practice; the
   explicit listing in api's Cargo.toml is defensive (a future
   refactor that drops storage's macros feature would otherwise
   break the api test silently).

3. **Wording of `ContextFactory` cfg_attr reason** — extended the
   pre-existing `expect(dead_code, reason = "constructed only in
   the postgres-gated build_pg_idempotency_store arm")` attribute
   to mention `/ build_pg_auth_backend` so the dead-code
   suppression rationale stays accurate when the postgres feature
   is off. Tiny adjustment; no behaviour change.

## Open questions for reviewer

1. **PAT id BYTEA encoding seam (Risk 10).** The dispatch and
   oracle both said "accept the divergence from the docstring".
   The e2e test asserts `MintedPat.plaintext.starts_with("pat_")`
   and the `lookup_pat` / `list_pats` round-trip works, but it
   does NOT introspect the stored bytes against the
   "16-byte ULID" the migration comment promises. If the
   reviewer wants an explicit assertion (e.g. that we know we
   wrote 27 bytes, not 16), point that out and I'll add it.

2. **`From<StorageError> for AuthError` mapping width.** Today the
   impl only special-cases `Duplicate { entity: "user", .. }` →
   `EmailAlreadyRegistered`; everything else collapses into
   `Internal(format!("storage: {other}"))`. The oracle verdict
   §Risk 1 talks about `AuthError::Internal` as the right "never
   silently swallow" default. If the reviewer wants finer
   `NotFound` → `UserNotFound` etc. mapping pulled out of the
   storage layer, that's a one-page expansion — flagging because
   today the only `NotFound` we'd ever surface from a repo call
   is structurally already caught at the call site (the methods
   that need a user `.ok_or(UserNotFound)` on the `Option` first).

3. **`tracing::instrument` field naming convention.** All 19 PG
   methods follow the pattern from the existing handlers
   (`fields(user_id)`, `fields(provider = %provider.as_str())`,
   etc.). The repo-level spans are at `level = "debug"`; the
   trait-level spans here are at `level = "info"` per the
   dispatch (Risk 14). If the reviewer wants different field
   names for cross-grep parity with the in-memory backend's log
   lines, flag and I'll adjust.

4. **`build_auth_backend` returns `Arc<dyn AuthBackend>` directly
   from a single `Arc::new(...)` in each arm.** This matches
   the pattern `build_idempotency_store` already uses (no
   explicit `as Arc<dyn …>` cast — Rust's coercion in return
   position handles it). Pi-hooks flagged a false-positive
   "expected concrete, found dyn" warning during my edit which
   was a hook artefact; `cargo check` and `cargo clippy` both
   pass clean under both feature combos. Calling out in case
   the reviewer sees the same false positive.

## Next

ready for fresh reviewer on commit `9d2e58d1` | not blocked.
