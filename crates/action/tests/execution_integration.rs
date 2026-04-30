//! Integration tests for execution traits (StatelessAction, StatefulAction, TriggerAction).
//!
//! These tests run real implementations with ActionContext/TriggerContext and assert
//! on ActionResult/ActionError. They do not test cancellation (that is the runtime's
//! responsibility via tokio::select!).

use std::sync::OnceLock;

use nebula_action::{
    Action, ActionError, ActionMetadata, ActionOutput, ActionResult, BreakReason, StatefulAction,
    StatefulActionAdapter, StatefulHandler, StatelessAction, TriggerAction, TriggerSource,
    testing::TestContextBuilder,
};
use nebula_core::{Dependencies, action_key};
use nebula_schema::{HasSchema, ValidSchema};

// ── TestSource — generic trigger source for test fixtures ───────────────────

struct TestSource;
impl TriggerSource for TestSource {
    type Event = serde_json::Value;
}

// Boilerplate helper used across fixtures.
fn empty_deps() -> &'static Dependencies {
    static D: OnceLock<Dependencies> = OnceLock::new();
    D.get_or_init(Dependencies::new)
}

fn json_schema() -> &'static ValidSchema {
    static S: OnceLock<ValidSchema> = OnceLock::new();
    S.get_or_init(<serde_json::Value as HasSchema>::schema)
}

// ── StatelessAction ─────────────────────────────────────────────────────────

struct EchoAction;

impl Action for EchoAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(action_key!("test.echo"), "Echo", "Echo input to output")
        })
    }

    fn input_schema() -> &'static ValidSchema {
        json_schema()
    }
    fn output_schema() -> &'static ValidSchema {
        json_schema()
    }
    fn dependencies() -> &'static Dependencies {
        empty_deps()
    }
}

impl StatelessAction for EchoAction {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[tokio::test]
async fn stateless_action_execute_returns_success() {
    let action = EchoAction;
    let ctx = TestContextBuilder::new().build();
    let input = serde_json::json!({ "x": 1 });
    let result = action.execute(input.clone(), &ctx).await.unwrap();
    match &result {
        ActionResult::Success { output } => {
            let v = output.as_value().expect("value");
            assert_eq!(v, &input);
        },
        _ => panic!("expected Success, got {result:?}"),
    }
}

// ── StatefulAction ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct U32Out(u32);

impl HasSchema for U32Out {
    fn schema() -> ValidSchema {
        ValidSchema::empty()
    }
}

struct CounterAction;

impl Action for CounterAction {
    type Input = ();
    type Output = U32Out;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(action_key!("test.counter"), "Counter", "Count then break")
        })
    }

    fn input_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<() as HasSchema>::schema)
    }
    fn output_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<U32Out as HasSchema>::schema)
    }
    fn dependencies() -> &'static Dependencies {
        empty_deps()
    }
}

impl StatefulAction for CounterAction {
    type State = u32;

    fn init_state(&self) -> Self::State {
        0
    }

    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        state: &mut Self::State,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        let count = *state;
        *state += 1;
        if count < 2 {
            Ok(ActionResult::Continue {
                output: ActionOutput::Value(U32Out(count)),
                progress: Some((count + 1) as f64 / 3.0),
                delay: None,
            })
        } else {
            Ok(ActionResult::Break {
                output: ActionOutput::Value(U32Out(count)),
                reason: BreakReason::Completed,
            })
        }
    }
}

#[tokio::test]
async fn stateful_action_continue_then_break() {
    let action = CounterAction;
    let ctx = TestContextBuilder::new().build();
    let mut state = 0u32;

    let r0 = action.execute((), &mut state, &ctx).await.unwrap();
    match &r0 {
        ActionResult::Continue { output, .. } => {
            assert_eq!(output.as_value(), Some(&U32Out(0)));
        },
        _ => panic!("expected Continue, got {r0:?}"),
    }
    assert_eq!(state, 1);

    let r1 = action.execute((), &mut state, &ctx).await.unwrap();
    match &r1 {
        ActionResult::Continue { output, .. } => {
            assert_eq!(output.as_value(), Some(&U32Out(1)));
        },
        _ => panic!("expected Continue, got {r1:?}"),
    }
    assert_eq!(state, 2);

    let r2 = action.execute((), &mut state, &ctx).await.unwrap();
    match &r2 {
        ActionResult::Break { output, reason } => {
            assert_eq!(output.as_value(), Some(&U32Out(2)));
            assert_eq!(*reason, BreakReason::Completed);
        },
        _ => panic!("expected Break, got {r2:?}"),
    }
    assert_eq!(state, 3);
}

// ── TriggerAction ───────────────────────────────────────────────────────────

struct NoOpTrigger;

impl Action for NoOpTrigger {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(
                action_key!("test.noop_trigger"),
                "NoOp Trigger",
                "Start/stop no-op",
            )
        })
    }

    fn input_schema() -> &'static ValidSchema {
        json_schema()
    }
    fn output_schema() -> &'static ValidSchema {
        json_schema()
    }
    fn dependencies() -> &'static Dependencies {
        empty_deps()
    }
}

impl TriggerAction for NoOpTrigger {
    type Source = TestSource;
    type Error = ActionError;

    async fn start(
        &self,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        Ok(())
    }

    async fn stop(
        &self,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        Ok(())
    }

    async fn handle(
        &self,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
        _event: serde_json::Value,
    ) -> Result<nebula_action::TriggerEventOutcome, ActionError> {
        Err(ActionError::fatal(
            "NoOpTrigger does not accept external events",
        ))
    }
}

#[tokio::test]
async fn trigger_action_start_stop_succeed() {
    let action = NoOpTrigger;
    let ctx = TestContextBuilder::new().build_trigger().0;
    action.start(&ctx).await.unwrap();
    action.stop(&ctx).await.unwrap();
}

// ── migrate_state tests ────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct MigratableState {
    count: u32,
    label: String,
}

struct MigratableAction;

impl Action for MigratableAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(
                action_key!("test.migratable"),
                "Migratable",
                "Migrates v1 state",
            )
        })
    }

    fn input_schema() -> &'static ValidSchema {
        json_schema()
    }
    fn output_schema() -> &'static ValidSchema {
        json_schema()
    }
    fn dependencies() -> &'static Dependencies {
        empty_deps()
    }
}

impl StatefulAction for MigratableAction {
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
        _input: <Self as Action>::Input,
        state: &mut Self::State,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
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
    let action = MigratableAction;
    let adapter = StatefulActionAdapter::new(action);
    let ctx = TestContextBuilder::new().build();

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
    let action = CounterAction;
    let adapter = StatefulActionAdapter::new(action);
    let ctx = TestContextBuilder::new().build();

    // Completely invalid state — CounterAction does not override migrate_state (returns None).
    let mut state = serde_json::json!("not_an_object");
    let result = adapter
        .execute(&serde_json::Value::Null, &mut state, &ctx)
        .await;

    nebula_action::assert_validation_error!(result);
}
