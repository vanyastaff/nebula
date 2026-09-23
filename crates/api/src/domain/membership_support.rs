//! Shared HTTP-boundary primitives for membership endpoints.

use std::{str::FromStr, sync::Arc};

use nebula_core::{Principal, ServiceAccountId, UserId};

use crate::{
    error::ApiError,
    state::{AppState, MembershipStore},
};

pub(super) fn store(state: &AppState) -> Result<&Arc<dyn MembershipStore>, ApiError> {
    state.membership_store.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("membership store is not configured".to_owned())
    })
}

pub(super) fn parse_principal(raw: &str) -> Result<Principal, ApiError> {
    if let Ok(id) = UserId::from_str(raw) {
        return Ok(Principal::User(id));
    }
    if let Ok(id) = ServiceAccountId::from_str(raw) {
        return Ok(Principal::ServiceAccount(id));
    }
    Err(ApiError::validation_message(
        "principal_id must be a `usr_<ULID>` or `svc_<ULID>` identity".to_owned(),
    ))
}

pub(super) fn principal_id(principal: &Principal) -> Result<String, ApiError> {
    match principal {
        Principal::User(id) => Ok(id.to_string()),
        Principal::ServiceAccount(id) => Ok(id.to_string()),
        _ => Err(ApiError::Internal(
            "membership authority returned an unsupported principal kind".to_owned(),
        )),
    }
}
