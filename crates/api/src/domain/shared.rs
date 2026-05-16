//! Cross-domain shared DTOs.
//!
//! Hosts response/query shapes that are not specific to a single domain
//! module: cursor-based pagination, the offset/page-based
//! [`PaginationParams`] used by the workflow/execution list endpoints, and
//! the canonical "operation succeeded" acknowledgement.
//!
//! All list endpoints use opaque cursor pagination per spec §05. Cursors
//! are base64-encoded JSON payloads — never parsed by clients.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Query parameters for cursor-based pagination.
///
/// `IntoParams` exposes both fields as individual `query` parameters in the
/// OpenAPI spec; the field-level `schema` attributes propagate so consumers
/// see the cursor as an opaque string.
#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CursorParams {
    /// Opaque cursor from a previous response's `next_cursor`.
    #[serde(default)]
    #[param(nullable = false)]
    pub cursor: Option<String>,
    /// Maximum number of items to return. Capped at `PaginationConfig::max_limit`.
    #[serde(default)]
    #[param(nullable = false)]
    pub limit: Option<u32>,
}

/// A paginated response envelope.
///
/// Concrete instantiations are inlined into the OpenAPI spec at the path
/// where they appear (utoipa expands the generic at `#[utoipa::path]` time);
/// no `aliases(...)` registration is required for the path to compile.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PaginatedResponse<T> {
    /// The items on this page.
    pub items: Vec<T>,
    /// Opaque cursor for fetching the next page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Whether more items exist beyond this page.
    pub has_more: bool,
}

impl<T: Serialize + ToSchema> PaginatedResponse<T> {
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

/// Internal cursor payload. Encoded/decoded as base64 JSON.
/// Not exposed to API clients — they see an opaque string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPayload {
    /// The ID of the last item on the current page.
    pub last_id: String,
    /// Optional secondary sort key for deterministic ordering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sort_key: Option<String>,
}

impl CursorPayload {
    /// Encode this payload into an opaque cursor string.
    pub fn encode(&self) -> Result<String, CursorError> {
        let json = serde_json::to_vec(self).map_err(|e| CursorError::Encode(e.to_string()))?;
        Ok(URL_SAFE_NO_PAD.encode(&json))
    }

    /// Decode an opaque cursor string back into a payload.
    pub fn decode(cursor: &str) -> Result<Self, CursorError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|e| CursorError::Decode(e.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|e| CursorError::Decode(e.to_string()))
    }
}

/// Errors from cursor encoding/decoding.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CursorError {
    /// Failed to serialize cursor payload.
    #[error("failed to encode cursor: {0}")]
    Encode(String),
    /// Failed to deserialize or base64-decode cursor string.
    #[error("invalid cursor: {0}")]
    Decode(String),
}

/// Offset/page-based pagination query parameters.
///
/// Used by the workflow and execution list endpoints. `unused_qualifications`
/// is silenced where this is consumed because the `IntoParams`-derived type
/// triggers it from inside the `#[utoipa::path(... params(PaginationParams))]`
/// expansion (utoipa 5.5 macro-generated code paths qualify the type).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PaginationParams {
    /// Page number (1-indexed)
    #[serde(default = "default_page")]
    #[param(minimum = 1)]
    pub page: usize,
    /// Page size (default 10, max 100)
    #[serde(default = "default_page_size")]
    #[param(minimum = 1, maximum = 100)]
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

/// Wrapper around `nebula_core::OrgRole` exposed at the API boundary.
///
/// Per ADR-0047 cross-layer schema strategy, the API contract MUST NOT embed
/// `nebula_core` types directly in OpenAPI components. The string form is
/// stable across role-set evolutions in the core crate.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "owner")]
pub struct OrgRoleDto(pub String);

impl OrgRoleDto {
    /// The canonical lowercase wire token for an [`nebula_core::OrgRole`].
    ///
    /// This is the **single** place the `OrgRole` ↔ wire-string mapping
    /// lives (ADR-0047 §3: the public spec must not embed `nebula_core`
    /// enum names — `OrgMember`/`OrgOwner` are internal Rust identifiers).
    /// The tokens (`member`/`billing`/`admin`/`owner`) are stable across
    /// core role-set evolution.
    ///
    /// `OrgRole` is `#[non_exhaustive]`. A future core variant with no
    /// token here **fails safe to the least-privilege token** (`member`)
    /// rather than fabricating a name or escalating — and
    /// `org_role_token_roundtrips_every_variant` (a unit test that
    /// enumerates the current set) plus this debug assertion make the
    /// omission loud the moment a variant is added, so the gap is fixed
    /// at the mapping, not silently shipped (canon §4.5).
    #[must_use]
    pub fn token(role: nebula_core::OrgRole) -> &'static str {
        use nebula_core::OrgRole::{OrgAdmin, OrgBilling, OrgMember, OrgOwner};
        match role {
            OrgMember => "member",
            OrgBilling => "billing",
            OrgAdmin => "admin",
            OrgOwner => "owner",
            unknown => {
                debug_assert!(
                    false,
                    "nebula_core::OrgRole gained a variant {unknown:?} with no \
                     OrgRoleDto wire token — add it to OrgRoleDto::token/parse"
                );
                "member"
            },
        }
    }

    /// Parse a wire token back into an [`nebula_core::OrgRole`].
    ///
    /// `None` for any token outside the canonical set — the handler maps
    /// that to a 400 (RFC 9457) rather than guessing a role (canon §4.5:
    /// no silent coercion of an unrecognised privilege level).
    #[must_use]
    pub fn parse(token: &str) -> Option<nebula_core::OrgRole> {
        use nebula_core::OrgRole::{OrgAdmin, OrgBilling, OrgMember, OrgOwner};
        match token {
            "member" => Some(OrgMember),
            "billing" => Some(OrgBilling),
            "admin" => Some(OrgAdmin),
            "owner" => Some(OrgOwner),
            _ => None,
        }
    }
}

impl From<nebula_core::OrgRole> for OrgRoleDto {
    fn from(role: nebula_core::OrgRole) -> Self {
        Self(Self::token(role).to_owned())
    }
}

/// Wrapper around `nebula_core::WorkspaceRole`.
///
/// Same reasoning as [`OrgRoleDto`] — wrap at the API boundary so
/// workspace-role taxonomy changes in `nebula-core` don't ripple into the
/// public spec.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "editor")]
pub struct WorkspaceRoleDto(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrip() {
        let payload = CursorPayload {
            last_id: "exe_01J9ABCDEF".to_string(),
            last_sort_key: Some("2026-01-01".to_string()),
        };
        let encoded = payload.encode().expect("encode must succeed");
        let decoded = CursorPayload::decode(&encoded).expect("decode must succeed");
        assert_eq!(decoded.last_id, "exe_01J9ABCDEF");
        assert_eq!(decoded.last_sort_key.as_deref(), Some("2026-01-01"));
    }

    #[test]
    fn cursor_decode_invalid_base64() {
        let err = CursorPayload::decode("not-valid-base64!!!").unwrap_err();
        assert!(matches!(err, CursorError::Decode(_)));
    }

    #[test]
    fn paginated_response_last_page() {
        use crate::domain::execution::dto::ExecutionResponse;
        let resp = PaginatedResponse::<ExecutionResponse>::last_page(vec![]);
        assert!(!resp.has_more);
        assert!(resp.next_cursor.is_none());
        assert_eq!(resp.items.len(), 0);
    }

    #[test]
    fn org_role_token_roundtrips_every_variant() {
        use nebula_core::OrgRole;
        for role in [
            OrgRole::OrgMember,
            OrgRole::OrgBilling,
            OrgRole::OrgAdmin,
            OrgRole::OrgOwner,
        ] {
            let token = OrgRoleDto::token(role);
            assert_eq!(
                OrgRoleDto::parse(token),
                Some(role),
                "token `{token}` must round-trip back to {role:?}"
            );
            assert_eq!(OrgRoleDto::from(role).0, token);
        }
        assert_eq!(
            OrgRoleDto::parse("superuser"),
            None,
            "unknown role tokens must not silently coerce"
        );
        // Internal Rust enum names must NOT be accepted as wire tokens.
        assert_eq!(OrgRoleDto::parse("OrgOwner"), None);
    }
}
