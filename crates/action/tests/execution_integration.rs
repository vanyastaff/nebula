//! Integration tests for execution traits (StatelessAction, StatefulAction, TriggerAction).
//!
//! These tests run real implementations with ActionContext/TriggerContext and assert
//! on ActionResult/ActionError. They do not test cancellation (that is the runtime's
//! responsibility via tokio::select!).

use nebula_action::{
    Action, ActionContext, ActionMetadata, ActionOutput, ActionResult, BreakReason, StatefulAction,
    StatefulActionAdapter, StatefulHandler, StatelessAction, TriggerAction, TriggerContext,
    dependency::ActionDependencies,
};
use nebula_core::{
    action_key,
    id::{ExecutionId, NodeId, WorkflowId},
};
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
        meta: ActionMetadata::new(action_key!("test.echo"), "Echo", "Echo input to output"),
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

    fn init_state(&self) -> Self::State {
        0
    }

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
        meta: ActionMetadata::new(action_key!("test.counter"), "Counter", "Count then break"),
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
        meta: ActionMetadata::new(
            action_key!("test.noop_trigger"),
            "NoOp Trigger",
            "Start/stop no-op",
        ),
    };
    let ctx = TriggerContext::new(WorkflowId::new(), NodeId::new(), CancellationToken::new());
    action.start(&ctx).await.unwrap();
    action.stop(&ctx).await.unwrap();
}

// ── migrate_state tests ────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct MigratableState {
    count: u32,
    label: String,
}

struct MigratableAction {
    meta: ActionMetadata,
}

impl ActionDependencies for MigratableAction {}

impl Action for MigratableAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl StatefulAction for MigratableAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;
    type State = MigratableState;

    fn init_state(&self) -> Self::State {
        MigratableState {
            count: 0,
            label: String::from("default"),
        }
    }

    fn migrate_state(&self, old: serde_json::Value) -> Option<Self::State> {
        // Handle v1 state: { "count": N } → add default label
        let count = old.get("count")?.as_u64()? as u32;
        Some(MigratableState {
            count,
            label: String::from("migrated"),
        })
    }

    async fn execute(
        &self,
        _input: Self::Input,
        state: &mut Self::State,
        _ctx: &impl nebula_action::Context,
    ) -> Result<ActionResult<Self::Output>, nebula_action::ActionError> {
        state.count += 1;
        Ok(ActionResult::Break {
            output: ActionOutput::Value(
                serde_json::json!({ "count": state.count, "label": state.label }),
            ),
            reason: BreakReason::Completed,
        })
    }
}

#[tokio::test]
async fn migrate_state_succeeds_from_v1() {
    let action = MigratableAction {
        meta: ActionMetadata::new(
            action_key!("test.migratable"),
            "Migratable",
            "Migrates v1 state",
        ),
    };
    let adapter = StatefulActionAdapter::new(action);
    let ctx = ActionContext::new(
        ExecutionId::new(),
        NodeId::new(),
        WorkflowId::new(),
        CancellationToken::new(),
    );

    // v1 state — missing the `label` field, so direct deser into MigratableState fails.
    // migrate_state should kick in and supply default label.
    let mut state = serde_json::json!({ "count": 5 });
    let result = adapter
        .execute(&serde_json::json!({}), &mut state, &ctx)
        .await;

    nebula_action::assert_break!(result);
}

#[tokio::test]
async fn migrate_state_propagates_error_when_none() {
    let action = CounterAction {
        meta: ActionMetadata::new(action_key!("test.counter"), "Counter", "Count then break"),
    };
    let adapter = StatefulActionAdapter::new(action);
    let ctx = ActionContext::new(
        ExecutionId::new(),
        NodeId::new(),
        WorkflowId::new(),
        CancellationToken::new(),
    );

    // Completely invalid state — CounterAction does not override migrate_state (returns None).
    let mut state = serde_json::json!("not_an_object");
    let result = adapter
        .execute(&serde_json::Value::Null, &mut state, &ctx)
        .await;

    nebula_action::assert_validation_error!(result);
}
