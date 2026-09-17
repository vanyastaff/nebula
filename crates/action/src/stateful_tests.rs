use std::sync::{Arc, OnceLock};

use nebula_core::Dependencies;

use super::*;
use crate::{
    output::ActionOutput,
    result::BreakReason,
    testing::{TestActionContext, TestContextBuilder},
};

fn make_ctx() -> TestActionContext {
    TestContextBuilder::new().build()
}

// ── StatefulActionAdapter tests ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CounterState {
    count: u32,
}

struct CounterAction;

impl Action for CounterAction {
    type Input = Value;
    type Output = Value;

    fn metadata() -> crate::ActionMetadataDraft {
        crate::ActionMetadataDraft::new(
            nebula_core::action_key!("test.counter"),
            crate::metadata_name!("Counter"),
            "Counts up to 3",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatefulAction for CounterAction {
    type State = CounterState;

    fn init_state(&self) -> CounterState {
        CounterState { count: 0 }
    }

    async fn execute(
        &self,
        _input: &<Self as Action>::Input,
        state: &mut Self::State,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        state.count += 1;
        if state.count >= 3 {
            Ok(ActionResult::Break {
                output: ActionOutput::Value(serde_json::json!({"final": state.count})),
                reason: BreakReason::Completed,
            })
        } else {
            Ok(ActionResult::Continue {
                output: ActionOutput::Value(serde_json::json!({"current": state.count})),
                progress: Some(state.count as f64 / 3.0),
                delay: None,
            })
        }
    }
}

#[test]
fn stateful_adapter_is_dyn_compatible() {
    let adapter = StatefulActionAdapter::new(CounterAction).expect("valid test catalog definition");
    let _: Arc<dyn StatefulHandle> = Arc::new(adapter);
}

#[tokio::test]
async fn stateful_adapter_init_state_serializes() {
    let adapter = StatefulActionAdapter::new(CounterAction).expect("valid test catalog definition");
    let state = adapter.init_state().unwrap();
    let cs: CounterState = serde_json::from_value(state).unwrap();
    assert_eq!(cs.count, 0);
}

#[tokio::test]
async fn stateful_adapter_iterates_with_state() {
    let adapter = StatefulActionAdapter::new(CounterAction).expect("valid test catalog definition");
    let handler: Arc<dyn StatefulHandle> = Arc::new(adapter);
    let ctx = make_ctx();
    let input = handler
        .prepare_input(ActionInput::Raw(serde_json::json!({})))
        .unwrap();
    let mut state = handler.init_state().unwrap();

    // Iteration 1: count goes 0 → 1, Continue
    let result = handler.dispatch(&input, &mut state, &ctx).await.unwrap();
    assert!(matches!(result, ActionResult::Continue { .. }));
    let cs: CounterState = serde_json::from_value(state.clone()).unwrap();
    assert_eq!(cs.count, 1);

    // Iteration 2: count goes 1 → 2, Continue
    let result = handler.dispatch(&input, &mut state, &ctx).await.unwrap();
    assert!(matches!(result, ActionResult::Continue { .. }));
    let cs: CounterState = serde_json::from_value(state.clone()).unwrap();
    assert_eq!(cs.count, 2);

    // Iteration 3: count goes 2 → 3, Break
    let result = handler.dispatch(&input, &mut state, &ctx).await.unwrap();
    assert!(matches!(result, ActionResult::Break { .. }));
    let cs: CounterState = serde_json::from_value(state.clone()).unwrap();
    assert_eq!(cs.count, 3);
}

#[tokio::test]
async fn stateful_adapter_returns_validation_error_on_bad_state() {
    let adapter = StatefulActionAdapter::new(CounterAction).expect("valid test catalog definition");
    let ctx = make_ctx();
    let input = adapter
        .prepare_input(ActionInput::Raw(serde_json::json!({})))
        .unwrap();
    let mut bad_state = serde_json::json!("not a counter state");

    let err = adapter
        .dispatch(&input, &mut bad_state, &ctx)
        .await
        .unwrap_err();
    assert!(matches!(err, ActionError::Validation { .. }));
}

#[tokio::test]
async fn stateful_adapter_bad_state_error_publishes_no_payload_text() {
    // Stored state can hold a session token or a credential. The durable
    // error record is built from `ActionError`'s `Display`, so a decode
    // failure must be described without quoting the field that failed to
    // decode.
    const MARKER: &str = "MARKER-9f3a-secret";
    let adapter = StatefulActionAdapter::new(CounterAction).expect("valid test catalog definition");
    let ctx = make_ctx();
    let input = adapter
        .prepare_input(ActionInput::Raw(serde_json::json!({})))
        .unwrap();
    // `CounterState::count` is a `u32`, so the string makes `from_value`
    // fail and `CounterAction` offers no migration.
    let mut stored_state = serde_json::json!({ "count": MARKER });

    let err = adapter
        .dispatch(&input, &mut stored_state, &ctx)
        .await
        .expect_err("a string count is not a CounterState");

    // Non-vacuous: prove the failure really is the state-decode path,
    // and that it produced a described detail rather than none.
    let ActionError::Validation {
        field,
        reason,
        detail,
    } = &err
    else {
        panic!("expected ActionError::Validation, got {err:?}");
    };
    assert_eq!(*field, "state");
    assert_eq!(*reason, ValidationReason::StateDeserialization);
    assert!(detail.is_some(), "the decode failure must be described");

    let rendered = err.to_string();
    assert!(!rendered.contains(MARKER), "{rendered}");
    let debugged = format!("{err:?}");
    assert!(!debugged.contains(MARKER), "{debugged}");
}

/// Action that mutates state to `mark` then fails with the configured error.
///
/// Used to prove that `StatefulActionAdapter::execute` flushes state
/// back to JSON before propagating errors — critical for avoiding
/// duplicated side effects on retry.
struct MutateThenFailAction {
    fail_with: ActionError,
    mark: u32,
}

impl Action for MutateThenFailAction {
    type Input = Value;
    type Output = Value;

    fn metadata() -> crate::ActionMetadataDraft {
        crate::ActionMetadataDraft::new(
            nebula_core::action_key!("test.mutate_fail"),
            crate::metadata_name!("MutateFail"),
            "Mutates state then fails",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatefulAction for MutateThenFailAction {
    type State = CounterState;

    fn init_state(&self) -> CounterState {
        CounterState { count: 0 }
    }

    async fn execute(
        &self,
        _input: &<Self as Action>::Input,
        state: &mut Self::State,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        state.count = self.mark;
        Err(self.fail_with.clone())
    }
}

fn mutate_fail(fail_with: ActionError, mark: u32) -> MutateThenFailAction {
    MutateThenFailAction { fail_with, mark }
}

#[tokio::test]
async fn stateful_adapter_checkpoints_state_on_retryable_error() {
    // Prove: an action that advances cursor/counter state and then returns
    // Retryable must have its mutations flushed to the JSON state so the
    // engine checkpoints the new position. Otherwise retry replays
    // completed work.
    let adapter = StatefulActionAdapter::new(mutate_fail(
        ActionError::retryable("transient upstream error"),
        42,
    ))
    .expect("valid test catalog definition");
    let ctx = make_ctx();
    let input = adapter
        .prepare_input(ActionInput::Raw(serde_json::json!({})))
        .unwrap();
    let mut state = serde_json::json!({ "count": 0 });

    let err = adapter
        .dispatch(&input, &mut state, &ctx)
        .await
        .unwrap_err();

    assert!(err.is_retryable(), "error must still be retryable");
    assert_eq!(
        state,
        serde_json::json!({ "count": 42 }),
        "state mutations before Err must be checkpointed"
    );
}

#[tokio::test]
async fn stateful_adapter_checkpoints_state_on_fatal_error() {
    // Symmetric invariant: even fatal errors must checkpoint state, so
    // an operator debugging the failure can see the position at which
    // the action gave up.
    let adapter = StatefulActionAdapter::new(mutate_fail(ActionError::fatal("schema mismatch"), 7))
        .expect("valid test catalog definition");
    let ctx = make_ctx();
    let input = adapter
        .prepare_input(ActionInput::Raw(serde_json::json!({})))
        .unwrap();
    let mut state = serde_json::json!({ "count": 0 });

    let err = adapter
        .dispatch(&input, &mut state, &ctx)
        .await
        .unwrap_err();

    assert!(err.is_fatal());
    assert_eq!(state, serde_json::json!({ "count": 7 }));
}

#[tokio::test]
async fn stateful_adapter_preserves_state_on_validation_error() {
    // Deserialization failure happens BEFORE the typed action runs, so
    // typed_state never existed and nothing should be written back. The
    // input JSON must remain verbatim so the engine can decide how to
    // recover (e.g., schema migration path outside the adapter).
    let adapter = StatefulActionAdapter::new(CounterAction).expect("valid test catalog definition");
    let ctx = make_ctx();
    let input = adapter
        .prepare_input(ActionInput::Raw(serde_json::json!({})))
        .unwrap();
    let bad = serde_json::json!("not a counter state");
    let mut state = bad.clone();

    let err = adapter
        .dispatch(&input, &mut state, &ctx)
        .await
        .unwrap_err();

    assert!(matches!(err, ActionError::Validation { .. }));
    assert_eq!(state, bad, "state must be untouched on deser failure");
}

#[test]
fn stateful_adapter_into_inner_returns_action() {
    let adapter = StatefulActionAdapter::new(CounterAction).expect("valid test catalog definition");
    let key = adapter.metadata().base().key().clone();
    let _action = adapter.into_inner();
    assert_eq!(key, nebula_core::action_key!("test.counter"));
}
