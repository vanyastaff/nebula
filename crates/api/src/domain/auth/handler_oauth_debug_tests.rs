use super::{OAuthCallbackParams, derive_oauth_redirect_uri, validate_oauth_callback_params};
use crate::domain::auth::backend::{AuthError, OAuthProvider};

#[test]
fn oauth_callback_params_debug_redacts_code_and_state() {
    let params = OAuthCallbackParams {
        state: "STATE_CANARY-438d".to_owned(),
        code: Some("CODE_CANARY-e9d3".to_owned()),
        error: Some("ERROR_CANARY-b150".to_owned()),
    };

    let debug = format!("{params:?}");
    assert!(!debug.contains("STATE_CANARY-438d"));
    assert!(!debug.contains("CODE_CANARY-e9d3"));
    assert!(!debug.contains("ERROR_CANARY-b150"));
}

#[test]
fn redirect_derivation_revalidates_custom_app_state_origin() {
    let redirect = derive_oauth_redirect_uri("https://nebula.example/", OAuthProvider::GitHub)
        .expect("canonical public origin must derive a callback");
    assert_eq!(
        redirect,
        "https://nebula.example/api/v1/auth/oauth/github/callback"
    );
    for base in [
        "https://nebula.example/nebula",
        "https://nebula.example/nebula/",
    ] {
        assert_eq!(
            derive_oauth_redirect_uri(base, OAuthProvider::GitHub)
                .expect("canonical mount prefix must derive a callback"),
            "https://nebula.example/nebula/api/v1/auth/oauth/github/callback"
        );
    }

    const CANARY: &str = "PUBLIC_ORIGIN_CANARY_DO_NOT_ECHO";
    for invalid in [
        format!("https://user:{CANARY}@nebula.example/"),
        format!("https://nebula.example/?secret={CANARY}"),
        format!("https://nebula.example/base//{CANARY}"),
        format!("https://nebula.example/base/%2F{CANARY}"),
        "http://nebula.example/".to_owned(),
    ] {
        let error = derive_oauth_redirect_uri(&invalid, OAuthProvider::GitHub)
            .expect_err("non-canonical AppState origin must fail closed");
        assert!(matches!(&error, AuthError::Internal(_)));
        assert!(!error.to_string().contains(CANARY));
        assert!(!format!("{error:?}").contains(CANARY));
    }
}

#[test]
fn callback_parameter_validation_is_bounded_and_secret_free() {
    let valid = OAuthCallbackParams {
        state: "A".repeat(43),
        code: Some("visible-code_~.-".to_owned()),
        error: None,
    };
    validate_oauth_callback_params(&valid).expect("bounded visible callback is valid");
    let provider_error = OAuthCallbackParams {
        state: "A".repeat(43),
        code: None,
        error: Some("access_denied".to_owned()),
    };
    validate_oauth_callback_params(&provider_error)
        .expect("bounded provider error callback is valid");

    const CANARY: &str = "CALLBACK_INPUT_CANARY_DO_NOT_ECHO";
    for invalid in [
        OAuthCallbackParams {
            state: CANARY.to_owned(),
            code: Some("code".to_owned()),
            error: None,
        },
        OAuthCallbackParams {
            state: "A".repeat(43),
            code: Some(String::new()),
            error: None,
        },
        OAuthCallbackParams {
            state: "A".repeat(43),
            code: Some(format!("{CANARY}\n")),
            error: None,
        },
        OAuthCallbackParams {
            state: "A".repeat(43),
            code: Some("x".repeat(4097)),
            error: None,
        },
        OAuthCallbackParams {
            state: "A".repeat(43),
            code: Some("code".to_owned()),
            error: Some(CANARY.to_owned()),
        },
        OAuthCallbackParams {
            state: "A".repeat(43),
            code: None,
            error: Some("x".repeat(257)),
        },
    ] {
        let error = match validate_oauth_callback_params(&invalid) {
            Ok(_) => panic!("invalid callback parameters must fail closed"),
            Err(error) => error,
        };
        assert!(!error.to_string().contains(CANARY));
        assert!(!format!("{error:?}").contains(CANARY));
    }
}
