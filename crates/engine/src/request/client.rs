use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::{Client as ReqwestClient, ClientBuilder as ReqwestClientBuilder};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::request::response::{AsyncTryFrom, Response};
use crate::request::{
    ApplyToRequest, RequestAuth, RequestError, RequestMethod, RequestOptions,
    RequestProtocol, RequestProxy,
};

/// Builder for HTTP requests
pub struct RequestBuilder {
    /// Options for the request
    options: RequestOptions,
}

impl RequestBuilder {
    /// Creates a new request builder with default options
    pub fn new() -> Self {
        Self {
            options: RequestOptions::default(),
        }
    }

    /// Creates a request builder from existing options
    pub fn from_options(options: RequestOptions) -> Self {
        Self { options }
    }

    /// Sets the HTTP method
    pub fn method(mut self, method: RequestMethod) -> Self {
        method.apply_to_options(&mut self.options).unwrap();
        self
    }

    /// Sets the request URL
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.options.url = Some(url.into());
        self
    }

    /// Sets the base URL
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.options.base_url = Some(url.into());
        self
    }

    /// Sets the request headers
    pub fn headers<I>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        self.options.headers.extend(headers);
        self
    }

    /// Adds a header
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.headers.insert(key.into(), value.into());
        self
    }

    /// Sets the request query parameters
    pub fn query_params<I>(mut self, params: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        self.options.query_params.extend(params);
        self
    }

    /// Adds a query parameter
    pub fn query_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.query_params.insert(key.into(), value.into());
        self
    }

    /// Sets the JSON body
    pub fn json<T: Serialize + ?Sized>(mut self, json: &T) -> Result<Self, RequestError> {
        self.options.json = Some(serde_json::to_value(json)?);
        Ok(self)
    }

    /// Sets the raw body
    pub fn body(mut self, body: impl Into<Value>) -> Self {
        self.options.body = Some(body.into());
        self
    }

    /// Sets the request timeout
    pub fn timeout(mut self, seconds: u64) -> Self {
        self.options.timeout = seconds;
        self
    }

    /// Sets whether to follow redirects
    pub fn follow_redirects(mut self, follow: bool) -> Self {
        self.options.follow_redirects = follow;
        self
    }

    /// Sets whether to validate response status
    pub fn validate_status(mut self, validate: bool) -> Self {
        self.options.validate_status = validate;
        self
    }

    /// Sets whether to verify SSL certificates
    pub fn use_ssl_verification(mut self, verify: bool) -> Self {
        self.options.use_ssl_verification = verify;
        self
    }

    /// Sets the User-Agent header
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.options.user_agent = Some(user_agent.into());
        self
    }

    /// Sets the authentication method
    pub fn auth(mut self, auth: RequestAuth) -> Self {
        auth.apply_to_options(&mut self.options).unwrap();
        self
    }

    /// Sets basic authentication
    pub fn basic_auth(self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.auth(RequestAuth::basic_auth(username, password))
    }

    /// Sets bearer token authentication
    pub fn bearer_auth(self, token: impl Into<String>) -> Self {
        self.auth(RequestAuth::bearer(token))
    }

    /// Sets the proxy
    pub fn proxy(mut self, proxy: RequestProxy) -> Self {
        proxy.apply_to_options(&mut self.options).unwrap();
        self
    }

    /// Sets the protocol
    pub fn protocol(mut self, protocol: RequestProtocol) -> Self {
        protocol.apply_to_options(&mut self.options).unwrap();
        self
    }

    /// Gets the request options
    pub fn build(self) -> RequestOptions {
        self.options
    }

    /// Sends the request and returns the response
    pub async fn send(self) -> Result<Response, RequestError> {
        Client::new()?.send(self.options).await
    }

    /// Sends the request and parses the response as JSON
    pub async fn send_and_parse<T: DeserializeOwned>(self) -> Result<T, RequestError> {
        let response = self.send().await?;
        response.json().map_err(Into::into)
    }

    // Factory methods for creating requests with different HTTP methods

    /// Creates a GET request
    pub fn get(url: impl Into<String>) -> Self {
        Self::new().method(RequestMethod::Get).url(url)
    }

    /// Creates a POST request
    pub fn post(url: impl Into<String>) -> Self {
        Self::new().method(RequestMethod::Post).url(url)
    }

    /// Creates a PUT request
    pub fn put(url: impl Into<String>) -> Self {
        Self::new().method(RequestMethod::Put).url(url)
    }

    /// Creates a DELETE request
    pub fn delete(url: impl Into<String>) -> Self {
        Self::new().method(RequestMethod::Delete).url(url)
    }

    /// Creates a PATCH request
    pub fn patch(url: impl Into<String>) -> Self {
        Self::new().method(RequestMethod::Patch).url(url)
    }

    /// Creates a HEAD request
    pub fn head(url: impl Into<String>) -> Self {
        Self::new().method(RequestMethod::Head).url(url)
    }

    /// Creates a OPTIONS request
    pub fn options(url: impl Into<String>) -> Self {
        Self::new().method(RequestMethod::Options).url(url)
    }
}

impl Default for RequestBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// HTTP client for sending requests
#[derive(Clone)]
pub struct Client {
    /// The underlying reqwest client
    client: ReqwestClient,
}

impl Client {
    /// Creates a new client with default settings
    pub fn new() -> Result<Self, RequestError> {
        let client = ReqwestClient::new();
        Ok(Self { client })
    }

    /// Creates a new client with custom settings
    pub fn with_options(options: &RequestOptions) -> Result<Self, RequestError> {
        let mut builder = ReqwestClientBuilder::new();

        // Set timeout
        builder = builder.timeout(Duration::from_secs(options.timeout));

        // Set redirect policy
        builder = builder.redirect(if options.follow_redirects {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        });

        // Set SSL verification
        if !options.use_ssl_verification {
            builder = builder.danger_accept_invalid_certs(true);
        }

        // Configure proxy if specified
        if let Some(proxy) = &options.proxy {
            let proxy_url = proxy.url();
            let reqwest_proxy = reqwest::Proxy::all(&proxy_url)
                .map_err(|e| RequestError::ProxyError(format!("Failed to create proxy: {}", e)))?;

            let proxy_with_auth =
                if let (Some(username), Some(password)) = (&proxy.username, &proxy.password) {
                    reqwest_proxy.basic_auth(username, password)
                } else {
                    reqwest_proxy
                };

            builder = builder.proxy(proxy_with_auth);
        }

        // Set default headers if needed
        // ...

        let client = builder
            .build()
            .map_err(|e| RequestError::BuildError(format!("Failed to build client: {}", e)))?;

        Ok(Self { client })
    }

    /// Sends a request with the given options
    pub async fn send(&self, options: RequestOptions) -> Result<Response, RequestError> {
        // If we have proxy settings or specific client settings,
        // we need to build a new client with those settings
        if options.proxy.is_some()
            || options.follow_redirects != true
            || options.use_ssl_verification != true
        {
            let client = Self::with_options(&options)?;
            return client.send_with_current_client(options).await;
        }

        // Otherwise use current client
        self.send_with_current_client(options).await
    }

    /// Sends a request using the current client instance
    pub(crate) async fn send_with_current_client(
        &self,
        options: RequestOptions,
    ) -> Result<Response, RequestError> {
        // Get the full URL
        let url = options
            .get_full_url()
            .ok_or_else(|| RequestError::InvalidUrl("No URL provided".to_string()))?;

        // Start timing the request
        let start_time = Instant::now();

        // Create the request
        let method = reqwest::Method::from_bytes(options.method.as_str().as_bytes())
            .map_err(|e| RequestError::Other(format!("Invalid method: {}", e)))?;

        let mut request = self.client.request(method, &url);

        // Add headers
        for (key, value) in &options.headers {
            request = request.header(key, value);
        }

        // Add query parameters
        if !options.query_params.is_empty() {
            request = request.query(&options.query_params);
        }

        // Add User-Agent
        if let Some(user_agent) = &options.user_agent {
            request = request.header(reqwest::header::USER_AGENT, user_agent);
        }

        // Add body or JSON
        if options.method.can_have_body() {
            if let Some(json) = &options.json {
                request = request.json(json);
            } else if let Some(body) = &options.body {
                request = request.json(body);
            }
        }

        // Set timeout
        request = request.timeout(Duration::from_secs(options.timeout));

        // Send the request
        let response = request
            .send()
            .await
            .map_err(|e| RequestError::RequestFailed(format!("Request failed: {}", e)))?;

        // Calculate duration
        let duration = start_time.elapsed();

        // Convert to our Response type
        let mut api_response = Response::async_try_from(response)
            .await
            .map_err(|e| RequestError::ResponseError(e))?;

        // Set the duration
        api_response.duration_ms = duration.as_millis() as u64;

        // Validate status if requested
        if options.validate_status {
            api_response = api_response.error_for_status()?;
        }

        Ok(api_response)
    }

    /// Sends a GET request
    pub async fn get(&self, url: impl Into<String>) -> Result<Response, RequestError> {
        let options = RequestOptions::builder()
            .url(url)
            .method(RequestMethod::Get)
            .build()?;

        self.send(options).await
    }

    /// Sends a POST request with JSON body
    pub async fn post_json<T: Serialize>(
        &self,
        url: impl Into<String>,
        json: &T,
    ) -> Result<Response, RequestError> {
        let json_value = serde_json::to_value(json)?;

        let options = RequestOptions::builder()
            .url(url)
            .method(RequestMethod::Post)
            .json(json_value)
            .build()?;

        self.send(options).await
    }

    /// Sends a PUT request with JSON body
    pub async fn put_json<T: Serialize>(
        &self,
        url: impl Into<String>,
        json: &T,
    ) -> Result<Response, RequestError> {
        let json_value = serde_json::to_value(json)?;

        let options = RequestOptions::builder()
            .url(url)
            .method(RequestMethod::Put)
            .json(json_value)
            .build()?;

        self.send(options).await
    }

    /// Sends a DELETE request
    pub async fn delete(&self, url: impl Into<String>) -> Result<Response, RequestError> {
        let options = RequestOptions::builder()
            .url(url)
            .method(RequestMethod::Delete)
            .build()?;

        self.send(options).await
    }
}

/// Implementation of AsyncTryFrom for reqwest::Response to convert it to our
/// Response type
#[async_trait]
impl AsyncTryFrom<reqwest::Response> for Response {
    type Error = crate::request::response::ResponseError;

    async fn async_try_from(resp: reqwest::Response) -> Result<Self, Self::Error> {
        let url = resp.url().to_string();
        let status = resp.status().as_u16();
        let status_text = resp.status().canonical_reason().map(|s| s.to_string());

        // Convert headers to a HashMap
        let headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                if let Ok(v_str) = v.to_str() {
                    Some((k.to_string(), v_str.to_string()))
                } else {
                    None
                }
            })
            .collect();

        let redirected = resp.status().is_redirection();

        // Get the response body
        let bytes = resp.bytes().await.map_err(|e| {
            crate::request::response::ResponseError::Other(format!(
                "Failed to get response body: {}",
                e
            ))
        })?;

        // Try to determine if the response is JSON
        let is_json = headers
            .iter()
            .any(|(k, v)| k.to_lowercase() == "content-type" && v.contains("application/json"));

        let mut response = Response {
            status,
            status_text,
            headers,
            body: None,
            text: None,
            url,
            redirected,
            duration_ms: 0, // This will be set by the caller
            bytes: Some(bytes.to_vec()),
        };

        // Try to parse as JSON if the content type is application/json
        if is_json {
            if let Ok(json) = serde_json::from_slice(&bytes) {
                response.body = Some(json);
            }
        }

        // Always try to provide text representation if possible
        if let Ok(text) = String::from_utf8(bytes.to_vec()) {
            response.text = Some(text);
        }

        Ok(response)
    }
}

// Helper functions for common request types

/// Sends a GET request
pub async fn get(url: impl Into<String>) -> Result<Response, RequestError> {
    RequestBuilder::get(url).send().await
}

/// Sends a POST request with JSON body
pub async fn post<T: Serialize>(
    url: impl Into<String>,
    json: &T,
) -> Result<Response, RequestError> {
    RequestBuilder::post(url).json(json)?.send().await
}

/// Sends a PUT request with JSON body
pub async fn put<T: Serialize>(url: impl Into<String>, json: &T) -> Result<Response, RequestError> {
    RequestBuilder::put(url).json(json)?.send().await
}

/// Sends a DELETE request
pub async fn delete(url: impl Into<String>) -> Result<Response, RequestError> {
    RequestBuilder::delete(url).send().await
}

/// Sends a PATCH request with JSON body
pub async fn patch<T: Serialize>(
    url: impl Into<String>,
    json: &T,
) -> Result<Response, RequestError> {
    RequestBuilder::patch(url).json(json)?.send().await
}

/// Sends a HEAD request
pub async fn head(url: impl Into<String>) -> Result<Response, RequestError> {
    RequestBuilder::head(url).send().await
}

/// Sends a OPTIONS request
pub async fn options(url: impl Into<String>) -> Result<Response, RequestError> {
    RequestBuilder::options(url).send().await
}

/// Sends a GET request and parses the response as JSON
pub async fn get_json<T: DeserializeOwned>(url: impl Into<String>) -> Result<T, RequestError> {
    let response = get(url).await?;
    response.json().map_err(Into::into)
}

/// Sends a POST request with JSON body and parses the response as JSON
pub async fn post_json<T: Serialize, R: DeserializeOwned>(
    url: impl Into<String>,
    json: &T,
) -> Result<R, RequestError> {
    let response = post(url, json).await?;
    response.json().map_err(Into::into)
}

/// Sends a PUT request with JSON body and parses the response as JSON
pub async fn put_json<T: Serialize, R: DeserializeOwned>(
    url: impl Into<String>,
    json: &T,
) -> Result<R, RequestError> {
    let response = put(url, json).await?;
    response.json().map_err(Into::into)
}

/// Sends a DELETE request and parses the response as JSON
pub async fn delete_json<R: DeserializeOwned>(url: impl Into<String>) -> Result<R, RequestError> {
    let response = delete(url).await?;
    response.json().map_err(Into::into)
}
