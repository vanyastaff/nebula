//! Atomic tenant-provisioning command and outcome.

use super::{OrgRow, PrincipalKind, WorkspaceRow};

/// Organization values supplied to initial tenant provisioning.
///
/// Storage authors `created_at`, starts `version` at zero, and creates only
/// live rows. `created_by` remains explicit audit identity and is therefore
/// part of exact replay comparison.
#[expect(
    clippy::derive_partial_eq_without_eq,
    reason = "JSON values are not Eq"
)]
#[derive(Debug, Clone, PartialEq)]
pub struct TenantOrgCreate {
    id: String,
    slug: String,
    display_name: String,
    created_by: String,
    plan: String,
    billing_email: Option<String>,
    settings: serde_json::Value,
}

impl TenantOrgCreate {
    /// Build validated organization creation values.
    ///
    /// # Errors
    /// Returns a payload-free error when a required stable value is empty.
    pub fn new(
        id: String,
        slug: String,
        display_name: String,
        created_by: String,
        plan: String,
        billing_email: Option<String>,
        settings: serde_json::Value,
    ) -> Result<Self, TenantProvisioningRequestError> {
        if id.is_empty()
            || slug.is_empty()
            || display_name.is_empty()
            || created_by.is_empty()
            || plan.is_empty()
        {
            return Err(TenantProvisioningRequestError);
        }
        Ok(Self {
            id,
            slug,
            display_name,
            created_by,
            plan,
            billing_email,
            settings,
        })
    }

    /// Opaque organization identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Active organization slug.
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }
    /// Organization display name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
    /// Audit identity that created the organization.
    #[must_use]
    pub fn created_by(&self) -> &str {
        &self.created_by
    }
    /// Initial plan tier.
    #[must_use]
    pub fn plan(&self) -> &str {
        &self.plan
    }
    /// Optional billing address.
    #[must_use]
    pub fn billing_email(&self) -> Option<&str> {
        self.billing_email.as_deref()
    }
    /// Initial organization settings.
    #[must_use]
    pub const fn settings(&self) -> &serde_json::Value {
        &self.settings
    }

    /// Compare the caller-owned semantic fields of a persisted live row.
    #[must_use]
    pub fn matches_persisted(&self, row: &OrgRow) -> bool {
        row.id == self.id
            && row.slug == self.slug
            && row.display_name == self.display_name
            && row.created_by == self.created_by
            && row.plan == self.plan
            && row.billing_email == self.billing_email
            && row.settings == self.settings
            && row.deleted_at.is_none()
    }

    /// Materialize a new live organization row with backend-authored time.
    #[must_use]
    pub fn materialize(&self, created_at: String) -> OrgRow {
        OrgRow {
            id: self.id.clone(),
            slug: self.slug.clone(),
            display_name: self.display_name.clone(),
            created_at,
            created_by: self.created_by.clone(),
            plan: self.plan.clone(),
            billing_email: self.billing_email.clone(),
            settings: self.settings.clone(),
            version: 0,
            deleted_at: None,
        }
    }
}

/// Workspace values supplied to initial tenant provisioning.
///
/// The parent organization, default marker, timestamp, initial version, and
/// live state are fixed by the atomic operation.
#[expect(
    clippy::derive_partial_eq_without_eq,
    reason = "JSON values are not Eq"
)]
#[derive(Debug, Clone, PartialEq)]
pub struct TenantDefaultWorkspaceCreate {
    id: String,
    slug: String,
    display_name: String,
    description: Option<String>,
    created_by: String,
    settings: serde_json::Value,
}

impl TenantDefaultWorkspaceCreate {
    /// Build validated default-workspace creation values.
    ///
    /// # Errors
    /// Returns a payload-free error when a required stable value is empty.
    pub fn new(
        id: String,
        slug: String,
        display_name: String,
        description: Option<String>,
        created_by: String,
        settings: serde_json::Value,
    ) -> Result<Self, TenantProvisioningRequestError> {
        if id.is_empty() || slug.is_empty() || display_name.is_empty() || created_by.is_empty() {
            return Err(TenantProvisioningRequestError);
        }
        Ok(Self {
            id,
            slug,
            display_name,
            description,
            created_by,
            settings,
        })
    }

    /// Opaque workspace identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Active workspace slug.
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }
    /// Workspace display name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
    /// Optional workspace description.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    /// Audit identity that created the workspace.
    #[must_use]
    pub fn created_by(&self) -> &str {
        &self.created_by
    }
    /// Initial workspace settings.
    #[must_use]
    pub const fn settings(&self) -> &serde_json::Value {
        &self.settings
    }

    /// Compare the caller-owned semantic fields of a persisted live default row.
    #[must_use]
    pub fn matches_persisted(&self, org_id: &str, row: &WorkspaceRow) -> bool {
        row.id == self.id
            && row.org_id == org_id
            && row.slug == self.slug
            && row.display_name == self.display_name
            && row.description == self.description
            && row.created_by == self.created_by
            && row.is_default
            && row.settings == self.settings
            && row.deleted_at.is_none()
    }

    /// Materialize a new live default workspace row with backend-authored time.
    #[must_use]
    pub fn materialize(&self, org_id: String, created_at: String) -> WorkspaceRow {
        WorkspaceRow {
            id: self.id.clone(),
            org_id,
            slug: self.slug.clone(),
            display_name: self.display_name.clone(),
            description: self.description.clone(),
            created_at,
            created_by: self.created_by.clone(),
            is_default: true,
            settings: self.settings.clone(),
            version: 0,
            deleted_at: None,
        }
    }
}

/// Complete semantic command required to bootstrap one tenant.
#[derive(Debug, Clone, PartialEq)]
pub struct TenantProvisioningRequest {
    org: TenantOrgCreate,
    default_workspace: TenantDefaultWorkspaceCreate,
    owner_principal_kind: PrincipalKind,
    owner_principal_id: String,
    owner_added_by: Option<String>,
}

impl TenantProvisioningRequest {
    /// Build a validated tenant-provisioning command.
    ///
    /// # Errors
    /// Returns a payload-free error when the owner identifier is empty.
    pub fn new(
        org: TenantOrgCreate,
        default_workspace: TenantDefaultWorkspaceCreate,
        owner_principal_kind: PrincipalKind,
        owner_principal_id: String,
        owner_added_by: Option<String>,
    ) -> Result<Self, TenantProvisioningRequestError> {
        if owner_principal_id.is_empty() {
            return Err(TenantProvisioningRequestError);
        }
        Ok(Self {
            org,
            default_workspace,
            owner_principal_kind,
            owner_principal_id,
            owner_added_by,
        })
    }

    /// Organization values to create or match semantically.
    #[must_use]
    pub const fn org(&self) -> &TenantOrgCreate {
        &self.org
    }
    /// Default workspace values to create or match semantically.
    #[must_use]
    pub const fn default_workspace(&self) -> &TenantDefaultWorkspaceCreate {
        &self.default_workspace
    }
    /// Principal domain for the initial owner.
    #[must_use]
    pub const fn owner_principal_kind(&self) -> PrincipalKind {
        self.owner_principal_kind
    }
    /// Initial owner's opaque principal identifier.
    #[must_use]
    pub fn owner_principal_id(&self) -> &str {
        &self.owner_principal_id
    }
    /// Actor recorded as the source of the owner grant.
    #[must_use]
    pub fn owner_added_by(&self) -> Option<&str> {
        self.owner_added_by.as_deref()
    }
}

/// A tenant-provisioning command is missing a required semantic value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("tenant provisioning request is invalid")]
pub struct TenantProvisioningRequestError;

/// Why tenant provisioning could not create or replay the requested state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantProvisioningConflict {
    /// Some durable identity or active slug is already bound differently.
    ExistingState,
}

/// Result of an atomic tenant-provisioning attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantProvisioningOutcome {
    /// All three tenant records were created atomically.
    Created,
    /// All three records already matched the exact semantic request.
    Replayed,
    /// Existing state prevented creation and no state was changed.
    Conflict(TenantProvisioningConflict),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> (TenantOrgCreate, TenantDefaultWorkspaceCreate) {
        let org = TenantOrgCreate::new(
            "org".into(),
            "org".into(),
            "Org".into(),
            "user".into(),
            "free".into(),
            None,
            serde_json::json!({}),
        )
        .unwrap();
        let workspace = TenantDefaultWorkspaceCreate::new(
            "workspace".into(),
            "default".into(),
            "Default".into(),
            None,
            "user".into(),
            serde_json::json!({}),
        )
        .unwrap();
        (org, workspace)
    }

    #[test]
    fn command_contains_semantics_but_no_backend_authored_state() {
        let (org, workspace) = values();
        let request = TenantProvisioningRequest::new(
            org.clone(),
            workspace.clone(),
            PrincipalKind::User,
            "owner".into(),
            Some("bootstrap".into()),
        )
        .unwrap();
        assert_eq!(request.org(), &org);
        assert_eq!(request.default_workspace(), &workspace);
        assert_eq!(request.owner_principal_id(), "owner");
        assert_eq!(request.owner_added_by(), Some("bootstrap"));
    }

    #[test]
    fn constructors_reject_empty_stable_values() {
        assert!(
            TenantOrgCreate::new(
                String::new(),
                "org".into(),
                "Org".into(),
                "user".into(),
                "free".into(),
                None,
                serde_json::json!({}),
            )
            .is_err()
        );
        assert!(
            TenantDefaultWorkspaceCreate::new(
                "workspace".into(),
                "default".into(),
                "Default".into(),
                None,
                String::new(),
                serde_json::json!({}),
            )
            .is_err()
        );
        let (org, workspace) = values();
        assert!(
            TenantProvisioningRequest::new(
                org,
                workspace,
                PrincipalKind::User,
                String::new(),
                None,
            )
            .is_err()
        );
    }
}
