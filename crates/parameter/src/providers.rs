//! Dynamic provider contracts for v2 parameter schemas.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::option::SelectOption;
use crate::runtime::ParameterValues;
use crate::spec::FieldSpec;

/// Canonical dynamic-provider response version supported by v2.
pub const DYNAMIC_PROVIDER_RESPONSE_VERSION: u16 = 1;

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

impl<T> DynamicProviderEnvelope<T> {
    /// Creates a response envelope using the canonical response version.
    #[must_use]
    pub fn new(kind: DynamicResponseKind, items: Vec<T>) -> Self {
        Self {
            response_version: DYNAMIC_PROVIDER_RESPONSE_VERSION,
            kind,
            items,
            next_cursor: None,
            schema_version: None,
        }
    }
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
#[non_exhaustive]
pub enum ProviderError {
    /// Provider key failed validation.
    #[error("invalid provider key `{key}`: {reason}")]
    InvalidKey { key: String, reason: String },
    /// Provider key is already registered in the target registry.
    #[error("provider already registered: {key}")]
    AlreadyRegistered { key: String },
    /// Requested provider key was not registered.
    #[error("provider not found: {key}")]
    NotFound { key: String },
    /// Provider returned an unexpected response kind.
    #[error("provider `{key}` returned unexpected response kind: expected {expected:?}, got {actual:?}")]
    KindMismatch {
        key: String,
        expected: DynamicResponseKind,
        actual: DynamicResponseKind,
    },
    /// Provider returned an unsupported response version.
    #[error(
        "provider `{key}` returned unsupported response version {version}; expected {expected}"
    )]
    UnsupportedVersion {
        key: String,
        version: u16,
        expected: u16,
    },
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
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidKey`] when `key` violates key format
    /// rules and [`ProviderError::AlreadyRegistered`] when a provider is
    /// already registered for `key`.
    pub fn register_option_provider(
        &mut self,
        key: impl Into<String>,
        provider: Arc<dyn OptionProvider>,
    ) -> Result<(), ProviderError> {
        let key = key.into();
        validate_provider_key(&key)?;

        if self.option_providers.contains_key(&key) {
            return Err(ProviderError::AlreadyRegistered { key });
        }

        self.option_providers.insert(key, provider);
        Ok(())
    }

    /// Registers or replaces an option provider under a stable key.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidKey`] when `key` violates key format
    /// rules.
    pub fn upsert_option_provider(
        &mut self,
        key: impl Into<String>,
        provider: Arc<dyn OptionProvider>,
    ) -> Result<Option<Arc<dyn OptionProvider>>, ProviderError> {
        let key = key.into();
        validate_provider_key(&key)?;
        Ok(self.option_providers.insert(key, provider))
    }

    /// Registers a dynamic-record provider under a stable key.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidKey`] when `key` violates key format
    /// rules and [`ProviderError::AlreadyRegistered`] when a provider is
    /// already registered for `key`.
    pub fn register_dynamic_record_provider(
        &mut self,
        key: impl Into<String>,
        provider: Arc<dyn DynamicRecordProvider>,
    ) -> Result<(), ProviderError> {
        let key = key.into();
        validate_provider_key(&key)?;

        if self.dynamic_record_providers.contains_key(&key) {
            return Err(ProviderError::AlreadyRegistered { key });
        }

        self.dynamic_record_providers.insert(key, provider);
        Ok(())
    }

    /// Registers or replaces a dynamic-record provider under a stable key.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::InvalidKey`] when `key` violates key format
    /// rules.
    pub fn upsert_dynamic_record_provider(
        &mut self,
        key: impl Into<String>,
        provider: Arc<dyn DynamicRecordProvider>,
    ) -> Result<Option<Arc<dyn DynamicRecordProvider>>, ProviderError> {
        let key = key.into();
        validate_provider_key(&key)?;
        Ok(self.dynamic_record_providers.insert(key, provider))
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

    /// Returns `true` when either provider registry contains `key`.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.option_providers.contains_key(key) || self.dynamic_record_providers.contains_key(key)
    }

    /// Returns the number of registered option providers.
    #[must_use]
    pub fn option_provider_count(&self) -> usize {
        self.option_providers.len()
    }

    /// Returns the number of registered dynamic-record providers.
    #[must_use]
    pub fn dynamic_record_provider_count(&self) -> usize {
        self.dynamic_record_providers.len()
    }

    /// Returns a sorted snapshot of registered option provider keys.
    #[must_use]
    pub fn option_provider_keys(&self) -> Vec<&str> {
        let mut keys = self
            .option_providers
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys
    }

    /// Returns a sorted snapshot of registered dynamic-record provider keys.
    #[must_use]
    pub fn dynamic_record_provider_keys(&self) -> Vec<&str> {
        let mut keys = self
            .dynamic_record_providers
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys
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
        let envelope = provider.resolve(request).await?;
        validate_envelope(key, &envelope, DynamicResponseKind::Options)?;
        Ok(envelope)
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
        let envelope = provider.resolve_fields(request).await?;
        validate_envelope(key, &envelope, DynamicResponseKind::Fields)?;
        Ok(envelope)
    }
}

fn validate_envelope<T>(
    key: &str,
    envelope: &DynamicProviderEnvelope<T>,
    expected_kind: DynamicResponseKind,
) -> Result<(), ProviderError> {
    if envelope.kind != expected_kind {
        return Err(ProviderError::KindMismatch {
            key: key.to_owned(),
            expected: expected_kind,
            actual: envelope.kind,
        });
    }

    if envelope.response_version != DYNAMIC_PROVIDER_RESPONSE_VERSION {
        return Err(ProviderError::UnsupportedVersion {
            key: key.to_owned(),
            version: envelope.response_version,
            expected: DYNAMIC_PROVIDER_RESPONSE_VERSION,
        });
    }

    Ok(())
}

fn validate_provider_key(key: &str) -> Result<(), ProviderError> {
    if key.trim().is_empty() {
        return Err(ProviderError::InvalidKey {
            key: key.to_owned(),
            reason: "key must not be empty or whitespace".to_owned(),
        });
    }

    if key.trim() != key {
        return Err(ProviderError::InvalidKey {
            key: key.to_owned(),
            reason: "key must not have leading or trailing whitespace".to_owned(),
        });
    }

    if !key
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(ProviderError::InvalidKey {
            key: key.to_owned(),
            reason: "key must use lowercase ascii letters, digits, '.', '_' or '-'".to_owned(),
        });
    }

    let first = key.chars().next();
    let last = key.chars().next_back();
    if matches!(first, Some('.' | '_' | '-')) || matches!(last, Some('.' | '_' | '-')) {
        return Err(ProviderError::InvalidKey {
            key: key.to_owned(),
            reason: "key must not start or end with '.', '_' or '-'".to_owned(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct NoopOptionProvider;

    #[async_trait]
    impl OptionProvider for NoopOptionProvider {
        async fn resolve(
            &self,
            _request: &ProviderRequest,
        ) -> Result<DynamicProviderEnvelope<SelectOption>, ProviderError> {
            Ok(DynamicProviderEnvelope::new(
                DynamicResponseKind::Options,
                Vec::new(),
            ))
        }
    }

    struct NoopDynamicProvider;

    #[async_trait]
    impl DynamicRecordProvider for NoopDynamicProvider {
        async fn resolve_fields(
            &self,
            _request: &ProviderRequest,
        ) -> Result<DynamicProviderEnvelope<FieldSpec>, ProviderError> {
            Ok(DynamicProviderEnvelope::new(
                DynamicResponseKind::Fields,
                Vec::new(),
            ))
        }
    }

    #[test]
    fn register_rejects_duplicate_key() {
        let mut registry = ProviderRegistry::new();
        let first = Arc::new(NoopOptionProvider);
        let second = Arc::new(NoopOptionProvider);

        registry
            .register_option_provider("catalog.regions", first)
            .expect("first registration must succeed");
        let err = registry
            .register_option_provider("catalog.regions", second)
            .expect_err("duplicate key must fail");

        assert!(matches!(err, ProviderError::AlreadyRegistered { .. }));
    }

    #[test]
    fn upsert_replaces_existing_provider() {
        let mut registry = ProviderRegistry::new();
        registry
            .register_dynamic_record_provider("sheet.columns", Arc::new(NoopDynamicProvider))
            .expect("initial registration must succeed");

        let replaced = registry
            .upsert_dynamic_record_provider("sheet.columns", Arc::new(NoopDynamicProvider))
            .expect("upsert should succeed");
        assert!(replaced.is_some());
    }

    #[test]
    fn key_validation_enforces_format() {
        let mut registry = ProviderRegistry::new();

        let err = registry
            .register_option_provider("Bad Key", Arc::new(NoopOptionProvider))
            .expect_err("uppercase/space key must fail");
        assert!(matches!(err, ProviderError::InvalidKey { .. }));

        let err = registry
            .register_option_provider(".bad", Arc::new(NoopOptionProvider))
            .expect_err("leading separator key must fail");
        assert!(matches!(err, ProviderError::InvalidKey { .. }));
    }
}
