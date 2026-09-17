use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

use super::*;
use crate::transport::oauth::test_support::{TestResponse, TlsFixture};

const TEST_DNS_ANSWER: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));

type TestRuntimeConfig = (
    OAuthProvidersConfig,
    HashMap<OAuthProvider, OAuthTestProviderProfile>,
);

fn provider_config(provider: OAuthProvider, client_id: &str) -> OAuthProvidersConfig {
    OAuthProvidersConfig {
        providers: HashMap::from([(
            provider,
            OAuthProviderConfig {
                client_id: SecretString::new(client_id.to_owned().into_boxed_str()),
                client_secret: SecretString::new("test-secret".to_owned().into_boxed_str()),
            },
        )]),
    }
}

fn manual_config(fixture: &TlsFixture) -> TestRuntimeConfig {
    (
        provider_config(OAuthProvider::GitHub, "test-client"),
        HashMap::from([(
            OAuthProvider::GitHub,
            OAuthTestProviderProfile::manual(
                "https://accounts.example.com/authorize".to_owned(),
                fixture.endpoint("/token"),
                fixture.endpoint("/userinfo"),
                Some(fixture.endpoint("/emails")),
                vec!["user:email".to_owned()],
            ),
        )]),
    )
}

fn oidc_config(fixture: &TlsFixture) -> TestRuntimeConfig {
    (
        provider_config(OAuthProvider::Google, "test-client"),
        HashMap::from([(
            OAuthProvider::Google,
            OAuthTestProviderProfile::oidc(fixture.endpoint("/discovery")),
        )]),
    )
}

fn google_manual_test_config(fixture: &TlsFixture) -> TestRuntimeConfig {
    (
        provider_config(OAuthProvider::Google, "google-client-id"),
        HashMap::from([(
            OAuthProvider::Google,
            OAuthTestProviderProfile::manual(
                "https://accounts.example.com/authorize".to_owned(),
                fixture.endpoint("/token"),
                fixture.endpoint("/userinfo"),
                None,
                GOOGLE_SCOPES
                    .iter()
                    .map(|scope| (*scope).to_owned())
                    .collect(),
            ),
        )]),
    )
}

fn runtime_for_fixture(
    (config, profiles): TestRuntimeConfig,
    fixture: &TlsFixture,
) -> OAuthIdentityRuntime {
    OAuthIdentityRuntime::from_config_for_test(
        config,
        profiles,
        fixture.trust_anchor(),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        vec![TEST_DNS_ANSWER],
    )
    .expect("fixed-policy test runtime must build")
    .expect("non-empty provider config must enable OAuth")
}

async fn wait_for_discovery_state(
    slot: &Arc<Mutex<DiscoveryState>>,
    ready: impl Fn(&DiscoveryState) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let is_ready = {
                let state = slot.lock().await;
                ready(&state)
            };
            if is_ready {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("background discovery flight published its terminal state");
}

#[test]
fn empty_config_disables_oauth_before_egress_construction() {
    let runtime = OAuthIdentityRuntime::from_config_with_egress(
        OAuthProvidersConfig::default(),
        HashMap::new(),
        || panic!("empty provider config must not construct outbound egress"),
    )
    .expect("empty provider config is a valid disabled state");

    assert!(runtime.is_none());
}

#[test]
fn runtime_debug_exposes_only_policy_shape() {
    const CLIENT_CANARY: &str = "CLIENT_CANARY_DO_NOT_LOG";
    const SECRET_CANARY: &str = "SECRET_CANARY_DO_NOT_LOG";
    let providers = HashMap::from([(
        OAuthProvider::GitHub,
        OAuthProviderConfig {
            client_id: SecretString::new(CLIENT_CANARY.to_owned().into_boxed_str()),
            client_secret: SecretString::new(SECRET_CANARY.to_owned().into_boxed_str()),
        },
    )]);
    let runtime = OAuthIdentityRuntime::from_config(OAuthProvidersConfig { providers })
        .expect("valid runtime config must build")
        .expect("non-empty config must enable OAuth");

    let debug = format!("{runtime:?}");
    assert!(debug.contains("configured_provider_count: 1"));
    for canary in [CLIENT_CANARY, SECRET_CANARY] {
        assert!(!debug.contains(canary), "runtime Debug leaked {canary}");
    }
}

#[tokio::test]
async fn identity_truth_table_uses_only_attested_email_sources() {
    let inline = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
        "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"Bearer"}"#),
        "/userinfo" => TestResponse::json(r#"{"id":41,"email":"unattested@example.com"}"#),
        "/emails" => {
            TestResponse::json(r#"[{"email":" User@Example.COM ","primary":true,"verified":true}]"#)
        },
        _ => TestResponse::failure(404),
    })
    .await;
    let runtime = runtime_for_fixture(manual_config(&inline), &inline);
    let pending = runtime
        .begin_identity_completion(
            runtime.begin_deadline(),
            OAuthProvider::GitHub,
            "state",
            "code",
            "https://nebula.example/api/v1/auth/oauth/github/callback",
            "verifier",
        )
        .await
        .expect("inline verified identity must resolve");
    assert_eq!(pending.subject(), "41");
    assert_eq!(
        runtime
            .resolve_verified_identity(pending)
            .await
            .expect("GitHub verified email must normalize")
            .into_string(),
        "user@example.com"
    );
    assert_eq!(
        inline
            .requests()
            .into_iter()
            .map(|request| request.path)
            .collect::<Vec<_>>(),
        ["/token", "/userinfo", "/emails"]
    );

    let fallback = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
        "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"bearer"}"#),
        "/userinfo" => TestResponse::json(r#"{"id":42,"email":"unattested@example.com"}"#),
        "/emails" => TestResponse::json(
            r#"[{"email":"other@example.com","primary":false,"verified":true},{"email":"Primary@Example.COM","primary":true,"verified":true}]"#,
        ),
        _ => TestResponse::failure(404),
    })
    .await;
    let runtime = runtime_for_fixture(manual_config(&fallback), &fallback);
    let pending = runtime
        .begin_identity_completion(
            runtime.begin_deadline(),
            OAuthProvider::GitHub,
            "state",
            "code",
            "https://nebula.example/api/v1/auth/oauth/github/callback",
            "verifier",
        )
        .await
        .expect("fallback identity must resolve");
    assert_eq!(pending.subject(), "42");
    assert_eq!(
        runtime
            .resolve_verified_identity(pending)
            .await
            .expect("primary verified fallback must resolve")
            .into_string(),
        "primary@example.com"
    );
    assert_eq!(
        fallback
            .requests()
            .into_iter()
            .map(|request| request.path)
            .collect::<Vec<_>>(),
        ["/token", "/userinfo", "/emails"]
    );

    let rejected = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
        "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"Bearer"}"#),
        "/userinfo" => TestResponse::json(r#"{"id":43}"#),
        "/emails" => TestResponse::json(
            r#"[{"email":"rejected@example.com","primary":false,"verified":true}]"#,
        ),
        _ => TestResponse::failure(404),
    })
    .await;
    let runtime = runtime_for_fixture(manual_config(&rejected), &rejected);
    let pending = runtime
        .begin_identity_completion(
            runtime.begin_deadline(),
            OAuthProvider::GitHub,
            "state",
            "code",
            "https://nebula.example/api/v1/auth/oauth/github/callback",
            "verifier",
        )
        .await
        .expect("userinfo response itself is structurally valid");
    let error = match runtime.resolve_verified_identity(pending).await {
        Ok(_) => panic!("non-primary GitHub email is not signup evidence"),
        Err(error) => error,
    };
    assert_eq!(error, OAuthFailureCode::VerifiedEmailUnavailable);
    assert_eq!(
        rejected
            .requests()
            .into_iter()
            .map(|request| request.path)
            .collect::<Vec<_>>(),
        ["/token", "/userinfo", "/emails"]
    );
}

#[tokio::test]
async fn discovery_singleflight_cache_and_one_hour_ttl_are_enforced() {
    let discovery = TlsFixture::spawn("oauth.test", |request, _| {
        assert_eq!(request.path, "/discovery");
        TestResponse::json(
            r#"{"issuer":"https://accounts.google.com","authorization_endpoint":"https://accounts.example.com/authorize","token_endpoint":"https://token.example.com/token","userinfo_endpoint":"https://userinfo.example.com/user","jwks_uri":"https://keys.example.com/jwks"}"#,
        )
        .delayed(Duration::from_millis(75))
    })
    .await;
    let runtime = Arc::new(runtime_for_fixture(oidc_config(&discovery), &discovery));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..16 {
        let runtime = Arc::clone(&runtime);
        tasks.spawn(async move {
            let deadline = runtime.begin_deadline();
            runtime
                .build_authorization_url(
                    &deadline,
                    OAuthProvider::Google,
                    "https://nebula.example/api/v1/auth/oauth/google/callback",
                    &format!("state-{index}"),
                    &format!("challenge-{index}"),
                )
                .await
        });
    }
    while let Some(result) = tasks.join_next().await {
        result
            .expect("singleflight task must join")
            .expect("every follower must receive discovered endpoints");
    }
    assert_eq!(
        discovery.requests().len(),
        1,
        "followers must share one fetch"
    );

    let slot = Arc::clone(
        runtime
            .discovery
            .get(&OAuthProvider::Google)
            .expect("provider cache slot must exist")
            .value(),
    );
    let mut state = slot.lock().await;
    let expires_at = state
        .cached
        .as_ref()
        .expect("successful discovery must be cached")
        .expires_at;
    let remaining = expires_at.saturating_duration_since(Instant::now());
    assert!(remaining <= DISCOVERY_TTL);
    assert!(remaining > Duration::from_mins(59));
    state
        .cached
        .as_mut()
        .expect("cache entry remains present")
        .expires_at = Instant::now() - Duration::from_millis(1);
    drop(state);

    let deadline = runtime.begin_deadline();
    runtime
        .build_authorization_url(
            &deadline,
            OAuthProvider::Google,
            "https://nebula.example/api/v1/auth/oauth/google/callback",
            "state-after-expiry",
            "challenge-after-expiry",
        )
        .await
        .expect("expired discovery cache must refetch");
    assert_eq!(discovery.requests().len(), 2);
}

#[tokio::test]
async fn discovery_flight_survives_initiator_abort_and_serves_follower() {
    let discovery = TlsFixture::spawn("oauth.test", |_request, _| {
        TestResponse::json(
            r#"{"issuer":"https://accounts.google.com","authorization_endpoint":"https://accounts.example.com/authorize","token_endpoint":"https://token.example.com/token","userinfo_endpoint":"https://userinfo.example.com/user"}"#,
        )
        .delayed(Duration::from_millis(100))
    })
    .await;
    let runtime = Arc::new(runtime_for_fixture(oidc_config(&discovery), &discovery));
    let initiating_runtime = Arc::clone(&runtime);
    let initiating = tokio::spawn(async move {
        let deadline = initiating_runtime.begin_deadline();
        initiating_runtime
            .build_authorization_url(
                &deadline,
                OAuthProvider::Google,
                "https://nebula.example/api/v1/auth/oauth/google/callback",
                "initiator-state",
                "initiator-challenge",
            )
            .await
    });
    discovery.wait_for_request_count(1).await;
    initiating.abort();
    assert!(
        initiating
            .await
            .expect_err("initiating caller must be cancelled")
            .is_cancelled()
    );

    let follower_deadline = runtime.begin_deadline();
    runtime
        .build_authorization_url(
            &follower_deadline,
            OAuthProvider::Google,
            "https://nebula.example/api/v1/auth/oauth/google/callback",
            "follower-state",
            "follower-challenge",
        )
        .await
        .expect("follower must receive the background-owned flight result");
    assert_eq!(discovery.requests().len(), 1);
}

#[tokio::test]
async fn discovery_flight_completes_cache_after_every_caller_cancels() {
    let discovery = TlsFixture::spawn("oauth.test", |_request, _| {
        TestResponse::json(
            r#"{"issuer":"https://accounts.google.com","authorization_endpoint":"https://accounts.example.com/authorize","token_endpoint":"https://token.example.com/token","userinfo_endpoint":"https://userinfo.example.com/user"}"#,
        )
        .delayed(Duration::from_millis(100))
    })
    .await;
    let runtime = Arc::new(runtime_for_fixture(oidc_config(&discovery), &discovery));
    let mut callers = Vec::new();
    for index in 0..4 {
        let runtime = Arc::clone(&runtime);
        callers.push(tokio::spawn(async move {
            let deadline = runtime.begin_deadline();
            runtime
                .build_authorization_url(
                    &deadline,
                    OAuthProvider::Google,
                    "https://nebula.example/api/v1/auth/oauth/google/callback",
                    &format!("cancelled-state-{index}"),
                    &format!("cancelled-challenge-{index}"),
                )
                .await
        }));
    }
    discovery.wait_for_request_count(1).await;
    for caller in callers {
        caller.abort();
    }

    let slot = Arc::clone(
        runtime
            .discovery
            .get(&OAuthProvider::Google)
            .expect("provider cache slot must exist")
            .value(),
    );
    wait_for_discovery_state(&slot, |state| {
        state.cached.is_some() && state.in_flight.is_none()
    })
    .await;

    let later_deadline = runtime.begin_deadline();
    runtime
        .build_authorization_url(
            &later_deadline,
            OAuthProvider::Google,
            "https://nebula.example/api/v1/auth/oauth/google/callback",
            "later-state",
            "later-challenge",
        )
        .await
        .expect("later caller must use the completed cache");
    assert_eq!(discovery.requests().len(), 1);
}

#[tokio::test]
async fn timed_out_discovery_caller_still_installs_failure_cooldown() {
    let discovery = TlsFixture::spawn("oauth.test", |_request, _| {
        TestResponse::failure(503).delayed(Duration::from_millis(100))
    })
    .await;
    let runtime = runtime_for_fixture(oidc_config(&discovery), &discovery);
    let short_deadline = OAuthFlowDeadline {
        expires_at: Instant::now() + Duration::from_millis(20),
    };
    assert_eq!(
        runtime
            .build_authorization_url(
                &short_deadline,
                OAuthProvider::Google,
                "https://nebula.example/api/v1/auth/oauth/google/callback",
                "timed-out-state",
                "timed-out-challenge",
            )
            .await
            .expect_err("caller's own deadline must still apply"),
        OAuthFailureCode::CompletionTimeout
    );

    let slot = Arc::clone(
        runtime
            .discovery
            .get(&OAuthProvider::Google)
            .expect("provider cooldown slot must exist")
            .value(),
    );
    wait_for_discovery_state(&slot, |state| {
        state.retry_not_before.is_some() && state.in_flight.is_none()
    })
    .await;

    let later_deadline = runtime.begin_deadline();
    assert_eq!(
        runtime
            .build_authorization_url(
                &later_deadline,
                OAuthProvider::Google,
                "https://nebula.example/api/v1/auth/oauth/google/callback",
                "later-state",
                "later-challenge",
            )
            .await
            .expect_err("cooldown must fail without refetch"),
        OAuthFailureCode::DiscoveryUnavailable
    );
    assert_eq!(discovery.requests().len(), 1);
}

#[tokio::test]
async fn discovery_failure_cooldown_suppresses_refetch_for_five_seconds() {
    let discovery = TlsFixture::spawn("oauth.test", |_request, _| TestResponse::failure(503)).await;
    let runtime = runtime_for_fixture(oidc_config(&discovery), &discovery);

    for _ in 0..2 {
        let deadline = runtime.begin_deadline();
        assert_eq!(
            runtime
                .build_authorization_url(
                    &deadline,
                    OAuthProvider::Google,
                    "https://nebula.example/api/v1/auth/oauth/google/callback",
                    "state",
                    "challenge",
                )
                .await
                .expect_err("failed discovery must remain unavailable"),
            OAuthFailureCode::DiscoveryUnavailable
        );
    }
    assert_eq!(
        discovery.requests().len(),
        1,
        "cooldown must suppress retry"
    );

    let slot = Arc::clone(
        runtime
            .discovery
            .get(&OAuthProvider::Google)
            .expect("provider cooldown slot must exist")
            .value(),
    );
    let mut state = slot.lock().await;
    let retry_at = state
        .retry_not_before
        .expect("failed discovery must install cooldown");
    let remaining = retry_at.saturating_duration_since(Instant::now());
    assert!(remaining <= DISCOVERY_FAILURE_COOLDOWN);
    assert!(remaining > Duration::from_secs(4));
    state.retry_not_before = Some(Instant::now() - Duration::from_millis(1));
    drop(state);

    let deadline = runtime.begin_deadline();
    let _ = runtime
        .build_authorization_url(
            &deadline,
            OAuthProvider::Google,
            "https://nebula.example/api/v1/auth/oauth/google/callback",
            "state-after-cooldown",
            "challenge-after-cooldown",
        )
        .await;
    assert_eq!(
        discovery.requests().len(),
        2,
        "elapsed cooldown permits retry"
    );
}

#[tokio::test]
async fn verified_email_fallback_reuses_the_original_absolute_deadline() {
    let fixture = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
        "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"Bearer"}"#)
            .delayed(Duration::from_millis(80)),
        "/userinfo" => TestResponse::json(r#"{"id":44}"#).delayed(Duration::from_millis(80)),
        "/emails" => TestResponse::json(
            r#"[{"email":"verified@example.com","primary":true,"verified":true}]"#,
        )
        .delayed(Duration::from_millis(150)),
        _ => TestResponse::failure(404),
    })
    .await;
    let runtime = runtime_for_fixture(manual_config(&fixture), &fixture);
    let deadline = OAuthFlowDeadline {
        expires_at: Instant::now() + Duration::from_millis(250),
    };
    let original_expiry = deadline.expires_at;
    let pending = runtime
        .begin_identity_completion(
            deadline,
            OAuthProvider::GitHub,
            "state",
            "code",
            "https://nebula.example/api/v1/auth/oauth/github/callback",
            "verifier",
        )
        .await
        .expect("primary identity stages fit within the shared budget");
    assert_eq!(pending.deadline.expires_at, original_expiry);
    let error = match runtime.resolve_verified_identity(pending).await {
        Ok(_) => panic!("fallback must not receive a fresh deadline"),
        Err(error) => error,
    };
    assert_eq!(error, OAuthFailureCode::CompletionTimeout);
    fixture.wait_for_request_count(3).await;
}

fn encoded_test_id_token(header_alg: &str, claims: serde_json::Value) -> SecretString {
    let header = serde_json::json!({"alg": header_alg, "typ": "JWT"});
    let compact = format!(
        "{}.{}.{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).expect("serialize JWT header")),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("serialize JWT claims")),
        URL_SAFE_NO_PAD.encode(b"syntactic-test-signature")
    );
    SecretString::new(compact.into_boxed_str())
}

fn valid_google_claims(
    now: i64,
    client_id: &str,
    state: &str,
    access_token: &str,
) -> serde_json::Value {
    serde_json::json!({
        "iss": GOOGLE_ISSUER,
        "sub": "google-subject-123",
        "aud": client_id,
        "exp": now + 600,
        "iat": now - 10,
        "nonce": nonce_for_state(state),
        "at_hash": expected_oidc_at_hash(access_token),
    })
}

#[test]
fn oidc_at_hash_matches_the_core_rs256_vector() {
    assert_eq!(
        expected_oidc_at_hash("jHkWEdUXMU1BwAsC4vtUsZwnNvTIxEl0z9K3vx5KF0Y"),
        "77QmUPtjPfzWtF2AnpK9RQ"
    );
}

#[test]
fn google_id_token_validation_rejects_every_identity_binding_mismatch() {
    const CLIENT_ID: &str = "google-client-id";
    const STATE: &str = "state-with-enough-randomness-for-test";
    const ACCESS_TOKEN: &str = "google-access-token";
    let now = 1_800_000_000;
    let client_id = SecretString::new(CLIENT_ID.to_owned().into_boxed_str());
    let access_token = SecretString::new(ACCESS_TOKEN.to_owned().into_boxed_str());

    let valid = encoded_test_id_token(
        "RS256",
        valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN),
    );
    let subject = validate_google_id_token(
        &valid,
        &access_token,
        &client_id,
        &nonce_for_state(STATE),
        now,
    )
    .expect("complete direct-token-endpoint evidence is valid");
    assert_eq!(subject.as_str(), "google-subject-123");

    let mut invalid = Vec::new();
    invalid.push(encoded_test_id_token(
        "none",
        valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN),
    ));
    for (field, value) in [
        ("iss", serde_json::json!("https://issuer.example.test")),
        ("aud", serde_json::json!("other-client")),
        ("exp", serde_json::json!(now - 61)),
        ("iat", serde_json::json!(now + 61)),
        ("nonce", serde_json::json!("wrong-nonce")),
        ("at_hash", serde_json::json!("wrong-at-hash")),
    ] {
        let mut claims = valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN);
        claims[field] = value;
        invalid.push(encoded_test_id_token("RS256", claims));
    }
    let mut missing_at_hash = valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN);
    missing_at_hash
        .as_object_mut()
        .expect("claims are an object")
        .remove("at_hash");
    invalid.push(encoded_test_id_token("RS256", missing_at_hash));
    let mut multi_audience = valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN);
    multi_audience["aud"] = serde_json::json!([CLIENT_ID, CLIENT_ID]);
    invalid.push(encoded_test_id_token("RS256", multi_audience));
    let mut wrong_azp = valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN);
    wrong_azp["azp"] = serde_json::json!("other-client");
    invalid.push(encoded_test_id_token("RS256", wrong_azp));
    let mut co_audience = valid_google_claims(now, CLIENT_ID, STATE, ACCESS_TOKEN);
    co_audience["aud"] = serde_json::json!([CLIENT_ID, "other-client"]);
    co_audience["azp"] = serde_json::json!(CLIENT_ID);
    invalid.push(encoded_test_id_token("RS256", co_audience));

    for id_token in invalid {
        let error = match validate_google_id_token(
            &id_token,
            &access_token,
            &client_id,
            &nonce_for_state(STATE),
            now,
        ) {
            Ok(_) => panic!("mismatched ID-token evidence must fail closed"),
            Err(error) => error,
        };
        assert_eq!(error, OAuthFailureCode::ProviderResponseInvalid);
    }
}

#[test]
fn google_signup_email_requires_gmail_or_matching_hosted_domain() {
    let gmail = validate_google_email(GoogleEmailEvidence {
        email: Some("User@Gmail.COM".to_owned()),
        email_verified: Some(true),
        hosted_domain: None,
    })
    .expect("verified Gmail is provisionable");
    assert_eq!(gmail.into_string(), "user@gmail.com");

    let workspace = validate_google_email(GoogleEmailEvidence {
        email: Some("User@Example.COM".to_owned()),
        email_verified: Some(true),
        hosted_domain: Some("example.com".to_owned()),
    })
    .expect("matching hosted domain is provisionable");
    assert_eq!(workspace.into_string(), "user@example.com");

    for evidence in [
        GoogleEmailEvidence {
            email: Some("user@example.com".to_owned()),
            email_verified: Some(true),
            hosted_domain: None,
        },
        GoogleEmailEvidence {
            email: Some("user@example.com".to_owned()),
            email_verified: Some(true),
            hosted_domain: Some("other.example".to_owned()),
        },
        GoogleEmailEvidence {
            email: Some("user@gmail.com".to_owned()),
            email_verified: Some(false),
            hosted_domain: None,
        },
    ] {
        let error = match validate_google_email(evidence) {
            Ok(_) => panic!("ineligible Google email evidence must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error, OAuthFailureCode::VerifiedEmailUnavailable);
    }
}

#[tokio::test]
async fn google_missing_or_mismatched_at_hash_stops_before_userinfo() {
    const STATE: &str = "google-state-for-at-hash-test";
    const ACCESS_TOKEN: &str = "google-access-token";

    let mut missing_claims =
        valid_google_claims(unix_timestamp(), "google-client-id", STATE, ACCESS_TOKEN);
    missing_claims
        .as_object_mut()
        .expect("claims are an object")
        .remove("at_hash");
    let missing_id_token = encoded_test_id_token("RS256", missing_claims);
    let missing_token_body = serde_json::json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "Bearer",
        "id_token": missing_id_token.expose_secret(),
    })
    .to_string();
    let missing = TlsFixture::spawn("oauth.test", move |request, _| {
        match request.path.as_str() {
            "/token" => TestResponse::json(missing_token_body.clone()),
            "/userinfo" => TestResponse::json(r#"{"sub":"must-not-be-fetched"}"#),
            _ => TestResponse::failure(404),
        }
    })
    .await;
    let runtime = runtime_for_fixture(google_manual_test_config(&missing), &missing);
    let result = runtime
        .begin_identity_completion(
            runtime.begin_deadline(),
            OAuthProvider::Google,
            STATE,
            "code",
            "https://nebula.example/api/v1/auth/oauth/google/callback",
            "verifier",
        )
        .await;
    assert!(matches!(
        result,
        Err(OAuthFailureCode::ProviderResponseInvalid)
    ));
    assert_eq!(
        missing
            .requests()
            .into_iter()
            .map(|request| request.path)
            .collect::<Vec<_>>(),
        ["/token"]
    );

    let mut claims = valid_google_claims(unix_timestamp(), "google-client-id", STATE, ACCESS_TOKEN);
    claims["at_hash"] = serde_json::json!("mismatched-at-hash");
    let id_token = encoded_test_id_token("RS256", claims);
    let token_body = serde_json::json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "Bearer",
        "id_token": id_token.expose_secret(),
    })
    .to_string();
    let mismatch = TlsFixture::spawn("oauth.test", move |request, _| {
        match request.path.as_str() {
            "/token" => TestResponse::json(token_body.clone()),
            "/userinfo" => TestResponse::json(r#"{"sub":"must-not-be-fetched"}"#),
            _ => TestResponse::failure(404),
        }
    })
    .await;
    let runtime = runtime_for_fixture(google_manual_test_config(&mismatch), &mismatch);
    let result = runtime
        .begin_identity_completion(
            runtime.begin_deadline(),
            OAuthProvider::Google,
            STATE,
            "code",
            "https://nebula.example/api/v1/auth/oauth/google/callback",
            "verifier",
        )
        .await;
    assert!(matches!(
        result,
        Err(OAuthFailureCode::ProviderResponseInvalid)
    ));
    assert_eq!(
        mismatch
            .requests()
            .into_iter()
            .map(|request| request.path)
            .collect::<Vec<_>>(),
        ["/token"]
    );
}

#[test]
fn pending_identity_and_wire_types_have_no_debug_surface() {
    static_assertions::assert_not_impl_any!(PendingExternalIdentity: std::fmt::Debug, Clone);
    static_assertions::assert_not_impl_any!(VerifiedEmailCapability: std::fmt::Debug, Clone);
    static_assertions::assert_not_impl_any!(TokenWireResponse: std::fmt::Debug);
    static_assertions::assert_not_impl_any!(GoogleUserinfoWire: std::fmt::Debug);
    static_assertions::assert_not_impl_any!(GitHubUserinfoWire: std::fmt::Debug);
    static_assertions::assert_not_impl_any!(ProvisionableEmail: std::fmt::Debug, Clone);
}

#[test]
fn provider_identity_fields_are_normalized_and_bounded() {
    assert_eq!(
        normalize_verified_email(" User@Example.COM ".to_owned()).expect("valid email normalizes"),
        "user@example.com"
    );
    for email in [
        "   ".to_owned(),
        "missing-at".to_owned(),
        "a@b@c".to_owned(),
        "a b@example.com".to_owned(),
        "a@-example.com".to_owned(),
        "a@example..com".to_owned(),
        "x".repeat(255),
    ] {
        assert!(normalize_verified_email(email).is_err());
    }
    for subject in ["", " leading", "trailing ", "line\nbreak"] {
        assert!(validate_subject(subject).is_err());
    }
    assert!(validate_subject(&"s".repeat(256)).is_err());
    assert!(validate_subject(&"s".repeat(255)).is_ok());
}

#[test]
fn token_wire_requires_non_empty_bearer_token() {
    for invalid in [
        r#"{"access_token":"","token_type":"Bearer"}"#,
        r#"{"access_token":"secret","token_type":"MAC"}"#,
        r#"{"access_token":"secret"}"#,
    ] {
        let parsed = serde_json::from_str::<TokenWireResponse>(invalid);
        assert!(
            parsed.is_err()
                || parsed.is_ok_and(|token| {
                    token.access_token.expose_secret().is_empty()
                        || !token.token_type.eq_ignore_ascii_case("bearer")
                })
        );
    }
    let token: TokenWireResponse =
        serde_json::from_str(r#"{"access_token":"secret","token_type":"bEaReR"}"#)
            .expect("case-insensitive bearer is valid");
    assert_eq!(token.token_type, "bEaReR");

    for invalid in [
        " leading",
        "trailing ",
        "embedded space",
        "line\nbreak",
        "tab\tbreak",
        "",
    ] {
        assert!(!valid_access_token(invalid), "accepted token {invalid:?}");
    }
    assert!(!valid_access_token(&"x".repeat(16 * 1024 + 1)));
    assert!(valid_access_token(&"x".repeat(16 * 1024)));
}

#[test]
fn oidc_token_auth_method_selection_is_normative_and_closed() {
    assert_eq!(
        select_discovered_token_auth_method(None).expect("OIDC omitted-field default"),
        TokenEndpointAuthMethod::ClientSecretBasic
    );
    assert_eq!(
        select_discovered_token_auth_method(Some(&[
            "client_secret_post".to_owned(),
            "client_secret_basic".to_owned(),
        ]))
        .expect("Basic is preferred when both are advertised"),
        TokenEndpointAuthMethod::ClientSecretBasic
    );
    assert_eq!(
        select_discovered_token_auth_method(Some(&["client_secret_post".to_owned()]))
            .expect("post is supported when Basic is absent"),
        TokenEndpointAuthMethod::ClientSecretPost
    );
    assert_eq!(
        select_discovered_token_auth_method(Some(&["private_key_jwt".to_owned()]))
            .expect_err("unsupported-only discovery list must fail closed"),
        OAuthFailureCode::DiscoveryUnavailable
    );
}
