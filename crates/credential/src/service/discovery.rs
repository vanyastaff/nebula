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
    /// capability bitflag.
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
                interactive: capabilities.contains(crate::Capabilities::INTERACTIVE),
                dynamic: capabilities.contains(crate::Capabilities::DYNAMIC),
                refreshable: capabilities.contains(crate::Capabilities::REFRESHABLE),
                testable: capabilities.contains(crate::Capabilities::TESTABLE),
                revocable: capabilities.contains(crate::Capabilities::REVOCABLE),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CredentialService;
    use crate::{ApiKeyCredential, Capabilities, Credential, CredentialRegistry};

    #[test]
    fn discovery_preserves_all_five_registry_capabilities() {
        let mut registry = CredentialRegistry::new();
        registry
            .register(ApiKeyCredential, "discovery-projection-test")
            .expect("valid credential");
        let metadata = registry
            .metadata(ApiKeyCredential::KEY)
            .expect("registered metadata");
        for capabilities in [
            Capabilities::empty(),
            Capabilities::all(),
            Capabilities::INTERACTIVE | Capabilities::DYNAMIC,
        ] {
            let projection = CredentialService::type_info(metadata, capabilities);
            assert_eq!(projection.pattern, metadata.pattern());
            assert_eq!(
                projection.capabilities.interactive,
                capabilities.contains(Capabilities::INTERACTIVE)
            );
            assert_eq!(
                projection.capabilities.dynamic,
                capabilities.contains(Capabilities::DYNAMIC)
            );
            assert_eq!(
                projection.capabilities.refreshable,
                capabilities.contains(Capabilities::REFRESHABLE)
            );
            assert_eq!(
                projection.capabilities.testable,
                capabilities.contains(Capabilities::TESTABLE)
            );
            assert_eq!(
                projection.capabilities.revocable,
                capabilities.contains(Capabilities::REVOCABLE)
            );
        }
    }
}
