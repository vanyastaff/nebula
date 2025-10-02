//! Workflow-specific errors for the Nebula workflow engine
//!
//! This module defines error types specific to workflow execution,
//! node processing, triggers, and workflow orchestration.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// Errors related to workflow definition and execution
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum WorkflowError {
    /// Invalid workflow definition
    #[error("Invalid workflow definition: {reason}")]
    InvalidDefinition { reason: String },

    /// Circular dependency detected in workflow
    #[error("Circular dependency detected: {path}")]
    CircularDependency { path: String },

    /// Missing required workflow parameter
    #[error("Missing required parameter '{parameter}' in workflow '{workflow_id}'")]
    MissingParameter {
        workflow_id: String,
        parameter: String,
    },

    /// Invalid workflow version
    #[error("Invalid workflow version '{version}' for workflow '{workflow_id}'")]
    InvalidVersion {
        workflow_id: String,
        version: String,
    },

    /// Workflow execution limit exceeded
    #[error("Workflow execution limit exceeded: {limit} executions in {period:?}")]
    ExecutionLimitExceeded { limit: u32, period: Duration },

    /// Workflow not found
    #[error("Workflow '{workflow_id}' not found")]
    NotFound { workflow_id: String },

    /// Workflow is disabled
    #[error("Workflow '{workflow_id}' is disabled")]
    Disabled { workflow_id: String },

    /// Invalid workflow state transition
    #[error("Invalid state transition from '{from}' to '{to}' for workflow '{workflow_id}'")]
    InvalidStateTransition {
        workflow_id: String,
        from: String,
        to: String,
    },
}

/// Errors related to individual workflow nodes
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum NodeError {
    /// Node execution failed
    #[error("Node '{node_id}' execution failed: {reason}")]
    ExecutionFailed { node_id: String, reason: String },

    /// Node configuration is invalid
    #[error("Invalid configuration for node '{node_id}': {reason}")]
    InvalidConfiguration { node_id: String, reason: String },

    /// Node type is not supported
    #[error("Unsupported node type '{node_type}' for node '{node_id}'")]
    UnsupportedType { node_id: String, node_type: String },

    /// Node input validation failed
    #[error("Input validation failed for node '{node_id}': {reason}")]
    InputValidationFailed { node_id: String, reason: String },

    /// Node output transformation failed
    #[error("Output transformation failed for node '{node_id}': {reason}")]
    OutputTransformationFailed { node_id: String, reason: String },

    /// Node timeout
    #[error("Node '{node_id}' execution timed out after {timeout:?}")]
    Timeout { node_id: String, timeout: Duration },

    /// Node dependency not satisfied
    #[error("Dependency '{dependency}' not satisfied for node '{node_id}'")]
    DependencyNotSatisfied { node_id: String, dependency: String },

    /// Node resource limit exceeded
    #[error("Resource limit exceeded for node '{node_id}': {resource} limit of {limit}")]
    ResourceLimitExceeded {
        node_id: String,
        resource: String,
        limit: String,
    },
}

/// Errors related to workflow triggers
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum TriggerError {
    /// Webhook trigger configuration invalid
    #[error("Invalid webhook trigger configuration: {reason}")]
    InvalidWebhookConfig { reason: String },

    /// Cron expression is invalid
    #[error("Invalid cron expression '{expression}': {reason}")]
    InvalidCronExpression { expression: String, reason: String },

    /// Trigger registration failed
    #[error("Failed to register trigger '{trigger_id}': {reason}")]
    RegistrationFailed { trigger_id: String, reason: String },

    /// Trigger not found
    #[error("Trigger '{trigger_id}' not found")]
    NotFound { trigger_id: String },

    /// Manual trigger requires authentication
    #[error("Manual trigger requires authentication for workflow '{workflow_id}'")]
    AuthenticationRequired { workflow_id: String },

    /// Event trigger payload invalid
    #[error("Invalid event payload for trigger '{trigger_id}': {reason}")]
    InvalidPayload { trigger_id: String, reason: String },

    /// Trigger rate limit exceeded
    #[error("Rate limit exceeded for trigger '{trigger_id}': {limit} triggers per {period:?}")]
    RateLimitExceeded {
        trigger_id: String,
        limit: u32,
        period: Duration,
    },
}

/// Errors related to external service connections
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum ConnectorError {
    /// Connection to external service failed
    #[error("Failed to connect to '{service}': {reason}")]
    ConnectionFailed { service: String, reason: String },

    /// API call to external service failed
    #[error("API call to '{service}' failed: {endpoint} returned {status}")]
    ApiCallFailed {
        service: String,
        endpoint: String,
        status: u16,
    },

    /// Service configuration invalid
    #[error("Invalid configuration for service '{service}': {reason}")]
    InvalidConfiguration { service: String, reason: String },

    /// Service quota exceeded
    #[error("Quota exceeded for service '{service}': {quota_type}")]
    QuotaExceeded { service: String, quota_type: String },

    /// Service temporarily unavailable
    #[error("Service '{service}' is temporarily unavailable: {reason}")]
    ServiceUnavailable { service: String, reason: String },

    /// Data transformation failed
    #[error("Data transformation failed for service '{service}': {reason}")]
    TransformationFailed { service: String, reason: String },

    /// Unsupported operation
    #[error("Operation '{operation}' not supported by service '{service}'")]
    UnsupportedOperation { service: String, operation: String },
}

/// Errors related to credentials and authentication
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum CredentialError {
    /// Credentials not found
    #[error("Credentials '{credential_id}' not found")]
    NotFound { credential_id: String },

    /// Credentials are invalid or expired
    #[error("Invalid or expired credentials for '{service}': {reason}")]
    InvalidCredentials { service: String, reason: String },

    /// Missing required credential fields
    #[error("Missing required fields in credentials '{credential_id}': {fields:?}")]
    MissingFields {
        credential_id: String,
        fields: Vec<String>,
    },

    /// Credential encryption/decryption failed
    #[error("Failed to decrypt credentials '{credential_id}': {reason}")]
    DecryptionFailed {
        credential_id: String,
        reason: String,
    },

    /// OAuth flow failed
    #[error("OAuth authentication failed for service '{service}': {reason}")]
    OAuthFailed { service: String, reason: String },

    /// API key is invalid
    #[error("Invalid API key for service '{service}'")]
    InvalidApiKey { service: String },

    /// Token refresh failed
    #[error("Failed to refresh token for service '{service}': {reason}")]
    TokenRefreshFailed { service: String, reason: String },
}

/// Errors related to workflow execution runtime
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum ExecutionError {
    /// Execution context is invalid
    #[error("Invalid execution context: {reason}")]
    InvalidContext { reason: String },

    /// Memory limit exceeded during execution
    #[error("Memory limit exceeded: used {used_mb}MB, limit {limit_mb}MB")]
    MemoryLimitExceeded { used_mb: u64, limit_mb: u64 },

    /// CPU time limit exceeded
    #[error("CPU time limit exceeded: used {used_ms}ms, limit {limit_ms}ms")]
    CpuLimitExceeded { used_ms: u64, limit_ms: u64 },

    /// Execution was cancelled
    #[error("Execution cancelled: {reason}")]
    Cancelled { reason: String },

    /// Data size limit exceeded
    #[error("Data size limit exceeded: {size_mb}MB exceeds limit of {limit_mb}MB")]
    DataSizeLimitExceeded { size_mb: u64, limit_mb: u64 },

    /// Concurrent execution limit reached
    #[error("Concurrent execution limit reached: {current} active executions, limit {limit}")]
    ConcurrencyLimitReached { current: u32, limit: u32 },

    /// Execution queue is full
    #[error("Execution queue is full: {queue_size} items, maximum {max_size}")]
    QueueFull { queue_size: u32, max_size: u32 },

    /// Variable resolution failed
    #[error("Failed to resolve variable '{variable}': {reason}")]
    VariableResolutionFailed { variable: String, reason: String },
}

// =============================================================================
// Error Classification and Retry Logic
// =============================================================================

impl WorkflowError {
    /// Check if this workflow error is retryable
    pub fn is_retryable(&self) -> bool {
        match self {
            WorkflowError::ExecutionLimitExceeded { .. } => true,
            WorkflowError::NotFound { .. } => false,
            WorkflowError::Disabled { .. } => false,
            WorkflowError::InvalidDefinition { .. } => false,
            WorkflowError::CircularDependency { .. } => false,
            WorkflowError::MissingParameter { .. } => false,
            WorkflowError::InvalidVersion { .. } => false,
            WorkflowError::InvalidStateTransition { .. } => false,
        }
    }

    /// Get error code for this workflow error
    pub fn error_code(&self) -> &'static str {
        match self {
            WorkflowError::InvalidDefinition { .. } => "WORKFLOW_INVALID_DEFINITION",
            WorkflowError::CircularDependency { .. } => "WORKFLOW_CIRCULAR_DEPENDENCY",
            WorkflowError::MissingParameter { .. } => "WORKFLOW_MISSING_PARAMETER",
            WorkflowError::InvalidVersion { .. } => "WORKFLOW_INVALID_VERSION",
            WorkflowError::ExecutionLimitExceeded { .. } => "WORKFLOW_EXECUTION_LIMIT_EXCEEDED",
            WorkflowError::NotFound { .. } => "WORKFLOW_NOT_FOUND",
            WorkflowError::Disabled { .. } => "WORKFLOW_DISABLED",
            WorkflowError::InvalidStateTransition { .. } => "WORKFLOW_INVALID_STATE_TRANSITION",
        }
    }
}

impl NodeError {
    /// Check if this node error is retryable
    pub fn is_retryable(&self) -> bool {
        match self {
            NodeError::ExecutionFailed { .. } => true,
            NodeError::Timeout { .. } => true,
            NodeError::ResourceLimitExceeded { .. } => true,
            NodeError::InvalidConfiguration { .. } => false,
            NodeError::UnsupportedType { .. } => false,
            NodeError::InputValidationFailed { .. } => false,
            NodeError::OutputTransformationFailed { .. } => false,
            NodeError::DependencyNotSatisfied { .. } => false,
        }
    }

    /// Get error code for this node error
    pub fn error_code(&self) -> &'static str {
        match self {
            NodeError::ExecutionFailed { .. } => "NODE_EXECUTION_FAILED",
            NodeError::InvalidConfiguration { .. } => "NODE_INVALID_CONFIGURATION",
            NodeError::UnsupportedType { .. } => "NODE_UNSUPPORTED_TYPE",
            NodeError::InputValidationFailed { .. } => "NODE_INPUT_VALIDATION_FAILED",
            NodeError::OutputTransformationFailed { .. } => "NODE_OUTPUT_TRANSFORMATION_FAILED",
            NodeError::Timeout { .. } => "NODE_TIMEOUT",
            NodeError::DependencyNotSatisfied { .. } => "NODE_DEPENDENCY_NOT_SATISFIED",
            NodeError::ResourceLimitExceeded { .. } => "NODE_RESOURCE_LIMIT_EXCEEDED",
        }
    }
}

impl TriggerError {
    /// Check if this trigger error is retryable
    pub fn is_retryable(&self) -> bool {
        match self {
            TriggerError::RegistrationFailed { .. } => true,
            TriggerError::RateLimitExceeded { .. } => true,
            TriggerError::InvalidWebhookConfig { .. } => false,
            TriggerError::InvalidCronExpression { .. } => false,
            TriggerError::NotFound { .. } => false,
            TriggerError::AuthenticationRequired { .. } => false,
            TriggerError::InvalidPayload { .. } => false,
        }
    }

    /// Get error code for this trigger error
    pub fn error_code(&self) -> &'static str {
        match self {
            TriggerError::InvalidWebhookConfig { .. } => "TRIGGER_INVALID_WEBHOOK_CONFIG",
            TriggerError::InvalidCronExpression { .. } => "TRIGGER_INVALID_CRON_EXPRESSION",
            TriggerError::RegistrationFailed { .. } => "TRIGGER_REGISTRATION_FAILED",
            TriggerError::NotFound { .. } => "TRIGGER_NOT_FOUND",
            TriggerError::AuthenticationRequired { .. } => "TRIGGER_AUTHENTICATION_REQUIRED",
            TriggerError::InvalidPayload { .. } => "TRIGGER_INVALID_PAYLOAD",
            TriggerError::RateLimitExceeded { .. } => "TRIGGER_RATE_LIMIT_EXCEEDED",
        }
    }
}

impl ConnectorError {
    /// Check if this connector error is retryable
    pub fn is_retryable(&self) -> bool {
        match self {
            ConnectorError::ConnectionFailed { .. } => true,
            ConnectorError::ServiceUnavailable { .. } => true,
            ConnectorError::ApiCallFailed { status, .. } => *status >= 500,
            ConnectorError::QuotaExceeded { .. } => true,
            ConnectorError::InvalidConfiguration { .. } => false,
            ConnectorError::TransformationFailed { .. } => false,
            ConnectorError::UnsupportedOperation { .. } => false,
        }
    }

    /// Get error code for this connector error
    pub fn error_code(&self) -> &'static str {
        match self {
            ConnectorError::ConnectionFailed { .. } => "CONNECTOR_CONNECTION_FAILED",
            ConnectorError::ApiCallFailed { .. } => "CONNECTOR_API_CALL_FAILED",
            ConnectorError::InvalidConfiguration { .. } => "CONNECTOR_INVALID_CONFIGURATION",
            ConnectorError::QuotaExceeded { .. } => "CONNECTOR_QUOTA_EXCEEDED",
            ConnectorError::ServiceUnavailable { .. } => "CONNECTOR_SERVICE_UNAVAILABLE",
            ConnectorError::TransformationFailed { .. } => "CONNECTOR_TRANSFORMATION_FAILED",
            ConnectorError::UnsupportedOperation { .. } => "CONNECTOR_UNSUPPORTED_OPERATION",
        }
    }
}

impl CredentialError {
    /// Check if this credential error is retryable
    pub fn is_retryable(&self) -> bool {
        match self {
            CredentialError::TokenRefreshFailed { .. } => true,
            CredentialError::OAuthFailed { .. } => true,
            CredentialError::NotFound { .. } => false,
            CredentialError::InvalidCredentials { .. } => false,
            CredentialError::MissingFields { .. } => false,
            CredentialError::DecryptionFailed { .. } => false,
            CredentialError::InvalidApiKey { .. } => false,
        }
    }

    /// Get error code for this credential error
    pub fn error_code(&self) -> &'static str {
        match self {
            CredentialError::NotFound { .. } => "CREDENTIAL_NOT_FOUND",
            CredentialError::InvalidCredentials { .. } => "CREDENTIAL_INVALID",
            CredentialError::MissingFields { .. } => "CREDENTIAL_MISSING_FIELDS",
            CredentialError::DecryptionFailed { .. } => "CREDENTIAL_DECRYPTION_FAILED",
            CredentialError::OAuthFailed { .. } => "CREDENTIAL_OAUTH_FAILED",
            CredentialError::InvalidApiKey { .. } => "CREDENTIAL_INVALID_API_KEY",
            CredentialError::TokenRefreshFailed { .. } => "CREDENTIAL_TOKEN_REFRESH_FAILED",
        }
    }
}

impl ExecutionError {
    /// Check if this execution error is retryable
    pub fn is_retryable(&self) -> bool {
        match self {
            ExecutionError::MemoryLimitExceeded { .. } => false,
            ExecutionError::CpuLimitExceeded { .. } => false,
            ExecutionError::DataSizeLimitExceeded { .. } => false,
            ExecutionError::ConcurrencyLimitReached { .. } => true,
            ExecutionError::QueueFull { .. } => true,
            ExecutionError::InvalidContext { .. } => false,
            ExecutionError::Cancelled { .. } => false,
            ExecutionError::VariableResolutionFailed { .. } => false,
        }
    }

    /// Get error code for this execution error
    pub fn error_code(&self) -> &'static str {
        match self {
            ExecutionError::InvalidContext { .. } => "EXECUTION_INVALID_CONTEXT",
            ExecutionError::MemoryLimitExceeded { .. } => "EXECUTION_MEMORY_LIMIT_EXCEEDED",
            ExecutionError::CpuLimitExceeded { .. } => "EXECUTION_CPU_LIMIT_EXCEEDED",
            ExecutionError::Cancelled { .. } => "EXECUTION_CANCELLED",
            ExecutionError::DataSizeLimitExceeded { .. } => "EXECUTION_DATA_SIZE_LIMIT_EXCEEDED",
            ExecutionError::ConcurrencyLimitReached { .. } => "EXECUTION_CONCURRENCY_LIMIT_REACHED",
            ExecutionError::QueueFull { .. } => "EXECUTION_QUEUE_FULL",
            ExecutionError::VariableResolutionFailed { .. } => {
                "EXECUTION_VARIABLE_RESOLUTION_FAILED"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_error_classification() {
        let error = WorkflowError::InvalidDefinition {
            reason: "missing nodes".to_string(),
        };
        assert!(!error.is_retryable());
        assert_eq!(error.error_code(), "WORKFLOW_INVALID_DEFINITION");

        let error = WorkflowError::ExecutionLimitExceeded {
            limit: 100,
            period: Duration::from_secs(3600),
        };
        assert!(error.is_retryable());
        assert_eq!(error.error_code(), "WORKFLOW_EXECUTION_LIMIT_EXCEEDED");
    }

    #[test]
    fn test_node_error_classification() {
        let error = NodeError::ExecutionFailed {
            node_id: "node-1".to_string(),
            reason: "HTTP timeout".to_string(),
        };
        assert!(error.is_retryable());
        assert_eq!(error.error_code(), "NODE_EXECUTION_FAILED");

        let error = NodeError::InvalidConfiguration {
            node_id: "node-1".to_string(),
            reason: "missing API key".to_string(),
        };
        assert!(!error.is_retryable());
        assert_eq!(error.error_code(), "NODE_INVALID_CONFIGURATION");
    }

    #[test]
    fn test_connector_error_retry_logic() {
        let error = ConnectorError::ApiCallFailed {
            service: "slack".to_string(),
            endpoint: "/api/chat.postMessage".to_string(),
            status: 500,
        };
        assert!(error.is_retryable()); // 5xx errors are retryable

        let error = ConnectorError::ApiCallFailed {
            service: "slack".to_string(),
            endpoint: "/api/chat.postMessage".to_string(),
            status: 400,
        };
        assert!(!error.is_retryable()); // 4xx errors are not retryable
    }

    #[test]
    fn test_execution_error_limits() {
        let error = ExecutionError::MemoryLimitExceeded {
            used_mb: 512,
            limit_mb: 256,
        };
        assert!(!error.is_retryable());
        assert_eq!(error.error_code(), "EXECUTION_MEMORY_LIMIT_EXCEEDED");

        let error = ExecutionError::QueueFull {
            queue_size: 1000,
            max_size: 1000,
        };
        assert!(error.is_retryable());
        assert_eq!(error.error_code(), "EXECUTION_QUEUE_FULL");
    }
}
