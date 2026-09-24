use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use nebula_storage_port::SecretBytes;

// Trait names referenced only by the tests now that `#[credential]`
// generates the trait impls via absolute paths: `Credential` (KEY /
// Properties), `CredentialLifecycle` (policy), and the capability
// sub-traits exercised by `assert_oauth2_capabilities`.
use crate::{AuthPattern, Credential, CredentialLifecycle, Interactive, Refreshable};

use super::*;

struct FixedAcquisitionTransport {
    status: u16,
    body: &'static [u8],
}

impl crate::runtime::AcquisitionTransport for FixedAcquisitionTransport {
    fn post_token<'a>(
        &'a self,
        _request: TokenPostRequest,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<TokenPostResponse, crate::runtime::AcquisitionTransportError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            TokenPostResponse::try_new(self.status, SecretBytes::new(self.body.to_vec()))
                .map_err(|_| crate::runtime::AcquisitionTransportError::ReadBody)
        })
    }
}

struct InspectingClientCredentialsRefresh {
    saw_saved_material: Arc<AtomicBool>,
}

struct FixedRefreshTransport {
    status: u16,
    body: &'static [u8],
}

struct CompletionTimestampTransport {
    completed_at: Arc<Mutex<Option<DateTime<Utc>>>>,
}

impl crate::runtime::RefreshTransport for CompletionTimestampTransport {
    fn post_token<'a>(
        &'a self,
        _request: TokenPostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<TokenPostResponse, crate::runtime::RefreshTransportError>>
                + Send
                + 'a,
        >,
    > {
        let completed_at = Arc::clone(&self.completed_at);
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let response_completed_at = Utc::now();
            *completed_at.lock().expect("completion timestamp lock") = Some(response_completed_at);
            TokenPostResponse::try_new(
                200,
                SecretBytes::new(
                    br#"{"access_token":"renewed-access","token_type":"Bearer","expires_in":0}"#
                        .to_vec(),
                ),
            )
            .map_err(|_| crate::runtime::RefreshTransportError::ReadBody)
        })
    }
}

impl crate::runtime::RefreshTransport for FixedRefreshTransport {
    fn post_token<'a>(
        &'a self,
        _request: TokenPostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<TokenPostResponse, crate::runtime::RefreshTransportError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            TokenPostResponse::try_new(self.status, SecretBytes::new(self.body.to_vec()))
                .map_err(|_| crate::runtime::RefreshTransportError::ReadBody)
        })
    }
}

impl crate::runtime::RefreshTransport for InspectingClientCredentialsRefresh {
    fn post_token<'a>(
        &'a self,
        request: TokenPostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<TokenPostResponse, crate::runtime::RefreshTransportError>>
                + Send
                + 'a,
        >,
    > {
        let saw_saved_material = Arc::clone(&self.saw_saved_material);
        Box::pin(async move {
            let form_matches = request.form().iter().any(|(key, value)| {
                key == "grant_type" && value.expose_secret() == "client_credentials"
            }) && request
                .form()
                .iter()
                .any(|(key, value)| key == "scope" && value.expose_secret() == "read write")
                && request.form().iter().all(|(key, value)| {
                    key != "refresh_token"
                        && value.expose_secret() != CLIENT_CREDENTIALS_REFRESH_MARKER
                });
            let basic_matches = request.basic_auth().is_some_and(|(client_id, secret)| {
                client_id.expose_secret() == "test_client_id"
                    && secret.expose_secret() == "test_client_secret"
            });
            let endpoint_matches =
                request.endpoint().expose_url().as_str() == "https://idp.example.com/token";
            saw_saved_material.store(
                form_matches && basic_matches && endpoint_matches,
                Ordering::SeqCst,
            );
            TokenPostResponse::try_new(
                200,
                SecretBytes::new(
                    br#"{"access_token":"renewed-access","token_type":"Bearer","expires_in":3600,"scope":"read write"}"#
                        .to_vec(),
                ),
            )
            .map_err(|_| crate::runtime::RefreshTransportError::ReadBody)
        })
    }
}

fn acquisition_context(status: u16, body: &'static [u8]) -> CredentialContext {
    CredentialContext::for_owner("test-user")
        .for_acquisition(Arc::new(FixedAcquisitionTransport { status, body }))
}

fn make_state() -> OAuth2State {
    OAuth2State {
        access_token: SecretString::new("tok_abc"),
        token_type: "Bearer".into(),
        refresh_token: Some(SecretString::new("ref_xyz")),
        expires_at: Some(Utc::now() + chrono::Duration::seconds(3600)),
        scopes: vec!["read".into(), "write".into()],
        client_id: SecretString::new("cid"),
        client_secret: SecretString::new("csecret"),
        token_url: "https://example.com/token?routing=state-url-canary".into(),
        auth_style: AuthStyle::Header,
    }
}

#[test]
fn key_is_oauth2() {
    assert_eq!(OAuth2Credential::KEY, "oauth2");
}

#[test]
fn lifecycle_policy_reflects_refresh_token_presence() {
    // With a refresh token the runtime can renew non-interactively.
    let with_token = make_state();
    let p = OAuth2Credential::policy(&with_token);
    assert_eq!(p.refresh, RefreshStrategy::RefreshToken);
    assert_eq!(p.revoke, RevokeStrategy::None);
    assert!(p.is_auto_renewable());
    assert!(p.is_expiring());

    // Without a refresh token the credential must re-acquire (the refresh
    // path would return ReauthRequired).
    let mut without = make_state();
    without.refresh_token = None;
    let p2 = OAuth2Credential::policy(&without);
    assert_eq!(
        p2.refresh,
        RefreshStrategy::ReAcquire {
            from: None,
            interactive: true
        }
    );
    assert!(!p2.is_auto_renewable());
}

// Capability membership names only the implemented provider paths.
#[expect(dead_code)]
fn assert_oauth2_capabilities()
where
    OAuth2Credential: Credential + Interactive + Refreshable,
{
}

#[test]
fn project_extracts_oauth2_token() {
    let state = make_state();
    let token = OAuth2Credential::project(&state);

    let header = token.bearer_header();
    // bearer_header returns SecretString per §15.5 — exposure happens at
    // the assertion site (test scope), never in production logs.
    assert!(header.expose_secret().contains("tok_abc"));
    assert_eq!(token.scopes, vec!["read", "write"]);
    assert!(token.expires_at.is_some());
}

#[test]
fn project_excludes_refresh_internals() {
    let state = make_state();
    let token = OAuth2Credential::project(&state);

    // OAuth2Token should not expose refresh_token, client_id, client_secret
    let serialized = serde_json::to_value(&token).unwrap();
    assert!(serialized.get("refresh_token").is_none());
    assert!(serialized.get("client_id").is_none());
    assert!(serialized.get("client_secret").is_none());
}

#[test]
fn metadata_has_correct_fields() {
    let mut registry = crate::CredentialRegistry::new();
    registry
        .register(OAuth2Credential, "oauth2-metadata-test")
        .expect("valid OAuth2 definition");
    let meta = registry
        .metadata(OAuth2Credential::KEY)
        .expect("registered OAuth2 metadata");
    assert_eq!(meta.pattern(), AuthPattern::OAuth2);
}

#[test]
fn properties_schema_is_admissible() {
    let params = nebula_schema::schema_of::<<OAuth2Credential as Credential>::Properties>()
        .expect("valid OAuth2 schema");
    assert!(!params.properties().is_empty());
}

const TEST_CALLBACK: &str = "https://app.example.com/oauth2/callback";

fn auth_code_pending() -> OAuth2Pending {
    OAuth2Pending {
        config: OAuth2Config::authorization_code(TEST_CALLBACK)
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .build(),
        client_id: "cid".into(),
        client_secret: SecretString::new("cs"),
        auth_style: AuthStyle::Header,
        pkce_verifier: SecretString::new("verifier_value"),
        state: "expected_state".into(),
        redirect_uri: TEST_CALLBACK.into(),
    }
}

#[tokio::test]
async fn continue_resolve_rejects_wrong_input_for_auth_code() {
    let pending = auth_code_pending();
    let ctx = CredentialContext::for_owner("test-user");
    let result = OAuth2Credential::continue_resolve(&pending, &UserInput::Poll, &ctx).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn continue_resolve_rejects_callback_without_code() {
    let pending = auth_code_pending();
    let ctx = CredentialContext::for_owner("test-user");
    let input = UserInput::Callback {
        params: HashMap::new(),
    };
    let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn continue_resolve_rejects_callback_missing_state_param() {
    let pending = auth_code_pending();
    let ctx = CredentialContext::for_owner("test-user");
    let mut params = HashMap::new();
    params.insert("code".to_owned(), "the_code".to_owned());
    let input = UserInput::Callback { params };
    let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
    assert!(matches!(result, Err(CredentialError::InvalidInput)));
}

#[tokio::test]
async fn continue_resolve_rejects_wrong_state() {
    let pending = auth_code_pending();
    let ctx = CredentialContext::for_owner("test-user");
    let mut params = HashMap::new();
    params.insert("code".to_owned(), "the_code".to_owned());
    params.insert("state".to_owned(), "attacker_state".to_owned());
    let input = UserInput::Callback { params };
    let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
    assert!(matches!(result, Err(CredentialError::InvalidInput)));
}

#[tokio::test]
async fn continue_resolve_rejects_length_mismatched_state() {
    let mut pending = auth_code_pending();
    pending.state = "aaa".into();
    let ctx = CredentialContext::for_owner("test-user");
    let mut params = HashMap::new();
    params.insert("code".to_owned(), "c".to_owned());
    params.insert("state".to_owned(), "aaaa".to_owned()); // longer
    let input = UserInput::Callback { params };
    let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
    assert!(matches!(result, Err(CredentialError::InvalidInput)));
}

#[tokio::test]
async fn continue_resolve_rejects_oversized_callback_before_dispatch() {
    let mut pending = auth_code_pending();
    pending.state = "A".repeat(43);
    let ctx = CredentialContext::for_owner("test-user");
    let input = UserInput::Callback {
        params: [
            ("code".to_owned(), "A".repeat(16 * 1024 + 1)),
            ("state".to_owned(), pending.state.clone()),
        ]
        .into(),
    };

    let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;

    assert!(matches!(result, Err(CredentialError::InvalidInput)));
}

// ── Authorization-code kickoff coverage ────────────────────────────

fn auth_code_properties() -> OAuth2AuthorizationCodeProperties {
    OAuth2AuthorizationCodeProperties {
        client: OAuth2ClientProperties {
            client_id: "test_client_id".to_owned(),
            client_secret: SecretString::new("test_client_secret"),
        },
        auth_url: "https://idp.example.com/authorize".to_owned(),
        token_url: "https://idp.example.com/token".to_owned(),
        scopes: Some(vec!["read".to_owned(), "write".to_owned()]),
        redirect_uri: TEST_CALLBACK.to_owned(),
        auth_style: AuthStyle::Header,
    }
}

fn client_credentials_properties() -> OAuth2ClientCredentialsProperties {
    OAuth2ClientCredentialsProperties {
        client: OAuth2ClientProperties {
            client_id: "test_client_id".to_owned(),
            client_secret: SecretString::new("test_client_secret"),
        },
        token_url: "https://idp.example.com/token".to_owned(),
        scopes: Some(vec!["read".to_owned(), "write".to_owned()]),
        auth_style: AuthStyle::Header,
    }
}

#[tokio::test]
async fn client_credentials_resolve_completes_through_acquisition_transport() {
    let properties = OAuth2Properties::ClientCredentials(client_credentials_properties());
    let ctx = acquisition_context(
        200,
        br#"{"access_token":"access-canary","token_type":"Bearer","expires_in":3600,"scope":"read write"}"#,
    );

    let result = OAuth2Credential::resolve(&properties, &ctx)
        .await
        .expect("client credentials exchange should succeed");
    let StaticResolveResult::Complete(state) = result else {
        panic!("client credentials must complete in one exchange");
    };
    assert_eq!(state.access_token.expose_secret(), "access-canary");
    assert_eq!(state.scopes, ["read", "write"]);
    assert!(!format!("{state:?}").contains("access-canary"));
}

#[tokio::test]
async fn client_credentials_ignores_provider_refresh_token_and_repeats_its_grant() {
    let properties = OAuth2Properties::ClientCredentials(client_credentials_properties());
    let acquired = OAuth2Credential::resolve(
        &properties,
        &acquisition_context(
            200,
            br#"{"access_token":"first-access","token_type":"Bearer","refresh_token":"provider-extension-token","expires_in":1,"scope":"read write"}"#,
        ),
    )
    .await
    .expect("initial client credentials exchange succeeds");
    let StaticResolveResult::Complete(mut state) = acquired else {
        panic!("client credentials resolve must complete");
    };
    assert!(has_client_credentials_marker(&state));

    let saw_saved_material = Arc::new(AtomicBool::new(false));
    let transport = Arc::new(InspectingClientCredentialsRefresh {
        saw_saved_material: Arc::clone(&saw_saved_material),
    });
    let ctx = CredentialContext::for_owner("test-user").for_refresh_critical_section(transport);
    let report = OAuth2Credential::refresh(
        &mut state,
        RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider),
    )
    .await
    .into_kind();

    assert!(matches!(
        report,
        crate::contract::RefreshReportKind::ProviderRefreshed
    ));
    assert!(saw_saved_material.load(Ordering::SeqCst));
    assert!(has_client_credentials_marker(&state));
}

#[tokio::test]
async fn expired_client_credentials_without_refresh_token_repeat_exchange() {
    let properties = OAuth2Properties::ClientCredentials(client_credentials_properties());
    let acquired = OAuth2Credential::resolve(
        &properties,
        &acquisition_context(
            200,
            br#"{"access_token":"first-access","token_type":"Bearer","expires_in":1,"scope":"read write"}"#,
        ),
    )
    .await
    .expect("initial client credentials exchange succeeds");
    let StaticResolveResult::Complete(mut state) = acquired else {
        panic!("client credentials resolve must complete");
    };
    assert!(has_client_credentials_marker(&state));
    let projected = OAuth2Credential::project(&state);
    assert!(!format!("{projected:?}").contains("client_credentials.v1"));
    assert!(!format!("{state:?}").contains("client_credentials.v1"));
    assert_eq!(
        OAuth2Credential::policy(&state).refresh,
        RefreshStrategy::RefreshToken
    );

    let saw_saved_material = Arc::new(AtomicBool::new(false));
    let transport = Arc::new(InspectingClientCredentialsRefresh {
        saw_saved_material: Arc::clone(&saw_saved_material),
    });
    let ctx = CredentialContext::for_owner("test-user").for_refresh_critical_section(transport);
    let report = OAuth2Credential::refresh(
        &mut state,
        RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider),
    )
    .await
    .into_kind();

    assert!(matches!(
        report,
        crate::contract::RefreshReportKind::ProviderRefreshed
    ));
    assert_eq!(state.access_token.expose_secret(), "renewed-access");
    assert!(has_client_credentials_marker(&state));
    assert!(saw_saved_material.load(Ordering::SeqCst));
    assert!(!format!("{state:?}").contains("renewed-access"));
    assert!(!format!("{state:?}").contains("test_client_secret"));
}

#[tokio::test]
async fn client_credentials_transient_or_malformed_400_does_not_require_reauth() {
    for body in [
        br#"{"error":"temporarily_unavailable"}"#.as_slice(),
        b"gateway generated a malformed response".as_slice(),
    ] {
        let mut state = make_state();
        state.refresh_token = Some(SecretString::new(CLIENT_CREDENTIALS_REFRESH_MARKER));
        let transport = Arc::new(FixedRefreshTransport { status: 400, body });
        let ctx = CredentialContext::for_owner("test-user").for_refresh_critical_section(transport);

        let report = OAuth2Credential::refresh(
            &mut state,
            RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider),
        )
        .await
        .into_kind();

        assert!(matches!(
            report,
            crate::contract::RefreshReportKind::OutcomeUnknown
        ));
        assert!(has_client_credentials_marker(&state));
    }
}

#[tokio::test]
async fn client_credentials_invalid_client_is_definitive_reauth() {
    let mut state = make_state();
    state.refresh_token = Some(SecretString::new(CLIENT_CREDENTIALS_REFRESH_MARKER));
    let transport = Arc::new(FixedRefreshTransport {
        status: 400,
        body: br#"{"error":"invalid_client"}"#,
    });
    let ctx = CredentialContext::for_owner("test-user").for_refresh_critical_section(transport);

    let report = OAuth2Credential::refresh(
        &mut state,
        RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider),
    )
    .await
    .into_kind();

    assert!(matches!(
        report,
        crate::contract::RefreshReportKind::ReauthRequired {
            reason: crate::resolve::ReauthReason::ProviderRejected,
            phase: crate::contract::RefreshReauthPhase::ProviderConfirmed,
        }
    ));
    assert!(has_client_credentials_marker(&state));
}

#[tokio::test]
async fn client_credentials_expiry_starts_after_completed_response() {
    let mut state = make_state();
    state.refresh_token = Some(SecretString::new(CLIENT_CREDENTIALS_REFRESH_MARKER));
    let completed_at = Arc::new(Mutex::new(None));
    let transport = Arc::new(CompletionTimestampTransport {
        completed_at: Arc::clone(&completed_at),
    });
    let ctx = CredentialContext::for_owner("test-user").for_refresh_critical_section(transport);

    let report = OAuth2Credential::refresh(
        &mut state,
        RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider),
    )
    .await
    .into_kind();

    assert!(matches!(
        report,
        crate::contract::RefreshReportKind::ProviderRefreshed
    ));
    let response_completed_at = completed_at
        .lock()
        .expect("completion timestamp lock")
        .expect("transport records completed response time");
    assert!(
        state.expires_at.expect("provider returned expires_in") >= response_completed_at,
        "token lifetime must start after the completed response"
    );
}

#[tokio::test]
async fn authorization_code_begin_and_callback_complete_through_acquisition_transport() {
    let properties = OAuth2Properties::AuthorizationCode(auth_code_properties());
    let begin = OAuth2Credential::begin(&properties, &CredentialContext::for_owner("test-user"))
        .await
        .expect("authorization kickoff should succeed");
    let ResolveResult::Pending { state: pending, .. } = begin else {
        panic!("authorization code must begin with pending state");
    };
    let input = UserInput::Callback {
        params: HashMap::from([
            ("code".to_owned(), "authorization-code".to_owned()),
            ("state".to_owned(), pending.state.clone()),
        ]),
    };
    let ctx = acquisition_context(
        200,
        br#"{"access_token":"callback-access","token_type":"bearer","refresh_token":"callback-refresh","scope":"read"}"#,
    );

    let completed = OAuth2Credential::continue_resolve(&pending, &input, &ctx)
        .await
        .expect("authorization code exchange should succeed");
    let ResolveResult::Complete(state) = completed else {
        panic!("valid callback must complete acquisition");
    };
    assert_eq!(state.access_token.expose_secret(), "callback-access");
    assert_eq!(
        state
            .refresh_token
            .as_ref()
            .expect("provider supplied a refresh token")
            .expose_secret(),
        "callback-refresh"
    );
}

#[tokio::test]
async fn acquisition_without_runtime_transport_fails_structurally() {
    let properties = OAuth2Properties::ClientCredentials(client_credentials_properties());
    let error = OAuth2Credential::resolve(&properties, &CredentialContext::for_owner("test-user"))
        .await
        .expect_err("initial acquisition authority must be runtime-stamped");
    assert!(matches!(
        error,
        CredentialError::AcquisitionTransportUnavailable
    ));
}

#[tokio::test]
async fn provider_rejection_and_malformed_success_are_payload_free() {
    let properties = OAuth2Properties::ClientCredentials(client_credentials_properties());
    let rejected = OAuth2Credential::resolve(
        &properties,
        &acquisition_context(401, b"provider-secret-canary"),
    )
    .await
    .expect_err("provider rejection must fail");
    assert!(!format!("{rejected:?} {rejected}").contains("provider-secret-canary"));

    let malformed = OAuth2Credential::resolve(
        &properties,
        &acquisition_context(200, b"malformed-success-secret-canary"),
    )
    .await
    .expect_err("malformed success has an unknown provider outcome");
    assert!(matches!(malformed, CredentialError::OutcomeUnknown));
    assert!(!format!("{malformed:?} {malformed}").contains("secret-canary"));
}

#[test]
fn provider_default_scopes_are_accepted_only_within_fixed_bounds() {
    let defaults = SecretString::new("provider.read provider.write");
    assert_eq!(
        parse_granted_scopes(Some(&defaults), &[]).expect("bounded provider defaults"),
        ["provider.read", "provider.write"]
    );

    let too_many = SecretString::new(
        (0..=64)
            .map(|index| format!("scope{index}"))
            .collect::<Vec<_>>()
            .join(" "),
    );
    assert!(matches!(
        parse_granted_scopes(Some(&too_many), &[]),
        Err(CredentialError::OutcomeUnknown)
    ));

    let oversized = SecretString::new("s".repeat(257));
    assert!(matches!(
        parse_granted_scopes(Some(&oversized), &[]),
        Err(CredentialError::OutcomeUnknown)
    ));
}

#[test]
fn explicit_scope_request_rejects_unrequested_grants() {
    let returned = SecretString::new("read admin");
    assert!(matches!(
        parse_granted_scopes(Some(&returned), &["read".to_owned()]),
        Err(CredentialError::OutcomeUnknown)
    ));
}

#[test]
fn oauth_properties_keep_client_secret_protected_until_explicit_extraction() {
    let properties = auth_code_properties();
    assert_eq!(
        properties.client.client_secret.expose_secret(),
        "test_client_secret"
    );
    assert!(!format!("{:?}", properties.client.client_secret).contains("test_client_secret"));
}

#[tokio::test]
async fn initiate_authorization_code_returns_redirect_with_pkce_and_state() {
    let properties = auth_code_properties();
    let (pending, request) =
        initiate_authorization_code(&properties).expect("kickoff should succeed");

    let url = match request {
        InteractionRequest::Redirect { url } => url,
        other => panic!("expected Redirect, got {other:?}"),
    };

    // RFC 6749 §4.1.1 + RFC 7636 PKCE — mandatory query parameters.
    assert!(url.contains("response_type=code"), "missing response_type");
    assert!(url.contains("client_id="), "missing client_id");
    assert!(url.contains("redirect_uri="), "missing redirect_uri");
    assert!(url.contains("scope="), "missing scope");
    assert!(url.contains("state="), "missing state");
    assert!(url.contains("code_challenge="), "missing code_challenge");
    assert!(
        url.contains("code_challenge_method=S256"),
        "missing or wrong code_challenge_method"
    );

    // Pending state populated for AuthorizationCode flow per §15.4.
    assert!(!pending.pkce_verifier.is_empty());
    assert!(!pending.state.is_empty());
    assert_eq!(pending.redirect_uri, TEST_CALLBACK);
}

#[tokio::test]
async fn initiate_authorization_code_csrf_state_is_unguessable() {
    let properties = auth_code_properties();
    let (pending1, _) = initiate_authorization_code(&properties).expect("first kickoff");
    let (pending2, _) = initiate_authorization_code(&properties).expect("second kickoff");

    let state1 = &pending1.state;
    let state2 = &pending2.state;

    assert_ne!(
        state1, state2,
        "anti-CSRF state must be unguessable across kickoffs"
    );

    // `generate_random_state` produces ≥128 bits of base64-encoded
    // entropy → at least 22 base64 chars.
    assert!(
        state1.len() >= 22,
        "state token should carry ≥128 bits of entropy: got {} chars",
        state1.len()
    );
}

#[tokio::test]
async fn refresh_returns_reauth_when_no_refresh_token() {
    let mut state = OAuth2State {
        access_token: SecretString::new("tok"),
        token_type: "Bearer".into(),
        refresh_token: None,
        expires_at: None,
        scopes: vec![],
        client_id: SecretString::new("cid"),
        client_secret: SecretString::new("cs"),
        token_url: "https://t.com/token".into(),
        auth_style: AuthStyle::Header,
    };

    let ctx = CredentialContext::for_owner("test-user");
    let outcome = OAuth2Credential::refresh(
        &mut state,
        RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider),
    )
    .await
    .into_kind();
    // Locally detected: never spoke to the IdP. Distinct from
    // `ProviderRejected` per wave-2 review (see ReauthReason rustdoc).
    assert!(matches!(
        outcome,
        crate::contract::RefreshReportKind::ReauthRequired {
            reason: crate::resolve::ReauthReason::MissingRefreshMaterial,
            phase: crate::contract::RefreshReauthPhase::BeforeDispatch,
        }
    ));
}

#[tokio::test]
async fn refresh_without_runtime_transport_is_exact_and_bounded_retryable() {
    let mut state = make_state();
    let ctx = CredentialContext::for_owner("test-user");
    let report = OAuth2Credential::refresh(
        &mut state,
        RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider),
    )
    .await
    .into_kind();

    let crate::contract::RefreshReportKind::NotApplied(context) = report else {
        panic!("missing runtime transport must be a proven not-applied refresh");
    };
    assert_eq!(
        context.phase(),
        crate::RefreshNotAppliedPhase::BeforeDispatch
    );
    assert_eq!(context.kind(), RefreshErrorKind::ProviderUnavailable);
    let RetryAdvice::After(delay) = context.retry() else {
        panic!("runtime composition can recover without a credential material update");
    };
    assert_eq!(delay.get(), Duration::from_mins(1));
}

#[test]
fn state_is_expired_with_margin() {
    let state = OAuth2State {
        access_token: SecretString::new("tok"),
        token_type: "Bearer".into(),
        refresh_token: None,
        expires_at: Some(Utc::now() + chrono::Duration::seconds(30)),
        scopes: vec![],
        client_id: SecretString::new("cid"),
        client_secret: SecretString::new("cs"),
        token_url: "https://t.com/token".into(),
        auth_style: AuthStyle::Header,
    };
    // Expires in 30s, margin is 60s => expired
    assert!(state.is_expired(Duration::from_mins(1)));
    // Margin is 0 => not expired
    assert!(!state.is_expired(Duration::from_secs(0)));
}

#[test]
fn no_expiry_never_expired() {
    let state = OAuth2State {
        access_token: SecretString::new("tok"),
        token_type: "Bearer".into(),
        refresh_token: None,
        expires_at: None,
        scopes: vec![],
        client_id: SecretString::new("cid"),
        client_secret: SecretString::new("cs"),
        token_url: "https://t.com/token".into(),
        auth_style: AuthStyle::Header,
    };
    assert!(!state.is_expired(Duration::from_secs(9999)));
}

#[test]
fn pending_state_zeroizes_all_fields_including_pkce_verifier_state_redirect() {
    let mut pending = OAuth2Pending {
        config: OAuth2Config::authorization_code(TEST_CALLBACK)
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .build(),
        client_id: "cid".into(),
        client_secret: SecretString::new("super_secret"),
        auth_style: AuthStyle::Header,
        pkce_verifier: SecretString::new("verifier_contents"),
        state: "state_contents".into(),
        redirect_uri: TEST_CALLBACK.into(),
    };

    pending.zeroize();
    assert!(pending.config.auth_url.is_empty());
    assert!(pending.config.token_url.is_empty());
    assert!(pending.config.scopes.is_empty());
    assert!(pending.config.redirect_uri.is_none());
    assert!(pending.client_secret.expose_secret().is_empty());
    assert!(pending.client_id.is_empty());
    assert!(pending.pkce_verifier.expose_secret().is_empty());
    assert!(pending.state.is_empty());
    assert!(pending.redirect_uri.is_empty());
}

#[test]
fn pending_state_debug_is_constant_and_redacts_all_urls() {
    let first = OAuth2Pending {
        config: OAuth2Config::authorization_code(TEST_CALLBACK)
            .auth_url("https://a.com/auth?diagnostic=auth-url-canary")
            .token_url("https://a.com/token?diagnostic=token-url-canary")
            .build(),
        client_id: "cid".into(),
        client_secret: SecretString::new("cs"),
        auth_style: AuthStyle::Header,
        pkce_verifier: SecretString::new("my_pkce_verifier_value"),
        state: "my_csrf_state_value".into(),
        redirect_uri: TEST_CALLBACK.into(),
    };
    let second = OAuth2Pending {
        config: OAuth2Config::authorization_code("https://different.example/callback")
            .auth_url("https://different.example/long/authorize/path")
            .token_url("https://different.example/token")
            .build(),
        client_id: "different-client".into(),
        client_secret: SecretString::new("different-secret"),
        auth_style: AuthStyle::PostBody,
        pkce_verifier: SecretString::new("different-verifier"),
        state: "different-state".into(),
        redirect_uri: "https://different.example/callback".into(),
    };
    let debug = format!("{first:?}");
    assert_eq!(debug, format!("{second:?}"));
    assert_eq!(debug, "OAuth2Pending(<redacted>)");
    for canary in [
        "my_pkce_verifier_value",
        "my_csrf_state_value",
        "auth-url-canary",
        "token-url-canary",
        TEST_CALLBACK,
    ] {
        assert!(!debug.contains(canary));
    }
}

#[test]
fn pending_state_expires_in_10_minutes() {
    let pending = OAuth2Pending {
        config: OAuth2Config::authorization_code(TEST_CALLBACK)
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .build(),
        client_id: "cid".into(),
        client_secret: SecretString::new("cs"),
        auth_style: AuthStyle::Header,
        pkce_verifier: SecretString::new("verifier"),
        state: "state".into(),
        redirect_uri: TEST_CALLBACK.into(),
    };

    assert_eq!(pending.expires_in(), Duration::from_mins(10));
}

#[test]
fn credential_state_v2_kind_and_version() {
    assert_eq!(OAuth2State::KIND, "oauth2");
    assert_eq!(OAuth2State::VERSION, 1);
}

#[test]
fn pending_state_kind() {
    assert_eq!(OAuth2Pending::KIND, "oauth2_pending");
}

#[test]
fn bearer_header_format() {
    let state = make_state();
    // bearer_header returns SecretString per §15.5 — exposure happens at
    // the assertion site (test scope), never in production logs.
    assert_eq!(state.bearer_header().expose_secret(), "Bearer tok_abc");
}

#[test]
fn oauth2_state_debug_redacts_secrets() {
    let state = make_state();
    let debug = format!("{state:?}");
    assert!(!debug.contains("tok_abc"), "access_token leaked in Debug");
    assert!(!debug.contains("ref_xyz"), "refresh_token leaked in Debug");
    assert!(!debug.contains("csecret"), "client_secret leaked in Debug");
    assert!(
        !debug.contains("state-url-canary"),
        "query-bearing token_url leaked in Debug"
    );
    assert!(!debug.contains("example.com"));
    assert!(debug.contains("[REDACTED]"));
    assert!(debug.contains("Bearer"));
}

#[test]
fn oauth2_state_zeroize_scrubs_token_url() {
    let mut state = make_state();
    state.zeroize();
    assert!(state.token_url.is_empty());
}

#[test]
fn build_auth_url_parse_failure_is_fixed_and_input_free() {
    let config = OAuth2Config::authorization_code(TEST_CALLBACK)
        .auth_url("://auth-url-diagnostic-canary")
        .token_url("https://provider.example/token")
        .build();

    let error = build_auth_url(&config, "client", "challenge", "state")
        .expect_err("invalid authorization endpoint must fail");
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains("auth-url-diagnostic-canary"));
    assert!(!diagnostic.contains("://"));
    assert!(diagnostic.contains("invalid OAuth2 authorization endpoint URL"));
}

#[test]
fn build_auth_url_rejects_endpoints_outside_oauth_egress_policy() {
    for auth_url in [
        "http://provider.example/authorize",
        "https://localhost/authorize",
        "https://127.0.0.1/authorize",
        "https://user@provider.example/authorize",
        "https://provider.example/authorize#fragment",
        "https://provider.example/authorize?state=attacker",
    ] {
        let config = OAuth2Config::authorization_code(TEST_CALLBACK)
            .auth_url(auth_url)
            .token_url("https://provider.example/token")
            .build();

        let result = build_auth_url(&config, "client", "challenge", "state");
        assert!(
            result.is_err(),
            "accepted authorization endpoint {auth_url}"
        );
    }
}

#[test]
fn build_auth_url_rejects_an_oversized_final_redirect() {
    let config = OAuth2Config::authorization_code(TEST_CALLBACK)
        .auth_url("https://provider.example/authorize")
        .token_url("https://provider.example/token")
        .scopes(["a".repeat(8 * 1024)])
        .build();

    let result = build_auth_url(&config, "client", "challenge", "state");
    assert!(result.is_err(), "accepted an oversized authorization URL");
}

#[test]
fn oauth2_state_serde_round_trip() {
    let state = make_state();

    // Default sink (logs, responses): every secret field redacts.
    let redacted = serde_json::to_string(&state).unwrap();
    assert!(
        !redacted.contains("tok_abc"),
        "access_token leaked to default serde sink"
    );
    assert!(
        !redacted.contains("ref_xyz"),
        "refresh_token leaked to default serde sink"
    );
    assert!(
        !redacted.contains("csecret"),
        "client_secret leaked to default serde sink"
    );

    // Storage scope preserves them for encrypted-at-rest persistence.
    let json =
        crate::serde_secret::expose_for_serialization(|| serde_json::to_string(&state)).unwrap();
    let restored: OAuth2State = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.access_token.expose_secret(), "tok_abc");
    assert_eq!(
        restored.refresh_token.as_ref().unwrap().expose_secret(),
        "ref_xyz"
    );
    assert_eq!(restored.client_id.expose_secret(), "cid");
    assert_eq!(restored.client_secret.expose_secret(), "csecret");
}
