//! Tenancy resolution middleware.
//!
//! Parses path segments from `/api/v1/orgs/{org}/workspaces/{ws}/...`
//! and resolves slug-or-ULID identifiers to typed IDs via [`OrgResolver`]
//! and [`WorkspaceResolver`] port traits.

use std::str::FromStr;

use axum::{
    extract::{OriginalUri, Request, State},
    middleware::Next,
    response::Response,
};
use nebula_core::{
    CredentialId, ExecutionId, OrgId, ResolvedIds, WorkflowId, WorkspaceId, slug::is_prefixed_ulid,
};

use crate::{errors::ApiError, state::AppState};

/// Middleware that resolves org and workspace identifiers from the URL path.
///
/// After this middleware runs, [`ResolvedIds`] is available in request extensions.
/// Handlers and downstream middleware can extract it via `Extension<ResolvedIds>`.
pub async fn tenancy_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // Use OriginalUri when available (set by axum's `nest()`) so
    // that the full path including `/api/v1` is visible even when
    // the router is nested and the request URI has been stripped.
    let path = request
        .extensions()
        .get::<OriginalUri>()
        .map(|ou| ou.path().to_owned())
        .unwrap_or_else(|| request.uri().path().to_owned());
    let resolved = resolve_path_ids(&state, &path).await?;

    if let Some(ids) = resolved {
        request.extensions_mut().insert(ids);
    }

    Ok(next.run(request).await)
}

/// Parse the API path and resolve identifiers.
/// Returns `None` for paths that don't have tenant segments.
async fn resolve_path_ids(state: &AppState, path: &str) -> Result<Option<ResolvedIds>, ApiError> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    // We need at least: api / v1 / orgs / {org}
    // Index:             0     1    2      3
    if segments.len() < 4 || segments[0] != "api" || segments[1] != "v1" {
        return Ok(None);
    }

    // Handle /api/v1/hooks/{org}/{ws}/{trigger} — special webhook path
    if segments[2] == "hooks" {
        return resolve_webhook_path(state, &segments).await;
    }

    // Handle /api/v1/orgs/{org}/...
    if segments[2] != "orgs" {
        return Ok(None);
    }

    let mut ids = ResolvedIds::default();

    // Resolve org
    let org_segment = segments[3];
    ids.org_id = Some(resolve_org(state, org_segment).await?);

    // Check for /workspaces/{ws}/...
    // segments: api/v1/orgs/{org}/workspaces/{ws}/...
    // Index:     0   1    2    3       4       5
    if segments.len() >= 6 && segments[4] == "workspaces" {
        let ws_segment = segments[5];
        let org_id = ids.org_id.expect("org_id just resolved");
        ids.workspace_id = Some(resolve_workspace(state, org_id, ws_segment).await?);

        // Check for nested resource identifiers:
        // /workspaces/{ws}/workflows/{wf}
        // /workspaces/{ws}/executions/{exec}
        // /workspaces/{ws}/credentials/{cred}
        if segments.len() >= 8 {
            match segments[6] {
                "workflows" => {
                    ids.workflow_id = Some(resolve_typed_id::<WorkflowId>(segments[7])?);
                },
                "executions" => {
                    ids.execution_id = Some(resolve_typed_id::<ExecutionId>(segments[7])?);
                },
                "credentials" => {
                    ids.credential_id = Some(resolve_typed_id::<CredentialId>(segments[7])?);
                },
                _ => {},
            }
        }
    }

    Ok(Some(ids))
}

/// Resolve webhook path: `/api/v1/hooks/{org}/{ws}/{trigger_slug}`
async fn resolve_webhook_path(
    state: &AppState,
    segments: &[&str],
) -> Result<Option<ResolvedIds>, ApiError> {
    if segments.len() < 5 {
        return Ok(None);
    }

    let mut ids = ResolvedIds {
        org_id: Some(resolve_org(state, segments[3]).await?),
        ..Default::default()
    };

    if segments.len() >= 6 {
        let org_id = ids.org_id.expect("org_id just resolved");
        ids.workspace_id = Some(resolve_workspace(state, org_id, segments[4]).await?);
    }

    // trigger_slug is left as a path parameter for the handler
    Ok(Some(ids))
}

/// Resolve an org identifier — either prefixed ULID or slug.
async fn resolve_org(state: &AppState, segment: &str) -> Result<OrgId, ApiError> {
    if is_prefixed_ulid(segment) {
        OrgId::from_str(segment)
            .map_err(|_| ApiError::NotFound(format!("invalid org identifier: {segment}")))
    } else {
        // Resolve slug via OrgResolver port
        match &state.org_resolver {
            Some(resolver) => resolver.resolve_by_slug(segment).await,
            None => Err(ApiError::NotFound(format!("org not found: {segment}"))),
        }
    }
}

/// Resolve a workspace identifier — either prefixed ULID or slug within an org.
async fn resolve_workspace(
    state: &AppState,
    org_id: OrgId,
    segment: &str,
) -> Result<WorkspaceId, ApiError> {
    if is_prefixed_ulid(segment) {
        WorkspaceId::from_str(segment)
            .map_err(|_| ApiError::NotFound(format!("invalid workspace identifier: {segment}")))
    } else {
        match &state.workspace_resolver {
            Some(resolver) => resolver.resolve_by_slug(org_id, segment).await,
            None => Err(ApiError::NotFound(format!(
                "workspace not found: {segment}"
            ))),
        }
    }
}

/// Resolve a typed ID from a path segment.
///
/// For resource-level IDs (workflow, execution, credential), the segment must
/// parse as the typed ID. Slug resolution for sub-resources will be added when
/// the repository layer is integrated.
fn resolve_typed_id<T: FromStr>(segment: &str) -> Result<T, ApiError>
where
    T::Err: std::fmt::Display,
{
    T::from_str(segment)
        .map_err(|e| ApiError::NotFound(format!("invalid identifier '{segment}': {e}")))
}
