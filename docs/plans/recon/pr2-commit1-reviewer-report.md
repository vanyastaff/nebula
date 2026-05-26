# PR2 commit 1 (storage) — Reviewer Report

**Commits reviewed:** `893a6e0f` (chore docs) + `a8a949ea` (feat storage)
**Verdict:** **LGTM with nits**

## Verdict justification

The storage delta is a clean, scope-correct foundation for the production
PG-backed `AuthBackend`. Every PG impl returns typed `StorageError`,
funnels every `sqlx` call through `map_db_err`, carries
`#[tracing::instrument(level = "debug", …)]`, and asserts at least one
invariant per file via `debug_assert!`. The two trait additions
(`VerificationTokenRepo`, `OAuthStateRepo`) extend the existing
`repos/user.rs` shape without disturbing `UserRepo` / `SessionRepo` /
`PatRepo`. Migration `0028_plane_a_oauth_state.sql` ships with PG and
SQLite parity, names the table to avoid colliding with the Plane-B
credential pending-state surface, and indexes the expected sweep
predicate. The single-shot consume pattern on
`PgVerificationTokenRepo` and `PgOAuthStateRepo` uses
`UPDATE … WHERE … consumed_at IS NULL AND expires_at > NOW()
RETURNING …`, which is the canonical PG pattern for atomic
consume-or-noop and is replay-safe. Verification commands all pass on
this worktree (fmt, clippy with `--features postgres` and
`--features postgres,sqlite`, nextest, doc). No code outside
`crates/storage/` was touched and the `chore(docs)` commit is
strictly `docs/plans/*` additions.

The only substantive concern is a factual mistake in the worker's
open-question framing about PostgreSQL isolation (see Decision §1),
which does not change the code but should not be carried forward as
project lore. Everything else is genuine nit territory.

## Blocker findings (must fix before commit 2 starts)

None.

## Nit findings (nice-to-fix; not blocking)

1. **Doc / report claim is wrong about PG default isolation.**
   The module-level doc on `crates/storage/src/pg/user.rs:14-19` says
   "concurrent failed logins may race so the per-row counter is
   best-effort", and the worker report restates this as "best-effort
   under postgres's default REPEATABLE READ default isolation".
   PostgreSQL's documented default is `READ COMMITTED`, not
   `REPEATABLE READ`. Under `READ COMMITTED`, a single
   `UPDATE … SET failed_login_count = failed_login_count + 1, …`
   statement uses row-level locking and **re-reads** the row on
   unblocking, so the counter strictly increments by one per
   successfully-committed call and the inline `CASE` evaluates against
   the post-increment value the racer actually wrote. The lockout
   arming is therefore exact, not best-effort, without `SELECT … FOR
   UPDATE`. Recommend toning down the doc comment and dropping the
   "REPEATABLE READ" phrasing in the worker report when it's archived.
2. **Six separate `SCHEMA_READY: OnceCell` cells** (one per pg test
   module: `oauth_state`, `pat`, `session`, `user`,
   `verification_token`, plus the pre-existing `control_queue`). The
   migration is idempotent (sqlx tracks applied migrations) so this is
   correct, but a shared `test_support::pg_pool()` helper would DRY
   this up. Pre-existing pattern in `pg/control_queue.rs`, so this is
   not new debt — flag for a follow-up cleanup commit, not this PR.
3. **`OAuthStateRow.redirect_uri`** is present in the row and
   migration but unused by the in-memory `OAuthStateEntry`
   (`crates/api/src/domain/auth/backend/oauth.rs:67-77`). It's
   nullable and only consumed in commit 3 — harmless forward-compat
   field, just worth noting.
4. **`bind(lockout_secs as f64)`** for `make_interval(secs => $3)` is
   functionally correct (PG `make_interval` takes `double precision`
   for `secs`), but the `as f64` cast is slightly opaque. A clearer
   alternative is `bind(f64::from(LOCKOUT_THRESHOLD))`-style or
   `(NOW() + ($3 || ' seconds')::interval)`. Pure style nit.
5. **`PgUserRepo::soft_delete` bumps `version` without a CAS check.**
   Matches the trait (which takes no `expected_version` on
   `soft_delete`), and the `deleted_at IS NULL` guard makes the
   operation idempotent under concurrent callers. Fine, but worth a
   one-line inline comment explaining the deliberate omission of CAS
   so reviewers of commit 3 don't ask the same question.
6. **PAT test fixture pads `random_id()` into a 32-byte `Vec<u8>`**
   instead of using a real SHA-256 helper. As the worker notes, the
   repo does byte-equality lookup so this is harmless for round-trip
   coverage. Nit: when commit 3 adds the API-level pat-mint helper, it
   would be nice to expose a `test_support::sha256_for_test()`
   shorthand so tests across crates exercise the same hash shape.
7. **No `cleanup_expired` round-trip test for
   `PgVerificationTokenRepo`** (the other three cleanup-bearing repos
   have one). Easy to add (~15 LOC) but the impl is a one-line
   `DELETE`, so coverage value is low.

## Re-run evidence

All commands executed from
`C:/Users/vanya/RustroverProjects/nebula/.worktrees/pg-auth-backend`:

- `cargo fmt -p nebula-storage -- --check`: **pass** (exit 0).
- `cargo clippy -p nebula-storage --all-targets -- -D warnings`:
  **pass** (cached, then forced rebuild via `touch` to confirm).
- `cargo clippy -p nebula-storage --all-targets --features postgres
  -- -D warnings`: **pass** (re-checked via `touch` → 6.88s clean
  build).
- `cargo clippy -p nebula-storage --all-targets --features
  postgres,sqlite -- -D warnings`: **pass** (re-checked via
  `touch` → 12.41s clean build).
- `cargo nextest run -p nebula-storage --features postgres`:
  **285 tests run, 285 passed, 0 skipped** in 2.038s. The 21 new
  DATABASE_URL-gated tests report `PASS` because
  `let Some(pool) = pool().await else { return };` short-circuits when
  `DATABASE_URL` is unset (project convention — matches the existing
  `pg/control_queue.rs` test fixture). This means the new tests are
  effectively "PASS-on-skip" in this environment; they exercise real
  SQL only when a worker explicitly sets `DATABASE_URL`. Not a
  regression — same shape as the pre-existing `pg::idempotency` and
  `refresh_claim_pg_integration` suites.
- `cargo nextest run -p nebula-storage --features postgres,sqlite`:
  **295 tests run, 295 passed, 0 skipped** in 2.240s (bonus run for
  the dual-feature configuration). All new tests still pass-on-skip;
  SQLite-side coverage was unaffected.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-storage --no-deps`:
  **pass**.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-storage --no-deps
  --features postgres`: **pass**.
- `rg -n "unwrap\(\)|expect\(|panic!"` across the five new
  `crates/storage/src/pg/*.rs` files: **0 hits in non-test code**.
  All matches are inside `#[cfg(all(test, feature = "postgres"))]`
  blocks (test fixtures, `DATABASE_URL` parsing, OnceCell init).
  This is the project convention and is acceptable.
- `rg -c "#\[tokio::test\]"` in the five new files:
  - `pg/oauth_state.rs`: 4
  - `pg/pat.rs`: 4
  - `pg/session.rs`: 4
  - `pg/user.rs`: 5
  - `pg/verification_token.rs`: 4
  - **Total: 21.** Worker claim verified.
- `rg -in "plane_a_oauth_states|oauth_states"
  crates/storage/migrations/ crates/storage/src/`:
  Only the new migration + the 5 new files reference
  `plane_a_oauth_states`. No `oauth_states` table name exists
  anywhere. No collision.

## Decisions on open questions (with rationale)

1. **Lockout race window — accept as-is, but fix the doc.** The
   worker frames this as "best-effort under postgres's default
   `REPEATABLE READ` default isolation". PostgreSQL default isolation
   is **`READ COMMITTED`**, not REPEATABLE READ
   (see PG docs: `default_transaction_isolation = 'read committed'`).
   Under `READ COMMITTED`, a single-statement `UPDATE` that targets
   the same row from two transactions serializes via row-level
   locking: the second transaction re-reads the row when it unblocks
   and re-applies the increment. So `failed_login_count` increments
   strictly by one per successfully-committed call, and the inline
   `CASE … WHEN failed_login_count + 1 >= $2 …` arms `locked_until`
   on the threshold-crossing call exactly as expected. No
   `SELECT … FOR UPDATE` is needed, and the framing as "best effort"
   is unnecessarily pessimistic. The actual best-effort question would
   be: "can a racer observe a stale `locked_until` and bypass the
   lockout?" — answer is no, because the predicate
   `WHERE locked_until > NOW()` is checked by the caller on the
   *post-commit* row state at login time, not on a snapshot. Verdict:
   **accept the code; trim the doc/report wording**.
2. **`record_login_*` does NOT bump `users.version` — agree.** This
   is the right call. `users.version` is the CAS knob for
   `UserRepo::update`, which protects domain mutations (profile,
   password, MFA enable/disable). Auth bookkeeping fields
   (`last_login_at`, `failed_login_count`, `locked_until`) are
   write-mostly side data; bumping `version` on every login attempt
   would invalidate concurrent profile patches and force callers to
   reload-and-retry on every successful login. Verdict: **agree, no
   change**.
3. **`OAuthStateRow.code_verifier` plaintext TEXT — accept.** PKCE
   `code_verifier` is high-entropy ephemeral session data (~64 bytes
   of CSPRNG, 10-min TTL, single-shot consume). It is not a long-lived
   secret in the same threat model as a refresh token or a credential
   client_secret (which Plane-B encrypts with the master key). An
   attacker who can read the `plane_a_oauth_states` table already has
   the keys to the kingdom; encrypting one ephemeral column would not
   meaningfully raise the bar, and pulling `nebula-credential`
   master-key access into `nebula-storage` would invert the
   architecture (storage should not depend on the credential
   subsystem). Verdict: **accept plaintext; do not encrypt before
   commit 3**.
4. **No SQLite parity for the 5 PG repos — correct per scope.** The
   plan §PR2 commit 1 explicitly asks for PG impls + migration parity
   (so the SQLite schema does not drift), with SQLite repos listed as
   a future increment. The legacy SQLite identity store at
   `crates/storage/src/sqlite/identity.rs` is a different surface
   entirely (`port_users` TEXT-id mirror), so a SQLite mirror of the
   five new traits would be a green-field addition, not a small
   parallel. Verdict: **correct; defer SQLite repos to a follow-up
   PR**.
5. **Test PAT hashes padded to 32-byte `BYTEA` — accept.**
   `PgPatRepo::get_by_hash` and the underlying `idx_pat_hash` partial
   index are byte-equality lookups — the repo never re-hashes the
   incoming bytes. Test correctness depends only on round-trip
   identity of the `hash` column, which the current fixture
   satisfies. The hash function itself (`pat::hash_for_lookup` in
   `nebula-api`) is exercised in API-side tests; duplicating it in
   `nebula-storage` would create a cross-crate hash invariant that
   does not currently exist. Verdict: **accept; commit 3 owns the
   API-side hash invariant tests**.

## Per-file audit

### `crates/storage/migrations/postgres/0028_plane_a_oauth_state.sql`

- TEXT primary key on `state` matches the rationale (CSPRNG url-safe
  string, not a ULID).
- Columns map 1:1 to `OAuthStateRow`; types are sound
  (`TIMESTAMPTZ`, `TEXT`, nullable where appropriate).
- The single index `idx_plane_a_oauth_states_cleanup ON (expires_at)
  WHERE consumed_at IS NULL` is exactly what `cleanup_expired` and
  `consume_by_state` care about (partial index keeps the live working
  set small).
- Table name `plane_a_oauth_states` avoids the Plane-B credential
  pending-state collision. Verified — no other migration references
  `oauth_states` or `plane_a_oauth_states`.
- No `ON DELETE` clauses needed (no FK — state is pre-login).

### `crates/storage/migrations/sqlite/0028_plane_a_oauth_state.sql`

- ISO-8601 `TEXT` timestamps match the existing SQLite convention
  (see `0002_user_auth.sql` sqlite mirror).
- Same partial index syntax — SQLite supports partial indexes since
  3.8.0.
- Column shapes are 1:1 with the PG migration (less the type
  promotion).

### `crates/storage/src/pg/user.rs`

- `UserTuple` column order is documented and matches `SELECT_COLS`.
- `create`: 14-column INSERT with explicit binds; pre-INSERT
  `debug_assert!` on `id` and `email`.
- `get` / `get_by_email`: both honour `deleted_at IS NULL`.
- `update`: CAS via `WHERE id = $1 AND version = $12 AND deleted_at
  IS NULL`, then on `rows_affected() == 0` does a follow-up SELECT to
  disambiguate **NotFound** vs **Conflict**. This is the right shape
  for typed errors. Tested at the trait level (no direct unit test
  here, but the in-memory contract and PgUserRepo error returns line
  up — flag for a `record_login_*` + `update` interaction test in
  commit 3).
- `soft_delete`: bumps `version` without an `expected_version`
  parameter; matches the trait signature. Could use a one-line
  comment on the deliberate omission of CAS (nit).
- `record_login_success`: clears `failed_login_count`,
  `locked_until`, refreshes `last_login_at`. Does NOT bump `version`
  with an inline comment explaining why — correct decision
  (Decision §2).
- `record_login_failure`: single statement with `CASE` on the
  post-increment value (`failed_login_count + 1 >= $2`). Atomic
  under PG `READ COMMITTED` row locking; the worker's "race window"
  concern (Decision §1) is overstated.
- `bind(lockout_secs as f64)` for `make_interval(secs => $3)` works
  but is slightly opaque — pure style nit.
- 5 `#[tokio::test]` cases covering: create+get roundtrip,
  case-insensitive email lookup, duplicate-email rejection,
  threshold-arming lockout, success-clears-failure-and-lock.

### `crates/storage/src/pg/session.rs`

- `SessionTuple` column order matches `SELECT_COLS`. The
  `ip_address::text AS ip_address` cast is necessary because sqlx
  has no built-in `Option<String> ↔ INET` mapping at workspace pin.
  The cast is IPv6-safe (PG `inet::text` returns canonical form).
- `create`: parameterised INSERT with `$6::inet` for the address —
  parses the bound text as INET. Invalid IP strings would surface at
  INSERT time as a typed PG error → `map_db_err` → `Connection`
  error variant. Acceptable contract; callers should pre-validate.
- `get`: honours `revoked_at IS NULL AND expires_at > NOW()` so the
  caller never has to re-check liveness — matches the trait doc.
- `touch`: same liveness predicate, no-op on dead rows.
- `revoke`: idempotent via the `revoked_at IS NULL` guard.
- `cleanup_expired`: returns `rows_affected()` per trait contract.
- 4 `#[tokio::test]` cases: roundtrip, duplicate-id rejection,
  revoke+get returns None idempotently, cleanup_expired isolates
  past vs live rows.

### `crates/storage/src/pg/pat.rs`

- `PatTuple` includes `Json<serde_json::Value>` for `scopes` —
  correct decoder for the JSONB column.
- `create`: 11-column INSERT, three `debug_assert!`s.
- `get_by_hash`: liveness filter is `revoked_at IS NULL AND
  (expires_at IS NULL OR expires_at > NOW())`. The `revoked_at IS
  NULL` matches the partial `idx_pat_hash` so the planner can use the
  index for O(log n) lookup, and the expiry check is then evaluated
  on the candidate row. Sound.
- `touch`: same liveness predicate.
- `revoke`: idempotent.
- `list_for_principal`: filters by `principal_kind + principal_id`
  with the same liveness predicate, ordered by `created_at`. Returns
  active tokens only — matches the in-memory contract.
- 4 `#[tokio::test]` cases.

### `crates/storage/src/pg/verification_token.rs`

- `TokenTuple` has `Option<Json<serde_json::Value>>` — payload is
  nullable JSONB.
- `create`: 7-column INSERT with three `debug_assert!`s.
- `consume_by_hash`: `UPDATE … WHERE token_hash = $1 AND consumed_at
  IS NULL AND expires_at > NOW() RETURNING …` — single-statement
  atomic consume; two concurrent consumers cannot both win.
  Replay-safe.
- `get_by_hash`: peek without consume, doc-flagged as test-only.
- `cleanup_expired`: `DELETE … WHERE expires_at <= NOW()`.
- `revoke_all_for_user(user_id, kind)`: `UPDATE … SET consumed_at =
  NOW() WHERE user_id = $1 AND kind = $2 AND consumed_at IS NULL`.
  The `kind` filter is the right shape for the realistic caller
  pattern ("invalidate all in-flight `password_reset` tokens after
  successful reset"). Returns `rows_affected()` per trait.
- 4 `#[tokio::test]` cases. **Nit:** no `cleanup_expired` test (the
  other repos have one).

### `crates/storage/src/pg/oauth_state.rs`

- `StateTuple` matches `SELECT_COLS`. TEXT PK on `state`.
- `create`: 7-column INSERT, three `debug_assert!`s.
- `consume_by_state`: same atomic `UPDATE … RETURNING` pattern as
  `consume_by_hash`. Replay defence is correct.
- `cleanup_expired`: `DELETE … WHERE expires_at <= NOW()`.
- `get_by_state`: peek-without-consume, doc-flagged as test helper.
- 4 `#[tokio::test]` cases.

### `crates/storage/src/repos/user.rs` (trait additions)

- `VerificationTokenRepo` (5 methods): `create`, `consume_by_hash`,
  `get_by_hash`, `cleanup_expired`, `revoke_all_for_user(kind)`.
  Matches the in-memory verify-email / complete-password-reset / MFA
  challenge usage. The `kind` parameter on `revoke_all_for_user` is
  the right scope (worker deviation §5 is sound).
- `OAuthStateRepo` (4 methods): `create`, `consume_by_state`,
  `cleanup_expired`, `get_by_state`. Sufficient for `start_oauth` +
  `complete_oauth` round-trip per in-memory backend at
  `in_memory.rs:539-577`.
- Both traits use `Send + Sync` + `impl Future<…> + Send` — same
  shape as the pre-existing `UserRepo` / `SessionRepo` / `PatRepo`.
- The three pre-existing traits are unmodified
  (`git diff 7c078e95..a8a949ea -- crates/storage/src/repos/user.rs`
  shows only additions after line 94).
- `repos/mod.rs` re-export is minimal:
  `pub use user::{OAuthStateRepo, PatRepo, SessionRepo, UserRepo,
  VerificationTokenRepo};`.
- `rows/mod.rs` re-export includes `OAuthStateRow` alongside the
  existing identity rows.

## Plan adherence

- **Scope creep:** None. `git show a8a949ea --name-only` touches only
  `crates/storage/`. `nebula-api` (`crates/api/`), `apps/`, the
  legacy `crates/storage/src/postgres/identity.rs`, and the in-memory
  adapter under `crates/storage/src/inmem/` are all unchanged
  (`git diff 7c078e95..a8a949ea -- crates/storage/src/postgres/
  crates/storage/src/inmem/ crates/api/ apps/` is empty).
- **Deliverables A-G (plan §PR2 commit 1):** all present:
  - A. Migration `0028_plane_a_oauth_state.sql` (PG + SQLite parity) ✓
  - B. `VerificationTokenRepo` + `OAuthStateRepo` trait definitions ✓
  - C. `PgUserRepo` ✓
  - D. `PgSessionRepo` ✓
  - E. `PgPatRepo` ✓
  - F. `PgVerificationTokenRepo` ✓
  - G. `PgOAuthStateRepo` ✓
- **`chore(docs)` commit `893a6e0f`:** five files, all under
  `docs/plans/` (plan, 2 recon files, 2 PR1 history files). No code
  changes.
- **Observability triple (per CLAUDE.md DoD), spot-check:**
  - `PgUserRepo::create`: typed `StorageError` ✓ + `#[instrument]` ✓ +
    `debug_assert!(!user.id.is_empty())` + `debug_assert!(!user.email
    .is_empty())` ✓.
  - `PgVerificationTokenRepo::consume_by_hash`: typed `StorageError`
    ✓ + `#[instrument]` ✓ + (no `debug_assert!` because input is a
    `&[u8]` not a row — file-level invariants are met by `create`).
  - `PgOAuthStateRepo::create`: typed `StorageError` ✓ +
    `#[instrument]` ✓ + three `debug_assert!`s on `state`,
    `provider`, `code_verifier` ✓.
  - `PgSessionRepo::create`: typed ✓ + `#[instrument]` ✓ +
    `debug_assert!` on `id` and `user_id` ✓.

## Recommendation

**LGTM → proceed to commit 2 (EmailPort + AuthBackendKind config).**

Optional cleanups that can land in commit 2 or as a small follow-up
commit on this branch (not blocking):

- Trim the "best-effort / REPEATABLE READ" framing on
  `pg/user.rs:14-19` so the project record reflects PG's actual
  default isolation.
- Add a one-line comment on `PgUserRepo::soft_delete` documenting the
  deliberate omission of CAS.
- Hoist the per-file `SCHEMA_READY` `OnceCell` + `pool()` boilerplate
  into a shared `test_support::pg::pool()` helper (also benefits the
  pre-existing `pg/control_queue.rs` test fixture).
- Add a `PgVerificationTokenRepo::cleanup_expired` test for symmetry
  with the other three cleanup-bearing repos.

None of these block commit 2. Storage foundation is solid; the next
commit can build the `EmailPort` trait and `AuthBackendKind` selector
on top of it without revisiting these files.
