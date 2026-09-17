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
#[path = "token_refresh_tests.rs"]
mod tests;
