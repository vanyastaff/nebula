//! OAuth2 token-refresh state logic (ADR-0092).
//!
//! SSRF endpoint validation (SEC-10), secret-scoped form composition, response
//! status interpretation, SEC-02 error redaction, and `OAuth2State` mutation all
//! live here — on the `nebula-credential` side of the [`RefreshTransport`] seam.
//! Network I/O is delegated to the injected transport; this module never links
//! reqwest.
//!
//! # Sentinel marking
//!
//! Per sub-spec `docs/INTEGRATION_MODEL.md` the holder marks the L2 claim row
//! `sentinel = RefreshInFlight` immediately before the IdP POST. That mark is
//! durably acknowledged by `RefreshCoordinator::refresh_coalesced` before it
//! starts the owned resolver closure that calls `refresh_oauth2_state`. This
//! module therefore cannot be entered through the coordinated path before the
//! point of no cancellation, and it does not need a `RefreshClaim` or claim
//! repository in the transport layer.
//!
//! After an exact outcome the coordinator wakes local waiters and dispatches a
//! best-effort row release; until that release completes (or TTL expires) the
//! sentinel row continues to coalesce other replicas. Unknown provider or
//! post-provider persistence outcomes leave a durable fail-closed poison after
//! expiry until explicit reconciliation; they are never made replayable by
//! time alone.

use std::fmt;

use chrono::Utc;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::AuthStyle;
use crate::SecretString;
use crate::credentials::OAuth2State;
use crate::runtime::refresh::transport::{RefreshTransport, TokenPostRequest};
use crate::runtime::{OAuthEndpointError, OAuthServerEndpoint};

pub use super::transport::OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES;

/// Refresh-related failures produced by [`refresh_oauth2_state`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TokenRefreshError {
    /// Stored state lacks a refresh token, so re-auth is required.
    #[error("no refresh_token available for token refresh")]
    MissingRefreshToken,
    /// Local validation/encoding failed before the transport was invoked.
    #[error("refresh token request rejected before dispatch: {0}")]
    PreDispatch(#[source] OAuthEndpointError),
    /// The transport was invoked but did not return a complete response.
    ///
    /// Connect, send, timeout, and response-read failures are deliberately
    /// collapsed: the current transport seam cannot prove that the provider
    /// did not consume or rotate the grant.
    #[error("refresh token provider outcome is unknown after dispatch")]
    TransportOutcomeUnknown,
    /// Provider definitively rejected the stored refresh grant.
    #[error("token endpoint rejected the refresh grant ({status})")]
    InvalidGrant {
        /// HTTP status code.
        status: u16,
    },
    /// Token endpoint returned non-success status.
    #[error("token endpoint returned {status}: {code}")]
    TokenEndpoint {
        /// HTTP status code.
        status: u16,
        /// Closed, low-cardinality OAuth error code.
        code: OAuthProviderErrorCode,
    },
    /// Token endpoint response could not be parsed as JSON.
    #[error("failed to parse token response")]
    Parse,
}

/// Closed OAuth token-endpoint error classification.
///
/// Provider-controlled extension text never enters diagnostics. Standard
/// error codes retain useful low-cardinality meaning; every absent, malformed,
/// oversized, control-bearing, or unknown value collapses to [`Self::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OAuthProviderErrorCode {
    /// RFC 6749 `invalid_request`.
    InvalidRequest,
    /// RFC 6749 `invalid_client`.
    InvalidClient,
    /// RFC 6749 `unauthorized_client`.
    UnauthorizedClient,
    /// RFC 6749 `unsupported_grant_type`.
    UnsupportedGrantType,
    /// RFC 6749 `invalid_scope`.
    InvalidScope,
    /// RFC 6749 `temporarily_unavailable`.
    TemporarilyUnavailable,
    /// RFC 6749 `server_error`.
    ServerError,
    /// Missing, malformed, extension, or otherwise unclassified provider code.
    Other,
}

impl fmt::Display for OAuthProviderErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidClient => "invalid_client",
            Self::UnauthorizedClient => "unauthorized_client",
            Self::UnsupportedGrantType => "unsupported_grant_type",
            Self::InvalidScope => "invalid_scope",
            Self::TemporarilyUnavailable => "temporarily_unavailable",
            Self::ServerError => "server_error",
            Self::Other => "other",
        })
    }
}

/// Execute OAuth2 refresh-token grant and mutate `state` in place.
///
/// Call order (security boundary — do not reorder):
/// 1. [`OAuthServerEndpoint::parse`] runs FIRST (HTTPS-only, safe URL shape,
///    no localhost or non-global literal). Returns `Err` before any I/O.
/// 2. Secret-scoped inner block builds a [`TokenPostRequest`] (form fields +
///    optional `basic_auth`). Secret borrows are released when the block ends;
///    the transport receives `SecretString` values that zeroize on drop.
/// 3. [`RefreshTransport::post_token`] is called — the ONLY network I/O.
/// 4. `parse_token_response_bytes` interprets status + bytes; SEC-02
///    redaction runs inside this crate, not in the transport.
/// 5. `update_state_from_token_response` mutates `state` on success.
///
/// SEC-10: the three secret values (refresh_token, client_id, client_secret)
/// are NOT extracted into `Zeroizing<String>` intermediates. Instead, secret
/// borrows live inside an inner block that returns the built
/// `TokenPostRequest`; the block ends → secret borrows drop → `state` is free
/// for `&mut` mutation in `update_state_from_token_response`. No owned
/// plaintext copy lives in our code; the unavoidable in-flight copy lives in
/// the transport's request serialization and is released after the response
/// future resolves.
pub async fn refresh_oauth2_state(
    state: &mut OAuth2State,
    transport: &dyn RefreshTransport,
) -> Result<(), TokenRefreshError> {
    // Step 1 — SSRF validation (must run before any I/O).
    let endpoint =
        OAuthServerEndpoint::parse(&state.token_url).map_err(TokenRefreshError::PreDispatch)?;

    let scope_joined: Option<String> = (!state.scopes.is_empty()).then(|| state.scopes.join(" "));

    // Step 2 — Build the request inside a tight secret-borrow scope.
    // After this block the secret borrows have dropped; only `TokenPostRequest`
    // (carrying `SecretString` values) crosses the block boundary.
    let req = {
        let refresh_tok = state
            .refresh_token
            .as_ref()
            .ok_or(TokenRefreshError::MissingRefreshToken)?
            .expose_secret();
        let client_id = state.client_id.expose_secret();
        let client_secret = state.client_secret.expose_secret();

        let mut form: Vec<(String, SecretString)> = vec![
            ("grant_type".to_owned(), SecretString::new("refresh_token")),
            ("refresh_token".to_owned(), SecretString::new(refresh_tok)),
        ];
        if let Some(ref scope) = scope_joined {
            form.push(("scope".to_owned(), SecretString::new(scope.as_str())));
        }

        let basic_auth = match state.auth_style {
            AuthStyle::Header => {
                // RFC 6749 §2.3.1 requires each raw component to be encoded
                // with application/x-www-form-urlencoded before the Basic
                // `client_id:client_secret` join and base64 step.
                Some((
                    form_encode_basic_component(client_id),
                    form_encode_basic_component(client_secret),
                ))
            },
            AuthStyle::PostBody => {
                // client_id / client_secret go in the form body.
                form.push(("client_id".to_owned(), SecretString::new(client_id)));
                form.push(("client_secret".to_owned(), SecretString::new(client_secret)));
                None
            },
        };

        TokenPostRequest::new(endpoint, form, basic_auth)
    };

    // Step 3 — delegate I/O to the transport (dumb pipe).
    let resp = transport
        .post_token(req)
        .await
        .map_err(|_error| TokenRefreshError::TransportOutcomeUnknown)?;

    // Steps 4 + 5 — status interpretation, bounded parse, SEC-02 redaction,
    // state mutation.  All on the credential side.
    let body = parse_token_response_bytes(resp.status(), resp.body().as_ref())?;
    update_state_from_token_response(state, body)?;
    Ok(())
}

fn form_encode_basic_component(raw: &str) -> SecretString {
    let mut encoded = Zeroizing::new(String::with_capacity(raw.len()));
    for part in url::form_urlencoded::byte_serialize(raw.as_bytes()) {
        encoded.push_str(part);
    }
    SecretString::new(std::mem::take(&mut *encoded))
}

/// Interpret a raw `(status, body_bytes)` pair from the transport.
///
/// SEC-01: `body_bytes` is already bounded to `max_response_bytes` by the
/// transport (mechanical enforcement); this function only interprets what it
/// receives.
/// SEC-02: non-success bodies parse only into a zeroizing, typed envelope.
/// Provider descriptions, URIs, extension fields, and raw parser text never
/// enter the error taxonomy.
fn parse_token_response_bytes(
    status: u16,
    body: &[u8],
) -> Result<TokenSuccessResponse, TokenRefreshError> {
    if !(200..300).contains(&status) {
        let code = parse_provider_error_code(body);
        if status == 400 && code == ParsedProviderErrorCode::InvalidGrant {
            return Err(TokenRefreshError::InvalidGrant { status });
        }
        return Err(TokenRefreshError::TokenEndpoint {
            status,
            code: code.into_public(),
        });
    }

    serde_json::from_slice(body).map_err(|_| TokenRefreshError::Parse)
}

#[derive(serde::Deserialize, Zeroize, ZeroizeOnDrop)]
struct TokenSuccessResponse {
    access_token: Option<SecretString>,
    token_type: Option<SecretString>,
    refresh_token: Option<SecretString>,
    expires_in: Option<u64>,
    scope: Option<SecretString>,
}

impl fmt::Debug for TokenSuccessResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenSuccessResponse(<redacted>)")
    }
}

#[derive(serde::Deserialize, Zeroize, ZeroizeOnDrop)]
struct TokenErrorResponse {
    error: Option<SecretString>,
}

impl fmt::Debug for TokenErrorResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenErrorResponse(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedProviderErrorCode {
    InvalidGrant,
    Public(OAuthProviderErrorCode),
}

impl ParsedProviderErrorCode {
    fn into_public(self) -> OAuthProviderErrorCode {
        match self {
            Self::InvalidGrant => OAuthProviderErrorCode::Other,
            Self::Public(code) => code,
        }
    }
}

fn parse_provider_error_code(body: &[u8]) -> ParsedProviderErrorCode {
    let Ok(mut envelope) = serde_json::from_slice::<TokenErrorResponse>(body) else {
        return ParsedProviderErrorCode::Public(OAuthProviderErrorCode::Other);
    };
    let Some(error) = envelope.error.take() else {
        return ParsedProviderErrorCode::Public(OAuthProviderErrorCode::Other);
    };
    let raw = error.expose_secret();
    if raw.is_empty()
        || raw.len() > 64
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return ParsedProviderErrorCode::Public(OAuthProviderErrorCode::Other);
    }

    match raw {
        "invalid_grant" => ParsedProviderErrorCode::InvalidGrant,
        "invalid_request" => {
            ParsedProviderErrorCode::Public(OAuthProviderErrorCode::InvalidRequest)
        },
        "invalid_client" => ParsedProviderErrorCode::Public(OAuthProviderErrorCode::InvalidClient),
        "unauthorized_client" => {
            ParsedProviderErrorCode::Public(OAuthProviderErrorCode::UnauthorizedClient)
        },
        "unsupported_grant_type" => {
            ParsedProviderErrorCode::Public(OAuthProviderErrorCode::UnsupportedGrantType)
        },
        "invalid_scope" => ParsedProviderErrorCode::Public(OAuthProviderErrorCode::InvalidScope),
        "temporarily_unavailable" => {
            ParsedProviderErrorCode::Public(OAuthProviderErrorCode::TemporarilyUnavailable)
        },
        "server_error" => ParsedProviderErrorCode::Public(OAuthProviderErrorCode::ServerError),
        _ => ParsedProviderErrorCode::Public(OAuthProviderErrorCode::Other),
    }
}

fn update_state_from_token_response(
    state: &mut OAuth2State,
    mut body: TokenSuccessResponse,
) -> Result<(), TokenRefreshError> {
    let access_token = body.access_token.as_ref().ok_or(TokenRefreshError::Parse)?;
    if !is_rfc5234_vschar(access_token.expose_secret()) {
        return Err(TokenRefreshError::Parse);
    }

    let token_type = body.token_type.as_ref().ok_or(TokenRefreshError::Parse)?;
    if !token_type.expose_secret().eq_ignore_ascii_case("bearer") {
        return Err(TokenRefreshError::Parse);
    }

    if body
        .refresh_token
        .as_ref()
        .is_some_and(|token| !is_rfc5234_vschar(token.expose_secret()))
    {
        return Err(TokenRefreshError::Parse);
    }

    let expires_at = body
        .expires_in
        .map(|expires_in| {
            let seconds = i64::try_from(expires_in).map_err(|_| TokenRefreshError::Parse)?;
            Utc::now()
                .checked_add_signed(chrono::Duration::seconds(seconds))
                .ok_or(TokenRefreshError::Parse)
        })
        .transpose()?;

    let scopes = body
        .scope
        .as_ref()
        .map(|value| {
            let mut returned = Vec::new();
            let raw = value.expose_secret();
            if !is_rfc6749_scope(raw) {
                return Err(TokenRefreshError::Parse);
            }
            for scope in raw.split(' ') {
                if !state.scopes.iter().any(|configured| configured == scope)
                    || returned.iter().any(|seen| seen == scope)
                {
                    return Err(TokenRefreshError::Parse);
                }
                returned.push(scope.to_owned());
            }
            Ok(returned)
        })
        .transpose()?;

    let access_token = body.access_token.take().ok_or(TokenRefreshError::Parse)?;
    state.access_token = access_token;
    "Bearer".clone_into(&mut state.token_type);
    if let Some(refresh_token) = body.refresh_token.take() {
        state.refresh_token = Some(refresh_token);
    }
    // A newly-issued token must never inherit the previous token's expired
    // deadline. OAuth makes `expires_in` optional; absence means the new
    // expiry is unknown and the framework re-validation floor becomes the
    // backstop.
    state.expires_at = expires_at;
    if let Some(scopes) = scopes {
        state.scopes = scopes;
    }

    Ok(())
}

fn is_rfc5234_vschar(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
}

fn is_rfc6749_scope(value: &str) -> bool {
    !value.is_empty()
        && value.split(' ').all(|token| {
            !token.is_empty()
                && token
                    .bytes()
                    .all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
        })
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin, sync::Mutex};

    use nebula_storage_port::SecretBytes;

    use crate::runtime::{RefreshTransportError, TokenPostResponse};

    use super::*;

    fn sample_state() -> OAuth2State {
        OAuth2State {
            access_token: SecretString::new("old-token"),
            token_type: "Bearer".to_owned(),
            refresh_token: Some(SecretString::new("refresh-1")),
            expires_at: None,
            scopes: vec!["read".to_owned(), "write".to_owned()],
            client_id: SecretString::new("client"),
            client_secret: SecretString::new("secret"),
            token_url: "https://example.com/token".to_owned(),
            auth_style: AuthStyle::Header,
        }
    }

    fn parse_success(raw: &[u8]) -> TokenSuccessResponse {
        serde_json::from_slice(raw).expect("valid typed token response")
    }

    #[test]
    fn update_state_requires_access_token() {
        let mut state = sample_state();
        let body = parse_success(br#"{"token_type":"Bearer"}"#);
        let err = update_state_from_token_response(&mut state, body).unwrap_err();
        assert!(matches!(err, TokenRefreshError::Parse));
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
            br#"{"access_token":"new-token","token_type":"Bearer","refresh_token":"has space"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","refresh_token":"t\u00f6k\u00e9n"}"#.as_slice(),
            br#"{"access_token":"new-token","scope":"read attacker-scope"}"#.as_slice(),
            br#"{"access_token":"new-token","scope":"read read"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":""}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":"read  write"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":"read\twrite"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":"read\u0022write"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":"read\u005cwrite"}"#.as_slice(),
            br#"{"access_token":"new-token","expires_in":18446744073709551615}"#.as_slice(),
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

            assert!(matches!(error, TokenRefreshError::Parse));
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

    #[derive(Default)]
    struct CapturingTransport {
        basic_auth: Mutex<Option<(String, String)>>,
    }

    impl RefreshTransport for CapturingTransport {
        fn post_token<'a>(
            &'a self,
            request: TokenPostRequest,
        ) -> Pin<
            Box<dyn Future<Output = Result<TokenPostResponse, RefreshTransportError>> + Send + 'a>,
        > {
            let observed = request.basic_auth().map(|(client_id, client_secret)| {
                (
                    client_id.expose_secret().to_owned(),
                    client_secret.expose_secret().to_owned(),
                )
            });
            *self.basic_auth.lock().expect("capture lock") = observed;
            Box::pin(async {
                TokenPostResponse::try_new(
                    200,
                    SecretBytes::new(
                        br#"{"access_token":"new-token","token_type":"Bearer"}"#.to_vec(),
                    ),
                )
                .map_err(|_| RefreshTransportError::ReadBody)
            })
        }
    }

    #[tokio::test]
    async fn header_auth_form_encodes_each_raw_component_before_basic_join() {
        let cases = [
            ("client:name", "client%3Aname"),
            ("client%name", "client%25name"),
            ("client+name", "client%2Bname"),
            ("client name", "client+name"),
            ("cliënt", "cli%C3%ABnt"),
        ];

        for (raw, expected) in cases {
            let transport = CapturingTransport::default();
            let mut state = sample_state();
            state.client_id = SecretString::new(raw);
            state.client_secret = SecretString::new(raw);

            refresh_oauth2_state(&mut state, &transport)
                .await
                .expect("capturing transport returns a valid token response");
            let observed = transport
                .basic_auth
                .lock()
                .expect("capture lock")
                .take()
                .expect("Header auth must carry Basic components");
            assert_eq!(observed.0, expected);
            assert_eq!(observed.1, expected);
        }
    }

    #[test]
    fn known_provider_code_is_closed_and_low_cardinality() {
        let body = b"{\"error\":\"invalid_client\"}";
        let err = parse_token_response_bytes(401, body).expect_err("401 should fail");
        assert!(matches!(
            err,
            TokenRefreshError::TokenEndpoint {
                status: 401,
                code: OAuthProviderErrorCode::InvalidClient,
            }
        ));
        assert_eq!(
            err.to_string(),
            "token endpoint returned 401: invalid_client"
        );
    }

    #[test]
    fn invalid_grant_remains_a_typed_reauthentication_signal() {
        let body = br#"{
            "error":"invalid_grant",
            "error_description":"refresh_token=diagnostic-canary",
            "error_uri":"https://attacker.example/diagnostic-canary"
        }"#;
        let err = parse_token_response_bytes(400, body).expect_err("400 should fail");
        assert!(matches!(
            err,
            TokenRefreshError::InvalidGrant { status: 400 }
        ));
        let diagnostic = format!("{err:?} {err}");
        assert!(!diagnostic.contains("diagnostic-canary"));
        assert!(!diagnostic.contains("attacker.example"));

        let error = parse_token_response_bytes(500, br#"{"error":"invalid_grant"}"#)
            .expect_err("server failure must not become durable reauthentication");
        assert!(matches!(
            error,
            TokenRefreshError::TokenEndpoint {
                status: 500,
                code: OAuthProviderErrorCode::Other,
            }
        ));
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
            let err =
                parse_token_response_bytes(400, body.as_bytes()).expect_err("400 should fail");
            assert!(matches!(
                err,
                TokenRefreshError::TokenEndpoint {
                    code: OAuthProviderErrorCode::Other,
                    ..
                }
            ));
            let diagnostic = format!("{err:?} {err}");
            assert!(!diagnostic.contains("secret-canary"));
            assert!(!diagnostic.contains(&oversized));
            assert!(!diagnostic.contains('\u{1b}'));
            assert_eq!(err.to_string(), "token endpoint returned 400: other");
        }
    }

    #[test]
    fn redirects_are_non_success_provider_responses() {
        for status in [301, 302, 303, 307, 308] {
            let err = parse_token_response_bytes(status, b"")
                .expect_err("redirect response must never parse as success");
            assert!(matches!(
                err,
                TokenRefreshError::TokenEndpoint {
                    code: OAuthProviderErrorCode::Other,
                    ..
                }
            ));
        }
    }

    #[test]
    fn success_parse_errors_are_fixed_and_input_free() {
        let body = br#"{"access_token": {"diagnostic-canary":"secret"}}"#;
        let err =
            parse_token_response_bytes(200, body).expect_err("invalid JSON shape should fail");
        assert!(matches!(err, TokenRefreshError::Parse));
        let diagnostic = format!("{err:?} {err}");
        assert!(!diagnostic.contains("diagnostic-canary"));
        assert_eq!(err.to_string(), "failed to parse token response");
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
}
