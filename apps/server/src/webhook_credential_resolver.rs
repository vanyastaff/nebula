//! First-party credential adapter for the API webhook bootstrap port.
//!
//! `nebula-api` owns only [`WebhookSecretResolver`]. This composition-root
//! adapter is the boundary that knows about [`CredentialService`], enforces
//! tenant-scoped binding validation, and decodes stored `whsec_` material into
//! the raw HMAC bytes consumed by webhook factories.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use nebula_api::transport::webhook::{SecretResolutionError, WebhookSecretResolver};
use nebula_credential::{
    CredentialService, CredentialServiceError, SigningKeyCredential, TenantScope,
    ValidatedCredentialBindingError,
};
use nebula_storage_port::Scope;
use tokio_util::sync::CancellationToken;

/// First-party adapter from credential runtime state to raw webhook HMAC key
/// bytes.
#[derive(Clone)]
pub(crate) struct CredentialBackedWebhookSecretResolver {
    service: Arc<CredentialService>,
}

impl CredentialBackedWebhookSecretResolver {
    #[must_use]
    pub(crate) fn new(service: Arc<CredentialService>) -> Self {
        Self { service }
    }
}

impl std::fmt::Debug for CredentialBackedWebhookSecretResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialBackedWebhookSecretResolver")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl WebhookSecretResolver for CredentialBackedWebhookSecretResolver {
    #[tracing::instrument(
        name = "webhook.credential_secret.resolve",
        skip_all,
        fields(secret_id = %secret_id)
    )]
    async fn resolve(
        &self,
        scope: &Scope,
        secret_id: &str,
    ) -> Result<Vec<u8>, SecretResolutionError> {
        let tenant_scope = TenantScope::from_scope(scope);
        let binding = self
            .service
            .validate_credential_binding(&tenant_scope, secret_id)
            .await
            .map_err(|error| {
                tracing::warn!(
                    target: "nebula::server::webhook::credential_secret",
                    error_class = "credential_binding",
                    secret_id = %secret_id,
                    "webhook credential binding validation failed"
                );
                map_binding_error(error)
            })?;

        let guard = self
            .service
            .resolve_for_slot::<SigningKeyCredential>(
                &tenant_scope,
                &binding,
                CancellationToken::new(),
            )
            .await
            .map_err(|error| {
                tracing::warn!(
                    target: "nebula::server::webhook::credential_secret",
                    error_class = "credential_resolution",
                    secret_id = %secret_id,
                    "webhook credential material resolution failed"
                );
                map_credential_error(error)
            })?;

        let decoded = decode_whsec(guard.key().expose_secret())?;

        tracing::debug!(
            target: "nebula::server::webhook::credential_secret",
            key_len = decoded.len(),
            "webhook signing secret resolved"
        );
        Ok(decoded)
    }
}

fn map_binding_error(error: ValidatedCredentialBindingError) -> SecretResolutionError {
    match error {
        ValidatedCredentialBindingError::NotFound { .. }
        | ValidatedCredentialBindingError::ScopeMismatch { .. } => SecretResolutionError::NotFound,
        ValidatedCredentialBindingError::CredentialTombstoned { .. }
        | ValidatedCredentialBindingError::Io(_) => SecretResolutionError::Unavailable,
        _ => SecretResolutionError::Unavailable,
    }
}

fn map_credential_error(_error: CredentialServiceError) -> SecretResolutionError {
    SecretResolutionError::Unavailable
}

fn decode_whsec(value: &str) -> Result<Vec<u8>, SecretResolutionError> {
    let encoded = value
        .strip_prefix("whsec_")
        .ok_or(SecretResolutionError::InvalidMaterial)?;
    let decoded = BASE64_STANDARD
        .decode(encoded)
        .map_err(|_| SecretResolutionError::InvalidMaterial)?;
    if decoded.is_empty() {
        return Err(SecretResolutionError::InvalidMaterial);
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, Method};
    use nebula_action::{
        MockClock, RequiredPolicy, SignatureError, SignatureScheme, WebhookRequest,
        hmac_sha256_compute,
    };
    use nebula_credential::CredentialDisplay;
    use nebula_storage::credential::{EnvKeyProvider, KeyProvider};
    use serde_json::json;

    const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";
    const SECRET_CANARY: &str = "whsec_DYNAMIC_PROVIDER_CANARY";
    const FIXED_TIMESTAMP: u64 = 1_700_000_000;

    async fn service() -> Arc<CredentialService> {
        let key: Arc<dyn KeyProvider> =
            Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid test encryption key"));
        crate::credential_composition::compose_memory_service(key)
            .await
            .expect("test credential service composes")
    }

    fn signed_request(webhook_id: &str, body: &[u8], secret: &[u8]) -> WebhookRequest {
        let timestamp = FIXED_TIMESTAMP.to_string();
        let mut signed = Vec::with_capacity(webhook_id.len() + timestamp.len() + body.len() + 2);
        signed.extend_from_slice(webhook_id.as_bytes());
        signed.push(b'.');
        signed.extend_from_slice(timestamp.as_bytes());
        signed.push(b'.');
        signed.extend_from_slice(body);

        let signature = format!(
            "v1,{}",
            BASE64_STANDARD.encode(hmac_sha256_compute(secret, &signed)),
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            "webhook-id",
            HeaderValue::from_str(webhook_id).expect("test webhook id is a valid header"),
        );
        headers.insert(
            "webhook-timestamp",
            HeaderValue::from_str(&timestamp).expect("test timestamp is a valid header"),
        );
        headers.insert(
            "webhook-signature",
            HeaderValue::from_str(&signature).expect("test signature is a valid header"),
        );

        WebhookRequest::try_new(
            Method::POST,
            "/webhook/test-path",
            None::<String>,
            headers,
            body.to_vec(),
        )
        .expect("test request satisfies webhook limits")
    }

    #[test]
    fn decode_accepts_standard_base64_and_rejects_other_formats() {
        let raw = [0x42_u8; 32];
        let encoded = format!("whsec_{}", BASE64_STANDARD.encode(raw));
        assert_eq!(decode_whsec(&encoded).expect("known vector decodes"), raw);

        assert!(matches!(
            decode_whsec("not-prefixed"),
            Err(SecretResolutionError::InvalidMaterial)
        ));
        assert!(matches!(
            decode_whsec("whsec_abc-def_ghi"),
            Err(SecretResolutionError::InvalidMaterial)
        ));
        assert!(matches!(
            decode_whsec("whsec_"),
            Err(SecretResolutionError::InvalidMaterial)
        ));
    }

    #[test]
    fn dynamic_credential_errors_collapse_to_fixed_text() {
        let error =
            map_credential_error(CredentialServiceError::Internal(SECRET_CANARY.to_owned()));
        let display = error.to_string();

        assert_eq!(error, SecretResolutionError::Unavailable);
        assert_eq!(display, "webhook signing secret is unavailable");
        assert!(!display.contains(SECRET_CANARY));
    }

    #[tokio::test]
    async fn resolver_round_trips_stored_signing_material() {
        let service = service().await;
        let scope = Scope::new("ws_round_trip", "org_round_trip");
        let tenant = TenantScope::from_scope(&scope);
        let expected = [0x5a_u8; 32];
        let stored = format!("whsec_{}", BASE64_STANDARD.encode(expected));
        let head = service
            .create(
                &tenant,
                "signing_key",
                json!({ "key": stored, "algorithm": "hmac-sha256" }),
                CredentialDisplay::default(),
            )
            .await
            .expect("signing credential is created");

        let resolver = CredentialBackedWebhookSecretResolver::new(service);
        let actual = resolver
            .resolve(&scope, &head.id)
            .await
            .expect("owner-scoped credential resolves");

        assert_eq!(actual, expected);

        let body = br#"{"event":"test.delivered"}"#;
        let webhook_id = "msg-round-trip-001";
        let request = signed_request(webhook_id, body, &actual);
        let clock = MockClock::at_unix_secs(FIXED_TIMESTAMP);
        RequiredPolicy::new()
            .with_secret(actual.clone())
            .with_scheme(SignatureScheme::StandardWebhooks)
            .verify_with(&request, &clock)
            .expect("resolved bytes verify a Standard Webhooks signature");

        let wrong_key = [0xff_u8; 32];
        let wrong_request = signed_request(webhook_id, body, &wrong_key);
        let result = RequiredPolicy::new()
            .with_secret(actual)
            .with_scheme(SignatureScheme::StandardWebhooks)
            .verify_with(&wrong_request, &clock);
        assert!(matches!(result, Err(SignatureError::SignatureInvalid)));
    }

    #[tokio::test]
    async fn resolver_rejects_cross_tenant_credential_without_leaking_material() {
        let service = service().await;
        let owner_scope = Scope::new("ws_owner", "org_owner");
        let owner = TenantScope::from_scope(&owner_scope);
        let stored = format!("whsec_{}", BASE64_STANDARD.encode([0x6b_u8; 32]));
        let head = service
            .create(
                &owner,
                "signing_key",
                json!({ "key": stored, "algorithm": "hmac-sha256" }),
                CredentialDisplay::default(),
            )
            .await
            .expect("owner credential is created");

        let resolver = CredentialBackedWebhookSecretResolver::new(service);
        let error = resolver
            .resolve(&Scope::new("ws_other", "org_other"), &head.id)
            .await
            .expect_err("another tenant cannot resolve the credential");
        let display = error.to_string();

        assert_eq!(error, SecretResolutionError::NotFound);
        assert_eq!(display, "webhook signing secret was not found");
        assert!(!display.contains("org_owner"));
        assert!(!display.contains("ws_owner"));
        assert!(!display.contains("whsec_"));
    }
}
