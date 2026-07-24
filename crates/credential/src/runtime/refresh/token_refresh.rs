//! OAuth2 token-refresh state logic (ADR-0092).
//!
//! SSRF endpoint validation (SEC-10), secret-scoped form composition, response
//! status interpretation, SEC-02 error redaction, and `OAuth2State` mutation all
//! live here — on the `nebula-credential` side of the
//! [`RefreshTransport`](super::transport::RefreshTransport) seam. Network I/O
//! is deliberately absent: this module prepares a typed dispatch payload and
//! interprets a completed response, leaving the provider future to the
//! phase-aware caller.
//!
//! # Sentinel marking
//!
//! Per sub-spec `docs/INTEGRATION_MODEL.md` the holder marks the L2 claim row
//! `sentinel = RefreshInFlight` immediately before the IdP POST. That mark is
//! durably acknowledged by `RefreshCoordinator::refresh_coalesced` before it
//! starts the owned resolver closure that dispatches the payload returned by
//! `prepare_oauth2_refresh`. This module therefore cannot be entered through
//! the coordinated path before the point of no cancellation, and it does not
//! need a `RefreshClaim` or claim repository in the transport layer.
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
use crate::runtime::refresh::transport::{TokenPostRequest, TokenPostResponse};
use crate::runtime::{OAuthEndpointError, OAuthServerEndpoint};

pub use super::transport::OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES;

/// Exact failure while preparing an OAuth2 refresh request.
///
/// Every variant is produced before a [`TokenPostRequest`] can cross the
/// transport boundary. The type therefore carries structural proof that no
/// provider state transition was attempted.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrepareTokenRefreshError {
    /// Stored state lacks a refresh token, so re-auth is required.
    #[error("no refresh_token available for token refresh")]
    MissingRefreshToken,
    /// The stored refresh token is not a non-empty RFC 5234 visible string.
    #[error("stored refresh_token is invalid")]
    InvalidRefreshToken,
    /// The stored scope set cannot be encoded as an RFC 6749 scope value.
    #[error("stored OAuth2 scopes are invalid")]
    InvalidScopes,
    /// Endpoint validation failed before request construction.
    #[error("refresh token request rejected before dispatch: {0}")]
    InvalidEndpoint(#[source] OAuthEndpointError),
}

/// Fully validated OAuth2 token-refresh dispatch payload.
///
/// The payload is intentionally linear: it is not `Clone`, and
/// [`Self::into_request`] consumes it at the exact provider-dispatch boundary.
/// Secret-bearing fields remain inside [`TokenPostRequest`] and keep its
/// constant-redacted `Debug` and zeroizing drop behavior.
#[must_use = "a prepared token refresh must be dispatched or explicitly dropped"]
pub(crate) struct PreparedTokenRefresh {
    request: TokenPostRequest,
}

impl PreparedTokenRefresh {
    /// Consume the prepared payload at the transport-dispatch boundary.
    pub(crate) fn into_request(self) -> TokenPostRequest {
        self.request
    }
}

impl fmt::Debug for PreparedTokenRefresh {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedTokenRefresh(<redacted>)")
    }
}

/// Interpretation of a completed OAuth2 token-endpoint response.
///
/// This enum says nothing about transport errors: absence of a complete
/// response is classified by the phase-aware caller. Each non-success variant
/// is intentionally distinct so replay-safe denials cannot be confused with
/// transient, malformed, or intermediary-controlled responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the completed provider response must determine refresh disposition"]
pub(crate) enum CompletedTokenRefresh {
    /// A fully validated success response was applied to stored state.
    Refreshed,
    /// RFC 6749 `invalid_grant` on the only protocol-consistent status.
    InvalidGrant {
        /// HTTP status code.
        status: u16,
    },
    /// A recognized RFC-consistent denial proves the request had no effect.
    DefinitiveNoEffect {
        /// HTTP status code.
        status: u16,
        /// Closed, low-cardinality OAuth error code.
        code: OAuthProviderErrorCode,
    },
    /// A transient, unknown, or status/code-mismatched denial is replay-unsafe.
    AmbiguousDenial {
        /// HTTP status code.
        status: u16,
        /// Closed, low-cardinality OAuth error code.
        code: OAuthProviderErrorCode,
    },
    /// A 2xx response could not be fully validated and applied.
    MalformedSuccess {
        /// HTTP status code.
        status: u16,
    },
}

/// Closed OAuth token-endpoint error classification.
///
/// Provider-controlled extension text never enters diagnostics. Standard
/// error codes retain useful low-cardinality meaning; every absent, malformed,
/// oversized, control-bearing, or unknown value collapses to [`Self::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OAuthProviderErrorCode {
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

impl OAuthProviderErrorCode {
    /// Return the fixed low-cardinality OAuth diagnostic code.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidClient => "invalid_client",
            Self::UnauthorizedClient => "unauthorized_client",
            Self::UnsupportedGrantType => "unsupported_grant_type",
            Self::InvalidScope => "invalid_scope",
            Self::TemporarilyUnavailable => "temporarily_unavailable",
            Self::ServerError => "server_error",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for OAuthProviderErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str((*self).as_str())
    }
}

/// Validate OAuth2 refresh state and build a transport-ready request.
///
/// Call order (security boundary — do not reorder):
/// 1. Local state and [`OAuthServerEndpoint`] validation complete.
/// 2. A secret-scoped block builds a [`TokenPostRequest`] (form fields +
///    optional `basic_auth`). Secret borrows are released when the block ends.
/// 3. The linear [`PreparedTokenRefresh`] is returned to the phase-aware
///    caller. No network future is created or polled in this module.
///
/// SEC-10: the three secret values (refresh_token, client_id, client_secret)
/// are NOT extracted into `Zeroizing<String>` intermediates. Instead, secret
/// borrows live inside an inner block that returns the built
/// `TokenPostRequest`; the block ends → secret borrows drop → `state` is free
/// for later `&mut` mutation. No ordinary owned plaintext copy lives in our
/// code; the unavoidable in-flight copy lives in the transport's request
/// serialization and is released after its response future resolves.
pub(crate) fn prepare_oauth2_refresh(
    state: &OAuth2State,
) -> Result<PreparedTokenRefresh, PrepareTokenRefreshError> {
    let refresh_token = state
        .refresh_token
        .as_ref()
        .ok_or(PrepareTokenRefreshError::MissingRefreshToken)?;
    if !is_rfc5234_vschar(refresh_token.expose_secret()) {
        return Err(PrepareTokenRefreshError::InvalidRefreshToken);
    }
    if state.scopes.iter().enumerate().any(|(index, scope)| {
        !is_rfc6749_scope_token(scope) || state.scopes[..index].contains(scope)
    }) {
        return Err(PrepareTokenRefreshError::InvalidScopes);
    }

    // SSRF validation must complete before request construction or I/O.
    let endpoint = OAuthServerEndpoint::parse(&state.token_url)
        .map_err(PrepareTokenRefreshError::InvalidEndpoint)?;
    let scope_joined: Option<String> = (!state.scopes.is_empty()).then(|| state.scopes.join(" "));

    // Build the request inside a tight secret-borrow scope.
    // After this block the secret borrows have dropped; only `TokenPostRequest`
    // (carrying `SecretString` values) crosses the block boundary.
    let request = {
        let refresh_tok = refresh_token.expose_secret();
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

    Ok(PreparedTokenRefresh { request })
}

/// Interpret one completed OAuth2 token response and update state on success.
///
/// SEC-01: the response body is structurally bounded by
/// [`TokenPostResponse::try_new`]. SEC-02: non-success bodies are parsed only
/// into a zeroizing typed envelope; provider descriptions, URIs, extension
/// fields, and raw parser text never enter the result taxonomy.
pub(crate) fn interpret_oauth2_refresh_response(
    state: &mut OAuth2State,
    response: TokenPostResponse,
) -> CompletedTokenRefresh {
    let status = response.status();
    if !(200..300).contains(&status) {
        return classify_completed_denial(
            status,
            parse_provider_error_code(response.body().as_ref()),
        );
    }

    let Ok(body) = serde_json::from_slice(response.body().as_ref()) else {
        return CompletedTokenRefresh::MalformedSuccess { status };
    };
    if update_state_from_token_response(state, body).is_err() {
        return CompletedTokenRefresh::MalformedSuccess { status };
    }
    CompletedTokenRefresh::Refreshed
}

fn form_encode_basic_component(raw: &str) -> SecretString {
    let mut encoded = Zeroizing::new(String::with_capacity(raw.len()));
    for part in url::form_urlencoded::byte_serialize(raw.as_bytes()) {
        encoded.push_str(part);
    }
    SecretString::new(std::mem::take(&mut *encoded))
}

fn classify_completed_denial(
    status: u16,
    parsed_code: ParsedProviderErrorCode,
) -> CompletedTokenRefresh {
    match parsed_code {
        ParsedProviderErrorCode::InvalidGrant if status == 400 => {
            CompletedTokenRefresh::InvalidGrant { status }
        },
        ParsedProviderErrorCode::Public(code) if is_definitive_no_effect(status, code) => {
            CompletedTokenRefresh::DefinitiveNoEffect { status, code }
        },
        parsed_code => CompletedTokenRefresh::AmbiguousDenial {
            status,
            code: parsed_code.into_public(),
        },
    }
}

fn is_definitive_no_effect(status: u16, code: OAuthProviderErrorCode) -> bool {
    matches!(
        (status, code),
        (
            400,
            OAuthProviderErrorCode::InvalidRequest
                | OAuthProviderErrorCode::InvalidClient
                | OAuthProviderErrorCode::UnauthorizedClient
                | OAuthProviderErrorCode::UnsupportedGrantType
                | OAuthProviderErrorCode::InvalidScope
        ) | (401, OAuthProviderErrorCode::InvalidClient)
    )
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MalformedTokenSuccess;

fn update_state_from_token_response(
    state: &mut OAuth2State,
    mut body: TokenSuccessResponse,
) -> Result<(), MalformedTokenSuccess> {
    let access_token = body.access_token.as_ref().ok_or(MalformedTokenSuccess)?;
    if !is_rfc5234_vschar(access_token.expose_secret()) {
        return Err(MalformedTokenSuccess);
    }

    let token_type = body.token_type.as_ref().ok_or(MalformedTokenSuccess)?;
    if !token_type.expose_secret().eq_ignore_ascii_case("bearer") {
        return Err(MalformedTokenSuccess);
    }

    if body
        .refresh_token
        .as_ref()
        .is_some_and(|token| !is_rfc5234_vschar(token.expose_secret()))
    {
        return Err(MalformedTokenSuccess);
    }

    let expires_at = body
        .expires_in
        .map(|expires_in| {
            let seconds = i64::try_from(expires_in).map_err(|_| MalformedTokenSuccess)?;
            Utc::now()
                .checked_add_signed(chrono::Duration::seconds(seconds))
                .ok_or(MalformedTokenSuccess)
        })
        .transpose()?;

    let scopes = body
        .scope
        .as_ref()
        .map(|value| {
            let mut returned = Vec::new();
            let raw = value.expose_secret();
            if !is_rfc6749_scope(raw) {
                return Err(MalformedTokenSuccess);
            }
            for scope in raw.split(' ') {
                if !state.scopes.iter().any(|configured| configured == scope)
                    || returned.iter().any(|seen| seen == scope)
                {
                    return Err(MalformedTokenSuccess);
                }
                returned.push(scope.to_owned());
            }
            Ok(returned)
        })
        .transpose()?;

    let access_token = body.access_token.take().ok_or(MalformedTokenSuccess)?;
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

fn is_rfc6749_scope_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
}

fn is_rfc6749_scope(value: &str) -> bool {
    !value.is_empty() && value.split(' ').all(is_rfc6749_scope_token)
}

#[cfg(test)]
mod tests {
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
            br#"{"access_token":"new-token","token_type":"Bearer","refresh_token":"has space"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","refresh_token":"t\u00f6k\u00e9n"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":"read attacker-scope"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":"read read"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":""}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":"read  write"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":"read\twrite"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":"read\u0022write"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","scope":"read\u005cwrite"}"#.as_slice(),
            br#"{"access_token":"new-token","token_type":"Bearer","expires_in":18446744073709551615}"#.as_slice(),
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
            let completed =
                interpret_oauth2_refresh_response(&mut state, token_response(status, b""));
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
        let canary =
            prepare_oauth2_refresh(&canary_state).expect("canary state still prepares safely");

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
}
