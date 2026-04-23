//! OAuth state signing and verification — CSRF protection for OAuth2 flows.
//!
//! Uses HMAC-SHA256 to sign a JSON payload containing the credential ID,
//! CSRF token, and expiry timestamp. The signed state is sent as the `state`
//! parameter in OAuth2 authorization requests, binding each authorization
//! attempt to a specific credential and preventing cross-credential
//! state confusion attacks.
//!
//! See ADR-0031 for the OAuth flow ownership decision.

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac, digest::KeyInit};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::errors::ApiError;

type HmacSha256 = Hmac<Sha256>;

/// HMAC-SHA256 produces a 32-byte (256-bit) message authentication code.
const HMAC_SHA256_SIGNATURE_BYTES: usize = 32;

/// Signed OAuth state payload bound to a credential and expiry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedOAuthState {
    /// CSRF nonce.
    pub csrf_token: String,
    /// Credential identifier from request path.
    pub credential_id: String,
    /// Expiration timestamp (UTC).
    pub expires_at: DateTime<Utc>,
}

/// Verification errors for signed OAuth state values.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// Encoded state is malformed.
    #[error("malformed oauth state")]
    Malformed,
    /// State signature verification failed.
    #[error("oauth state signature mismatch")]
    SignatureMismatch,
    /// State has expired.
    #[error("oauth state expired")]
    Expired,
    /// State was issued for another credential.
    #[error("oauth state credential mismatch")]
    CredentialMismatch,
}

impl From<StateError> for ApiError {
    fn from(err: StateError) -> Self {
        match err {
            StateError::Malformed
            | StateError::SignatureMismatch
            | StateError::CredentialMismatch
            | StateError::Expired => {
                ApiError::Unauthorized(format!("OAuth state validation failed: {err}"))
            },
        }
    }
}

/// HMAC-backed signer/verifier for OAuth state blobs.
pub struct OAuthStateSigner {
    secret: Vec<u8>,
}

impl OAuthStateSigner {
    /// Construct a signer from secret bytes.
    #[must_use]
    pub fn new(secret: &[u8]) -> Self {
        Self {
            secret: secret.to_vec(),
        }
    }

    /// Sign state payload and return URL-safe opaque token.
    pub fn sign(&self, payload: &SignedOAuthState) -> Result<String, StateError> {
        let encoded_payload = serde_json::to_vec(payload).map_err(|_| StateError::Malformed)?;
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).map_err(|_| StateError::Malformed)?;
        mac.update(&encoded_payload);
        let signature = mac.finalize().into_bytes();

        let mut blob = Vec::with_capacity(signature.len() + encoded_payload.len());
        blob.extend_from_slice(&signature);
        blob.extend_from_slice(&encoded_payload);
        Ok(URL_SAFE_NO_PAD.encode(blob))
    }

    /// Verify state blob, check expiry, and enforce credential binding.
    pub fn verify_for_credential(
        &self,
        encoded: &str,
        expected_credential_id: &str,
    ) -> Result<SignedOAuthState, StateError> {
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| StateError::Malformed)?;
        if decoded.len() <= HMAC_SHA256_SIGNATURE_BYTES {
            return Err(StateError::Malformed);
        }

        let (signature, payload_bytes) = decoded.split_at(HMAC_SHA256_SIGNATURE_BYTES);
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).map_err(|_| StateError::Malformed)?;
        mac.update(payload_bytes);
        mac.verify_slice(signature)
            .map_err(|_| StateError::SignatureMismatch)?;

        let payload: SignedOAuthState =
            serde_json::from_slice(payload_bytes).map_err(|_| StateError::Malformed)?;

        if payload.expires_at < Utc::now() {
            return Err(StateError::Expired);
        }
        if payload.credential_id != expected_credential_id {
            return Err(StateError::CredentialMismatch);
        }
        Ok(payload)
    }
}

/// Build a new signed OAuth state with default max TTL (10 minutes).
pub fn build_signed_state(
    signer: &OAuthStateSigner,
    credential_id: &str,
    csrf_token: String,
) -> Result<(String, SignedOAuthState), StateError> {
    let payload = SignedOAuthState {
        csrf_token,
        credential_id: credential_id.to_owned(),
        expires_at: Utc::now()
            + chrono::Duration::from_std(Duration::from_mins(10)).unwrap_or_default(),
    };
    let encoded = signer.sign(&payload)?;
    Ok((encoded, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer() -> OAuthStateSigner {
        OAuthStateSigner::new(b"test-oauth-state-secret-32-bytes-min")
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let signer = signer();
        let (encoded, payload) = build_signed_state(&signer, "cred_01", "csrf_01".to_owned())
            .expect("state should be signed");

        let verified = signer
            .verify_for_credential(&encoded, "cred_01")
            .expect("state should verify");
        assert_eq!(verified.csrf_token, payload.csrf_token);
        assert_eq!(verified.credential_id, payload.credential_id);
    }

    #[test]
    fn verify_rejects_wrong_credential() {
        let signer = signer();
        let (encoded, _) = build_signed_state(&signer, "cred_01", "csrf_01".to_owned())
            .expect("state should be signed");

        let err = signer
            .verify_for_credential(&encoded, "cred_02")
            .expect_err("credential mismatch must fail");
        assert!(matches!(err, StateError::CredentialMismatch));
    }
}
