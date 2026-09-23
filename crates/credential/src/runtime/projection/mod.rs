//! Read/project-only credential runtime for execution workers.

use std::{fmt, future::Future, pin::Pin, sync::Arc};

use tokio_util::sync::CancellationToken;

use crate::{
    Capabilities, CredentialId, CredentialKey, CredentialPersistence, CredentialRegistry,
    DispatchOps, ErasedPendingStore, StateSource,
};

use crate::TenantScope;
pub(crate) mod slot;

pub use slot::{
    CredentialGuardMetadata, CredentialSlotResolveError, CredentialSlotResolver,
    ErasedCredentialGuard, ErasedCredentialGuardTypeError,
};
use slot::{SlotResolutionRequest, resolve_slot_with};

/// Construction failure for [`CredentialProjectionRuntime`].
///
/// Variants intentionally carry no registry keys, provider names, or backend
/// diagnostics. Composition code can identify its static registration site,
/// while traces emitted before return retain the credential key needed by an
/// operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialProjectionRuntimeBuildError {
    /// A registered credential type has no base projection operation.
    #[error("credential projection operations are incomplete")]
    ProjectionOpsMissing,
    /// A registered credential advertises capabilities without matching ops.
    #[error("credential capability operations are incomplete")]
    CapabilityOpsMissing,
    /// Dynamic credentials need lease lifecycle authority that projection-only workers do not own.
    #[error("dynamic credentials are not supported by the projection runtime")]
    DynamicUnsupported,
}

/// Read/project-only credential runtime for execution composition roots.
///
/// This runtime owns no refresh coordinator, lease lifecycle, reclaim sweep,
/// pending acquisition store, observer, or management command authority. Its
/// persistence input must already be the secure layered
/// `Audit(Encryption(raw))` store assembled by the composition root.
pub struct CredentialProjectionRuntime {
    store: Arc<dyn CredentialPersistence>,
    registry: Arc<CredentialRegistry>,
    ops: Arc<DispatchOps<ErasedPendingStore>>,
    source: StateSource,
}

impl CredentialProjectionRuntime {
    /// Construct a projection runtime from pre-secured collaborators.
    ///
    /// The registry must have a base projector and matching operation closure
    /// for every capability it advertises. In particular, `DYNAMIC` is rejected
    /// because this runtime deliberately has no lease lifecycle.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialProjectionRuntimeBuildError::ProjectionOpsMissing`]
    /// when a registry key has no base projector, or
    /// [`CredentialProjectionRuntimeBuildError::CapabilityOpsMissing`] when
    /// any advertised capability lacks a matching operation closure. Dynamic
    /// credentials return [`CredentialProjectionRuntimeBuildError::DynamicUnsupported`].
    pub fn from_secure_parts(
        store: Arc<dyn CredentialPersistence>,
        registry: Arc<CredentialRegistry>,
        ops: Arc<DispatchOps<ErasedPendingStore>>,
        source: StateSource,
    ) -> Result<Self, CredentialProjectionRuntimeBuildError> {
        validate_projection_parts(registry.as_ref(), ops.as_ref())?;
        Ok(Self {
            store,
            registry,
            ops,
            source,
        })
    }
}

impl fmt::Debug for CredentialProjectionRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialProjectionRuntime([redacted])")
    }
}

impl CredentialSlotResolver for CredentialProjectionRuntime {
    fn resolve_slot<'a>(
        &'a self,
        scope: &'a TenantScope,
        credential_id: CredentialId,
        expected_key: CredentialKey,
        required_capabilities: Capabilities,
        cancel: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ErasedCredentialGuard, CredentialSlotResolveError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(resolve_slot_with(
            self.store.as_ref(),
            self.registry.as_ref(),
            self.ops.as_ref(),
            &self.source,
            SlotResolutionRequest {
                scope,
                credential_id,
                expected_key,
                required_capabilities,
                cancel,
            },
        ))
    }
}

fn validate_projection_parts(
    registry: &CredentialRegistry,
    ops: &DispatchOps<ErasedPendingStore>,
) -> Result<(), CredentialProjectionRuntimeBuildError> {
    for key in registry.iter_keys() {
        if !ops.contains(key) {
            tracing::error!(
                credential.key = key,
                "credential projection runtime rejected missing base ops"
            );
            return Err(CredentialProjectionRuntimeBuildError::ProjectionOpsMissing);
        }

        let advertised = registry.capabilities_of(key).unwrap_or_default();
        if advertised.contains(Capabilities::DYNAMIC) {
            tracing::error!(
                credential.key = key,
                "credential projection runtime rejected dynamic credential"
            );
            return Err(CredentialProjectionRuntimeBuildError::DynamicUnsupported);
        }
        let missing = advertised.difference(ops.capabilities_of(key));
        if !missing.is_empty() {
            tracing::error!(
                credential.key = key,
                ?missing,
                "credential projection runtime rejected missing capability ops"
            );
            return Err(CredentialProjectionRuntimeBuildError::CapabilityOpsMissing);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BearerTokenCredential, BearerTokenProperties, Credential, CredentialContext,
        CredentialError, CredentialMetadataDraft, Dynamic, OAuth2Credential, SecretToken,
        contract::plugin_capability_report, metadata_name, register_refreshable_ops,
        register_runtime_ops, resolve::StaticResolveResult,
    };
    use nebula_core::credential_key;

    struct DynamicFixtureCredential;

    impl Credential for DynamicFixtureCredential {
        type Properties = BearerTokenProperties;
        type Scheme = SecretToken;
        type State = SecretToken;

        const KEY: &'static str = "projection_dynamic_fixture";

        fn metadata() -> CredentialMetadataDraft {
            CredentialMetadataDraft::new(
                credential_key!("projection_dynamic_fixture"),
                metadata_name!("Projection Dynamic Fixture"),
                "Projection-only runtime dynamic rejection fixture.",
            )
        }

        fn project(state: &Self::State) -> Self::Scheme {
            state.clone()
        }

        async fn resolve(
            properties: &Self::Properties,
            _ctx: &CredentialContext,
        ) -> Result<StaticResolveResult<Self::State>, CredentialError> {
            Ok(StaticResolveResult::Complete(SecretToken::new(
                properties.token.clone(),
            )))
        }
    }

    impl Dynamic for DynamicFixtureCredential {
        async fn release(
            _state: &Self::State,
            _ctx: &CredentialContext,
        ) -> Result<(), CredentialError> {
            Ok(())
        }
    }

    impl plugin_capability_report::IsInteractive for DynamicFixtureCredential {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsRefreshable for DynamicFixtureCredential {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsRevocable for DynamicFixtureCredential {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsTestable for DynamicFixtureCredential {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsDynamic for DynamicFixtureCredential {
        const VALUE: bool = true;
    }

    #[test]
    fn accepts_complete_static_projection_registration() {
        let mut registry = CredentialRegistry::new();
        registry
            .register(BearerTokenCredential, "projection-test")
            .expect("fixture registry key is unique");
        let mut ops = DispatchOps::new();
        register_runtime_ops::<BearerTokenCredential, ErasedPendingStore>(&mut ops)
            .expect("fixture ops key is unique");

        assert_eq!(validate_projection_parts(&registry, &ops), Ok(()));
    }

    #[test]
    fn rejects_registry_entry_without_projection_ops() {
        let mut registry = CredentialRegistry::new();
        registry
            .register(BearerTokenCredential, "projection-test")
            .expect("fixture registry key is unique");
        let ops = DispatchOps::new();

        assert_eq!(
            validate_projection_parts(&registry, &ops),
            Err(CredentialProjectionRuntimeBuildError::ProjectionOpsMissing)
        );
    }

    #[test]
    fn rejects_advertised_capability_without_matching_ops() {
        let mut registry = CredentialRegistry::new();
        registry
            .register(OAuth2Credential, "projection-test")
            .expect("fixture registry key is unique");
        let mut ops = DispatchOps::new();
        register_runtime_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)
            .expect("fixture base ops key is unique");
        // Deliberately omit every capability registrar. Adding only refresh
        // would still leave OAuth2's interactive/revoke/test capabilities
        // incomplete and must remain rejected.

        assert_eq!(
            validate_projection_parts(&registry, &ops),
            Err(CredentialProjectionRuntimeBuildError::CapabilityOpsMissing)
        );

        register_refreshable_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)
            .expect("fixture refresh ops attach to the base entry");
        assert_eq!(
            validate_projection_parts(&registry, &ops),
            Err(CredentialProjectionRuntimeBuildError::CapabilityOpsMissing)
        );
    }

    #[test]
    fn rejects_dynamic_credentials_even_with_base_projection_ops() {
        let mut registry = CredentialRegistry::new();
        registry
            .register(DynamicFixtureCredential, "projection-test")
            .expect("fixture registry key is unique");
        let mut ops = DispatchOps::new();
        register_runtime_ops::<DynamicFixtureCredential, ErasedPendingStore>(&mut ops)
            .expect("fixture base ops key is unique");

        assert_eq!(
            validate_projection_parts(&registry, &ops),
            Err(CredentialProjectionRuntimeBuildError::DynamicUnsupported)
        );
    }

    #[test]
    fn build_error_debug_and_display_are_payload_free() {
        let errors = [
            CredentialProjectionRuntimeBuildError::ProjectionOpsMissing,
            CredentialProjectionRuntimeBuildError::CapabilityOpsMissing,
            CredentialProjectionRuntimeBuildError::DynamicUnsupported,
        ];
        for error in errors {
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains("credential.key"));
            assert!(!rendered.contains("provider"));
            assert!(!rendered.contains("secret"));
        }
    }

    #[test]
    fn build_error_debug_retains_only_the_typed_variant() {
        assert_eq!(
            format!(
                "{:?}",
                CredentialProjectionRuntimeBuildError::ProjectionOpsMissing
            ),
            "ProjectionOpsMissing"
        );
    }
}
