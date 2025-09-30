//! Testing utilities for resource management

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use uuid::Uuid;

#[cfg(feature = "mockall")]
use mockall::{mock, predicate::*};

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    lifecycle::LifecycleState,
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    traits::{HealthCheckable, HealthStatus},
};

/// Test resource manager for unit testing
pub struct TestResourceManager {
    /// Mock resources by type ID
    resources: Arc<Mutex<HashMap<String, Arc<dyn TestableResource>>>>,
    /// Call history for verification
    call_history: Arc<Mutex<Vec<ResourceCall>>>,
}

impl TestResourceManager {
    /// Create a new test resource manager
    pub fn new() -> Self {
        Self {
            resources: Arc::new(Mutex::new(HashMap::new())),
            call_history: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Register a mock resource
    pub fn register_mock<T>(&self, resource_id: String, resource: T)
    where
        T: TestableResource + 'static,
    {
        let mut resources = self.resources.lock().unwrap();
        resources.insert(resource_id, Arc::new(resource));
    }

    /// Get a mock resource
    pub async fn get_mock<T>(&self, resource_id: &str) -> ResourceResult<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        let resources = self.resources.lock().unwrap();
        let resource = resources
            .get(resource_id)
            .ok_or_else(|| ResourceError::unavailable(resource_id, "Mock resource not found", false))?;

        // Record the call
        {
            let mut history = self.call_history.lock().unwrap();
            history.push(ResourceCall::Acquire {
                resource_id: resource_id.to_string(),
                timestamp: chrono::Utc::now(),
            });
        }

        // Simulate resource acquisition
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // In a real implementation, this would be a proper downcast
        // For testing purposes, we'll return a placeholder
        Ok(Arc::new(unsafe { std::mem::zeroed() }))
    }

    /// Get call history for verification
    pub fn call_history(&self) -> Vec<ResourceCall> {
        self.call_history.lock().unwrap().clone()
    }

    /// Clear call history
    pub fn clear_history(&self) {
        self.call_history.lock().unwrap().clear();
    }

    /// Verify that a specific call was made
    pub fn verify_call(&self, expected_call: &ResourceCall) -> bool {
        let history = self.call_history.lock().unwrap();
        history.iter().any(|call| call.matches(expected_call))
    }
}

impl Default for TestResourceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for testable resources
pub trait TestableResource: Send + Sync {
    /// Create a mock instance
    fn create_mock(&self) -> Box<dyn std::any::Any + Send + Sync>;

    /// Get the resource type name
    fn type_name(&self) -> &str;

    /// Simulate a health check
    fn mock_health_check(&self) -> HealthStatus {
        HealthStatus::Healthy
    }
}

/// Record of resource calls for testing verification
#[derive(Debug, Clone)]
pub enum ResourceCall {
    /// Resource acquisition
    Acquire {
        resource_id: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// Resource release
    Release {
        resource_id: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// Health check
    HealthCheck {
        resource_id: String,
        result: HealthStatus,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// Resource creation
    Create {
        resource_id: String,
        config: serde_json::Value,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
}

impl ResourceCall {
    /// Check if this call matches another call (ignoring timestamp)
    pub fn matches(&self, other: &ResourceCall) -> bool {
        match (self, other) {
            (
                ResourceCall::Acquire { resource_id: id1, .. },
                ResourceCall::Acquire { resource_id: id2, .. },
            ) => id1 == id2,
            (
                ResourceCall::Release { resource_id: id1, .. },
                ResourceCall::Release { resource_id: id2, .. },
            ) => id1 == id2,
            (
                ResourceCall::HealthCheck { resource_id: id1, .. },
                ResourceCall::HealthCheck { resource_id: id2, .. },
            ) => id1 == id2,
            (
                ResourceCall::Create { resource_id: id1, .. },
                ResourceCall::Create { resource_id: id2, .. },
            ) => id1 == id2,
            _ => false,
        }
    }
}

/// Mock resource implementation for testing
pub struct MockResource {
    /// Resource metadata
    metadata: ResourceMetadata,
    /// Expected behavior
    behavior: MockBehavior,
}

impl MockResource {
    /// Create a new mock resource
    pub fn new(metadata: ResourceMetadata) -> Self {
        Self {
            metadata,
            behavior: MockBehavior::default(),
        }
    }

    /// Configure the mock behavior
    pub fn with_behavior(mut self, behavior: MockBehavior) -> Self {
        self.behavior = behavior;
        self
    }
}

/// Configuration for mock resource behavior
#[derive(Debug, Clone)]
pub struct MockBehavior {
    /// Whether resource creation should succeed
    pub creation_succeeds: bool,
    /// Health check result to return
    pub health_status: HealthStatus,
    /// Simulated latency for operations
    pub operation_latency: std::time::Duration,
    /// Whether to simulate random failures
    pub random_failures: bool,
    /// Failure probability (0.0 to 1.0)
    pub failure_probability: f64,
}

impl Default for MockBehavior {
    fn default() -> Self {
        Self {
            creation_succeeds: true,
            health_status: HealthStatus::Healthy,
            operation_latency: std::time::Duration::from_millis(10),
            random_failures: false,
            failure_probability: 0.1,
        }
    }
}

/// Mock resource configuration
#[derive(Debug, Clone)]
pub struct MockResourceConfig {
    /// Configuration name
    pub name: String,
    /// Mock data
    pub data: serde_json::Value,
}

impl ResourceConfig for MockResourceConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.name.is_empty() {
            return Err(ResourceError::configuration("Name cannot be empty"));
        }
        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.name.is_empty() {
            self.name = other.name;
        }
        if !other.data.is_null() {
            self.data = other.data;
        }
    }
}

/// Mock resource instance
pub struct MockResourceInstance {
    /// Instance metadata
    instance_id: Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: std::sync::RwLock<LifecycleState>,

    /// Mock behavior
    behavior: MockBehavior,
}

impl MockResourceInstance {
    /// Create a new mock instance
    pub fn new(
        resource_id: ResourceId,
        context: ResourceContext,
        behavior: MockBehavior,
    ) -> Self {
        Self {
            instance_id: Uuid::new_v4(),
            resource_id,
            context,
            created_at: chrono::Utc::now(),
            last_accessed: std::sync::Mutex::new(None),
            state: std::sync::RwLock::new(LifecycleState::Ready),
            behavior,
        }
    }

    /// Simulate some work being done
    pub async fn simulate_work(&self) -> ResourceResult<()> {
        // Simulate operation latency
        tokio::time::sleep(self.behavior.operation_latency).await;

        // Simulate random failures
        if self.behavior.random_failures {
            if rand::random::<f64>() < self.behavior.failure_probability {
                return Err(ResourceError::internal(
                    self.resource_id.unique_key(),
                    "Simulated random failure",
                ));
            }
        }

        self.touch();
        Ok(())
    }
}

impl ResourceInstance for MockResourceInstance {
    fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    fn resource_id(&self) -> &ResourceId {
        &self.resource_id
    }

    fn lifecycle_state(&self) -> LifecycleState {
        *self.state.read().unwrap()
    }

    fn context(&self) -> &ResourceContext {
        &self.context
    }

    fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.created_at
    }

    fn last_accessed_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_accessed.lock().unwrap()
    }

    fn touch(&mut self) {
        *self.last_accessed.lock().unwrap() = Some(chrono::Utc::now());
    }
}

#[async_trait]
impl HealthCheckable for MockResourceInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        // Simulate health check latency
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        Ok(self.behavior.health_status.clone())
    }
}

#[async_trait]
impl Resource for MockResource {
    type Config = MockResourceConfig;
    type Instance = MockResourceInstance;

    fn metadata(&self) -> ResourceMetadata {
        self.metadata.clone()
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        // Validate configuration
        config.validate()?;

        // Check if creation should succeed
        if !self.behavior.creation_succeeds {
            return Err(ResourceError::initialization(
                self.metadata.id.unique_key(),
                "Mock resource creation failed",
            ));
        }

        // Simulate creation latency
        tokio::time::sleep(self.behavior.operation_latency).await;

        Ok(MockResourceInstance::new(
            self.metadata.id.clone(),
            context.clone(),
            self.behavior.clone(),
        ))
    }

    async fn cleanup(&self, _instance: Self::Instance) -> ResourceResult<()> {
        // Simulate cleanup latency
        tokio::time::sleep(self.behavior.operation_latency).await;
        Ok(())
    }

    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        Ok(matches!(
            instance.lifecycle_state(),
            LifecycleState::Ready | LifecycleState::Idle | LifecycleState::InUse
        ))
    }
}

impl TestableResource for MockResource {
    fn create_mock(&self) -> Box<dyn std::any::Any + Send + Sync> {
        Box::new(self.clone())
    }

    fn type_name(&self) -> &str {
        &self.metadata.id.name
    }

    fn mock_health_check(&self) -> HealthStatus {
        self.behavior.health_status.clone()
    }
}

impl Clone for MockResource {
    fn clone(&self) -> Self {
        Self {
            metadata: self.metadata.clone(),
            behavior: self.behavior.clone(),
        }
    }
}

/// Builder for creating test scenarios
pub struct TestScenarioBuilder {
    /// Resources to register
    resources: Vec<(String, Box<dyn TestableResource>)>,
    /// Expected calls
    expected_calls: Vec<ResourceCall>,
}

impl TestScenarioBuilder {
    /// Create a new test scenario builder
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
            expected_calls: Vec::new(),
        }
    }

    /// Add a mock resource to the scenario
    pub fn with_resource<T>(mut self, resource_id: String, resource: T) -> Self
    where
        T: TestableResource + 'static,
    {
        self.resources.push((resource_id, Box::new(resource)));
        self
    }

    /// Add an expected call to the scenario
    pub fn expect_call(mut self, call: ResourceCall) -> Self {
        self.expected_calls.push(call);
        self
    }

    /// Build the test scenario
    pub fn build(self) -> TestScenario {
        let manager = TestResourceManager::new();

        for (resource_id, resource) in self.resources {
            manager.register_mock(resource_id, resource);
        }

        TestScenario {
            manager,
            expected_calls: self.expected_calls,
        }
    }
}

impl Default for TestScenarioBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A complete test scenario with resources and expectations
pub struct TestScenario {
    /// The test resource manager
    pub manager: TestResourceManager,
    /// Expected calls for verification
    expected_calls: Vec<ResourceCall>,
}

impl TestScenario {
    /// Verify that all expected calls were made
    pub fn verify(&self) -> bool {
        self.expected_calls
            .iter()
            .all(|expected| self.manager.verify_call(expected))
    }

    /// Get a summary of verification results
    pub fn verification_summary(&self) -> TestVerificationSummary {
        let total_expected = self.expected_calls.len();
        let verified = self
            .expected_calls
            .iter()
            .filter(|expected| self.manager.verify_call(expected))
            .count();

        TestVerificationSummary {
            total_expected,
            verified,
            success_rate: if total_expected > 0 {
                verified as f64 / total_expected as f64
            } else {
                1.0
            },
            actual_calls: self.manager.call_history(),
        }
    }
}

/// Summary of test verification results
pub struct TestVerificationSummary {
    /// Total number of expected calls
    pub total_expected: usize,
    /// Number of verified calls
    pub verified: usize,
    /// Success rate (0.0 to 1.0)
    pub success_rate: f64,
    /// Actual calls made during the test
    pub actual_calls: Vec<ResourceCall>,
}

impl TestVerificationSummary {
    /// Check if the test passed (all expectations met)
    pub fn passed(&self) -> bool {
        self.success_rate >= 1.0
    }
}

/// Convenience macros for testing
#[macro_export]
macro_rules! expect_resource_call {
    (acquire, $resource_id:expr) => {
        $crate::testing::ResourceCall::Acquire {
            resource_id: $resource_id.to_string(),
            timestamp: chrono::Utc::now(),
        }
    };
    (release, $resource_id:expr) => {
        $crate::testing::ResourceCall::Release {
            resource_id: $resource_id.to_string(),
            timestamp: chrono::Utc::now(),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::scoping::ResourceScope;

    #[tokio::test]
    async fn test_mock_resource() {
        let metadata = ResourceMetadata::new(
            ResourceId::new("test", "1.0"),
            "Test resource".to_string(),
        );

        let mock_resource = MockResource::new(metadata);
        let config = MockResourceConfig {
            name: "test_config".to_string(),
            data: serde_json::json!({"key": "value"}),
        };
        let context = ResourceContext::new(
            "test_workflow".to_string(),
            "Test Workflow".to_string(),
            "test_execution".to_string(),
            "test".to_string(),
        );

        let instance = mock_resource.create(&config, &context).await.unwrap();
        assert_eq!(instance.lifecycle_state(), LifecycleState::Ready);

        let health = instance.health_check().await.unwrap();
        assert_eq!(health, HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn test_resource_manager() {
        let manager = TestResourceManager::new();

        let metadata = ResourceMetadata::new(
            ResourceId::new("test", "1.0"),
            "Test resource".to_string(),
        );
        let mock_resource = MockResource::new(metadata);

        manager.register_mock("test".to_string(), mock_resource);

        // Test would normally get a resource here
        let history = manager.call_history();
        assert!(history.is_empty());
    }

    #[test]
    fn test_scenario_builder() {
        let metadata = ResourceMetadata::new(
            ResourceId::new("test", "1.0"),
            "Test resource".to_string(),
        );
        let mock_resource = MockResource::new(metadata);

        let scenario = TestScenarioBuilder::new()
            .with_resource("test".to_string(), mock_resource)
            .expect_call(ResourceCall::Acquire {
                resource_id: "test".to_string(),
                timestamp: chrono::Utc::now(),
            })
            .build();

        let summary = scenario.verification_summary();
        assert_eq!(summary.total_expected, 1);
    }
}