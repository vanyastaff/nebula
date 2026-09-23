use nebula_storage_port::SecretBytes;

use crate::runtime::TokenPostResponse;

use super::*;

fn sample_state() -> OAuth2State {
    OAuth2State {
        access_token: SecretString::new("old-token"),
        token_type: "Bearer".to_owned(),
        refresh_token: Some(SecretString::new("refresh-1")),
        expires_at: None,
        scopes: vec!["read".to_owned(), "write".to_owned()],
        grant_type: crate::GrantType::AuthorizationCode,
        client_id: SecretString::new("client"),
        client_secret: SecretString::new("secret"),
        token_url: "https://example.com/token".to_owned(),
        auth_style: AuthStyle::Header,
    }
}

fn parse_success(raw: &[u8]) -> TokenSuccessResponse {
    serde_json::from_slice(raw).expect("valid typed token response")
}

fn token_response(status: u16, body: &[u8]) -> TokenPostResponse {
    TokenPostResponse::try_new(status, SecretBytes::new(body.to_vec()))
        .expect("test response is policy-valid")
}

#[test]
fn update_state_requires_access_token() {
    let mut state = sample_state();
    let body = parse_success(br#"{"token_type":"Bearer"}"#);
    let err = update_state_from_token_response(&mut state, body).unwrap_err();
    assert_eq!(err, MalformedTokenSuccess);
}

#[test]
fn update_state_applies_refresh_response_fields() {
    let mut state = sample_state();
    let body = parse_success(
        br#"{
            "access_token":"new-token",
            "token_type":"bearer",
            "refresh_token":"refresh-2",
            "expires_in":3600,
            "scope":"read write"
        }"#,
    );
    update_state_from_token_response(&mut state, body).expect("response should apply");

    assert_eq!(state.access_token.expose_secret(), "new-token");
    assert_eq!(state.token_type, "Bearer");
    assert_eq!(state.scopes, vec!["read".to_owned(), "write".to_owned()]);
    assert_eq!(
        state
            .refresh_token
            .as_ref()
            .expect("refresh token")
            .expose_secret(),
        "refresh-2"
    );
    assert!(state.expires_at.is_some());
}

#[test]
fn missing_expires_in_clears_the_previous_tokens_deadline() {
    let mut state = sample_state();
    state.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
    let body = parse_success(br#"{"access_token":"new-token","token_type":"Bearer"}"#);

    update_state_from_token_response(&mut state, body).expect("response should apply");

    assert!(
        state.expires_at.is_none(),
        "a new token without expires_in must not inherit an expired deadline"
    );
}

#[test]
fn rejected_provider_metadata_does_not_partially_mutate_state() {
    for raw in [
        br#"{"access_token":"new-token","token_type":"secret-canary"}"#.as_slice(),
        br#"{"access_token":"new-token"}"#.as_slice(),
        br#"{"access_token":"","token_type":"Bearer"}"#.as_slice(),
        br#"{"access_token":"has space","token_type":"Bearer"}"#.as_slice(),
        br#"{"access_token":"has\u0000control","token_type":"Bearer"}"#.as_slice(),
        br#"{"access_token":"t\u00f6k\u00e9n","token_type":"Bearer"}"#.as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","refresh_token":""}"#.as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","refresh_token":"has space"}"#
            .as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","refresh_token":"t\u00f6k\u00e9n"}"#
            .as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","scope":"read attacker-scope"}"#
            .as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","scope":"read read"}"#.as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","scope":""}"#.as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","scope":"read  write"}"#.as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","scope":"read\twrite"}"#.as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","scope":"read\u0022write"}"#
            .as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","scope":"read\u005cwrite"}"#
            .as_slice(),
        br#"{"access_token":"new-token","token_type":"Bearer","expires_in":18446744073709551615}"#
            .as_slice(),
    ] {
        let mut state = sample_state();
        let old_access = state.access_token.expose_secret().to_owned();
        let old_refresh = state
            .refresh_token
            .as_ref()
            .expect("sample refresh token")
            .expose_secret()
            .to_owned();
        let body = parse_success(raw);

        let error = update_state_from_token_response(&mut state, body)
            .expect_err("invalid provider metadata must fail");

        assert_eq!(error, MalformedTokenSuccess);
        assert_eq!(state.access_token.expose_secret(), old_access);
        assert_eq!(
            state
                .refresh_token
                .as_ref()
                .expect("refresh token retained")
                .expose_secret(),
            old_refresh
        );
        assert_eq!(state.token_type, "Bearer");
        assert_eq!(state.scopes, vec!["read", "write"]);
        assert!(state.expires_at.is_none());
    }
}

#[test]
fn exact_vschar_boundaries_and_case_insensitive_bearer_are_accepted() {
    let mut state = sample_state();
    let body = parse_success(
        br#"{
            "access_token":"!~",
            "token_type":"bEaReR",
            "refresh_token":"!~"
        }"#,
    );

    update_state_from_token_response(&mut state, body).expect("VSCHAR boundaries are valid");

    assert_eq!(state.access_token.expose_secret(), "!~");
    assert_eq!(state.token_type, "Bearer");
    assert_eq!(
        state
            .refresh_token
            .as_ref()
            .expect("returned refresh token")
            .expose_secret(),
        "!~"
    );
}

#[test]
fn header_auth_form_encodes_each_raw_component_before_basic_join() {
    let cases = [
        ("client:name", "client%3Aname"),
        ("client%name", "client%25name"),
        ("client+name", "client%2Bname"),
        ("client name", "client+name"),
        ("cliënt", "cli%C3%ABnt"),
    ];

    for (raw, expected) in cases {
        let mut state = sample_state();
        state.client_id = SecretString::new(raw);
        state.client_secret = SecretString::new(raw);

        let prepared = prepare_oauth2_refresh(&state).expect("sample state prepares a request");
        let request = prepared.into_request();
        let observed = request
            .basic_auth()
            .map(|(client_id, client_secret)| {
                (
                    client_id.expose_secret().to_owned(),
                    client_secret.expose_secret().to_owned(),
                )
            })
            .expect("Header auth must carry Basic components");
        assert_eq!(observed.0, expected);
        assert_eq!(observed.1, expected);
    }
}

#[test]
fn known_provider_code_is_closed_and_low_cardinality() {
    let body = b"{\"error\":\"invalid_client\"}";
    let mut state = sample_state();
    let completed = interpret_oauth2_refresh_response(&mut state, token_response(401, body));
    assert_eq!(
        completed,
        CompletedTokenRefresh::DefinitiveNoEffect {
            status: 401,
            code: OAuthProviderErrorCode::InvalidClient,
        }
    );
    assert_eq!(
        format!("{completed:?}"),
        "DefinitiveNoEffect { status: 401, code: InvalidClient }"
    );
}

#[test]
fn invalid_grant_remains_a_typed_reauthentication_signal() {
    let body = br#"{
        "error":"invalid_grant",
        "error_description":"refresh_token=diagnostic-canary",
        "error_uri":"https://attacker.example/diagnostic-canary"
    }"#;
    let mut state = sample_state();
    let completed = interpret_oauth2_refresh_response(&mut state, token_response(400, body));
    assert_eq!(
        completed,
        CompletedTokenRefresh::InvalidGrant { status: 400 }
    );
    let diagnostic = format!("{completed:?}");
    assert!(!diagnostic.contains("diagnostic-canary"));
    assert!(!diagnostic.contains("attacker.example"));

    let completed = interpret_oauth2_refresh_response(
        &mut state,
        token_response(500, br#"{"error":"invalid_grant"}"#),
    );
    assert_eq!(
        completed,
        CompletedTokenRefresh::AmbiguousDenial {
            status: 500,
            code: OAuthProviderErrorCode::Other,
        }
    );
}

#[test]
fn arbitrary_oversized_and_control_bearing_codes_never_reach_diagnostics() {
    let oversized = "x".repeat(65);
    let cases = [
        r#"{"error":"extension-secret-canary"}"#.to_owned(),
        format!(r#"{{"error":"{oversized}"}}"#),
        r#"{"error":"invalid_client\u001b[31msecret-canary"}"#.to_owned(),
    ];

    for body in cases {
        let mut state = sample_state();
        let completed =
            interpret_oauth2_refresh_response(&mut state, token_response(400, body.as_bytes()));
        assert!(matches!(
            completed,
            CompletedTokenRefresh::AmbiguousDenial {
                code: OAuthProviderErrorCode::Other,
                ..
            }
        ));
        let diagnostic = format!("{completed:?}");
        assert!(!diagnostic.contains("secret-canary"));
        assert!(!diagnostic.contains(&oversized));
        assert!(!diagnostic.contains('\u{1b}'));
    }
}

#[test]
fn redirects_are_non_success_provider_responses() {
    for status in [301, 302, 303, 307, 308] {
        let mut state = sample_state();
        let completed = interpret_oauth2_refresh_response(&mut state, token_response(status, b""));
        assert!(matches!(
            completed,
            CompletedTokenRefresh::AmbiguousDenial {
                code: OAuthProviderErrorCode::Other,
                ..
            }
        ));
    }
}

#[test]
fn success_parse_errors_are_fixed_and_input_free() {
    let body = br#"{"access_token": {"diagnostic-canary":"secret"}}"#;
    let mut state = sample_state();
    let completed = interpret_oauth2_refresh_response(&mut state, token_response(200, body));
    assert_eq!(
        completed,
        CompletedTokenRefresh::MalformedSuccess { status: 200 }
    );
    let diagnostic = format!("{completed:?}");
    assert!(!diagnostic.contains("diagnostic-canary"));
}

#[test]
fn typed_response_debug_is_constant_and_redacted() {
    let first = parse_success(br#"{"access_token":"short"}"#);
    let second = parse_success(
        br#"{
            "access_token":"access-diagnostic-canary-with-a-different-length",
            "refresh_token":"refresh-diagnostic-canary",
            "token_type":"Bearer",
            "scope":"read",
            "expires_in":42
        }"#,
    );
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
    assert!(!format!("{second:?}").contains("diagnostic-canary"));

    let first_error: TokenErrorResponse =
        serde_json::from_slice(br#"{"error":"invalid_client"}"#).expect("valid error");
    let second_error: TokenErrorResponse =
        serde_json::from_slice(br#"{"error":"diagnostic-canary"}"#).expect("valid error");
    assert_eq!(format!("{first_error:?}"), format!("{second_error:?}"));
    assert!(!format!("{second_error:?}").contains("diagnostic-canary"));
}

#[test]
fn preparation_returns_typed_failures_before_a_dispatch_payload_exists() {
    let mut missing = sample_state();
    missing.refresh_token = None;
    assert!(matches!(
        prepare_oauth2_refresh(&missing),
        Err(PrepareTokenRefreshError::MissingRefreshToken)
    ));

    let mut malformed_token = sample_state();
    malformed_token.refresh_token = Some(SecretString::new(""));
    assert!(matches!(
        prepare_oauth2_refresh(&malformed_token),
        Err(PrepareTokenRefreshError::InvalidRefreshToken)
    ));

    let mut malformed_scopes = sample_state();
    malformed_scopes.scopes = vec!["read write".to_owned()];
    assert!(matches!(
        prepare_oauth2_refresh(&malformed_scopes),
        Err(PrepareTokenRefreshError::InvalidScopes)
    ));
    malformed_scopes.scopes = vec!["read".to_owned(), "read".to_owned()];
    assert!(matches!(
        prepare_oauth2_refresh(&malformed_scopes),
        Err(PrepareTokenRefreshError::InvalidScopes)
    ));

    let mut invalid_endpoint = sample_state();
    invalid_endpoint.token_url = "http://provider.example/token".to_owned();
    assert!(matches!(
        prepare_oauth2_refresh(&invalid_endpoint),
        Err(PrepareTokenRefreshError::InvalidEndpoint(
            OAuthEndpointError::HttpsRequired
        ))
    ));
}

#[test]
fn prepared_payload_debug_is_constant_and_secret_free() {
    let short = prepare_oauth2_refresh(&sample_state()).expect("sample state prepares");
    let mut canary_state = sample_state();
    canary_state.refresh_token = Some(SecretString::new(
        "refresh-diagnostic-canary-with-a-different-length",
    ));
    canary_state.client_id = SecretString::new("client-diagnostic-canary");
    canary_state.client_secret = SecretString::new("secret-diagnostic-canary");
    let canary = prepare_oauth2_refresh(&canary_state).expect("canary state still prepares safely");

    assert_eq!(format!("{short:?}"), format!("{canary:?}"));
    assert!(!format!("{canary:?}").contains("diagnostic-canary"));
}

#[test]
fn completed_denials_are_partitioned_by_replay_safety() {
    let response = |status, body: &'static [u8]| {
        TokenPostResponse::try_new(status, SecretBytes::new(body.to_vec()))
            .expect("test response is policy-valid")
    };

    let mut state = sample_state();
    assert_eq!(
        interpret_oauth2_refresh_response(
            &mut state,
            response(400, br#"{"error":"invalid_grant"}"#),
        ),
        CompletedTokenRefresh::InvalidGrant { status: 400 }
    );

    for (status, body, code) in [
        (
            400,
            br#"{"error":"invalid_request"}"#.as_slice(),
            OAuthProviderErrorCode::InvalidRequest,
        ),
        (
            400,
            br#"{"error":"invalid_client"}"#.as_slice(),
            OAuthProviderErrorCode::InvalidClient,
        ),
        (
            400,
            br#"{"error":"unauthorized_client"}"#.as_slice(),
            OAuthProviderErrorCode::UnauthorizedClient,
        ),
        (
            400,
            br#"{"error":"unsupported_grant_type"}"#.as_slice(),
            OAuthProviderErrorCode::UnsupportedGrantType,
        ),
        (
            400,
            br#"{"error":"invalid_scope"}"#.as_slice(),
            OAuthProviderErrorCode::InvalidScope,
        ),
        (
            401,
            br#"{"error":"invalid_client"}"#.as_slice(),
            OAuthProviderErrorCode::InvalidClient,
        ),
    ] {
        assert_eq!(
            interpret_oauth2_refresh_response(&mut state, response(status, body)),
            CompletedTokenRefresh::DefinitiveNoEffect { status, code }
        );
    }

    for (status, body, code) in [
        (
            302,
            br#"{"error":"invalid_client"}"#.as_slice(),
            OAuthProviderErrorCode::InvalidClient,
        ),
        (
            400,
            br#"{"error":"temporarily_unavailable"}"#.as_slice(),
            OAuthProviderErrorCode::TemporarilyUnavailable,
        ),
        (
            400,
            br#"{"error":"server_error"}"#.as_slice(),
            OAuthProviderErrorCode::ServerError,
        ),
        (
            400,
            br#"{"error":"extension_code"}"#.as_slice(),
            OAuthProviderErrorCode::Other,
        ),
        (
            401,
            br#"{"error":"invalid_request"}"#.as_slice(),
            OAuthProviderErrorCode::InvalidRequest,
        ),
        (
            408,
            br#"{"error":"invalid_client"}"#.as_slice(),
            OAuthProviderErrorCode::InvalidClient,
        ),
        (
            429,
            br#"{"error":"invalid_scope"}"#.as_slice(),
            OAuthProviderErrorCode::InvalidScope,
        ),
        (
            503,
            br#"{"error":"invalid_client"}"#.as_slice(),
            OAuthProviderErrorCode::InvalidClient,
        ),
    ] {
        assert_eq!(
            interpret_oauth2_refresh_response(&mut state, response(status, body)),
            CompletedTokenRefresh::AmbiguousDenial { status, code }
        );
    }

    assert_eq!(state.access_token.expose_secret(), "old-token");
    assert_eq!(
        state
            .refresh_token
            .as_ref()
            .expect("sample refresh token remains present")
            .expose_secret(),
        "refresh-1"
    );
}

#[test]
fn completed_success_is_applied_only_after_full_validation() {
    let mut malformed_state = sample_state();
    let old_access_token = malformed_state.access_token.expose_secret().to_owned();
    let malformed = TokenPostResponse::try_new(
        200,
        SecretBytes::new(br#"{"access_token":"new-token","token_type":"not-bearer"}"#.to_vec()),
    )
    .expect("test response is policy-valid");

    assert_eq!(
        interpret_oauth2_refresh_response(&mut malformed_state, malformed),
        CompletedTokenRefresh::MalformedSuccess { status: 200 }
    );
    assert_eq!(
        malformed_state.access_token.expose_secret(),
        old_access_token
    );

    let mut refreshed_state = sample_state();
    let success = TokenPostResponse::try_new(
        200,
        SecretBytes::new(
            br#"{
                "access_token":"new-token",
                "token_type":"Bearer",
                "refresh_token":"refresh-2",
                "scope":"read"
            }"#
            .to_vec(),
        ),
    )
    .expect("test response is policy-valid");

    assert_eq!(
        interpret_oauth2_refresh_response(&mut refreshed_state, success),
        CompletedTokenRefresh::Refreshed
    );
    assert_eq!(refreshed_state.access_token.expose_secret(), "new-token");
    assert_eq!(refreshed_state.scopes, vec!["read".to_owned()]);
}
