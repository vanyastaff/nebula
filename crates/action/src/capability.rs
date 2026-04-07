//! Capability module interfaces for action and trigger contexts.
//!
//! These traits are object-safe boundaries injected by runtime/engine so
//! action code can access resources, credentials, and logging without
//! coupling to concrete manager implementations.

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nebula_core::id::ExecutionId;
use nebula_credential::CredentialSnapshot;

use crate::ActionError;

/// Object-safe resource accessor injected into [`crate::ActionContext`].
#[async_trait]
pub trait ResourceAccessor: Send + Sync {
    /// Acquire a resource by key.
    ///
    /// Returns a type-erased instance that action code can downcast.
    async fn acquire(&self, key: &str) -> Result<Box<dyn Any + Send + Sync>, ActionError>;

    /// Check whether a resource exists for the given key.
    async fn exists(&self, key: &str) -> bool;
}

/// Object-safe credential accessor injected into [`crate::ActionContext`].
#[async_trait]
pub trait CredentialAccessor: Send + Sync {
    /// Retrieve a credential snapshot by id.
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, ActionError>;

    /// Check whether a credential exists for the given id.
    async fn has(&self, id: &str) -> bool;
}

/// Log severity for action-scoped logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionLogLevel {
    /// Trace-level diagnostic event.
    Trace,
    /// Debug-level diagnostic event.
    Debug,
    /// Informational event.
    Info,
    /// Warning event.
    Warn,
    /// Error event.
    Error,
}

/// Object-safe logging capability injected into action contexts.
pub trait ActionLogger: Send + Sync {
    /// Emit a message at the given level.
    fn log(&self, level: ActionLogLevel, message: &str);
}

/// Object-safe scheduling capability injected into trigger contexts.
#[async_trait]
pub trait TriggerScheduler: Send + Sync {
    /// Schedule the next trigger run after the given delay.
    async fn schedule_after(&self, delay: Duration) -> Result<(), ActionError>;
}

/// Object-safe execution emission capability injected into trigger contexts.
#[async_trait]
pub trait ExecutionEmitter: Send + Sync {
    /// Start a new execution for this trigger's workflow with the given input.
    async fn emit(&self, input: serde_json::Value) -> Result<ExecutionId, ActionError>;
}

/// No-op logger used when runtime does not inject a logger capability.
#[derive(Debug, Default)]
pub struct NoopActionLogger;

impl ActionLogger for NoopActionLogger {
    fn log(&self, _level: ActionLogLevel, _message: &str) {}
}

/// No-op scheduler used when runtime does not inject trigger scheduling.
#[derive(Debug, Default)]
pub struct NoopTriggerScheduler;

#[async_trait]
impl TriggerScheduler for NoopTriggerScheduler {
    async fn schedule_after(&self, _delay: Duration) -> Result<(), ActionError> {
        Err(ActionError::fatal(
            "trigger scheduler capability is not configured in TriggerContext",
        ))
    }
}

/// No-op emitter used when runtime does not inject execution emission.
#[derive(Debug, Default)]
pub struct NoopExecutionEmitter;

#[async_trait]
impl ExecutionEmitter for NoopExecutionEmitter {
    async fn emit(&self, _input: serde_json::Value) -> Result<ExecutionId, ActionError> {
        Err(ActionError::fatal(
            "execution emitter capability is not configured in TriggerContext",
        ))
    }
}

/// No-op resource accessor used when runtime does not inject resources.
#[derive(Debug, Default)]
pub struct NoopResourceAccessor;

#[async_trait]
impl ResourceAccessor for NoopResourceAccessor {
    async fn acquire(&self, _key: &str) -> Result<Box<dyn Any + Send + Sync>, ActionError> {
        Err(ActionError::fatal(
            "resource capability is not configured in ActionContext",
        ))
    }

    async fn exists(&self, _key: &str) -> bool {
        false
    }
}

/// No-op credential accessor used when runtime does not inject credentials.
#[derive(Debug, Default)]
pub struct NoopCredentialAccessor;

#[async_trait]
impl CredentialAccessor for NoopCredentialAccessor {
    async fn get(&self, _id: &str) -> Result<CredentialSnapshot, ActionError> {
        Err(ActionError::fatal(
            "credential capability is not configured in ActionContext",
        ))
    }

    async fn has(&self, _id: &str) -> bool {
        false
    }
}

/// Default resource accessor capability.
#[must_use]
pub fn default_resource_accessor() -> Arc<dyn ResourceAccessor> {
    Arc::new(NoopResourceAccessor)
}

/// Default credential accessor capability.
#[must_use]
pub fn default_credential_accessor() -> Arc<dyn CredentialAccessor> {
    Arc::new(NoopCredentialAccessor)
}

/// Default action logger capability.
#[must_use]
pub fn default_action_logger() -> Arc<dyn ActionLogger> {
    Arc::new(NoopActionLogger)
}

/// Default trigger scheduler capability.
#[must_use]
pub fn default_trigger_scheduler() -> Arc<dyn TriggerScheduler> {
    Arc::new(NoopTriggerScheduler)
}

/// Default execution emitter capability.
#[must_use]
pub fn default_execution_emitter() -> Arc<dyn ExecutionEmitter> {
    Arc::new(NoopExecutionEmitter)
}
