use serde::{Deserialize, Serialize};
use strum_macros::{AsRefStr, Display};

use crate::request::{ApplyToRequest, RequestError, RequestOptions};

/// HTTP protocols
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum RequestProtocol {
    /// Standard HTTP protocol
    #[default]
    Http,
    /// Secure HTTP protocol
    Https,
    /// SOCKS4 proxy protocol
    Socks4,
    /// SOCKS5 proxy protocol
    Socks5,
}

impl RequestProtocol {
    /// Returns whether the protocol is secure (uses encryption)
    pub fn is_secure(&self) -> bool {
        matches!(self, Self::Https)
    }

    /// Returns whether the protocol is HTTP-like
    pub fn is_http(&self) -> bool {
        matches!(self, Self::Http | Self::Https)
    }

    /// Returns whether the protocol is SOCKS-like
    pub fn is_socks(&self) -> bool {
        matches!(self, Self::Socks4 | Self::Socks5)
    }

    /// Returns the default port for the protocol
    pub fn default_port(&self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
            Self::Socks4 | Self::Socks5 => 1080,
        }
    }

    /// Returns the URL scheme for the protocol
    pub fn scheme(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Socks4 => "socks4",
            Self::Socks5 => "socks5",
        }
    }
}

impl ApplyToRequest for RequestProtocol {
    fn apply_to_options(&self, options: &mut RequestOptions) -> Result<(), RequestError> {
        // If base URL has no explicit protocol, add our protocol
        if let Some(base_url) = &options.base_url {
            if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
                options.base_url = Some(format!("{}://{}", self.scheme(), base_url));
            }
        }

        // Set special options for some protocols
        match self {
            Self::Https => {
                options.use_ssl_verification = true;
            }
            Self::Http => {
                // Nothing special for HTTP
            }
            Self::Socks4 | Self::Socks5 => {
                // SOCKS protocols usually require a proxy
                // But that's handled by RequestProxy
            }
        }

        Ok(())
    }
}

impl std::str::FromStr for RequestProtocol {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "http" => Ok(Self::Http),
            "https" => Ok(Self::Https),
            "socks4" => Ok(Self::Socks4),
            "socks5" => Ok(Self::Socks5),
            _ => Err(format!("Unknown protocol: {}", s)),
        }
    }
}
