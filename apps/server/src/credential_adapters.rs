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
    Capabilities, CredentialRegistry,
    runtime::{
        AcquisitionTransport, AcquisitionTransportError, RefreshTransport, RefreshTransportError,
        TokenPostRequest, TokenPostResponse,
    },
};
use nebula_schema::JsonSchemaExportError;
use nebula_storage_port::SecretBytes;
use zeroize::Zeroizing;

use crate::oauth_egress::build_oauth_client;

/// Reqwest-backed OAuth token transport for the first-party process.
#[derive(Clone)]
pub(crate) struct ReqwestOAuthTransport {
    client: reqwest::Client,
}

impl ReqwestOAuthTransport {
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

impl std::fmt::Debug for ReqwestOAuthTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ReqwestOAuthTransport")
    }
}

impl RefreshTransport for ReqwestOAuthTransport {
    fn post_token<'a>(
        &'a self,
        request: TokenPostRequest,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<TokenPostResponse, RefreshTransportError>> + Send + 'a>,
    > {
        Box::pin(async move {
            post_token(&self.client, request)
                .await
                .map_err(|error| match error {
                    OAuthHttpError::Send => RefreshTransportError::Send,
                    OAuthHttpError::ReadBody => RefreshTransportError::ReadBody,
                })
        })
    }
}

impl AcquisitionTransport for ReqwestOAuthTransport {
    fn post_token<'a>(
        &'a self,
        request: TokenPostRequest,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<TokenPostResponse, AcquisitionTransportError>> + Send + 'a>,
    > {
        Box::pin(async move {
            post_token(&self.client, request)
                .await
                .map_err(|error| match error {
                    OAuthHttpError::Send => AcquisitionTransportError::Send,
                    OAuthHttpError::ReadBody => AcquisitionTransportError::ReadBody,
                })
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum OAuthHttpError {
    Send,
    ReadBody,
}

async fn post_token(
    client: &reqwest::Client,
    request: TokenPostRequest,
) -> Result<TokenPostResponse, OAuthHttpError> {
    let form_pairs: Vec<(&str, &str)> = request
        .form()
        .iter()
        .map(|(key, value)| (key.as_str(), value.expose_secret()))
        .collect();
    let max_response_bytes = request.max_response_bytes();
    let mut builder = client
        .post(request.endpoint().expose_url().clone())
        .form(&form_pairs);
    if let Some((user, password)) = request.basic_auth() {
        builder = builder.basic_auth(user.expose_secret(), Some(password.expose_secret()));
    }
    drop(form_pairs);
    drop(request);

    let response = builder.send().await.map_err(|_| OAuthHttpError::Send)?;
    let status = response.status().as_u16();
    let body = read_bounded(response, max_response_bytes)
        .await
        .map_err(|_| OAuthHttpError::ReadBody)?;
    TokenPostResponse::try_new(status, body).map_err(|_| OAuthHttpError::ReadBody)
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
    // Keep the source immutable for as long as its exported snapshot is used.
    _registry: Arc<CredentialRegistry>,
    descriptors: Vec<CredentialTypeDescriptor>,
}

impl RegistryCredentialSchema {
    /// Fail composition if any admitted schema cannot be exported.
    #[tracing::instrument(name = "server.credential_catalog.build", skip_all)]
    pub(crate) fn new(registry: Arc<CredentialRegistry>) -> Result<Self, JsonSchemaExportError> {
        let descriptors = registry
            .catalog()
            .map(|(metadata, capabilities)| Self::descriptor(metadata, capabilities))
            .collect::<Result<Vec<_>, _>>()
            .inspect_err(|_| tracing::error!("credential catalog schema export failed"))?;
        Ok(Self {
            _registry: registry,
            descriptors,
        })
    }

    fn descriptor(
        metadata: &nebula_credential::CredentialMetadata,
        capabilities: Capabilities,
    ) -> Result<CredentialTypeDescriptor, JsonSchemaExportError> {
        let key = metadata.key().as_str().to_owned();
        Ok(CredentialTypeDescriptor {
            key,
            name: metadata.name().to_owned(),
            description: metadata.description().to_owned(),
            auth_pattern: format!("{:?}", metadata.pattern()),
            capabilities: CredentialCapabilityFlags {
                interactive: capabilities.contains(Capabilities::INTERACTIVE),
                refreshable: capabilities.contains(Capabilities::REFRESHABLE),
                testable: capabilities.contains(Capabilities::TESTABLE),
                revocable: capabilities.contains(Capabilities::REVOCABLE),
            },
            icon: metadata.icon().as_inline().map(str::to_owned),
            documentation_url: metadata.documentation_url().map(str::to_owned),
            schema_json: metadata.schema().json_schema()?.to_value(),
        })
    }
}

impl CredentialSchemaPort for RegistryCredentialSchema {
    fn list_types(&self) -> Vec<CredentialTypeDescriptor> {
        self.descriptors.clone()
    }

    fn get_type(&self, credential_key: &str) -> Option<CredentialTypeDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.key == credential_key)
            .cloned()
    }
}

#[cfg(test)]
#[path = "credential_adapters_tests.rs"]
mod transport_security_tests;
