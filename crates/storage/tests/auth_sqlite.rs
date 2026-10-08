//! SQLite account persistence through public storage contracts and file reopen.

#![cfg(feature = "sqlite")]

use chrono::{Duration, Utc};
use nebula_storage::{
    StorageError,
    auth::{
        AccountLifecycle, AccountTokenOutcome, PasswordRegistration, SessionDraft, SessionRepo,
        UserRepo, VerificationTokenRepo, VerificationTokenRow,
        sqlite::{
            SqliteAccountLifecycle, SqliteSessionRepo, SqliteUserRepo, SqliteVerificationTokenRepo,
        },
    },
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

async fn open(path: &std::path::Path) -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
                .foreign_keys(true)
                .busy_timeout(std::time::Duration::from_secs(2)),
        )
        .await
        .expect("SQLite deployment");
    nebula_storage::sqlite::init_schema(&pool)
        .await
        .expect("admitted schema");
    pool
}

async fn oauth_deployment(path: &std::path::Path) -> nebula_storage::sqlite::DeploymentPool {
    let deployment = nebula_storage::sqlite::DeploymentPool::connect(
        SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    nebula_storage::sqlite::init_schema(deployment.pool())
        .await
        .unwrap();
    deployment
}

async fn register(pool: &SqlitePool, id: u8, hash: u8) -> Result<(), StorageError> {
    SqliteAccountLifecycle::new(pool.clone())
        .register_password_user(&PasswordRegistration {
            user_id: &[id; 16],
            email: &format!("account-{id}@example.test"),
            display_name: "Account owner",
            password_hash: "original-hash",
            verification_hash: &[hash; 32],
            created_at: Utc::now(),
            verification_expires_at: Utc::now() + Duration::hours(1),
        })
        .await
}

async fn token(pool: &SqlitePool, user: u8, hash: u8, kind: &str) {
    SqliteVerificationTokenRepo::new(pool.clone())
        .create(&VerificationTokenRow {
            token_hash: vec![hash; 32],
            user_id: vec![user; 16],
            kind: kind.into(),
            payload: Some(serde_json::json!({"purpose": "test"})),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::hours(1),
            consumed_at: None,
        })
        .await
        .expect("token");
}

#[tokio::test]
async fn registration_is_atomic_and_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accounts.db");
    let pool = open(&path).await;
    register(&pool, 1, 1).await.unwrap();
    assert!(matches!(
        register(&pool, 2, 1).await,
        Err(StorageError::Duplicate {
            entity: "verification_token",
            ..
        })
    ));
    pool.close().await;
    let pool = open(&path).await;
    let users = SqliteUserRepo::new(pool.clone());
    assert!(
        users.get(&[2; 16]).await.unwrap().is_none(),
        "failed second insert rolled back user"
    );
    let user = users
        .get_by_email("ACCOUNT-1@EXAMPLE.TEST")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.id, vec![1; 16]);
    assert_eq!(user.password_hash.as_deref(), Some("original-hash"));
    let tokens = SqliteVerificationTokenRepo::new(pool.clone());
    assert_eq!(
        tokens.get_by_hash(&[1; 32]).await.unwrap().unwrap().user_id,
        vec![1; 16]
    );
    pool.close().await;
}

#[tokio::test]
async fn email_redemption_preserves_wrong_kind_and_archived_account_tokens() {
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .unwrap();
    register(&pool, 1, 1).await.unwrap();
    token(&pool, 1, 2, "password_reset").await;
    let accounts = SqliteAccountLifecycle::new(pool.clone());
    assert_eq!(
        accounts.verify_email(&[2; 32]).await.unwrap(),
        AccountTokenOutcome::InvalidToken
    );
    assert_eq!(
        accounts.verify_email(&[1; 32]).await.unwrap(),
        AccountTokenOutcome::Applied
    );
    assert_eq!(
        accounts.verify_email(&[1; 32]).await.unwrap(),
        AccountTokenOutcome::InvalidToken
    );
    register(&pool, 3, 3).await.unwrap();
    SqliteUserRepo::new(pool.clone())
        .soft_delete(&[3; 16])
        .await
        .unwrap();
    assert_eq!(
        accounts.verify_email(&[3; 32]).await.unwrap(),
        AccountTokenOutcome::UserUnavailable
    );
    let tokens = SqliteVerificationTokenRepo::new(pool.clone());
    assert!(
        tokens
            .get_by_hash(&[2; 32])
            .await
            .unwrap()
            .unwrap()
            .consumed_at
            .is_none()
    );
    assert!(tokens.get_by_hash(&[3; 32]).await.unwrap().is_none());
    let consumed: Option<i64> =
        sqlx::query_scalar("SELECT consumed_at FROM verification_tokens WHERE token_hash = ?")
            .bind([3_u8; 32].as_slice())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        consumed.is_none(),
        "archived account redemption rolled back token consumption"
    );
    pool.close().await;
}

#[tokio::test]
async fn sibling_password_resets_have_one_winner_across_independent_pools() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accounts.db");
    let first = open(&path).await;
    let second = open(&path).await;
    register(&first, 1, 1).await.unwrap();
    token(&first, 1, 2, "password_reset").await;
    token(&first, 1, 3, "password_reset").await;
    let a = SqliteAccountLifecycle::new(first.clone());
    let b = SqliteAccountLifecycle::new(second.clone());
    let (a, b) = tokio::join!(
        a.reset_password(&[2; 32], "first-hash"),
        b.reset_password(&[3; 32], "second-hash")
    );
    let outcomes = [a.unwrap(), b.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| **r == AccountTokenOutcome::Applied)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| **r == AccountTokenOutcome::InvalidToken)
            .count(),
        1
    );
    let user = SqliteUserRepo::new(first.clone())
        .get(&[1; 16])
        .await
        .unwrap()
        .unwrap();
    let expected = if outcomes[0] == AccountTokenOutcome::Applied {
        "first-hash"
    } else {
        "second-hash"
    };
    assert_eq!(user.password_hash.as_deref(), Some(expected));
    for hash in [2, 3] {
        assert!(
            SqliteVerificationTokenRepo::new(first.clone())
                .get_by_hash(&[hash; 32])
                .await
                .unwrap()
                .unwrap()
                .consumed_at
                .is_some()
        );
    }
    first.close().await;
    second.close().await;
}

#[tokio::test]
async fn session_lookup_uses_only_digest_and_persists_revocation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accounts.db");
    let pool = open(&path).await;
    register(&pool, 1, 1).await.unwrap();
    let now = Utc::now();
    let bearer = b"SESSION_BEARER_CANARY-72f0";
    SqliteSessionRepo::new(pool.clone())
        .create(
            bearer,
            &SessionDraft {
                user_id: vec![1; 16],
                created_at: now,
                last_active_at: now,
                expires_at: now + Duration::hours(1),
                ip_address: Some("2001:db8::1".into()),
                user_agent: Some("test".into()),
                revoked_at: None,
            },
        )
        .await
        .unwrap();
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_digest FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        stored,
        nebula_storage::auth::session_token::session_token_digest(bearer).as_bytes()
    );
    assert_ne!(stored, bearer);
    pool.close().await;
    let pool = open(&path).await;
    let sessions = SqliteSessionRepo::new(pool.clone());
    let session = sessions.get(bearer).await.unwrap().unwrap();
    assert_eq!(session.ip_address.as_deref(), Some("2001:db8::1"));
    assert!(sessions.get(b"wrong bearer").await.unwrap().is_none());
    sessions.revoke(bearer).await.unwrap();
    let revoked: i64 = sqlx::query_scalar("SELECT revoked_at FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    sessions.revoke(bearer).await.unwrap();
    sessions.touch(bearer).await.unwrap();
    assert_eq!(
        revoked,
        sqlx::query_scalar::<_, i64>("SELECT revoked_at FROM sessions")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    pool.close().await;
    let pool = open(&path).await;
    assert!(
        SqliteSessionRepo::new(pool.clone())
            .get(bearer)
            .await
            .unwrap()
            .is_none()
    );
    pool.close().await;
}

#[tokio::test]
async fn verification_token_consumption_is_kind_scoped_and_single_use() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accounts.db");
    let first = open(&path).await;
    let second = open(&path).await;
    register(&first, 1, 1).await.unwrap();
    token(&first, 1, 2, "mfa_challenge").await;
    let a = SqliteVerificationTokenRepo::new(first.clone());
    let b = SqliteVerificationTokenRepo::new(second.clone());
    assert!(
        a.consume_by_hash_and_kind(&[2; 32], "password_reset")
            .await
            .unwrap()
            .is_none()
    );
    let (x, y) = tokio::join!(
        a.consume_by_hash_and_kind(&[2; 32], "mfa_challenge"),
        b.consume_by_hash_and_kind(&[2; 32], "mfa_challenge")
    );
    assert_eq!(
        [x.unwrap(), y.unwrap()]
            .iter()
            .filter(|row| row.is_some())
            .count(),
        1
    );
    first.close().await;
    second.close().await;
}

fn identity_codec() -> std::sync::Arc<nebula_storage::auth::identity_secret::IdentitySecretCodec> {
    let key = nebula_storage::credential::EnvKeyProvider::from_base64(
        "ERERERERERERERERERERERERERERERERERERERERERE=",
    )
    .unwrap();
    std::sync::Arc::new(
        nebula_storage::auth::identity_secret::IdentitySecretCodec::new(std::sync::Arc::new(key))
            .unwrap(),
    )
}

#[tokio::test]
async fn mfa_installation_is_exact_single_use_and_reseals_for_active_purpose() {
    use nebula_storage::auth::{
        MfaEnrollmentCandidate, MfaEnrollmentInstallOutcome, MfaEnrollmentRepo,
        identity_secret::TotpSecretPurpose, sqlite::SqliteMfaEnrollmentRepo,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accounts.db");
    let first = open(&path).await;
    let second = open(&path).await;
    register(&first, 1, 1).await.unwrap();
    let codec = identity_codec();
    let a = SqliteMfaEnrollmentRepo::new(first.clone(), codec.clone());
    let b = SqliteMfaEnrollmentRepo::new(second.clone(), codec.clone());
    let seed = b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
    for id in [1, 2] {
        a.replace_candidate(
            &MfaEnrollmentCandidate::new(
                [id; 32],
                vec![1; 16],
                codec
                    .seal_totp_seed(TotpSecretPurpose::EnrollmentCandidate, &[1; 16], seed)
                    .unwrap(),
                Utc::now(),
                Utc::now() + Duration::minutes(10),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    }
    let users = SqliteUserRepo::new(first.clone());
    assert!(!users.get(&[1; 16]).await.unwrap().unwrap().mfa_enabled);
    assert_eq!(
        a.install_candidate(&[1; 16], &[1; 32]).await.unwrap(),
        MfaEnrollmentInstallOutcome::CandidateUnavailable
    );
    let (x, y) = tokio::join!(
        a.install_candidate(&[1; 16], &[2; 32]),
        b.install_candidate(&[1; 16], &[2; 32])
    );
    assert_eq!(
        [x.unwrap(), y.unwrap()]
            .iter()
            .filter(|outcome| **outcome == MfaEnrollmentInstallOutcome::Installed)
            .count(),
        1
    );
    assert!(a.get_live_candidate(&[1; 16]).await.unwrap().is_none());
    let user = users.get(&[1; 16]).await.unwrap().unwrap();
    assert!(user.mfa_enabled);
    let active = user.mfa_secret_envelope.unwrap();
    assert_eq!(
        codec
            .open_totp_seed(TotpSecretPurpose::Active, &[1; 16], &active)
            .unwrap()
            .plaintext
            .as_slice(),
        seed
    );
    assert!(
        codec
            .open_totp_seed(TotpSecretPurpose::EnrollmentCandidate, &[1; 16], &active)
            .is_err()
    );
    first.close().await;
    second.close().await;
    let reopened = open(&path).await;
    let user = SqliteUserRepo::new(reopened.clone())
        .get(&[1; 16])
        .await
        .unwrap()
        .unwrap();
    assert!(user.mfa_enabled);
    assert_eq!(user.mfa_secret_envelope.unwrap(), active);
    reopened.close().await;
}

#[tokio::test]
async fn rejected_mfa_owner_rolls_back_candidate_consumption() {
    use nebula_storage::auth::{
        MfaEnrollmentCandidate, MfaEnrollmentRepo, identity_secret::TotpSecretPurpose,
        sqlite::SqliteMfaEnrollmentRepo,
    };
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .unwrap();
    register(&pool, 1, 1).await.unwrap();
    let codec = identity_codec();
    let mfa = SqliteMfaEnrollmentRepo::new(pool.clone(), codec.clone());
    mfa.replace_candidate(
        &MfaEnrollmentCandidate::new(
            [1; 32],
            vec![1; 16],
            codec
                .seal_totp_seed(
                    TotpSecretPurpose::EnrollmentCandidate,
                    &[1; 16],
                    b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
                )
                .unwrap(),
            Utc::now(),
            Utc::now() + Duration::minutes(10),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    SqliteUserRepo::new(pool.clone())
        .soft_delete(&[1; 16])
        .await
        .unwrap();
    assert!(matches!(
        mfa.install_candidate(&[1; 16], &[1; 32]).await,
        Err(StorageError::NotFound { entity: "user", .. })
    ));
    assert!(mfa.get_live_candidate(&[1; 16]).await.unwrap().is_none());
    let retained: Vec<u8> =
        sqlx::query_scalar("SELECT enrollment_id FROM mfa_enrollment_candidates WHERE user_id = ?")
            .bind([1_u8; 16].as_slice())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        retained,
        vec![1; 32],
        "rejected installation retained the exact candidate"
    );
    pool.close().await;
}

#[tokio::test]
async fn pat_revocation_is_owner_qualified_and_excludes_expired_lookup() {
    use nebula_storage::auth::{PatRepo, PersonalAccessTokenRow, sqlite::SqlitePatRepo};
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .unwrap();
    let pats = SqlitePatRepo::new(pool.clone());
    for id in [1, 2] {
        pats.create(&PersonalAccessTokenRow {
            id: vec![id; 16],
            principal_kind: "user".into(),
            principal_id: vec![3; 16],
            name: "automation".into(),
            prefix: "nbl_pat".into(),
            hash: vec![id; 32],
            scopes: serde_json::json!(["read"]),
            created_at: Utc::now(),
            last_used_at: None,
            expires_at: if id == 1 {
                None
            } else {
                Some(Utc::now() - Duration::hours(1))
            },
            revoked_at: None,
        })
        .await
        .unwrap();
    }
    assert!(pats.get_by_hash(&[1; 32]).await.unwrap().is_some());
    assert!(pats.get_by_hash(&[2; 32]).await.unwrap().is_none());
    assert_eq!(
        pats.list_for_principal("user", &[3; 16])
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        !pats
            .revoke_for_principal(&[1; 16], "user", &[4; 16])
            .await
            .unwrap()
    );
    assert!(
        !pats
            .revoke_for_principal(&[1; 16], "service_account", &[3; 16])
            .await
            .unwrap()
    );
    assert!(pats.get_by_hash(&[1; 32]).await.unwrap().is_some());
    for id in [1, 2] {
        assert!(
            pats.revoke_for_principal(&[id; 16], "user", &[3; 16])
                .await
                .unwrap()
        );
        assert!(
            pats.revoke_for_principal(&[id; 16], "user", &[3; 16])
                .await
                .unwrap()
        );
    }
    assert!(
        pats.list_for_principal("user", &[3; 16])
            .await
            .unwrap()
            .is_empty()
    );
    pool.close().await;
}

#[tokio::test]
async fn user_cas_and_login_bookkeeping_keep_distinct_versions() {
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .unwrap();
    register(&pool, 1, 1).await.unwrap();
    let users = SqliteUserRepo::new(pool.clone());
    for _ in 0..5 {
        users.record_login_failure(&[1; 16]).await.unwrap();
    }
    let mut user = users.get(&[1; 16]).await.unwrap().unwrap();
    assert_eq!(user.failed_login_count, 5);
    assert!(user.locked_until.unwrap() > Utc::now());
    assert_eq!(user.version, 0);
    users.record_login_success(&[1; 16]).await.unwrap();
    user = users.get(&[1; 16]).await.unwrap().unwrap();
    assert_eq!(user.failed_login_count, 0);
    assert!(user.locked_until.is_none());
    assert_eq!(user.version, 0);
    user.display_name = "updated".into();
    users.update(&user, 0).await.unwrap();
    assert!(matches!(
        users.update(&user, 0).await,
        Err(StorageError::Conflict {
            actual: 1,
            expected: 0,
            ..
        })
    ));
    users.soft_delete(&[1; 16]).await.unwrap();
    assert!(matches!(
        users.update(&user, 1).await,
        Err(StorageError::NotFound { entity: "user", .. })
    ));
    pool.close().await;
}

fn oauth_command(
    id: u8,
    subject: &str,
    email: Option<&str>,
) -> nebula_storage::auth::OAuthLoginFinalizeCommand {
    use nebula_storage::auth::{
        OAuthLoginFinalizeCommand, OAuthLoginMfaChallengeDraft, OAuthLoginSessionDraft,
        OAuthLoginUserDraft,
    };
    let now = Utc::now();
    OAuthLoginFinalizeCommand {
        provider: "github".into(),
        subject: subject.into(),
        verified_email: email.map(str::to_owned),
        candidate_user: OAuthLoginUserDraft {
            id: vec![id; 16],
            display_name: "OAuth account".into(),
            avatar_url: None,
            created_at: now,
        },
        session: OAuthLoginSessionDraft {
            token: vec![id; 32],
            created_at: now,
            last_active_at: now,
            expires_at: now + Duration::hours(1),
            ip_address: None,
            user_agent: None,
        },
        mfa_challenge: OAuthLoginMfaChallengeDraft {
            token_hash: [id; 32],
            created_at: now,
            expires_at: now + Duration::minutes(5),
        },
    }
}

#[tokio::test]
async fn concurrent_oauth_callbacks_converge_and_email_collisions_never_link() {
    use nebula_storage::auth::{
        OAuthLoginFinalizeOutcome, OAuthLoginFinalizer, sqlite::SqliteOAuthLoginFinalizer,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accounts.db");
    let first = open(&path).await;
    let second = open(&path).await;
    let a = SqliteOAuthLoginFinalizer::new(first.clone());
    let b = SqliteOAuthLoginFinalizer::new(second.clone());
    let (x, y) = tokio::join!(
        a.finalize(oauth_command(1, "subject", Some("user@example.test"))),
        b.finalize(oauth_command(2, "subject", Some("user@example.test")))
    );
    let OAuthLoginFinalizeOutcome::Finalized(x) = x.unwrap() else {
        panic!("first login refused")
    };
    let OAuthLoginFinalizeOutcome::Finalized(y) = y.unwrap() else {
        panic!("second login refused")
    };
    assert_eq!(x.user.id, y.user.id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
            .fetch_one(&first)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions")
            .fetch_one(&first)
            .await
            .unwrap(),
        2
    );
    assert!(matches!(
        a.finalize(oauth_command(3, "other-subject", Some("user@example.test")))
            .await
            .unwrap(),
        OAuthLoginFinalizeOutcome::AccountLinkRequired
    ));
    assert!(matches!(
        a.finalize(oauth_command(4, "unknown-subject", None))
            .await
            .unwrap(),
        OAuthLoginFinalizeOutcome::VerifiedEmailRequired
    ));
    let OAuthLoginFinalizeOutcome::Finalized(repeat) =
        a.finalize(oauth_command(5, "subject", None)).await.unwrap()
    else {
        panic!("existing link requires no email")
    };
    assert_eq!(repeat.user.id, x.user.id);
    SqliteUserRepo::new(first.clone())
        .soft_delete(&x.user.id)
        .await
        .unwrap();
    assert!(matches!(
        a.finalize(oauth_command(6, "subject", Some("new@example.test")))
            .await
            .unwrap(),
        OAuthLoginFinalizeOutcome::LinkedUserUnavailable
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM external_identities")
            .fetch_one(&first)
            .await
            .unwrap(),
        1
    );
    first.close().await;
    second.close().await;
}

#[tokio::test]
async fn oauth_finalization_with_mfa_creates_only_a_challenge_and_rolls_back_failed_artifacts() {
    use nebula_storage::auth::{
        OAuthLoginFinalizeOutcome, OAuthLoginFinalizer, sqlite::SqliteOAuthLoginFinalizer,
    };
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .unwrap();
    let finalizer = SqliteOAuthLoginFinalizer::new(pool.clone());
    let OAuthLoginFinalizeOutcome::Finalized(first) = finalizer
        .finalize(oauth_command(1, "subject", Some("user@example.test")))
        .await
        .unwrap()
    else {
        panic!("first login")
    };
    let mut second = oauth_command(2, "other", Some("other@example.test"));
    second.session.token = vec![1; 32];
    assert!(
        finalizer.finalize(second).await.is_err(),
        "duplicate session fails transaction"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM external_identities")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let codec = identity_codec();
    let envelope = codec
        .seal_totp_seed(
            nebula_storage::auth::identity_secret::TotpSecretPurpose::Active,
            &first.user.id,
            b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
        )
        .unwrap();
    sqlx::query("UPDATE users SET mfa_enabled = 1, mfa_secret_envelope = ? WHERE id = ?")
        .bind(envelope)
        .bind(&first.user.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        finalizer
            .finalize(oauth_command(3, "subject", None))
            .await
            .unwrap(),
        OAuthLoginFinalizeOutcome::MfaRequired
    ));
    assert!(
        SqliteSessionRepo::new(pool.clone())
            .get(&[3; 32])
            .await
            .unwrap()
            .is_none()
    );
    let challenge = SqliteVerificationTokenRepo::new(pool.clone())
        .consume_by_hash_and_kind(&[3; 32], "mfa_challenge")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(challenge.user_id, first.user.id);
    pool.close().await;
}

fn state(value: &str) -> nebula_storage::auth::OAuthStateRow {
    nebula_storage::auth::OAuthStateRow {
        state: value.into(),
        provider: "github".into(),
        code_verifier: "verifier".into(),
        redirect_uri: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::minutes(5),
        consumed_at: None,
    }
}

#[tokio::test]
async fn contended_oauth_does_not_retain_the_only_deployment_connection() {
    use nebula_storage::auth::{OAuthStateAdmission, OAuthStateRepo, sqlite::SqliteOAuthStateRepo};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accounts.db");
    let writer = open(&path).await;
    let deployment = oauth_deployment(&path).await;
    let shared = deployment.pool();
    let repo = SqliteOAuthStateRepo::new(&deployment);
    let held = writer.begin_with("BEGIN IMMEDIATE").await.unwrap();
    assert_eq!(
        repo.admit(&state("contended")).await.unwrap(),
        OAuthStateAdmission::Contended
    );
    // Keep the independent writer locked while an unrelated read uses the sole
    // deployment connection: bounding only the OAuth response is insufficient.
    tokio::time::timeout(
        std::time::Duration::from_millis(500),
        sqlx::query_scalar::<_, i64>("SELECT 1").fetch_one(shared),
    )
    .await
    .expect("admission must release the physical connection")
    .unwrap();
    held.rollback().await.unwrap();
    shared.close().await;
    writer.close().await;
}

#[tokio::test]
async fn oauth_admission_is_bounded_and_recovers_after_cancelled_begin() {
    use nebula_storage::auth::{OAuthStateAdmission, OAuthStateRepo, sqlite::SqliteOAuthStateRepo};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accounts.db");
    let first = open(&path).await;
    let deployment = oauth_deployment(&path).await;
    let second = deployment.pool();
    let repo = SqliteOAuthStateRepo::new(&deployment);
    let held = first.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        repo.admit(&state("contended")),
    )
    .await
    .expect("bounded response")
    .unwrap();
    assert_eq!(outcome, OAuthStateAdmission::Contended);
    held.rollback().await.unwrap();
    // A timed-out BEGIN must not later insert a state or leave a transaction
    // behind on a reused connection.
    assert!(repo.get_by_state("contended").await.unwrap().is_none());
    assert_eq!(
        repo.admit(&state("accepted")).await.unwrap(),
        OAuthStateAdmission::Created
    );
    assert!(
        repo.consume_by_state_and_provider("accepted", "google")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repo.consume_by_state_and_provider("accepted", "github")
            .await
            .unwrap()
            .is_some()
    );
    assert!(repo.consume_by_state("accepted").await.unwrap().is_none());
    first.close().await;
    second.close().await;
}

#[tokio::test]
async fn oauth_commit_contention_rolls_back_and_returns_contended() {
    use nebula_storage::auth::{OAuthStateAdmission, OAuthStateRepo, sqlite::SqliteOAuthStateRepo};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reader-blocked.db");
    let reader = open(&path).await;
    let deployment = oauth_deployment(&path).await;
    let repo = SqliteOAuthStateRepo::new(&deployment);
    let mut held = reader.begin().await.unwrap();
    sqlx::query("SELECT state FROM oauth_states")
        .fetch_all(&mut *held)
        .await
        .unwrap();
    assert_eq!(
        repo.admit(&state("blocked-commit")).await.unwrap(),
        OAuthStateAdmission::Contended
    );
    held.rollback().await.unwrap();
    assert!(repo.get_by_state("blocked-commit").await.unwrap().is_none());
    assert_eq!(
        repo.admit(&state("after-reader")).await.unwrap(),
        OAuthStateAdmission::Created
    );
    reader.close().await;
    deployment.pool().close().await;
}

#[tokio::test]
async fn oauth_capacity_is_shared_by_independent_pools() {
    use nebula_storage::auth::{
        OAUTH_STATE_CAPACITY, OAuthStateAdmission, OAuthStateRepo, sqlite::SqliteOAuthStateRepo,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accounts.db");
    let first_deployment = oauth_deployment(&path).await;
    let second_deployment = oauth_deployment(&path).await;
    let first = first_deployment.pool();
    let second = second_deployment.pool();
    let now = Utc::now();
    sqlx::query(
        "WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM numbers WHERE n < ?)
        INSERT INTO oauth_states(state, provider, code_verifier, created_at, expires_at)
        SELECT 'seed-' || n, 'github', 'verifier', ?, ? FROM numbers",
    )
    .bind(i64::from(OAUTH_STATE_CAPACITY) - 1)
    .bind(now.timestamp_micros())
    .bind((now + Duration::minutes(5)).timestamp_micros())
    .execute(first)
    .await
    .unwrap();
    let a = SqliteOAuthStateRepo::new(&first_deployment);
    let b = SqliteOAuthStateRepo::new(&second_deployment);
    let x = state("last-a");
    let y = state("last-b");
    let (x, y) = tokio::join!(a.admit(&x), b.admit(&y));
    assert_eq!(
        [x.unwrap(), y.unwrap()]
            .iter()
            .filter(|x| **x == OAuthStateAdmission::Created)
            .count(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM oauth_states WHERE consumed_at IS NULL")
            .fetch_one(first)
            .await
            .unwrap(),
        i64::from(OAUTH_STATE_CAPACITY)
    );
    assert_eq!(
        a.admit(&state("full")).await.unwrap(),
        OAuthStateAdmission::AtCapacity
    );
    a.consume_by_state("seed-1").await.unwrap().unwrap();
    assert_eq!(
        b.admit(&state("reclaimed")).await.unwrap(),
        OAuthStateAdmission::Created
    );
    first.close().await;
    second.close().await;
}

#[tokio::test]
async fn secret_admission_rejects_invalid_active_material_without_rewriting_it() {
    use nebula_storage::auth::sqlite::admit_identity_secrets;
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .unwrap();
    register(&pool, 1, 1).await.unwrap();
    let codec = identity_codec();
    admit_identity_secrets(&pool, &codec).await.unwrap();
    let invalid = b"UNENCRYPTED_SECRET_CANARY-4c9a";
    sqlx::query("UPDATE users SET mfa_enabled = 1, mfa_secret_envelope = ? WHERE id = ?")
        .bind(invalid.as_slice())
        .bind([1_u8; 16].as_slice())
        .execute(&pool)
        .await
        .unwrap();
    let error = admit_identity_secrets(&pool, &codec).await.unwrap_err();
    assert!(matches!(error, StorageError::Corrupt(_)));
    assert!(!format!("{error:?}").contains("UNENCRYPTED_SECRET_CANARY"));
    let stored: Vec<u8> = sqlx::query_scalar("SELECT mfa_secret_envelope FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, invalid);
    pool.close().await;
}

struct IdentityKey {
    version: &'static str,
    key: std::sync::Arc<nebula_crypto::EncryptionKey>,
}

impl nebula_storage::credential::KeyProvider for IdentityKey {
    fn current(
        &self,
    ) -> Result<nebula_storage::credential::KeySnapshot, nebula_storage::credential::ProviderError>
    {
        nebula_storage::credential::KeySnapshot::new(self.version, self.key.clone())
    }
}

fn rotation_codec(include_old: bool) -> nebula_storage::auth::identity_secret::IdentitySecretCodec {
    use std::sync::Arc;
    nebula_storage::auth::identity_secret::IdentitySecretCodec::with_legacy_keys(
        Arc::new(IdentityKey {
            version: "new",
            key: Arc::new(nebula_crypto::EncryptionKey::from_bytes([2; 32])),
        }),
        if include_old {
            vec![(
                "old".into(),
                Arc::new(nebula_crypto::EncryptionKey::from_bytes([1; 32])),
            )]
        } else {
            Vec::new()
        },
    )
    .unwrap()
}

fn old_codec() -> nebula_storage::auth::identity_secret::IdentitySecretCodec {
    use std::sync::Arc;
    nebula_storage::auth::identity_secret::IdentitySecretCodec::new(Arc::new(IdentityKey {
        version: "old",
        key: Arc::new(nebula_crypto::EncryptionKey::from_bytes([1; 32])),
    }))
    .unwrap()
}

async fn active_envelope(pool: &SqlitePool, id: u8, bytes: &[u8]) {
    sqlx::query("UPDATE users SET mfa_enabled = 1, mfa_secret_envelope = ? WHERE id = ?")
        .bind(bytes)
        .bind([id; 16].as_slice())
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn startup_rotates_multiple_pages_and_pending_secrets_before_new_key_only_reopen() {
    use nebula_storage::auth::{
        MfaEnrollmentCandidate, MfaEnrollmentRepo,
        identity_secret::TotpSecretPurpose,
        sqlite::{SqliteMfaEnrollmentRepo, admit_identity_secrets},
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rotation.db");
    let pool = open(&path).await;
    let old = std::sync::Arc::new(old_codec());
    let seed = b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
    for id in 1..=129 {
        register(&pool, id, id).await.unwrap();
        let envelope = old
            .seal_totp_seed(TotpSecretPurpose::Active, &[id; 16], seed)
            .unwrap();
        active_envelope(&pool, id, &envelope).await;
    }
    let pending = old
        .seal_totp_seed(TotpSecretPurpose::EnrollmentCandidate, &[129; 16], seed)
        .unwrap();
    SqliteMfaEnrollmentRepo::new(pool.clone(), old)
        .replace_candidate(
            &MfaEnrollmentCandidate::new(
                [1; 32],
                vec![129; 16],
                pending,
                Utc::now(),
                Utc::now() + Duration::minutes(10),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    admit_identity_secrets(&pool, &rotation_codec(true))
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users WHERE version = 1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        129
    );
    pool.close().await;
    let pool = open(&path).await;
    let current = rotation_codec(false);
    admit_identity_secrets(&pool, &current).await.unwrap();
    let last = SqliteUserRepo::new(pool.clone())
        .get(&[129; 16])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        current
            .open_totp_seed(
                TotpSecretPurpose::Active,
                &last.id,
                last.mfa_secret_envelope.as_deref().unwrap()
            )
            .unwrap()
            .plaintext
            .as_slice(),
        seed
    );
    let candidate = SqliteMfaEnrollmentRepo::new(pool.clone(), std::sync::Arc::new(current))
        .get_live_candidate(&[129; 16])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rotation_codec(false)
            .open_totp_seed(
                TotpSecretPurpose::EnrollmentCandidate,
                candidate.user_id(),
                candidate.secret_envelope()
            )
            .unwrap()
            .plaintext
            .as_slice(),
        seed
    );
    pool.close().await;
}

#[tokio::test]
async fn startup_rejects_wrong_owner_purpose_and_unavailable_key_without_partial_rotation() {
    use nebula_storage::auth::{
        identity_secret::TotpSecretPurpose, sqlite::admit_identity_secrets,
    };
    let pool = nebula_storage::sqlite::open_memory_deployment()
        .await
        .unwrap();
    register(&pool, 1, 1).await.unwrap();
    register(&pool, 2, 2).await.unwrap();
    let old = old_codec();
    let seed = b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
    let first = old
        .seal_totp_seed(TotpSecretPurpose::Active, &[1; 16], seed)
        .unwrap();
    active_envelope(&pool, 1, &first).await;
    assert!(matches!(
        admit_identity_secrets(&pool, &rotation_codec(false)).await,
        Err(StorageError::Corrupt(_))
    ));
    for rejected in [
        first.clone(),
        old.seal_totp_seed(TotpSecretPurpose::EnrollmentCandidate, &[2; 16], seed)
            .unwrap(),
    ] {
        active_envelope(&pool, 2, &rejected).await;
        assert!(matches!(
            admit_identity_secrets(&pool, &rotation_codec(true)).await,
            Err(StorageError::Corrupt(_))
        ));
        let unchanged = SqliteUserRepo::new(pool.clone())
            .get(&[1; 16])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.version, 0, "earlier rotation rolled back");
        assert_eq!(
            unchanged.mfa_secret_envelope.as_deref(),
            Some(first.as_slice())
        );
    }
    pool.close().await;
}

#[tokio::test]
async fn cancelled_borrower_restores_policy_and_preserves_in_memory_database() {
    use sqlx::Connection;
    let deployment = nebula_storage::sqlite::DeploymentPool::connect(
        SqliteConnectOptions::new().in_memory(true),
    )
    .await
    .unwrap();
    let pool = deployment.pool();
    nebula_storage::sqlite::init_schema(pool).await.unwrap();
    register(pool, 1, 1).await.unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let borrowed = pool.clone();
    let task = tokio::spawn(async move {
        let mut connection = borrowed.acquire().await.unwrap();
        sqlx::query("PRAGMA busy_timeout = 0")
            .execute(&mut *connection)
            .await
            .unwrap();
        let mut tx = connection.begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::query("DELETE FROM users")
            .execute(&mut *tx)
            .await
            .unwrap();
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    ready.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let mut connection =
        tokio::time::timeout(std::time::Duration::from_millis(500), pool.acquire())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA busy_timeout")
            .fetch_one(&mut *connection)
            .await
            .unwrap(),
        5000
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users")
            .fetch_one(&mut *connection)
            .await
            .unwrap(),
        1
    );
    drop(connection);
    pool.close().await;
}
