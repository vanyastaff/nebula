# PR2 commit 1 (storage layer) — Worker Report

**Branch:** feat/api-pg-auth-backend
**Worktree:** .worktrees/pg-auth-backend
**Base:** 7c078e95 (post-#737 merge)
**Commits added:** 893a6e0f (chore(docs)), a8a949ea (feat(storage))

## Summary

Storage layer for the production PG-backed `AuthBackend` is in place.
Two new repository traits land (`VerificationTokenRepo`, `OAuthStateRepo`),
one new row type (`OAuthStateRow`), one new migration with
PG + SQLite parity (`0028_plane_a_oauth_state.sql`), and five
Postgres impls — `PgUserRepo`, `PgSessionRepo`, `PgPatRepo`,
`PgVerificationTokenRepo`, `PgOAuthStateRepo` — all backing the
existing identity tables from migrations 0001 + 0002 plus the new
Plane-A state table. The three pre-existing trait definitions in
`crates/storage/src/repos/user.rs` (`UserRepo`, `SessionRepo`,
`PatRepo`) are left untouched per the recon; the legacy
`PgUserStore` at `crates/storage/src/postgres/identity.rs` and the
in-memory adapter under `crates/storage/src/inmem/` are out of
scope as required. Every PG impl returns typed `StorageError`,
funnels every sqlx call through `map_db_err`, carries
`#[tracing::instrument(level = "debug", ...)]` spans, and asserts
input invariants with `debug_assert!`. DATABASE_URL-gated tests
provide happy-path + duplicate coverage (21 cases total) and
compile cleanly on `--features postgres,sqlite`.

## Files changed (`git diff --stat HEAD~2..HEAD`)

```
 .../postgres/0028_plane_a_oauth_state.sql          |  29 ++
 .../migrations/sqlite/0028_plane_a_oauth_state.sql |  22 +
 crates/storage/src/pg/mod.rs                       |  10 +
 crates/storage/src/pg/oauth_state.rs               | 276 +++++++++++++
 crates/storage/src/pg/pat.rs                       | 330 +++++++++++++++
 crates/storage/src/pg/session.rs                   | 297 ++++++++++++++
 crates/storage/src/pg/user.rs                      | 403 ++++++++++++++++++
 crates/storage/src/pg/verification_token.rs        | 315 ++++++++++++++
 crates/storage/src/repos/mod.rs                    |   2 +-
 crates/storage/src/repos/user.rs                   |  83 +++-
 crates/storage/src/rows/mod.rs                     |   4 +-
 crates/storage/src/rows/user.rs                    |  24 ++
 .../2026-05-25-002-feat-api-m3-closure-plan.md     | 455 +++++++++++++++++++++
 docs/plans/recon/m3-api-auth-state.md              | 386 +++++++++++++++++
 docs/plans/recon/m3-otlp-state.md                  | 300 ++++++++++++++
 docs/plans/recon/pr1-reviewer-report.md            | 327 +++++++++++++++
 docs/plans/recon/pr1-worker-report.md              | 147 +++++++
 17 files changed, 3407 insertions(+), 3 deletions(-)
```

Code-only delta (excluding the chore(docs) commit) is 1792 LOC across
12 files — within the §PR2 commit 1 ~860 LOC estimate from the plan
once the inline DATABASE_URL-gated test code (~900 LOC) is counted
alongside the impls.

## Verification

- `cargo fmt -p nebula-storage -- --check`: **pass**
- `cargo clippy -p nebula-storage --all-targets -- -D warnings`: **pass** (default features)
- `cargo clippy -p nebula-storage --all-targets --features postgres -- -D warnings`: **pass** (after one fix: `Duration::from_secs(15 * 60)` → `Duration::from_mins(15)` per `clippy::duration_suboptimal_units`)
- `cargo clippy -p nebula-storage --all-targets --features postgres,sqlite -- -D warnings`: **pass**
- `cargo nextest run -p nebula-storage --features postgres`: **285 tests run, 285 passed, 0 skipped, 0 failed** (the 21 new DATABASE_URL-gated tests return early when `DATABASE_URL` is unset; they show as `PASS` because `let Some(pool) = pool().await else { return };` short-circuits on the missing-env path)
- `RUSTDOCFLAGS=\"-D warnings\" cargo doc -p nebula-storage --no-deps`: **pass** (no warnings, default features)
- `RUSTDOCFLAGS=\"-D warnings\" cargo doc -p nebula-storage --no-deps --features postgres`: **pass** (no warnings)

The 21 newly registered tests are:

```
pg::oauth_state::tests::{cleanup_expired_deletes_only_past_rows,
                         consume_by_state_is_single_shot,
                         create_get_roundtrip,
                         duplicate_state_is_rejected}
pg::pat::tests::{create_get_by_hash_roundtrip,
                 duplicate_id_is_rejected,
                 list_for_principal_returns_only_active_tokens,
                 revoke_hides_token_from_lookup}
pg::session::tests::{cleanup_expired_deletes_only_past_rows,
                     create_get_roundtrip,
                     duplicate_id_is_rejected,
                     revoke_then_get_returns_none_idempotent}
pg::user::tests::{create_get_roundtrip,
                  duplicate_email_among_active_users_is_rejected,
                  get_by_email_is_case_insensitive,
                  record_login_failure_increments_then_locks,
                  record_login_success_clears_failures_and_lock}
pg::verification_token::tests::{consume_by_hash_is_single_shot,
                                create_get_roundtrip,
                                duplicate_hash_is_rejected,
                                revoke_all_for_user_only_touches_unconsumed_of_kind}
```

## Deviations from plan

1. **Lockout policy constants surfaced as `pub const`** — `PgUserRepo`
   exposes `LOCKOUT_THRESHOLD: i32 = 5` and `LOCKOUT_DURATION:
   Duration = Duration::from_mins(15)` at the module level so the
   tests can assert against the same numbers the impl uses and so a
   future shared-config refactor has a clean seam to hoist them onto.
   This is additive — not contradicting the plan, which only requires
   that the lockout columns be honoured.

2. **`record_login_*` do NOT bump `users.version`.** Inline comment in
   `pg/user.rs::record_login_success` explains: these are auth-time
   bookkeeping updates, not domain mutations, and bumping `version`
   here would cause spurious CAS conflicts for concurrent
   `UserRepo::update` callers (profile patches, password resets, etc.).
   This matches the trait-method contract (`UserRepo::update` is the
   only CAS-protected method) but is worth flagging for the reviewer.

3. **`pg::user` re-exported as `pub(crate) mod user`, not `mod user`.**
   The session/pat/verification_token DATABASE_URL-gated tests seed a
   user via `crate::pg::user::PgUserRepo` so they can satisfy the
   `user_id` FK on `sessions` / `personal_access_tokens` /
   `verification_tokens`. `pub(crate)` keeps the module reachable
   inside the crate without expanding the public surface beyond the
   already-exported `PgUserRepo` re-export at the `pg::` root. Other
   pg modules remain `mod`.

4. **`pg/session.rs` casts `ip_address::text`** in `SELECT_COLS` so the
   Postgres `INET` column projects directly into `Option<String>` (the
   shape `SessionRow` declares). On INSERT the bind uses `$6::inet` to
   parse the text back. No behaviour change versus the in-memory
   backend; just necessary because sqlx has no built-in
   `Option<String> ↔ INET` mapping at workspace pin (`sqlx 0.8` with
   default features).

5. **`PgVerificationTokenRepo::revoke_all_for_user` takes a `kind`
   filter.** The recon description left the parameter set unspecified;
   I added `kind: &str` because every realistic caller is
   kind-scoped (e.g. "revoke all unconsumed `password_reset` tokens
   for this user after they successfully reset their password"). A
   future caller that wants all kinds at once can call the method per
   kind without extra round trips matter.

6. **No `cleanup_expired` for `PgPatRepo`.** The trait does not define
   one (PATs use long-lived `expires_at IS NULL` or operator-set
   expiry; cleanup is a future ops concern). Matches the trait
   definition the recon documented.

7. **OAuth state table named `plane_a_oauth_states`** (not
   `oauth_states`), per the explicit task instruction — avoids
   colliding with the Plane-B credential pending-state surface
   (`pending_credentials` and friends) in the `0008_credentials.sql`
   family.

## Open questions for reviewer

1. **Lockout race window.** Two concurrent failed logins for the same
   user issue independent `UPDATE users SET failed_login_count =
   failed_login_count + 1, locked_until = CASE ...` statements; the
   per-row counter is best-effort under postgres's default REPEATABLE
   READ default isolation. The threshold-arming `CASE` is evaluated on
   the post-increment value inside the same statement, so once the
   threshold is crossed by either racer `locked_until` is set. Is
   that the contract you want, or should this be wrapped in
   `SELECT ... FOR UPDATE` to make the counter strictly monotonic?
   (The in-memory backend has the same best-effort semantics, so I
   left it consistent.)

2. **`record_login_*` versioning policy.** Confirm the decision in
   Deviation §2 — should `record_login_success` bump `users.version`
   so subscribers/CAS-protected mutations see a fresh version after a
   login event? My read says no; flagging in case there's a contract
   I missed.

3. **`OAuthStateRow.code_verifier` is plaintext TEXT.** The recon
   suggested this could be encrypted; I kept it as TEXT because the
   row is single-shot (consumed atomically on callback) and lives at
   most ~10 minutes. Encrypting would require pulling
   `nebula-credential` master-key access into `nebula-storage` for
   one short-lived field. Flagging if you want this re-evaluated
   before commit 3 (PgAuthBackend façade) lands.

4. **No SQLite parity for the five new repos.** The plan and the task
   only ask for PG impls + migration parity (so SQLite stays
   schema-compatible for future work). Confirm that no SQLite repo is
   expected at this commit — the in-memory adapter and SQLite repos
   for these traits are listed as a "future increment" in the task
   instructions.

5. **`test_support::random_id` 16-byte BYTEA reuse for PAT hashes in
   tests.** The PAT `hash` column is conceptually a SHA-256 digest
   (32 bytes); test fixtures pad a `random_id` into a 32-byte buffer
   to match the column shape without doing real hashing. The repo
   code never re-hashes — `get_by_hash` is a byte-equality lookup —
   so this is harmless for round-trip coverage. Flagging in case the
   reviewer wants a real SHA-256 helper landed alongside.

## Next

Ready for fresh reviewer on commits 893a6e0f..a8a949ea
(`feat/api-pg-auth-backend`). PR2 commit 2 (EmailPort + AuthBackendKind
config) and commit 3 (PgAuthBackend façade + composition root) follow
under the same branch per the plan, but should land only after this
storage layer is signed off.
