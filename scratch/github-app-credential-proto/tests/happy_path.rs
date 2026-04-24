//! Happy path: single replica resolves + refreshes GitHub App credential.
//!
//! Demonstrates:
//! - Credential trait works for multi-step refresh flow
//! - JWT RS256 signing works with SecretString-wrapped PEM
//! - Wiremock validates Bearer JWT exchange
//! - project() yields OAuth2Token с installation_token as Bearer

mod common;

use github_app_credential_proto::{
    GitHubAppCredential, GitHubAppState, refresh_github_app_token,
};
use nebula_credential::{Credential, CredentialContext, SecretString};

#[tokio::test]
async fn refresh_populates_installation_token() {
    let (mock, counter) = common::start_mock_github().await;

    let mut state = GitHubAppState {
        app_id: "12345".to_string(),
        installation_id: "99999".to_string(),
        private_key_pem: SecretString::new(common::TEST_RSA_PRIVATE_PEM.to_string()),
        api_base_url: mock.uri(),
        installation_token: None,
        token_expires_at: None,
    };

    // Before refresh — no token.
    assert!(state.installation_token.is_none());
    assert_eq!(counter.count(), 0);

    // Refresh.
    refresh_github_app_token(&mut state).await.expect("refresh succeeds");

    // After refresh — token populated.
    let token = state
        .installation_token
        .as_ref()
        .expect("token populated")
        .expose_secret()
        .to_string();
    assert!(token.starts_with("ghs_mock_token_hit_"));
    assert!(state.token_expires_at.is_some());

    // Mock should have been hit exactly once.
    assert_eq!(counter.count(), 1);
}

#[tokio::test]
async fn project_produces_bearer_with_installation_token() {
    let (mock, _counter) = common::start_mock_github().await;

    let mut state = GitHubAppState {
        app_id: "42".to_string(),
        installation_id: "1".to_string(),
        private_key_pem: SecretString::new(common::TEST_RSA_PRIVATE_PEM.to_string()),
        api_base_url: mock.uri(),
        installation_token: None,
        token_expires_at: None,
    };

    refresh_github_app_token(&mut state).await.expect("refresh succeeds");

    let scheme = GitHubAppCredential::project(&state);
    let header = scheme.bearer_header();

    assert!(header.starts_with("Bearer ghs_mock_token_hit_"));
}

#[tokio::test]
async fn refresh_fails_on_bad_jwt_rejection() {
    // Mock server returns 401 for wrong Bearer.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path_regex(
            r"^/app/installations/[^/]+/access_tokens$",
        ))
        .respond_with(wiremock::ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "message": "Bad credentials"
        })))
        .mount(&server)
        .await;

    let mut state = GitHubAppState {
        app_id: "12345".to_string(),
        installation_id: "99999".to_string(),
        private_key_pem: SecretString::new(common::TEST_RSA_PRIVATE_PEM.to_string()),
        api_base_url: server.uri(),
        installation_token: None,
        token_expires_at: None,
    };

    let err = refresh_github_app_token(&mut state)
        .await
        .expect_err("401 should propagate");

    assert!(format!("{err}").contains("401"), "error should mention status: {err}");
    assert!(state.installation_token.is_none(), "token unchanged on failure");
}

/// Uses Credential trait's full resolve() path (not the bare refresh function).
#[tokio::test]
async fn credential_trait_resolve_builds_initial_state() {
    let (mock, _counter) = common::start_mock_github().await;

    let values = nebula_schema::FieldValues::from_json(serde_json::json!({
        "app_id": "777",
        "installation_id": "333",
        "private_key_pem": common::TEST_RSA_PRIVATE_PEM.trim(),
        "api_base_url": mock.uri(),
    }))
    .expect("values parse");

    // Minimal CredentialContext for tests.
    let ctx = CredentialContext::for_test("scratch-tester");

    let result = GitHubAppCredential::resolve(&values, &ctx)
        .await
        .expect("resolve succeeds");

    use nebula_credential::resolve::ResolveResult;
    match result {
        ResolveResult::Complete(state) => {
            assert_eq!(state.app_id, "777");
            assert_eq!(state.installation_id, "333");
            assert!(state.installation_token.is_none());
        }
        _ => panic!("expected ResolveResult::Complete"),
    }
}
