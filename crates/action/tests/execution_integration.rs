//! Integration tests for execution traits (StatelessAction, StatefulAction, TriggerAction).
//!
//! These tests run real implementations with ActionContext/TriggerContext and assert
//! on ActionResult/ActionError. They do not test cancellation (that is the runtime's
//! responsibility via tokio::select!).

use nebula_action::{
    Action, ActionContext, ActionMetadata, ActionOutput, ActionResult, BreakReason, StatefulAction,
    StatelessAction, TriggerAction, TriggerContext,
};
use nebula_action::dependency::ActionDependencies;
use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use tokio_util::sync::CancellationToken;

// ── StatelessAction ─────────────────────────────────────────────────────────

struct EchoAction {
    meta: ActionMetadata,
}

impl ActionDependencies for EchoAction {}

impl Action for EchoAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatelessAction for EchoAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl nebula_action::Context,
    ) -> Result<ActionResult<Self::Output>, nebula_action::ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[tokio::test]
async fn stateless_action_execute_returns_success() {
    let action = EchoAction {
        meta: ActionMetadata::new("test.echo", "Echo", "Echo input to output"),
    };
    let ctx = ActionContext::new(
        ExecutionId::new(),
        NodeId::new(),
        WorkflowId::new(),
        CancellationToken::new(),
    );
    let input = serde_json::json!({ "x": 1 });
    let result = action.execute(input.clone(), &ctx).await.unwrap();
    match &result {
        ActionResult::Success { output } => {
            let v = output.as_value().expect("value");
            assert_eq!(v, &input);
        }
        _ => panic!("expected Success, got {:?}", result),
    }
}

// ── StatefulAction ──────────────────────────────────────────────────────────

struct CounterAction {
    meta: ActionMetadata,
}

impl ActionDependencies for CounterAction {}

impl Action for CounterAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
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
    ) -> Result<ActionResult<Self::Output>, nebula_action::ActionError> {
        let count = *state;
        *state += 1;
        if count < 2 {
            Ok(ActionResult::Continue {
                output: ActionOutput::Value(count),
                progress: Some((count + 1) as f64 / 3.0),
                delay: None,
            })
        } else {
            Ok(ActionResult::Break {
                output: ActionOutput::Value(count),
                reason: BreakReason::Completed,
            })
        }
    }
}

#[tokio::test]
async fn stateful_action_continue_then_break() {
    let action = CounterAction {
        meta: ActionMetadata::new("test.counter", "Counter", "Count then break"),
    };
    let ctx = ActionContext::new(
        ExecutionId::new(),
        NodeId::new(),
        WorkflowId::new(),
        CancellationToken::new(),
    );
    let mut state = 0u32;

    let r0 = action.execute((), &mut state, &ctx).await.unwrap();
    match &r0 {
        ActionResult::Continue { output, .. } => {
            assert_eq!(output.as_value(), Some(&0));
        }
        _ => panic!("expected Continue, got {:?}", r0),
    }
    assert_eq!(state, 1);

    let r1 = action.execute((), &mut state, &ctx).await.unwrap();
    match &r1 {
        ActionResult::Continue { output, .. } => {
            assert_eq!(output.as_value(), Some(&1));
        }
        _ => panic!("expected Continue, got {:?}", r1),
    }
    assert_eq!(state, 2);

    let r2 = action.execute((), &mut state, &ctx).await.unwrap();
    match &r2 {
        ActionResult::Break { output, reason } => {
            assert_eq!(output.as_value(), Some(&2));
            assert_eq!(*reason, BreakReason::Completed);
        }
        _ => panic!("expected Break, got {:?}", r2),
    }
    assert_eq!(state, 3);
}

// ── TriggerAction ───────────────────────────────────────────────────────────

struct NoOpTrigger {
    meta: ActionMetadata,
}

impl ActionDependencies for NoOpTrigger {}

impl Action for NoOpTrigger {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl TriggerAction for NoOpTrigger {
    async fn start(&self, _ctx: &TriggerContext) -> Result<(), nebula_action::ActionError> {
        Ok(())
    }

    async fn stop(&self, _ctx: &TriggerContext) -> Result<(), nebula_action::ActionError> {
        Ok(())
    }
}

#[tokio::test]
async fn trigger_action_start_stop_succeed() {
    let action = NoOpTrigger {
        meta: ActionMetadata::new("test.noop_trigger", "NoOp Trigger", "Start/stop no-op"),
    };
    let ctx = TriggerContext::new(WorkflowId::new(), NodeId::new(), CancellationToken::new());
    action.start(&ctx).await.unwrap();
    action.stop(&ctx).await.unwrap();
}
