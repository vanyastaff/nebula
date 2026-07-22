//! Concrete first-party credential adapters owned by the server process.
//!
//! These types implement lower-layer runtime and API read-model ports without
//! entering `nebula-api`'s default public surface.

use std::{future::Future, sync::Arc, time::Duration};

use futures::StreamExt as _;
use nebula_api::ports::credential_schema::{
    CredentialCapabilityFlags, CredentialSchemaPort, CredentialTypeDescriptor,
};
use nebula_credential::{
    AnyCredential, Capabilities, CredentialRegistry,
    runtime::{RefreshTransport, RefreshTransportError, TokenPostRequest, TokenPostResponse},
};
use nebula_schema::ValidSchema;

const OAUTH_TOKEN_HTTP_MAX_REDIRECTS: usize = 5;
const OAUTH_TOKEN_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Reqwest-backed token refresh transport for the first-party process.
#[derive(Debug, Clone)]
pub(crate) struct ReqwestRefreshTransport {
    client: reqwest::Client,
}

impl ReqwestRefreshTransport {
    /// Build the process-wide policy-bearing client.
    pub(crate) fn new() -> Result<Self, reqwest::Error> {
        reqwest::Client::builder()
            .connect_timeout(OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT)
            .timeout(OAUTH_TOKEN_HTTP_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(
                OAUTH_TOKEN_HTTP_MAX_REDIRECTS,
            ))
            .build()
            .map(|client| Self { client })
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
                .form
                .iter()
                .map(|(key, value)| (key.as_str(), value.expose_secret()))
                .collect();
            let mut builder = self.client.post(&request.url).form(&form_pairs);
            if let Some((user, password)) = &request.basic_auth {
                builder = builder.basic_auth(user, Some(password.expose_secret()));
            }

            let response = builder
                .send()
                .await
                .map_err(|error| RefreshTransportError::Send(error.to_string()))?;
            let status = response.status().as_u16();
            let body = read_bounded(response, request.max_response_bytes)
                .await
                .map_err(|error| RefreshTransportError::ReadBody(error.to_string()))?;
            Ok(TokenPostResponse { status, body })
        })
    }
}

async fn read_bounded(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, ReadBoundedError> {
    if let Some(claimed) = response.content_length() {
        let max = u64::try_from(max_bytes).unwrap_or(u64::MAX);
        if claimed > max {
            return Err(ReadBoundedError::ContentLengthTooLarge {
                claimed,
                max: max_bytes,
            });
        }
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(ReadBoundedError::Read)?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ReadBoundedError::BodyTooLarge { max: max_bytes });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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
