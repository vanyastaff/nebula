use serde::{Deserialize, Serialize};
use strum_macros::{AsRefStr, Display};

use crate::request::{ApplyToRequest, RequestError, RequestOptions};

/// HTTP request methods
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[strum(serialize_all = "UPPERCASE")]
#[serde(rename_all = "UPPERCASE")]
pub enum RequestMethod {
    /// GET method for retrieving resources
    #[default]
    Get,
    /// POST method for creating resources
    Post,
    /// PUT method for updating resources
    Put,
    /// DELETE method for removing resources
    Delete,
    /// PATCH method for partial updates
    Patch,
    /// HEAD method for retrieving headers only
    Head,
    /// OPTIONS method for checking available methods
    Options,
}

impl RequestMethod {
    /// Checks if this method can have a request body
    pub fn can_have_body(&self) -> bool {
        match self {
            Self::Get | Self::Head => false,
            Self::Post | Self::Put | Self::Delete | Self::Patch | Self::Options => true,
        }
    }

    /// Checks if the method is safe (doesn't modify the server state)
    pub fn is_safe(&self) -> bool {
        match self {
            Self::Get | Self::Head | Self::Options => true,
            Self::Post | Self::Put | Self::Delete | Self::Patch => false,
        }
    }

    /// Checks if the method is idempotent (repeating has the same effect as a
    /// single request)
    pub fn is_idempotent(&self) -> bool {
        match self {
            Self::Get | Self::Head | Self::Put | Self::Delete | Self::Options => true,
            Self::Post | Self::Patch => false,
        }
    }

    /// Returns the recommended timeout for this method (in seconds)
    pub fn recommended_timeout(&self) -> u64 {
        match self {
            Self::Get | Self::Head => 30,               // Fast read operations
            Self::Post | Self::Put | Self::Patch => 60, // Write operations may take longer
            Self::Delete => 45,                         /* Deletion might take an intermediate */
            // amount of time
            Self::Options => 15, // Usually fast metadata
        }
    }

    /// Returns the string representation of the method in uppercase
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Patch => "PATCH",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }
}

impl ApplyToRequest for RequestMethod {
    fn apply_to_options(&self, options: &mut RequestOptions) -> Result<(), RequestError> {
        options.method = *self;

        // Set the recommended timeout if not already set
        if options.timeout == crate::request::DEFAULT_TIMEOUT {
            options.timeout = self.recommended_timeout();
        }

        // Add the necessary headers depending on the method
        match self {
            Self::Post | Self::Put | Self::Patch => {
                // For methods with body, add Content-Type if not already specified
                if !options
                    .headers
                    .iter()
                    .any(|(k, _)| k.to_lowercase() == "content-type")
                {
                    options
                        .headers
                        .insert("Content-Type".to_string(), "application/json".to_string());
                }
            }
            Self::Get => {
                // For GET, add a caching header if not already specified
                if !options
                    .headers
                    .iter()
                    .any(|(k, _)| k.to_lowercase() == "cache-control")
                {
                    options
                        .headers
                        .insert("Cache-Control".to_string(), "no-cache".to_string());
                }
            }
            _ => {}
        }

        Ok(())
    }
}

impl core::str::FromStr for RequestMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            "PUT" => Ok(Self::Put),
            "DELETE" => Ok(Self::Delete),
            "PATCH" => Ok(Self::Patch),
            "HEAD" => Ok(Self::Head),
            "OPTIONS" => Ok(Self::Options),
            _ => Err(format!("Unknown HTTP method: {}", s)),
        }
    }
}
