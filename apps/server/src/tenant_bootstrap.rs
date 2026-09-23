//! Explicit operator bootstrap for the first durable tenant.
//!
//! Bootstrap is disabled unless every setting is present. Stable identifiers
//! make startup retries deterministic, while the storage port owns atomicity
//! across the organization, default workspace, and initial owner grant.

use std::{collections::HashMap, future::Future, str::FromStr, sync::Arc};

use nebula_api::domain::auth::backend::{AuthBackend, AuthError, UserProfile};
use nebula_core::{OrgId, Slug, SlugKind, UserId, WorkspaceId};
use nebula_storage_port::{
    dto::{
        PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate, TenantProvisioningOutcome,
        TenantProvisioningRequest,
    },
    store::TenantProvisioningStore,
};
use thiserror::Error;

const ORG_ID: &str = "NEBULA_BOOTSTRAP_ORG_ID";
const ORG_SLUG: &str = "NEBULA_BOOTSTRAP_ORG_SLUG";
const ORG_NAME: &str = "NEBULA_BOOTSTRAP_ORG_NAME";
const ORG_PLAN: &str = "NEBULA_BOOTSTRAP_ORG_PLAN";
const WORKSPACE_ID: &str = "NEBULA_BOOTSTRAP_WORKSPACE_ID";
const WORKSPACE_SLUG: &str = "NEBULA_BOOTSTRAP_WORKSPACE_SLUG";
const WORKSPACE_NAME: &str = "NEBULA_BOOTSTRAP_WORKSPACE_NAME";
const OWNER_USER_ID: &str = "NEBULA_BOOTSTRAP_OWNER_USER_ID";

const VARIABLES: [&str; 8] = [
    ORG_ID,
    ORG_SLUG,
    ORG_NAME,
    ORG_PLAN,
    WORKSPACE_ID,
    WORKSPACE_SLUG,
    WORKSPACE_NAME,
    OWNER_USER_ID,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TenantBootstrapConfig {
    org_id: OrgId,
    org_slug: Slug,
    org_name: String,
    org_plan: String,
    workspace_id: WorkspaceId,
    workspace_slug: Slug,
    workspace_name: String,
    owner_user_id: UserId,
}

impl TenantBootstrapConfig {
    pub(crate) fn from_env() -> Result<Option<Self>, TenantBootstrapError> {
        let mut values = HashMap::new();
        for name in VARIABLES {
            if let Some(value) = std::env::var_os(name) {
                let value = value
                    .into_string()
                    .map_err(|_| TenantBootstrapError::InvalidValue(name))?;
                values.insert(name, value);
            }
        }
        Self::from_lookup(|name| values.get(name).cloned())
    }

    fn from_lookup(
        lookup: impl FnMut(&str) -> Option<String>,
    ) -> Result<Option<Self>, TenantBootstrapError> {
        let values = VARIABLES.map(lookup);
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let [
            org_id,
            org_slug,
            org_name,
            org_plan,
            workspace_id,
            workspace_slug,
            workspace_name,
            owner_user_id,
        ] = values.map(|value| value.ok_or(TenantBootstrapError::IncompleteConfig));

        let org_name = non_empty(org_name?, ORG_NAME)?;
        let org_plan = non_empty(org_plan?, ORG_PLAN)?;
        let workspace_name = non_empty(workspace_name?, WORKSPACE_NAME)?;
        Ok(Some(Self {
            org_id: OrgId::from_str(&org_id?)
                .map_err(|_| TenantBootstrapError::InvalidValue(ORG_ID))?,
            org_slug: Slug::new(&org_slug?, SlugKind::Org)
                .map_err(|_| TenantBootstrapError::InvalidValue(ORG_SLUG))?,
            org_name,
            org_plan,
            workspace_id: WorkspaceId::from_str(&workspace_id?)
                .map_err(|_| TenantBootstrapError::InvalidValue(WORKSPACE_ID))?,
            workspace_slug: Slug::new(&workspace_slug?, SlugKind::Workspace)
                .map_err(|_| TenantBootstrapError::InvalidValue(WORKSPACE_SLUG))?,
            workspace_name,
            owner_user_id: UserId::from_str(&owner_user_id?)
                .map_err(|_| TenantBootstrapError::InvalidValue(OWNER_USER_ID))?,
        }))
    }

    fn request(&self) -> Result<TenantProvisioningRequest, TenantBootstrapError> {
        let owner_id = self.owner_user_id.to_string();
        let org = TenantOrgCreate::new(
            self.org_id.to_string(),
            self.org_slug.to_string(),
            self.org_name.clone(),
            owner_id.clone(),
            self.org_plan.clone(),
            None,
            serde_json::json!({}),
        )
        .map_err(|_| TenantBootstrapError::InvalidRequest)?;
        let workspace = TenantDefaultWorkspaceCreate::new(
            self.workspace_id.to_string(),
            self.workspace_slug.to_string(),
            self.workspace_name.clone(),
            None,
            owner_id.clone(),
            serde_json::json!({}),
        )
        .map_err(|_| TenantBootstrapError::InvalidRequest)?;
        TenantProvisioningRequest::new(
            org,
            workspace,
            PrincipalKind::User,
            owner_id,
            Some("operator-bootstrap".to_owned()),
        )
        .map_err(|_| TenantBootstrapError::InvalidRequest)
    }
}

fn non_empty(value: String, variable: &'static str) -> Result<String, TenantBootstrapError> {
    if value.trim().is_empty() {
        Err(TenantBootstrapError::InvalidValue(variable))
    } else {
        Ok(value)
    }
}

#[derive(Debug, Error)]
pub(crate) enum TenantBootstrapError {
    #[error("tenant bootstrap configuration must set every bootstrap variable")]
    IncompleteConfig,
    #[error("tenant bootstrap configuration has an invalid value for {0}")]
    InvalidValue(&'static str),
    #[error("tenant bootstrap request is invalid")]
    InvalidRequest,
    #[error("tenant bootstrap owner does not exist in the selected authentication backend")]
    OwnerNotFound,
    #[error("tenant bootstrap owner must have a verified email")]
    OwnerEmailUnverified,
    #[error("tenant bootstrap owner lookup failed")]
    OwnerLookupFailed,
    #[error("tenant bootstrap storage operation failed")]
    StorageFailed,
    #[error("tenant bootstrap conflicts with existing durable state")]
    Conflict,
}

/// Validate the owner through Plane A before creating any tenant authority.
pub(crate) async fn bootstrap_tenant(
    config: Option<TenantBootstrapConfig>,
    auth_backend: &Arc<dyn AuthBackend>,
    provisioner: &Arc<dyn TenantProvisioningStore>,
) -> Result<(), TenantBootstrapError> {
    bootstrap_tenant_with_lookup(
        config,
        |owner_id| async move { auth_backend.get_user_profile(&owner_id).await },
        provisioner,
    )
    .await
}

async fn bootstrap_tenant_with_lookup<F, Fut>(
    config: Option<TenantBootstrapConfig>,
    lookup_owner: F,
    provisioner: &Arc<dyn TenantProvisioningStore>,
) -> Result<(), TenantBootstrapError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<UserProfile, AuthError>>,
{
    let Some(config) = config else {
        return Ok(());
    };

    let profile = lookup_owner(config.owner_user_id.to_string())
        .await
        .map_err(map_owner_lookup_error)?;
    validate_owner_profile(&config, &profile)?;

    provision_verified_tenant(&config, provisioner).await
}

async fn provision_verified_tenant(
    config: &TenantBootstrapConfig,
    provisioner: &Arc<dyn TenantProvisioningStore>,
) -> Result<(), TenantBootstrapError> {
    match provisioner
        .provision_tenant(config.request()?)
        .await
        .map_err(|error| {
            tracing::error!(
                error.category = crate::storage_diagnostics::storage_error_category(&error),
                "tenant bootstrap storage operation failed"
            );
            TenantBootstrapError::StorageFailed
        })? {
        TenantProvisioningOutcome::Created => {
            tracing::info!(org.id = %config.org_id, workspace.id = %config.workspace_id, "tenant bootstrap created durable authority");
            Ok(())
        },
        TenantProvisioningOutcome::Replayed => {
            tracing::info!(org.id = %config.org_id, workspace.id = %config.workspace_id, "tenant bootstrap matched existing durable authority");
            Ok(())
        },
        TenantProvisioningOutcome::Conflict(_) => Err(TenantBootstrapError::Conflict),
    }
}

fn map_owner_lookup_error(error: AuthError) -> TenantBootstrapError {
    if matches!(error, AuthError::UserNotFound) {
        TenantBootstrapError::OwnerNotFound
    } else {
        tracing::error!("tenant bootstrap owner lookup failed");
        TenantBootstrapError::OwnerLookupFailed
    }
}

fn validate_owner_profile(
    config: &TenantBootstrapConfig,
    profile: &UserProfile,
) -> Result<(), TenantBootstrapError> {
    if UserId::from_str(&profile.user_id) != Ok(config.owner_user_id) {
        return Err(TenantBootstrapError::OwnerLookupFailed);
    }
    if !profile.email_verified {
        return Err(TenantBootstrapError::OwnerEmailUnverified);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use nebula_storage_port::{StorageError, dto::TenantProvisioningConflict};

    use super::*;

    #[derive(Debug)]
    struct FixedProvisioner(TenantProvisioningOutcome);

    #[async_trait]
    impl TenantProvisioningStore for FixedProvisioner {
        async fn provision_tenant(
            &self,
            _request: TenantProvisioningRequest,
        ) -> Result<TenantProvisioningOutcome, StorageError> {
            Ok(self.0)
        }
    }

    #[derive(Debug)]
    struct RecordingProvisioner {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl TenantProvisioningStore for RecordingProvisioner {
        async fn provision_tenant(
            &self,
            _request: TenantProvisioningRequest,
        ) -> Result<TenantProvisioningOutcome, StorageError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(TenantProvisioningOutcome::Created)
        }
    }

    fn complete_values() -> HashMap<&'static str, String> {
        HashMap::from([
            (ORG_ID, OrgId::new().to_string()),
            (ORG_SLUG, "acme-corp".to_owned()),
            (ORG_NAME, "Acme Corp".to_owned()),
            (ORG_PLAN, "team".to_owned()),
            (WORKSPACE_ID, WorkspaceId::new().to_string()),
            (WORKSPACE_SLUG, "main".to_owned()),
            (WORKSPACE_NAME, "Main".to_owned()),
            (OWNER_USER_ID, UserId::new().to_string()),
        ])
    }

    #[test]
    fn bootstrap_is_disabled_when_every_variable_is_absent() {
        assert_eq!(TenantBootstrapConfig::from_lookup(|_| None).unwrap(), None);
    }

    #[test]
    fn partial_configuration_fails_closed() {
        let error = TenantBootstrapConfig::from_lookup(|name| {
            (name == ORG_ID).then(|| OrgId::new().to_string())
        })
        .unwrap_err();
        assert!(matches!(error, TenantBootstrapError::IncompleteConfig));
    }

    #[test]
    fn ids_and_slug_kinds_are_validated() {
        let mut values = complete_values();
        values.insert(WORKSPACE_ID, OrgId::new().to_string());
        assert!(matches!(
            TenantBootstrapConfig::from_lookup(|name| values.get(name).cloned()),
            Err(TenantBootstrapError::InvalidValue(WORKSPACE_ID))
        ));

        let mut values = complete_values();
        values.insert(ORG_SLUG, "ab".to_owned());
        assert!(matches!(
            TenantBootstrapConfig::from_lookup(|name| values.get(name).cloned()),
            Err(TenantBootstrapError::InvalidValue(ORG_SLUG))
        ));
    }

    #[test]
    fn request_uses_stable_ids_and_fixes_owner_role() {
        let values = complete_values();
        let config = TenantBootstrapConfig::from_lookup(|name| values.get(name).cloned())
            .unwrap()
            .unwrap();
        let request = config.request().unwrap();
        assert_eq!(request.org().id(), config.org_id.to_string());
        assert_eq!(
            request.default_workspace().id(),
            config.workspace_id.to_string()
        );
        assert_eq!(request.owner_principal_kind(), PrincipalKind::User);
        assert_eq!(
            request.owner_principal_id(),
            config.owner_user_id.to_string()
        );
        assert_eq!(request.owner_added_by(), Some("operator-bootstrap"));
    }

    #[test]
    fn owner_must_match_and_have_verified_email() {
        let values = complete_values();
        let config = TenantBootstrapConfig::from_lookup(|name| values.get(name).cloned())
            .unwrap()
            .unwrap();
        let mut profile = UserProfile {
            user_id: config.owner_user_id.to_string(),
            email: "owner@example.com".to_owned(),
            display_name: "Owner".to_owned(),
            avatar_url: None,
            email_verified: false,
            mfa_enabled: false,
        };
        assert!(matches!(
            validate_owner_profile(&config, &profile),
            Err(TenantBootstrapError::OwnerEmailUnverified)
        ));
        profile.email_verified = true;
        assert!(validate_owner_profile(&config, &profile).is_ok());
        profile.user_id = UserId::new().to_string();
        assert!(matches!(
            validate_owner_profile(&config, &profile),
            Err(TenantBootstrapError::OwnerLookupFailed)
        ));
    }

    #[tokio::test]
    async fn rejected_owner_never_reaches_provisioning() {
        let values = complete_values();
        let config = TenantBootstrapConfig::from_lookup(|name| values.get(name).cloned())
            .unwrap()
            .unwrap();
        let matching_profile = UserProfile {
            user_id: config.owner_user_id.to_string(),
            email: "owner@example.com".to_owned(),
            display_name: "Owner".to_owned(),
            avatar_url: None,
            email_verified: true,
            mfa_enabled: false,
        };
        let mut mismatched_profile = matching_profile.clone();
        mismatched_profile.user_id = UserId::new().to_string();
        let mut unverified_profile = matching_profile;
        unverified_profile.email_verified = false;

        let cases = [
            Err(AuthError::UserNotFound),
            Err(AuthError::Internal("backend unavailable".to_owned())),
            Ok(mismatched_profile),
            Ok(unverified_profile),
        ];
        for lookup_result in cases {
            let recording = Arc::new(RecordingProvisioner {
                calls: AtomicUsize::new(0),
            });
            let provisioner: Arc<dyn TenantProvisioningStore> = recording.clone();
            let result = bootstrap_tenant_with_lookup(
                Some(config.clone()),
                |_| async move { lookup_result },
                &provisioner,
            )
            .await;
            assert!(result.is_err());
            assert_eq!(recording.calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn created_and_exact_replay_both_admit_startup() {
        let values = complete_values();
        let config = TenantBootstrapConfig::from_lookup(|name| values.get(name).cloned())
            .unwrap()
            .unwrap();
        for outcome in [
            TenantProvisioningOutcome::Created,
            TenantProvisioningOutcome::Replayed,
        ] {
            let store: Arc<dyn TenantProvisioningStore> = Arc::new(FixedProvisioner(outcome));
            assert!(provision_verified_tenant(&config, &store).await.is_ok());
        }
    }

    #[tokio::test]
    async fn durable_conflict_aborts_startup() {
        let values = complete_values();
        let config = TenantBootstrapConfig::from_lookup(|name| values.get(name).cloned())
            .unwrap()
            .unwrap();
        let store: Arc<dyn TenantProvisioningStore> = Arc::new(FixedProvisioner(
            TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::ExistingState),
        ));
        assert!(matches!(
            provision_verified_tenant(&config, &store).await,
            Err(TenantBootstrapError::Conflict)
        ));
    }
}
