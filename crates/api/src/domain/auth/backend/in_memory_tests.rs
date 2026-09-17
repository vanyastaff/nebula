use super::*;
use crate::domain::auth::backend::dto::SecretString;

fn signup_req(email: &str) -> SignupRequest {
    SignupRequest {
        email: email.to_owned(),
        password: SecretString::new("hunter22".to_owned()),
        display_name: "Test User".to_owned(),
    }
}

async fn enroll_mfa_user(backend: &InMemoryAuthBackend, email: &str) -> (UserProfile, String) {
    let profile = backend
        .register_user(signup_req(email))
        .await
        .expect("seed MFA user");
    let enrollment = backend
        .start_mfa_enrollment(&profile.user_id)
        .await
        .expect("start MFA enrollment");
    let code = mfa::current_code(&enrollment.secret_base32).expect("current MFA code");
    backend
        .confirm_mfa_enrollment(&profile.user_id, &code)
        .await
        .expect("confirm MFA enrollment");
    (profile, enrollment.secret_base32)
}

fn oauth_material(
    session_id: &str,
    challenge_token: &str,
    challenge_expires_unix: u64,
) -> PreparedOAuthLogin {
    PreparedOAuthLogin {
        session_id: session_id.to_owned(),
        csrf_token: "test-csrf".to_owned(),
        session_expires_at: Utc::now(),
        session_expires_unix: u64::MAX,
        session_authenticated_at: Utc::now(),
        challenge_token: challenge_token.to_owned(),
        challenge_expires_unix,
    }
}

fn backend_with_oauth(providers: crate::config::OAuthProvidersConfig) -> InMemoryAuthBackend {
    let runtime = Arc::new(
        crate::OAuthIdentityRuntime::from_config(providers)
            .expect("test OAuth runtime must build")
            .expect("test OAuth provider set must enable the runtime"),
    );
    InMemoryAuthBackend::new().with_oauth_runtime(runtime)
}

#[tokio::test]
async fn register_then_login_returns_authenticated() {
    let b = InMemoryAuthBackend::new();
    let profile = b
        .register_user(signup_req("alice@nebula.dev"))
        .await
        .unwrap();
    assert_eq!(profile.email, "alice@nebula.dev");
    assert!(!profile.email_verified);
    assert!(!profile.mfa_enabled);

    let outcome = b
        .authenticate_password("alice@nebula.dev", "hunter22", None)
        .await
        .unwrap();
    match outcome {
        PasswordOutcome::Authenticated(p) => assert_eq!(p.user_id, profile.user_id),
        PasswordOutcome::MfaRequired { .. } => panic!("MFA not enabled"),
    }
}

#[tokio::test]
async fn signup_emits_verification_email() {
    let b = InMemoryAuthBackend::new();
    b.register_user(signup_req("a@b.c")).await.unwrap();
    let emails = b.emails();
    assert_eq!(emails.len(), 1);
    assert_eq!(emails[0].kind, EmailKind::Verification);
    assert_eq!(emails[0].to, "a@b.c");
}

#[tokio::test]
async fn login_with_wrong_password_is_invalid_credentials() {
    let b = InMemoryAuthBackend::new();
    b.register_user(signup_req("c@d.e")).await.unwrap();
    let err = b
        .authenticate_password("c@d.e", "wrong", None)
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
}

#[tokio::test]
async fn five_failures_lock_account() {
    let b = InMemoryAuthBackend::new();
    b.register_user(signup_req("locked@e.f")).await.unwrap();
    for _ in 0..5 {
        let _ = b.authenticate_password("locked@e.f", "wrong", None).await;
    }
    let err = b
        .authenticate_password("locked@e.f", "hunter22", None)
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::AccountLocked));
}

#[tokio::test]
async fn duplicate_signup_conflicts() {
    let b = InMemoryAuthBackend::new();
    b.register_user(signup_req("dup@e.f")).await.unwrap();
    let err = b.register_user(signup_req("dup@e.f")).await.unwrap_err();
    assert!(matches!(err, AuthError::EmailAlreadyRegistered));
}

#[tokio::test]
async fn create_session_then_resolve_principal() {
    let b = InMemoryAuthBackend::new();
    let profile = b.register_user(signup_req("s@e.f")).await.unwrap();
    let session = b.create_session(&profile.user_id).await.unwrap();
    let session_auth = b
        .get_principal_by_session(&session.id)
        .await
        .unwrap()
        .expect("session is live");
    assert!(matches!(session_auth.principal, Principal::User(_)));
    assert!(session_auth.authenticated_at <= Utc::now());
}

#[tokio::test]
async fn revoke_session_clears_lookup() {
    let b = InMemoryAuthBackend::new();
    let profile = b.register_user(signup_req("r@e.f")).await.unwrap();
    let session = b.create_session(&profile.user_id).await.unwrap();
    b.revoke_session(&session.id).await.unwrap();
    let resolved = b.get_principal_by_session(&session.id).await.unwrap();
    assert!(resolved.is_none());
}

#[tokio::test]
async fn email_verification_flips_flag() {
    let b = InMemoryAuthBackend::new();
    b.register_user(signup_req("v@e.f")).await.unwrap();
    let token = b.emails()[0].body.clone();
    b.verify_email(&token).await.unwrap();
    let user = b.lookup_user_by_email("v@e.f").unwrap();
    assert!(user.email_verified);
    // Replay rejected.
    let err = b.verify_email(&token).await.unwrap_err();
    assert!(matches!(err, AuthError::InvalidToken));
}

#[tokio::test]
async fn password_reset_round_trips() {
    let b = InMemoryAuthBackend::new();
    b.register_user(signup_req("p@e.f")).await.unwrap();
    // Drain the verification email so we only see the reset email next.
    b.default_echo
        .as_ref()
        .expect("default echo sink is wired when no custom port is injected")
        .drain();
    b.request_password_reset("p@e.f").await.unwrap();
    let token = b.emails()[0].body.clone();
    b.complete_password_reset(&token, "newpass1").await.unwrap();
    let outcome = b
        .authenticate_password("p@e.f", "newpass1", None)
        .await
        .unwrap();
    assert!(matches!(outcome, PasswordOutcome::Authenticated(_)));
}

#[tokio::test]
async fn mfa_enrollment_then_login_with_code() {
    let b = InMemoryAuthBackend::new();
    let profile = b.register_user(signup_req("m@e.f")).await.unwrap();
    let enrol = b.start_mfa_enrollment(&profile.user_id).await.unwrap();
    let code = mfa::current_code(&enrol.secret_base32).unwrap();
    b.confirm_mfa_enrollment(&profile.user_id, &code)
        .await
        .unwrap();

    let login_no_code = b
        .authenticate_password("m@e.f", "hunter22", None)
        .await
        .unwrap();
    let challenge = match login_no_code {
        PasswordOutcome::MfaRequired { challenge_token } => challenge_token,
        PasswordOutcome::Authenticated(_) => panic!("MFA should be required"),
    };
    let new_code = mfa::current_code(&enrol.secret_base32).unwrap();
    let final_profile = b.verify_mfa(&challenge, &new_code).await.unwrap();
    assert_eq!(final_profile.user_id, profile.user_id);
}

#[tokio::test]
async fn starting_reenrollment_preserves_the_active_factor_until_confirmation() {
    let b = InMemoryAuthBackend::new();
    let (profile, active_secret) = enroll_mfa_user(&b, "mfa-reenroll@e.f").await;

    let candidate = b
        .start_mfa_enrollment(&profile.user_id)
        .await
        .expect("start replacement MFA enrollment");
    assert_ne!(candidate.secret_base32, active_secret);

    let challenge_token = match b
        .authenticate_password("mfa-reenroll@e.f", "hunter22", None)
        .await
        .expect("active MFA must still gate password login")
    {
        PasswordOutcome::MfaRequired { challenge_token } => challenge_token,
        PasswordOutcome::Authenticated(_) => {
            panic!("starting a replacement must not disable active MFA")
        },
    };
    let active_code = mfa::current_code(&active_secret).expect("active MFA code");
    b.verify_mfa(&challenge_token, &active_code)
        .await
        .expect("the active factor must survive an abandoned enrollment");

    let candidate_code =
        mfa::current_code(&candidate.secret_base32).expect("replacement candidate MFA code");
    let wrong_candidate_code = format!(
        "{:06}",
        (candidate_code.parse::<u32>().expect("numeric TOTP") + 1) % 1_000_000
    );
    let error = b
        .confirm_mfa_enrollment(&profile.user_id, &wrong_candidate_code)
        .await
        .expect_err("wrong replacement code must reject");
    assert!(matches!(error, AuthError::InvalidMfaCode));

    let second_challenge = match b
        .authenticate_password("mfa-reenroll@e.f", "hunter22", None)
        .await
        .expect("failed replacement must preserve active MFA")
    {
        PasswordOutcome::MfaRequired { challenge_token } => challenge_token,
        PasswordOutcome::Authenticated(_) => {
            panic!("failed replacement must not disable active MFA")
        },
    };
    let active_code = mfa::current_code(&active_secret).expect("active MFA code");
    b.verify_mfa(&second_challenge, &active_code)
        .await
        .expect("the original factor must remain authoritative");
}

#[tokio::test]
async fn confirmed_enrollment_is_single_use() {
    let b = InMemoryAuthBackend::new();
    let profile = b
        .register_user(signup_req("mfa-single-use@e.f"))
        .await
        .expect("register user");
    let candidate = b
        .start_mfa_enrollment(&profile.user_id)
        .await
        .expect("start MFA enrollment");
    let code = mfa::current_code(&candidate.secret_base32).expect("candidate MFA code");

    b.confirm_mfa_enrollment(&profile.user_id, &code)
        .await
        .expect("first confirmation installs candidate");
    let replay = b
        .confirm_mfa_enrollment(&profile.user_id, &code)
        .await
        .expect_err("confirmed candidate must not be replayable");
    assert!(matches!(replay, AuthError::InvalidMfaCode));
}

#[tokio::test]
async fn concurrent_enrollment_confirmation_has_exactly_one_winner() {
    let b = Arc::new(InMemoryAuthBackend::new());
    let profile = b
        .register_user(signup_req("mfa-concurrent@e.f"))
        .await
        .expect("register user");
    let candidate = b
        .start_mfa_enrollment(&profile.user_id)
        .await
        .expect("start MFA enrollment");
    let code = mfa::current_code(&candidate.secret_base32).expect("candidate MFA code");

    let left = {
        let b = Arc::clone(&b);
        let user_id = profile.user_id.clone();
        let code = code.clone();
        tokio::spawn(async move { b.confirm_mfa_enrollment(&user_id, &code).await })
    };
    let right = {
        let b = Arc::clone(&b);
        let user_id = profile.user_id.clone();
        tokio::spawn(async move { b.confirm_mfa_enrollment(&user_id, &code).await })
    };
    let outcomes = [
        left.await.expect("left join"),
        right.await.expect("right join"),
    ];

    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| matches!(result, Err(AuthError::InvalidMfaCode)))
            .count(),
        1
    );
}

#[tokio::test]
async fn expired_enrollment_candidate_cannot_be_installed() {
    let b = InMemoryAuthBackend::new();
    let profile = b
        .register_user(signup_req("mfa-expired@e.f"))
        .await
        .expect("register user");
    let candidate = b
        .start_mfa_enrollment(&profile.user_id)
        .await
        .expect("start MFA enrollment");
    let user_id: UserId = profile.user_id.parse().expect("valid user id");
    b.pending_mfa_enrollments
        .lock()
        .get_mut(&user_id)
        .expect("pending candidate")
        .expires_at = 0;
    let code = mfa::current_code(&candidate.secret_base32).expect("candidate MFA code");

    let error = b
        .confirm_mfa_enrollment(&profile.user_id, &code)
        .await
        .expect_err("expired candidate must reject");
    assert!(matches!(error, AuthError::InvalidMfaCode));
    assert!(
        !b.get_user_profile(&profile.user_id)
            .await
            .expect("load profile")
            .mfa_enabled
    );
    assert!(!b.pending_mfa_enrollments.lock().contains_key(&user_id));
}

#[tokio::test]
async fn wrong_code_does_not_consume_a_live_enrollment_candidate() {
    let b = InMemoryAuthBackend::new();
    let profile = b
        .register_user(signup_req("mfa-retry@e.f"))
        .await
        .expect("register user");
    let candidate = b
        .start_mfa_enrollment(&profile.user_id)
        .await
        .expect("start MFA enrollment");
    let code = mfa::current_code(&candidate.secret_base32).expect("candidate MFA code");
    let wrong_code = format!(
        "{:06}",
        (code.parse::<u32>().expect("numeric TOTP") + 1) % 1_000_000
    );

    let error = b
        .confirm_mfa_enrollment(&profile.user_id, &wrong_code)
        .await
        .expect_err("wrong code must reject");
    assert!(matches!(error, AuthError::InvalidMfaCode));
    b.confirm_mfa_enrollment(&profile.user_id, &code)
        .await
        .expect("a later correct code may consume the live candidate");
}

#[tokio::test]
async fn pat_lookup_round_trip() {
    use crate::domain::auth::backend::pat::{self, MintedPat};
    let b = InMemoryAuthBackend::new();
    let profile = b.register_user(signup_req("t@e.f")).await.unwrap();
    let user_id: UserId = profile.user_id.parse().unwrap();
    let MintedPat { plaintext, record } =
        pat::mint_pat(user_id, "ci".to_owned(), vec![], None).unwrap();
    b.pats.insert(record.hash, record.clone());

    let resolved = b.lookup_pat(&plaintext).await.unwrap().expect("active");
    assert_eq!(resolved.id, record.id);
    // Wrong prefix is rejected by hash_for_lookup before the map probe.
    let bad = b
        .lookup_pat("nbl_sk_zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz")
        .await;
    assert!(matches!(bad, Err(AuthError::InvalidCredentials)));
}

#[tokio::test]
async fn oauth_start_persists_state_entry() {
    // GitHub's fixed runtime profile needs no discovery request, so this
    // unit test can exercise real authorization-URL emission offline.
    use std::collections::HashMap;

    use crate::config::{OAuthProviderConfig, OAuthProvidersConfig};
    use secrecy::SecretString;

    let mut providers = HashMap::new();
    providers.insert(
        OAuthProvider::GitHub,
        OAuthProviderConfig {
            client_id: SecretString::new("test-client".into()),
            client_secret: SecretString::new("test-secret".into()),
        },
    );
    let cfg = OAuthProvidersConfig { providers };
    let b = backend_with_oauth(cfg);
    let start = b
        .start_oauth(
            OAuthProvider::GitHub,
            "https://nebula.test/api/v1/auth/oauth/github/callback",
        )
        .await
        .unwrap();
    assert!(
        start.authorize_url.contains("state="),
        "authorize URL must include state query param: {}",
        start.authorize_url
    );
    assert!(
        start.authorize_url.contains("code_challenge_method=S256"),
        "authorize URL must include PKCE S256 marker"
    );
    let states = b.oauth_state.lock();
    let entry = states.get(&start.state).expect("state persisted");
    assert_eq!(
        entry.redirect_uri, "https://nebula.test/api/v1/auth/oauth/github/callback",
        "start must persist the handler-derived redirect_uri"
    );
}

#[tokio::test]
async fn oauth_start_fails_closed_at_the_storage_owned_capacity_before_egress() {
    use std::collections::HashMap;

    use crate::config::{OAuthProviderConfig, OAuthProvidersConfig};
    use secrecy::SecretString;

    let mut providers = HashMap::new();
    providers.insert(
        OAuthProvider::GitHub,
        OAuthProviderConfig {
            client_id: SecretString::new("test-client".into()),
            client_secret: SecretString::new("test-secret".into()),
        },
    );
    let backend = backend_with_oauth(OAuthProvidersConfig { providers });
    {
        let mut states = backend.oauth_state.lock();
        let expires_at = expiry_unix(OAUTH_STATE_TTL);
        for index in 0..OAUTH_STATE_CAPACITY {
            states.insert(
                format!("capacity-state-{index}"),
                OAuthStateEntry {
                    provider: OAuthProvider::GitHub,
                    code_verifier: "verifier".to_owned(),
                    expires_at,
                    redirect_uri: "https://nebula.test/api/v1/auth/oauth/github/callback"
                        .to_owned(),
                },
            );
        }
    }

    let error = backend
        .start_oauth(
            OAuthProvider::GitHub,
            "https://nebula.test/api/v1/auth/oauth/github/callback",
        )
        .await
        .expect_err("hard-cap saturation must fail closed");

    assert!(matches!(error, AuthError::RateLimit));
    assert_eq!(
        backend.oauth_state.lock().len(),
        OAUTH_STATE_CAPACITY as usize
    );
}

/// `start_oauth` returns `ProviderNotConfigured` when the provider is absent.
#[tokio::test]
async fn start_oauth_returns_provider_not_configured_when_provider_absent() {
    let b = InMemoryAuthBackend::new();
    let err = b
        .start_oauth(
            OAuthProvider::Google,
            "https://nebula.test/api/v1/auth/oauth/google/callback",
        )
        .await
        .expect_err("missing provider config must error");
    assert!(matches!(err, AuthError::ProviderNotConfigured));
}

/// GitHub uses the runtime-owned canonical authorize endpoint and scope.
#[tokio::test]
async fn start_oauth_emits_canonical_github_authorize_url() {
    use std::collections::HashMap;

    use crate::config::{OAuthProviderConfig, OAuthProvidersConfig};
    use secrecy::SecretString;

    let mut providers = HashMap::new();
    providers.insert(
        OAuthProvider::GitHub,
        OAuthProviderConfig {
            client_id: SecretString::new("gh-app-id".into()),
            client_secret: SecretString::new("gh-app-secret".into()),
        },
    );
    let cfg = OAuthProvidersConfig { providers };
    let b = backend_with_oauth(cfg);
    let start = b
        .start_oauth(
            OAuthProvider::GitHub,
            "https://nebula.test/api/v1/auth/oauth/github/callback",
        )
        .await
        .unwrap();
    let url = start.authorize_url;
    assert!(url.starts_with("https://github.com/login/oauth/authorize"));
    assert!(url.contains("client_id=gh-app-id"));
    assert!(url.contains("scope=user%3Aemail"));
    assert!(url.contains("code_challenge_method=S256"));
}

#[test]
fn concurrent_same_oauth_subject_converges_to_one_user() {
    let backend = Arc::new(InMemoryAuthBackend::new());
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut resolved = Vec::new();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for (email, session_id) in [
            ("left@example.test", "left-session"),
            ("right@example.test", "right-session"),
        ] {
            let backend = Arc::clone(&backend);
            let barrier = Arc::clone(&barrier);
            handles.push(scope.spawn(move || {
                barrier.wait();
                let prepared = oauth_material(session_id, "challenge", u64::MAX);
                backend
                    .finalize_oauth_login(
                        OAuthProvider::Google,
                        "stable-subject",
                        Some(email),
                        UserId::new(),
                        &prepared,
                    )
                    .map(|outcome| match outcome {
                        InMemoryOAuthFinalizeOutcome::SessionCreated(user) => user,
                        _ => panic!("verified email must create a session"),
                    })
                    .expect("finalization succeeds")
            }));
        }
        for handle in handles {
            resolved.push(handle.join().expect("worker does not panic"));
        }
    });

    assert_eq!(resolved[0].id, resolved[1].id);
    assert_eq!(backend.users.len(), 1);
    assert_eq!(backend.users_by_email.len(), 1);
    assert_eq!(backend.external_identities.len(), 1);
    assert_eq!(backend.sessions.len(), 2);
}

#[test]
fn concurrent_oauth_subjects_with_same_email_require_explicit_linking() {
    let backend = Arc::new(InMemoryAuthBackend::new());
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut resolved = Vec::new();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for (subject, session_id) in [
            ("left-subject", "left-session"),
            ("right-subject", "right-session"),
        ] {
            let backend = Arc::clone(&backend);
            let barrier = Arc::clone(&barrier);
            handles.push(scope.spawn(move || {
                barrier.wait();
                let prepared = oauth_material(session_id, "challenge", u64::MAX);
                backend.finalize_oauth_login(
                    OAuthProvider::GitHub,
                    subject,
                    Some("shared@example.test"),
                    UserId::new(),
                    &prepared,
                )
            }));
        }
        for handle in handles {
            resolved.push(handle.join().expect("worker does not panic"));
        }
    });

    assert!(matches!(
        (&resolved[0], &resolved[1]),
        (
            Ok(InMemoryOAuthFinalizeOutcome::SessionCreated(_)),
            Err(AuthError::AccountLinkRequired)
        ) | (
            Err(AuthError::AccountLinkRequired),
            Ok(InMemoryOAuthFinalizeOutcome::SessionCreated(_))
        )
    ));
    assert_eq!(backend.users.len(), 1);
    assert_eq!(backend.users_by_email.len(), 1);
    assert_eq!(backend.external_identities.len(), 1);
    assert_eq!(backend.sessions.len(), 1);
}

#[tokio::test]
async fn existing_verified_local_email_is_never_auto_linked() {
    let backend = InMemoryAuthBackend::new();
    let profile = backend
        .register_user(signup_req("owned@example.test"))
        .await
        .expect("seed local account");
    let user_id: UserId = profile.user_id.parse().expect("profile id is valid");
    backend.users.alter(&user_id, |_, mut user| {
        user.email_verified = true;
        user
    });

    let prepared = oauth_material("must-not-persist", "must-not-persist-challenge", u64::MAX);
    let result = backend.finalize_oauth_login(
        OAuthProvider::Google,
        "new-provider-subject",
        Some("owned@example.test"),
        UserId::new(),
        &prepared,
    );

    assert!(matches!(result, Err(AuthError::AccountLinkRequired)));
    assert_eq!(backend.users.len(), 1);
    assert_eq!(backend.external_identities.len(), 0);
    assert_eq!(backend.sessions.len(), 0);
}

#[tokio::test]
async fn linked_oauth_user_with_mfa_never_receives_a_session_before_totp() {
    let backend = InMemoryAuthBackend::new();
    let (profile, _) = enroll_mfa_user(&backend, "oauth-mfa@example.test").await;
    let user_id: UserId = profile.user_id.parse().expect("profile id is valid");
    backend.external_identities.insert(
        (
            OAuthProvider::GitHub.as_str().to_owned(),
            "linked-mfa-subject".to_owned(),
        ),
        user_id.as_bytes().to_vec(),
    );

    let prepared = oauth_material("must-not-persist", "oauth-mfa-challenge", u64::MAX);
    let outcome = backend
        .finalize_oauth_login(
            OAuthProvider::GitHub,
            "linked-mfa-subject",
            None,
            UserId::new(),
            &prepared,
        )
        .expect("linked identity resolves");

    assert!(matches!(outcome, InMemoryOAuthFinalizeOutcome::MfaRequired));
    assert_eq!(
        backend.sessions.len(),
        0,
        "Nebula MFA must gate session creation after OAuth first factor"
    );
    let challenge = backend
        .mfa_challenges
        .get("oauth-mfa-challenge")
        .expect("MFA challenge is persisted under the identity mutex");
    assert_eq!(challenge.user_id, user_id);
}

#[tokio::test]
async fn oauth_mfa_challenge_is_user_bound_single_use_and_expiring() {
    let backend = InMemoryAuthBackend::new();
    let (owner, owner_secret) = enroll_mfa_user(&backend, "oauth-owner@example.test").await;
    let owner_id: UserId = owner.user_id.parse().expect("owner id is valid");
    backend.external_identities.insert(
        (
            OAuthProvider::GitHub.as_str().to_owned(),
            "oauth-owner-subject".to_owned(),
        ),
        owner_id.as_bytes().to_vec(),
    );

    let owner_code = mfa::current_code(&owner_secret).expect("current owner code");
    let mut other_code = owner_code.clone();
    for index in 0..4 {
        let (_, other_secret) =
            enroll_mfa_user(&backend, &format!("oauth-other-{index}@example.test")).await;
        other_code = mfa::current_code(&other_secret).expect("current other-user code");
        if other_code != owner_code {
            break;
        }
    }
    assert_ne!(other_code, owner_code, "independent TOTP fixtures collided");

    let cross_user = oauth_material("cross-user-session", "cross-user-challenge", u64::MAX);
    let outcome = backend
        .finalize_oauth_login(
            OAuthProvider::GitHub,
            "oauth-owner-subject",
            None,
            UserId::new(),
            &cross_user,
        )
        .expect("issue owner challenge");
    assert!(matches!(outcome, InMemoryOAuthFinalizeOutcome::MfaRequired));
    let wrong_user = backend
        .verify_mfa("cross-user-challenge", &other_code)
        .await
        .expect_err("another user's TOTP must not satisfy the challenge");
    assert!(matches!(wrong_user, AuthError::InvalidMfaCode));
    assert_eq!(backend.sessions.len(), 0);
    let burned = backend
        .verify_mfa("cross-user-challenge", &owner_code)
        .await
        .expect_err("a failed challenge is still single-use");
    assert!(matches!(burned, AuthError::InvalidToken));

    let expired_material = oauth_material("expired-session", "expired-challenge", 0);
    backend
        .finalize_oauth_login(
            OAuthProvider::GitHub,
            "oauth-owner-subject",
            None,
            UserId::new(),
            &expired_material,
        )
        .expect("issue expired challenge fixture");
    let expired = backend
        .verify_mfa("expired-challenge", &owner_code)
        .await
        .expect_err("expired challenge must fail closed");
    assert!(matches!(expired, AuthError::InvalidToken));
    assert_eq!(backend.sessions.len(), 0);

    let fresh = oauth_material("fresh-session", "fresh-challenge", u64::MAX);
    backend
        .finalize_oauth_login(
            OAuthProvider::GitHub,
            "oauth-owner-subject",
            None,
            UserId::new(),
            &fresh,
        )
        .expect("issue fresh challenge");
    let verified = backend
        .verify_mfa("fresh-challenge", &owner_code)
        .await
        .expect("owner TOTP verifies the OAuth challenge");
    assert_eq!(verified.user_id, owner.user_id);
    assert_eq!(
        backend.sessions.len(),
        0,
        "verification alone mints no session"
    );
    let session = backend
        .create_session(&verified.user_id)
        .await
        .expect("session is minted only after local MFA verification");
    assert_eq!(session.principal, Principal::User(owner_id));
    let replay = backend
        .verify_mfa("fresh-challenge", &owner_code)
        .await
        .expect_err("successful OAuth MFA challenge cannot be replayed");
    assert!(matches!(replay, AuthError::InvalidToken));
}

#[tokio::test]
async fn with_email_port_routes_through_injected_port() {
    // Caller-owned EchoSink — the test keeps the Arc so it can
    // assert the injected port (not the default sink that was
    // dropped) actually saw the verification email.
    let custom = Arc::new(EchoSink::default());
    let custom_port: Arc<dyn EmailPort> = Arc::clone(&custom) as _;
    let backend = InMemoryAuthBackend::new().with_email_port(custom_port);

    backend
        .register_user(signup_req("inject@nebula.dev"))
        .await
        .expect("register must succeed against the injected port");

    // The injected port received the verification email.
    let captured = custom.peek();
    assert_eq!(
        captured.len(),
        1,
        "injected port must receive the verification email"
    );
    assert_eq!(captured[0].to, "inject@nebula.dev");
    assert_eq!(captured[0].kind, EmailKind::Verification);

    // The default echo handle was dropped by `with_email_port`, so
    // the back-compat `emails()` shim now returns an empty Vec —
    // proving the default sink is no longer the source of truth.
    assert!(
        backend.emails().is_empty(),
        "with_email_port must drop the default echo: `emails()` should be empty"
    );
}
