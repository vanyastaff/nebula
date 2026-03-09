//! Dynamic provider contracts for v2 parameter schemas.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::option::SelectOption;
use crate::runtime::ParameterValues;
use crate::schema::FieldSpec;

/// Shared versioned response envelope used by all dynamic providers.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DynamicProviderEnvelope<T> {
    /// Provider payload contract version.
    pub response_version: u16,
    /// Logical payload kind.
    pub kind: DynamicResponseKind,
    /// Ordered response items.
    pub items: Vec<T>,
    /// Optional pagination cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Optional upstream schema/version marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
}

/// Logical response kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicResponseKind {
    /// Dynamic select options.
    Options,
    /// Dynamic field definitions.
    Fields,
}

/// Shared request context for provider resolution.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProviderRequest {
    /// Requesting field id.
    pub field_id: String,
    /// Current runtime values.
    pub values: ParameterValues,
    /// Optional search filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Optional pagination cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Provider contract error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// Requested provider key was not registered.
    #[error("provider not found: {key}")]
    NotFound { key: String },
    /// Provider returned an unexpected response kind.
    #[error("provider `{key}` returned unexpected response kind")]
    KindMismatch { key: String },
    /// Provider returned an unsupported response version.
    #[error("provider `{key}` returned unsupported response version {version}")]
    UnsupportedVersion { key: String, version: u16 },
    /// Provider-specific failure.
    #[error("provider `{key}` failed: {message}")]
    ResolveFailed { key: String, message: String },
}

/// Object-safe dynamic option provider.
#[async_trait]
pub trait OptionProvider: Send + Sync {
    /// Resolves select options.
    async fn resolve(
        &self,
        request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<SelectOption>, ProviderError>;
}

/// Object-safe dynamic field provider used by `DynamicRecord` fields.
#[async_trait]
pub trait DynamicRecordProvider: Send + Sync {
    /// Resolves dynamic field definitions.
    async fn resolve_fields(
        &self,
        request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<FieldSpec>, ProviderError>;
}

/// In-memory registry for dynamic providers.
#[derive(Default)]
pub struct ProviderRegistry {
    option_providers: HashMap<String, Arc<dyn OptionProvider>>,
    dynamic_record_providers: HashMap<String, Arc<dyn DynamicRecordProvider>>,
}

impl ProviderRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an option provider under a stable key.
    pub fn register_option_provider(
        &mut self,
        key: impl Into<String>,
        provider: Arc<dyn OptionProvider>,
    ) {
        self.option_providers.insert(key.into(), provider);
    }

    /// Registers a dynamic-record provider under a stable key.
    pub fn register_dynamic_record_provider(
        &mut self,
        key: impl Into<String>,
        provider: Arc<dyn DynamicRecordProvider>,
    ) {
        self.dynamic_record_providers.insert(key.into(), provider);
    }

    /// Returns a registered option provider, if present.
    #[must_use]
    pub fn option_provider(&self, key: &str) -> Option<&Arc<dyn OptionProvider>> {
        self.option_providers.get(key)
    }

    /// Returns a registered dynamic-record provider, if present.
    #[must_use]
    pub fn dynamic_record_provider(&self, key: &str) -> Option<&Arc<dyn DynamicRecordProvider>> {
        self.dynamic_record_providers.get(key)
    }

    /// Resolves dynamic select options using the provider key.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::NotFound`] when no provider is registered for `key`.
    pub async fn resolve_options(
        &self,
        key: &str,
        request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<SelectOption>, ProviderError> {
        let provider = self
            .option_provider(key)
            .ok_or_else(|| ProviderError::NotFound {
                key: key.to_owned(),
            })?;
        provider.resolve(request).await
    }

    /// Resolves dynamic fields using the provider key.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::NotFound`] when no provider is registered for `key`.
    pub async fn resolve_fields(
        &self,
        key: &str,
        request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<FieldSpec>, ProviderError> {
        let provider =
            self.dynamic_record_provider(key)
                .ok_or_else(|| ProviderError::NotFound {
                    key: key.to_owned(),
                })?;
        provider.resolve_fields(request).await
    }
}
