//! Opt-in HTTP credential CRUD client (`http` feature).
//!
//! Requires a Tokio runtime. Each method sends one request, never follows redirects,
//! and never retries. A mutation whose acknowledgement cannot be interpreted has an
//! unknown outcome: callers must inspect server state before deciding what to do.

use std::{
    fmt,
    time::{Duration, SystemTime},
};

use reqwest::{
    Method, Url,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue, RETRY_AFTER},
};
use serde::{Serialize, de::DeserializeOwned};

use super::credential::v1::{
    ContinueResolveCredentialRequest, ContinueResolveCredentialResponse, CreateCredentialRequest,
    CreateCredentialResponse, Credential, CredentialProblem, CredentialProblemKind,
    DeleteCredentialResponse, GetCredentialResponse, ListCredentialsRequest,
    ListCredentialsResponse, ProblemDetails, ReauthorizeCredentialRequest,
    ReauthorizeCredentialResponse, ReconcileCredentialRequest, ReconcileCredentialResponse,
    ResolveCredentialRequest, ResolveCredentialResponse, RetryAfter, UpdateCredentialRequest,
};

/// Bearer authority used only in the Authorization header. Debug is redacted.
pub struct BearerToken(HeaderValue);

impl BearerToken {
    /// Validate an opaque bearer token without retaining invalid input in errors.
    pub fn new(token: &str) -> Result<Self, HttpError> {
        if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(HttpError::new(HttpErrorKind::InvalidConfiguration));
        }
        let authorization = zeroize::Zeroizing::new(format!("Bearer {token}"));
        let mut value = HeaderValue::from_str(&authorization)
            .map_err(|_| HttpError::new(HttpErrorKind::InvalidConfiguration))?;
        value.set_sensitive(true);
        Ok(Self(value))
    }
}

impl fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BearerToken([REDACTED])")
    }
}

/// Bounded transport settings. Zero timeouts and zero response limits are rejected.
#[derive(Debug, Clone, Copy)]
pub struct HttpOptions {
    /// Maximum time allowed to establish a connection (default: five seconds).
    pub connect_timeout: Duration,
    /// Maximum total request/response time (default: thirty seconds).
    pub request_timeout: Duration,
    /// Maximum response bytes buffered (default: one MiB).
    pub max_response_bytes: usize,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
            max_response_bytes: 1024 * 1024,
        }
    }
}

/// Stable, secret-free classification. None of these failures triggers a retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpErrorKind {
    /// Invalid base URL, bearer, tenant selector, or limits; no request was sent.
    InvalidConfiguration,
    /// Request construction failed before transport; no request was sent.
    InvalidRequest,
    /// A read-only request could not obtain a complete response.
    Transport,
    /// A read-only response was oversized or did not match the public contract.
    InvalidResponse,
    /// The server returned a typed RFC 9457 failure response.
    Problem,
    /// A mutation might have been applied; its acknowledgement is unavailable or invalid.
    OutcomeUnknown,
}

/// Transport error whose formatting excludes URLs, authority, and response content.
pub struct HttpError {
    kind: HttpErrorKind,
    status: Option<u16>,
    retry_after: Option<RetryAfter>,
    problem: Option<Box<CredentialProblem>>,
}

impl HttpError {
    fn new(kind: HttpErrorKind) -> Self {
        Self {
            kind,
            status: None,
            retry_after: None,
            problem: None,
        }
    }

    /// Stable failure category; never implies replay safety.
    #[must_use]
    pub fn kind(&self) -> HttpErrorKind {
        self.kind
    }

    /// HTTP status when response headers were received.
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        self.status
    }

    /// Parsed Retry-After advice; this client never schedules a retry.
    #[must_use]
    pub fn retry_after(&self) -> Option<RetryAfter> {
        self.retry_after
    }

    /// Parsed server problem. Its text and extension values are untrusted response
    /// content: do not log them as secrets may be echoed by a misconfigured server.
    #[must_use]
    pub fn problem(&self) -> Option<&CredentialProblem> {
        self.problem.as_deref()
    }
}

impl fmt::Debug for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpError")
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("retry_after", &self.retry_after)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "credential HTTP request failed ({:?})", self.kind)
    }
}

impl std::error::Error for HttpError {}

/// One authenticated connection configuration, reusable across explicit tenant scopes.
#[derive(Clone)]
pub struct HttpClient {
    transport: reqwest::Client,
    base: Url,
    max_response_bytes: usize,
}

impl fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpClient").finish_non_exhaustive()
    }
}

impl HttpClient {
    /// Connect to a deployment base URL (origin plus optional mount prefix).
    /// `/api/v1` is appended by the client. Queries, fragments and URL userinfo are
    /// rejected. HTTP is supported for local deployments; use HTTPS across networks.
    pub fn new(
        base_url: &str,
        bearer: BearerToken,
        options: HttpOptions,
    ) -> Result<Self, HttpError> {
        let base = Url::parse(base_url)
            .map_err(|_| HttpError::new(HttpErrorKind::InvalidConfiguration))?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || options.connect_timeout.is_zero()
            || options.request_timeout.is_zero()
            || options.max_response_bytes == 0
        {
            return Err(HttpError::new(HttpErrorKind::InvalidConfiguration));
        }
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(AUTHORIZATION, bearer.0);
        let transport = reqwest::Client::builder()
            .default_headers(headers)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(options.connect_timeout)
            .timeout(options.request_timeout)
            .build()
            .map_err(|_| HttpError::new(HttpErrorKind::InvalidConfiguration))?;
        Ok(Self {
            transport,
            base,
            max_response_bytes: options.max_response_bytes,
        })
    }

    /// Select an organization and workspace. Selectors are encoded as individual
    /// path segments and cannot replace the destination origin or mount prefix.
    pub fn credentials(
        &self,
        organization: &str,
        workspace: &str,
    ) -> Result<CredentialClient, HttpError> {
        validate_selector(organization)?;
        validate_selector(workspace)?;
        let mut collection = self.base.clone();
        collection
            .path_segments_mut()
            .map_err(|()| HttpError::new(HttpErrorKind::InvalidConfiguration))?
            .pop_if_empty()
            .extend([
                "api",
                "v1",
                "orgs",
                organization,
                "workspaces",
                workspace,
                "credentials",
            ]);
        Ok(CredentialClient {
            client: self.clone(),
            collection,
        })
    }
}

/// Credential CRUD methods bound to one explicit organization/workspace pair.
#[derive(Clone)]
pub struct CredentialClient {
    client: HttpClient,
    collection: Url,
}

impl fmt::Debug for CredentialClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialClient").finish_non_exhaustive()
    }
}

impl CredentialClient {
    /// Start credential acquisition once. Pending responses must be continued with a
    /// client carrying the exact same bearer token; this client never follows the
    /// interaction URL, polls, refreshes authentication, or replays the request.
    pub async fn resolve(
        &self,
        request: &ResolveCredentialRequest,
    ) -> Result<ResolveCredentialResponse, HttpError> {
        self.write(Method::POST, self.collection_url(&["resolve"])?, request)
            .await
    }

    /// Continue one pending acquisition once using this client's unchanged bearer.
    pub async fn continue_resolve(
        &self,
        request: &ContinueResolveCredentialRequest,
    ) -> Result<ContinueResolveCredentialResponse, HttpError> {
        self.write(
            Method::POST,
            self.collection_url(&["resolve", "continue"])?,
            request,
        )
        .await
    }

    /// Start reauthorization for an existing credential once. The request contains
    /// only provider data; identity and aggregate fences remain server-owned.
    pub async fn reauthorize(
        &self,
        credential_id: &str,
        request: &ReauthorizeCredentialRequest,
    ) -> Result<ReauthorizeCredentialResponse, HttpError> {
        let mut url = self.item_url(credential_id)?;
        url.path_segments_mut()
            .map_err(|()| HttpError::new(HttpErrorKind::InvalidConfiguration))?
            .push("reauthorize");
        self.write(Method::POST, url, request).await
    }

    /// Read one page of credential metadata.
    pub async fn list(
        &self,
        query: &ListCredentialsRequest,
    ) -> Result<ListCredentialsResponse, HttpError> {
        let mut url = self.collection.clone();
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(page) = query.page {
                pairs.append_pair("page", &page.to_string());
            }
            if let Some(size) = query.page_size {
                pairs.append_pair("page_size", &size.to_string());
            }
            if let Some(key) = &query.credential_key {
                pairs.append_pair("credential_key", key);
            }
            if let Some(pattern) = &query.auth_pattern {
                pairs.append_pair("auth_pattern", pattern);
            }
        }
        let request = self.client.transport.get(url);
        self.send(request, false).await
    }

    /// Create once. An unknown outcome must be reconciled before another attempt.
    pub async fn create(
        &self,
        request: &CreateCredentialRequest,
    ) -> Result<CreateCredentialResponse, HttpError> {
        self.write(Method::POST, self.collection.clone(), request)
            .await
    }

    /// Read one credential's public metadata.
    pub async fn get(&self, credential_id: &str) -> Result<GetCredentialResponse, HttpError> {
        self.send(
            self.client.transport.get(self.item_url(credential_id)?),
            false,
        )
        .await
    }

    /// Update once, preserving the caller's optimistic version when supplied.
    pub async fn update(
        &self,
        credential_id: &str,
        request: &UpdateCredentialRequest,
    ) -> Result<Credential, HttpError> {
        self.write(Method::PUT, self.item_url(credential_id)?, request)
            .await
    }

    /// Delete once. No Idempotency-Key or automatic replay is used.
    pub async fn delete(&self, credential_id: &str) -> Result<DeleteCredentialResponse, HttpError> {
        self.send(
            self.client.transport.delete(self.item_url(credential_id)?),
            true,
        )
        .await
    }

    /// Record one operator-established provider outcome exactly once.
    ///
    /// The server makes an identical request idempotent, but this client never
    /// retries automatically. After an unknown acknowledgement, callers may
    /// resend the same operation, decision, and evidence.
    pub async fn reconcile(
        &self,
        credential_id: &str,
        request: &ReconcileCredentialRequest,
    ) -> Result<ReconcileCredentialResponse, HttpError> {
        let mut url = self.item_url(credential_id)?;
        url.path_segments_mut()
            .map_err(|()| HttpError::new(HttpErrorKind::InvalidConfiguration))?
            .push("reconcile");
        self.write(Method::POST, url, request).await
    }

    fn item_url(&self, id: &str) -> Result<Url, HttpError> {
        validate_selector(id)?;
        let mut url = self.collection.clone();
        url.path_segments_mut()
            .map_err(|()| HttpError::new(HttpErrorKind::InvalidConfiguration))?
            .push(id);
        Ok(url)
    }

    fn collection_url(&self, segments: &[&str]) -> Result<Url, HttpError> {
        let mut url = self.collection.clone();
        url.path_segments_mut()
            .map_err(|()| HttpError::new(HttpErrorKind::InvalidConfiguration))?
            .extend(segments.iter().copied());
        Ok(url)
    }

    async fn write<T: DeserializeOwned>(
        &self,
        method: Method,
        url: Url,
        value: &impl Serialize,
    ) -> Result<T, HttpError> {
        let body =
            serde_json::to_vec(value).map_err(|_| HttpError::new(HttpErrorKind::InvalidRequest))?;
        self.send(
            self.client
                .transport
                .request(method, url)
                .header(CONTENT_TYPE, "application/json")
                .body(body),
            true,
        )
        .await
    }

    #[tracing::instrument(name = "sdk.credential.http", skip_all, fields(mutation))]
    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        mutation: bool,
    ) -> Result<T, HttpError> {
        let request = request
            .build()
            .map_err(|_| HttpError::new(HttpErrorKind::InvalidRequest))?;
        let mut failure = HttpError::new(if mutation {
            HttpErrorKind::OutcomeUnknown
        } else {
            HttpErrorKind::Transport
        });
        let mut response = self
            .client
            .transport
            .execute(request)
            .await
            .map_err(|_| HttpError::new(failure.kind))?;
        failure.status = Some(response.status().as_u16());
        failure.retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);
        if !mutation {
            failure.kind = HttpErrorKind::InvalidResponse;
        }
        if response
            .content_length()
            .is_some_and(|size| size > self.client.max_response_bytes as u64)
        {
            return Err(failure);
        }
        let status = response.status();
        let media_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        let is_problem =
            media_type.is_some_and(|mime| mime.eq_ignore_ascii_case("application/problem+json"));
        let is_json = media_type.is_some_and(|mime| mime.eq_ignore_ascii_case("application/json"));
        let mut body = zeroize::Zeroizing::new(Vec::new());
        loop {
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(_) => {
                    if !mutation {
                        failure.kind = HttpErrorKind::Transport;
                    }
                    return Err(failure);
                },
            };
            if chunk.len() > self.client.max_response_bytes.saturating_sub(body.len()) {
                return Err(failure);
            }
            body.extend_from_slice(&chunk);
        }
        if status.as_u16() == 200 && is_json {
            return serde_json::from_slice(&body).map_err(|_| failure);
        }
        if (status.is_client_error() || status.is_server_error())
            && is_problem
            && let Ok(problem) = serde_json::from_slice::<ProblemDetails>(&body)
            && problem.status == status.as_u16()
        {
            let problem = CredentialProblem {
                problem,
                retry_after: failure.retry_after,
            };
            failure.kind = if problem.credential_kind() == CredentialProblemKind::OutcomeUnknown {
                HttpErrorKind::OutcomeUnknown
            } else {
                HttpErrorKind::Problem
            };
            failure.problem = Some(Box::new(problem));
        }
        Err(failure)
    }
}

fn validate_selector(selector: &str) -> Result<(), HttpError> {
    if selector.is_empty()
        || matches!(selector, "." | "..")
        || selector.chars().any(char::is_control)
    {
        return Err(HttpError::new(HttpErrorKind::InvalidConfiguration));
    }
    Ok(())
}

fn parse_retry_after(value: &str) -> Option<RetryAfter> {
    let seconds = (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse::<u64>().ok())
        .flatten()
        .or_else(|| {
            httpdate::parse_http_date(value)
                .ok()?
                .duration_since(SystemTime::now())
                .ok()
                .map(|delay| {
                    delay
                        .as_secs()
                        .saturating_add(u64::from(delay.subsec_nanos() > 0))
                })
        })?;
    RetryAfter::from_seconds(seconds)
}
