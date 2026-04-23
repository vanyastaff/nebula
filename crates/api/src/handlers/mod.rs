//! Handlers
//!
//! Thin HTTP endpoint handlers.
//! Each handler extracts data from the request and delegates to a service or port.

pub mod auth;
pub mod catalog;
#[cfg(feature = "credential-oauth")]
pub mod credential;
#[cfg(feature = "credential-oauth")]
pub mod credential_oauth;
pub mod execution;
pub mod health;
pub mod me;
pub mod openapi;
pub mod org;
pub mod resource;
pub mod webhook;
pub mod workflow;

pub use catalog::{get_action, get_plugin, list_actions, list_plugins};
pub use execution::{
    cancel_execution, get_execution, get_execution_logs, get_execution_outputs, list_executions,
    start_execution,
};
pub use health::{health_check, readiness_check, version_info};
pub use workflow::{
    activate_workflow, create_workflow, delete_workflow, execute_workflow, get_workflow,
    list_workflows, update_workflow, validate_workflow_handler,
};
