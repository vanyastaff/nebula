# Example: Build a Dashboard Projection

Use scoped/filtered subscriptions to maintain a compact dashboard state in memory.

```rust
use std::collections::HashMap;

use nebula_eventbus::{EventBus, EventFilter, ScopedEvent, SubscriptionScope};

#[derive(Debug, Clone)]
enum ExecutionEvent {
    Started { execution_id: String, workflow_id: String },
    Completed { execution_id: String, workflow_id: String },
}

impl ScopedEvent for ExecutionEvent {
    fn workflow_id(&self) -> Option<&str> {
        match self {
            Self::Started { workflow_id, .. } | Self::Completed { workflow_id, .. } => {
                Some(workflow_id)
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let bus = EventBus::<ExecutionEvent>::new(512);
    let scope = SubscriptionScope::workflow("wf-critical");
    let mut sub = bus.subscribe_scoped(scope);

    // Extra filtering: keep only completion events.
    let mut completion_sub = bus.subscribe_filtered(EventFilter::custom(|event| {
        matches!(event, ExecutionEvent::Completed { .. })
    }));

    let mut projection: HashMap<String, &'static str> = HashMap::new();

    let _ = bus.emit(ExecutionEvent::Started {
        execution_id: "e1".into(),
        workflow_id: "wf-critical".into(),
    });

    if let Some(ExecutionEvent::Started { execution_id, .. }) = sub.recv().await {
        projection.insert(execution_id, "running");
    }

    let _ = bus.emit(ExecutionEvent::Completed {
        execution_id: "e1".into(),
        workflow_id: "wf-critical".into(),
    });

    if let Some(ExecutionEvent::Completed { execution_id, .. }) = completion_sub.recv().await {
        projection.insert(execution_id, "done");
    }
}
```

## Pattern summary

- scoped subscriber reduces fan-out cost for irrelevant workflow data,
- filtered subscriber enables specialized projections,
- dashboard projection remains eventually consistent and cheap.
