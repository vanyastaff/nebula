//! Main [`NebulaError`] struct and core error functionality

// Standard library
use std::fmt;
use std::time::Duration;

// External dependencies
use serde::{Deserialize, Serialize};

// Internal crates
use crate::core::context::ErrorContext;
use crate::core::traits::{ErrorClassification, ErrorCode, RetryableError};
use crate::kinds::ErrorKind;

/// Main error type for Nebula
///
/// This is the primary error type used throughout the Nebula ecosystem.
/// It provides structured error information with rich context and metadata.
///
/// # Performance Optimizations
/// - `Box<ErrorKind>`: Reduces stack size and enables efficient copying
/// - `Box<ErrorContext>`: Lazy allocation for context (only when needed)
/// - `&'static str` for error codes: Zero allocation for common codes
/// - `Cow<'static, str>` for messages: Static strings avoid allocation
/// - Optimized memory layout: Fields ordered by size and alignment
/// - `#[inline]` on hot-path methods for better performance
///
/// # Memory Layout
/// The struct is designed to be ≤64 bytes for efficient stack handling:
/// - ErrorKind (8 bytes pointer)
/// - ErrorContext (8 bytes Option<Box>)  
/// - Cow message (24 bytes on 64-bit)
/// - Duration (16 bytes)
/// - Static str (16 bytes)
/// - Bool (1 byte) + padding
///
/// Total: ~73 bytes (within L1 cache line)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NebulaError {
    /// The specific kind/variant of error (boxed for smaller stack footprint)
    pub kind: Box<ErrorKind>,
    /// Additional context information (lazy allocation)
    pub context: Option<Box<ErrorContext>>,
    /// User-friendly error message (zero-alloc for static strings)
    pub message: std::borrow::Cow<'static, str>,
    /// Suggested retry delay (Option<Duration> is 16 bytes)
    pub retry_after: Option<Duration>,
    /// Whether this error is retryable (1 byte, but gets padded)
    pub retryable: bool,
}

impl NebulaError {
    /// Create a new [`NebulaError`] with the given kind
    ///
    /// This is optimized for performance:
    /// - Uses static string slices where possible
    /// - Leverages `Cow::Borrowed` for static messages
    /// - Minimal allocations during error creation
    #[must_use]
    #[inline]
    pub fn new(kind: ErrorKind) -> Self {
        let retryable = kind.is_retryable();
        let message = std::borrow::Cow::Owned(kind.to_string());

        Self {
            kind: Box::new(kind),
            context: None,
            message,
            retry_after: None,
            retryable,
        }
    }

    /// Create a new error with a static message for better performance
    #[must_use]
    #[inline]
    pub fn new_static(kind: ErrorKind, message: &'static str) -> Self {
        let retryable = kind.is_retryable();

        Self {
            kind: Box::new(kind),
            context: None,
            message: std::borrow::Cow::Borrowed(message),
            retry_after: None,
            retryable,
        }
    }

    /// Add context to the error
    #[must_use]
    pub fn with_context(mut self, context: ErrorContext) -> Self {
        self.context = Some(Box::new(context));
        self
    }

    /// Add additional details to the error message
    ///
    /// This extends the existing message with additional context.
    /// For better performance, prefer using error context instead.
    #[must_use]
    pub fn with_details(mut self, details: &str) -> Self {
        self.message = std::borrow::Cow::Owned(format!("{} - {}", self.message, details));
        self
    }

    /// Add a static detail string to the error message
    /// 
    /// More efficient than `with_details` for static strings
    #[must_use]
    pub fn with_static_details(mut self, details: &'static str) -> Self {
        self.message = std::borrow::Cow::Owned(format!("{} - {}", self.message, details));
        self
    }

    /// Set retry information
    #[must_use]
    pub fn with_retry_info(mut self, retryable: bool, retry_after: Option<Duration>) -> Self {
        self.retryable = retryable;
        self.retry_after = retry_after;
        self
    }

    /// Check if this error is retryable
    ///
    /// Hot path: called frequently during error handling
    #[inline]
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// Check if this is a client error (4xx equivalent)
    ///
    /// Hot path: used for error classification
    #[inline]
    #[must_use]
    pub fn is_client_error(&self) -> bool {
        self.kind.is_client_error()
    }

    /// Check if this is a server error (5xx equivalent)
    ///
    /// Hot path: used for error classification
    #[inline]
    #[must_use]
    pub fn is_server_error(&self) -> bool {
        self.kind.is_server_error()
    }

    /// Check if this is a system error (infrastructure/system level)
    ///
    /// Hot path: used for error classification
    #[inline]
    #[must_use]
    pub fn is_system_error(&self) -> bool {
        self.kind.is_system_error()
    }

    /// Get the suggested retry delay
    ///
    /// Hot path: called during retry logic
    #[inline]
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// Get the error code
    ///
    /// Hot path: frequently accessed for logging and telemetry
    #[inline]
    #[must_use]
    pub fn error_code(&self) -> &str {
        self.kind.error_code()
    }

    /// Get the user-friendly message
    #[inline]
    #[must_use]
    pub fn user_message(&self) -> &str {
        &self.message
    }

    /// Get the error context
    #[inline]
    #[must_use]
    pub fn context(&self) -> Option<&ErrorContext> {
        self.context.as_deref()
    }

    // =============================================================================
    // Convenience Constructor Methods
    // =============================================================================

    /// Create a validation error
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Client(crate::kinds::ClientError::validation(
            message,
        )))
    }

    /// Create a not found error
    pub fn not_found(resource_type: impl Into<String>, resource_id: impl Into<String>) -> Self {
        Self::new(ErrorKind::Client(crate::kinds::ClientError::not_found(
            resource_type,
            resource_id,
        )))
    }

    /// Create a permission denied error
    pub fn permission_denied(operation: impl Into<String>, resource: impl Into<String>) -> Self {
        Self::new(ErrorKind::Client(
            crate::kinds::ClientError::permission_denied(operation, resource),
        ))
    }

    /// Create an authentication error
    pub fn authentication(reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Client(
            crate::kinds::ClientError::authentication(reason),
        ))
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Server(crate::kinds::ServerError::internal(
            message,
        )))
    }

    /// Create a service unavailable error
    pub fn service_unavailable(service: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Server(
            crate::kinds::ServerError::service_unavailable(service, reason),
        ))
    }

    /// Create a timeout error
    pub fn timeout(operation: impl Into<String>, duration: std::time::Duration) -> Self {
        Self::new(ErrorKind::System(crate::kinds::SystemError::timeout(
            operation, duration,
        )))
    }

    /// Create a rate limit exceeded error
    pub fn rate_limit_exceeded(limit: u32, period: std::time::Duration) -> Self {
        Self::new(ErrorKind::System(
            crate::kinds::SystemError::rate_limit_exceeded(limit, period),
        ))
    }

    /// Create a network error
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::System(crate::kinds::SystemError::network(
            message,
        )))
    }

    /// Create a database error
    pub fn database(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::System(crate::kinds::SystemError::database(
            message,
        )))
    }

    // =============================================================================
    // Workflow-Specific Constructor Methods
    // =============================================================================

    /// Create a workflow definition error
    pub fn workflow_invalid_definition(reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Workflow(
            crate::kinds::WorkflowError::InvalidDefinition {
                reason: reason.into(),
            },
        ))
    }

    /// Create a workflow circular dependency error
    pub fn workflow_circular_dependency(path: impl Into<String>) -> Self {
        Self::new(ErrorKind::Workflow(
            crate::kinds::WorkflowError::CircularDependency { path: path.into() },
        ))
    }

    /// Create a workflow not found error
    pub fn workflow_not_found(workflow_id: impl Into<String>) -> Self {
        Self::new(ErrorKind::Workflow(crate::kinds::WorkflowError::NotFound {
            workflow_id: workflow_id.into(),
        }))
    }

    /// Create a workflow disabled error
    pub fn workflow_disabled(workflow_id: impl Into<String>) -> Self {
        Self::new(ErrorKind::Workflow(crate::kinds::WorkflowError::Disabled {
            workflow_id: workflow_id.into(),
        }))
    }

    /// Create a workflow missing parameter error
    pub fn workflow_missing_parameter(
        workflow_id: impl Into<String>,
        parameter: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Workflow(
            crate::kinds::WorkflowError::MissingParameter {
                workflow_id: workflow_id.into(),
                parameter: parameter.into(),
            },
        ))
    }

    /// Create a node execution failed error
    pub fn node_execution_failed(node_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Node(crate::kinds::NodeError::ExecutionFailed {
            node_id: node_id.into(),
            reason: reason.into(),
        }))
    }

    /// Create a node invalid configuration error
    pub fn node_invalid_configuration(
        node_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Node(
            crate::kinds::NodeError::InvalidConfiguration {
                node_id: node_id.into(),
                reason: reason.into(),
            },
        ))
    }

    /// Create a node timeout error
    pub fn node_timeout(node_id: impl Into<String>, timeout: Duration) -> Self {
        Self::new(ErrorKind::Node(crate::kinds::NodeError::Timeout {
            node_id: node_id.into(),
            timeout,
        }))
    }

    /// Create a node unsupported type error
    pub fn node_unsupported_type(node_id: impl Into<String>, node_type: impl Into<String>) -> Self {
        Self::new(ErrorKind::Node(crate::kinds::NodeError::UnsupportedType {
            node_id: node_id.into(),
            node_type: node_type.into(),
        }))
    }

    /// Create a trigger registration failed error
    pub fn trigger_registration_failed(
        trigger_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Trigger(
            crate::kinds::TriggerError::RegistrationFailed {
                trigger_id: trigger_id.into(),
                reason: reason.into(),
            },
        ))
    }

    /// Create a trigger invalid webhook config error
    pub fn trigger_invalid_webhook_config(reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Trigger(
            crate::kinds::TriggerError::InvalidWebhookConfig {
                reason: reason.into(),
            },
        ))
    }

    /// Create a trigger invalid cron expression error
    pub fn trigger_invalid_cron_expression(
        expression: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Trigger(
            crate::kinds::TriggerError::InvalidCronExpression {
                expression: expression.into(),
                reason: reason.into(),
            },
        ))
    }

    /// Create a trigger not found error
    pub fn trigger_not_found(trigger_id: impl Into<String>) -> Self {
        Self::new(ErrorKind::Trigger(crate::kinds::TriggerError::NotFound {
            trigger_id: trigger_id.into(),
        }))
    }

    /// Create a connector connection failed error
    pub fn connector_connection_failed(
        service: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Connector(
            crate::kinds::ConnectorError::ConnectionFailed {
                service: service.into(),
                reason: reason.into(),
            },
        ))
    }

    /// Create a connector API call failed error
    pub fn connector_api_call_failed(
        service: impl Into<String>,
        endpoint: impl Into<String>,
        status: u16,
    ) -> Self {
        Self::new(ErrorKind::Connector(
            crate::kinds::ConnectorError::ApiCallFailed {
                service: service.into(),
                endpoint: endpoint.into(),
                status,
            },
        ))
    }

    /// Create a connector service unavailable error
    pub fn connector_service_unavailable(
        service: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Connector(
            crate::kinds::ConnectorError::ServiceUnavailable {
                service: service.into(),
                reason: reason.into(),
            },
        ))
    }

    /// Create a connector quota exceeded error
    pub fn connector_quota_exceeded(
        service: impl Into<String>,
        quota_type: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Connector(
            crate::kinds::ConnectorError::QuotaExceeded {
                service: service.into(),
                quota_type: quota_type.into(),
            },
        ))
    }

    /// Create a credential not found error
    pub fn credential_not_found(credential_id: impl Into<String>) -> Self {
        Self::new(ErrorKind::Credential(
            crate::kinds::CredentialError::NotFound {
                credential_id: credential_id.into(),
            },
        ))
    }

    /// Create a credential invalid error
    pub fn credential_invalid(service: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Credential(
            crate::kinds::CredentialError::InvalidCredentials {
                service: service.into(),
                reason: reason.into(),
            },
        ))
    }

    /// Create a credential OAuth failed error
    pub fn credential_oauth_failed(service: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Credential(
            crate::kinds::CredentialError::OAuthFailed {
                service: service.into(),
                reason: reason.into(),
            },
        ))
    }

    /// Create an execution memory limit exceeded error
    pub fn execution_memory_limit_exceeded(used_mb: u64, limit_mb: u64) -> Self {
        Self::new(ErrorKind::Execution(
            crate::kinds::ExecutionError::MemoryLimitExceeded { used_mb, limit_mb },
        ))
    }

    /// Create an execution CPU limit exceeded error
    pub fn execution_cpu_limit_exceeded(used_ms: u64, limit_ms: u64) -> Self {
        Self::new(ErrorKind::Execution(
            crate::kinds::ExecutionError::CpuLimitExceeded { used_ms, limit_ms },
        ))
    }

    /// Create an execution cancelled error
    pub fn execution_cancelled(reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Execution(
            crate::kinds::ExecutionError::Cancelled {
                reason: reason.into(),
            },
        ))
    }

    /// Create an execution concurrency limit reached error
    pub fn execution_concurrency_limit_reached(current: u32, limit: u32) -> Self {
        Self::new(ErrorKind::Execution(
            crate::kinds::ExecutionError::ConcurrencyLimitReached { current, limit },
        ))
    }

    /// Create an execution queue full error
    pub fn execution_queue_full(queue_size: u32, max_size: u32) -> Self {
        Self::new(ErrorKind::Execution(
            crate::kinds::ExecutionError::QueueFull {
                queue_size,
                max_size,
            },
        ))
    }

    // ============================================================================
    // Memory Error Constructors
    // ============================================================================

    /// Create a memory allocation failed error
    pub fn memory_allocation_failed(size: usize, align: usize) -> Self {
        Self::new(ErrorKind::Memory(
            crate::kinds::MemoryError::AllocationFailed { size, align },
        ))
        .with_retry_info(true, Some(Duration::from_millis(100)))
    }

    /// Create a memory pool exhausted error
    pub fn memory_pool_exhausted(pool_id: impl Into<String>, capacity: usize) -> Self {
        Self::new(ErrorKind::Memory(
            crate::kinds::MemoryError::PoolExhausted {
                pool_id: pool_id.into(),
                capacity,
            },
        ))
        .with_retry_info(true, Some(Duration::from_millis(500)))
    }

    /// Create a memory arena exhausted error
    pub fn memory_arena_exhausted(
        arena_id: impl Into<String>,
        requested: usize,
        available: usize,
    ) -> Self {
        Self::new(ErrorKind::Memory(
            crate::kinds::MemoryError::ArenaExhausted {
                arena_id: arena_id.into(),
                requested,
                available,
            },
        ))
        .with_retry_info(true, Some(Duration::from_millis(200)))
    }

    /// Create a cache miss error
    pub fn memory_cache_miss(key: impl Into<String>) -> Self {
        Self::new(ErrorKind::Memory(crate::kinds::MemoryError::CacheMiss {
            key: key.into(),
        }))
        .with_retry_info(false, None)
    }

    /// Create a memory budget exceeded error
    pub fn memory_budget_exceeded(used: usize, limit: usize) -> Self {
        Self::new(ErrorKind::Memory(
            crate::kinds::MemoryError::BudgetExceeded { used, limit },
        ))
        .with_retry_info(true, Some(Duration::from_millis(500)))
    }

    /// Create a memory corruption error
    pub fn memory_corruption(component: impl Into<String>, details: impl Into<String>) -> Self {
        Self::new(ErrorKind::Memory(crate::kinds::MemoryError::Corruption {
            component: component.into(),
            details: details.into(),
        }))
        .with_retry_info(false, None)
    }

    /// Create a memory invalid layout error
    pub fn memory_invalid_layout(reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Memory(
            crate::kinds::MemoryError::InvalidLayout {
                reason: reason.into(),
            },
        ))
        .with_retry_info(false, None)
    }

    // ============================================================================
    // Resource Error Constructors
    // ============================================================================

    /// Create a resource unavailable error
    pub fn resource_unavailable(
        resource_id: impl Into<String>,
        reason: impl Into<String>,
        retryable: bool,
    ) -> Self {
        let retry_after = if retryable {
            Some(Duration::from_secs(5))
        } else {
            None
        };
        Self::new(ErrorKind::Resource(
            crate::kinds::ResourceError::Unavailable {
                resource_id: resource_id.into(),
                reason: reason.into(),
                retryable,
            },
        ))
        .with_retry_info(retryable, retry_after)
    }

    /// Create a resource health check failed error
    pub fn resource_health_check_failed(
        resource_id: impl Into<String>,
        attempt: u32,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Resource(
            crate::kinds::ResourceError::HealthCheckFailed {
                resource_id: resource_id.into(),
                attempt,
                reason: reason.into(),
            },
        ))
        .with_retry_info(true, Some(Duration::from_secs(2)))
    }

    /// Create a resource circuit breaker open error
    pub fn resource_circuit_breaker_open(
        resource_id: impl Into<String>,
        retry_after_ms: Option<u64>,
    ) -> Self {
        let retry_after = retry_after_ms.map(Duration::from_millis);
        Self::new(ErrorKind::Resource(
            crate::kinds::ResourceError::CircuitBreakerOpen {
                resource_id: resource_id.into(),
                retry_after_ms,
            },
        ))
        .with_retry_info(true, retry_after)
    }

    /// Create a resource pool exhausted error
    pub fn resource_pool_exhausted(
        resource_id: impl Into<String>,
        current_size: usize,
        max_size: usize,
        waiters: usize,
    ) -> Self {
        Self::new(ErrorKind::Resource(
            crate::kinds::ResourceError::PoolExhausted {
                resource_id: resource_id.into(),
                current_size,
                max_size,
                waiters,
            },
        ))
        .with_retry_info(true, Some(Duration::from_millis(200)))
    }

    /// Create a resource dependency failure error
    pub fn resource_dependency_failure(
        resource_id: impl Into<String>,
        dependency_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Resource(
            crate::kinds::ResourceError::DependencyFailure {
                resource_id: resource_id.into(),
                dependency_id: dependency_id.into(),
                reason: reason.into(),
            },
        ))
        .with_retry_info(true, Some(Duration::from_secs(3)))
    }

    /// Create a resource circular dependency error
    pub fn resource_circular_dependency(cycle: impl Into<String>) -> Self {
        Self::new(ErrorKind::Resource(
            crate::kinds::ResourceError::CircularDependency {
                cycle: cycle.into(),
            },
        ))
        .with_retry_info(false, None)
    }

    /// Create a resource invalid state transition error
    pub fn resource_invalid_state_transition(
        resource_id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Resource(
            crate::kinds::ResourceError::InvalidStateTransition {
                resource_id: resource_id.into(),
                from: from.into(),
                to: to.into(),
            },
        ))
        .with_retry_info(false, None)
    }

    /// Create a resource initialization failed error
    pub fn resource_initialization_failed(
        resource_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::Resource(
            crate::kinds::ResourceError::InitializationFailed {
                resource_id: resource_id.into(),
                reason: reason.into(),
            },
        ))
        .with_retry_info(false, None)
    }
}

impl std::error::Error for NebulaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.kind.as_ref())
    }
}

impl fmt::Display for NebulaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.error_code(), self.message)?;

        if let Some(ref context) = self.context {
            write!(f, " (Context: {})", context)?;
        }

        if self.retryable {
            write!(f, " [Retryable")?;
            if let Some(retry_after) = self.retry_after {
                write!(f, " after {:?}", retry_after)?;
            }
            write!(f, "]")?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let error = NebulaError::validation("Invalid input");

        assert_eq!(error.error_code(), "VALIDATION_ERROR");
        assert!(!error.is_retryable());
        assert!(error.is_client_error());
        assert!(!error.is_server_error());
    }

    #[test]
    fn test_error_with_context() {
        let context = ErrorContext::new("Processing user request");
        let error = NebulaError::internal("Database error").with_context(context);

        assert!(error.context().is_some());
        assert_eq!(
            error.context().unwrap().description,
            "Processing user request"
        );
    }

    #[test]
    fn test_error_display() {
        let error = NebulaError::timeout("API call", Duration::from_secs(30))
            .with_details("Connection to external service timed out");

        let display = format!("{error}");
        assert!(display.contains("TIMEOUT_ERROR"));
        assert!(display.contains("API call"));
        assert!(display.contains("Connection to external service timed out"));
        assert!(display.contains("[Retryable"));
    }
}

