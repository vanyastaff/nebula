//! Testing utilities for resource management

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::context::ResourceContext;
use crate::error::{ResourceError, ResourceResult};
use crate::health::HealthStatus;
use crate::resource::{Resource, ResourceConfig};

/// Test resource manager for unit testing
pub struct TestResourceManager {
    resources: Arc<Mutex<HashMap<String, Arc<dyn TestableResource>>>>,
    call_history: Arc<Mutex<Vec<ResourceCall>>>,
}

impl TestResourceManager {
    pub fn new() -> Self {
        Self {
            resources: Arc::new(Mutex::new(HashMap::new())),
            call_history: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn register_mock<T>(&self, resource_id: String, resource: T)
    where
        T: TestableResource + 'static,
    {
        self.resources.lock().insert(resource_id, Arc::new(resource));
    }

    pub async fn get_mock<T>(&self, resource_id: &str) -> ResourceResult<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        let resources = self.resources.lock();
        let testable = resources.get(resource_id).ok_or_else(|| {
            ResourceError::unavailable(resource_id, "Mock resource not found", false)
        })?;

        self.call_history.lock().push(ResourceCall::Acquire {
            resource_id: resource_id.to_string(),
            timestamp: chrono::Utc::now(),
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        testable
            .create_mock()
            .downcast::<T>()
            .map(|boxed| Arc::new(*boxed))
            .map_err(|_| {
                ResourceError::internal(
                    resource_id,
                    format!(
                        "Mock resource type mismatch: expected {}",
                        std::any::type_name::<T>(),
                    ),
                )
            })
    }

    pub fn call_history(&self) -> Vec<ResourceCall> {
        self.call_history.lock().clone()
    }

    pub fn clear_history(&self) {
        self.call_history.lock().clear();
    }

    pub fn verify_call(&self, expected_call: &ResourceCall) -> bool {
        self.call_history
            .lock()
            .iter()
            .any(|call| call.matches(expected_call))
    }
}

impl Default for TestResourceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for testable resources
pub trait TestableResource: Send + Sync {
    fn create_mock(&self) -> Box<dyn std::any::Any + Send + Sync>;
    fn type_name(&self) -> &str;

    fn mock_health_check(&self) -> HealthStatus {
        HealthStatus::healthy()
    }
}

/// Record of resource calls for testing verification
#[derive(Debug, Clone)]
pub enum ResourceCall {
    Acquire {
        resource_id: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    Release {
        resource_id: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    HealthCheck {
        resource_id: String,
        result: HealthStatus,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    Create {
        resource_id: String,
        config: serde_json::Value,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
}

impl ResourceCall {
    pub fn matches(&self, other: &ResourceCall) -> bool {
        match (self, other) {
            (Self::Acquire { resource_id: a, .. }, Self::Acquire { resource_id: b, .. }) => a == b,
            (Self::Release { resource_id: a, .. }, Self::Release { resource_id: b, .. }) => a == b,
            (
                Self::HealthCheck { resource_id: a, .. },
                Self::HealthCheck { resource_id: b, .. },
            ) => a == b,
            (Self::Create { resource_id: a, .. }, Self::Create { resource_id: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// Mock resource implementation for testing
pub struct MockResource {
    id: String,
    behavior: MockBehavior,
}

impl MockResource {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            behavior: MockBehavior::default(),
        }
    }

    pub fn with_behavior(mut self, behavior: MockBehavior) -> Self {
        self.behavior = behavior;
        self
    }
}

#[derive(Debug, Clone)]
pub struct MockBehavior {
    pub creation_succeeds: bool,
    pub health_status: HealthStatus,
    pub operation_latency: std::time::Duration,
}

impl Default for MockBehavior {
    fn default() -> Self {
        Self {
            creation_succeeds: true,
            health_status: HealthStatus::healthy(),
            operation_latency: std::time::Duration::from_millis(10),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct MockResourceConfig {
    pub name: String,
}

impl ResourceConfig for MockResourceConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.name.is_empty() {
            return Err(ResourceError::configuration("Name cannot be empty"));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct MockResourceInstance {
    pub value: String,
}

#[async_trait]
impl Resource for MockResource {
    type Config = MockResourceConfig;
    type Instance = MockResourceInstance;

    fn id(&self) -> &str {
        &self.id
    }

    async fn create(
        &self,
        config: &Self::Config,
        _ctx: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        if !self.behavior.creation_succeeds {
            return Err(ResourceError::initialization(
                &self.id,
                "Mock resource creation failed",
            ));
        }
        tokio::time::sleep(self.behavior.operation_latency).await;
        Ok(MockResourceInstance {
            value: format!("{}-instance", config.name),
        })
    }
}

impl Clone for MockResource {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            behavior: self.behavior.clone(),
        }
    }
}

impl TestableResource for MockResource {
    fn create_mock(&self) -> Box<dyn std::any::Any + Send + Sync> {
        Box::new(self.clone())
    }

    fn type_name(&self) -> &str {
        &self.id
    }

    fn mock_health_check(&self) -> HealthStatus {
        self.behavior.health_status.clone()
    }
}

/// Builder for creating test scenarios
pub struct TestScenarioBuilder {
    resources: Vec<(String, Box<dyn TestableResource>)>,
    expected_calls: Vec<ResourceCall>,
}

impl TestScenarioBuilder {
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
            expected_calls: Vec::new(),
        }
    }

    pub fn with_resource<T>(mut self, resource_id: String, resource: T) -> Self
    where
        T: TestableResource + 'static,
    {
        self.resources.push((resource_id, Box::new(resource)));
        self
    }

    pub fn expect_call(mut self, call: ResourceCall) -> Self {
        self.expected_calls.push(call);
        self
    }

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

pub struct TestScenario {
    pub manager: TestResourceManager,
    expected_calls: Vec<ResourceCall>,
}

impl TestScenario {
    pub fn verify(&self) -> bool {
        self.expected_calls
            .iter()
            .all(|expected| self.manager.verify_call(expected))
    }

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

pub struct TestVerificationSummary {
    pub total_expected: usize,
    pub verified: usize,
    pub success_rate: f64,
    pub actual_calls: Vec<ResourceCall>,
}

impl TestVerificationSummary {
    pub fn passed(&self) -> bool {
        self.success_rate >= 1.0
    }
}

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
    use crate::scope::ResourceScope;

    #[tokio::test]
    async fn test_mock_resource() {
        let mock = MockResource::new("test-resource");
        let config = MockResourceConfig {
            name: "test_config".to_string(),
        };
        let ctx = ResourceContext::new(ResourceScope::Global, "wf", "ex");

        let instance = mock.create(&config, &ctx).await.unwrap();
        assert_eq!(instance.value, "test_config-instance");
    }

    #[tokio::test]
    async fn test_resource_manager() {
        let manager = TestResourceManager::new();
        let mock = MockResource::new("test");
        manager.register_mock("test".to_string(), mock);

        let history = manager.call_history();
        assert!(history.is_empty());
    }

    #[test]
    fn test_scenario_builder() {
        let mock = MockResource::new("test");

        let scenario = TestScenarioBuilder::new()
            .with_resource("test".to_string(), mock)
            .expect_call(ResourceCall::Acquire {
                resource_id: "test".to_string(),
                timestamp: chrono::Utc::now(),
            })
            .build();

        let summary = scenario.verification_summary();
        assert_eq!(summary.total_expected, 1);
    }
}
