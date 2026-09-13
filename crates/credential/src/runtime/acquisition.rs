//! OAuth2 initial-acquisition transport boundary.
//!
//! This port is intentionally distinct from refresh transport authority.
//! Initial token exchange can consume one-time authorization codes or create
//! provider grants, while refresh is dispatched only inside the refresh
//! coordinator's provider/persistence critical section. Sharing request and
//! response envelopes does not share authority: each operation receives only
//! its own trait object through [`crate::CredentialContext`].

use std::{future::Future, pin::Pin};

use super::{TokenPostRequest, TokenPostResponse};

/// Payload-free failure from an OAuth2 initial token exchange transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AcquisitionTransportError {
    /// Connecting to or sending the request failed.
    #[error("token acquisition request failed")]
    Send,
    /// Reading or structurally bounding the response failed.
    #[error("token acquisition response read failed")]
    ReadBody,
}

/// Narrow transport port for OAuth2 initial token acquisition.
///
/// Implementations must apply the same DNS rebinding, TLS, timeout, and body
/// bounds documented by [`TokenPostRequest`]. Provider response semantics stay
/// in `nebula-credential`; the adapter is only a hardened byte transport.
pub trait AcquisitionTransport: Send + Sync {
    /// POST one fully composed token request and return its bounded response.
    fn post_token<'a>(
        &'a self,
        request: TokenPostRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<TokenPostResponse, AcquisitionTransportError>> + Send + 'a>,
    >;
}
