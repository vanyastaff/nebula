//! Test-only credential catalog/form read model over a
//! `nebula_credential::CredentialRegistry`.
//!
//! Production selection and its concrete adapter live in `apps/server`; this
//! module is compiled only with unsupported `test-util`. Mutation validation
//! is intentionally absent from this port and remains canonical inside
//! `CredentialService` after the command authority decision.

use std::sync::Arc;

use crate::ports::credential_schema::{
    CredentialCapabilityFlags, CredentialSchemaPort, CredentialTypeDescriptor,
};
#[cfg(any(test, feature = "test-util"))]
use nebula_credential::{ApiKeyCredential, BasicAuthCredential, SigningKeyCredential};
use nebula_credential::{Capabilities, CredentialMetadata, CredentialRegistry};
use nebula_schema::JsonSchemaExportError;

/// Complete catalog snapshot exported from a registered credential set.
pub struct RegistryCredentialSchema {
    // Keep the source immutable for as long as its exported snapshot is used.
    _registry: Arc<CredentialRegistry>,
    descriptors: Vec<CredentialTypeDescriptor>,
}

impl RegistryCredentialSchema {
    /// Export every admitted definition before making the catalog available.
    ///
    /// # Errors
    /// Returns [`JsonSchemaExportError`] if any schema fails to export. No
    /// partial catalog or permissive replacement schema is published.
    #[tracing::instrument(name = "api.credential_catalog.build", skip_all)]
    pub fn new(registry: Arc<CredentialRegistry>) -> Result<Self, JsonSchemaExportError> {
        let descriptors = registry
            .catalog()
            .map(|(metadata, capabilities)| Self::descriptor(metadata, capabilities))
            .collect::<Result<Vec<_>, _>>()
            .inspect_err(|_| tracing::error!("credential catalog schema export failed"))?;
        Ok(Self {
            _registry: registry,
            descriptors,
        })
    }

    fn flags(caps: Capabilities) -> CredentialCapabilityFlags {
        CredentialCapabilityFlags {
            interactive: caps.contains(Capabilities::INTERACTIVE),
            refreshable: caps.contains(Capabilities::REFRESHABLE),
            testable: caps.contains(Capabilities::TESTABLE),
            revocable: caps.contains(Capabilities::REVOCABLE),
        }
    }

    fn descriptor(
        meta: &CredentialMetadata,
        caps: Capabilities,
    ) -> Result<CredentialTypeDescriptor, JsonSchemaExportError> {
        let key = meta.key().as_str().to_owned();
        let schema_json = meta.schema().json_schema()?.to_value();
        Ok(CredentialTypeDescriptor {
            key,
            name: meta.name().to_owned(),
            description: meta.description().to_owned(),
            auth_pattern: format!("{:?}", meta.pattern()),
            capabilities: Self::flags(caps),
            icon: meta.icon().as_inline().map(str::to_owned),
            documentation_url: meta.documentation_url().map(str::to_owned),
            schema_json,
        })
    }
}

impl CredentialSchemaPort for RegistryCredentialSchema {
    fn list_types(&self) -> Vec<CredentialTypeDescriptor> {
        self.descriptors.clone()
    }

    fn get_type(&self, credential_key: &str) -> Option<CredentialTypeDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.key == credential_key)
            .cloned()
    }
}

/// Failure to construct the complete reference credential catalog.
#[derive(Debug, thiserror::Error)]
pub enum CredentialCatalogBuildError {
    /// A credential definition failed registry admission.
    #[error("credential registry registration failed")]
    Registry(#[from] nebula_credential::RegisterError),
    /// An admitted credential schema could not be exported.
    #[error("credential catalog schema export failed")]
    SchemaExport(#[from] JsonSchemaExportError),
}

/// Build the first-party catalog registry for API reference/test composition.
/// Production creates one shared registry for runtime and catalog inside its
/// apps-owned composition root.
///
/// # Errors
///
/// Returns [`nebula_credential::RegisterError`] if definition admission fails
/// or a credential key is already registered.
#[cfg(any(test, feature = "test-util"))]
pub(crate) fn default_registry() -> Result<CredentialRegistry, nebula_credential::RegisterError> {
    let mut registry = CredentialRegistry::new();
    registry.register(ApiKeyCredential, "nebula-credential")?;
    registry.register(BasicAuthCredential, "nebula-credential")?;
    // signing_key: static non-interactive credential used for webhook HMAC
    // secrets (Standard Webhooks `whsec_` format).
    registry.register(SigningKeyCredential, "nebula-credential")?;
    Ok(registry)
}

/// Build the default test catalog with the first-party credential types.
///
/// # Errors
///
/// Returns [`CredentialCatalogBuildError`] if registration or schema export
/// fails. A catalog is returned only when every definition is exported.
#[cfg(any(test, feature = "test-util"))]
pub fn try_default_registry_port()
-> Result<Arc<dyn CredentialSchemaPort>, CredentialCatalogBuildError> {
    Ok(Arc::new(RegistryCredentialSchema::new(Arc::new(
        default_registry()?,
    ))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port() -> RegistryCredentialSchema {
        let mut reg = CredentialRegistry::new();
        reg.register(ApiKeyCredential, "nebula-credential")
            .expect("api_key registers (statically unique key)");
        RegistryCredentialSchema::new(Arc::new(reg)).expect("registered catalog exports")
    }

    #[test]
    fn get_type_exports_capable_descriptor_and_default_port_lists_first_party() {
        let p = port();
        let d = p.get_type("api_key").expect("api_key present");
        assert_eq!(d.key, "api_key");
        assert!(
            d.schema_json.get("properties").is_some(),
            "json_schema() export must carry properties: {:?}",
            d.schema_json
        );
        assert!(p.get_type("nope").is_none());

        // The composition default registers the curated first-party static
        // set; parked implementations are deliberately absent.
        let default = try_default_registry_port().expect("first-party set registers (unique KEYs)");
        let listed = default.list_types();
        for k in ["api_key", "basic_auth", "signing_key"] {
            assert!(
                listed.iter().any(|t| t.key == k),
                "default port must register {k}; got {:?}",
                listed.iter().map(|t| &t.key).collect::<Vec<_>>()
            );
        }
        assert!(
            listed.iter().all(|t| t.key != "oauth2"),
            "oauth2 requires explicit provider configuration and a production acquisition transport"
        );
    }

    #[test]
    fn catalog_snapshot_preserves_every_registered_definition() {
        let registry = Arc::new(default_registry().expect("first-party definitions register"));
        let port = RegistryCredentialSchema::new(Arc::clone(&registry))
            .expect("all registered schemas export");
        let listed = port.list_types();
        assert_eq!(listed.len(), registry.catalog().count());
        for (metadata, capabilities) in registry.catalog() {
            let descriptor = port
                .get_type(metadata.key().as_str())
                .expect("catalog snapshot contains every admitted definition");
            assert_eq!(descriptor.name, metadata.name());
            assert_eq!(
                descriptor.schema_json,
                metadata
                    .schema()
                    .json_schema()
                    .expect("admitted schema exports")
                    .to_value()
            );
            assert_eq!(
                descriptor.capabilities.testable,
                capabilities.contains(Capabilities::TESTABLE)
            );
            assert!(listed.iter().any(|listed| listed.key == descriptor.key));
        }
    }

    #[test]
    fn catalog_keeps_its_source_registry_immutable() {
        let mut registry = Arc::new(default_registry().expect("first-party definitions register"));
        let port = RegistryCredentialSchema::new(Arc::clone(&registry))
            .expect("all registered schemas export");
        assert!(
            Arc::get_mut(&mut registry).is_none(),
            "a live catalog must prevent mutation of its source registry"
        );
        drop(port);
        assert!(Arc::get_mut(&mut registry).is_some());
    }
}
