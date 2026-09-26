//! The author's credential hook: how a resource's pinned slots become
//! request headers, applied after the attempt is granted.

use base64::Engine as _;
use http::{HeaderMap, HeaderName, HeaderValue, header::AUTHORIZATION};
use nebula_credential::{CredentialGuard, IdentityPassword, OAuth2Token, SecretToken};
use nebula_resource::{CredentialUnavailableReason, ErrorKind, PinSlots, Provider, call::OpError};
use zeroize::{Zeroize, Zeroizing};

use super::config::HttpTransport;

mod sealed {
    use nebula_credential::{OAuth2Token, SecretToken};
    use nebula_resource::{ErrorKind, call::OpError};

    /// Seals [`BearerMaterial`](super::BearerMaterial) and reads the token.
    pub trait Sealed {
        /// The bearer token, or why it cannot be used now.
        fn bearer_token(&self) -> Result<&str, OpError>;
    }

    impl Sealed for SecretToken {
        fn bearer_token(&self) -> Result<&str, OpError> {
            Ok(self.token().expose_secret())
        }
    }

    impl Sealed for OAuth2Token {
        fn bearer_token(&self) -> Result<&str, OpError> {
            if self.is_expired() {
                return Err(OpError::new(
                    ErrorKind::Transient,
                    "oauth2 access token expired; waiting for its refresh",
                ));
            }
            Ok(self.access_token().expose_secret())
        }
    }
}

/// Credential material that authenticates as `Authorization: Bearer …`:
/// [`SecretToken`] (bearer tokens, API tokens) and [`OAuth2Token`]. Sealed.
pub trait BearerMaterial: sealed::Sealed + Zeroize + Send + Sync + 'static {}

impl BearerMaterial for SecretToken {}
impl BearerMaterial for OAuth2Token {}

/// An HTTP API resource: a [`Provider`] whose instance holds an
/// [`HttpTransport`], and which says how its pinned credential slots
/// authenticate a request.
///
/// ```
/// use nebula_sdk::integration::credential::BearerTokenCredential;
/// use nebula_sdk::integration::resource::{
///     CredentialSlot, Error, OpError, Provider, Resident, ResidentProvider, Resource,
///     ResourceContext, ResourceKey, ResourceMetadataDraft, resource_key,
///     http::{Authorize, HttpApi, HttpConfig, HttpTransport},
/// };
///
/// #[derive(Resource)]
/// struct GitHub {
///     #[credential(key = "token")]
///     token: CredentialSlot<BearerTokenCredential>,
/// }
///
/// #[async_trait::async_trait]
/// impl Provider for GitHub {
///     type Config = HttpConfig;
///     type Instance = HttpTransport;
///     type Topology = Resident<Self>;
///
///     fn key() -> ResourceKey {
///         resource_key!("example.github")
///     }
///
///     fn metadata() -> ResourceMetadataDraft {
///         ResourceMetadataDraft::new(
///             Self::key(),
///             nebula_sdk::prelude::metadata_name!("GitHub"),
///             "",
///         )
///     }
///
///     async fn create(&self, config: &HttpConfig, _: &ResourceContext) -> Result<HttpTransport, Error> {
///         HttpTransport::new(config)
///     }
/// }
///
/// impl ResidentProvider for GitHub {}
///
/// impl HttpApi for GitHub {
///     fn authorize(slots: &Self::Pinned, auth: &mut Authorize<'_>) -> Result<(), OpError> {
///         auth.bearer(slots.token())
///     }
/// }
/// ```
pub trait HttpApi: Provider + PinSlots
where
    Self::Instance: AsRef<HttpTransport>,
{
    /// Applies the unit's pinned credentials to one attempt's request.
    ///
    /// Called after the attempt was granted, last, just before the request
    /// is sent; an error settles the attempt `NotSent`. A slot pinned
    /// `None` fails the attempt `CredentialUnavailable` (`Absent`) through
    /// the [`Authorize`] helpers.
    ///
    /// # Errors
    ///
    /// Whatever the helpers return, or the author's own refusal.
    fn authorize(slots: &Self::Pinned, auth: &mut Authorize<'_>) -> Result<(), OpError>;
}

/// The credential headers of one attempt, filled by
/// [`HttpApi::authorize`]. Values are built in zeroized buffers and marked
/// sensitive, so they never reach `Debug` or HTTP/2 header compression
/// tables.
pub struct Authorize<'a> {
    headers: &'a mut HeaderMap,
}

impl std::fmt::Debug for Authorize<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.keys().map(HeaderName::as_str).collect();
        formatter
            .debug_struct("Authorize")
            .field("headers", &names)
            .finish()
    }
}

fn absent() -> OpError {
    OpError::new(
        ErrorKind::CredentialUnavailable {
            reason: CredentialUnavailableReason::Absent,
        },
        "no credential bound to the slot this request needs",
    )
}

/// `parts` joined in one zeroized buffer sized up front, so no growth
/// leaves an unzeroed copy behind.
fn joined(parts: &[&str]) -> Zeroizing<String> {
    let mut buffer = Zeroizing::new(String::with_capacity(
        parts.iter().map(|part| part.len()).sum(),
    ));
    for part in parts {
        buffer.push_str(part);
    }
    buffer
}

fn sensitive(value: &str) -> Result<HeaderValue, OpError> {
    let mut header = HeaderValue::from_str(value).map_err(|_| {
        OpError::new(
            ErrorKind::Permanent,
            "credential material is not a valid header value",
        )
    })?;
    header.set_sensitive(true);
    Ok(header)
}

impl<'a> Authorize<'a> {
    pub(super) fn new(headers: &'a mut HeaderMap) -> Self {
        Self { headers }
    }

    /// `Authorization: Bearer <token>`.
    ///
    /// # Errors
    ///
    /// `CredentialUnavailable` (`Absent`) for an unbound slot; `Transient`
    /// for an expired OAuth2 token; `Permanent` for a token that is not a
    /// valid header value.
    pub fn bearer<S: BearerMaterial>(
        &mut self,
        guard: Option<&CredentialGuard<S>>,
    ) -> Result<(), OpError> {
        let material: &S = guard.ok_or_else(absent)?;
        let value = joined(&["Bearer ", material.bearer_token()?]);
        self.headers.insert(AUTHORIZATION, sensitive(&value)?);
        Ok(())
    }

    /// `Authorization: Basic <base64(identity:password)>`.
    ///
    /// # Errors
    ///
    /// `CredentialUnavailable` (`Absent`) for an unbound slot.
    pub fn basic(
        &mut self,
        guard: Option<&CredentialGuard<IdentityPassword>>,
    ) -> Result<(), OpError> {
        let material: &IdentityPassword = guard.ok_or_else(absent)?;
        let pair = joined(&[
            material.identity(),
            ":",
            material.password().expose_secret(),
        ]);
        let mut value = Zeroizing::new(String::with_capacity(
            "Basic ".len() + base64::encoded_len(pair.len(), true).unwrap_or(0),
        ));
        value.push_str("Basic ");
        base64::engine::general_purpose::STANDARD.encode_string(pair.as_bytes(), &mut value);
        self.headers.insert(AUTHORIZATION, sensitive(&value)?);
        Ok(())
    }

    /// The token as the value of header `name` (`X-Api-Key`). There is no
    /// query-parameter scheme: a key in a URL leaks into logs.
    ///
    /// # Errors
    ///
    /// `CredentialUnavailable` (`Absent`) for an unbound slot; `Permanent`
    /// for an invalid header name or value.
    pub fn api_key_header(
        &mut self,
        name: &'static str,
        guard: Option<&CredentialGuard<SecretToken>>,
    ) -> Result<(), OpError> {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| OpError::new(ErrorKind::Permanent, "invalid api key header name"))?;
        let material: &SecretToken = guard.ok_or_else(absent)?;
        let value = sensitive(material.token().expose_secret())?;
        self.headers.insert(name, value);
        Ok(())
    }

    /// Sends the request without credentials, stating so.
    ///
    /// # Errors
    ///
    /// Never; the signature matches the other helpers.
    pub fn none(&mut self) -> Result<(), OpError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use nebula_credential::SecretString;

    use super::*;

    fn apply(
        fill: impl FnOnce(&mut Authorize<'_>) -> Result<(), OpError>,
    ) -> Result<HeaderMap, OpError> {
        let mut headers = HeaderMap::new();
        fill(&mut Authorize::new(&mut headers))?;
        Ok(headers)
    }

    fn token(value: &str) -> CredentialGuard<SecretToken> {
        CredentialGuard::new(SecretToken::new(SecretString::new(value)))
    }

    #[test]
    fn an_unbound_slot_is_credential_unavailable_absent() {
        let error = apply(|auth| auth.bearer::<SecretToken>(None)).expect_err("absent");
        assert_eq!(
            *error.kind(),
            ErrorKind::CredentialUnavailable {
                reason: CredentialUnavailableReason::Absent
            }
        );
        assert!(apply(|auth| auth.basic(None)).is_err());
        assert!(apply(|auth| auth.api_key_header("x-api-key", None)).is_err());
    }

    #[test]
    fn bearer_values_are_sensitive() {
        let guard = token("tok-secret");
        let headers = apply(|auth| auth.bearer(Some(&guard))).expect("bearer");
        let value = headers.get(AUTHORIZATION).expect("authorization");
        assert!(value.is_sensitive());
        assert_eq!(value.to_str().expect("ascii"), "Bearer tok-secret");
        assert!(!format!("{headers:?}").contains("tok-secret"));
    }

    #[test]
    fn an_expired_oauth2_token_is_transient() {
        let expired = CredentialGuard::new(
            OAuth2Token::new(SecretString::new("tok"))
                .with_expires_at(chrono::Utc::now() - chrono::Duration::seconds(1)),
        );
        let error = apply(|auth| auth.bearer(Some(&expired))).expect_err("expired");
        assert_eq!(*error.kind(), ErrorKind::Transient);
        let live = CredentialGuard::new(OAuth2Token::new(SecretString::new("tok")));
        apply(|auth| auth.bearer(Some(&live))).expect("no expiry known");
    }

    #[test]
    fn basic_and_api_key_header_shapes() {
        let pair = CredentialGuard::new(IdentityPassword::new(
            "alice",
            SecretString::new("open sesame"),
        ));
        let headers = apply(|auth| auth.basic(Some(&pair))).expect("basic");
        assert_eq!(
            headers.get(AUTHORIZATION).map(HeaderValue::as_bytes),
            Some(b"Basic YWxpY2U6b3BlbiBzZXNhbWU=".as_slice())
        );
        let key = token("key-secret");
        let headers = apply(|auth| auth.api_key_header("X-Api-Key", Some(&key))).expect("key");
        let value = headers.get("x-api-key").expect("header");
        assert!(value.is_sensitive());
        assert_eq!(value.as_bytes(), b"key-secret");
        assert!(apply(|auth| auth.api_key_header("bad name", Some(&key))).is_err());
    }
}
