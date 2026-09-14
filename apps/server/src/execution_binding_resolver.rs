//! Production resolution of authority-free executable-plan bindings.

use std::{fmt, sync::Arc};

use nebula_core::{CredentialId, ResourceId};
use nebula_credential::{Capabilities, CredentialHead, CredentialService, CredentialServiceError};
use nebula_engine::{BindingResolutionError, BindingResolutionFuture, ExecutionBindingResolver};
use nebula_execution::{
    BindingContractVersion, CredentialBindingContractV2, CredentialCapability,
    ExecutionBindingEntryV2, ExecutionBindingManifestV2, ExecutionBindingSiteV2,
    ExecutionBindingTargetV2, ResourceBindingContractV2,
};
use nebula_metadata::Metadata;
use nebula_plugin::{
    FrozenPluginRegistry, PlanBindingContract, PlanBindingRequirement,
    PlanBindingSelectorProvenance, PlanBindingSite,
};
use nebula_storage_port::{Scope, StorageError, dto::ResourceRow, store::ResourceStore};

/// Deployment-owned resolver backed by tenant-scoped management stores.
pub(crate) struct ServerExecutionBindingResolver {
    credentials: Arc<CredentialService>,
    resources: Arc<dyn ResourceStore>,
    registry: Arc<FrozenPluginRegistry>,
}

impl ServerExecutionBindingResolver {
    pub(crate) fn new(
        credentials: Arc<CredentialService>,
        resources: Arc<dyn ResourceStore>,
        registry: Arc<FrozenPluginRegistry>,
    ) -> Self {
        Self {
            credentials,
            resources,
            registry,
        }
    }

    #[tracing::instrument(
        name = "execution_bindings.resolve",
        skip_all,
        fields(binding_count = requirements.len(), outcome = tracing::field::Empty)
    )]
    async fn resolve_all(
        &self,
        scope: &Scope,
        requirements: &[PlanBindingRequirement],
    ) -> Result<ExecutionBindingManifestV2, BindingResolutionError> {
        let mut entries = Vec::with_capacity(requirements.len());
        for requirement in requirements {
            match self.resolve_one(scope, requirement).await {
                Ok(entry) => entries.push(entry),
                Err(BindingResolutionError::NotFound) if !requirement.required() => {},
                Err(error) => {
                    tracing::Span::current().record("outcome", "rejected");
                    return Err(error);
                },
            }
        }

        let manifest = ExecutionBindingManifestV2::new(entries)
            .map_err(|_| BindingResolutionError::InvalidManifest)?;
        tracing::Span::current().record("outcome", "resolved");
        Ok(manifest)
    }

    async fn resolve_one(
        &self,
        scope: &Scope,
        requirement: &PlanBindingRequirement,
    ) -> Result<ExecutionBindingEntryV2, BindingResolutionError> {
        let target = match requirement.contract() {
            PlanBindingContract::Credential {
                key,
                version,
                required_capabilities,
                ..
            } => {
                let selected = self
                    .select_credential(
                        scope,
                        requirement.selector(),
                        requirement.selector_provenance(),
                    )
                    .await?;
                let registered = self
                    .registry
                    .resolve_credential(key)
                    .ok_or(BindingResolutionError::Incompatible)?;
                let registered_metadata = registered
                    .metadata()
                    .map_err(|_| BindingResolutionError::Incompatible)?;
                if selected.credential_key != key.as_str()
                    || registered_metadata.base().version() != version
                    || !registered.capabilities().contains(*required_capabilities)
                {
                    return Err(BindingResolutionError::Incompatible);
                }

                let credential_id = CredentialId::parse(&selected.id)
                    .map_err(|_| BindingResolutionError::InvalidManifest)?;
                let contract_version = BindingContractVersion::parse(&version.to_string())
                    .map_err(|_| BindingResolutionError::InvalidManifest)?;
                ExecutionBindingTargetV2::Credential {
                    credential_id,
                    contract: CredentialBindingContractV2::new(
                        key.clone(),
                        contract_version,
                        credential_capabilities(*required_capabilities),
                    ),
                }
            },
            PlanBindingContract::Resource { key, version, .. } => {
                let selected = self
                    .select_resource(
                        scope,
                        requirement.selector(),
                        requirement.selector_provenance(),
                    )
                    .await?;
                let registered = self
                    .registry
                    .resolve_resource(key)
                    .ok_or(BindingResolutionError::Incompatible)?;
                let registered_metadata = registered
                    .metadata()
                    .map_err(|_| BindingResolutionError::Incompatible)?;
                if selected.kind != key.as_str() || registered_metadata.base().version() != version
                {
                    return Err(BindingResolutionError::Incompatible);
                }

                let resource_id = ResourceId::parse(&selected.id)
                    .map_err(|_| BindingResolutionError::InvalidManifest)?;
                ExecutionBindingTargetV2::Resource {
                    resource_id,
                    contract: ResourceBindingContractV2::new(
                        key.clone(),
                        BindingContractVersion::parse(&version.to_string())
                            .map_err(|_| BindingResolutionError::InvalidManifest)?,
                    ),
                }
            },
            _ => return Err(BindingResolutionError::InvalidManifest),
        };

        ExecutionBindingEntryV2::new(
            binding_site(requirement.site())?,
            requirement.slot_key(),
            target,
        )
        .map_err(|_| BindingResolutionError::InvalidManifest)
    }

    async fn select_credential(
        &self,
        scope: &Scope,
        selector: &str,
        provenance: PlanBindingSelectorProvenance,
    ) -> Result<CredentialHead, BindingResolutionError> {
        let credential_scope = nebula_credential::TenantScope::from_scope(scope);
        match provenance {
            PlanBindingSelectorProvenance::DefaultName => {
                let heads = self
                    .credentials
                    .list(&credential_scope)
                    .await
                    .map_err(map_credential_error)?;
                select_unique(heads, |head| {
                    credential_name_matches(head.display.display_name.as_deref(), selector)
                })
            },
            PlanBindingSelectorProvenance::CredentialIdOverride => {
                let credential_id =
                    CredentialId::parse(selector).map_err(|_| BindingResolutionError::NotFound)?;
                self.credentials
                    .get(&credential_scope, &credential_id.to_string())
                    .await
                    .map_err(map_credential_error)
            },
            PlanBindingSelectorProvenance::Legacy
            | PlanBindingSelectorProvenance::ResourceIdOverride => {
                Err(BindingResolutionError::InvalidManifest)
            },
            _ => Err(BindingResolutionError::InvalidManifest),
        }
    }

    async fn select_resource(
        &self,
        scope: &Scope,
        selector: &str,
        provenance: PlanBindingSelectorProvenance,
    ) -> Result<ResourceRow, BindingResolutionError> {
        match provenance {
            PlanBindingSelectorProvenance::DefaultName => {
                let rows = self
                    .resources
                    .list(scope)
                    .await
                    .map_err(map_resource_error)?;
                select_unique(
                    rows.into_iter()
                        .filter(|row| resource_is_visible(row, scope)),
                    |row| resource_name_matches(row, selector),
                )
            },
            PlanBindingSelectorProvenance::ResourceIdOverride => {
                let resource_id =
                    ResourceId::parse(selector).map_err(|_| BindingResolutionError::NotFound)?;
                let selected = self
                    .resources
                    .get(scope, &resource_id.to_string())
                    .await
                    .map_err(map_resource_error)?;
                selected
                    .filter(|row| resource_is_visible(row, scope))
                    .ok_or(BindingResolutionError::NotFound)
            },
            PlanBindingSelectorProvenance::Legacy
            | PlanBindingSelectorProvenance::CredentialIdOverride => {
                Err(BindingResolutionError::InvalidManifest)
            },
            _ => Err(BindingResolutionError::InvalidManifest),
        }
    }
}

impl fmt::Debug for ServerExecutionBindingResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ServerExecutionBindingResolver")
    }
}

impl ExecutionBindingResolver for ServerExecutionBindingResolver {
    fn resolve<'a>(
        &'a self,
        scope: &'a Scope,
        requirements: &'a [PlanBindingRequirement],
    ) -> BindingResolutionFuture<'a> {
        Box::pin(self.resolve_all(scope, requirements))
    }
}

fn binding_site(site: &PlanBindingSite) -> Result<ExecutionBindingSiteV2, BindingResolutionError> {
    match site {
        PlanBindingSite::Node(key) => Ok(ExecutionBindingSiteV2::Node(key.clone())),
        PlanBindingSite::Trigger(key) => Ok(ExecutionBindingSiteV2::Trigger(key.clone())),
        _ => Err(BindingResolutionError::InvalidManifest),
    }
}

fn credential_capabilities(
    capabilities: Capabilities,
) -> impl Iterator<Item = CredentialCapability> {
    [
        (Capabilities::INTERACTIVE, CredentialCapability::Interactive),
        (Capabilities::REFRESHABLE, CredentialCapability::Refreshable),
        (Capabilities::REVOCABLE, CredentialCapability::Revocable),
        (Capabilities::TESTABLE, CredentialCapability::Testable),
        (Capabilities::DYNAMIC, CredentialCapability::Dynamic),
    ]
    .into_iter()
    .filter_map(move |(flag, capability)| capabilities.contains(flag).then_some(capability))
}

fn select_unique<T>(
    values: impl IntoIterator<Item = T>,
    mut matches: impl FnMut(&T) -> bool,
) -> Result<T, BindingResolutionError> {
    let mut matches = values.into_iter().filter(|value| matches(value));
    let selected = matches.next().ok_or(BindingResolutionError::NotFound)?;
    if matches.next().is_some() {
        return Err(BindingResolutionError::Ambiguous);
    }
    Ok(selected)
}

fn resource_is_visible(row: &ResourceRow, scope: &Scope) -> bool {
    row.workspace_id == scope.workspace_id && row.deleted_at.is_none()
}

fn resource_name_matches(row: &ResourceRow, selector: &str) -> bool {
    row.display_name == selector || row.slug == selector
}

fn credential_name_matches(display_name: Option<&str>, selector: &str) -> bool {
    display_name == Some(selector)
}

fn map_credential_error(error: CredentialServiceError) -> BindingResolutionError {
    match error {
        CredentialServiceError::NotFound { .. } | CredentialServiceError::ScopeViolation { .. } => {
            BindingResolutionError::NotFound
        },
        _ => BindingResolutionError::Unavailable,
    }
}

fn map_resource_error(error: StorageError) -> BindingResolutionError {
    match error {
        StorageError::NotFound { .. } | StorageError::ScopeViolation { .. } => {
            BindingResolutionError::NotFound
        },
        _ => BindingResolutionError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn unique_selection_distinguishes_missing_and_ambiguous_names() {
        assert_eq!(
            select_unique(["other", "target"], |value| *value == "target"),
            Ok("target")
        );
        assert_eq!(
            select_unique(["target", "target"], |value| *value == "target"),
            Err(BindingResolutionError::Ambiguous)
        );
        assert_eq!(
            select_unique(["other"], |value| *value == "target"),
            Err(BindingResolutionError::NotFound)
        );
    }

    #[test]
    fn resource_default_name_matches_exact_display_name_or_slug() {
        let row = ResourceRow {
            id: ResourceId::new().to_string(),
            workspace_id: "ws_test".to_owned(),
            slug: "primary-http".to_owned(),
            display_name: "Primary HTTP".to_owned(),
            kind: "core.http".to_owned(),
            config: serde_json::json!({}),
            credential_bindings: BTreeMap::new(),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            created_by: "system".to_owned(),
            version: 0,
            deleted_at: None,
        };

        assert!(resource_name_matches(&row, "Primary HTTP"));
        assert!(resource_name_matches(&row, "primary-http"));
        assert!(!resource_name_matches(&row, "primary http"));
        assert!(!resource_name_matches(&row, "Primary"));
    }

    #[test]
    fn credential_default_name_comparison_is_exact() {
        assert!(credential_name_matches(
            Some("Production API"),
            "Production API"
        ));
        assert!(!credential_name_matches(
            Some("Production API"),
            "production api"
        ));
        assert!(!credential_name_matches(None, "Production API"));
    }
}
