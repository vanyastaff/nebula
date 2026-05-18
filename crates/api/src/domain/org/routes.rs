//! Organization-level routes — authenticated + org-scoped.
//!
//! The **member** routes (`list_members` / `add_member` / `remove_member`)
//! are live (real 200/201 + typed errors). The **org-record** and
//! **service-account** routes are still honest-501 stubs marked
//! `#[deprecated]` so the generated OpenAPI spec flags them per stub-endpoint policy
//! Stub Endpoint Policy. The deprecation lint is silenced at module level
//! because the stub handlers are intentionally mounted (returning 501) so
//! the route table stays in sync with the published spec — the
//! non-deprecated member handlers are unaffected by the allow.
#![allow(deprecated)]

use nebula_core::Permission;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handler;
use crate::{access, state::AppState};

/// Organization routes under `/api/v1/orgs/{org}/*`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(access::protected(
            Permission::OrgRead,
            routes!(handler::get_org),
        ))
        .routes(access::protected(
            Permission::OrgUpdate,
            routes!(handler::update_org),
        ))
        .routes(access::protected(
            Permission::OrgDelete,
            routes!(handler::delete_org),
        ))
        .routes(access::protected(
            Permission::MemberRead,
            routes!(handler::list_members),
        ))
        .routes(access::protected(
            Permission::MemberInvite,
            routes!(handler::add_member),
        ))
        .routes(access::protected(
            Permission::MemberRemove,
            routes!(handler::remove_member),
        ))
        .routes(access::protected(
            Permission::ServiceAccountManage,
            routes!(
                handler::list_service_accounts,
                handler::create_service_account,
                handler::delete_service_account
            ),
        ))
}
