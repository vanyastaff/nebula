use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

use axum::{
    Json,
    body::to_bytes,
    extract::{OriginalUri, Path, Query, State},
    http::{
        HeaderMap, HeaderValue, StatusCode, Uri,
        header::{COOKIE, HOST, SET_COOKIE},
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};

use super::{
    OAUTH_TRANSACTION_COOKIE_HASH_DOMAIN, OAUTH_TRANSACTION_COOKIE_LIMIT,
    OAUTH_TRANSACTION_COOKIE_PREFIX, OAuthCallbackParams, OAuthTransactionBinding,
    mfa_complete_login, oauth_callback, oauth_start, oauth_transaction_cookie_count,
    validate_oauth_request_authority,
};
use crate::{
    AppState, OAuthIdentityRuntime,
    config::{OAuthProviderConfig, OAuthProvidersConfig},
    domain::auth::backend::{
        AuthBackend, CSRF_COOKIE, InMemoryAuthBackend, MfaLoginCompleteRequest, OAuthProvider,
        SESSION_COOKIE, mfa,
    },
    error::ApiError,
    transport::oauth::{
        OAuthTestProviderProfile,
        test_support::{TestResponse, TlsFixture},
    },
};

const PUBLIC_URL: &str = "https://nebula.example/nebula";
const CALLBACK_PATH: &str = "/api/v1/auth/oauth/github/callback";
const TEST_DNS_ANSWER: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));

struct StartedFlow {
    state: String,
    cookie_pair: String,
    set_cookie: String,
}

fn request_headers(authority: &str, cookie_pairs: &[String]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HOST,
        HeaderValue::from_str(authority).expect("test authority is a valid header value"),
    );
    for pair in cookie_pairs {
        headers.append(
            COOKIE,
            HeaderValue::from_str(pair).expect("test cookie pair is a valid header value"),
        );
    }
    headers
}

fn manual_config(
    fixture: &TlsFixture,
) -> (
    OAuthProvidersConfig,
    HashMap<OAuthProvider, OAuthTestProviderProfile>,
) {
    (
        OAuthProvidersConfig {
            providers: HashMap::from([(
                OAuthProvider::GitHub,
                OAuthProviderConfig {
                    client_id: SecretString::new("test-client".to_owned().into_boxed_str()),
                    client_secret: SecretString::new("test-secret".to_owned().into_boxed_str()),
                },
            )]),
        },
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

fn state_and_backend_with_oauth(fixture: &TlsFixture) -> (AppState, Arc<InMemoryAuthBackend>) {
    let (config, profiles) = manual_config(fixture);
    let runtime = OAuthIdentityRuntime::from_config_for_test(
        config,
        profiles,
        fixture.trust_anchor(),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        vec![TEST_DNS_ANSWER],
    )
    .expect("test OAuth runtime must build")
    .expect("configured test provider must enable OAuth");
    let backend = Arc::new(InMemoryAuthBackend::new().with_oauth_runtime(Arc::new(runtime)));
    let auth_backend: Arc<dyn AuthBackend> = Arc::clone(&backend) as _;
    let state = crate::state::test_state_with_in_memory_stores()
        .with_auth_backend(auth_backend)
        .with_public_url(PUBLIC_URL);
    (state, backend)
}

fn state_with_oauth(fixture: &TlsFixture) -> AppState {
    state_and_backend_with_oauth(fixture).0
}

async fn start_flow(
    state: AppState,
    authority: &str,
    cookies: &[String],
) -> Result<StartedFlow, ApiError> {
    let response = oauth_start(
        State(state),
        Path("github".to_owned()),
        OriginalUri(
            "/api/v1/auth/oauth/github"
                .parse()
                .expect("test start URI is valid"),
        ),
        request_headers(authority, cookies),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| {
            value
                .to_str()
                .expect("transaction Set-Cookie is visible ASCII")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(set_cookie.len(), 1, "start sets exactly one cookie");
    let set_cookie = set_cookie
        .into_iter()
        .next()
        .expect("one transaction cookie was asserted");
    let cookie_pair = set_cookie
        .split(';')
        .next()
        .expect("Set-Cookie begins with a cookie pair")
        .to_owned();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read OAuth start response body");
    let body: serde_json::Value =
        serde_json::from_slice(&body).expect("OAuth start response is JSON");
    let state = body["state"]
        .as_str()
        .expect("OAuth start response carries state")
        .to_owned();
    Ok(StartedFlow {
        state,
        cookie_pair,
        set_cookie,
    })
}

fn callback_params(state: &str) -> OAuthCallbackParams {
    OAuthCallbackParams {
        state: state.to_owned(),
        code: Some("visible-code".to_owned()),
        error: None,
    }
}

async fn callback(
    state: AppState,
    provider: &str,
    flow_state: &str,
    authority: &str,
    cookies: &[String],
) -> Result<axum::response::Response, ApiError> {
    let uri: Uri =
        format!("/api/v1/auth/oauth/{provider}/callback?state={flow_state}&code=visible-code")
            .parse()
            .expect("test callback URI is valid");
    oauth_callback(
        State(state),
        Path(provider.to_owned()),
        OriginalUri(uri),
        request_headers(authority, cookies),
        Ok(Query(callback_params(flow_state))),
    )
    .await
}

async fn provider_error_callback(
    state: AppState,
    flow_state: &str,
    authority: &str,
    cookies: &[String],
    include_code: bool,
    provider_error: &str,
) -> Result<axum::response::Response, ApiError> {
    let uri: Uri = format!(
        "{CALLBACK_PATH}?state={flow_state}&error={provider_error}&error_description=IGNORED_PROVIDER_TEXT"
    )
    .parse()
    .expect("test provider-error callback URI is valid");
    oauth_callback(
        State(state),
        Path("github".to_owned()),
        OriginalUri(uri),
        request_headers(authority, cookies),
        Ok(Query(OAuthCallbackParams {
            state: flow_state.to_owned(),
            code: include_code.then(|| "unexpected-code".to_owned()),
            error: Some(provider_error.to_owned()),
        })),
    )
    .await
}

fn assert_status(error: &ApiError, expected: StatusCode) {
    assert_eq!(error.to_problem_details().0, expected);
}

fn assert_transaction_cookie_security(flow: &StartedFlow) {
    let (name, value) = flow
        .cookie_pair
        .split_once('=')
        .expect("transaction cookie has a name and value");
    let encoded_hash = name
        .strip_prefix(OAUTH_TRANSACTION_COOKIE_PREFIX)
        .expect("transaction cookie uses the canonical __Host prefix");
    let decoded_hash = URL_SAFE_NO_PAD
        .decode(encoded_hash)
        .expect("cookie name suffix is base64url");
    let provider_name = OAuthProvider::GitHub.as_str();
    let mut expected_hash = Sha256::new();
    expected_hash.update(OAUTH_TRANSACTION_COOKIE_HASH_DOMAIN);
    expected_hash.update((provider_name.len() as u64).to_be_bytes());
    expected_hash.update(provider_name.as_bytes());
    expected_hash.update((flow.state.len() as u64).to_be_bytes());
    expected_hash.update(flow.state.as_bytes());
    assert_eq!(
        decoded_hash,
        expected_hash.finalize().as_slice(),
        "cookie name hashes the explicit domain and length-delimited tuple"
    );
    assert_eq!(encoded_hash.len(), 43, "SHA-256 base64url is untruncated");
    assert_eq!(value, format!("v1.github.{}", flow.state));

    let attributes = flow.set_cookie.split("; ").collect::<Vec<_>>();
    assert_eq!(attributes[0], flow.cookie_pair);
    assert_eq!(attributes[1], "Path=/");
    assert_eq!(attributes[2], "Max-Age=600");
    assert!(attributes[3].starts_with("Expires="));
    assert_eq!(attributes[4..], ["Secure", "HttpOnly", "SameSite=Lax"]);
    assert!(!flow.set_cookie.contains("Domain="));
}

#[test]
fn transaction_cookie_parser_is_unique_exact_and_conservatively_bounded() {
    let binding = OAuthTransactionBinding::new(OAuthProvider::GitHub, &"A".repeat(43));
    let exact = format!("{}={}", binding.name, binding.value);

    let correct = request_headers("nebula.example", std::slice::from_ref(&exact));
    binding
        .validate_request(&correct)
        .expect("one exact binding is accepted");

    for invalid in [
        request_headers("nebula.example", &[]),
        request_headers(
            "nebula.example",
            &[format!("{}={}x", binding.name, binding.value)],
        ),
        request_headers(
            "nebula.example",
            &[format!("{}={} ", binding.name, binding.value)],
        ),
        request_headers("nebula.example", &[exact.clone(), exact]),
    ] {
        assert!(
            binding.validate_request(&invalid).is_err(),
            "missing, mismatched, trailing-byte, and duplicate bindings fail closed"
        );
    }

    let mut bounded = HeaderMap::new();
    bounded.append(
        COOKIE,
        HeaderValue::from_static(
            "__Host-nebula-oauth-malformed; unrelated=1; __Host-nebula-oauth-other=x",
        ),
    );
    assert_eq!(
        oauth_transaction_cookie_count(&bounded).expect("ASCII Cookie header is countable"),
        2,
        "canonical-prefix entries count even without '='"
    );
}

#[test]
fn request_authority_matches_canonical_host_and_normalized_default_port() {
    let path: Uri = CALLBACK_PATH.parse().expect("callback path URI is valid");
    for authority in ["nebula.example", "NEBULA.EXAMPLE:443"] {
        validate_oauth_request_authority(PUBLIC_URL, &request_headers(authority, &[]), &path)
            .expect("canonical host with implicit or explicit default port is accepted");
    }
    validate_oauth_request_authority(
        PUBLIC_URL,
        &HeaderMap::new(),
        &"https://nebula.example/api/v1/auth/oauth/github/callback"
            .parse()
            .expect("absolute callback URI is valid"),
    )
    .expect("HTTP/2-style URI authority is accepted without Host");

    for headers in [
        HeaderMap::new(),
        request_headers("alias.example", &[]),
        request_headers("nebula.example:8443", &[]),
    ] {
        assert!(
            validate_oauth_request_authority(PUBLIC_URL, &headers, &path).is_err(),
            "missing, alias, and wrong-port authorities fail closed"
        );
    }

    let mut duplicate = request_headers("nebula.example", &[]);
    duplicate.append(HOST, HeaderValue::from_static("nebula.example"));
    assert!(validate_oauth_request_authority(PUBLIC_URL, &duplicate, &path).is_err());
}

#[tokio::test]
async fn callback_binding_blocks_before_egress_then_correct_flow_succeeds_and_clears() {
    let fixture = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
        "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"Bearer"}"#),
        "/userinfo" => TestResponse::json(r#"{"id":101}"#),
        "/emails" => {
            TestResponse::json(r#"[{"email":"bound@example.com","primary":true,"verified":true}]"#)
        },
        _ => TestResponse::failure(404),
    })
    .await;
    let state = state_with_oauth(&fixture);
    let flow = start_flow(state.clone(), "nebula.example", &[])
        .await
        .expect("OAuth start succeeds");
    assert_transaction_cookie_security(&flow);

    let (name, _) = flow
        .cookie_pair
        .split_once('=')
        .expect("started cookie has a name");
    for attempt in [
        callback(state.clone(), "github", &flow.state, "nebula.example", &[]).await,
        callback(
            state.clone(),
            "github",
            &flow.state,
            "nebula.example",
            &[format!("{name}=wrong")],
        )
        .await,
        callback(
            state.clone(),
            "github",
            &flow.state,
            "nebula.example",
            &[flow.cookie_pair.clone(), flow.cookie_pair.clone()],
        )
        .await,
        callback(
            state.clone(),
            "google",
            &flow.state,
            "nebula.example",
            std::slice::from_ref(&flow.cookie_pair),
        )
        .await,
    ] {
        let error = attempt.expect_err("invalid browser binding must fail");
        assert_status(&error, StatusCode::UNAUTHORIZED);
    }
    let authority_error = callback(
        state.clone(),
        "github",
        &flow.state,
        "nebula.example:8443",
        std::slice::from_ref(&flow.cookie_pair),
    )
    .await
    .expect_err("wrong callback port must fail before binding/backend");
    assert_status(&authority_error, StatusCode::BAD_REQUEST);
    assert!(
        fixture.requests().is_empty(),
        "rejected bindings and authority mismatch cannot reach provider egress"
    );

    let response = callback(
        state,
        "github",
        &flow.state,
        "nebula.example:443",
        std::slice::from_ref(&flow.cookie_pair),
    )
    .await
    .expect("correct browser binding reaches the backend");
    assert_eq!(response.status(), StatusCode::OK);
    let expected_clear =
        OAuthTransactionBinding::new(OAuthProvider::GitHub, &flow.state).cleared_cookie();
    let cleared = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().expect("response cookie is ASCII"))
        .any(|cookie| cookie == expected_clear.as_str());
    assert!(cleared);
    assert!(expected_clear.contains("Max-Age=0"));
    assert!(expected_clear.contains("Expires=Thu, 01 Jan 1970 00:00:00 GMT"));
    assert_eq!(
        fixture.requests().len(),
        3,
        "token + userinfo + verified email exactly once"
    );
}

#[tokio::test]
async fn linked_oauth_user_with_local_mfa_gets_only_a_one_time_challenge() {
    let fixture = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
        "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"Bearer"}"#),
        "/userinfo" => {
            TestResponse::json(r#"{"id":303,"amr":["mfa"],"acr":"provider-high-assurance"}"#)
        },
        "/emails" => TestResponse::json(
            r#"[{"email":"oauth-mfa@example.com","primary":true,"verified":true}]"#,
        ),
        _ => TestResponse::failure(404),
    })
    .await;
    let (state, backend) = state_and_backend_with_oauth(&fixture);

    // First login creates the OAuth-only local user and its initial
    // session atomically. Local MFA is enrolled only after that.
    let first = start_flow(state.clone(), "nebula.example", &[])
        .await
        .expect("first OAuth start succeeds");
    let first_response = callback(
        state.clone(),
        "github",
        &first.state,
        "nebula.example",
        std::slice::from_ref(&first.cookie_pair),
    )
    .await
    .expect("first OAuth callback succeeds");
    assert_eq!(first_response.status(), StatusCode::OK);
    let first_body = to_bytes(first_response.into_body(), usize::MAX)
        .await
        .expect("read first OAuth response");
    let first_body: serde_json::Value =
        serde_json::from_slice(&first_body).expect("first OAuth response is JSON");
    let user_id = first_body["user"]["user_id"]
        .as_str()
        .expect("first OAuth response carries user id");
    let enrollment = backend
        .start_mfa_enrollment(user_id)
        .await
        .expect("start local MFA enrollment");
    let enrollment_code =
        mfa::current_code(&enrollment.secret_base32).expect("current enrollment code");
    backend
        .confirm_mfa_enrollment(user_id, &enrollment_code)
        .await
        .expect("confirm local MFA enrollment");

    // A provider claim that it performed MFA never substitutes for the
    // independently enrolled Nebula factor.
    let second = start_flow(state.clone(), "nebula.example", &[])
        .await
        .expect("second OAuth start succeeds");
    let response = callback(
        state.clone(),
        "github",
        &second.state,
        "nebula.example",
        std::slice::from_ref(&second.cookie_pair),
    )
    .await
    .expect("linked OAuth callback reaches local MFA gate");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let cookies = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().expect("response cookie is ASCII").to_owned())
        .collect::<Vec<_>>();
    let expected_clear =
        OAuthTransactionBinding::new(OAuthProvider::GitHub, &second.state).cleared_cookie();
    assert_eq!(cookies, [expected_clear]);
    assert!(!cookies.iter().any(|cookie| {
        cookie.starts_with(&format!("{SESSION_COOKIE}="))
            || cookie.starts_with(&format!("{CSRF_COOKIE}="))
    }));
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read MFA-required OAuth response");
    let body: serde_json::Value =
        serde_json::from_slice(&body).expect("MFA-required response is JSON");
    assert_eq!(body["mfa_required"], true);
    assert!(body.get("session_id").is_none());
    assert!(body.get("csrf_token").is_none());
    let challenge_token = body["challenge_token"]
        .as_str()
        .expect("MFA-required response carries a challenge")
        .to_owned();

    let code = mfa::current_code(&enrollment.secret_base32).expect("current login MFA code");
    let completed = mfa_complete_login(
        State(state.clone()),
        Json(MfaLoginCompleteRequest {
            code: code.clone(),
            challenge_token: challenge_token.clone(),
        }),
    )
    .await
    .expect("local MFA completes OAuth login");
    assert_eq!(completed.status(), StatusCode::OK);
    let completed_cookies = completed
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().expect("session cookie is ASCII"))
        .collect::<Vec<_>>();
    assert!(
        completed_cookies
            .iter()
            .any(|cookie| cookie.starts_with(&format!("{SESSION_COOKIE}=")))
    );
    assert!(
        completed_cookies
            .iter()
            .any(|cookie| cookie.starts_with(&format!("{CSRF_COOKIE}=")))
    );

    let replay = mfa_complete_login(
        State(state),
        Json(MfaLoginCompleteRequest {
            code,
            challenge_token,
        }),
    )
    .await;
    let replay_error = match replay {
        Ok(_) => panic!("MFA challenge replay must not create another session"),
        Err(error) => error,
    };
    assert_status(&replay_error, StatusCode::UNAUTHORIZED);
    assert_eq!(
        fixture.requests().len(),
        5,
        "repeat linked login fetches token + userinfo but not email"
    );
}

#[tokio::test]
async fn parallel_starts_coexist_and_the_ninth_start_is_rejected() {
    let fixture = TlsFixture::spawn("oauth.test", |_request, _| TestResponse::failure(500)).await;
    let state = state_with_oauth(&fixture);
    let mut jar = Vec::new();
    let mut names = HashSet::new();

    for index in 0..OAUTH_TRANSACTION_COOKIE_LIMIT {
        let authority = if index % 2 == 0 {
            "nebula.example"
        } else {
            "NEBULA.EXAMPLE:443"
        };
        let flow = start_flow(state.clone(), authority, &jar)
            .await
            .unwrap_or_else(|error| panic!("start {index} failed: {error:?}"));
        assert_transaction_cookie_security(&flow);
        let name = flow
            .cookie_pair
            .split_once('=')
            .expect("started cookie has a name")
            .0
            .to_owned();
        assert!(names.insert(name), "parallel flows need distinct names");
        jar.push(flow.cookie_pair);
    }

    let error = match start_flow(state, "nebula.example", &jar).await {
        Ok(_) => panic!("the ninth active transaction must be rejected"),
        Err(error) => error,
    };
    assert_status(&error, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        fixture.requests().is_empty(),
        "authorization starts never use provider egress for manual endpoints"
    );
}

#[tokio::test]
async fn provider_error_consumes_bound_state_clears_cookie_and_never_uses_egress() {
    const ERROR_CANARY: &str = "PROVIDER_ERROR_CANARY_DO_NOT_ECHO";
    let fixture = TlsFixture::spawn("oauth.test", |_request, _| TestResponse::failure(500)).await;
    let state = state_with_oauth(&fixture);
    let flow = start_flow(state.clone(), "nebula.example", &[])
        .await
        .expect("OAuth start succeeds");

    let missing_cookie = provider_error_callback(
        state.clone(),
        &flow.state,
        "nebula.example",
        &[],
        false,
        ERROR_CANARY,
    )
    .await
    .expect_err("provider error without browser binding is rejected");
    assert_status(&missing_cookie, StatusCode::UNAUTHORIZED);

    let both = provider_error_callback(
        state.clone(),
        &flow.state,
        "nebula.example",
        std::slice::from_ref(&flow.cookie_pair),
        true,
        ERROR_CANARY,
    )
    .await
    .expect_err("code plus error is malformed");
    assert_status(&both, StatusCode::BAD_REQUEST);
    assert!(fixture.requests().is_empty());

    let denied = provider_error_callback(
        state.clone(),
        &flow.state,
        "nebula.example",
        std::slice::from_ref(&flow.cookie_pair),
        false,
        ERROR_CANARY,
    )
    .await
    .expect("bound provider error reaches cancellation");
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    let expected_clear =
        OAuthTransactionBinding::new(OAuthProvider::GitHub, &flow.state).cleared_cookie();
    assert!(denied.headers().get_all(SET_COOKIE).iter().any(|value| {
        value
            .to_str()
            .is_ok_and(|cookie| cookie == expected_clear.as_str())
    }));
    let denied_body = to_bytes(denied.into_body(), usize::MAX)
        .await
        .expect("read provider denial response");
    let denied_body =
        String::from_utf8(denied_body.to_vec()).expect("provider denial response is UTF-8");
    assert!(denied_body.contains("OAuth authorization was not granted"));
    assert!(!denied_body.contains(ERROR_CANARY));
    assert!(!denied_body.contains("IGNORED_PROVIDER_TEXT"));

    let replay = callback(
        state,
        "github",
        &flow.state,
        "nebula.example",
        std::slice::from_ref(&flow.cookie_pair),
    )
    .await
    .expect("bound replay returns an HTTP problem response");
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    assert!(
        fixture.requests().is_empty(),
        "denial and replay consume state without token/userinfo egress"
    );
}

#[tokio::test]
async fn two_started_transactions_can_complete_independently() {
    let fixture = TlsFixture::spawn("oauth.test", |request, _| match request.path.as_str() {
        "/token" => TestResponse::json(r#"{"access_token":"token","token_type":"bearer"}"#),
        "/userinfo" => TestResponse::json(r#"{"id":202}"#),
        "/emails" => TestResponse::json(
            r#"[{"email":"parallel@example.com","primary":true,"verified":true}]"#,
        ),
        _ => TestResponse::failure(404),
    })
    .await;
    let state = state_with_oauth(&fixture);
    let first = start_flow(state.clone(), "nebula.example", &[])
        .await
        .expect("first start succeeds");
    let second = start_flow(
        state.clone(),
        "nebula.example:443",
        std::slice::from_ref(&first.cookie_pair),
    )
    .await
    .expect("second start coexists");
    assert_ne!(first.state, second.state);
    assert_ne!(first.cookie_pair, second.cookie_pair);
    let jar = vec![first.cookie_pair.clone(), second.cookie_pair.clone()];

    for flow in [&first, &second] {
        let response = callback(state.clone(), "github", &flow.state, "nebula.example", &jar)
            .await
            .expect("each parallel transaction completes");
        assert_eq!(response.status(), StatusCode::OK);
        let expected_clear =
            OAuthTransactionBinding::new(OAuthProvider::GitHub, &flow.state).cleared_cookie();
        assert!(
            response
                .headers()
                .get_all(SET_COOKIE)
                .iter()
                .any(|value| { value.to_str().is_ok_and(|cookie| cookie == expected_clear) })
        );
    }
    assert_eq!(fixture.requests().len(), 5);
}
