//! Application State
//!
//! Shared state для всех handlers через Arc.
//! Содержит только порты (traits) — не зависит от конкретных реализаций.

use nebula_config::Config;
use nebula_storage::{ExecutionRepo, WorkflowRepo};
use std::sync::Arc;

/// Application State, передаваемый через Router::with_state
#[derive(Clone)]
pub struct AppState {
    /// Configuration
    pub config: Arc<Config>,
    
    /// Workflow Repository (port/trait)
    pub workflow_repo: Arc<dyn WorkflowRepo>,
    
    /// Execution Repository (port/trait)
    pub execution_repo: Arc<dyn ExecutionRepo>,
    
    // TODO: Добавить другие порты по мере необходимости:
    // pub task_queue: Arc<dyn TaskQueue>,
    // pub credential_store: Arc<dyn CredentialStore>,
}

impl AppState {
    /// Create new AppState with provided dependencies
    pub fn new(
        config: Config,
        workflow_repo: Arc<dyn WorkflowRepo>,
        execution_repo: Arc<dyn ExecutionRepo>,
    ) -> Self {
        Self {
            config: Arc::new(config),
            workflow_repo,
            execution_repo,
        }
    }
}


