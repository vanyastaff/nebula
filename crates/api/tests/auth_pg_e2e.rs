//! `PgAuthBackend` end-to-end lifecycle.
//!
//! Gated on `DATABASE_URL`: when the env var is absent the test no-ops
//! cleanly (same posture as every `crates/storage/src/pg/*::tests`
//! suite). When set, the test drives a complete identity lifecycle
//! through the production [`PgAuthBackend`] against a real Postgres:
//!
//! 1. `register_user` → durable user + verification email queued on a
//!    caller-owned `Arc<EchoSink>`.
//! 2. `verify_email` (consume the token) → `email_verified` flips true.
//! 3. `authenticate_password` (no MFA) → `PasswordOutcome::Authenticated`.
//! 4. `start_mfa_enrollment` + `confirm_mfa_enrollment` (with a fresh
//!    TOTP code).
//! 5. `authenticate_password` (MFA enabled, no totp) →
//!    `PasswordOutcome::MfaRequired` + challenge token.
//! 6. `verify_mfa` consumes the challenge and returns the profile.
//! 7. `create_pat` mints a PAT; `lookup_pat` round-trips by plaintext;
//!    `list_pats` returns it; `revoke_pat` hides it on subsequent
//!    lookups.
//! 8. `request_password_reset` → reset email queued.
//! 9. `complete_password_reset` (consume reset token, change password,
//!    revoke sibling reset tokens, bump version).
//! 10. `authenticate_password` with the new password succeeds.
//! 11. `start_oauth` persists a `plane_a_oauth_states` row.
//! 12. `complete_oauth` consumes the state atomically AND returns
//!     `NotImplemented` (provider code-exchange is not yet wired); a
//!     second `complete_oauth` against the same state surfaces
//!     `InvalidToken` (replay defence).
//!
//! The whole flow runs against ONE shared `Arc<dyn EmailPort>` so we
//! can assert that the verification, reset, and challenge mails reach
//! the caller-controlled echo sink (not some hidden default sink).
//!
//! ## Compile gating
//!
//! All bodies are `#[cfg(feature = "postgres")]` — when the feature
//! is off the file compiles to an empty crate and produces no tests.

#![cfg(feature = "postgres")]

use std::sync::Arc;

use nebula_api::{
    domain::auth::backend::{
        AuthBackend, CreatePatParams, PasswordOutcome, PgAuthBackend, SignupRequest,
        dto::SecretString, error::AuthError, mfa, oauth::OAuthProvider,
    },
    ports::email::{EchoSink, EmailKind, EmailPort},
};
use nebula_core::UserId;
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};

/// Mirrors `nebula_storage::pg::user::LOCKOUT_THRESHOLD` (the module is
/// `pub(crate)` so the constant is not re-exported from
/// `nebula-storage`). If the storage-side value changes from 5, this
/// constant must change with it; the lockout assertions below will
/// otherwise fail loudly (off-by-one between the test loop bound and
/// the storage-side CASE check) — which is the desired regression
/// signal. Source: `crates/storage/src/pg/user.rs:34`.
const LOCKOUT_THRESHOLD_LOCAL: u32 = 5;

// `sqlx::migrate!` resolves paths relative to the calling crate's
// `CARGO_MANIFEST_DIR` (`crates/api`); the production schema lives in
// the sibling storage crate.
static SPEC16_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../storage/migrations/postgres");
static SCHEMA_READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Connect to `DATABASE_URL`, apply the spec-16 migrations once per
/// test process, or return `None` to skip. Mirrors the
/// `crates/storage/src/pg/*::tests::pool` convention exactly.
async fn pool() -> Option<Pool<Postgres>> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => return None,
        Err(err) => panic!("DATABASE_URL is set but invalid: {err}"),
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    SCHEMA_READY
        .get_or_init(|| async {
            SPEC16_MIGRATOR
                .run(&pool)
                .await
                .expect("spec-16 postgres migrations");
        })
        .await;
    Some(pool)
}

/// Generate a unique-per-run email so re-runs against a persistent
/// Postgres do not collide on the `idx_users_email_active` unique index.
fn unique_email(label: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is sane")
        .as_nanos();
    format!("{label}-{nanos:x}@nebula-e2e.test")
}

/// Build a fresh backend wired to the caller's echo sink so the test
/// can introspect every delivered message against ONE known port.
///
/// `metrics: None` keeps the existing lifecycle test orthogonal to the
/// `nebula_api_auth_*` emission seam; the dedicated
/// `auth_metrics.rs` test exercises the metrics path against the same
/// constructor.
fn build_backend(pool: Pool<Postgres>) -> (Arc<PgAuthBackend>, Arc<EchoSink>) {
    let sink = Arc::new(EchoSink::default());
    let port: Arc<dyn EmailPort> = Arc::clone(&sink) as _;
    (Arc::new(PgAuthBackend::new(pool, port, None)), sink)
}

fn signup_for(email: &str) -> SignupRequest {
    SignupRequest {
        email: email.to_owned(),
        password: SecretString::new("hunter22".to_owned()),
        display_name: "Pg E2E".to_owned(),
    }
}

#[tokio::test]
async fn pg_auth_backend_full_lifecycle() {
    let Some(pool) = pool().await else { return };
    let (backend, sink) = build_backend(pool);
    let email = unique_email("lifecycle");

    // ── 1. signup ─────────────────────────────────────────────────────
    let profile = backend
        .register_user(signup_for(&email))
        .await
        .expect("register_user");
    assert_eq!(profile.email, email);
    assert!(!profile.email_verified, "fresh user is unverified");
    assert!(!profile.mfa_enabled, "fresh user has no mfa");

    // Verification email landed on the caller-owned sink.
    let verification_token = {
        let drained = sink.drain();
        assert_eq!(
            drained.len(),
            1,
            "exactly one verification email must be delivered"
        );
        let msg = &drained[0];
        assert_eq!(msg.to, email);
        assert_eq!(msg.kind, EmailKind::Verification);
        msg.body.clone()
    };

    // ── 2. verify email ───────────────────────────────────────────────
    backend
        .verify_email(&verification_token)
        .await
        .expect("verify_email");
    let post_verify = backend
        .get_user_profile(&profile.user_id)
        .await
        .expect("get_user_profile");
    assert!(
        post_verify.email_verified,
        "verify_email must flip email_verified"
    );
    // Replay of the same token is rejected.
    let replay_err = backend
        .verify_email(&verification_token)
        .await
        .expect_err("verify_email replay must reject");
    assert!(matches!(replay_err, AuthError::InvalidToken));

    // ── 3. login without MFA ──────────────────────────────────────────
    match backend
        .authenticate_password(&email, "hunter22", None)
        .await
        .expect("authenticate_password (no mfa)")
    {
        PasswordOutcome::Authenticated(p) => assert_eq!(p.user_id, profile.user_id),
        PasswordOutcome::MfaRequired { .. } => panic!("MFA is not enabled yet"),
    }

    // ── 4. enroll + confirm MFA ───────────────────────────────────────
    let enrol = backend
        .start_mfa_enrollment(&profile.user_id)
        .await
        .expect("start_mfa_enrollment");
    let first_code = mfa::current_code(&enrol.secret_base32).expect("current_code");
    backend
        .confirm_mfa_enrollment(&profile.user_id, &first_code)
        .await
        .expect("confirm_mfa_enrollment");

    // ── 5. login with MFA required ────────────────────────────────────
    let challenge_token = match backend
        .authenticate_password(&email, "hunter22", None)
        .await
        .expect("authenticate_password (mfa required)")
    {
        PasswordOutcome::MfaRequired { challenge_token } => challenge_token,
        PasswordOutcome::Authenticated(_) => panic!("MFA should be required"),
    };

    // ── 6. complete MFA ───────────────────────────────────────────────
    let mfa_code = mfa::current_code(&enrol.secret_base32).expect("current_code");
    let mfa_profile = backend
        .verify_mfa(&challenge_token, &mfa_code)
        .await
        .expect("verify_mfa");
    assert_eq!(mfa_profile.user_id, profile.user_id);
    // Replay rejected — the challenge consumed atomically.
    let replay_err = backend
        .verify_mfa(&challenge_token, &mfa_code)
        .await
        .expect_err("challenge replay must reject");
    assert!(matches!(replay_err, AuthError::InvalidToken));

    // ── 7. PAT lifecycle ──────────────────────────────────────────────
    let minted = backend
        .create_pat(
            &profile.user_id,
            CreatePatParams {
                name: "e2e-cli".to_owned(),
                scopes: vec!["workflows:read".to_owned()],
                ttl_seconds: None,
            },
        )
        .await
        .expect("create_pat");
    assert!(minted.plaintext.starts_with("pat_"));
    let resolved = backend
        .lookup_pat(&minted.plaintext)
        .await
        .expect("lookup_pat")
        .expect("active pat resolves");
    assert_eq!(resolved.id, minted.record.id);
    assert_eq!(resolved.scopes, vec!["workflows:read".to_owned()]);

    let listed = backend
        .list_pats(&profile.user_id)
        .await
        .expect("list_pats");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, minted.record.id);

    backend
        .revoke_pat(&profile.user_id, &minted.record.id)
        .await
        .expect("revoke_pat");
    assert!(
        backend
            .lookup_pat(&minted.plaintext)
            .await
            .expect("lookup after revoke")
            .is_none(),
        "revoked pat must not surface"
    );
    let listed_after = backend
        .list_pats(&profile.user_id)
        .await
        .expect("list_pats after revoke");
    assert!(
        listed_after.is_empty(),
        "revoked pat must not appear in list_pats"
    );
    // Idempotent: a second `revoke_pat` for the same (still-revoked) PAT
    // must return Ok so DELETE /me/tokens/{pat} stays safe to retry. The
    // pre-fix path returned `UserNotFound` (404) here because the active
    // lookup hides revoked tokens; the ownership probe now distinguishes
    // "unknown PAT" from "already revoked".
    backend
        .revoke_pat(&profile.user_id, &minted.record.id)
        .await
        .expect("revoke_pat is idempotent for an already-revoked token");
    // Unknown PAT (never created) still surfaces UserNotFound — the
    // idempotency carve-out is scoped to rows that exist for this user.
    let unknown_err = backend
        .revoke_pat(&profile.user_id, "pat_0000000000000000000000")
        .await
        .expect_err("revoking an unknown PAT must remain UserNotFound");
    assert!(matches!(unknown_err, AuthError::UserNotFound));

    // ── 8. forgot password ────────────────────────────────────────────
    // Issue TWO live reset tokens so the sibling-revoke contract is
    // exercised: completing the reset with `reset_token_a` must atomically
    // burn `reset_token_b` inside the same tx so a stolen second link
    // cannot be replayed after a successful password change.
    backend
        .request_password_reset(&email)
        .await
        .expect("request_password_reset (a)");
    backend
        .request_password_reset(&email)
        .await
        .expect("request_password_reset (b)");
    let (reset_token_a, reset_token_b) = {
        let drained = sink.drain();
        assert_eq!(
            drained.len(),
            2,
            "two password reset emails must be delivered (one per request)"
        );
        for msg in &drained {
            assert_eq!(msg.to, email);
            assert_eq!(msg.kind, EmailKind::PasswordReset);
        }
        (drained[0].body.clone(), drained[1].body.clone())
    };
    assert_ne!(
        reset_token_a, reset_token_b,
        "each request_password_reset must mint a fresh plaintext"
    );
    // Forgot-password is enumeration-safe: an unknown email returns Ok
    // without sending a message.
    backend
        .request_password_reset("nobody@nowhere.test")
        .await
        .expect("request_password_reset on unknown email is silent ok");
    assert!(
        sink.peek().is_empty(),
        "no email must be sent for an unknown address"
    );

    // ── 9. complete password reset ────────────────────────────────────
    backend
        .complete_password_reset(&reset_token_a, "newpass2")
        .await
        .expect("complete_password_reset");
    // Replay of the consumed reset token is rejected (atomic single-shot).
    let replay_err = backend
        .complete_password_reset(&reset_token_a, "newpass2")
        .await
        .expect_err("reset token replay must reject");
    assert!(matches!(replay_err, AuthError::InvalidToken));
    // Sibling-revoke: the OTHER reset token minted before the successful
    // reset must now also be rejected, even though it was never the one
    // the user clicked. This is the security regression the contract
    // guards against — a stolen second link cannot be cashed in after
    // the legitimate reset.
    let sibling_err = backend
        .complete_password_reset(&reset_token_b, "newpass-other")
        .await
        .expect_err("sibling reset token must be invalidated by the first reset");
    assert!(
        matches!(sibling_err, AuthError::InvalidToken),
        "sibling reset token must surface InvalidToken, got: {sibling_err:?}"
    );

    // ── 10. re-login with the new password ────────────────────────────
    // After a password reset, MFA stays enabled — the second factor
    // requires a fresh challenge token.
    let post_reset_challenge = match backend
        .authenticate_password(&email, "newpass2", None)
        .await
        .expect("authenticate_password with new password")
    {
        PasswordOutcome::MfaRequired { challenge_token } => challenge_token,
        PasswordOutcome::Authenticated(_) => panic!("MFA still required after reset"),
    };
    let new_code = mfa::current_code(&enrol.secret_base32).expect("current_code");
    let post_reset_profile = backend
        .verify_mfa(&post_reset_challenge, &new_code)
        .await
        .expect("verify_mfa post-reset");
    assert_eq!(post_reset_profile.user_id, profile.user_id);
    // The old password no longer works.
    let old_pw_err = backend
        .authenticate_password(&email, "hunter22", None)
        .await
        .expect_err("old password must be rejected after reset");
    assert!(matches!(old_pw_err, AuthError::InvalidCredentials));

    // ── 11. start OAuth (persists row) ────────────────────────────────
    let oauth_start = backend
        .start_oauth(
            OAuthProvider::Google,
            "https://nebula.test/auth/oauth/google/callback",
        )
        .await
        .expect("start_oauth");
    assert!(oauth_start.authorize_url.contains("state="));
    assert!(!oauth_start.state.is_empty());

    // ── 12. complete OAuth ────────────────────────────────────────────
    // Provider configs (client_id / client_secret) are deferred to a
    // follow-up — the PG path consumes the state row atomically
    // (replay defence) and then returns NotImplemented.
    let not_impl = backend
        .complete_oauth(
            OAuthProvider::Google,
            &oauth_start.state,
            "fake-code",
            "https://nebula.test/auth/oauth/google/callback",
        )
        .await
        .expect_err("complete_oauth must return NotImplemented");
    assert!(
        matches!(not_impl, AuthError::NotImplemented(_)),
        "expected NotImplemented, got: {not_impl:?}"
    );
    // Replay: the row was consumed by the first call, so a second one
    // returns InvalidToken (atomic single-shot).
    let replay_err = backend
        .complete_oauth(
            OAuthProvider::Google,
            &oauth_start.state,
            "fake-code",
            "https://nebula.test/auth/oauth/google/callback",
        )
        .await
        .expect_err("oauth state replay must reject");
    assert!(matches!(replay_err, AuthError::InvalidToken));

    // ── final invariants ──────────────────────────────────────────────
    // Profile reflects the post-reset world: still verified, MFA on,
    // same user_id.
    let final_profile = backend
        .get_user_profile(&profile.user_id)
        .await
        .expect("get_user_profile final");
    assert_eq!(final_profile.user_id, profile.user_id);
    assert!(final_profile.email_verified);
    assert!(final_profile.mfa_enabled);
}

#[tokio::test]
async fn pg_auth_backend_session_round_trip() {
    let Some(pool) = pool().await else { return };
    let (backend, _sink) = build_backend(pool);
    let email = unique_email("session");

    let profile = backend
        .register_user(signup_for(&email))
        .await
        .expect("register_user");

    let session = backend
        .create_session(&profile.user_id)
        .await
        .expect("create_session");
    assert!(!session.id.is_empty());
    assert!(!session.csrf_token.is_empty());

    // The middleware-facing resolver returns the principal for a live
    // session.
    let principal = backend
        .get_principal_by_session(&session.id)
        .await
        .expect("get_principal_by_session")
        .expect("session is live");
    assert!(matches!(principal, nebula_core::Principal::User(_)));

    backend
        .revoke_session(&session.id)
        .await
        .expect("revoke_session");
    let resolved_after_revoke = backend
        .get_principal_by_session(&session.id)
        .await
        .expect("get_principal_by_session post-revoke");
    assert!(
        resolved_after_revoke.is_none(),
        "revoked session must not resolve a principal"
    );
    // Revoke is idempotent.
    backend
        .revoke_session(&session.id)
        .await
        .expect("idempotent revoke_session");
}

#[tokio::test]
async fn pg_auth_backend_duplicate_signup_is_email_already_registered() {
    let Some(pool) = pool().await else { return };
    let (backend, _sink) = build_backend(pool);
    let email = unique_email("dup");

    backend
        .register_user(signup_for(&email))
        .await
        .expect("first register");
    let err = backend
        .register_user(signup_for(&email))
        .await
        .expect_err("second register must reject");
    assert!(matches!(err, AuthError::EmailAlreadyRegistered));
}

/// Regression: a valid `password_reset` token sent to `verify_mfa`
/// MUST NOT be burned by a blind consume. The kind-aware atomic
/// consume on the verification-token repo leaves the row untouched
/// for the legitimate `complete_password_reset` follow-up.
#[tokio::test]
async fn pg_auth_backend_verify_mfa_does_not_burn_password_reset_token() {
    let Some(pool) = pool().await else { return };
    let (backend, sink) = build_backend(pool);
    let email = unique_email("kind-guard");

    backend
        .register_user(signup_for(&email))
        .await
        .expect("register_user");
    // Discard the email-verification mail so the next drain only sees
    // the password-reset token under test.
    let _ = sink.drain();

    backend
        .request_password_reset(&email)
        .await
        .expect("request_password_reset");
    let reset_token = {
        let drained = sink.drain();
        assert_eq!(
            drained.len(),
            1,
            "exactly one password reset email must be delivered"
        );
        drained[0].body.clone()
    };

    // Send the reset token to `verify_mfa` (the wrong route). Pre-fix
    // this would atomically consume the row and then bail on the
    // in-memory kind check, leaving the legitimate password reset
    // unable to complete. Post-fix the kind-aware UPDATE skips the row,
    // returns `InvalidToken` without touching it, and the real reset
    // route still works.
    let wrong_route_err = backend
        .verify_mfa(&reset_token, "000000")
        .await
        .expect_err("reset token at verify_mfa must reject");
    assert!(matches!(wrong_route_err, AuthError::InvalidToken));

    backend
        .complete_password_reset(&reset_token, "newpass3")
        .await
        .expect("reset token must still be consumable on the correct route");
}

/// Regression: an OAuth state minted for one provider sent to
/// `complete_oauth` under a different provider MUST NOT consume the
/// row. The provider-aware atomic consume leaves the state available
/// for the legitimate callback at the right provider.
#[tokio::test]
async fn pg_auth_backend_complete_oauth_does_not_burn_cross_provider_state() {
    let Some(pool) = pool().await else { return };
    let (backend, _sink) = build_backend(pool);

    let start = backend
        .start_oauth(
            OAuthProvider::Google,
            "https://nebula.test/auth/oauth/google/callback",
        )
        .await
        .expect("start_oauth");

    // Wrong provider — must not consume the row.
    let wrong_provider_err = backend
        .complete_oauth(
            OAuthProvider::GitHub,
            &start.state,
            "fake-code",
            "https://nebula.test/auth/oauth/github/callback",
        )
        .await
        .expect_err("cross-provider state must reject");
    assert!(matches!(wrong_provider_err, AuthError::InvalidToken));

    // Correct provider — the state row is still consumable.
    let correct_err = backend
        .complete_oauth(
            OAuthProvider::Google,
            &start.state,
            "fake-code",
            "https://nebula.test/auth/oauth/google/callback",
        )
        .await
        .expect_err("complete_oauth still returns NotImplemented after consume");
    assert!(
        matches!(correct_err, AuthError::NotImplemented(_)),
        "expected NotImplemented after successful consume, got: {correct_err:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// Lockout coverage (M3.1 follow-up)
//
// Storage-layer logic in `crates/storage/src/pg/user.rs`
// (`record_login_failure` arms `locked_until` once
// `failed_login_count + 1 >= LOCKOUT_THRESHOLD`) and
// `PgAuthBackend::authenticate_password` returns `AuthError::AccountLocked`
// when `users.locked_until > NOW()`. Both surfaces have unit-test
// coverage in their owning crates; the three tests below drive the
// end-to-end flow through the backend trait so a future refactor that
// breaks the wiring is caught against the production code path.
//
// The current rate-limit middleware (`crates/api/src/middleware/rate_limit.rs`)
// is per-IP only; a per-account dimension is explicitly out of scope
// of this follow-up — the lockout column is the auth-aware throttle
// surface for 1.0.
// ─────────────────────────────────────────────────────────────────────

/// Convert the `UserProfile::user_id` String back into the 16-byte
/// payload that the `users` PG table indexes on.
fn user_id_bytes(user_id: &str) -> [u8; 16] {
    user_id
        .parse::<UserId>()
        .expect("user_id must round-trip through ULID parser")
        .as_bytes()
}

/// Run an authenticated `register_user` + `verify_email` flow so the
/// account is in the same state every lockout test starts from.
async fn verified_user(
    backend: &Arc<PgAuthBackend>,
    sink: &Arc<EchoSink>,
    email: &str,
) -> nebula_api::domain::auth::backend::UserProfile {
    let profile = backend
        .register_user(signup_for(email))
        .await
        .expect("register_user");
    let verification_token = {
        let drained = sink.drain();
        assert_eq!(
            drained.len(),
            1,
            "register_user must enqueue exactly one verification email"
        );
        assert_eq!(drained[0].kind, EmailKind::Verification);
        drained[0].body.clone()
    };
    backend
        .verify_email(&verification_token)
        .await
        .expect("verify_email");
    profile
}

/// Read `(failed_login_count, locked_until)` directly from the row so
/// the test can assert the storage-layer state independent of the
/// backend's read path.
async fn lockout_state(
    pool: &Pool<Postgres>,
    user_id: &str,
) -> (i32, Option<chrono::DateTime<chrono::Utc>>) {
    let id_bytes = user_id_bytes(user_id);
    sqlx::query_as::<_, (i32, Option<chrono::DateTime<chrono::Utc>>)>(
        "SELECT failed_login_count, locked_until FROM users WHERE id = $1",
    )
    .bind(&id_bytes[..])
    .fetch_one(pool)
    .await
    .expect("SELECT lockout state")
}

/// Force-expire the lockout window without sleeping for 15 minutes.
/// Mirrors what a future operator unlock RPC would do, but the test
/// owns the manipulation explicitly so the production code path stays
/// untouched. Asserts the row exists so a typo'd id fails loudly.
///
/// Uses `INTERVAL '1 hour'` instead of `'1 second'` so a PG-vs-test
/// process clock skew >1 s (common in containerised CI) cannot leave
/// the row still locked when the test next probes (#751 Copilot review).
async fn expire_lockout(pool: &Pool<Postgres>, user_id: &str) {
    let id_bytes = user_id_bytes(user_id);
    let rows = sqlx::query(
        "UPDATE users \
             SET locked_until = NOW() - INTERVAL '1 hour' \
             WHERE id = $1 AND locked_until IS NOT NULL",
    )
    .bind(&id_bytes[..])
    .execute(pool)
    .await
    .expect("UPDATE locked_until")
    .rows_affected();
    assert_eq!(
        rows, 1,
        "expire_lockout must touch exactly one armed lock; got {rows}"
    );
}

/// Lockout fires once `record_login_failure` crosses
/// `LOCKOUT_THRESHOLD`. The Nth wrong-password attempt still returns
/// `InvalidCredentials` (the lock arms on the same call but the
/// pre-check guard already let it through); the (N+1)th attempt
/// observes the armed lock and short-circuits to `AccountLocked`.
#[tokio::test]
async fn pg_auth_backend_locks_after_threshold_failures() {
    let Some(pool) = pool().await else { return };
    let (backend, sink) = build_backend(pool.clone());
    let email = unique_email("lockout");
    let profile = verified_user(&backend, &sink, &email).await;

    // Drive N wrong-password attempts. Each is `InvalidCredentials`;
    // the Nth call records the failure that arms `locked_until`.
    for attempt in 1..=LOCKOUT_THRESHOLD_LOCAL {
        let err = backend
            .authenticate_password(&email, "wrong-password", None)
            .await
            .expect_err("wrong password must reject before lockout");
        assert!(
            matches!(err, AuthError::InvalidCredentials),
            "attempt {attempt}/{LOCKOUT_THRESHOLD_LOCAL} must surface InvalidCredentials, got: {err:?}"
        );
    }

    // Storage row reflects the armed lockout immediately.
    let (count_at_lock, locked_until) = lockout_state(&pool, &profile.user_id).await;
    assert_eq!(
        count_at_lock, LOCKOUT_THRESHOLD_LOCAL as i32,
        "failed_login_count must equal the threshold after N rejected attempts"
    );
    let lock_deadline = locked_until.expect("locked_until must be armed");
    // Allow 5 s of negative skew: PG and the test process can disagree on
    // wall-clock time by ~1 s in containerised CI; treating the lock as
    // armed if `lock_deadline` is at least "recent" past gives the test
    // headroom without weakening the assertion (LOCKOUT_DURATION is 15 min
    // so a 5 s skew is still ~0.5% of the window) (#751 Copilot review).
    assert!(
        lock_deadline > chrono::Utc::now() - chrono::Duration::seconds(5),
        "locked_until ({lock_deadline}) must be in the future immediately after the Nth failure"
    );

    // The (N+1)th attempt — even with the CORRECT password — must be
    // rejected with AccountLocked, NOT InvalidCredentials. This is the
    // pre-password lockout guard at `pg.rs:authenticate_password`.
    let locked_err = backend
        .authenticate_password(&email, "hunter22", None)
        .await
        .expect_err("correct password during lockout window must reject");
    assert!(
        matches!(locked_err, AuthError::AccountLocked),
        "locked account must surface AccountLocked, got: {locked_err:?}"
    );

    // The wrong password during the lockout window also surfaces
    // AccountLocked — proves the guard runs before the password check
    // (no `record_login_failure` side-effect; counter does not
    // increment past the threshold).
    let wrong_during_lock = backend
        .authenticate_password(&email, "still-wrong", None)
        .await
        .expect_err("wrong password during lockout window must reject");
    assert!(
        matches!(wrong_during_lock, AuthError::AccountLocked),
        "wrong password during lockout must surface AccountLocked, got: {wrong_during_lock:?}"
    );
    let (count_after_locked_attempts, _) = lockout_state(&pool, &profile.user_id).await;
    assert_eq!(
        count_after_locked_attempts, LOCKOUT_THRESHOLD_LOCAL as i32,
        "failed_login_count must not grow once the lockout pre-check fires"
    );
}

/// Once the lockout window expires, a correct-password login must
/// succeed and `record_login_success` must clear both the failure
/// counter and `locked_until`.
#[tokio::test]
async fn pg_auth_backend_lockout_window_expiry_allows_login() {
    let Some(pool) = pool().await else { return };
    let (backend, sink) = build_backend(pool.clone());
    let email = unique_email("lockout-expire");
    let profile = verified_user(&backend, &sink, &email).await;

    // Drive the account into the locked state.
    for _ in 0..LOCKOUT_THRESHOLD_LOCAL {
        let _ = backend
            .authenticate_password(&email, "wrong-password", None)
            .await
            .expect_err("wrong password must reject");
    }
    let (_, locked_until) = lockout_state(&pool, &profile.user_id).await;
    assert!(
        locked_until.is_some(),
        "sanity: account must be locked before we expire the window"
    );

    // Force-expire the window without waiting 15 minutes.
    expire_lockout(&pool, &profile.user_id).await;

    // Correct password during the expired window succeeds. The user
    // has no MFA enabled, so the outcome is Authenticated.
    match backend
        .authenticate_password(&email, "hunter22", None)
        .await
        .expect("authenticate_password after lockout expiry")
    {
        PasswordOutcome::Authenticated(p) => {
            assert_eq!(p.user_id, profile.user_id);
        },
        PasswordOutcome::MfaRequired { .. } => {
            panic!("MFA is not enabled on the lockout fixture")
        },
    }

    // record_login_success cleared both columns.
    let (final_count, final_lock) = lockout_state(&pool, &profile.user_id).await;
    assert_eq!(
        final_count, 0,
        "successful auth must reset failed_login_count to 0"
    );
    assert!(
        final_lock.is_none(),
        "successful auth must clear locked_until, got: {final_lock:?}"
    );
}

/// Negative control: below-threshold failures followed by a single
/// success must clear the counter without ever arming the lock. This
/// covers the `record_login_success` side-effect on the happy path
/// and proves the threshold check is `>=` not `>`.
#[tokio::test]
async fn pg_auth_backend_success_clears_subthreshold_failures() {
    let Some(pool) = pool().await else { return };
    let (backend, sink) = build_backend(pool.clone());
    let email = unique_email("lockout-clears");
    let profile = verified_user(&backend, &sink, &email).await;

    // Drive THRESHOLD - 1 wrong-password attempts. Each is
    // InvalidCredentials; the lock must NOT arm.
    let below_threshold = LOCKOUT_THRESHOLD_LOCAL - 1;
    for attempt in 1..=below_threshold {
        let err = backend
            .authenticate_password(&email, "wrong-password", None)
            .await
            .expect_err("sub-threshold wrong password must reject");
        assert!(
            matches!(err, AuthError::InvalidCredentials),
            "sub-threshold attempt {attempt} must surface InvalidCredentials, got: {err:?}"
        );
    }
    let (mid_count, mid_lock) = lockout_state(&pool, &profile.user_id).await;
    assert_eq!(
        mid_count, below_threshold as i32,
        "failed_login_count must reflect each below-threshold failure"
    );
    assert!(
        mid_lock.is_none(),
        "sub-threshold failures must never arm locked_until, got: {mid_lock:?}"
    );

    // One successful authentication wipes the counter.
    match backend
        .authenticate_password(&email, "hunter22", None)
        .await
        .expect("correct password below threshold must succeed")
    {
        PasswordOutcome::Authenticated(p) => assert_eq!(p.user_id, profile.user_id),
        PasswordOutcome::MfaRequired { .. } => panic!("MFA is not enabled"),
    }
    let (after_count, after_lock) = lockout_state(&pool, &profile.user_id).await;
    assert_eq!(
        after_count, 0,
        "successful auth must reset failed_login_count, got: {after_count}"
    );
    assert!(
        after_lock.is_none(),
        "successful auth must keep locked_until null when never armed, got: {after_lock:?}"
    );
}
