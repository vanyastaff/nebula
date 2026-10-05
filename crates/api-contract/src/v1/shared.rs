//! Cross-domain shared DTOs.
//!
//! Hosts response/query shapes that are not specific to a single domain
//! module: cursor-based pagination, the offset/page-based
//! [`PaginationParams`] used by the workflow/execution list endpoints, and
//! the canonical "operation succeeded" acknowledgement.
//!
//! All list endpoints use opaque cursor pagination per spec §05. Cursors
//! are base64-encoded JSON payloads — never parsed by clients.

use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::{IntoParams, ToSchema};

/// Query parameters for cursor-based pagination.
///
/// `IntoParams` exposes both fields as individual `query` parameters in the
/// OpenAPI spec; the field-level `schema` attributes propagate so consumers
/// see the cursor as an opaque string.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct CursorParams {
    /// Opaque cursor from a previous response's `next_cursor`.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", param(nullable = false))]
    pub cursor: Option<String>,
    /// Maximum number of items to return. Capped at `PaginationConfig::max_limit`.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", param(nullable = false))]
    pub limit: Option<u32>,
}

/// A paginated response envelope.
///
/// Concrete instantiations are inlined into the OpenAPI spec at the path
/// where they appear (utoipa expands the generic at `#[utoipa::path]` time);
/// no `aliases(...)` registration is required for the path to compile.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PaginatedResponse<T> {
    /// The items on this page.
    pub items: Vec<T>,
    /// Opaque cursor for fetching the next page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Whether more items exist beyond this page.
    pub has_more: bool,
}

impl<T> PaginatedResponse<T> {
    /// Create a response page.
    pub fn new(items: Vec<T>, next_cursor: Option<String>, has_more: bool) -> Self {
        Self {
            items,
            next_cursor,
            has_more,
        }
    }

    /// Convenience: create a final page with no more results.
    pub fn last_page(items: Vec<T>) -> Self {
        Self {
            items,
            next_cursor: None,
            has_more: false,
        }
    }
}

/// Offset/page-based pagination query parameters.
///
/// Used by the workflow and execution list endpoints. `unused_qualifications`
/// is silenced where this is consumed because the `IntoParams`-derived type
/// triggers it from inside the `#[utoipa::path(... params(PaginationParams))]`
/// expansion (utoipa 5.5 macro-generated code paths qualify the type).
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct PaginationParams {
    /// Page number (1-indexed)
    #[serde(default = "default_page")]
    #[cfg_attr(feature = "openapi", param(minimum = 1))]
    pub page: usize,
    /// Page size (default 10, max 100)
    #[serde(default = "default_page_size")]
    #[cfg_attr(feature = "openapi", param(minimum = 1, maximum = 100))]
    pub page_size: usize,
}

fn default_page() -> usize {
    1
}

fn default_page_size() -> usize {
    10
}

impl PaginationParams {
    /// Calculate offset for database query (0-indexed)
    pub fn offset(&self) -> usize {
        self.page.saturating_sub(1).saturating_mul(self.page_size)
    }

    /// Get validated limit (capped at 100)
    pub fn limit(&self) -> usize {
        self.page_size.min(100)
    }
}

/// Generic acknowledgement response for endpoints that have no other body
/// to return on success (delete, password reset, email verification, …).
///
/// Replaces the `Json(json!({"deleted": true}))` / `Json(json!({"reset": true}))`
/// idioms documented in the M3.2 audit so the OpenAPI spec advertises one
/// shape per success acknowledgement instead of N ad-hoc schemas.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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

/// Wrapper around `nebula_core::OrgRole` exposed at the API boundary.
///
/// Per stub-endpoint policy cross-layer schema strategy, the API contract MUST NOT embed
/// `nebula_core` types directly in OpenAPI components. The string form is
/// stable across role-set evolutions in the core crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(transparent)]
#[cfg_attr(feature = "openapi", schema(value_type = String, example = "owner"))]
pub struct OrgRoleDto(pub String);

/// Wrapper around `nebula_core::WorkspaceRole`.
///
/// Same reasoning as [`OrgRoleDto`] — wrap at the API boundary so
/// workspace-role taxonomy changes in `nebula-core` don't ripple into the
/// public spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(transparent)]
#[cfg_attr(feature = "openapi", schema(value_type = WorkspaceRoleSchema, example = "editor"))]
pub struct WorkspaceRoleDto(pub String);

/// Closed OpenAPI vocabulary for [`WorkspaceRoleDto`].
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
#[cfg_attr(
    not(feature = "openapi"),
    expect(
        dead_code,
        reason = "schema-only enum defines the closed wire vocabulary"
    )
)]
enum WorkspaceRoleSchema {
    Viewer,
    Runner,
    Editor,
    Admin,
}
