//! Frozen request identity for historical tenant-provisioning receipts.

use nebula_storage_port::{
    StorageError,
    dto::{TenantProvisioningConflict, TenantProvisioningOutcome, TenantProvisioningRequest},
};
use sha2::{Digest, Sha256};

pub(crate) const REQUEST_VERSION: i64 = 1;

/// Version 1 hashes all caller-owned fields as compact JSON with recursively
/// sorted object keys. Storage-authored instants and subsequent tenant edits
/// are excluded. Preserve this encoding when extending the provisioning command.
pub(crate) fn request_digest(
    request: &TenantProvisioningRequest,
) -> Result<[u8; 32], StorageError> {
    let mut value = encode_request(request);
    value.sort_all_objects();
    Ok(Sha256::digest(serde_json::to_vec(&value)?).into())
}

/// Frozen version-1 command shared by receipts and operator enrollment.
pub(crate) fn encode_request(request: &TenantProvisioningRequest) -> serde_json::Value {
    let org = request.org();
    let workspace = request.default_workspace();
    serde_json::json!({
        "version": REQUEST_VERSION,
        "org": {
            "id": org.id(), "slug": org.slug(), "display_name": org.display_name(),
            "created_by": org.created_by(), "plan": org.plan(),
            "billing_email": org.billing_email(), "settings": org.settings(),
        },
        "workspace": {
            "id": workspace.id(), "slug": workspace.slug(),
            "display_name": workspace.display_name(), "description": workspace.description(),
            "created_by": workspace.created_by(), "settings": workspace.settings(),
        },
        "owner": {
            "kind": request.owner_principal_kind().as_str(),
            "id": request.owner_principal_id(), "added_by": request.owner_added_by(),
        },
    })
}

/// Rebuild through validated constructors; stored JSON is never trusted input.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn decode_request(
    value: serde_json::Value,
) -> Result<TenantProvisioningRequest, StorageError> {
    use nebula_storage_port::dto::{PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate};
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Org {
        id: String,
        slug: String,
        display_name: String,
        created_by: String,
        plan: String,
        billing_email: Option<String>,
        settings: serde_json::Value,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Workspace {
        id: String,
        slug: String,
        display_name: String,
        description: Option<String>,
        created_by: String,
        settings: serde_json::Value,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Owner {
        kind: String,
        id: String,
        added_by: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Command {
        version: i64,
        org: Org,
        workspace: Workspace,
        owner: Owner,
    }

    fn corrupt() -> StorageError {
        StorageError::Corrupt("invalid stored tenant provisioning command".into())
    }
    let command: Command = serde_json::from_value(value).map_err(|_| corrupt())?;
    if command.version != REQUEST_VERSION {
        return Err(corrupt());
    }
    let kind = match command.owner.kind.as_str() {
        "user" => PrincipalKind::User,
        "service_account" => PrincipalKind::ServiceAccount,
        _ => return Err(corrupt()),
    };
    let org = command.org;
    let workspace = command.workspace;
    TenantProvisioningRequest::new(
        TenantOrgCreate::new(
            org.id,
            org.slug,
            org.display_name,
            org.created_by,
            org.plan,
            org.billing_email,
            org.settings,
        )
        .map_err(|_| corrupt())?,
        TenantDefaultWorkspaceCreate::new(
            workspace.id,
            workspace.slug,
            workspace.display_name,
            workspace.description,
            workspace.created_by,
            workspace.settings,
        )
        .map_err(|_| corrupt())?,
        kind,
        command.owner.id,
        command.owner.added_by,
    )
    .map_err(|_| corrupt())
}

/// Decode the two legal receipt forms; unknown encodings fail closed.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn replay_outcome(
    version: Option<i64>,
    stored_digest: Option<Vec<u8>>,
    workspace_id: Option<String>,
    expected_digest: &[u8; 32],
) -> Result<TenantProvisioningOutcome, StorageError> {
    match (version, stored_digest, workspace_id) {
        (None, None, None) => {
            tracing::debug!(
                outcome = "preexisting",
                "tenant provisioning identity is sealed"
            );
            Ok(TenantProvisioningOutcome::Conflict(
                TenantProvisioningConflict::PreexistingTenant,
            ))
        },
        (Some(REQUEST_VERSION), Some(digest), Some(_)) if digest.len() == 32 => {
            Ok(compare_digest(&digest, expected_digest))
        },
        _ => Err(StorageError::Corrupt(
            "invalid tenant provisioning receipt".into(),
        )),
    }
}

pub(crate) fn compare_digest(stored: &[u8], expected: &[u8; 32]) -> TenantProvisioningOutcome {
    if stored == expected {
        tracing::debug!(outcome = "replayed", "tenant provisioning receipt matched");
        TenantProvisioningOutcome::Replayed
    } else {
        tracing::debug!(
            outcome = "request_mismatch",
            "tenant provisioning receipt rejected the request"
        );
        TenantProvisioningOutcome::Conflict(TenantProvisioningConflict::RequestMismatch)
    }
}
