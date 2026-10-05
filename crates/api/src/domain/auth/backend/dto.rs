//! Server compatibility imports and transport-to-domain mappings.

pub use nebula_api_contract::v1::auth::*;

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_not_impl_any!(SecretString: Clone);

    #[test]
    fn secret_string_debug_redacts() {
        let s = SecretString::new("hunter2".to_owned());
        assert_eq!(format!("{s:?}"), "SecretString(***)");
        assert_eq!(s.expose(), "hunter2");
        assert_eq!(s.len(), 7);
        assert!(!s.is_empty());
    }

    #[test]
    fn login_request_deserializes_secret() {
        let req: LoginRequest =
            serde_json::from_str(r#"{"email":"a@b.c","password":"secret","totp":"123456"}"#)
                .expect("parse");
        assert_eq!(req.email, "a@b.c");
        assert_eq!(req.password.expose(), "secret");
        assert_eq!(req.totp.as_deref(), Some("123456"));
    }

    #[test]
    fn oauth_start_response_debug_redacts_url_and_state() {
        let response = OAuthStartResponse {
            authorize_url: "https://idp.example/authorize?state=URL_CANARY-a62b".to_owned(),
            state: "STATE_CANARY-c77e".to_owned(),
        };

        let debug = format!("{response:?}");
        assert!(!debug.contains("URL_CANARY-a62b"));
        assert!(!debug.contains("STATE_CANARY-c77e"));

        let wire = serde_json::to_value(&response).expect("OAuth start response serializes");
        assert_eq!(
            wire["authorize_url"],
            "https://idp.example/authorize?state=URL_CANARY-a62b"
        );
        assert_eq!(wire["state"], "STATE_CANARY-c77e");
    }

    #[test]
    fn mfa_challenge_response_debug_redacts_plaintext_but_wire_keeps_it() {
        const CANARY: &str = "MFA_CHALLENGE_CANARY-6f47";
        let response = MfaChallengeResponse {
            mfa_required: true,
            challenge_token: CANARY.to_owned(),
        };

        assert!(!format!("{response:?}").contains(CANARY));
        let wire = serde_json::to_value(&response).expect("MFA challenge response serializes");
        assert_eq!(wire["challenge_token"], CANARY);
    }

    #[test]
    fn auth_dto_debug_redacts_login_reset_mfa_and_session_authority() {
        const CANARY: &str = "AUTHORITY_CANARY-8f2c";
        let profile = || UserProfile {
            user_id: CANARY.to_owned(),
            email: format!("{CANARY}@example.test"),
            display_name: CANARY.to_owned(),
            avatar_url: Some(CANARY.to_owned()),
            email_verified: true,
            mfa_enabled: true,
        };
        let debug_values = [
            format!(
                "{:?}",
                LoginRequest {
                    email: format!("{CANARY}@example.test"),
                    password: SecretString::new(CANARY.to_owned()),
                    totp: Some(CANARY.to_owned()),
                }
            ),
            format!(
                "{:?}",
                ResetPasswordRequest {
                    token: CANARY.to_owned(),
                    new_password: SecretString::new(CANARY.to_owned()),
                }
            ),
            format!(
                "{:?}",
                VerifyEmailRequest {
                    token: CANARY.to_owned(),
                }
            ),
            format!(
                "{:?}",
                MfaConfirmEnrollRequest {
                    code: CANARY.to_owned(),
                }
            ),
            format!(
                "{:?}",
                MfaLoginCompleteRequest {
                    code: CANARY.to_owned(),
                    challenge_token: CANARY.to_owned(),
                }
            ),
            format!(
                "{:?}",
                LoginResponse {
                    user: profile(),
                    csrf_token: CANARY.to_owned(),
                }
            ),
            format!(
                "{:?}",
                MfaEnrollResponse {
                    otpauth_uri: format!("otpauth://totp/{CANARY}?secret={CANARY}"),
                    secret_base32: CANARY.to_owned(),
                }
            ),
        ];

        for debug in debug_values {
            assert!(!debug.contains(CANARY), "Debug leaked authority: {debug}");
        }

        let login_wire = serde_json::to_value(LoginResponse {
            user: profile(),
            csrf_token: CANARY.to_owned(),
        })
        .expect("login response serializes");
        assert!(login_wire.get("session_id").is_none());
        assert_eq!(login_wire["csrf_token"], CANARY);
    }
}
