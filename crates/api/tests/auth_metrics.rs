//! `nebula_api_auth_*` emission seam coverage.
//!
//! Validates the locked-spec metrics wired in
//! `crates/api/src/domain/auth/backend/{pg,in_memory}.rs` against the
//! closed `auth_outcome` and `auth_oauth_provider` label sets defined in
//! [`nebula_metrics::naming`].
//!
//! Two complementary suites live here:
//!
//! - The [`memory_backend`] tests run **without** `DATABASE_URL` so the
//!   emission seam is exercised in CI on every PR, even when no Postgres
//!   is reachable. They cover the `outcome=success` and
//!   `outcome=invalid_creds` paths against [`InMemoryAuthBackend`] per
//!   the oracle locked-spec risk item 5 ("label-key drift catcher
//!   regardless of DATABASE_URL availability").
//! - The [`pg_backend`] tests are `#[cfg(feature = "postgres")]` and
//!   silently no-op when `DATABASE_URL` is absent (mirrors the
//!   `auth_pg_e2e.rs` gating pattern). When the env var is set they
//!   exercise the same closed-set against [`PgAuthBackend`] using a
//!   live Postgres.

#![cfg(test)]

use std::sync::Arc;

use nebula_api::{
    domain::auth::backend::{
        AuthBackend, InMemoryAuthBackend, PasswordOutcome, SignupRequest, dto::SecretString,
        error::AuthError,
    },
    ports::email::{EchoSink, EmailPort},
};
use nebula_metrics::{
    MetricsRegistry,
    naming::{NEBULA_API_AUTH_ATTEMPTS_TOTAL, NEBULA_API_AUTH_DURATION_SECONDS, auth_outcome},
};

/// Bump-then-snapshot helper: returns the labeled counter's current
/// value, instantiating it on demand. Used to assert the seam wired the
/// closed-set `outcome` label correctly.
fn counter_value(registry: &MetricsRegistry, name: &str, outcome: &'static str) -> u64 {
    let labels = registry.interner().single("outcome", outcome);
    registry
        .counter_labeled(name, &labels)
        .expect("counter_labeled lookup")
        .get()
}

/// Same for the histogram — returns the sample count.
fn histogram_count(registry: &MetricsRegistry, outcome: &'static str) -> usize {
    let labels = registry.interner().single("outcome", outcome);
    registry
        .histogram_labeled(NEBULA_API_AUTH_DURATION_SECONDS, &labels)
        .expect("histogram_labeled lookup")
        .count()
}

fn signup_for(email: &str) -> SignupRequest {
    SignupRequest {
        email: email.to_owned(),
        password: SecretString::new("hunter22".to_owned()),
        display_name: "Auth Metrics".to_owned(),
    }
}

/// Build an `InMemoryAuthBackend` with metrics wired through a fresh
/// `MetricsRegistry`. The caller keeps the registry handle so it can
/// assert against post-call counter / histogram state.
fn build_in_memory_with_metrics() -> (InMemoryAuthBackend, Arc<MetricsRegistry>) {
    let registry = Arc::new(MetricsRegistry::new());
    let sink = Arc::new(EchoSink::default());
    let port: Arc<dyn EmailPort> = Arc::clone(&sink) as _;
    let backend = InMemoryAuthBackend::new()
        .with_email_port(port)
        .with_metrics(Some(Arc::clone(&registry)));
    (backend, registry)
}

mod memory_backend {
    //! `InMemoryAuthBackend` emission coverage (NOT `DATABASE_URL`-gated).
    //!
    //! The oracle locked-spec risk item 5 calls out that the
    //! production PG path is `DATABASE_URL`-gated, so a memory-backend
    //! test is required to catch label-key drift in CI on every PR
    //! regardless of Postgres availability.

    use super::*;

    /// `register_user` then `authenticate_password` (correct credentials)
    /// must each bump `attempts_total{outcome=success}` exactly once
    /// and observe the histogram twice (one per method).
    #[tokio::test]
    async fn success_path_increments_success_outcome() {
        let (backend, registry) = build_in_memory_with_metrics();
        let email = "memory-success@nebula-test.example";

        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::SUCCESS
            ),
            0,
            "fresh registry: success counter must start at zero"
        );

        let profile = backend
            .register_user(signup_for(email))
            .await
            .expect("register_user");
        assert_eq!(profile.email, email);

        // After register: one success bump + one histogram sample.
        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::SUCCESS
            ),
            1,
            "register_user must increment outcome=success"
        );
        assert_eq!(
            histogram_count(&registry, auth_outcome::SUCCESS),
            1,
            "register_user must observe duration histogram"
        );

        match backend
            .authenticate_password(email, "hunter22", None)
            .await
            .expect("authenticate_password")
        {
            PasswordOutcome::Authenticated(p) => assert_eq!(p.user_id, profile.user_id),
            PasswordOutcome::MfaRequired { .. } => panic!("MFA not enabled"),
        }

        // After authenticate (success): success counter == 2 and
        // histogram count == 2.
        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::SUCCESS
            ),
            2,
            "authenticate_password (ok) must increment outcome=success"
        );
        assert_eq!(
            histogram_count(&registry, auth_outcome::SUCCESS),
            2,
            "authenticate_password must observe duration histogram"
        );
        // And no spurious bumps on other outcomes.
        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::INVALID_CREDS,
            ),
            0,
            "no invalid_creds bump on the happy path"
        );
    }

    /// `authenticate_password` with a wrong password must bump
    /// `attempts_total{outcome=invalid_creds}` (NOT `success`) and add
    /// to the `outcome=invalid_creds` histogram bucket.
    #[tokio::test]
    async fn invalid_password_increments_invalid_creds_outcome() {
        let (backend, registry) = build_in_memory_with_metrics();
        let email = "memory-invalid@nebula-test.example";

        backend
            .register_user(signup_for(email))
            .await
            .expect("register_user");

        // Drain the success-from-register counter so the assertion
        // below reads the post-failed-login delta only.
        let success_after_register = counter_value(
            &registry,
            NEBULA_API_AUTH_ATTEMPTS_TOTAL,
            auth_outcome::SUCCESS,
        );
        assert_eq!(success_after_register, 1);
        let invalid_before = counter_value(
            &registry,
            NEBULA_API_AUTH_ATTEMPTS_TOTAL,
            auth_outcome::INVALID_CREDS,
        );

        let err = backend
            .authenticate_password(email, "wrong-password", None)
            .await
            .expect_err("wrong-password authenticate must fail");
        assert!(matches!(err, AuthError::InvalidCredentials));

        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::INVALID_CREDS
            ),
            invalid_before + 1,
            "wrong-password authenticate must increment outcome=invalid_creds"
        );
        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::SUCCESS
            ),
            success_after_register,
            "wrong-password authenticate must NOT increment outcome=success"
        );
        assert!(
            histogram_count(&registry, auth_outcome::INVALID_CREDS) >= 1,
            "wrong-password authenticate must observe duration histogram \
             under outcome=invalid_creds"
        );
    }

    /// `register_user` with a duplicate email must bump
    /// `attempts_total{outcome=conflict}` per the oracle per-method map
    /// (`EmailAlreadyRegistered -> conflict`).
    #[tokio::test]
    async fn duplicate_signup_increments_conflict_outcome() {
        let (backend, registry) = build_in_memory_with_metrics();
        let email = "memory-conflict@nebula-test.example";

        backend
            .register_user(signup_for(email))
            .await
            .expect("first register_user");

        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::CONFLICT
            ),
            0,
            "conflict counter must be zero before the duplicate signup"
        );

        let err = backend
            .register_user(signup_for(email))
            .await
            .expect_err("duplicate register_user must fail");
        assert!(matches!(err, AuthError::EmailAlreadyRegistered));

        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::CONFLICT
            ),
            1,
            "duplicate register_user must increment outcome=conflict"
        );
    }

    /// `None` metrics registry must not panic. The `IdempotencyLayer`
    /// precedent guarantees the `let Some(reg) = ... else { return; };`
    /// early-return pattern; the auth backends mirror this discipline
    /// so a backend constructed without metrics still runs the full
    /// flow.
    #[tokio::test]
    async fn no_metrics_registry_is_a_no_op_on_emission() {
        let backend = InMemoryAuthBackend::new();
        let email = "memory-no-metrics@nebula-test.example";

        backend
            .register_user(signup_for(email))
            .await
            .expect("register_user without metrics must succeed");
        match backend
            .authenticate_password(email, "hunter22", None)
            .await
            .expect("authenticate_password without metrics must succeed")
        {
            PasswordOutcome::Authenticated(_) => {},
            PasswordOutcome::MfaRequired { .. } => panic!("MFA not enabled"),
        }
    }
}

#[cfg(feature = "postgres")]
mod pg_backend {
    //! `PgAuthBackend` emission coverage.
    //!
    //! `DATABASE_URL`-gated. Mirrors `auth_pg_e2e.rs::pool` exactly:
    //! when the env var is absent the test no-ops; when set it runs
    //! against a live Postgres with the spec-16 migrations applied
    //! once per process.
    //!
    //! Asserts the same `outcome=success` and `outcome=invalid_creds`
    //! invariants as the memory-backend suite so a wire regression in
    //! the PG path (e.g. wrong counter constant, label-key typo) is
    //! caught by CI when the gated suite runs.

    use nebula_api::domain::auth::backend::PgAuthBackend;
    use sqlx::{Pool, Postgres, postgres::PgPoolOptions};

    use super::*;

    static SPEC16_MIGRATOR: sqlx::migrate::Migrator =
        sqlx::migrate!("../storage/migrations/postgres");
    static SCHEMA_READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

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
    /// Postgres do not collide on the `idx_users_email_active` unique
    /// index.
    fn unique_email(label: &str) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is sane")
            .as_nanos();
        format!("{label}-{nanos:x}@nebula-metrics.test")
    }

    fn build_pg_backend_with_metrics(
        pool: Pool<Postgres>,
    ) -> (Arc<PgAuthBackend>, Arc<MetricsRegistry>) {
        let registry = Arc::new(MetricsRegistry::new());
        let sink = Arc::new(EchoSink::default());
        let port: Arc<dyn EmailPort> = Arc::clone(&sink) as _;
        let backend = Arc::new(PgAuthBackend::new(pool, port, Some(Arc::clone(&registry))));
        (backend, registry)
    }

    /// `PgAuthBackend::register_user` + `authenticate_password` happy
    /// path must each bump `attempts_total{outcome=success}` and
    /// observe the duration histogram. Mirrors the
    /// `memory_backend::success_path_increments_success_outcome` test
    /// against the production PG path.
    #[tokio::test]
    async fn pg_success_path_increments_success_outcome() {
        let Some(pool) = pool().await else { return };
        let (backend, registry) = build_pg_backend_with_metrics(pool);
        let email = unique_email("pg-success");

        let profile = backend
            .register_user(signup_for(&email))
            .await
            .expect("register_user");

        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::SUCCESS
            ),
            1,
            "PgAuthBackend::register_user must increment outcome=success"
        );
        assert_eq!(
            histogram_count(&registry, auth_outcome::SUCCESS),
            1,
            "PgAuthBackend::register_user must observe duration histogram"
        );

        match backend
            .authenticate_password(&email, "hunter22", None)
            .await
            .expect("authenticate_password")
        {
            PasswordOutcome::Authenticated(p) => assert_eq!(p.user_id, profile.user_id),
            PasswordOutcome::MfaRequired { .. } => panic!("MFA not enabled"),
        }

        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::SUCCESS
            ),
            2,
            "PgAuthBackend::authenticate_password (ok) must increment outcome=success"
        );
    }

    /// Wrong-password against `PgAuthBackend` bumps
    /// `attempts_total{outcome=invalid_creds}` (NOT `success`).
    #[tokio::test]
    async fn pg_invalid_password_increments_invalid_creds_outcome() {
        let Some(pool) = pool().await else { return };
        let (backend, registry) = build_pg_backend_with_metrics(pool);
        let email = unique_email("pg-invalid");

        backend
            .register_user(signup_for(&email))
            .await
            .expect("register_user");

        let success_after_register = counter_value(
            &registry,
            NEBULA_API_AUTH_ATTEMPTS_TOTAL,
            auth_outcome::SUCCESS,
        );
        let invalid_before = counter_value(
            &registry,
            NEBULA_API_AUTH_ATTEMPTS_TOTAL,
            auth_outcome::INVALID_CREDS,
        );

        let err = backend
            .authenticate_password(&email, "wrong-password", None)
            .await
            .expect_err("wrong-password authenticate must fail");
        assert!(matches!(err, AuthError::InvalidCredentials));

        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::INVALID_CREDS,
            ),
            invalid_before + 1,
            "PgAuthBackend wrong-password authenticate must increment outcome=invalid_creds"
        );
        assert_eq!(
            counter_value(
                &registry,
                NEBULA_API_AUTH_ATTEMPTS_TOTAL,
                auth_outcome::SUCCESS
            ),
            success_after_register,
            "PgAuthBackend wrong-password authenticate must NOT bump outcome=success"
        );
    }
}
