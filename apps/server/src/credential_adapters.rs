//! Concrete first-party credential adapters owned by the server process.
//!
//! These types implement lower-layer runtime and API read-model ports without
//! entering `nebula-api`'s default public surface.

use std::{future::Future, sync::Arc};

use futures::StreamExt as _;
use nebula_api::ports::credential_schema::{
    CredentialCapabilityFlags, CredentialSchemaPort, CredentialTypeDescriptor,
};
use nebula_credential::{
    AnyCredential, Capabilities, CredentialRegistry,
    runtime::{RefreshTransport, RefreshTransportError, TokenPostRequest, TokenPostResponse},
};
use nebula_schema::ValidSchema;
use nebula_storage_port::SecretBytes;
use zeroize::Zeroizing;

use crate::oauth_egress::build_oauth_client;

/// Reqwest-backed token refresh transport for the first-party process.
#[derive(Clone)]
pub(crate) struct ReqwestRefreshTransport {
    client: reqwest::Client,
}

impl ReqwestRefreshTransport {
    /// Build the process-wide policy-bearing client.
    pub(crate) fn new() -> Result<Self, reqwest::Error> {
        build_oauth_client().map(|client| Self { client })
    }

    #[cfg(test)]
    fn for_test(
        trust_anchor: reqwest::Certificate,
        connect_ip: std::net::IpAddr,
        dns_answers: Vec<std::net::IpAddr>,
    ) -> Result<Self, reqwest::Error> {
        crate::oauth_egress::build_test_oauth_client(trust_anchor, connect_ip, dns_answers)
            .map(|client| Self { client })
    }
}

impl std::fmt::Debug for ReqwestRefreshTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ReqwestRefreshTransport")
    }
}

impl RefreshTransport for ReqwestRefreshTransport {
    fn post_token<'a>(
        &'a self,
        request: TokenPostRequest,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<TokenPostResponse, RefreshTransportError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let form_pairs: Vec<(&str, &str)> = request
                .form()
                .iter()
                .map(|(key, value)| (key.as_str(), value.expose_secret()))
                .collect();
            let max_response_bytes = request.max_response_bytes();
            let mut builder = self
                .client
                .post(request.endpoint().expose_url().clone())
                .form(&form_pairs);
            if let Some((user, password)) = request.basic_auth() {
                builder = builder.basic_auth(user.expose_secret(), Some(password.expose_secret()));
            }
            drop(form_pairs);
            drop(request);

            let response = builder
                .send()
                .await
                .map_err(|_| RefreshTransportError::Send)?;
            let status = response.status().as_u16();
            let body = read_bounded(response, max_response_bytes)
                .await
                .map_err(|_| RefreshTransportError::ReadBody)?;
            TokenPostResponse::try_new(status, body).map_err(|_| RefreshTransportError::ReadBody)
        })
    }
}

async fn read_bounded(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<SecretBytes, ReadBoundedError> {
    if let Some(claimed) = response.content_length() {
        let max = u64::try_from(max_bytes).unwrap_or(u64::MAX);
        if claimed > max {
            return Err(ReadBoundedError::ContentLengthTooLarge {
                claimed,
                max: max_bytes,
            });
        }
    }

    let mut body = Zeroizing::new(Vec::new());
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(ReadBoundedError::Read)?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ReadBoundedError::BodyTooLarge { max: max_bytes });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(SecretBytes::from(body))
}

#[derive(Debug, thiserror::Error)]
enum ReadBoundedError {
    #[error("token response too large: Content-Length {claimed} (max {max} bytes)")]
    ContentLengthTooLarge { claimed: u64, max: usize },
    #[error("token response body exceeded {max} bytes")]
    BodyTooLarge { max: usize },
    #[error("read token response body: {0}")]
    Read(#[source] reqwest::Error),
}

/// Catalog projection over the exact registry used by the runtime.
pub(crate) struct RegistryCredentialSchema {
    registry: Arc<CredentialRegistry>,
}

impl RegistryCredentialSchema {
    pub(crate) fn new(registry: Arc<CredentialRegistry>) -> Self {
        Self { registry }
    }

    fn descriptor(&self, credential: &dyn AnyCredential) -> CredentialTypeDescriptor {
        let metadata = credential.metadata();
        let key = credential.credential_key().to_owned();
        let capabilities = self
            .registry
            .capabilities_of(&key)
            .unwrap_or_else(Capabilities::empty);
        CredentialTypeDescriptor {
            key,
            name: metadata.base.name.clone(),
            description: metadata.base.description.clone(),
            auth_pattern: format!("{:?}", metadata.pattern),
            capabilities: CredentialCapabilityFlags {
                interactive: capabilities.contains(Capabilities::INTERACTIVE),
                refreshable: capabilities.contains(Capabilities::REFRESHABLE),
                testable: capabilities.contains(Capabilities::TESTABLE),
                revocable: capabilities.contains(Capabilities::REVOCABLE),
            },
            icon: metadata.base.icon.as_inline().map(str::to_owned),
            documentation_url: metadata.base.documentation_url,
            schema_json: export_schema(&metadata.base.schema),
        }
    }
}

fn export_schema(schema: &ValidSchema) -> serde_json::Value {
    schema
        .json_schema()
        .ok()
        .and_then(|exported| serde_json::to_value(&exported).ok())
        .unwrap_or_else(|| serde_json::json!({ "type": "object" }))
}

impl CredentialSchemaPort for RegistryCredentialSchema {
    fn list_types(&self) -> Vec<CredentialTypeDescriptor> {
        self.registry
            .iter_compatible(Capabilities::empty())
            .filter_map(|(key, _)| {
                self.registry
                    .resolve_any(key)
                    .map(|credential| self.descriptor(credential))
            })
            .collect()
    }

    fn get_type(&self, credential_key: &str) -> Option<CredentialTypeDescriptor> {
        self.registry
            .resolve_any(credential_key)
            .map(|credential| self.descriptor(credential))
    }
}

#[cfg(test)]
#[path = "credential_adapters_tests.rs"]
mod transport_security_tests;
