# Archived From "docs/archive/final.md"

### nebula-testing
**Назначение:** Инструменты для тестирования workflows и actions.

```rust
pub struct WorkflowTestHarness {
    engine: MockEngine,
    resources: MockResourceManager,
}

impl WorkflowTestHarness {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn with_mock_action(mut self, id: &str, handler: impl Fn(serde_json::Value) -> serde_json::Value) -> Self {
        self.engine.register_mock(id, handler);
        self
    }
    
    pub async fn execute(&self, workflow: WorkflowDefinition, input: serde_json::Value) -> TestResult {
        let execution = self.engine.execute(workflow, input).await?;
        
        TestResult {
            output: execution.output,
            events: execution.events,
            metrics: execution.metrics,
            node_outputs: execution.node_outputs,
        }
    }
}

// Тестирование Action
#[tokio::test]
async fn test_email_action() {
    let harness = ActionTestHarness::new()
        .with_credential("smtp", mock_smtp_credential())
        .with_resource::<MockEmailClient>();
    
    let result = harness.execute::<EmailAction>(EmailInput {
        to: "test@example.com",
        subject: "Test",
        body: "Hello",
    }).await;
    
    assert!(result.is_success());
    assert_eq!(result.output.message_id, "mock-123");
}

// Тестирование Workflow
#[tokio::test]
async fn test_registration_workflow() {
    let harness = WorkflowTestHarness::new()
        .with_mock_action("validation.user", |input| {
            json!({ "validated": true, "data": input })
        })
        .with_mock_action("database.insert", |input| {
            json!({ "id": 123, "created": true })
        });
    
    let result = harness.execute(
        registration_workflow(),
        json!({ "email": "user@example.com" })
    ).await;
    
    assert_eq!(result.node_outputs["create_user"]["id"], 123);
}
```

---

## Presentation Layer

