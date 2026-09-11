//! Type-discovery surface of [`CredentialService`] — secret-free
//! descriptors of registered credential types for pickers / UIs.
//!
//! Split out of `facade.rs` (behaviour-preserving code motion — no logic
//! change). Reads only the `pub(crate)` [`CredentialRegistry`](crate::CredentialRegistry) field.

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
            .catalog()
            .map(|(metadata, capabilities)| Self::type_info(metadata, capabilities))
            .collect()
    }

    /// Project a single credential type's descriptor, or `None` when the
    /// key is not registered.
    #[must_use]
    pub fn get_type(&self, key: &str) -> Option<CredentialTypeInfo> {
        Some(Self::type_info(
            self.registry.metadata(key)?,
            self.registry.capabilities_of(key)?,
        ))
    }

    /// Build a [`CredentialTypeInfo`] from the registry metadata +
    /// capability bitflag. Returns `None` if the registry has no
    /// instance for `key` (cannot project metadata).
    fn type_info(
        metadata: &crate::CredentialMetadata,
        capabilities: crate::Capabilities,
    ) -> CredentialTypeInfo {
        CredentialTypeInfo {
            key: metadata.key().as_str().to_owned(),
            name: metadata.name().to_owned(),
            description: metadata.description().to_owned(),
            pattern: metadata.pattern(),
            capabilities: TypeCapabilities {
                refreshable: capabilities.contains(crate::Capabilities::REFRESHABLE),
                testable: capabilities.contains(crate::Capabilities::TESTABLE),
                revocable: capabilities.contains(crate::Capabilities::REVOCABLE),
            },
        }
    }
}
