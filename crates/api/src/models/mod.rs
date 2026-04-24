//! Models (DTOs)
//!
//! Request and response models for API endpoints.

pub mod catalog;
pub mod credential;
pub mod execution;
pub mod health;
pub mod workflow;

pub use catalog::{
    ActionDetailResponse, ActionSummary, ListActionsResponse, ListPluginsResponse,
    PluginDetailResponse, PluginSummary,
};
pub use execution::{
    ExecutionLogsResponse, ExecutionOutputsResponse, ExecutionResponse, ListExecutionsResponse,
    RunningExecutionSummary, StartExecutionRequest,
};
pub use health::{DependenciesStatus, HealthResponse, ReadinessResponse};
pub use workflow::{
    CreateWorkflowRequest, ListWorkflowsResponse, UpdateWorkflowRequest, WorkflowResponse,
    WorkflowValidateResponse,
};
