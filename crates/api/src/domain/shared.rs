//! Server pagination cursors and tenant-role mappings.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
pub use nebula_api_contract::v1::shared::*;
use serde::{Deserialize, Serialize};

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

/// The canonical lowercase wire token for an [`nebula_core::OrgRole`].
///
/// This is the **single** place the `OrgRole` ↔ wire-string mapping
/// lives (3: the public spec must not embed `nebula_core`
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
/// at the mapping, not silently shipped (honest capability contract).
#[must_use]
pub(crate) fn org_role_token(role: nebula_core::OrgRole) -> &'static str {
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
/// that to a 400 (RFC 9457) rather than guessing a role (honest capability contract:
/// no silent coercion of an unrecognised privilege level).
#[must_use]
pub(crate) fn parse_org_role(token: &str) -> Option<nebula_core::OrgRole> {
    use nebula_core::OrgRole::{OrgAdmin, OrgBilling, OrgMember, OrgOwner};
    match token {
        "member" => Some(OrgMember),
        "billing" => Some(OrgBilling),
        "admin" => Some(OrgAdmin),
        "owner" => Some(OrgOwner),
        _ => None,
    }
}

pub(crate) fn org_role_dto(role: nebula_core::OrgRole) -> OrgRoleDto {
    OrgRoleDto(org_role_token(role).to_owned())
}

/// Stable public token for an internal workspace role.
#[must_use]
pub(crate) fn workspace_role_token(role: nebula_core::WorkspaceRole) -> &'static str {
    use nebula_core::WorkspaceRole::{
        WorkspaceAdmin, WorkspaceEditor, WorkspaceRunner, WorkspaceViewer,
    };
    match role {
        WorkspaceViewer => "viewer",
        WorkspaceRunner => "runner",
        WorkspaceEditor => "editor",
        WorkspaceAdmin => "admin",
        unknown => {
            debug_assert!(
                false,
                "nebula_core::WorkspaceRole gained a variant {unknown:?} without a wire token"
            );
            "viewer"
        },
    }
}

/// Parse one canonical public token without accepting internal enum names.
#[must_use]
pub(crate) fn parse_workspace_role(token: &str) -> Option<nebula_core::WorkspaceRole> {
    use nebula_core::WorkspaceRole::{
        WorkspaceAdmin, WorkspaceEditor, WorkspaceRunner, WorkspaceViewer,
    };
    match token {
        "viewer" => Some(WorkspaceViewer),
        "runner" => Some(WorkspaceRunner),
        "editor" => Some(WorkspaceEditor),
        "admin" => Some(WorkspaceAdmin),
        _ => None,
    }
}

pub(crate) fn workspace_role_dto(role: nebula_core::WorkspaceRole) -> WorkspaceRoleDto {
    WorkspaceRoleDto(workspace_role_token(role).to_owned())
}
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
            let token = org_role_token(role);
            assert_eq!(
                parse_org_role(token),
                Some(role),
                "token `{token}` must round-trip back to {role:?}"
            );
            assert_eq!(org_role_dto(role).0, token);
        }
        assert_eq!(
            parse_org_role("superuser"),
            None,
            "unknown role tokens must not silently coerce"
        );
        // Internal Rust enum names must NOT be accepted as wire tokens.
        assert_eq!(parse_org_role("OrgOwner"), None);
    }

    #[test]
    fn workspace_role_token_roundtrips_every_variant() {
        use nebula_core::WorkspaceRole;
        for role in [
            WorkspaceRole::WorkspaceViewer,
            WorkspaceRole::WorkspaceRunner,
            WorkspaceRole::WorkspaceEditor,
            WorkspaceRole::WorkspaceAdmin,
        ] {
            let token = workspace_role_token(role);
            assert_eq!(parse_workspace_role(token), Some(role));
            assert_eq!(workspace_role_dto(role).0, token);
        }
        assert_eq!(parse_workspace_role("WorkspaceAdmin"), None);
    }
}
