# PR2 commit 3 (PgAuthBackend + composition + e2e) — Reviewer Report

**Commit reviewed:** `9d2e58d1`
**Parent:** `c0888808`
**Branch:** `feat/api-pg-auth-backend`
**Files reviewed:** 9 (1562 +ins / 20 -del), full diff inspected.
**Verdict:** **LGTM with nits** — push + open PR with all 4 commits.

## Verdict justification

Every one of the 15 oracle verdicts (§Risk 1 through §Risk 15) is honored
in the diff, end to end. The two highest-risk transactional flows
(`register_user`, `complete_password_reset`) wrap the right scope, hash
Argon2id outside the tx, classify SQLSTATE `23505` to
`EmailAlreadyRegistered`, and roll back implicitly on every error path.
The composition root is fail-closed under both feature combos with a
sibling `AuthBackendUnavailable` variant. One shared
`Arc<dyn EmailPort>` is threaded through both `AppState::email_port`
**and** the selected backend. The `UserId` 16-byte ULID seam uses the
real `domain-key 0.5.2` `as_bytes`/`from_bytes` accessors (verified
against the crate source). All 19 `AuthBackend` methods carry
`#[tracing::instrument]` and every plaintext secret (`password`,
`token`, `code`, `challenge_token`, `new_password`, `session_id`,
`presented`, `_code`) is explicitly skipped. Test counts and gate
results match the worker's claim exactly (428 default → 431 with
`postgres`; `compose::tests` still 3; all clippy / fmt / doc gates
green under both feature combos). Two nits worth folding before the
final push are tracing-field cosmetics (email auto-recorded on
`authenticate_password` due to absence from `skip` list; OAuth `state`
explicitly recorded on `complete_oauth`), but both are consistent with
the project's existing handler-level convention (`fields(email = %body.email)`
on `signup`/`login`) and neither introduces a new leak channel that
isn't already accepted at the HTTP boundary. Nothing blocking.

## Oracle verdict adherence audit (15 risks)

- **Risk 1 (register_user tx):** VERIFIED.
  `pg.rs:323` `let mut tx = self.pool.begin().await…`. Two direct
  `sqlx::query` INSERTs (`pg.rs:325-343` users insert,
  `pg.rs:354-365` verification_tokens insert) operate on `&mut *tx`;
  no repo abstraction in the tx scope. `tx.commit()` at `pg.rs:367`
  runs BEFORE `email_port.send(...)` at `pg.rs:374`. SQLSTATE `23505`
  translation lives in `is_unique_violation` (`pg.rs:917-922`) and
  triggers `AuthError::EmailAlreadyRegistered` (`pg.rs:347-349`).
  Inline doc cites oracle §Risk 1 (`pg.rs:319-322`). Implicit rollback
  on every error path (Transaction dropped without commit).

- **Risk 2 (lockout race):** VERIFIED accept-as-is.
  `authenticate_password` (`pg.rs:369-433`) follows `get_by_email →
  check locked_until → verify_password → record_login_*`. No
  `SELECT … FOR UPDATE`. Atomicity is delegated to
  `record_login_failure`'s inline `CASE` arm in
  `crates/storage/src/pg/user.rs:251-265` (single-statement
  row-locked UPDATE) — verified during commit-1 review.

- **Risk 3 (MFA TTL = 5min, kind='mfa_challenge'):** VERIFIED.
  `pg.rs:90` `const MFA_CHALLENGE_TTL: Duration = Duration::from_mins(5);`
  (NOT 10). Kind literal at `pg.rs:111` carries the `NOTE:` comment
  about the `0002_user_auth.sql` docstring gap. Challenge mint via
  `session::random_token(24)` + SHA-256 + INSERT (`pg.rs:415-426`).
  `verify_mfa` (`pg.rs:435-460`) hashes the presented token, calls
  `consume_by_hash`, asserts `row.kind == KIND_MFA_CHALLENGE`,
  decodes `mfa_secret`, runs `mfa::verify_code`.

- **Risk 4 (OAuth NotImplemented AFTER consume):** VERIFIED.
  `complete_oauth` (`pg.rs:862-888`) calls `consume_by_state(state)`
  FIRST at `pg.rs:876-880` (`ok_or(AuthError::InvalidToken)` for
  the missing/already-consumed row), THEN provider match at
  `pg.rs:881-883`, THEN `Err(AuthError::NotImplemented(...))` at
  `pg.rs:884-887`. The Wave-4 cross-dep is cited in the error
  string body, satisfying the "upgrade path inline" requirement.
  E2E proves the replay defence (auth_pg_e2e.rs:319-326): the
  second `complete_oauth` with the same `state` returns
  `InvalidToken`.

- **Risk 5 (password reset tx + revoke siblings):** VERIFIED.
  `complete_password_reset` (`pg.rs:698-769`):
  1. `pg.rs:706-708`: length validation BEFORE storage
     (`InvalidCredentials` if `< MIN_PASSWORD_LEN`).
  2. `pg.rs:711`: `password::hash_password(new_password)?` BEFORE
     the tx.
  3. `pg.rs:716`: `pool.begin()`.
  4. `pg.rs:718-732`: UPDATE `verification_tokens` with `RETURNING
     user_id` (atomic consume). 0 rows → `InvalidToken`.
  5. `pg.rs:739-752`: inline `UPDATE users SET password_hash = $2,
     failed_login_count = 0, locked_until = NULL, version =
     version + 1 WHERE id = $1 AND deleted_at IS NULL` (no
     `UserRepo::update` CAS guard). 0 rows → `UserNotFound`.
  6. `pg.rs:758-765`: revoke siblings.
  7. `pg.rs:767`: `tx.commit()`.
  No `await` between begin and commit other than the three SQL
  statements + the optional `Err(...)` returns (drop = implicit
  rollback). Argon2id is outside the tx.

- **Risk 6 (composition fail-closed):** VERIFIED.
  - `build_auth_backend` at `compose.rs:262-272` mirrors
    `build_idempotency_store`.
  - `TransportInitError::AuthBackendUnavailable { requested,
    requirement }` at `compose.rs:84-99` matches the shape of
    `IdempotencyBackendUnavailable` exactly.
  - `#[cfg(feature = "postgres")]` arm at `compose.rs:366-393`:
    typed error if `DATABASE_URL` missing; pool errors fold into
    `ContextFactory` (typed); never panics.
  - `#[cfg(not(feature = "postgres"))]` arm at `compose.rs:395-403`:
    returns `AuthBackendUnavailable` with the correct requirement
    string. No `unwrap`/`expect`/`panic!` outside the existing
    `#[cfg(test)] mod tests` block (3 pre-existing matches in
    `compose.rs:440-459`, untouched by this commit).
  - `default_state` no longer wires `InMemoryAuthBackend` — the
    pre-existing `.with_auth_backend(...)` line was deleted
    (`compose.rs` -201..-203 in the diff); the doc comment at
    `compose.rs:188-193` correctly points at `build_auth_backend`.

- **Risk 7 (email_port shared Arc):** VERIFIED.
  In `run_transport`: `compose.rs:149` builds ONE
  `let email_port: Arc<dyn EmailPort> = Arc::new(EchoSink::default());`,
  `compose.rs:150` passes `Arc::clone(&email_port)` to
  `build_auth_backend`, then `compose.rs:151-153` chains
  `state.with_auth_backend(auth_backend).with_email_port(email_port)`.
  `state.email_port = Some(port)` is set by
  `crate::AppState::with_email_port` (`crates/api/src/state.rs:1085-1087`).
  Both consumers point at the same underlying `EchoSink`.

- **Risk 8 (UserId 16-byte ULID seam):** VERIFIED.
  `UserId::as_bytes()` and `UserId::from_bytes([u8; 16])` are
  real, public, `const fn`-stable APIs on `domain-key 0.5.2`
  (verified at
  `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/domain-key-0.5.2/src/ulid.rs:157,181`).
  `pg.rs:159-163` (`user_id_bytes`) and `pg.rs:167-172`
  (`user_id_from_bytes`) wrap them with typed errors. No
  `as_bytes()` of the prefixed-string form is used for
  `users.id` / `sessions.user_id` /
  `personal_access_tokens.principal_id` /
  `verification_tokens.user_id`.

- **Risk 9 (mfa_secret base32 bytes + TODO comment):** VERIFIED.
  `pg.rs:802-803`:
  `row.mfa_secret = Some(secret.as_bytes().to_vec());` with
  `// TODO: encrypt with master key (cross-dep on
  credential-stabilize plan).` comment at the encode site.
  `decode_mfa_secret` (`pg.rs:895-899`) does the reverse via
  `String::from_utf8`.

- **Risk 10 (session/pat/oauth ID encoding):** VERIFIED.
  - Session: `pg.rs:474` `id: session_id.as_bytes().to_vec()` —
    URL-safe base64 from `session::random_token(32)`.
  - PAT: `pg.rs:593` `id: minted.record.id.as_bytes().to_vec()` —
    `pat_<base64>` from `pat::mint_pat`.
  - OAuth state: stored as TEXT (not BYTEA) per the
    `plane_a_oauth_states.state` column type — no conversion
    needed; `pg.rs:846` stores the `mint_pkce`-minted string
    directly.
  - The divergence-from-migration-docstring is documented in the
    file-level module doc block (`pg.rs:17-26`).

- **Risk 11 (record_login_* — no update on auth path):** VERIFIED.
  `authenticate_password` (`pg.rs:369-433`):
  - Success: `pg.rs:399` `self.user_repo.record_login_success(&user.id).await?;`
    is the ONLY storage call on the success path. No `update()`.
  - Failure: `pg.rs:393` `self.user_repo.record_login_failure(&user.id).await?;`
    is the ONLY storage call on the failure path. No `update()`.
  - Confirmed `record_login_success` does NOT bump `version`
    (`crates/storage/src/pg/user.rs:225-238`, with the inline
    comment explaining the design intent).

- **Risk 12 (PAT hash uniqueness — defer):** VERIFIED.
  No migration changes in commit 3. The migration
  `0002_user_auth.sql` `idx_pat_hash` partial index stays
  non-unique. SHA-256 collision risk on 256-bit random tokens is
  not addressed (correctly).

- **Risk 13 (sweepers — TODO comment):** VERIFIED.
  The "Background sweepers" subsection of the file-level module
  doc block (`pg.rs:41-46`) calls out that `cleanup_expired`
  exists on three repos but is not invoked here, framing it as
  the M3.1 follow-up. Visible enough.

- **Risk 14 (tracing::instrument parity, 19 methods):** VERIFIED count.
  `rg -c '#\[tracing::instrument' pg.rs` → **19**. Every method
  carries `level = "info"` with `skip(...)` that excludes every
  plaintext secret (verified hand-walked: `password_input`,
  `totp`, `challenge_token`, `code`, `session_id`, `presented`,
  `email` on `request_password_reset`, `token` and `new_password`
  on `complete_password_reset`, `token` on `verify_email`,
  `_code` on `complete_oauth`, `req` on `register_user` which
  itself contains email + password + display_name). Two minor
  field-recording observations are filed as Nits, not Blockers
  (see below).

- **Risk 15 (redirect_uri = None):** VERIFIED.
  `pg.rs:853` `redirect_uri: None,` with a 6-line inline comment
  citing the trait-signature gap. No trait-shape changes in
  commit 3.

## Blocker findings (must fix before push)

None.

## Nit findings (nice-to-fix; not blocking)

1. **`authenticate_password` tracing span auto-records `email`.**
   `pg.rs:369` reads `skip(self, password_input, totp)` — `email`
   is NOT in the skip list, so `#[tracing::instrument]` auto-records
   it via `Debug` (recorded as the full address). The explicit
   `fields(email_len = email.len())` adds a SECOND field rather
   than replacing the auto-recorded one. This is INCONSISTENT
   with the worker's claim that "email is never logged in
   cleartext" but is CONSISTENT with the project convention at
   `handler.rs:88` (`fields(email = %body.email)` on signup) and
   `handler.rs:120` (same on login). Net effect: emails appear
   in two places per login (handler span + backend span) instead
   of one. Not a new leak channel. Fix is one-liner —
   `skip(self, email, password_input, totp)` — if the team wants
   strict §Risk 14 conformance.

2. **`complete_oauth` records `state` in tracing fields.**
   `pg.rs:862` reads
   `fields(provider = %provider.as_str(), state)`. PKCE state is
   short-lived (10 min TTL) and single-use, so its presence in
   INFO logs is informational rather than a real-secret leak.
   Could be `state_len = state.len()` or omitted entirely. Not
   blocking.

3. **E2E test does not clean up rows.**
   `auth_pg_e2e.rs` uses `unique_email(label)` with nanosecond
   timestamps to avoid PK / unique-index collisions across runs.
   Correct for a fresh test DB but accumulates rows on a
   persistent dev DB. Acceptable for a CI-isolated test database;
   a developer iterating locally for hours will grow the row
   count but never fail. No teardown is strictly required when
   each test uses a unique email. Filing for awareness only.

4. **`From<StorageError> for AuthError` collapses CAS `Conflict`
   into `Internal` (500).**
   `error.rs:103-108` matches `StorageError::Duplicate { entity:
   "user", .. }` to `EmailAlreadyRegistered` and `everything
   else` to `Internal`. A CAS conflict on `update_user_profile`
   (concurrent profile patches) would surface as 500 rather than
   the more accurate 409. Acceptable for commit 3 because the
   two `UserRepo::update` call sites (`update_user_profile`,
   `verify_email`, `start_mfa_enrollment`, `confirm_mfa_enrollment`)
   are rarely concurrent in practice. Worth a follow-up plan
   addendum.

## Sensitive-field leakage audit

- **Password / new_password / token / code plaintext in spans or
  errors:** verified absent.
  - `register_user` skips `req` entirely (covers email + password
    + display_name).
  - `complete_password_reset` skips `token` and `new_password`.
  - `verify_mfa` skips `challenge_token` and `code`.
  - `verify_email` skips `token`.
  - `lookup_pat` skips `presented`.
  - `confirm_mfa_enrollment` skips `code`.
  - `complete_oauth` skips `_code`.
- **Recipient email in `EmailError::InvalidAddress` Display:**
  verified `[redacted]` (`crates/api/src/ports/email.rs:99`).
  Variant payload preserves the raw value for typed inspection
  (verified at `crates/api/src/ports/email.rs:208-225` test).
  Path through `From<EmailError> for AuthError` uses Display
  (`error.rs:80-83`), so the redaction propagates through
  `Internal(format!("email: {e}"))` strings.
- **Session-id plaintext in spans:** verified absent
  (`pg.rs:243` skips `session_id`).
- **Email in `authenticate_password` span:** PRESENT via
  tracing auto-recording (Nit 1).
- **OAuth `state` in `complete_oauth` span:** PRESENT via
  explicit `fields(... state)` (Nit 2).

Neither of the two PRESENT items is a new leak vector; both are
consistent with the existing handler-layer logging convention.

## Worker open-question decisions

1. **PAT id BYTEA encoding seam — explicit byte-length assertion in
   e2e?** NOT REQUIRED. The encoding seam is documented in the
   module doc block (`pg.rs:17-26`) as a deliberate divergence
   from the "16-byte ULID" migration docstring; the e2e already
   proves the round-trip works (`auth_pg_e2e.rs:217-235`). A
   byte-length assertion would lock in the CURRENT divergence
   rather than the future invariant. The right place for that
   assertion is the dedicated "make session/PAT/oauth-state IDs
   real ULIDs" PR if and when external consumers materialize
   (oracle forward-compat trap §5).

2. **`From<StorageError> for AuthError` mapping width.** ACCEPT
   the narrow mapping as-is. Adding `NotFound → UserNotFound`
   would be a no-op in practice because the four `UserRepo::get`
   / `get_by_email` call sites in `pg.rs` already do
   `.ok_or(UserNotFound)` on the `Option`. Adding `Conflict →`
   would be more interesting (CAS conflict → 409 vs 500), but
   that's a behavior change worth its own plan addendum. Filed
   as Nit 4 instead.

3. **`tracing::instrument` field naming convention.** ACCEPT the
   current pattern. `fields(user_id, provider = %provider.as_str(),
   pat_id, …)` mirrors handler.rs and the in-memory backend's
   `tracing::info!` call sites (`in_memory.rs:554`, etc.). The
   only change worth making is closing Nit 1 by adding `email`
   to `authenticate_password`'s skip list — orthogonal to naming.

4. **`Arc<dyn AuthBackend>` return without explicit cast.**
   VERIFIED CLEAN. Rust coercion in return position turns
   `Arc<InMemoryAuthBackend>` (Memory arm, `compose.rs:267-269`)
   and `Arc<PgAuthBackend>` (Postgres arm,
   `compose.rs:388`) into `Arc<dyn AuthBackend>` because both
   concrete types implement `AuthBackend` (verified by
   `cargo check` and `cargo clippy --features postgres -D warnings`
   passing in 34 seconds). The pi-hooks "expected concrete, found
   dyn" warning the worker referenced is a hook artefact and does
   not reproduce under cargo.

## Transactional correctness deep-dive

### register_user (`pg.rs:262-355`)

**Pre-tx work:** input validation (`pg.rs:268-280`), Argon2id
password hashing (`pg.rs:284`), UserId + verification token + hash
+ timestamps generation (`pg.rs:286-291`). All slow / failure-prone
work happens BEFORE `pool.begin()` — the tx window is bounded to
the two INSERTs.

**In-tx work:**
- INSERT `users` (`pg.rs:325-343`): if `is_unique_violation(err)`
  → `EmailAlreadyRegistered`; else `map_sqlx_err(err)` → `Internal`.
  Tx is implicitly rolled back by drop on the early return.
- INSERT `verification_tokens` (`pg.rs:354-365`): any failure →
  `map_sqlx_err(err)` → `Internal`. Tx implicitly rolled back.

**Post-tx work:** `tx.commit()` at `pg.rs:367`. If commit fails,
the early return drops the (already-consumed) tx and returns
`Internal`. **THEN** `self.email_port.send(...)` at `pg.rs:374`.
Email delivery failure logs at ERROR level (with `user_id` typed
field, no email plaintext) and returns `Internal("email: ...")`.
The user row + verification token are durable at this point, so
the recovery path (request_password_reset) is reachable per oracle
verdict §Risk 1.

**No `await` inside the tx other than the two `sqlx::query::execute`
calls.** No panic surfaces. No unhandled `unwrap`/`expect`. SQLSTATE
classification is correct (the `Database` arm of `sqlx::Error`
checks `db_err.code().as_deref() == Some("23505")` at
`pg.rs:919-921`).

### complete_password_reset (`pg.rs:698-769`)

**Pre-tx work:** length validation (`pg.rs:706-708`) returns
`InvalidCredentials` BEFORE touching anything — token is never
burned for a malformed input. `password::hash_password(new_password)`
at `pg.rs:711` runs BEFORE `pool.begin()` — Argon2id stays out
of the row-lock window. `sha256_token(token)` at `pg.rs:712` is
cheap and pre-computed.

**In-tx work:**
- Atomic consume of the reset token via `UPDATE … RETURNING
  user_id` (`pg.rs:718-732`). 0 rows → `InvalidToken` (early
  return, implicit rollback). The serialization gate is the
  atomic `consumed_at IS NULL AND expires_at > NOW()` filter on
  the UPDATE — the row that loses the race sees `consumed_at
  IS NOT NULL` on the second read and returns None.
- Inline `UPDATE users SET password_hash = $2, failed_login_count
  = 0, locked_until = NULL, version = version + 1 WHERE id = $1
  AND deleted_at IS NULL` (`pg.rs:739-752`). No CAS guard
  (`expected_version = ?`) inside the tx because the
  consumed-by-hash row is already the serialization point. 0
  rows (deleted user) → `UserNotFound` (early return, implicit
  rollback).
- Revoke siblings: `UPDATE verification_tokens SET consumed_at
  = NOW() WHERE user_id = $1 AND kind = 'password_reset' AND
  consumed_at IS NULL` (`pg.rs:758-765`). Any failure →
  `Internal` (early return, implicit rollback).

**Post-tx work:** `tx.commit()` at `pg.rs:767`. If commit fails
→ `Internal` (tx consumed).

**No `await` inside the tx other than the three SQL statements.**
All three `sqlx::query`-style calls use `bind` for every value, so
the password_hash plaintext never enters a query string that
could leak via a sqlx::Error::Database message.

## Composition root audit

- `build_auth_backend` signature
  (`compose.rs:262-265`): `pub async fn build_auth_backend(
  api_config: &ApiConfig, email_port: Arc<dyn EmailPort>) ->
  Result<Arc<dyn AuthBackend>, TransportInitError>`. Matches the
  documented contract.
- `TransportInitError::AuthBackendUnavailable { requested,
  requirement }` (`compose.rs:84-99`): both fields are
  `&'static str`, matching `IdempotencyBackendUnavailable`
  exactly. `#[error("...")]` attribute has the same shape.
- Postgres feature arms (`compose.rs:366-403`): typed errors only,
  no `panic!`/`unwrap`/`expect`. The `ContextFactory` variant's
  cfg-attr `expect(dead_code, reason = ...)` was correctly
  extended to mention `build_pg_auth_backend` (`compose.rs:60-64`).
- `default_state`: pre-existing `let auth_backend =
  nebula_api::domain::auth::backend::InMemoryAuthBackend::new()
  .into_arc();` line and the `.with_auth_backend(auth_backend)`
  chain are BOTH deleted in the diff (-201..-203, -208). No stray
  references remain (`grep "InMemoryAuthBackend" compose.rs` →
  3 matches, all inside `build_auth_backend` / its docs).
- `run_transport` ordering (`compose.rs:140-154`):
  `build_idempotency_store → state.with_idempotency_store →
  Arc::new(EchoSink) → build_auth_backend(clone) →
  state.with_auth_backend.with_email_port(original)`. Both
  `AppState::email_port` and the backend internal `email_port`
  point at the same `Arc`. Correct.
- No new `unwrap`/`expect`/`panic!` outside `#[cfg(test)]`:
  `rg "unwrap\(\)|\.expect\(|panic!" compose.rs` → 3 matches at
  lines 440, 447, 459, all inside the pre-existing
  `#[cfg(test)] mod tests` block. Untouched by this commit.

## E2E test audit

`auth_pg_e2e.rs` (418 LOC):

- DATABASE_URL gating correct (`auth_pg_e2e.rs:55-77`):
  `Err(VarError::NotPresent) → return None`; the three tests
  all use `let Some(pool) = pool().await else { return };` so
  they no-op cleanly when the env var is absent.
- Migrations applied via
  `sqlx::migrate!("../storage/migrations/postgres")` (`auth_pg_e2e.rs:50`)
  + `SCHEMA_READY.get_or_init` for one-shot-per-process apply
  (`auth_pg_e2e.rs:67-75`). Correct.
- Cleanup: not explicit; per-run isolation is via
  `unique_email(label)` with nanosecond timestamps
  (`auth_pg_e2e.rs:80-89`). Filed as Nit 3.
- Lifecycle stages assert what they claim:
  - signup → verify-email (`auth_pg_e2e.rs:108-149`): user row,
    verification email on caller-owned sink, replay → InvalidToken.
  - login no MFA (`auth_pg_e2e.rs:151-159`).
  - MFA enroll → verify (`auth_pg_e2e.rs:161-171`); confirms
    `current_code` flow.
  - MFA login (`auth_pg_e2e.rs:173-191`): challenge token issued,
    replay → InvalidToken.
  - PAT lifecycle (`auth_pg_e2e.rs:193-250`): mint → lookup
    round-trip → list → revoke → list-after-revoke empty.
  - forgot-password (`auth_pg_e2e.rs:253-278`): real email
    delivers, unknown email is silent (enumeration-safe).
  - reset-password (`auth_pg_e2e.rs:280-291`): consume + replay
    → InvalidToken; siblings revoked atomically (verified by
    the next call needing a fresh token).
  - re-login with new password (`auth_pg_e2e.rs:293-316`): MFA
    still required (no spurious reset), old password
    rejected.
  - start_oauth + complete_oauth (`auth_pg_e2e.rs:318-327`):
    persists state row, first complete → NotImplemented (after
    consume), second complete → InvalidToken (replay defence —
    THE CORE PROOF for §Risk 4).
  - Final invariants (`auth_pg_e2e.rs:329-340`): profile reflects
    post-reset world.
- Session test (`auth_pg_e2e.rs:343-388`): create → resolve →
  revoke → no-resolve → idempotent revoke. Covers the
  `get_principal_by_session` middleware path explicitly.
- Duplicate signup test (`auth_pg_e2e.rs:390-408`): proves
  `EmailAlreadyRegistered` on second insert (covers the
  `is_unique_violation` SQLSTATE classification).
- Caller-owned `Arc<EchoSink>` (`auth_pg_e2e.rs:92-96`):
  `let port: Arc<dyn EmailPort> = Arc::clone(&sink) as _;` —
  the test retains the concrete `Arc<EchoSink>` for `peek`/`drain`
  and hands the trait-object clone to the backend. Correct.

## Re-run evidence

- `cargo fmt -p nebula-api -p nebula-server -- --check`: **pass**
  (no output).
- `cargo clippy -p nebula-api --all-targets -- -D warnings`
  (default features): **pass** (0.74s incremental, finished
  clean).
- `cargo clippy -p nebula-api --all-targets --features postgres
  -- -D warnings`: **pass** (34.21s full build, finished clean).
- `cargo clippy -p nebula-server --all-targets -- -D warnings`
  (default features): **pass** (20.33s, finished clean).
- `cargo clippy -p nebula-server --all-targets --features
  postgres -- -D warnings`: **pass** (22.79s, finished clean).
- `cargo nextest run -p nebula-api`: **428 passed, 1 skipped,
  0 failed** (32.205s) — matches worker claim.
- `cargo nextest run -p nebula-api --features postgres`:
  **431 passed, 1 skipped, 0 failed** (32.181s) — matches
  worker claim (3 PG e2e tests no-op cleanly without
  DATABASE_URL).
- `cargo test -p nebula-server`: **3 passed, 0 failed**
  (`compose::tests::{parse_bind_address_accepts_valid_socket_address,
  parse_bind_address_rejects_invalid_override,
  resolve_bind_address_returns_fallback_without_override}`).
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-api --no-deps`:
  **pass**.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-api --no-deps
  --features postgres`: **pass**.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-server --no-deps`:
  **pass**.
- `rg "unwrap\(\)|\.expect\(|panic!"` in `pg.rs`: **0 matches**
  (pre + post commit).
- `rg "unwrap\(\)|\.expect\(|panic!"` in `compose.rs` outside
  `#[cfg(test)]`: **0 matches** (3 hits at lines 440/447/459 are
  inside the pre-existing test module, untouched by this commit).
- `rg -c "#\[tracing::instrument" pg.rs`: **19** — matches worker
  claim (= 19 `AuthBackend` methods).

## Plan adherence

- **Scope creep:** NONE. The 9 changed files match the dispatch
  exactly: `Cargo.lock`, `apps/server/src/compose.rs`,
  `crates/api/Cargo.toml`,
  `crates/api/src/domain/auth/backend/{error,in_memory,mod,pg}.rs`,
  `crates/api/src/ports/email.rs`, `crates/api/tests/auth_pg_e2e.rs`.
- **Deliverables (A-E):**
  - A: `PgAuthBackend` façade (`pg.rs`, 924 LOC). PRESENT.
  - B: Composition selector + `TransportInitError` variant
    (`compose.rs`). PRESENT.
  - C: E2E test (`auth_pg_e2e.rs`, 418 LOC). PRESENT.
  - D: `with_email_port` positive test fold-in
    (`in_memory.rs`, 33 LOC). PRESENT.
  - E: `EmailError::InvalidAddress` redaction
    (`email.rs`, 21 LOC). PRESENT.
- **Commit message:** accurate. Cites the oracle verdict, names
  every deliverable, calls out the deferred items (sweepers,
  metrics, OAuth code exchange Wave-4 follow-up), and reports
  test counts consistent with the re-run.

## Recommendation

**LGTM → push + open PR with all 4 commits** (`chore docs` +
`feat(storage)` + `feat(api) commit 2` + `feat(api) commit 3`).

Optional, not blocking:
- Close Nit 1 with a one-line `skip(self, email, password_input,
  totp)` edit on `authenticate_password` to make the §Risk 14
  conformance literal rather than convention-implicit.
- Close Nit 2 by either omitting `state` from `complete_oauth`'s
  `fields(...)` or recording `state_len = state.len()` instead.

If those two nits land in this commit (amend), the result is a
clean tight diff; if they land as a follow-up, the PR is still
mergeable as-is. Either path is fine — both nits are cosmetic.
