use std::sync::Arc;

use std::future::Future;

use chrono::{Duration, Utc};
use sqlx::postgres::PgPoolOptions;
use tokio::sync::Barrier;

use super::*;
use crate::{
    StorageError,
    pg::{PgExternalIdentityRepo, PgSessionRepo, PgUserRepo},
    repos::{
        ExternalIdentityRepo, OAuthLoginFinalizeCommand, OAuthLoginFinalizeOutcome,
        OAuthLoginFinalized, OAuthLoginMfaChallengeDraft, OAuthLoginSessionDraft,
        OAuthLoginUserDraft, SessionRepo, UserRepo,
    },
    rows::SessionDraft,
    test_support::{random_id, test_user},
};

static SPEC16_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
static SCHEMA_READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn pool() -> Option<Pool<Postgres>> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => return None,
        Err(err) => panic!("DATABASE_URL is set but invalid: {err}"),
    };
    // A private schema, not `public`. Two of these tests must seed a row
    // that migration 0038's CHECK constraint forbids, which means dropping
    // that constraint for the length of the seed — safe only when the
    // schema is this module's own.
    let schema = format!("nebula_oauth_login_{}", std::process::id());
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect");
    for statement in [
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&admin)
            .await
            .expect("create this module's private schema");
    }
    admin.close().await;

    let search_path = format!("SET search_path TO {schema}");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _meta| {
            let search_path = search_path.clone();
            Box::pin(async move {
                sqlx::query(sqlx::AssertSqlSafe(search_path))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .expect("connect");
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

/// Drop a named `users` CHECK constraint for the duration of one seed.
///
/// Migration 0038 added constraints that make malformed identity rows
/// unreachable through ordinary writes — and thereby unreachable for the
/// tests whose subject is what the finalizer does when it *encounters* one.
/// Those states are still reachable in the field: a database adopted from
/// before 0038 carries rows the constraints never validated, so the
/// finalizer's fail-closed behaviour is defence in depth that has to keep
/// working.
///
/// These tests own a private schema, so the constraint they drop is their
/// own copy and no concurrent test can observe the window. It is restored
/// `NOT VALID` because the seeded row is exactly what it would reject —
/// the constraint must guard future writes without retroactively failing
/// the fixture it was suspended for.
async fn without_users_check<Seed, Fut>(
    pool: &Pool<Postgres>,
    constraint: &str,
    definition: &str,
    seed: Seed,
) where
    Seed: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE users DROP CONSTRAINT {constraint}"
    )))
    .execute(pool)
    .await
    .expect("the 0038 constraint exists to be dropped");
    seed().await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE users ADD CONSTRAINT {constraint} CHECK ({definition}) NOT VALID"
    )))
    .execute(pool)
    .await
    .expect("restore the constraint for the rest of the schema");
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", hex::encode(&random_id()[..6]))
}

fn command(
    provider: &str,
    subject: &str,
    verified_email: Option<&str>,
) -> OAuthLoginFinalizeCommand {
    let now = Utc::now();
    OAuthLoginFinalizeCommand {
        provider: provider.to_owned(),
        subject: subject.to_owned(),
        verified_email: verified_email.map(str::to_owned),
        candidate_user: OAuthLoginUserDraft {
            id: random_id(),
            display_name: "OAuth test user".to_owned(),
            avatar_url: None,
            created_at: now,
        },
        session: OAuthLoginSessionDraft {
            token: random_id(),
            created_at: now,
            last_active_at: now,
            expires_at: now + Duration::hours(2),
            ip_address: Some("192.0.2.42".to_owned()),
            user_agent: Some("nebula-storage-oauth-test/1.0".to_owned()),
        },
        mfa_challenge: OAuthLoginMfaChallengeDraft {
            token_hash: random_id()
                .repeat(2)
                .try_into()
                .expect("32-byte challenge hash"),
            created_at: now,
            expires_at: now + Duration::minutes(5),
        },
    }
}

fn finalized(outcome: OAuthLoginFinalizeOutcome) -> OAuthLoginFinalized {
    match outcome {
        OAuthLoginFinalizeOutcome::Finalized(finalized) => *finalized,
        _ => panic!("expected finalized OAuth login"),
    }
}

#[test]
fn user_insert_converges_on_the_active_email_index() {
    let sql = INSERT_CANDIDATE_USER_SQL
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(sql.contains("ON CONFLICT (LOWER(email)) WHERE deleted_at IS NULL DO NOTHING"));
}

#[test]
fn identity_insert_converges_without_rebinding_the_winner() {
    let sql = INSERT_EXTERNAL_IDENTITY_SQL
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(sql.contains("ON CONFLICT (provider, subject) DO NOTHING RETURNING user_id"));
    assert!(!sql.contains("DO UPDATE"));
}

#[test]
fn command_validation_rejects_noncanonical_or_oversized_identity_inputs() {
    let valid = command("google", "subject-1", Some("alice@example.test"));
    assert!(validate_common_command(&valid).is_ok());

    let invalid_emails = [
        "Alice@example.test".to_owned(),
        " alice@example.test".to_owned(),
        format!("{}@example.test", "a".repeat(MAX_EMAIL_BYTES)),
    ];
    for email in invalid_emails {
        let invalid = command("google", "subject-1", Some(&email));
        assert!(validate_common_command(&invalid).is_err());
    }

    let invalid_provider = command("Google", "subject-1", Some("alice@example.test"));
    assert!(validate_common_command(&invalid_provider).is_err());
    let oversized_subject = "s".repeat(MAX_SUBJECT_BYTES + 1);
    let invalid_subject = command("google", &oversized_subject, Some("alice@example.test"));
    assert!(validate_common_command(&invalid_subject).is_err());
    let mut invalid_user_id = command("google", "subject-1", Some("alice@example.test"));
    invalid_user_id.candidate_user.id.pop();
    assert!(validate_common_command(&invalid_user_id).is_err());
    let mut zero_challenge = command("google", "subject-1", Some("alice@example.test"));
    zero_challenge.mfa_challenge.token_hash = [0; 32];
    assert!(validate_common_command(&zero_challenge).is_err());
}

#[tokio::test]
async fn same_subject_and_email_race_converges_to_one_user_and_link() {
    let Some(pool) = pool().await else { return };
    let provider = unique("same-subject-provider");
    let subject = unique("same-subject");
    let email = format!("{}@example.test", unique("same-subject"));
    let left_command = command(&provider, &subject, Some(&email));
    let right_command = command(&provider, &subject, Some(&email));
    let left_session_token = left_command.session.token.clone();
    let right_session_token = right_command.session.token.clone();
    let left_session_digest = session_token_digest(&left_session_token);
    let right_session_digest = session_token_digest(&right_session_token);
    let left_challenge_hash = left_command.mfa_challenge.token_hash;
    let right_challenge_hash = right_command.mfa_challenge.token_hash;
    let barrier = Arc::new(Barrier::new(2));
    let left_barrier = Arc::clone(&barrier);
    let right_barrier = Arc::clone(&barrier);
    let left_finalizer = PgOAuthLoginFinalizer::new(pool.clone());
    let right_finalizer = left_finalizer.clone();

    let (left, right) = tokio::join!(
        async move {
            left_barrier.wait().await;
            left_finalizer.finalize(left_command).await
        },
        async move {
            right_barrier.wait().await;
            right_finalizer.finalize(right_command).await
        }
    );
    let left = finalized(left.expect("left finalization"));
    let right = finalized(right.expect("right finalization"));

    assert_eq!(left.user.id, right.user.id);
    assert!(!left.user.mfa_enabled);
    assert!(!right.user.mfa_enabled);
    assert_eq!(left.session_token, left_session_token);
    assert_eq!(right.session_token, right_session_token);
    let user_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE LOWER(email) = LOWER($1) AND deleted_at IS NULL",
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .expect("count users");
    assert_eq!(user_count, 1);
    let linked_user: Vec<u8> = sqlx::query_scalar(
        "SELECT user_id FROM external_identities WHERE provider = $1 AND subject = $2",
    )
    .bind(&provider)
    .bind(&subject)
    .fetch_one(&pool)
    .await
    .expect("linked user");
    assert_eq!(linked_user, left.user.id);
    let session_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions WHERE token_digest = $1 OR token_digest = $2",
    )
    .bind(left_session_digest.as_bytes().as_slice())
    .bind(right_session_digest.as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .expect("count sessions");
    assert_eq!(session_count, 2);
    let challenge_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM verification_tokens WHERE token_hash = $1 OR token_hash = $2",
    )
    .bind(left_challenge_hash.as_slice())
    .bind(right_challenge_hash.as_slice())
    .fetch_one(&pool)
    .await
    .expect("count unused MFA challenges");
    assert_eq!(challenge_count, 0);
}

#[tokio::test]
async fn different_subjects_with_the_same_email_require_explicit_linking() {
    let Some(pool) = pool().await else { return };
    let provider = unique("shared-email-provider");
    let left_subject = unique("left-subject");
    let right_subject = unique("right-subject");
    let email = format!("{}@example.test", unique("shared-email"));
    let left_command = command(&provider, &left_subject, Some(&email));
    let right_command = command(&provider, &right_subject, Some(&email));
    let left_session_token = left_command.session.token.clone();
    let right_session_token = right_command.session.token.clone();
    let left_session_digest = session_token_digest(&left_session_token);
    let right_session_digest = session_token_digest(&right_session_token);
    let barrier = Arc::new(Barrier::new(2));
    let left_barrier = Arc::clone(&barrier);
    let right_barrier = Arc::clone(&barrier);
    let left_finalizer = PgOAuthLoginFinalizer::new(pool.clone());
    let right_finalizer = left_finalizer.clone();

    let (left, right) = tokio::join!(
        async move {
            left_barrier.wait().await;
            left_finalizer.finalize(left_command).await
        },
        async move {
            right_barrier.wait().await;
            right_finalizer.finalize(right_command).await
        }
    );
    let left = left.expect("left finalization");
    let right = right.expect("right finalization");
    let finalized = match (left, right) {
        (
            OAuthLoginFinalizeOutcome::Finalized(finalized),
            OAuthLoginFinalizeOutcome::AccountLinkRequired,
        )
        | (
            OAuthLoginFinalizeOutcome::AccountLinkRequired,
            OAuthLoginFinalizeOutcome::Finalized(finalized),
        ) => *finalized,
        _ => panic!("exactly one subject must claim an unowned email"),
    };

    let user_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE LOWER(email) = LOWER($1) AND deleted_at IS NULL",
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .expect("count users");
    assert_eq!(user_count, 1);
    let link_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM external_identities WHERE provider = $1 AND subject IN ($2, $3)",
    )
    .bind(&provider)
    .bind(&left_subject)
    .bind(&right_subject)
    .fetch_one(&pool)
    .await
    .expect("count links");
    assert_eq!(link_count, 1);
    let session_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions WHERE token_digest = $1 OR token_digest = $2",
    )
    .bind(left_session_digest.as_bytes().as_slice())
    .bind(right_session_digest.as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .expect("count sessions");
    assert_eq!(session_count, 1);
    assert!(
        finalized.session_token == left_session_token
            || finalized.session_token == right_session_token
    );
}

#[tokio::test]
async fn same_subject_with_different_emails_removes_the_losing_candidate() {
    let Some(pool) = pool().await else { return };
    let provider = unique("different-email-provider");
    let subject = unique("different-email-subject");
    let left_email = format!("{}@example.test", unique("left-email"));
    let right_email = format!("{}@example.test", unique("right-email"));
    let left_command = command(&provider, &subject, Some(&left_email));
    let right_command = command(&provider, &subject, Some(&right_email));
    let barrier = Arc::new(Barrier::new(2));
    let left_barrier = Arc::clone(&barrier);
    let right_barrier = Arc::clone(&barrier);
    let left_finalizer = PgOAuthLoginFinalizer::new(pool.clone());
    let right_finalizer = left_finalizer.clone();

    let (left, right) = tokio::join!(
        async move {
            left_barrier.wait().await;
            left_finalizer.finalize(left_command).await
        },
        async move {
            right_barrier.wait().await;
            right_finalizer.finalize(right_command).await
        }
    );
    let left = finalized(left.expect("left finalization"));
    let right = finalized(right.expect("right finalization"));

    assert_eq!(left.user.id, right.user.id);
    let candidate_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE (LOWER(email) = LOWER($1) OR LOWER(email) = LOWER($2)) AND deleted_at IS NULL",
    )
    .bind(&left_email)
    .bind(&right_email)
    .fetch_one(&pool)
    .await
    .expect("count candidate users");
    assert_eq!(candidate_count, 1);
}

#[tokio::test]
async fn existing_link_does_not_require_email_and_keeps_its_original_user() {
    let Some(pool) = pool().await else { return };
    let provider = unique("existing-link-provider");
    let subject = unique("existing-link-subject");
    let mut user = test_user(&format!("{}@example.test", unique("existing-link")));
    user.email_verified_at = Some(Utc::now());
    PgUserRepo::new(pool.clone())
        .create(&user)
        .await
        .expect("seed user");
    PgExternalIdentityRepo::new(pool.clone())
        .link_external(&user.id, &provider, &subject, Some(&user.email))
        .await
        .expect("seed identity link");

    let command = command(&provider, &subject, None);
    let session_token = command.session.token.clone();
    let outcome = PgOAuthLoginFinalizer::new(pool)
        .finalize(command)
        .await
        .expect("finalize existing link");
    let finalized = finalized(outcome);

    assert_eq!(finalized.user.id, user.id);
    assert_eq!(finalized.session_token, session_token);
}

#[tokio::test]
async fn existing_link_with_mfa_never_creates_a_session() {
    let Some(pool) = pool().await else { return };
    let provider = unique("existing-mfa-link-provider");
    let subject = unique("existing-mfa-link-subject");
    let mut user = test_user(&format!("{}@example.test", unique("existing-mfa-link")));
    user.email_verified_at = Some(Utc::now());
    user.mfa_enabled = true;
    user.mfa_secret_envelope = Some(b"test-envelope".to_vec());
    PgUserRepo::new(pool.clone())
        .create(&user)
        .await
        .expect("seed MFA user");
    PgExternalIdentityRepo::new(pool.clone())
        .link_external(&user.id, &provider, &subject, Some(&user.email))
        .await
        .expect("seed identity link");
    let command = command(&provider, &subject, None);
    let session_digest = session_token_digest(&command.session.token);
    let challenge_hash = command.mfa_challenge.token_hash;

    let outcome = PgOAuthLoginFinalizer::new(pool.clone())
        .finalize(command)
        .await
        .expect("finalize existing MFA link");

    assert!(matches!(outcome, OAuthLoginFinalizeOutcome::MfaRequired));
    let session_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
            .bind(session_digest.as_bytes().as_slice())
            .fetch_one(&pool)
            .await
            .expect("count sessions");
    assert_eq!(session_count, 0);
    let challenge: (Vec<u8>, Vec<u8>, String, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
        "SELECT token_hash, user_id, kind, consumed_at \
             FROM verification_tokens WHERE token_hash = $1",
    )
    .bind(challenge_hash.as_slice())
    .fetch_one(&pool)
    .await
    .expect("load MFA challenge");
    assert_eq!(challenge.0, challenge_hash);
    assert_eq!(challenge.1, user.id);
    assert_eq!(challenge.2, "mfa_challenge");
    assert!(challenge.3.is_none());
}

#[tokio::test]
async fn linked_mfa_user_without_a_secret_rolls_back_every_authority_artifact() {
    let Some(pool) = pool().await else { return };

    for (case, secret) in [("missing", None), ("empty", Some(Vec::new()))] {
        let provider = unique(&format!("invalid-mfa-{case}-provider"));
        let subject = unique(&format!("invalid-mfa-{case}-subject"));
        let mut user = test_user(&format!(
            "{}@example.test",
            unique(&format!("invalid-mfa-{case}"))
        ));
        user.email_verified_at = Some(Utc::now());
        user.mfa_enabled = true;
        user.mfa_secret_envelope = secret;
        // The `empty` case is a zero-byte envelope, which 0038's bounds
        // check forbids — the very shape an adopted pre-0038 database can
        // still hold, and the shape this test exists to prove the
        // finalizer refuses.
        without_users_check(
            &pool,
            "chk_users_mfa_secret_envelope_bounds",
            "mfa_secret_envelope IS NULL OR octet_length(mfa_secret_envelope) BETWEEN 1 AND 4096",
            || async {
                PgUserRepo::new(pool.clone())
                    .create(&user)
                    .await
                    .expect("seed invalid MFA user");
            },
        )
        .await;
        PgExternalIdentityRepo::new(pool.clone())
            .link_external(&user.id, &provider, &subject, Some(&user.email))
            .await
            .expect("seed identity link");
        let command = command(&provider, &subject, None);
        let session_digest = session_token_digest(&command.session.token);
        let challenge_hash = command.mfa_challenge.token_hash;

        let result = PgOAuthLoginFinalizer::new(pool.clone())
            .finalize(command)
            .await;

        assert!(matches!(result, Err(StorageError::Internal(_))));
        let session_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
                .bind(session_digest.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count sessions");
        assert_eq!(session_count, 0, "{case} secret created a session");
        let challenge_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM verification_tokens WHERE token_hash = $1")
                .bind(challenge_hash.as_slice())
                .fetch_one(&pool)
                .await
                .expect("count challenges");
        assert_eq!(challenge_count, 0, "{case} secret created a challenge");
    }
}

#[tokio::test]
async fn mfa_challenge_hash_collision_fails_without_creating_a_session() {
    let Some(pool) = pool().await else { return };
    let provider = unique("challenge-collision-provider");
    let subject = unique("challenge-collision-subject");
    let mut user = test_user(&format!("{}@example.test", unique("challenge-collision")));
    user.email_verified_at = Some(Utc::now());
    user.mfa_enabled = true;
    user.mfa_secret_envelope = Some(b"test-envelope".to_vec());
    PgUserRepo::new(pool.clone())
        .create(&user)
        .await
        .expect("seed MFA user");
    PgExternalIdentityRepo::new(pool.clone())
        .link_external(&user.id, &provider, &subject, Some(&user.email))
        .await
        .expect("seed identity link");
    let command = command(&provider, &subject, None);
    let session_digest = session_token_digest(&command.session.token);
    let challenge_hash = command.mfa_challenge.token_hash;
    sqlx::query(
        "INSERT INTO verification_tokens \
         (token_hash, user_id, kind, payload, created_at, expires_at, consumed_at) \
         VALUES ($1, $2, 'mfa_challenge', NULL, NOW(), NOW() + INTERVAL '5 minutes', NULL)",
    )
    .bind(challenge_hash.as_slice())
    .bind(&user.id)
    .execute(&pool)
    .await
    .expect("seed colliding challenge hash");

    let result = PgOAuthLoginFinalizer::new(pool.clone())
        .finalize(command)
        .await;

    assert!(matches!(result, Err(StorageError::Internal(_))));
    let session_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
            .bind(session_digest.as_bytes().as_slice())
            .fetch_one(&pool)
            .await
            .expect("count sessions");
    assert_eq!(session_count, 0);
    let challenge_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM verification_tokens WHERE token_hash = $1")
            .bind(challenge_hash.as_slice())
            .fetch_one(&pool)
            .await
            .expect("count challenges");
    assert_eq!(
        challenge_count, 1,
        "collision must not overwrite the existing token"
    );
}

#[tokio::test]
async fn malformed_linked_user_id_is_rejected_before_session_commit() {
    let Some(pool) = pool().await else { return };
    let provider = unique("malformed-link-provider");
    let subject = unique("malformed-link-subject");
    let mut user = test_user(&format!("{}@example.test", unique("malformed-link")));
    user.id.pop();
    user.email_verified_at = Some(Utc::now());
    without_users_check(
        &pool,
        "chk_users_identity_id_length",
        "octet_length(id) = 16",
        || async {
            PgUserRepo::new(pool.clone())
                .create(&user)
                .await
                .expect("seed malformed user");
        },
    )
    .await;
    PgExternalIdentityRepo::new(pool.clone())
        .link_external(&user.id, &provider, &subject, Some(&user.email))
        .await
        .expect("seed malformed identity link");
    let command = command(&provider, &subject, None);
    let session_digest = session_token_digest(&command.session.token);

    let result = PgOAuthLoginFinalizer::new(pool.clone())
        .finalize(command)
        .await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("malformed linked user id must fail closed"),
    };
    assert_eq!(
        error.to_string(),
        "internal: OAuth login finalization failed"
    );
    let session_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
            .bind(session_digest.as_bytes().as_slice())
            .fetch_one(&pool)
            .await
            .expect("count sessions");
    assert_eq!(session_count, 0);
}

#[tokio::test]
async fn soft_deleted_linked_user_is_not_rebound_by_email() {
    let Some(pool) = pool().await else { return };
    let provider = unique("deleted-link-provider");
    let subject = unique("deleted-link-subject");
    let mut user = test_user(&format!("{}@example.test", unique("deleted-link")));
    user.email_verified_at = Some(Utc::now());
    let users = PgUserRepo::new(pool.clone());
    users.create(&user).await.expect("seed user");
    PgExternalIdentityRepo::new(pool.clone())
        .link_external(&user.id, &provider, &subject, Some(&user.email))
        .await
        .expect("seed identity link");
    users.soft_delete(&user.id).await.expect("soft delete user");
    let command = command(&provider, &subject, Some("replacement@example.test"));
    let session_digest = session_token_digest(&command.session.token);

    let outcome = PgOAuthLoginFinalizer::new(pool.clone())
        .finalize(command)
        .await
        .expect("semantic rejection");
    assert!(matches!(
        outcome,
        OAuthLoginFinalizeOutcome::LinkedUserUnavailable
    ));
    let session_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
            .bind(session_digest.as_bytes().as_slice())
            .fetch_one(&pool)
            .await
            .expect("count sessions");
    assert_eq!(session_count, 0);
    let replacement_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE LOWER(email) = LOWER($1) AND deleted_at IS NULL",
    )
    .bind("replacement@example.test")
    .fetch_one(&pool)
    .await
    .expect("count replacement users");
    assert_eq!(replacement_count, 0);
}

#[tokio::test]
async fn existing_local_email_requires_explicit_linking_without_writes() {
    let Some(pool) = pool().await else { return };
    let provider = unique("unverified-provider");
    let subject = unique("unverified-subject");
    let mut user = test_user(&format!("{}@example.test", unique("existing-email")));
    user.email_verified_at = Some(Utc::now());
    PgUserRepo::new(pool.clone())
        .create(&user)
        .await
        .expect("seed user");
    let command = command(&provider, &subject, Some(&user.email));
    let session_digest = session_token_digest(&command.session.token);

    let outcome = PgOAuthLoginFinalizer::new(pool.clone())
        .finalize(command)
        .await
        .expect("semantic rejection");
    assert!(matches!(
        outcome,
        OAuthLoginFinalizeOutcome::AccountLinkRequired
    ));
    let link_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM external_identities WHERE provider = $1 AND subject = $2",
    )
    .bind(&provider)
    .bind(&subject)
    .fetch_one(&pool)
    .await
    .expect("count links");
    assert_eq!(link_count, 0);
    let session_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
            .bind(session_digest.as_bytes().as_slice())
            .fetch_one(&pool)
            .await
            .expect("count sessions");
    assert_eq!(session_count, 0);
}

#[tokio::test]
async fn session_failure_rolls_back_new_user_and_identity_link() {
    let Some(pool) = pool().await else { return };
    let provider = unique("rollback-provider");
    let subject = unique("rollback-subject");
    let email = format!("{}@example.test", unique("rollback"));
    let mut seed_user = test_user(&format!("{}@example.test", unique("session-owner")));
    seed_user.email_verified_at = Some(Utc::now());
    PgUserRepo::new(pool.clone())
        .create(&seed_user)
        .await
        .expect("seed session owner");
    let now = Utc::now();
    let duplicate_session_token = random_id();
    PgSessionRepo::new(pool.clone())
        .create(
            &duplicate_session_token,
            &SessionDraft {
                user_id: seed_user.id.clone(),
                created_at: now,
                last_active_at: now,
                expires_at: now + Duration::hours(1),
                ip_address: None,
                user_agent: None,
                revoked_at: None,
            },
        )
        .await
        .expect("seed colliding session");
    let mut command = command(&provider, &subject, Some(&email));
    let candidate_user_id = command.candidate_user.id.clone();
    command.session.token = duplicate_session_token;

    let result = PgOAuthLoginFinalizer::new(pool.clone())
        .finalize(command)
        .await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("session collision must fail the transaction"),
    };
    assert!(matches!(&error, StorageError::Internal(_)));
    assert_eq!(
        error.to_string(),
        "internal: OAuth login finalization failed"
    );
    let candidate_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1")
        .bind(&candidate_user_id)
        .fetch_one(&pool)
        .await
        .expect("count candidate user");
    assert_eq!(candidate_count, 0);
    let email_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE LOWER(email) = LOWER($1) AND deleted_at IS NULL",
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .expect("count candidate email");
    assert_eq!(email_count, 0);
    let link_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM external_identities WHERE provider = $1 AND subject = $2",
    )
    .bind(&provider)
    .bind(&subject)
    .fetch_one(&pool)
    .await
    .expect("count links");
    assert_eq!(link_count, 0);
}
