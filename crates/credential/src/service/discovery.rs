//! Type-discovery surface of [`CredentialService`] — secret-free
//! descriptors of registered credential types for pickers / UIs.
//!
//! Split out of `facade.rs` (behaviour-preserving code motion — no logic
//! change). Reads only the `pub(crate)` [`CredentialRegistry`] field.

use super::facade::{CredentialService, CredentialTypeInfo, TypeCapabilities};

impl CredentialService {
    /// List every registered credential type as a secret-free
    /// descriptor. Capability flags come from the [`CredentialRegistry`]
    /// bitflag (computed from sub-trait membership at registration), not
    /// self-attested metadata.
    ///
    /// [`CredentialRegistry`]: crate::CredentialRegistry
    #[must_use]
    pub fn list_types(&self) -> Vec<CredentialTypeInfo> {
        self.registry
            .iter_compatible(crate::Capabilities::empty())
            .filter_map(|(key, _caps)| self.type_info(key))
            .collect()
    }

    /// Project a single credential type's descriptor, or `None` when the
    /// key is not registered.
    #[must_use]
    pub fn get_type(&self, key: &str) -> Option<CredentialTypeInfo> {
        if !self.registry.contains(key) {
            return None;
        }
        self.type_info(key)
    }

    /// Build a [`CredentialTypeInfo`] from the registry metadata +
    /// capability bitflag. Returns `None` if the registry has no
    /// instance for `key` (cannot project metadata).
    fn type_info(&self, key: &str) -> Option<CredentialTypeInfo> {
        let metadata = self.registry.resolve_any(key)?.metadata();
        Some(CredentialTypeInfo {
            key: metadata.base.key.as_str().to_owned(),
            name: metadata.base.name.clone(),
            description: metadata.base.description.clone(),
            pattern: metadata.pattern,
            capabilities: TypeCapabilities {
                refreshable: self.registry.is_refreshable(key),
                testable: self.registry.is_testable(key),
                revocable: self.registry.is_revocable(key),
            },
        })
    }
}
