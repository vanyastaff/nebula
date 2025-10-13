//! Global fields configuration

use serde::{Deserialize, Serialize};

/// Global fields configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Fields {
    /// Service name
    pub service: Option<String>,
    /// Environment (dev/staging/prod)
    pub env: Option<String>,
    /// Version
    pub version: Option<String>,
    /// Instance ID
    pub instance: Option<String>,
    /// Region
    pub region: Option<String>,
    /// Custom fields
    #[serde(flatten)]
    pub custom: std::collections::BTreeMap<String, serde_json::Value>,
}

impl Fields {
    /// Create fields from environment variables
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            service: std::env::var("NEBULA_SERVICE").ok(),
            env: std::env::var("NEBULA_ENV").ok(),
            version: std::env::var("NEBULA_VERSION")
                .ok()
                .or_else(|| option_env!("CARGO_PKG_VERSION").map(String::from)),
            instance: std::env::var("NEBULA_INSTANCE").ok(),
            region: std::env::var("NEBULA_REGION").ok(),
            custom: std::collections::BTreeMap::default(),
        }
    }

    /// Check if fields are empty
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.service.is_none()
            && self.env.is_none()
            && self.version.is_none()
            && self.instance.is_none()
            && self.region.is_none()
            && self.custom.is_empty()
    }
}
