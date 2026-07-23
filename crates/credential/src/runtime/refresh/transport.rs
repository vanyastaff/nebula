//! The OAuth2 token-refresh transport seam (ADR-0092).
//!
//! [`RefreshTransport`] is the **narrow** port that inverts the only network
//! I/O on the credential refresh path: a single bounded HTTP `POST` to an
//! OAuth2 token endpoint. It exists so the reqwest dependency (and any other
//! HTTP client) lives in the first-party composition root (`apps/server`)
//! rather than in this Core crate. `apps/server` is the sole first-party
//! network implementation; API fixtures inject a deterministic no-network
//! adapter, while downstream hosts must explicitly supply an adapter that
//! honors this port's security contract.
//!
//! # Security boundary (deliberately asymmetric — keep it that way)
//!
//! Everything that constitutes a *policy decision* stays on the
//! `nebula-credential` side of this seam; the transport is a dumb pipe:
//!
//! | Concern | Owner | Why |
//! |---------|-------|-----|
//! | SSRF endpoint validation (HTTPS shape + global literal) | **credential** ([`OAuthServerEndpoint`], built BEFORE `post_token`) | A transport-side string check could be skipped by a second composition root wiring a different `RefreshTransport`. |
//! | DNS answer validation (all answers global; exact answers used for connect) | **composition root** using credential-owned `validate_oauth_dns_answers` | DNS resolution happens inside the concrete client's connect path; validating earlier would leave a rebinding TOCTOU window. |
//! | OAuth2 form composition + `AuthStyle` placement of `client_id`/`client_secret` | **credential** | Which fields, header-vs-body, is OAuth2 domain knowledge, not transport knowledge. |
//! | Response status interpretation + typed parse + `OAuth2State` mutation + closed diagnostics | **credential** | The transport returns an opaque zeroizing `(status, bytes)` value; it never interprets it. |
//! | SEC-01 bounded body | **credential dictates** via [`TokenPostRequest::max_response_bytes`]; transport MUST enforce it | The cap value is policy; the mechanical read is the transport's. |
//!
//! The transport's ONLY job: take a fully-composed request, perform the POST
//! with its hardened client (connect-time all-answer global-unicast validation
//! is mandatory — DNS rebinding cannot be caught by credential-side URL
//! validation), read at most `max_response_bytes`, and return the status +
//! zeroizing bytes. It makes no decisions about *what* to send or *whether*
//! the response is acceptable.
//!
//! A wider seam (e.g. `refresh(&mut OAuth2State)`) would export the SSRF
//! defense and secret-scoping into the composition root, where a second wiring
//! could bypass them. Do not widen it.

use std::{fmt, future::Future, pin::Pin};

use nebula_storage_port::SecretBytes;

use crate::SecretString;
use crate::runtime::OAuthServerEndpoint;

/// Fixed hard cap for OAuth token-endpoint response bodies.
///
/// [`TokenPostRequest::max_response_bytes`] tells transports where to stop
/// streaming, while [`TokenPostResponse::try_new`] enforces the same boundary
/// structurally for every first-party or downstream adapter.
pub const OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// A fully-composed OAuth2 token-endpoint `POST`, ready to transmit.
///
/// Built entirely by `nebula-credential` (endpoint already SSRF-validated, form
/// already assembled per `AuthStyle`). Secret-bearing values are carried as
/// [`SecretString`] so the zeroizing best-effort survives up to the transport
/// boundary; the transport exposes them only to hand reqwest its (unavoidable,
/// non-zeroizing) serialized copy.
pub struct TokenPostRequest {
    /// Target token endpoint. URL shape and literal hosts are already
    /// validated; the transport MUST still validate every connect-time DNS
    /// answer and return those exact addresses to its connector.
    endpoint: OAuthServerEndpoint,
    /// `application/x-www-form-urlencoded` fields, in order. Non-secret keys
    /// (`grant_type`, `scope`) are wrapped uniformly; secret keys
    /// (`refresh_token`, and `client_id`/`client_secret` under `PostBody`)
    /// carry the actual secrets.
    form: Vec<(String, SecretString)>,
    /// RFC 6749 §2.3.1 form-encoded HTTP Basic components
    /// `(client_id, client_secret)` for `AuthStyle::Header`; `None` for
    /// `PostBody` (where raw values live in `form` and the transport's form
    /// serializer encodes them exactly once).
    basic_auth: Option<(SecretString, SecretString)>,
}

impl TokenPostRequest {
    pub(crate) fn new(
        endpoint: OAuthServerEndpoint,
        form: Vec<(String, SecretString)>,
        basic_auth: Option<(SecretString, SecretString)>,
    ) -> Self {
        Self {
            endpoint,
            form,
            basic_auth,
        }
    }

    /// Borrow the validated token endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &OAuthServerEndpoint {
        &self.endpoint
    }

    /// Borrow the ordered form fields.
    #[must_use]
    pub fn form(&self) -> &[(String, SecretString)] {
        &self.form
    }

    /// Borrow optional HTTP Basic credentials.
    #[must_use]
    pub fn basic_auth(&self) -> Option<&(SecretString, SecretString)> {
        self.basic_auth.as_ref()
    }

    /// Return the mandatory response-body cap.
    #[must_use]
    pub fn max_response_bytes(&self) -> usize {
        OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES
    }
}

impl fmt::Debug for TokenPostRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenPostRequest(<redacted>)")
    }
}

/// Raw outcome of [`RefreshTransport::post_token`] — status + bounded body.
///
/// The transport does NOT interpret these; the credential side performs the
/// success/error branch, bounded parse, and SEC-02 redaction.
pub struct TokenPostResponse {
    status: u16,
    body: SecretBytes,
}

impl TokenPostResponse {
    /// Construct a response from its status and zeroizing bounded body.
    ///
    /// The fixed policy cap is checked again here even when the concrete
    /// transport already enforced it while streaming. This prevents a custom
    /// adapter from injecting an unbounded body across the port.
    ///
    /// # Errors
    ///
    /// Returns [`TokenPostResponseError::BodyTooLarge`] when `body` exceeds
    /// [`OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES`], or
    /// [`TokenPostResponseError::InvalidStatus`] when `status` is outside the
    /// HTTP status-code range `100..=599`.
    pub fn try_new(status: u16, body: SecretBytes) -> Result<Self, TokenPostResponseError> {
        if !(100..=599).contains(&status) {
            return Err(TokenPostResponseError::InvalidStatus);
        }
        if body.len() > OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES {
            return Err(TokenPostResponseError::BodyTooLarge);
        }
        Ok(Self { status, body })
    }

    /// Return the HTTP status code.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Borrow the opaque zeroizing response body.
    #[must_use]
    pub fn body(&self) -> &SecretBytes {
        &self.body
    }
}

/// Failure to construct a policy-valid [`TokenPostResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TokenPostResponseError {
    /// The numeric value is not an HTTP status code.
    #[error("OAuth token response status is invalid")]
    InvalidStatus,
    /// The opaque body exceeds the credential-owned fixed cap.
    #[error("OAuth token response exceeds the fixed body limit")]
    BodyTooLarge,
}

impl fmt::Debug for TokenPostResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenPostResponse(<redacted>)")
    }
}

/// Transport-level failure (connect / send / read). Carries no secret-bearing
/// detail; the credential side wraps it into a `SecretFreeMessage` provider
/// error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshTransportError {
    /// Connecting to / sending the request failed (incl. transport-enforced
    /// private-IP refusal, TLS errors, timeouts).
    #[error("token endpoint request failed")]
    Send,
    /// Reading the (bounded) response body failed.
    #[error("token endpoint response read failed")]
    ReadBody,
}

/// The narrow OAuth2 token-`POST` port. Object-safe (`Arc<dyn RefreshTransport>`)
/// via a boxed future; one allocation per refresh is negligible on this rare
/// path. The reqwest implementation lives in the composition root.
pub trait RefreshTransport: Send + Sync {
    /// Perform the composed token-endpoint `POST` and return the raw
    /// `(status, bounded-body)`. Implementations MUST honor
    /// [`TokenPostRequest::max_response_bytes`] and MUST NOT inspect or
    /// transform the body.
    fn post_token<'a>(
        &'a self,
        request: TokenPostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<TokenPostResponse, RefreshTransportError>> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_bearing_transport_dtos_have_constant_redacted_debug() {
        let endpoint =
            OAuthServerEndpoint::parse("https://provider.example/token").expect("valid endpoint");
        let first_request = TokenPostRequest::new(
            endpoint.clone(),
            vec![("refresh_token".to_owned(), SecretString::new("short"))],
            None,
        );
        let second_request = TokenPostRequest::new(
            endpoint,
            vec![(
                "refresh_token".to_owned(),
                SecretString::new("request-diagnostic-canary-with-a-different-length"),
            )],
            Some((
                SecretString::new("client-diagnostic-canary"),
                SecretString::new("client-secret-diagnostic-canary"),
            )),
        );
        assert_eq!(format!("{first_request:?}"), format!("{second_request:?}"));
        assert!(!format!("{second_request:?}").contains("diagnostic-canary"));

        let first_response = TokenPostResponse::try_new(200, SecretBytes::new(b"short".to_vec()))
            .expect("bounded response");
        let second_response = TokenPostResponse::try_new(
            200,
            SecretBytes::new(b"response-diagnostic-canary-with-a-different-length".to_vec()),
        )
        .expect("bounded response");
        assert_eq!(
            format!("{first_response:?}"),
            format!("{second_response:?}")
        );
        assert!(!format!("{second_response:?}").contains("diagnostic-canary"));
    }

    #[test]
    fn oversized_custom_adapter_response_is_structurally_rejected() {
        let oversized = SecretBytes::new(vec![
            0;
            OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES.saturating_add(1)
        ]);
        let error = TokenPostResponse::try_new(200, oversized)
            .expect_err("custom adapter must not cross the fixed response-body boundary");

        assert_eq!(error, TokenPostResponseError::BodyTooLarge);
        assert_eq!(
            error.to_string(),
            "OAuth token response exceeds the fixed body limit"
        );
    }

    #[test]
    fn response_at_fixed_body_limit_is_accepted() {
        let boundary = SecretBytes::new(vec![0; OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES]);
        assert!(TokenPostResponse::try_new(200, boundary).is_ok());
    }

    #[test]
    fn response_status_range_is_structurally_enforced() {
        for status in [0, 1, 99, 600, u16::MAX] {
            let error = TokenPostResponse::try_new(status, SecretBytes::default())
                .expect_err("non-HTTP status must not cross the transport port");
            assert_eq!(error, TokenPostResponseError::InvalidStatus);
            assert_eq!(error.to_string(), "OAuth token response status is invalid");
        }

        for status in [100, 599] {
            assert!(
                TokenPostResponse::try_new(status, SecretBytes::default()).is_ok(),
                "HTTP status boundary {status} must be accepted"
            );
        }
    }
}
