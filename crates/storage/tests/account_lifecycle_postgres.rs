//! Live PostgreSQL evidence for account-owned atomic transitions.

#![cfg(feature = "postgres")]

use std::{future::Future, str::FromStr};

use chrono::{Duration, Utc};
use nebula_storage::{
    StorageError,
    auth::{
        AccountLifecycle, AccountTokenOutcome, PasswordRegistration, PatRepo,
        PersonalAccessTokenRow, UserRepo, VerificationTokenRepo, VerificationTokenRow,
        postgres::{PgAccountLifecycle, PgPatRepo, PgUserRepo, PgVerificationTokenRepo},
    },
};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

async fn with_database<F, T>(test: F)
where
    F: FnOnce(PgPool) -> T + Send + 'static,
    T: Future<Output = ()> + Send + 'static,
{
    let dsn = std::env::var("DATABASE_URL").expect("SETUP: live PostgreSQL required");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&dsn)
        .await
        .expect("SETUP: admin");
    let schema = format!("account_lifecycle_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("SETUP: private schema");
    let options = PgConnectOptions::from_str(&dsn)
        .expect("SETUP: options")
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("SETUP: pool");
    let test_pool = pool.clone();
    // Join isolates assertion panics so the private schema is removed on failure too.
    let result = tokio::spawn(async move {
        nebula_storage::postgres::init_schema(&test_pool)
            .await
            .expect("SETUP: admitted schema");
        test(test_pool).await;
    })
    .await;
    pool.close().await;
    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await;
    admin.close().await;
    cleanup.expect("CLEANUP: private schema");
    result.expect("account lifecycle assertions");
}

async fn register(lifecycle: &PgAccountLifecycle, id: u8, hash: u8) {
    lifecycle
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
        .expect("register account");
}

async fn reset_token(tokens: &PgVerificationTokenRepo, user: u8, hash: u8) {
    tokens
        .create(&VerificationTokenRow {
            token_hash: vec![hash; 32],
            user_id: vec![user; 16],
            kind: "password_reset".into(),
            payload: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::hours(1),
            consumed_at: None,
        })
        .await
        .expect("reset token");
}

#[tokio::test]
async fn second_insert_failure_rolls_back_registration_without_leaking_values() {
    with_database(|pool| async move {
        let lifecycle = PgAccountLifecycle::new(pool.clone());
        let users = PgUserRepo::new(pool.clone());
        let tokens = PgVerificationTokenRepo::new(pool);
        register(&lifecycle, 1, 1).await;
        let failed = PasswordRegistration {
            user_id: &[2; 16],
            email: "signup-canary@example.test",
            display_name: "display-canary",
            password_hash: "password-hash-canary",
            verification_hash: &[1; 32],
            created_at: Utc::now(),
            verification_expires_at: Utc::now() + Duration::hours(1),
        };
        let error = lifecycle
            .register_password_user(&failed)
            .await
            .expect_err("duplicate verification hash");
        assert!(matches!(
            error,
            StorageError::Duplicate {
                entity: "verification_token",
                ..
            }
        ));
        for diagnostic in [
            error.to_string(),
            format!("{error:?}"),
            format!("{failed:?}"),
        ] {
            for canary in ["signup-canary", "display-canary", "password-hash-canary"] {
                assert!(
                    !diagnostic.contains(canary),
                    "diagnostic leaked signup data"
                );
            }
        }
        assert!(
            users
                .get(&[2; 16])
                .await
                .expect("read rolled-back user")
                .is_none()
        );
        let token = tokens
            .get_by_hash(&[1; 32])
            .await
            .expect("existing token")
            .expect("token retained");
        assert_eq!(token.user_id, vec![1; 16]);
        assert!(token.consumed_at.is_none());
    })
    .await;
}

#[tokio::test]
async fn email_redemption_checks_kind_replay_and_rolls_back_for_archived_accounts() {
    with_database(|pool| async move {
        let lifecycle = PgAccountLifecycle::new(pool.clone());
        let users = PgUserRepo::new(pool.clone());
        let tokens = PgVerificationTokenRepo::new(pool.clone());
        register(&lifecycle, 1, 1).await;
        reset_token(&tokens, 1, 2).await;
        let mut expired = tokens.get_by_hash(&[1; 32]).await.unwrap().unwrap();
        expired.token_hash = vec![6; 32];
        expired.expires_at = Utc::now() - Duration::seconds(1);
        tokens
            .create(&expired)
            .await
            .expect("expired verification token");
        assert_eq!(
            lifecycle.verify_email(&[6; 32]).await.unwrap(),
            AccountTokenOutcome::InvalidToken
        );
        assert!(
            tokens
                .get_by_hash(&[6; 32])
                .await
                .unwrap()
                .unwrap()
                .consumed_at
                .is_none()
        );
        assert_eq!(
            lifecycle.verify_email(&[2; 32]).await.expect("wrong kind"),
            AccountTokenOutcome::InvalidToken
        );
        assert!(
            tokens
                .get_by_hash(&[2; 32])
                .await
                .unwrap()
                .unwrap()
                .consumed_at
                .is_none()
        );
        assert_eq!(
            lifecycle.verify_email(&[1; 32]).await.expect("verify"),
            AccountTokenOutcome::Applied
        );
        assert!(
            users
                .get(&[1; 16])
                .await
                .unwrap()
                .unwrap()
                .email_verified_at
                .is_some()
        );
        assert_eq!(
            lifecycle.verify_email(&[1; 32]).await.expect("replay"),
            AccountTokenOutcome::InvalidToken
        );

        register(&lifecycle, 3, 3).await;
        users.soft_delete(&[3; 16]).await.expect("archive");
        assert_eq!(
            lifecycle.verify_email(&[3; 32]).await.expect("archived"),
            AccountTokenOutcome::UserUnavailable
        );
        assert!(tokens.get_by_hash(&[3; 32]).await.unwrap().is_none());
        let consumed: Option<chrono::DateTime<Utc>> =
            sqlx::query_scalar("SELECT consumed_at FROM verification_tokens WHERE token_hash = $1")
                .bind([3_u8; 32].as_slice())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            consumed.is_none(),
            "archived account redemption rolled back token consumption"
        );
    })
    .await;
}

#[tokio::test]
async fn reset_is_single_use_revokes_siblings_and_preserves_unrelated_authority() {
    with_database(|pool| async move {
        let lifecycle = PgAccountLifecycle::new(pool.clone());
        let users = PgUserRepo::new(pool.clone());
        let tokens = PgVerificationTokenRepo::new(pool.clone());
        register(&lifecycle, 1, 1).await;
        reset_token(&tokens, 1, 2).await;
        reset_token(&tokens, 1, 3).await;
        let mut user = users.get(&[1; 16]).await.unwrap().unwrap();
        user.failed_login_count = 4;
        user.locked_until = Some(Utc::now() + Duration::hours(1));
        user.mfa_enabled = true;
        user.mfa_secret_envelope = Some(vec![7; 32]);
        users
            .update(&user, user.version)
            .await
            .expect("existing MFA and lockout");

        assert_eq!(
            lifecycle
                .reset_password(&[1; 32], "wrong-kind")
                .await
                .unwrap(),
            AccountTokenOutcome::InvalidToken
        );
        let (first, second) = tokio::join!(
            lifecycle.reset_password(&[2; 32], "replacement-a"),
            lifecycle.reset_password(&[2; 32], "replacement-b"),
        );
        let outcomes = [first.expect("first reset"), second.expect("second reset")];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == AccountTokenOutcome::Applied)
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == AccountTokenOutcome::InvalidToken)
                .count(),
            1
        );
        let user = users.get(&[1; 16]).await.unwrap().unwrap();
        assert_eq!(
            user.password_hash.as_deref(),
            Some(if outcomes[0] == AccountTokenOutcome::Applied {
                "replacement-a"
            } else {
                "replacement-b"
            })
        );
        assert_eq!(user.failed_login_count, 0);
        assert!(user.locked_until.is_none());
        assert!(user.mfa_enabled);
        assert_eq!(user.mfa_secret_envelope, Some(vec![7; 32]));
        assert!(user.email_verified_at.is_none());
        assert_eq!(
            lifecycle
                .reset_password(&[3; 32], "stolen-sibling")
                .await
                .unwrap(),
            AccountTokenOutcome::InvalidToken
        );
        assert!(
            tokens
                .get_by_hash(&[1; 32])
                .await
                .unwrap()
                .unwrap()
                .consumed_at
                .is_none()
        );

        register(&lifecycle, 4, 4).await;
        reset_token(&tokens, 4, 5).await;
        users.soft_delete(&[4; 16]).await.expect("archive");
        assert_eq!(
            lifecycle
                .reset_password(&[5; 32], "unavailable")
                .await
                .unwrap(),
            AccountTokenOutcome::UserUnavailable
        );
        assert!(tokens.get_by_hash(&[5; 32]).await.unwrap().is_none());
        let consumed: Option<chrono::DateTime<Utc>> =
            sqlx::query_scalar("SELECT consumed_at FROM verification_tokens WHERE token_hash = $1")
                .bind([5_u8; 32].as_slice())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            consumed.is_none(),
            "archived reset rolled back token consumption"
        );
    })
    .await;
}

#[tokio::test]
async fn pat_revocation_is_owner_qualified_and_idempotent() {
    with_database(|pool| async move {
        let pats = PgPatRepo::new(pool);
        pats.create(&PersonalAccessTokenRow {
            id: vec![1; 16],
            principal_kind: "user".into(),
            principal_id: vec![2; 16],
            name: "automation".into(),
            prefix: "pat_test".into(),
            hash: vec![3; 32],
            scopes: serde_json::json!([]),
            created_at: Utc::now(),
            last_used_at: None,
            expires_at: None,
            revoked_at: None,
        })
        .await
        .expect("create PAT");
        assert!(
            !pats
                .revoke_for_principal(&[1; 16], "user", &[9; 16])
                .await
                .unwrap()
        );
        assert!(
            !pats
                .revoke_for_principal(&[1; 16], "service_account", &[2; 16])
                .await
                .unwrap()
        );
        assert!(pats.get_by_hash(&[3; 32]).await.unwrap().is_some());
        assert!(
            pats.revoke_for_principal(&[1; 16], "user", &[2; 16])
                .await
                .unwrap()
        );
        assert!(pats.get_by_hash(&[3; 32]).await.unwrap().is_none());
        assert!(
            pats.revoke_for_principal(&[1; 16], "user", &[2; 16])
                .await
                .unwrap()
        );
        assert!(
            !pats
                .revoke_for_principal(&[9; 16], "user", &[2; 16])
                .await
                .unwrap()
        );
    })
    .await;
}
