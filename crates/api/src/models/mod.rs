//! Models (DTOs)
//!
//! Request and response models for API endpoints.
//!
//! Per ADR-0047 cross-layer schema strategy, every type that crosses the API
//! boundary lives here and derives `utoipa::ToSchema`. Cross-layer types
//! from `nebula-core` / `nebula-storage` / `nebula-engine` /
//! `nebula-credential` are wrapped at this boundary (e.g. [`me::OrgRoleDto`])
//! so the OpenAPI contract is decoupled from internal type evolution.

pub mod catalog;
pub mod credential;
pub mod execution;
pub mod health;
pub mod me;
pub mod org;
pub mod pagination;
pub mod resource;
pub mod system;
pub mod workflow;

pub use catalog::{
    ActionDetailResponse, ActionSummary, ListActionsResponse, ListPluginsResponse,
    PluginDetailResponse, PluginSummary,
};
pub use execution::{
    ExecutionLogsResponse, ExecutionOutputsResponse, ExecutionResponse, ListExecutionsResponse,
    RunningExecutionSummary, StartExecutionRequest,
};
pub use health::{DependenciesStatus, HealthResponse, ReadinessResponse, VersionInfo};
pub use me::{
    CreateTokenRequest, CreateTokenResponse, MeResponse, MyOrgsResponse, MyTokensResponse,
    OrgRoleDto, OrgSummary, TokenSummary, UpdateMeRequest, WorkspaceRoleDto,
};
pub use org::{
    CreateServiceAccountRequest, CreateServiceAccountResponse, InviteMemberRequest,
    InviteMemberResponse, MemberSummary, MembersResponse, OrgResponse, ServiceAccountSummary,
    ServiceAccountsResponse, UpdateOrgRequest,
};
pub use pagination::{CursorParams, CursorPayload, PaginatedResponse};
pub use resource::{
    CreateResourceRequest, CreateResourceResponse, ListResourcesResponse, ResourcePhase,
    ResourceStatusDto, ResourceSummary, UpdateResourceRequest, UpdateResourceResponse,
};
pub use system::AckResponse;
pub use workflow::{
    CreateWorkflowRequest, ListWorkflowsResponse, UpdateWorkflowRequest, WorkflowResponse,
    WorkflowValidateResponse,
};
