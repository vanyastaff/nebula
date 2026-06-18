//! Production [`WebhookSecretResolver`] backed by [`CredentialService`], plus
//! the [`mint_whsec`] factory for fresh signing secrets.
//!
//! [`CredentialBackedWebhookSecretResolver`] converts a `signing_key` credential
//! stored with a `whsec_<base64>` key string into the raw HMAC key bytes that
//! the Standard Webhooks verifier consumes — stripping the `whsec_` prefix and
//! base64-decoding the remainder.
//!
//! [`mint_whsec`] generates a fresh CSPRNG-backed secret in that format,
//! suitable for creating a new `signing_key` credential via
//! [`nebula_credential::CredentialService::create`].
//!
//! # Tenant isolation
//!
//! Resolution goes through
//! [`CredentialService::validate_credential_binding`] →
//! [`CredentialService::resolve_for_slot`], which enforces the owner check at
//! both the binding-validation step (cross-tenant id → typed error rather than
//! `NotFound`) and the guard-acquire step (fingerprint re-check in depth).
//! There is no raw-resolve-by-id path.
//!
//! # Secret discipline
//!
//! The `whsec_` string and the decoded key bytes never appear in tracing
//! fields or error messages.  Typed errors on decode failure name the
//! structural problem (missing prefix, invalid base64, empty result) without
//! echoing the material.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use nebula_credential::{
    CredentialService, SigningKeyCredential, TenantScope, ValidatedCredentialBindingError,
};
use nebula_storage_port::Scope;
use rand::Rng as _;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use super::bootstrap::{SecretResolutionError, WebhookSecretResolver};

// ── Typed error ───────────────────────────────────────────────────────────────

/// Errors that can occur inside [`CredentialBackedWebhookSecretResolver::resolve`].
///
/// Every failure out of [`resolve`](WebhookSecretResolver::resolve) is a
/// `ResolverError` boxed into [`SecretResolutionError`].  The messages
/// intentionally contain NO secret material — only structural descriptions.
#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum ResolverError {
    /// The binding-validation step rejected the credential id (not found,
    /// cross-tenant, or tombstoned).
    #[error("credential binding validation failed: {0}")]
    Binding(#[from] ValidatedCredentialBindingError),
    /// The credential guard-acquire or decryption step failed.
    #[error("credential resolution failed: {0}")]
    Credential(#[from] nebula_credential::CredentialServiceError),
    /// The stored key value does not start with the expected `whsec_` prefix.
    #[error("signing key does not carry the required 'whsec_' prefix")]
    MissingWhsecPrefix,
    /// The base64 portion after the `whsec_` prefix is not valid standard
    /// base64.
    #[error("signing key base64 payload is not valid base64")]
    InvalidBase64(#[from] base64::DecodeError),
    /// The decoded key is zero-length; the verifier would build a fail-closed
    /// handler.  Reject before handing back to the bootstrap.
    #[error("signing key decoded to zero bytes; a non-empty key is required")]
    EmptyKey,
}

// ── Prod resolver ─────────────────────────────────────────────────────────────

/// Production [`WebhookSecretResolver`] that resolves a `signing_key`
/// credential from [`CredentialService`] and decodes its `whsec_<base64>` key
/// to the raw HMAC bytes consumed by the Standard Webhooks verifier.
///
/// Construction is cheap — just an `Arc` clone of the service.
#[derive(Clone)]
pub struct CredentialBackedWebhookSecretResolver {
    service: Arc<CredentialService>,
}

impl CredentialBackedWebhookSecretResolver {
    /// Wrap an existing `CredentialService`.
    #[must_use]
    pub fn new(service: Arc<CredentialService>) -> Self {
        Self { service }
    }
}

impl std::fmt::Debug for CredentialBackedWebhookSecretResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialBackedWebhookSecretResolver")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl WebhookSecretResolver for CredentialBackedWebhookSecretResolver {
    #[tracing::instrument(
        name = "webhook.secret_resolver.resolve",
        skip_all,
        fields(secret_id = %secret_id)
    )]
    async fn resolve(
        &self,
        scope: &Scope,
        secret_id: &str,
    ) -> Result<Vec<u8>, SecretResolutionError> {
        let tenant_scope = TenantScope::from_scope(scope);

        // Step 1: validate the binding (owner check, tombstone check).
        // All three failure modes (NotFound / ScopeMismatch / CredentialTombstoned)
        // are routed through `ResolverError::Binding` so every failure out of
        // this function is a `ResolverError` behind the box.
        let binding = self
            .service
            .validate_credential_binding(&tenant_scope, secret_id)
            .await
            .map_err(|e| {
                tracing::warn!(
                    target: "nebula::api::webhook::secret_resolver",
                    error = %e,
                    "credential binding validation failed for webhook secret"
                );
                Box::new(ResolverError::Binding(e)) as SecretResolutionError
            })?;

        // Step 2: acquire the typed guard (decrypt + cache hit).
        let guard = self
            .service
            .resolve_for_slot::<SigningKeyCredential>(
                &tenant_scope,
                &binding,
                // Unlinked token is intentional: bootstrap and reload callers
                // have no cancellation context, and resolution is a bounded
                // store-load + decrypt (no unbounded wait).
                CancellationToken::new(),
            )
            .await
            .map_err(|e| {
                tracing::warn!(
                    target: "nebula::api::webhook::secret_resolver",
                    error = %e,
                    "credential slot resolution failed for webhook secret"
                );
                Box::new(ResolverError::Credential(e)) as SecretResolutionError
            })?;

        // Step 3: expose the raw string and decode the whsec_ payload.
        // Intentionally NOT tracing the secret value.
        let raw = guard.key().expose_secret();
        let raw_bytes = decode_whsec(raw).map_err(|e| Box::new(e) as SecretResolutionError)?;

        tracing::debug!(
            target: "nebula::api::webhook::secret_resolver",
            key_len = raw_bytes.len(),
            "webhook signing secret resolved and decoded"
        );

        Ok(raw_bytes)
    }
}

// ── Secret minting ────────────────────────────────────────────────────────────

/// Mint a fresh Standard-Webhooks signing secret.
///
/// Returns a `whsec_<base64>` string suitable for use as the `key` field
/// when creating a `signing_key` credential.  The prefix follows the
/// [Standard Webhooks](https://www.standardwebhooks.com/) convention;
/// [`CredentialBackedWebhookSecretResolver`] strips the prefix and
/// base64-decodes the remainder to produce the raw HMAC key bytes consumed
/// by the verifier.
///
/// # Entropy
///
/// Uses 32 bytes from the OS CSPRNG (`rand::rng()`).  32 bytes = 256
/// bits of entropy, well above the HMAC-SHA256 key-length recommendation.
#[must_use]
pub fn mint_whsec() -> String {
    let mut raw = [0u8; 32];
    rand::rng().fill_bytes(&mut raw);
    format!("whsec_{}", BASE64_STANDARD.encode(raw))
}

// ── Decode helper ─────────────────────────────────────────────────────────────

/// Decode a `whsec_<base64>` string to the raw key bytes.
///
/// Strips the `whsec_` prefix, base64-decodes the remainder using
/// **standard** (not URL-safe) base64, and rejects an empty result.
/// Errors carry no secret material.
///
/// # Errors
///
/// - [`ResolverError::MissingWhsecPrefix`] when the string lacks the prefix.
/// - [`ResolverError::InvalidBase64`] when the payload is malformed base64,
///   including URL-safe base64 characters (`-` or `_`) not present in the
///   standard alphabet.
/// - [`ResolverError::EmptyKey`] when the decoded bytes are empty.
pub(crate) fn decode_whsec(s: &str) -> Result<Vec<u8>, ResolverError> {
    let b64 = s
        .strip_prefix("whsec_")
        .ok_or(ResolverError::MissingWhsecPrefix)?;
    let bytes = BASE64_STANDARD.decode(b64)?;
    if bytes.is_empty() {
        return Err(ResolverError::EmptyKey);
    }
    Ok(bytes)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── mint_whsec ────────────────────────────────────────────────────────────

    /// `mint_whsec` returns a `whsec_`-prefixed string that is base64-decodable
    /// to exactly 32 bytes.
    #[test]
    fn mint_whsec_prefix_and_length() {
        let secret = mint_whsec();
        assert!(
            secret.starts_with("whsec_"),
            "mint_whsec must start with 'whsec_'; got {secret:?}"
        );
        let b64_part = secret.strip_prefix("whsec_").unwrap();
        let decoded = BASE64_STANDARD
            .decode(b64_part)
            .expect("base64 portion must be valid standard base64");
        assert_eq!(decoded.len(), 32, "decoded key must be exactly 32 bytes");
    }

    /// Two consecutive calls produce different values (entropy).
    #[test]
    fn mint_whsec_is_random() {
        let a = mint_whsec();
        let b = mint_whsec();
        assert_ne!(
            a, b,
            "consecutive mint_whsec calls must produce distinct values"
        );
    }

    // ── decode_whsec ──────────────────────────────────────────────────────────

    #[test]
    fn decode_whsec_round_trips_known_vector() {
        // Standard Webhooks example key: 32 zero bytes base64-encoded.
        let raw = [0u8; 32];
        let encoded = BASE64_STANDARD.encode(raw);
        let whsec = format!("whsec_{encoded}");
        let decoded = decode_whsec(&whsec).expect("known-vector must decode");
        assert_eq!(decoded, raw, "decoded bytes must match original");
    }

    #[test]
    fn decode_whsec_rejects_missing_prefix() {
        let err = decode_whsec("AAAA").expect_err("no prefix must fail");
        assert!(
            matches!(err, ResolverError::MissingWhsecPrefix),
            "expected MissingWhsecPrefix, got {err:?}"
        );
    }

    #[test]
    fn decode_whsec_rejects_invalid_base64() {
        let err = decode_whsec("whsec_!!!notbase64").expect_err("bad base64 must fail");
        assert!(
            matches!(err, ResolverError::InvalidBase64(_)),
            "expected InvalidBase64, got {err:?}"
        );
    }

    #[test]
    fn decode_whsec_rejects_empty_payload() {
        // base64("") == "" — produces zero bytes after decoding.
        let err = decode_whsec("whsec_").expect_err("empty payload must fail");
        assert!(
            matches!(err, ResolverError::EmptyKey),
            "expected EmptyKey, got {err:?}"
        );
    }

    #[test]
    fn decode_whsec_rejects_empty_bytes_after_decode() {
        // Explicit: the empty string encodes to "" in standard base64.
        let encoded = BASE64_STANDARD.encode(b"");
        let whsec = format!("whsec_{encoded}");
        let err = decode_whsec(&whsec).expect_err("zero-length decoded bytes must fail");
        assert!(
            matches!(err, ResolverError::EmptyKey),
            "expected EmptyKey, got {err:?}"
        );
    }

    /// A `whsec_` payload encoded with URL-safe base64 (`-` / `_` chars) must
    /// fail with `InvalidBase64`, not silently produce wrong bytes.
    ///
    /// `mint_whsec` always emits standard base64 (`+` / `/`); this test confirms
    /// the decoder rejects the URL-safe variant a caller might accidentally
    /// produce from a different base64 library.
    #[test]
    fn decode_whsec_rejects_url_safe_base64() {
        // Craft a payload that contains URL-safe characters not in the standard
        // alphabet.  A 3-byte sequence encodes to base64 containing characters
        // from the full 64-char set; we inject '-' and '_' explicitly.
        let url_safe_payload = "whsec_abc-def_ghi";
        let err = decode_whsec(url_safe_payload)
            .expect_err("URL-safe base64 chars must be rejected by the standard decoder");
        assert!(
            matches!(err, ResolverError::InvalidBase64(_)),
            "expected InvalidBase64 for URL-safe base64 input, got {err:?}"
        );
    }
}
