//! Archived accounts hide their owned authentication artifacts on both SQL backends.

#![cfg(any(feature = "sqlite", feature = "postgres"))]

use chrono::{Duration, Utc};
use nebula_storage::{
    StorageError,
    auth::{
        AuthPersistence, MfaEnrollmentCandidate, PasswordRegistration, SessionDraft,
        VerificationTokenRow,
        identity_secret::{IdentitySecretCodec, TotpSecretPurpose},
    },
    credential::EnvKeyProvider,
};
use std::sync::Arc;

fn codec() -> Arc<IdentitySecretCodec> {
    Arc::new(
        IdentitySecretCodec::new(Arc::new(
            EnvKeyProvider::from_base64("ERERERERERERERERERERERERERERERERERERERERERE=").unwrap(),
        ))
        .unwrap(),
    )
}

async fn archived_children(auth: &AuthPersistence) {
    let now = Utc::now();
    auth.accounts()
        .register_password_user(&PasswordRegistration {
            user_id: &[1; 16],
            email: "archived@example.test",
            display_name: "Archived owner",
            password_hash: "test-hash",
            verification_hash: &[1; 32],
            created_at: now,
            verification_expires_at: now + Duration::hours(1),
        })
        .await
        .unwrap();
    let mut session = SessionDraft {
        user_id: vec![1; 16],
        created_at: now,
        last_active_at: now,
        expires_at: now + Duration::hours(1),
        ip_address: None,
        user_agent: None,
        revoked_at: None,
    };
    auth.sessions().create(b"live", &session).await.unwrap();
    session.expires_at = now - Duration::hours(1);
    auth.sessions().create(b"expired", &session).await.unwrap();
    let mut token = VerificationTokenRow {
        token_hash: vec![2; 32],
        user_id: vec![1; 16],
        kind: "password_reset".into(),
        payload: None,
        created_at: now,
        expires_at: now - Duration::hours(1),
        consumed_at: None,
    };
    auth.verification_tokens().create(&token).await.unwrap();
    let candidate = |owner: u8, id: u8| {
        MfaEnrollmentCandidate::new(
            [id; 32],
            vec![owner; 16],
            auth.identity_secrets()
                .seal_totp_seed(
                    TotpSecretPurpose::EnrollmentCandidate,
                    &[owner; 16],
                    b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
                )
                .unwrap(),
            now,
            now + Duration::minutes(10),
        )
        .unwrap()
    };
    auth.mfa_enrollments()
        .replace_candidate(&candidate(1, 1))
        .await
        .unwrap();
    auth.users().soft_delete(&[1; 16]).await.unwrap();

    assert!(auth.sessions().get(b"live").await.unwrap().is_none());
    assert!(
        auth.verification_tokens()
            .get_by_hash(&[1; 32])
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        auth.mfa_enrollments()
            .get_live_candidate(&[1; 16])
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        auth.verification_tokens()
            .consume_by_hash(&[1; 32])
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        auth.verification_tokens()
            .consume_by_hash_and_kind(&[1; 32], "email_verification")
            .await
            .unwrap()
            .is_none()
    );
    auth.sessions().touch(b"live").await.unwrap();
    auth.sessions().revoke(b"live").await.unwrap();
    assert_eq!(
        auth.verification_tokens()
            .revoke_all_for_user(&[1; 16], "email_verification")
            .await
            .unwrap(),
        0
    );
    assert_eq!(auth.sessions().cleanup_expired().await.unwrap(), 0);
    assert_eq!(
        auth.verification_tokens().cleanup_expired().await.unwrap(),
        0
    );

    // Both a tombstone and a missing parent reject new owned artifacts.
    for owner in [1, 2] {
        session.user_id = vec![owner; 16];
        token.user_id = vec![owner; 16];
        token.token_hash = vec![3; 32];
        assert!(matches!(
            auth.sessions().create(b"new", &session).await,
            Err(StorageError::NotFound { entity: "user", .. })
        ));
        assert!(matches!(
            auth.verification_tokens().create(&token).await,
            Err(StorageError::NotFound { entity: "user", .. })
        ));
        assert!(matches!(
            auth.mfa_enrollments()
                .replace_candidate(&candidate(owner, 2))
                .await,
            Err(StorageError::NotFound { entity: "user", .. })
        ));
    }
}

// Raw inspection proves preservation; public absence alone cannot prove rollback.
const PRESERVED: &str = "SELECT
    (SELECT COUNT(*) FROM sessions WHERE revoked_at IS NULL AND last_active_at = created_at),
    (SELECT COUNT(*) FROM verification_tokens WHERE consumed_at IS NULL),
    (SELECT enrollment_id FROM mfa_enrollment_candidates)";

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_archive_hides_and_preserves_owned_artifacts() {
    let deployment = nebula_storage::sqlite::DeploymentPool::connect(
        sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    nebula_storage::sqlite::init_schema(deployment.pool())
        .await
        .unwrap();
    archived_children(&AuthPersistence::sqlite(&deployment, codec())).await;
    let evidence: (i64, i64, Vec<u8>) = sqlx::query_as(sqlx::AssertSqlSafe(PRESERVED))
        .fetch_one(deployment.pool())
        .await
        .unwrap();
    assert_eq!(evidence, (2, 2, vec![1; 32]));
    deployment.pool().close().await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "requires live PostgreSQL"]
async fn postgres_archive_hides_and_preserves_owned_artifacts() {
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use std::str::FromStr;
    let dsn = std::env::var("DATABASE_URL").expect("live PostgreSQL required");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&dsn)
        .await
        .unwrap();
    let schema = format!("auth_archive_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let options = PgConnectOptions::from_str(&dsn)
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .unwrap();
    let test_pool = pool.clone();
    let result = tokio::spawn(async move {
        nebula_storage::postgres::init_schema(&test_pool)
            .await
            .unwrap();
        archived_children(&AuthPersistence::postgres(test_pool.clone(), codec())).await;
        let evidence: (i64, i64, Vec<u8>) = sqlx::query_as(sqlx::AssertSqlSafe(PRESERVED))
            .fetch_one(&test_pool)
            .await
            .unwrap();
        assert_eq!(evidence, (2, 2, vec![1; 32]));
        archive_serializes_with_child_creation(&test_pool).await;
    })
    .await;
    pool.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    result.unwrap();
}

#[cfg(feature = "postgres")]
async fn archive_serializes_with_child_creation(pool: &sqlx::PgPool) {
    for kind in 0..3_u8 {
        let id = [kind + 2; 16];
        let auth = AuthPersistence::postgres(pool.clone(), codec());
        let now = Utc::now();
        auth.accounts()
            .register_password_user(&PasswordRegistration {
                user_id: &id,
                email: &format!("archive-race-{kind}@example.test"),
                display_name: "Owner",
                password_hash: "test",
                verification_hash: &[kind + 10; 32],
                created_at: now,
                verification_expires_at: now + Duration::hours(1),
            })
            .await
            .unwrap();
        let mut archive = pool.begin().await.unwrap();
        let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *archive)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET deleted_at = NOW() WHERE id = $1")
            .bind(id.as_slice())
            .execute(&mut *archive)
            .await
            .unwrap();
        let create = tokio::spawn(async move {
            match kind {
                0 => {
                    auth.sessions()
                        .create(
                            b"archive-race",
                            &SessionDraft {
                                user_id: id.to_vec(),
                                created_at: now,
                                last_active_at: now,
                                expires_at: now + Duration::hours(1),
                                ip_address: None,
                                user_agent: None,
                                revoked_at: None,
                            },
                        )
                        .await
                },
                1 => {
                    auth.verification_tokens()
                        .create(&VerificationTokenRow {
                            token_hash: vec![22; 32],
                            user_id: id.to_vec(),
                            kind: "password_reset".into(),
                            payload: None,
                            created_at: now,
                            expires_at: now + Duration::hours(1),
                            consumed_at: None,
                        })
                        .await
                },
                _ => {
                    auth.mfa_enrollments()
                        .replace_candidate(
                            &MfaEnrollmentCandidate::new(
                                [22; 32],
                                id.to_vec(),
                                auth.identity_secrets()
                                    .seal_totp_seed(
                                        TotpSecretPurpose::EnrollmentCandidate,
                                        &id,
                                        b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
                                    )
                                    .unwrap(),
                                now,
                                now + Duration::minutes(10),
                            )
                            .unwrap(),
                        )
                        .await
                },
            }
        });
        // Observe the database wait, not merely task scheduling. Creation must
        // recheck the owner's tombstone after this transaction commits.
        let waited = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let blocked: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))",
                ).bind(blocker).fetch_one(pool).await.unwrap();
                if blocked { break; }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await;
        archive.commit().await.unwrap();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), create)
            .await
            .unwrap()
            .unwrap();
        waited.expect("child creation must wait for the owner archive transaction");
        assert!(matches!(
            outcome,
            Err(StorageError::NotFound { entity: "user", .. })
        ));
    }
}
