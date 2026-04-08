//! Outbound webhook delivery with HMAC-SHA256 signing and retry.
//!
//! [`WebhookDeliverer`] sends POST requests to external endpoints, signing the
//! payload with HMAC-SHA256 and retrying on transient failures.
//!
//! # Example
//!
//! ```no_run
//! use nebula_webhook::deliverer::{WebhookDeliverer, WebhookEndpoint};
//!
//! # async fn run() -> Result<(), nebula_webhook::Error> {
//! let deliverer = WebhookDeliverer::new(3);
//!
//! let endpoint = WebhookEndpoint {
//!     url: "https://example.com/webhook".to_string(),
//!     secret: b"shared-secret".to_vec(),
//!     enabled: true,
//! };
//!
//! deliverer.deliver(&endpoint, b"hello world").await?;
//! # Ok(())
//! # }
//! ```

use crate::Error;
use hmac::{Hmac, Mac, digest::KeyInit};
use sha2::Sha256;
use tracing::{debug, error, warn};

/// Configuration for an outbound webhook endpoint.
#[derive(Clone)]
pub struct WebhookEndpoint {
    /// Full URL the payload will be POSTed to.
    pub url: String,

    /// Shared secret used to compute the HMAC-SHA256 signature.
    pub secret: Vec<u8>,

    /// Whether this endpoint is active.  [`WebhookDeliverer::deliver`] returns
    /// `Ok(())` immediately without making a network call when `false`.
    pub enabled: bool,
}

impl std::fmt::Debug for WebhookEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookEndpoint")
            .field("url", &self.url)
            .field("secret", &"[REDACTED]")
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// Outbound webhook delivery with HMAC signing and configurable retries.
///
/// Each call to [`deliver`](Self::deliver) signs the payload with
/// HMAC-SHA256 and attaches the signature as the `X-Nebula-Signature-256`
/// header using the `sha256=<hex>` format (compatible with GitHub webhooks).
///
/// Retries are performed with linear back-off (500 ms × attempt) on any
/// response with a 5xx status or a connection error.  4xx responses are
/// treated as permanent failures and are not retried.
#[derive(Debug)]
pub struct WebhookDeliverer {
    client: reqwest::Client,
    /// Maximum number of delivery attempts (1 = no retry).
    max_retries: u32,
}

impl WebhookDeliverer {
    /// Create a new deliverer.
    ///
    /// # Arguments
    ///
    /// * `max_retries` — total number of attempts.  Pass `1` to disable
    ///   retries, `3` for two additional retry attempts, etc.
    #[must_use]
    pub fn new(max_retries: u32) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client builder is valid"),
            max_retries: max_retries.max(1),
        }
    }

    /// Deliver `payload` to `endpoint`.
    ///
    /// Signs the raw bytes with HMAC-SHA256 and adds the
    /// `X-Nebula-Signature-256: sha256=<hex>` header.  Retries up to
    /// `max_retries` times on 5xx responses or connection errors.
    ///
    /// # Errors
    ///
    /// - [`Error::Other`] — all retry attempts are exhausted (5xx or connection
    ///   errors on every attempt).
    /// - [`Error::Config`] — the HMAC key is invalid (e.g. rejected by the
    ///   underlying digest implementation).
    /// - [`Error::Other`] — the remote returned a 4xx response (permanent
    ///   failure; not retried).
    ///
    /// Returns `Ok(())` immediately when `endpoint.enabled` is `false`.
    ///
    /// # Cancel safety
    ///
    /// This method is **not** cancel-safe: if the future is dropped while a
    /// request is in flight the retry loop terminates and the delivery may be
    /// incomplete.
    pub async fn deliver(&self, endpoint: &WebhookEndpoint, payload: &[u8]) -> Result<(), Error> {
        if !endpoint.enabled {
            debug!(url = %endpoint.url, "Endpoint disabled, skipping delivery");
            return Ok(());
        }

        let signature = sign_payload(&endpoint.secret, payload)?;
        let signature_header = format!("sha256={signature}");

        let mut last_error: Option<String> = None;

        for attempt in 1..=self.max_retries {
            debug!(
                url = %endpoint.url,
                attempt,
                max = self.max_retries,
                "Attempting outbound webhook delivery"
            );

            match self
                .client
                .post(&endpoint.url)
                .header("Content-Type", "application/json")
                .header("X-Nebula-Signature-256", &signature_header)
                .body(payload.to_vec())
                .send()
                .await
            {
                Ok(response) => {
                    let status = response.status();

                    if status.is_success() {
                        debug!(
                            url = %endpoint.url,
                            status = status.as_u16(),
                            attempt,
                            "Outbound webhook delivered successfully"
                        );
                        return Ok(());
                    }

                    // 4xx: permanent failure, do not retry
                    if status.is_client_error() {
                        let msg = format!(
                            "Delivery rejected by {}: HTTP {}",
                            endpoint.url,
                            status.as_u16()
                        );
                        error!(url = %endpoint.url, status = status.as_u16(), "{}", msg);
                        return Err(Error::other(msg));
                    }

                    // 5xx: transient, retry
                    let msg = format!(
                        "Server error from {}: HTTP {}",
                        endpoint.url,
                        status.as_u16()
                    );
                    warn!(url = %endpoint.url, status = status.as_u16(), attempt, "{}", msg);
                    last_error = Some(msg);
                }
                Err(e) => {
                    let msg = format!("Connection error to {}: {}", endpoint.url, e);
                    warn!(url = %endpoint.url, attempt, "{}", msg);
                    last_error = Some(msg);
                }
            }

            // Wait before retrying (linear back-off: 500 ms × attempt)
            if attempt < self.max_retries {
                let delay = std::time::Duration::from_millis(500 * u64::from(attempt));
                tokio::time::sleep(delay).await;
            }
        }

        let msg = last_error.unwrap_or_else(|| format!("All {} attempts failed", self.max_retries));
        error!(url = %endpoint.url, "{}", msg);
        Err(Error::other(msg))
    }
}

/// Compute HMAC-SHA256 of `payload` with `secret` and return the hex-encoded digest.
///
/// HMAC-SHA256 accepts keys of any length (including empty), so this function
/// does not reject short secrets — callers are responsible for using a
/// sufficiently strong secret.
fn sign_payload(secret: &[u8], payload: &[u8]) -> Result<String, Error> {
    // `Hmac::<Sha256>::new_from_slice` succeeds for any key length per RFC 2104.
    let mut mac = Hmac::<Sha256>::new_from_slice(secret)
        .map_err(|e| Error::config(format!("invalid HMAC key: {e}")))?;
    mac.update(payload);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

// TODO(#follow-up): Add HTTP-level delivery tests using a mock server (e.g.
// `wiremock` or `httpmock`).  These require a test server that can simulate
// 2xx, 4xx, and 5xx responses as well as connection failures, to verify the
// retry logic and back-off behaviour end-to-end.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_payload_produces_valid_hex() {
        let sig = sign_payload(b"secret", b"hello").unwrap();
        // 64 hex chars = 32 bytes = SHA-256 output
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sign_payload_deterministic() {
        let a = sign_payload(b"key", b"data").unwrap();
        let b = sign_payload(b"key", b"data").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn sign_payload_different_keys_differ() {
        let a = sign_payload(b"key1", b"data").unwrap();
        let b = sign_payload(b"key2", b"data").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn sign_payload_empty_payload_succeeds() {
        // HMAC-SHA256 accepts empty payloads (it HMACs the empty message)
        let result = sign_payload(b"key", b"");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 64);
    }

    #[tokio::test]
    async fn deliver_disabled_endpoint_returns_ok() {
        let deliverer = WebhookDeliverer::new(3);
        let endpoint = WebhookEndpoint {
            url: "http://localhost:9999/should-not-be-called".to_string(),
            secret: b"secret".to_vec(),
            enabled: false,
        };
        // Should return Ok immediately without making a network call
        assert!(deliverer.deliver(&endpoint, b"payload").await.is_ok());
    }

    #[test]
    fn deliverer_min_retries_is_one() {
        // max_retries = 0 should be clamped to 1
        let d = WebhookDeliverer::new(0);
        assert_eq!(d.max_retries, 1);
    }
}
