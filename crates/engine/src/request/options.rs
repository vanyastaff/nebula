use std::collections::HashMap;

use derive_builder::Builder;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::request::{RequestAuth, RequestError, RequestMethod, RequestProxy, Response};

pub const DEFAULT_TIMEOUT: u64 = 30;

#[derive(Debug, Clone, Default, PartialEq, Builder, Serialize, Deserialize)]
#[builder(
    pattern = "owned",
    setter(strip_option, into),
    build_fn(error = "RequestError")
)]
pub struct RequestOptions {
    #[builder(setter(strip_option), default)]
    pub url: Option<String>,

    #[builder(setter(strip_option), default)]
    pub base_url: Option<String>,

    #[builder(default)]
    pub method: RequestMethod,

    #[builder(setter(strip_option), default)]
    pub auth: Option<RequestAuth>,

    #[builder(setter(strip_option), default)]
    pub proxy: Option<RequestProxy>,

    #[builder(default)]
    pub headers: HashMap<String, String>,

    #[builder(default)]
    pub query_params: HashMap<String, String>,

    #[builder(setter(strip_option), default)]
    pub body: Option<Value>,

    #[builder(setter(strip_option), default)]
    pub json: Option<Value>,

    #[builder(default = "DEFAULT_TIMEOUT")]
    pub timeout: u64,

    #[builder(default = "true")]
    pub follow_redirects: bool,

    #[builder(default = "false")]
    pub validate_status: bool,

    #[builder(setter(strip_option), default)]
    pub user_agent: Option<String>,

    #[builder(default = "true")]
    pub use_ssl_verification: bool,
}

impl RequestOptions {
    pub fn builder() -> RequestOptionsBuilder {
        RequestOptionsBuilder::default()
    }

    pub fn get_full_url(&self) -> Option<String> {
        match (&self.base_url, &self.url) {
            (Some(base), Some(path)) => {
                let base = base.trim_end_matches('/');
                let path = path.trim_start_matches('/');
                Some(format!("{}/{}", base, path))
            }
            (Some(base), None) => Some(base.clone()),
            (None, Some(url)) => Some(url.clone()),
            (None, None) => None,
        }
    }

    /// Adds a header to the request
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Adds multiple headers to the request
    pub fn with_headers(
        mut self,
        headers: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        for (key, value) in headers {
            self.headers.insert(key.into(), value.into());
        }
        self
    }

    /// Adds a query parameter to the request
    pub fn with_query_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query_params.insert(key.into(), value.into());
        self
    }

    /// Adds multiple query parameters to the request
    pub fn with_query_params(
        mut self,
        params: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        for (key, value) in params {
            self.query_params.insert(key.into(), value.into());
        }
        self
    }

    /// Sets basic authentication for the request
    pub fn with_basic_auth(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.auth = Some(RequestAuth::basic_auth(username, password));
        self
    }

    /// Sets bearer token authentication for the request
    pub fn with_bearer_auth(mut self, token: impl Into<String>) -> Self {
        self.auth = Some(RequestAuth::bearer(token));
        self
    }

    /// Executes the request and returns the response
    pub async fn execute(self) -> Result<Response, RequestError> {
        // Import the Client from the client module
        use crate::request::client::Client;

        // Create a client with the appropriate settings
        let client = Client::with_options(&self)?;

        // Send the request
        client.send_with_current_client(self).await
    }

    /// Executes the request and parses the response as JSON
    pub async fn execute_and_parse<T: DeserializeOwned>(self) -> Result<T, RequestError> {
        let response = self.execute().await?;
        response.json().map_err(Into::into)
    }
}
