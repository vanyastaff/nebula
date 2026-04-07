//! Framework-level webhook signature verification.
//!
//! Verifiers run **before** the payload is dispatched to triggers,
//! rejecting unauthenticated requests at the HTTP layer.

use async_trait::async_trait;
use axum::http::HeaderMap;
use bytes::Bytes;

use crate::error::Error;

/// Framework-level webhook signature verifier.
///
/// Implementations verify that incoming webhook requests are authentic
/// before they reach the trigger action.
///
/// # Examples
///
/// ```
/// use nebula_webhook::verifier::{WebhookVerifier, HmacSha256Verifier};
///
/// let verifier = HmacSha256Verifier::new(b"secret", "X-Hub-Signature-256")
///     .with_prefix("sha256=");
/// ```
#[async_trait]
pub trait WebhookVerifier: Send + Sync {
    /// Verify the request is authentic.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SignatureInvalid`] if verification fails.
    async fn verify(&self, headers: &HeaderMap, body: &Bytes) -> Result<(), Error>;
}

/// HMAC-SHA256 signature verifier (Stripe, GitHub, Slack pattern).
///
/// Computes HMAC-SHA256 of the body using the shared secret and compares
/// against the signature in the specified header using constant-time
/// comparison.
///
/// # Examples
///
/// ```
/// use nebula_webhook::HmacSha256Verifier;
///
/// // GitHub-style: "sha256=<hex>"
/// let verifier = HmacSha256Verifier::new(b"webhook-secret", "X-Hub-Signature-256")
///     .with_prefix("sha256=");
///
/// // Stripe-style: bare hex in custom header
/// let verifier = HmacSha256Verifier::new(b"whsec_xxx", "Stripe-Signature");
/// ```
pub struct HmacSha256Verifier {
    /// The shared secret key.
    secret: Vec<u8>,
    /// Header name containing the signature (e.g., "X-Hub-Signature-256").
    header_name: String,
    /// Optional prefix to strip from header value (e.g., "sha256=").
    prefix: Option<String>,
}

impl std::fmt::Debug for HmacSha256Verifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HmacSha256Verifier")
            .field("secret", &"<redacted>")
            .field("header_name", &self.header_name)
            .field("prefix", &self.prefix)
            .finish()
    }
}

impl HmacSha256Verifier {
    /// Create a new HMAC-SHA256 verifier.
    ///
    /// # Arguments
    ///
    /// * `secret` - The shared secret key for HMAC computation.
    /// * `header_name` - The HTTP header containing the signature.
    #[must_use]
    pub fn new(secret: impl Into<Vec<u8>>, header_name: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            header_name: header_name.into(),
            prefix: None,
        }
    }

    /// Set a prefix to strip from the header value before comparing.
    ///
    /// Many providers prepend a scheme identifier (e.g., `sha256=`) to
    /// the hex-encoded signature. This method configures the verifier
    /// to strip that prefix before decoding.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }
}

#[async_trait]
impl WebhookVerifier for HmacSha256Verifier {
    async fn verify(&self, headers: &HeaderMap, body: &Bytes) -> Result<(), Error> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let sig_header = headers
            .get(&self.header_name)
            .ok_or_else(|| {
                Error::signature_invalid(format!("missing header: {}", self.header_name))
            })?
            .to_str()
            .map_err(|_| Error::signature_invalid("signature header is not valid UTF-8"))?;

        // Strip prefix if configured
        let sig_hex = match &self.prefix {
            Some(prefix) => sig_header
                .strip_prefix(prefix.as_str())
                .unwrap_or(sig_header),
            None => sig_header,
        };

        // Decode hex signature
        let expected = hex::decode(sig_hex)
            .map_err(|_| Error::signature_invalid("signature is not valid hex"))?;

        // Compute HMAC
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret)
            .map_err(|_| Error::signature_invalid("invalid HMAC key length"))?;
        mac.update(body);

        // Constant-time comparison
        mac.verify_slice(&expected)
            .map_err(|_| Error::signature_invalid("signature mismatch"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hmac_sha256_verifies_valid_signature() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let secret = b"test-secret";
        let body = Bytes::from("hello world");

        // Compute expected signature
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(&body);
        let sig = hex::encode(mac.finalize().into_bytes());

        let verifier = HmacSha256Verifier::new(secret.to_vec(), "X-Signature");
        let mut headers = HeaderMap::new();
        headers.insert("X-Signature", sig.parse().unwrap());

        assert!(verifier.verify(&headers, &body).await.is_ok());
    }

    #[tokio::test]
    async fn hmac_sha256_rejects_invalid_signature() {
        let verifier = HmacSha256Verifier::new(b"secret".to_vec(), "X-Signature");
        let mut headers = HeaderMap::new();
        headers.insert("X-Signature", "deadbeef".parse().unwrap());
        let body = Bytes::from("hello");

        let result = verifier.verify(&headers, &body).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::SignatureInvalid
        ));
    }

    #[tokio::test]
    async fn hmac_sha256_rejects_missing_header() {
        let verifier = HmacSha256Verifier::new(b"secret".to_vec(), "X-Signature");
        let headers = HeaderMap::new();
        let body = Bytes::from("hello");

        let result = verifier.verify(&headers, &body).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::SignatureInvalid
        ));
    }

    #[tokio::test]
    async fn hmac_sha256_strips_prefix() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let secret = b"secret";
        let body = Bytes::from("data");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(&body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let verifier =
            HmacSha256Verifier::new(secret.to_vec(), "X-Hub-Signature-256").with_prefix("sha256=");
        let mut headers = HeaderMap::new();
        headers.insert("X-Hub-Signature-256", sig.parse().unwrap());

        assert!(verifier.verify(&headers, &body).await.is_ok());
    }

    #[tokio::test]
    async fn hmac_sha256_rejects_invalid_hex() {
        let verifier = HmacSha256Verifier::new(b"secret".to_vec(), "X-Signature");
        let mut headers = HeaderMap::new();
        headers.insert("X-Signature", "not-hex-zzzz".parse().unwrap());
        let body = Bytes::from("hello");

        let result = verifier.verify(&headers, &body).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn hmac_sha256_prefix_mismatch_still_attempts_decode() {
        // If prefix doesn't match, the full header value is used as-is
        let verifier =
            HmacSha256Verifier::new(b"secret".to_vec(), "X-Signature").with_prefix("sha512=");
        let mut headers = HeaderMap::new();
        // Value doesn't start with "sha512=", so full value is used
        headers.insert("X-Signature", "sha256=abcd".parse().unwrap());
        let body = Bytes::from("hello");

        // Will fail because "sha256=abcd" is not valid hex
        let result = verifier.verify(&headers, &body).await;
        assert!(result.is_err());
    }
}
