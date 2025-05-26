use serde::{Deserialize, Serialize};

use crate::request::{ApplyToRequest, RequestError, RequestOptions, RequestProtocol};

/// Proxy configuration for HTTP requests
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequestProxy {
    /// Proxy host
    pub host: String,

    /// Proxy port
    pub port: u16,

    /// Proxy protocol
    #[serde(default)]
    pub protocol: RequestProtocol,

    /// Username for authentication (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    /// Password for authentication (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl RequestProxy {
    /// Creates a new proxy
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            protocol: RequestProtocol::Http,
            username: None,
            password: None,
        }
    }

    /// Creates an HTTP proxy
    pub fn http(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            protocol: RequestProtocol::Http,
            username: None,
            password: None,
        }
    }

    /// Creates an HTTPS proxy
    pub fn https(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            protocol: RequestProtocol::Https,
            username: None,
            password: None,
        }
    }

    /// Creates a SOCKS4 proxy
    pub fn socks4(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            protocol: RequestProtocol::Socks4,
            username: None,
            password: None,
        }
    }

    /// Creates a SOCKS5 proxy
    pub fn socks5(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            protocol: RequestProtocol::Socks5,
            username: None,
            password: None,
        }
    }

    /// Adds authentication to the proxy
    pub fn with_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self.password = Some(password.into());
        self
    }

    /// Sets the proxy protocol
    pub fn with_protocol(mut self, protocol: RequestProtocol) -> Self {
        self.protocol = protocol;
        self
    }

    /// Returns the full proxy URL
    pub fn url(&self) -> String {
        format!("{}://{}:{}", self.protocol.scheme(), self.host, self.port)
    }
}

impl core::fmt::Display for RequestProxy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.url())
    }
}

impl ApplyToRequest for RequestProxy {
    fn apply_to_options(&self, options: &mut RequestOptions) -> Result<(), RequestError> {
        // Save proxy in options
        options.proxy = Some(self.clone());
        Ok(())
    }
}
