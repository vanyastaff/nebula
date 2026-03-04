# Examples

## Stateless Action + Runtime (ACT-T024)

End-to-end minimal flow:

1. Define a typed `StatelessAction`.
2. Register it via `ActionRegistry::register_stateless`.
3. Execute it through `ActionRuntime`.

```rust,ignore
use std::sync::Arc;

use nebula_action::{
    Action, ActionComponents, ActionContext, ActionError, ActionMetadata, ActionResult,
    StatelessAction,
};
use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use nebula_runtime::{ActionRegistry, ActionRuntime, DataPassingPolicy};
use nebula_sandbox_inprocess::{ActionExecutor, InProcessSandbox};
use nebula_telemetry::event::EventBus;
use nebula_telemetry::metrics::MetricsRegistry;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct EchoAction {
    meta: ActionMetadata,
}

impl EchoAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new("example.echo", "Echo", "Echo input payload"),
        }
    }
}

impl Action for EchoAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
    fn components(&self) -> ActionComponents {
        ActionComponents::new()
    }
}

impl StatelessAction for EchoAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl nebula_action::Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[tokio::main]
async fn main() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless(EchoAction::new());

    let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
        Box::pin(async move { Ok(ActionResult::success(input)) })
    });
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let event_bus = Arc::new(EventBus::new(64));
    let metrics = Arc::new(MetricsRegistry::new());

    let runtime = ActionRuntime::new(
        registry,
        sandbox,
        DataPassingPolicy::default(),
        event_bus,
        metrics,
    );

    let ctx = ActionContext::new(
        ExecutionId::new(),
        NodeId::new(),
        WorkflowId::new(),
        CancellationToken::new(),
    );

    let out = runtime
        .execute_action("example.echo", serde_json::json!({"hello":"nebula"}), ctx)
        .await
        .expect("runtime execution");

    println!("{out:?}");
}
```

## Stateful Action + State Loop (ACT-T025)

```rust,ignore
use nebula_action::{
    Action, ActionComponents, ActionError, ActionMetadata, ActionOutput, ActionResult,
    BreakReason, StatefulAction,
};

struct CounterAction {
    meta: ActionMetadata,
}

impl Action for CounterAction {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
    fn components(&self) -> ActionComponents { ActionComponents::new() }
}

impl StatefulAction for CounterAction {
    type Input = ();
    type Output = u32;
    type State = u32;

    async fn execute(
        &self,
        _input: Self::Input,
        state: &mut Self::State,
        _ctx: &impl nebula_action::Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let current = *state;
        *state += 1;

        if current < 3 {
            Ok(ActionResult::Continue {
                output: ActionOutput::Value(current),
                progress: Some((current + 1) as f64 / 4.0),
                delay: None,
            })
        } else {
            Ok(ActionResult::Break {
                output: ActionOutput::Value(current),
                reason: BreakReason::Completed,
            })
        }
    }
}
```

## Trigger Action (Webhook/Poll Starter) (ACT-T026)

```rust,ignore
use nebula_action::{Action, ActionComponents, ActionError, ActionMetadata, TriggerAction, TriggerContext};

struct CronLikeTrigger {
    meta: ActionMetadata,
}

impl Action for CronLikeTrigger {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
    fn components(&self) -> ActionComponents { ActionComponents::new() }
}

impl TriggerAction for CronLikeTrigger {
    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        // schedule next tick
        ctx.schedule_after(std::time::Duration::from_secs(60)).await?;

        // emit new workflow execution payload
        let _execution_id = ctx.emit_execution(serde_json::json!({
            "trigger": "cron",
            "at": chrono::Utc::now().to_rfc3339(),
        })).await?;

        Ok(())
    }

    async fn stop(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
        Ok(())
    }
}
```
