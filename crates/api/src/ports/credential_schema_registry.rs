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
use nebula_credential::{AnyCredential, Capabilities, CredentialRegistry};
#[cfg(any(test, feature = "test-util"))]
use nebula_credential::{ApiKeyCredential, BasicAuthCredential, SigningKeyCredential};
use nebula_schema::ValidSchema;

/// `CredentialSchemaPort` backed by a registered credential set.
pub struct RegistryCredentialSchema {
    registry: Arc<CredentialRegistry>,
}

impl RegistryCredentialSchema {
    /// Wrap a populated registry.
    #[must_use]
    pub fn new(registry: Arc<CredentialRegistry>) -> Self {
        Self { registry }
    }

    fn flags(caps: Capabilities) -> CredentialCapabilityFlags {
        CredentialCapabilityFlags {
            interactive: caps.contains(Capabilities::INTERACTIVE),
            refreshable: caps.contains(Capabilities::REFRESHABLE),
            testable: caps.contains(Capabilities::TESTABLE),
            revocable: caps.contains(Capabilities::REVOCABLE),
        }
    }

    fn descriptor(&self, any: &dyn AnyCredential) -> CredentialTypeDescriptor {
        let meta = any.metadata();
        let key = any.credential_key().to_owned();
        // Structural JSON-Schema export. On failure, an empty object
        // schema (never a panic); the api-owned public projection still
        // strips predicate operands at the wire (#6).
        let schema_json = export_schema(&meta.base.schema);
        let caps = self
            .registry
            .capabilities_of(&key)
            .unwrap_or_else(Capabilities::empty);
        CredentialTypeDescriptor {
            key,
            name: meta.base.name.clone(),
            description: meta.base.description.clone(),
            auth_pattern: format!("{:?}", meta.pattern),
            capabilities: Self::flags(caps),
            icon: meta.base.icon.as_inline().map(str::to_owned),
            documentation_url: meta.base.documentation_url,
            schema_json,
        }
    }
}

fn export_schema(schema: &ValidSchema) -> serde_json::Value {
    schema
        .json_schema()
        .ok()
        .and_then(|exported| serde_json::to_value(&exported).ok())
        .unwrap_or_else(|| serde_json::json!({ "type": "object" }))
}

impl CredentialSchemaPort for RegistryCredentialSchema {
    fn list_types(&self) -> Vec<CredentialTypeDescriptor> {
        // `iter_compatible(empty)` enumerates every registered type
        // (registry.rs:212 — empty is a subset of any capability set).
        self.registry
            .iter_compatible(Capabilities::empty())
            .filter_map(|(k, _caps)| self.registry.resolve_any(k).map(|any| self.descriptor(any)))
            .collect()
    }

    fn get_type(&self, credential_key: &str) -> Option<CredentialTypeDescriptor> {
        self.registry
            .resolve_any(credential_key)
            .map(|any| self.descriptor(any))
    }
}

/// Build the first-party catalog registry for API reference/test composition.
/// Production creates one shared registry for runtime and catalog inside its
/// apps-owned composition root.
///
/// # Errors
///
/// Returns [`nebula_credential::RegisterError`] if a credential KEY is
/// already registered (a composition bug — distinct first-party const
/// KEYs make this unreachable in practice, but the library returns the
/// typed error rather than panicking; the caller decides how to surface
/// it — AGENTS.md "no `expect()` in library code").
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
/// Returns [`nebula_credential::RegisterError`] if a credential KEY is
/// already registered (a composition bug — distinct first-party const
/// KEYs make this unreachable in practice, but the library returns the
/// typed error rather than panicking; the caller decides how to surface
/// it — AGENTS.md "no `expect()` in library code").
#[cfg(any(test, feature = "test-util"))]
pub fn try_default_registry_port()
-> Result<Arc<dyn CredentialSchemaPort>, nebula_credential::RegisterError> {
    Ok(Arc::new(RegistryCredentialSchema::new(Arc::new(
        default_registry()?,
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port() -> RegistryCredentialSchema {
        let mut reg = CredentialRegistry::new();
        reg.register(ApiKeyCredential, "nebula-credential")
            .expect("api_key registers (statically unique key)");
        RegistryCredentialSchema::new(Arc::new(reg))
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
            "oauth2 remains implemented in nebula-credential but must stay out of the default API catalog until universal pending acquisition is wired"
        );
    }
}
