//! Authenticated resolution of authority-free executable-plan bindings.

use std::{fmt, future::Future, pin::Pin};

use nebula_execution::{
    CredentialCapability, ExecutionBindingEntryV2, ExecutionBindingManifestV2,
    ExecutionBindingSiteV2, ExecutionBindingTargetV2,
};
use nebula_plugin::{PlanBindingContract, PlanBindingRequirement, PlanBindingSite};
use nebula_storage_port::Scope;

/// Boxed future returned by [`ExecutionBindingResolver`].
pub type BindingResolutionFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<ExecutionBindingManifestV2, BindingResolutionError>> + Send + 'a,
    >,
>;

/// Deployment-owned resolver for authority-free plan binding selectors.
///
/// Implementations must resolve names and typed overrides under `scope`, validate
/// the selected object's exact contract, and return one manifest entry for every
/// required plan slot. The engine independently compares that manifest with the
/// immutable plan before persisting or consuming it.
pub trait ExecutionBindingResolver: Send + Sync {
    /// Resolve every plan requirement under the authenticated tenant scope.
    fn resolve<'a>(
        &'a self,
        scope: &'a Scope,
        requirements: &'a [PlanBindingRequirement],
    ) -> BindingResolutionFuture<'a>;
}

impl fmt::Debug for dyn ExecutionBindingResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExecutionBindingResolver(..)")
    }
}

/// Secret-free failures from authenticated binding selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BindingResolutionError {
    /// No object matched a required selector in the authenticated tenant.
    #[error("a required workflow binding was not found")]
    NotFound,
    /// More than one owner-local object matched a default-name selector.
    #[error("a workflow binding selector is ambiguous")]
    Ambiguous,
    /// The selected object does not satisfy the exact slot contract.
    #[error("a workflow binding does not satisfy its declared contract")]
    Incompatible,
    /// The binding backend could not complete the lookup.
    #[error("workflow binding resolution is unavailable")]
    Unavailable,
    /// The resolver returned a malformed or incomplete manifest.
    #[error("workflow binding resolution returned an invalid manifest")]
    InvalidManifest,
}

pub(crate) fn validate_manifest(
    requirements: &[PlanBindingRequirement],
    manifest: &ExecutionBindingManifestV2,
) -> Result<(), BindingResolutionError> {
    if manifest.entries().iter().any(|entry| {
        !requirements
            .iter()
            .any(|requirement| entry_matches(entry, requirement))
    }) || requirements.iter().any(|requirement| {
        requirement.required()
            && !manifest
                .entries()
                .iter()
                .any(|entry| entry_matches(entry, requirement))
    }) {
        return Err(BindingResolutionError::InvalidManifest);
    }
    Ok(())
}

fn entry_matches(entry: &ExecutionBindingEntryV2, requirement: &PlanBindingRequirement) -> bool {
    if entry.slot_key() != requirement.slot_key() || !site_matches(entry.site(), requirement.site())
    {
        return false;
    }
    match (entry.target(), requirement.contract()) {
        (
            ExecutionBindingTargetV2::Credential { contract, .. },
            PlanBindingContract::Credential {
                key,
                version,
                required_capabilities,
                ..
            },
        ) => {
            contract.key() == key
                && contract.version().as_str() == version.to_string()
                && contract
                    .required_capabilities()
                    .iter()
                    .copied()
                    .eq(credential_capabilities(*required_capabilities))
        },
        (
            ExecutionBindingTargetV2::Resource { contract, .. },
            PlanBindingContract::Resource { key, version, .. },
        ) => contract.key() == key && contract.version().as_str() == version.to_string(),
        _ => false,
    }
}

fn site_matches(left: &ExecutionBindingSiteV2, right: &PlanBindingSite) -> bool {
    matches!(
        (left, right),
        (ExecutionBindingSiteV2::Node(left), PlanBindingSite::Node(right))
            | (ExecutionBindingSiteV2::Trigger(left), PlanBindingSite::Trigger(right))
            if left == right
    )
}

fn credential_capabilities(
    capabilities: nebula_credential::Capabilities,
) -> impl Iterator<Item = CredentialCapability> {
    [
        (
            nebula_credential::Capabilities::INTERACTIVE,
            CredentialCapability::Interactive,
        ),
        (
            nebula_credential::Capabilities::REFRESHABLE,
            CredentialCapability::Refreshable,
        ),
        (
            nebula_credential::Capabilities::REVOCABLE,
            CredentialCapability::Revocable,
        ),
        (
            nebula_credential::Capabilities::TESTABLE,
            CredentialCapability::Testable,
        ),
        (
            nebula_credential::Capabilities::DYNAMIC,
            CredentialCapability::Dynamic,
        ),
    ]
    .into_iter()
    .filter_map(move |(flag, capability)| capabilities.contains(flag).then_some(capability))
}
