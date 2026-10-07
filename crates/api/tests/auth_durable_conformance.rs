//! Identical public authentication policy on both durable deployments.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use nebula_api::{
    ApiConfig, OAuthIdentityRuntime, app,
    config::{OAuthProviderConfig, OAuthProvidersConfig},
    domain::auth::backend::{
        AuthBackend, AuthError, CSRF_COOKIE, CreatePatParams, DurableAuthBackend, OAuthProvider,
        PasswordOutcome, SESSION_COOKIE, SecretString, SignupRequest, mfa,
    },
    ports::email::EchoSink,
};
use nebula_storage::auth::{
    AuthPersistence, UserRepo, VerificationTokenRepo, VerificationTokenRow,
    identity_secret::IdentitySecretCodec,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;

fn codec() -> Arc<IdentitySecretCodec> {
    Arc::new(
        IdentitySecretCodec::new(Arc::new(
            nebula_storage::credential::EnvKeyProvider::from_base64(
                "MzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzM=",
            )
            .unwrap(),
        ))
        .unwrap(),
    )
}

async fn assert_durable_policy(
    persistence: AuthPersistence,
    users: &dyn UserRepo,
    tokens: &dyn VerificationTokenRepo,
) {
    let sink = Arc::new(EchoSink::default());
    let runtime = OAuthIdentityRuntime::from_config(OAuthProvidersConfig {
        providers: [OAuthProvider::Google, OAuthProvider::GitHub]
            .into_iter()
            .map(|provider| {
                (
                    provider,
                    OAuthProviderConfig {
                        client_id: secrecy::SecretString::new("test-client".into()),
                        client_secret: secrecy::SecretString::new("test-secret".into()),
                    },
                )
            })
            .collect(),
    })
    .unwrap()
    .unwrap();
    let backend = Arc::new(
        DurableAuthBackend::new(persistence, sink.clone(), None)
            .with_oauth_runtime(Arc::new(runtime)),
    );
    let email = format!("{}@durable.test", nebula_core::UserId::new());
    let signup = || SignupRequest {
        email: email.clone(),
        password: SecretString::new("Strong-Passw0rd-2026".into()),
        display_name: "Durable owner".into(),
    };
    let profile = backend.register_user(signup()).await.unwrap();
    assert!(matches!(
        backend.register_user(signup()).await,
        Err(AuthError::EmailAlreadyRegistered)
    ));
    let verification = sink.drain().pop().unwrap().body;
    backend.verify_email(&verification).await.unwrap();
    assert!(matches!(
        backend.verify_email(&verification).await,
        Err(AuthError::InvalidToken)
    ));
    let id = profile
        .user_id
        .parse::<nebula_core::UserId>()
        .unwrap()
        .as_bytes();
    let expired = format!("expired-{email}");
    tokens
        .create(&VerificationTokenRow {
            token_hash: Sha256::digest(expired.as_bytes()).to_vec(),
            user_id: id.to_vec(),
            kind: "email_verification".into(),
            payload: None,
            created_at: Utc::now() - Duration::hours(2),
            expires_at: Utc::now() - Duration::hours(1),
            consumed_at: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        backend.verify_email(&expired).await,
        Err(AuthError::InvalidToken)
    ));

    let session = backend.create_session(&profile.user_id).await.unwrap();
    let state = common::build_me_state().with_auth_backend(backend.clone());
    let response = app::build_app(state, &ApiConfig::for_test())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/mfa/enroll")
                .header(
                    "cookie",
                    format!(
                        "{SESSION_COOKIE}={}; {CSRF_COOKIE}={}",
                        session.id, session.csrf_token
                    ),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "durable sessions do not bypass CSRF"
    );

    let enrollment = backend
        .start_mfa_enrollment(&profile.user_id)
        .await
        .unwrap();
    backend
        .confirm_mfa_enrollment(
            &profile.user_id,
            &mfa::current_code(&enrollment.secret_base32).unwrap(),
        )
        .await
        .unwrap();
    let PasswordOutcome::MfaRequired { challenge_token } = backend
        .authenticate_password(&email, "Strong-Passw0rd-2026", None)
        .await
        .unwrap()
    else {
        panic!("MFA required")
    };
    assert!(matches!(
        backend.verify_mfa(&challenge_token, "invalid-code").await,
        Err(AuthError::InvalidMfaCode)
    ));
    assert!(
        matches!(
            backend
                .verify_mfa(
                    &challenge_token,
                    &mfa::current_code(&enrollment.secret_base32).unwrap()
                )
                .await,
            Err(AuthError::InvalidToken)
        ),
        "attempt consumes its challenge"
    );

    let callback = "https://nebula.test/api/v1/auth/oauth/google/callback";
    let oauth = backend
        .start_oauth(OAuthProvider::Google, callback)
        .await
        .unwrap();
    assert!(matches!(
        backend
            .complete_oauth(OAuthProvider::GitHub, &oauth.state, "fake", callback)
            .await,
        Err(AuthError::InvalidToken)
    ));
    assert!(matches!(
        backend
            .complete_oauth(
                OAuthProvider::Google,
                &oauth.state,
                "fake",
                "https://changed.test/callback"
            )
            .await,
        Err(AuthError::OAuthFailed)
    ));
    assert!(matches!(
        backend
            .complete_oauth(OAuthProvider::Google, &oauth.state, "fake", callback)
            .await,
        Err(AuthError::InvalidToken)
    ));

    for _ in 0..5 {
        assert!(matches!(
            backend
                .authenticate_password(&email, "wrong-password", None)
                .await,
            Err(AuthError::InvalidCredentials)
        ));
    }
    assert!(matches!(
        backend
            .authenticate_password(&email, "Strong-Passw0rd-2026", None)
            .await,
        Err(AuthError::AccountLocked)
    ));
    let mut locked = users.get(&id).await.unwrap().unwrap();
    assert_eq!(locked.failed_login_count, 5);
    locked.locked_until = Some(Utc::now() - Duration::seconds(1));
    users.update(&locked, locked.version).await.unwrap();
    assert!(matches!(
        backend
            .authenticate_password(&email, "Strong-Passw0rd-2026", None)
            .await
            .unwrap(),
        PasswordOutcome::MfaRequired { .. }
    ));
    assert_eq!(users.get(&id).await.unwrap().unwrap().failed_login_count, 0);

    let pat = backend
        .create_pat(
            &profile.user_id,
            CreatePatParams {
                name: "archive probe".into(),
                scopes: vec!["full_access".into()],
                ttl_seconds: None,
            },
        )
        .await
        .unwrap();
    assert!(backend.lookup_pat(&pat.plaintext).await.unwrap().is_some());
    assert!(
        backend
            .get_principal_by_session(&session.id)
            .await
            .unwrap()
            .is_some()
    );
    users.soft_delete(&id).await.unwrap();
    assert!(backend.lookup_pat(&pat.plaintext).await.unwrap().is_none());
    assert!(
        backend
            .get_principal_by_session(&session.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn sqlite_durable_policy() {
    use nebula_storage::{
        auth::sqlite::{SqliteUserRepo, SqliteVerificationTokenRepo, admit_identity_secrets},
        sqlite::DeploymentPool,
    };
    let deployment =
        DeploymentPool::connect(sqlx::sqlite::SqliteConnectOptions::new().in_memory(true))
            .await
            .unwrap();
    let pool = deployment.pool();
    nebula_storage::sqlite::init_schema(pool).await.unwrap();
    let codec = codec();
    admit_identity_secrets(pool, &codec).await.unwrap();
    assert_durable_policy(
        AuthPersistence::sqlite(&deployment, codec),
        &SqliteUserRepo::new(pool.clone()),
        &SqliteVerificationTokenRepo::new(pool.clone()),
    )
    .await;
    pool.close().await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "requires live PostgreSQL; run with DATABASE_URL and --run-ignored all"]
async fn postgres_durable_policy() {
    use nebula_storage::auth::postgres::{PgUserRepo, PgVerificationTokenRepo};
    let url = std::env::var("DATABASE_URL").expect("live PostgreSQL required");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    assert_durable_policy(
        AuthPersistence::postgres(pool.clone(), codec()),
        &PgUserRepo::new(pool.clone()),
        &PgVerificationTokenRepo::new(pool.clone()),
    )
    .await;
    pool.close().await;
}
