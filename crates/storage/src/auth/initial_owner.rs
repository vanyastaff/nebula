//! Identity-owned, permanent admission of one deployment's initial owner.

use nebula_core::UserId;
use nebula_storage_port::dto::TenantProvisioningRequest;

/// Initial account prepared by authentication policy, without email verification.
/// Password hashing and input validation happen before entering storage. The
/// non-secret tenant command is frozen with the account in one transaction.
pub struct InitialOwnerRegistration<'a> {
    /// New account identity, shared by the account and frozen owner grant.
    pub user_id: UserId,
    /// Normalized login email; operator enrollment does not verify this address.
    pub email: &'a str,
    /// Validated account display name.
    pub display_name: &'a str,
    /// Prepared password hash, never a plaintext password.
    pub password_hash: &'a str,
    /// Exact command the tenant owner may accept when enrollment resumes.
    pub tenant_request: &'a TenantProvisioningRequest,
}

impl std::fmt::Debug for InitialOwnerRegistration<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InitialOwnerRegistration")
            .finish_non_exhaustive()
    }
}

/// Outcome of trying to reserve initial ownership and create its account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum InitialOwnerBegin {
    /// Account and frozen tenant command committed together.
    Begun,
    /// Enrollment was already recorded; begin never replaces its identity or password.
    AlreadyStarted,
    /// Preexisting identity or tenant history permanently closed initial enrollment.
    Unavailable,
}

/// Durable enrollment progress, separate from current access to its tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum InitialOwnerStatus {
    /// No identity or tenant history has closed initial enrollment.
    Available,
    /// Prior deployment use permanently closed initial enrollment.
    Sealed,
    /// Identity committed; the tenant command has not yet been accepted.
    Pending,
    /// The pending enrollment account is missing or archived.
    OwnerUnavailable,
    /// The tenant receipt proves historical acceptance, not present membership.
    Accepted,
    /// Tenant history conflicts with the frozen enrollment command.
    Conflict,
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) enum Enrollment {
    Available,
    Sealed,
    Pending {
        user_id: [u8; 16],
        request: Box<TenantProvisioningRequest>,
    },
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn validate_registration(
    registration: &InitialOwnerRegistration<'_>,
) -> Result<(), crate::StorageError> {
    if registration.email.trim().is_empty()
        || registration.display_name.trim().is_empty()
        || registration.password_hash.is_empty()
        || !matches_owner(
            &registration.user_id.as_bytes(),
            registration.tenant_request,
        )
    {
        return Err(crate::StorageError::InvalidInput(
            "initial owner registration is invalid".into(),
        ));
    }
    Ok(())
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn decode_enrollment(
    state: String,
    user_id: Option<Vec<u8>>,
    request: Option<serde_json::Value>,
) -> Result<Enrollment, crate::StorageError> {
    let corrupt = || crate::StorageError::Corrupt("initial owner enrollment is invalid".into());
    match (state.as_str(), user_id, request) {
        ("available", None, None) => Ok(Enrollment::Available),
        ("sealed", None, None) => Ok(Enrollment::Sealed),
        ("enrolled", Some(user_id), Some(request)) => {
            let user_id = <[u8; 16]>::try_from(user_id).map_err(|_| corrupt())?;
            let request =
                crate::tenant_provisioning::decode_request(request).map_err(|_| corrupt())?;
            if !matches_owner(&user_id, &request) {
                return Err(corrupt());
            }
            Ok(Enrollment::Pending {
                user_id,
                request: Box::new(request),
            })
        },
        _ => Err(corrupt()),
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn matches_owner(user_id: &[u8; 16], request: &TenantProvisioningRequest) -> bool {
    use nebula_core::{OrgId, WorkspaceId};
    use nebula_storage_port::dto::PrincipalKind;

    // Membership lookup uses the canonical textual identity, not a parsed alias.
    let canonical_owner = UserId::from_bytes(*user_id).to_string();
    request.owner_principal_kind() == PrincipalKind::User
        && request.owner_principal_id() == canonical_owner
        && request.org().created_by() == request.owner_principal_id()
        && request.default_workspace().created_by() == request.owner_principal_id()
        && request.org().id().parse::<OrgId>().is_ok()
        && request
            .default_workspace()
            .id()
            .parse::<WorkspaceId>()
            .is_ok()
}

#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
mod tests {
    use super::*;
    use nebula_core::{OrgId, WorkspaceId};
    use nebula_storage_port::dto::{PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate};

    fn tenant_request(owner: UserId) -> TenantProvisioningRequest {
        let owner = owner.to_string();
        TenantProvisioningRequest::new(
            TenantOrgCreate::new(
                OrgId::new().to_string(),
                "personal".into(),
                "Personal".into(),
                owner.clone(),
                "free".into(),
                None,
                serde_json::json!({}),
            )
            .unwrap(),
            TenantDefaultWorkspaceCreate::new(
                WorkspaceId::new().to_string(),
                "default".into(),
                "Default".into(),
                None,
                owner.clone(),
                serde_json::json!({}),
            )
            .unwrap(),
            PrincipalKind::User,
            owner,
            Some("operator-enrollment".into()),
        )
        .unwrap()
    }

    #[test]
    fn registration_rejects_a_different_frozen_owner_without_disclosing_input() {
        let owner = UserId::new();
        let request = tenant_request(owner);
        let mut registration = InitialOwnerRegistration {
            user_id: owner,
            email: "private-email@example.com",
            display_name: "Private name",
            password_hash: "private-hash-canary",
            tenant_request: &request,
        };
        assert!(validate_registration(&registration).is_ok());
        let debug = format!("{registration:?}");
        for secret in [
            registration.email,
            registration.display_name,
            registration.password_hash,
        ] {
            assert!(!debug.contains(secret));
        }
        registration.user_id = UserId::new();
        assert!(matches!(
            validate_registration(&registration),
            Err(crate::StorageError::InvalidInput(_))
        ));
    }

    #[test]
    fn stored_enrollment_requires_an_exact_state_and_account_binding() {
        let owner = UserId::new();
        let request = crate::tenant_provisioning::encode_request(&tenant_request(owner));
        assert!(matches!(
            decode_enrollment(
                "enrolled".into(),
                Some(owner.as_bytes().to_vec()),
                Some(request.clone())
            ),
            Ok(Enrollment::Pending { .. })
        ));
        for (state, user_id, request) in [
            ("unknown", None, None),
            ("available", Some(owner.as_bytes().to_vec()), None),
            ("sealed", None, Some(request.clone())),
            ("enrolled", None, Some(request.clone())),
            ("enrolled", Some(vec![0; 15]), Some(request.clone())),
            (
                "enrolled",
                Some(UserId::new().as_bytes().to_vec()),
                Some(request),
            ),
        ] {
            assert!(matches!(
                decode_enrollment(state.into(), user_id, request),
                Err(crate::StorageError::Corrupt(_))
            ));
        }
    }
}
