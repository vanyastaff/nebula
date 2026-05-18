//! First-party API access grants and scope parsing.

pub mod grant;
pub mod layer;
pub mod route;
pub mod scope;

pub use grant::{AccessDenied, Grant};
pub use layer::require_permission;
pub use route::{
    AccessCoverageError, REQUIRED_PERMISSION_EXTENSION, assert_tenant_access_coverage, protected,
};
pub use scope::{
    FULL_ACCESS_SCOPE, ScopeParseError, UNSUPPORTED_PERMISSION_SCOPE, parse_pat_grant,
    permission_from_scope, permission_scope, validate_new_pat_scopes,
};
