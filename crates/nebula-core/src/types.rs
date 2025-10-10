//! Common types and utilities for Nebula
//!
//! This module provides shared types, constants, and utility functions
//! that are used across different parts of the system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::id::{ExecutionId, NodeId, TenantId, UserId, WorkflowId};

/// Version information for Nebula components
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Major version number
    pub major: u32,

    /// Minor version number
    pub minor: u32,

    /// Patch version number
    pub patch: u32,

    /// Pre-release identifier (e.g., "alpha", "beta", "rc.1")
    pub pre: Option<String>,

    /// Build metadata
    pub build: Option<String>,
}

impl Version {
    /// Create a new version
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
            pre: None,
            build: None,
        }
    }

    /// Create a version with pre-release identifier
    pub fn with_pre(mut self, pre: impl Into<String>) -> Self {
        self.pre = Some(pre.into());
        self
    }

    /// Create a version with build metadata
    pub fn with_build(mut self, build: impl Into<String>) -> Self {
        self.build = Some(build.into());
        self
    }

    /// Check if this version is stable (no pre-release)
    pub fn is_stable(&self) -> bool {
        self.pre.is_none()
    }

    /// Check if this version is compatible with another version
    pub fn is_compatible_with(&self, other: &Version) -> bool {
        self.major == other.major && self.minor == other.minor
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;

        if let Some(pre) = &self.pre {
            write!(f, "-{}", pre)?;
        }

        if let Some(build) = &self.build {
            write!(f, "+{}", build)?;
        }

        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then(self.pre.cmp(&other.pre))
    }
}

/// Status of an entity or operation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// Entity is active and ready
    Active,

    /// Entity is inactive or disabled
    Inactive,

    /// Entity is in progress
    InProgress,

    /// Entity has completed successfully
    Completed,

    /// Entity has failed
    Failed,

    /// Entity is pending
    Pending,

    /// Entity is cancelled
    Cancelled,

    /// Entity is suspended
    Suspended,

    /// Entity is in error state
    Error,
}

impl Status {
    /// Check if the status indicates success
    pub fn is_success(&self) -> bool {
        matches!(self, Status::Completed)
    }

    /// Check if the status indicates failure
    pub fn is_failure(&self) -> bool {
        matches!(self, Status::Failed | Status::Error)
    }

    /// Check if the status indicates completion
    pub fn is_completed(&self) -> bool {
        matches!(self, Status::Completed | Status::Failed | Status::Cancelled)
    }

    /// Check if the status indicates active state
    pub fn is_active(&self) -> bool {
        matches!(self, Status::Active | Status::InProgress | Status::Pending)
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Active => write!(f, "active"),
            Status::Inactive => write!(f, "inactive"),
            Status::InProgress => write!(f, "in_progress"),
            Status::Completed => write!(f, "completed"),
            Status::Failed => write!(f, "failed"),
            Status::Pending => write!(f, "pending"),
            Status::Cancelled => write!(f, "cancelled"),
            Status::Suspended => write!(f, "suspended"),
            Status::Error => write!(f, "error"),
        }
    }
}

/// Priority level for operations
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum Priority {
    /// Lowest priority
    Low = 1,

    /// Normal priority
    #[default]
    Normal = 2,

    /// High priority
    High = 3,

    /// Critical priority
    Critical = 4,

    /// Emergency priority
    Emergency = 5,
}

impl Priority {
    /// Get the numeric value of the priority
    pub fn value(&self) -> u8 {
        match self {
            Priority::Low => 1,
            Priority::Normal => 2,
            Priority::High => 3,
            Priority::Critical => 4,
            Priority::Emergency => 5,
        }
    }

    /// Check if this priority is urgent
    pub fn is_urgent(&self) -> bool {
        matches!(
            self,
            Priority::High | Priority::Critical | Priority::Emergency
        )
    }

    /// Check if this priority is critical
    pub fn is_critical(&self) -> bool {
        matches!(self, Priority::Critical | Priority::Emergency)
    }
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::Low => write!(f, "low"),
            Priority::Normal => write!(f, "normal"),
            Priority::High => write!(f, "high"),
            Priority::Critical => write!(f, "critical"),
            Priority::Emergency => write!(f, "emergency"),
        }
    }
}

/// Result of an operation with status and optional data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult<T> {
    /// Status of the operation
    pub status: Status,

    /// Optional data returned by the operation
    pub data: Option<T>,

    /// Optional error message
    pub error: Option<String>,

    /// Timestamp when the operation completed
    pub completed_at: chrono::DateTime<chrono::Utc>,

    /// Duration of the operation
    pub duration: std::time::Duration,
}

impl<T> OperationResult<T> {
    /// Create a successful result
    pub fn success(data: T, duration: std::time::Duration) -> Self {
        Self {
            status: Status::Completed,
            data: Some(data),
            error: None,
            completed_at: chrono::Utc::now(),
            duration,
        }
    }

    /// Create a failed result
    pub fn failure(error: impl Into<String>, duration: std::time::Duration) -> Self {
        Self {
            status: Status::Failed,
            data: None,
            error: Some(error.into()),
            completed_at: chrono::Utc::now(),
            duration,
        }
    }

    /// Check if the operation was successful
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Check if the operation failed
    pub fn is_failure(&self) -> bool {
        self.status.is_failure()
    }

    /// Get the data if successful
    pub fn data(&self) -> Option<&T> {
        self.data.as_ref()
    }

    /// Get the error message if failed
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// Context for operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationContext {
    /// Unique identifier for the operation
    pub operation_id: String,

    /// Execution ID if applicable
    pub execution_id: Option<ExecutionId>,

    /// Workflow ID if applicable
    pub workflow_id: Option<WorkflowId>,

    /// Node ID if applicable
    pub node_id: Option<NodeId>,

    /// User ID if applicable
    pub user_id: Option<UserId>,

    /// Tenant ID if applicable
    pub tenant_id: Option<TenantId>,

    /// Priority of the operation
    pub priority: Priority,

    /// Additional metadata
    pub metadata: HashMap<String, String>,

    /// Timestamp when the context was created
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl OperationContext {
    /// Create a new operation context
    pub fn new(operation_id: impl Into<String>) -> Self {
        Self {
            operation_id: operation_id.into(),
            execution_id: None,
            workflow_id: None,
            node_id: None,
            user_id: None,
            tenant_id: None,
            priority: Priority::default(),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Set the execution ID
    pub fn with_execution_id(mut self, execution_id: ExecutionId) -> Self {
        self.execution_id = Some(execution_id);
        self
    }

    /// Set the workflow ID
    pub fn with_workflow_id(mut self, workflow_id: WorkflowId) -> Self {
        self.workflow_id = Some(workflow_id);
        self
    }

    /// Set the node ID
    pub fn with_node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = Some(node_id);
        self
    }

    /// Set the user ID
    pub fn with_user_id(mut self, user_id: UserId) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Set the tenant ID
    pub fn with_tenant_id(mut self, tenant_id: TenantId) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    /// Set the priority
    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Check if this context has execution information
    pub fn has_execution_context(&self) -> bool {
        self.execution_id.is_some()
    }

    /// Check if this context has workflow information
    pub fn has_workflow_context(&self) -> bool {
        self.workflow_id.is_some()
    }

    /// Check if this context has user information
    pub fn has_user_context(&self) -> bool {
        self.user_id.is_some()
    }

    /// Check if this context has tenant information
    pub fn has_tenant_context(&self) -> bool {
        self.tenant_id.is_some()
    }
}

/// Utility functions for common operations
pub mod utils {
    use super::*;

    /// Generate a unique operation ID
    pub fn generate_operation_id() -> String {
        use uuid::Uuid;
        format!("op_{}", Uuid::new_v4().simple())
    }

    /// Format duration in human-readable format
    pub fn format_duration(duration: std::time::Duration) -> String {
        let secs = duration.as_secs();
        let millis = duration.subsec_millis();

        if secs > 0 {
            format!("{}.{:03}s", secs, millis)
        } else {
            format!("{}ms", millis)
        }
    }

    /// Parse a version string
    pub fn parse_version(version_str: &str) -> Result<Version, String> {
        // Handle pre-release and build metadata
        let (version_part, pre_build) = if let Some(idx) = version_str.find('-') {
            let (v, rest) = version_str.split_at(idx);
            (v, Some(rest))
        } else if let Some(idx) = version_str.find('+') {
            let (v, rest) = version_str.split_at(idx);
            (v, Some(rest))
        } else {
            (version_str, None)
        };

        // Parse major.minor.patch
        let parts: Vec<&str> = version_part.split('.').collect();

        if parts.len() < 3 {
            return Err("Version must have at least major.minor.patch".to_string());
        }

        let major = parts[0]
            .parse::<u32>()
            .map_err(|_| "Invalid major version")?;
        let minor = parts[1]
            .parse::<u32>()
            .map_err(|_| "Invalid minor version")?;
        let patch = parts[2]
            .parse::<u32>()
            .map_err(|_| "Invalid patch version")?;

        let mut version = Version::new(major, minor, patch);

        // Handle pre-release and build metadata
        if let Some(pre_build_str) = pre_build {
            if let Some(pre) = pre_build_str.strip_prefix('-') {
                if let Some(build_idx) = pre.find('+') {
                    let (pre_part, build_part) = pre.split_at(build_idx);
                    version = version.with_pre(pre_part);
                    version = version.with_build(&build_part[1..]);
                } else {
                    version = version.with_pre(pre);
                }
            } else if pre_build_str.starts_with('+') {
                version = version.with_build(&pre_build_str[1..]);
            }
        }

        Ok(version)
    }

    /// Check if a string is a valid identifier
    pub fn is_valid_identifier(s: &str) -> bool {
        if s.is_empty() {
            return false;
        }

        let mut chars = s.chars();
        let first = chars
            .next()
            .expect("s is non-empty, checked above");

        // First character must be alphabetic or underscore
        if !first.is_alphabetic() && first != '_' {
            return false;
        }

        // Remaining characters must be alphanumeric, underscore, or hyphen
        chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_creation() {
        let version = Version::new(1, 2, 3);
        assert_eq!(version.to_string(), "1.2.3");
        assert!(version.is_stable());
    }

    #[test]
    fn test_version_with_pre() {
        let version = Version::new(1, 0, 0).with_pre("alpha");
        assert_eq!(version.to_string(), "1.0.0-alpha");
        assert!(!version.is_stable());
    }

    #[test]
    fn test_version_compatibility() {
        let v1 = Version::new(1, 2, 0);
        let v2 = Version::new(1, 2, 1);
        assert!(v1.is_compatible_with(&v2));

        let v3 = Version::new(1, 3, 0);
        assert!(!v1.is_compatible_with(&v3));
    }

    #[test]
    fn test_status_checks() {
        assert!(Status::Completed.is_success());
        assert!(Status::Failed.is_failure());
        assert!(Status::Completed.is_completed());
        assert!(Status::Active.is_active());
    }

    #[test]
    fn test_priority_checks() {
        assert!(Priority::High.is_urgent());
        assert!(Priority::Critical.is_critical());
        assert_eq!(Priority::Normal.value(), 2);
    }

    #[test]
    fn test_operation_result() {
        let data = "test data";
        let duration = std::time::Duration::from_millis(100);

        let success = OperationResult::success(data, duration);
        assert!(success.is_success());
        assert_eq!(success.data(), Some(&data));

        let failure: OperationResult<&str> = OperationResult::failure("test error", duration);
        assert!(failure.is_failure());
        assert_eq!(failure.error(), Some("test error"));
    }

    #[test]
    fn test_operation_context() {
        let context = OperationContext::new("test-op")
            .with_execution_id(ExecutionId::new())
            .with_priority(Priority::High)
            .with_metadata("key", "value");

        assert!(context.has_execution_context());
        assert_eq!(context.priority, Priority::High);
        assert_eq!(context.metadata.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn test_utils() {
        let operation_id = utils::generate_operation_id();
        assert!(operation_id.starts_with("op_"));

        let duration = std::time::Duration::from_millis(1500);
        assert_eq!(utils::format_duration(duration), "1.500s");

        assert!(utils::is_valid_identifier("test_identifier"));
        assert!(utils::is_valid_identifier("test-identifier"));
        assert!(!utils::is_valid_identifier("123identifier"));
        assert!(!utils::is_valid_identifier(""));
    }

    #[test]
    fn test_version_parsing() {
        let version = utils::parse_version("1.2.3").unwrap();
        assert_eq!(version.major, 1);
        assert_eq!(version.minor, 2);
        assert_eq!(version.patch, 3);

        let version_with_pre = utils::parse_version("1.0.0-alpha").unwrap();
        assert_eq!(version_with_pre.pre, Some("alpha".to_string()));

        let version_with_build = utils::parse_version("1.0.0+build.123").unwrap();
        assert_eq!(version_with_build.build, Some("build.123".to_string()));
    }
}
