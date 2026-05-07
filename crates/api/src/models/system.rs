//! System-level cross-cutting DTOs.
//!
//! Hosts response shapes that are not specific to a single domain
//! module — currently the canonical "operation succeeded" acknowledgement.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Generic acknowledgement response for endpoints that have no other body
/// to return on success (delete, password reset, email verification, …).
///
/// Replaces the `Json(json!({"deleted": true}))` / `Json(json!({"reset": true}))`
/// idioms documented in the M3.2 audit so the OpenAPI spec advertises one
/// shape per success acknowledgement instead of N ad-hoc schemas.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AckResponse {
    /// `true` on the success path. Always present so consumers can match on
    /// the field unconditionally.
    pub ok: bool,
}

impl AckResponse {
    /// Build an `ok = true` acknowledgement.
    #[must_use]
    pub fn ok() -> Self {
        Self { ok: true }
    }
}
