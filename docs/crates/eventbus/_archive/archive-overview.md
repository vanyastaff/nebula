# Archived From "docs/archive/overview.md"

### nebula-eventbus
**Назначение:** Pub/sub система для асинхронной коммуникации между компонентами.

**Ключевые компоненты:**
- Scoped subscriptions
- Event filtering
- Distributed events
- Automatic cleanup

```rust
// События workflow lifecycle
pub enum WorkflowEvent {
    WorkflowDeployed { workflow_id, version, deployed_by },
    WorkflowUpdated { workflow_id, old_version, new_version },
}

pub enum ExecutionEvent {
    ExecutionStarted { execution_id, workflow_id, input_data },
    ExecutionCompleted { execution_id, result, duration },
    ExecutionFailed { execution_id, error, retry_count },
}

// Подписка с scope
let subscription = event_bus.subscribe_scoped(
    |event: &ExecutionEvent| async move {
        println!("Execution event: {:?}", event);
    },
    SubscriptionScope::Workflow(workflow_id),
    Some(EventFilter::EventType("execution")),
);

// Публикация из контекста
context.emit_event(NodeEvent::NodeStarted {
    execution_id: context.execution_id.clone(),
    node_id: current_node,
    start_time: SystemTime::now(),
}).await?;
```

---
