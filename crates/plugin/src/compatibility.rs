//! Exact frozen-registry compatibility for integrity-checked Graph-v1 plans.

use nebula_core::{CredentialKey, Dependencies, PluginSetId, ResourceKey, WorkerFlavorRevisionId};
use nebula_error::{ActivationDiagnostic, ActivationDiagnostics};

use crate::{
    FrozenPluginRegistry,
    compiler::{project_action_contract, project_credential_contract, project_resource_contract},
    plan::{
        ExecutablePlanRevision, RecordedDependenciesV1, RecordedExecutablePlanRevisionV1,
        RecordedPluginV1,
    },
    resolved_plugin::{
        ActionContractSnapshot, CredentialContractSnapshot, ResourceContractSnapshot,
    },
};

/// Exact-registry incompatibility for an integrity-checked executable plan.
///
/// Compatibility proves that the supplied frozen registry still exposes the
/// exact Graph-v1 contracts captured in the plan. It does not authenticate
/// artifacts, retain a plan, authorize a tenant, resolve selectors, or admit
/// an execution.
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum PlanRegistryCompatibilityError {
    /// The plan was compiled against another logical plugin set.
    #[classify(
        category = "validation",
        code = "PLUGIN_PLAN_COMPATIBILITY:PLUGIN_SET_MISMATCH"
    )]
    #[error("executable plan requires a different frozen plugin set")]
    PluginSetMismatch {
        /// Plugin-set identity pinned by the plan.
        plan: PluginSetId,
        /// Plugin-set identity exposed by the supplied registry.
        registry: PluginSetId,
    },

    /// The plan was compiled against another exact worker flavor.
    #[classify(
        category = "validation",
        code = "PLUGIN_PLAN_COMPATIBILITY:WORKER_FLAVOR_MISMATCH"
    )]
    #[error("executable plan requires a different frozen worker flavor")]
    WorkerFlavorMismatch {
        /// Worker-flavor identity pinned by the plan.
        plan: WorkerFlavorRevisionId,
        /// Worker-flavor identity exposed by the supplied registry.
        registry: WorkerFlavorRevisionId,
    },

    /// A used plugin/component/dependency contract no longer matches.
    #[classify(
        category = "validation",
        code = "PLUGIN_PLAN_COMPATIBILITY:CONTRACT_MISMATCH"
    )]
    #[error("frozen registry does not match executable-plan section '{section}'")]
    ContractMismatch {
        /// Stable, payload-free contract section.
        section: &'static str,
    },
}

impl ExecutablePlanRevision {
    /// Check this plan against one exact frozen plugin registry.
    ///
    /// The plan's plugin-set and worker-flavor identities must match, and
    /// every used plugin, action, resource, credential, dependency type,
    /// capability set, schema, and port contract is compared with the
    /// registry's immutable snapshots. Abstract binding selector text is plan
    /// content and is deliberately not resolved here.
    ///
    /// # Errors
    ///
    /// Returns [`PlanRegistryCompatibilityError`] when an identity or used
    /// contract differs. A successful result is not artifact authentication,
    /// tenant authorization, retention, or execution admission.
    pub fn validate_against(
        &self,
        registry: &FrozenPluginRegistry,
    ) -> Result<(), PlanRegistryCompatibilityError> {
        let plan_plugin_set = self.plugin_set_id();
        let registry_plugin_set = registry.plugin_set().id();
        if plan_plugin_set != registry_plugin_set {
            return Err(PlanRegistryCompatibilityError::PluginSetMismatch {
                plan: plan_plugin_set,
                registry: registry_plugin_set,
            });
        }

        let plan_flavor = self.worker_flavor_revision_id();
        let registry_flavor = registry.revision().id();
        if plan_flavor != registry_flavor {
            return Err(PlanRegistryCompatibilityError::WorkerFlavorMismatch {
                plan: plan_flavor,
                registry: registry_flavor,
            });
        }

        validate_recorded_contracts(self.recorded(), registry)
    }
}

fn mismatch(section: &'static str) -> PlanRegistryCompatibilityError {
    PlanRegistryCompatibilityError::ContractMismatch { section }
}

fn validate_recorded_contracts(
    record: &RecordedExecutablePlanRevisionV1,
    registry: &FrozenPluginRegistry,
) -> Result<(), PlanRegistryCompatibilityError> {
    for plugin in &record.content.plugins {
        validate_plugin(plugin, registry)?;
    }

    for action in &record.content.actions {
        let plugin_key = action.plugin_key.parse().map_err(|_| mismatch("actions"))?;
        let action_key = action.key.parse().map_err(|_| mismatch("actions"))?;
        let plugin = registry
            .get(&plugin_key)
            .ok_or_else(|| mismatch("actions"))?;
        let projected = project_action_contract(&plugin, &action_key)
            .map_err(|_| mismatch("actions"))?
            .ok_or_else(|| mismatch("actions"))?;
        if &projected != action {
            return Err(mismatch("actions"));
        }
        let dependencies = plugin
            .action_contract(&action_key)
            .map(ActionContractSnapshot::dependencies)
            .ok_or_else(|| mismatch("actions"))?;
        validate_dependency_coherence(&plugin_key, dependencies, registry)?;
    }

    for resource in &record.content.resources {
        let plugin_key = resource
            .plugin_key
            .parse()
            .map_err(|_| mismatch("resources"))?;
        let resource_key = resource.key.parse().map_err(|_| mismatch("resources"))?;
        let plugin = registry
            .get(&plugin_key)
            .ok_or_else(|| mismatch("resources"))?;
        let projected = project_resource_contract(&plugin, &resource_key)
            .map_err(|_| mismatch("resources"))?
            .ok_or_else(|| mismatch("resources"))?;
        if &projected != resource {
            return Err(mismatch("resources"));
        }
        let dependencies = plugin
            .resource_contract(&resource_key)
            .map(ResourceContractSnapshot::dependencies)
            .ok_or_else(|| mismatch("resources"))?;
        validate_dependency_coherence(&plugin_key, dependencies, registry)?;
    }

    for credential in &record.content.credentials {
        let plugin_key = credential
            .plugin_key
            .parse()
            .map_err(|_| mismatch("credentials"))?;
        let credential_key = credential
            .key
            .parse()
            .map_err(|_| mismatch("credentials"))?;
        let plugin = registry
            .get(&plugin_key)
            .ok_or_else(|| mismatch("credentials"))?;
        let projected = project_credential_contract(&plugin, &credential_key)
            .map_err(|_| mismatch("credentials"))?
            .ok_or_else(|| mismatch("credentials"))?;
        if &projected != credential {
            return Err(mismatch("credentials"));
        }
    }

    validate_recorded_dependency_shape(record, registry)
}

fn validate_plugin(
    recorded: &RecordedPluginV1,
    registry: &FrozenPluginRegistry,
) -> Result<(), PlanRegistryCompatibilityError> {
    let key = recorded.key.parse().map_err(|_| mismatch("plugins"))?;
    let plugin = registry.get(&key).ok_or_else(|| mismatch("plugins"))?;
    let mut actual = plugin.version().clone();
    actual.build = semver::BuildMetadata::EMPTY;
    let recorded = semver::Version::try_from(&recorded.version).map_err(|_| mismatch("plugins"))?;
    if actual != recorded {
        return Err(mismatch("plugins"));
    }
    Ok(())
}

fn validate_dependency_coherence(
    owner: &nebula_core::PluginKey,
    dependencies: &Dependencies,
    registry: &FrozenPluginRegistry,
) -> Result<(), PlanRegistryCompatibilityError> {
    for requirement in dependencies.resources() {
        let (provider, snapshot) =
            find_resource(registry, &requirement.key).ok_or_else(|| mismatch("resources"))?;
        if snapshot.type_id() != requirement.type_id {
            return Err(mismatch("dependencies.type"));
        }
        validate_plugin_edge(owner, provider.key(), registry)?;
    }
    for requirement in dependencies.credentials() {
        let (provider, snapshot) =
            find_credential(registry, &requirement.key).ok_or_else(|| mismatch("credentials"))?;
        if snapshot.type_id() != requirement.type_id {
            return Err(mismatch("dependencies.type"));
        }
        validate_plugin_edge(owner, provider.key(), registry)?;
    }
    for slot in dependencies.slot_fields() {
        match &slot.kind {
            nebula_core::SlotKind::Resource { key, type_id, .. } => {
                let (provider, snapshot) =
                    find_resource(registry, key).ok_or_else(|| mismatch("resources"))?;
                if snapshot.type_id() != *type_id {
                    return Err(mismatch("dependencies.type"));
                }
                validate_plugin_edge(owner, provider.key(), registry)?;
            },
            nebula_core::SlotKind::Credential { key, type_id, .. } => {
                let (provider, snapshot) =
                    find_credential(registry, key).ok_or_else(|| mismatch("credentials"))?;
                if snapshot.type_id() != *type_id {
                    return Err(mismatch("dependencies.type"));
                }
                validate_plugin_edge(owner, provider.key(), registry)?;
            },
        }
    }
    Ok(())
}

fn validate_recorded_dependency_shape(
    record: &RecordedExecutablePlanRevisionV1,
    registry: &FrozenPluginRegistry,
) -> Result<(), PlanRegistryCompatibilityError> {
    for action in &record.content.actions {
        validate_recorded_dependencies(&action.dependencies, registry)?;
    }
    for resource in &record.content.resources {
        validate_recorded_dependencies(&resource.dependencies, registry)?;
    }
    Ok(())
}

fn validate_recorded_dependencies(
    dependencies: &RecordedDependenciesV1,
    registry: &FrozenPluginRegistry,
) -> Result<(), PlanRegistryCompatibilityError> {
    for dependency in &dependencies.resources {
        let key = dependency
            .key
            .parse::<ResourceKey>()
            .map_err(|_| mismatch("resources"))?;
        if find_resource(registry, &key).is_none() {
            return Err(mismatch("resources"));
        }
    }
    for dependency in &dependencies.credentials {
        let key = dependency
            .key
            .parse::<CredentialKey>()
            .map_err(|_| mismatch("credentials"))?;
        if find_credential(registry, &key).is_none() {
            return Err(mismatch("credentials"));
        }
    }
    Ok(())
}

fn validate_plugin_edge(
    owner: &nebula_core::PluginKey,
    provider: &nebula_core::PluginKey,
    registry: &FrozenPluginRegistry,
) -> Result<(), PlanRegistryCompatibilityError> {
    if owner == provider {
        return Ok(());
    }
    let owner = registry
        .get(owner)
        .ok_or_else(|| mismatch("plugins.dependencies"))?;
    let provider = registry
        .get(provider)
        .ok_or_else(|| mismatch("plugins.dependencies"))?;
    if owner.manifest().dependencies().iter().any(|dependency| {
        dependency.key() == provider.key() && dependency.req().matches(provider.version())
    }) {
        Ok(())
    } else {
        Err(mismatch("plugins.dependencies"))
    }
}

/// Locate the plugin providing `key`, choosing the provider deterministically.
///
/// `FrozenPluginRegistry::iter` order is unspecified, so an arbitrary winner
/// among two plugins that both namespace-own a key made this compatibility
/// verdict vary between processes over identical inputs. It must agree with
/// the compiler's choice, which is the lowest plugin key.
fn find_resource<'a>(
    registry: &'a FrozenPluginRegistry,
    key: &ResourceKey,
) -> Option<(&'a crate::ResolvedPlugin, &'a ResourceContractSnapshot)> {
    registry
        .iter()
        .filter_map(|(plugin_key, plugin)| {
            plugin
                .resource_contract(key)
                .map(|snapshot| (plugin_key.as_str(), plugin.as_ref(), snapshot))
        })
        .min_by_key(|(plugin_key, _, _)| *plugin_key)
        .map(|(_, plugin, snapshot)| (plugin, snapshot))
}

/// Locate the plugin providing `key`, choosing deterministically.
///
/// Same reasoning as [`find_resource`].
fn find_credential<'a>(
    registry: &'a FrozenPluginRegistry,
    key: &CredentialKey,
) -> Option<(&'a crate::ResolvedPlugin, &'a CredentialContractSnapshot)> {
    registry
        .iter()
        .filter_map(|(plugin_key, plugin)| {
            plugin
                .credential_contract(key)
                .map(|snapshot| (plugin_key.as_str(), plugin.as_ref(), snapshot))
        })
        .min_by_key(|(plugin_key, _, _)| *plugin_key)
        .map(|(_, plugin, snapshot)| (plugin, snapshot))
}

/// Build one diagnostic from constant parts, or fall back to a complete one.
///
/// Every field here is a compiler-authored constant or a typed identifier, so
/// construction cannot legitimately fail. The fallback exists because a
/// rejection that reports nothing is worse than one that reports a coarse
/// code: a caller must never receive an empty diagnostic list.
fn diagnostic(
    code: &str,
    path: &str,
    expected: String,
    actual: String,
    remediation: &str,
) -> ActivationDiagnostic {
    ActivationDiagnostic::new(code, path, expected, actual, remediation).unwrap_or_else(|| {
        ActivationDiagnostic::new(
            code,
            path,
            "<contract>",
            "<unavailable>",
            "recompile the plan against the frozen registry this worker loads",
        )
        .unwrap_or_else(|| unreachable!("the fallback diagnostic uses non-empty constants"))
    })
}

impl ActivationDiagnostics for PlanRegistryCompatibilityError {
    fn activation_diagnostics(&self) -> Vec<ActivationDiagnostic> {
        let single = match self {
            Self::PluginSetMismatch { plan, registry } => diagnostic(
                "PLUGIN_PLAN_COMPATIBILITY:PLUGIN_SET_MISMATCH",
                "/plan/plugin_set_id",
                plan.to_string(),
                registry.to_string(),
                "load the worker flavor whose plugin set this plan was compiled against, \
                 or recompile the plan against this worker's frozen registry",
            ),
            Self::WorkerFlavorMismatch { plan, registry } => diagnostic(
                "PLUGIN_PLAN_COMPATIBILITY:WORKER_FLAVOR_MISMATCH",
                "/plan/worker_flavor_revision_id",
                plan.to_string(),
                registry.to_string(),
                "dispatch this plan to a worker running its pinned flavor, \
                 or recompile the plan against this worker's flavor",
            ),
            // The section is a stable, payload-free contract name; the differing
            // contract body is deliberately not reported, because it can carry
            // schema defaults and credential metadata.
            Self::ContractMismatch { section } => diagnostic(
                "PLUGIN_PLAN_COMPATIBILITY:CONTRACT_MISMATCH",
                &format!("/plan/{section}"),
                "the contract recorded when this plan was compiled".to_owned(),
                "a different contract in the frozen registry".to_owned(),
                "recompile the plan against this worker's frozen registry",
            ),
        };
        vec![single]
    }
}

#[cfg(test)]
mod activation_diagnostic_tests {
    use nebula_core::PluginSetId;

    use super::*;

    #[test]
    fn every_compatibility_rejection_reports_all_five_fields() {
        let plan_set = PluginSetId::from_bytes([0x11; 32]);
        let registry_set = PluginSetId::from_bytes([0x22; 32]);
        let flavor_plan = WorkerFlavorRevisionId::from_bytes([0x33; 32]);
        let flavor_registry = WorkerFlavorRevisionId::from_bytes([0x44; 32]);

        let rejections = [
            PlanRegistryCompatibilityError::PluginSetMismatch {
                plan: plan_set,
                registry: registry_set,
            },
            PlanRegistryCompatibilityError::WorkerFlavorMismatch {
                plan: flavor_plan,
                registry: flavor_registry,
            },
            PlanRegistryCompatibilityError::ContractMismatch { section: "actions" },
        ];

        for rejection in &rejections {
            let diagnostics = rejection.activation_diagnostics();
            assert!(
                !diagnostics.is_empty(),
                "a rejection with nothing to report is not actionable"
            );
            for reported in diagnostics {
                for field in [
                    reported.code(),
                    reported.path(),
                    reported.expected(),
                    reported.actual(),
                    reported.remediation(),
                ] {
                    assert!(!field.trim().is_empty(), "NS14 requires all five fields");
                }
            }
        }
    }

    /// A mismatch names both sides, so a caller can tell which artifact to move
    /// without re-deriving either identity.
    #[test]
    fn an_identity_mismatch_reports_both_sides() {
        let plan = WorkerFlavorRevisionId::from_bytes([0x33; 32]);
        let registry = WorkerFlavorRevisionId::from_bytes([0x44; 32]);
        let reported = PlanRegistryCompatibilityError::WorkerFlavorMismatch { plan, registry }
            .activation_diagnostics();

        let only = reported.first().expect("one diagnostic is reported");
        assert_eq!(only.expected(), plan.to_string());
        assert_eq!(only.actual(), registry.to_string());
        assert_ne!(only.expected(), only.actual());
    }
}
