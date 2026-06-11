//! The OAuth2 token-refresh transport seam (ADR-0092).
//!
//! [`RefreshTransport`] is the **narrow** port that inverts the only network
//! I/O on the credential refresh path: a single bounded HTTP `POST` to an
//! OAuth2 token endpoint. It exists so the reqwest dependency (and any other
//! HTTP client) lives in the composition root (`nebula-api` / `nebula-engine`)
//! rather than in this Core crate.
//!
//! # Security boundary (deliberately asymmetric — keep it that way)
//!
//! Everything that constitutes a *policy decision* stays on the
//! `nebula-credential` side of this seam; the transport is a dumb pipe:
//!
//! | Concern | Owner | Why |
//! |---------|-------|-----|
//! | SSRF endpoint validation (https-only, no localhost / private / link-local / ULA) | **credential** (`validate_token_endpoint`, runs BEFORE `post_token`) | A transport-side check could be skipped by a second composition root wiring a different `RefreshTransport`. |
//! | OAuth2 form composition + `AuthStyle` placement of `client_id`/`client_secret` | **credential** | Which fields, header-vs-body, is OAuth2 domain knowledge, not transport knowledge. |
//! | Response status interpretation + body parse + `OAuth2State` mutation + SEC-02 error redaction | **credential** | The transport returns raw `(status, bytes)`; it never interprets them. |
//! | SEC-01 bounded body | **credential dictates** via [`TokenPostRequest::max_response_bytes`]; transport MUST enforce it | The cap value is policy; the mechanical read is the transport's. |
//!
//! The transport's ONLY job: take a fully-composed request, perform the POST
//! with its hardened client (connect-time private-IP blocking is the
//! defense-in-depth layer it SHOULD add — DNS-rebind cannot be caught by the
//! credential-side string validation), read at most `max_response_bytes`, and
//! return the status + bytes. It makes no decisions about *what* to send or
//! *whether* the response is acceptable.
//!
//! A wider seam (e.g. `refresh(&mut OAuth2State)`) would export the SSRF
//! defense and secret-scoping into the composition root, where a second wiring
//! could bypass them. Do not widen it.

use std::future::Future;
use std::pin::Pin;

use crate::SecretString;

/// A fully-composed OAuth2 token-endpoint `POST`, ready to transmit.
///
/// Built entirely by `nebula-credential` (URL already SSRF-validated, form
/// already assembled per `AuthStyle`). Secret-bearing values are carried as
/// [`SecretString`] so the zeroizing best-effort survives up to the transport
/// boundary; the transport exposes them only to hand reqwest its (unavoidable,
/// non-zeroizing) serialized copy.
pub struct TokenPostRequest {
    /// Target token endpoint. Already validated https + non-private by the
    /// credential side; the transport SHOULD still apply connect-time private-IP
    /// blocking against DNS rebinding.
    pub url: String,
    /// `application/x-www-form-urlencoded` fields, in order. Non-secret keys
    /// (`grant_type`, `scope`) are wrapped uniformly; secret keys
    /// (`refresh_token`, and `client_id`/`client_secret` under `PostBody`)
    /// carry the actual secrets.
    pub form: Vec<(String, SecretString)>,
    /// HTTP Basic credentials `(client_id, client_secret)` for `AuthStyle::Header`;
    /// `None` for `PostBody` (where they live in `form` instead).
    pub basic_auth: Option<(String, SecretString)>,
    /// SEC-01 hard cap on the response body the transport will read. The
    /// transport MUST stop reading past this and return what it has (the
    /// credential side treats an over-cap/parse failure as a refresh error).
    pub max_response_bytes: usize,
}

/// Raw outcome of [`RefreshTransport::post_token`] — status + bounded body.
///
/// The transport does NOT interpret these; the credential side performs the
/// success/error branch, bounded parse, and SEC-02 redaction.
pub struct TokenPostResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body, already truncated to `max_response_bytes`.
    pub body: Vec<u8>,
}

/// Transport-level failure (connect / send / read). Carries no secret-bearing
/// detail; the credential side wraps it into a `SecretFreeMessage` provider
/// error.
#[derive(Debug, thiserror::Error)]
pub enum RefreshTransportError {
    /// Connecting to / sending the request failed (incl. transport-enforced
    /// private-IP refusal, TLS errors, timeouts).
    #[error("token endpoint request failed: {0}")]
    Send(String),
    /// Reading the (bounded) response body failed.
    #[error("token endpoint response read failed: {0}")]
    ReadBody(String),
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
