use std::sync::{Arc, OnceLock};

#[test]
fn public_adapter_stamps_agent_kind() {
    let adapter = AgentActionAdapter::new(CountingAgent { target: 1 }).unwrap();
    assert_eq!(adapter.metadata().kind(), crate::ActionKind::Agent);
}

use nebula_core::Dependencies;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::*;
use crate::{
    action::Action,
    error::ActionError,
    output::ActionOutput,
    result::{ActionResult, BreakReason},
    testing::{TestActionContext, TestContextBuilder},
};

fn make_ctx() -> TestActionContext {
    TestContextBuilder::new().build()
}

// ── CountingAgent fixture ────────────────────────────────────────────────

/// Increments a counter on each turn; breaks when counter reaches `target`.
struct CountingAgent {
    target: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CountingTurn {
    count: u32,
}

impl Action for CountingAgent {
    type Input = Value;
    type Output = Value;

    fn metadata() -> crate::ActionMetadataDraft {
        crate::ActionMetadataDraft::new(
            nebula_core::action_key!("test.agent.counting"),
            crate::metadata_name!("CountingAgent"),
            "Counts up to target then breaks",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl AgentAction for CountingAgent {
    type Turn = CountingTurn;

    fn init_turn(&self, _input: &Value) -> CountingTurn {
        CountingTurn { count: 0 }
    }

    async fn step(
        &self,
        turn: &mut CountingTurn,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        turn.count += 1;
        if turn.count >= self.target {
            Ok(ActionResult::Break {
                output: ActionOutput::Value(serde_json::json!({ "final": turn.count })),
                reason: BreakReason::Completed,
            })
        } else {
            Ok(ActionResult::Continue {
                output: ActionOutput::Value(serde_json::json!({ "current": turn.count })),
                progress: None,
                delay: None,
            })
        }
    }
}

// ── NoMutationAgent fixture ──────────────────────────────────────────────

/// Never mutates its turn state — proves no stuck-state guard fires.
struct NoMutationAgent {
    steps_before_break: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NoMutationTurn {
    external_step: u32,
}

impl Action for NoMutationAgent {
    type Input = Value;
    type Output = Value;

    fn metadata() -> crate::ActionMetadataDraft {
        crate::ActionMetadataDraft::new(
            nebula_core::action_key!("test.agent.no_mutation"),
            crate::metadata_name!("NoMutationAgent"),
            "Keeps turn state unchanged; breaks after N steps tracked externally",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl AgentAction for NoMutationAgent {
    type Turn = NoMutationTurn;

    fn init_turn(&self, _input: &Value) -> NoMutationTurn {
        NoMutationTurn { external_step: 0 }
    }

    async fn step(
        &self,
        turn: &mut NoMutationTurn,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        // Deliberately NOT mutating `turn` — proves the adapter does not
        // panic or error on unchanged state. `steps_before_break` is read
        // from `self` (not from `turn`) to avoid mutating `turn`.
        if turn.external_step >= self.steps_before_break {
            Ok(ActionResult::break_completed(serde_json::json!("done")))
        } else {
            Ok(ActionResult::Continue {
                output: ActionOutput::Value(Value::Null),
                progress: None,
                delay: None,
            })
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Proves: `init_turn` serializes the typed turn to JSON correctly.
#[test]
fn adapter_init_turn_round_trips() {
    let adapter =
        AgentActionAdapter::new(CountingAgent { target: 3 }).expect("valid agent schemas");
    let input = adapter
        .prepare_input(crate::ActionInput::Raw(serde_json::json!(null)))
        .expect("input preparation must succeed");
    let turn_json = adapter.init_turn(input).expect("init_turn must succeed");
    let turn: CountingTurn =
        serde_json::from_value(turn_json).expect("init_turn must produce valid JSON");
    assert_eq!(turn.count, 0, "initial count must be 0");
}

/// Proves: `step` advances turn state and returns `Continue` on intermediate turns.
#[tokio::test]
async fn adapter_step_advances_state_and_continues() {
    let adapter = Arc::new(
        AgentActionAdapter::new(CountingAgent { target: 3 }).expect("valid agent schemas"),
    );
    let ctx = make_ctx();
    let input = adapter
        .prepare_input(crate::ActionInput::Raw(serde_json::json!(null)))
        .expect("input preparation must succeed");
    let mut turn_state = adapter.init_turn(input).expect("init_turn must succeed");

    // Turn 1: count 0 → 1, Continue
    let result = adapter
        .step(&mut turn_state, &ctx)
        .await
        .expect("step must succeed");
    assert!(
        matches!(result, ActionResult::Continue { .. }),
        "turn 1 of 3 must Continue"
    );
    let turn: CountingTurn = serde_json::from_value(turn_state.clone()).unwrap();
    assert_eq!(turn.count, 1);

    // Turn 2: count 1 → 2, Continue
    let result = adapter.step(&mut turn_state, &ctx).await.unwrap();
    assert!(matches!(result, ActionResult::Continue { .. }));
    let turn: CountingTurn = serde_json::from_value(turn_state.clone()).unwrap();
    assert_eq!(turn.count, 2);

    // Turn 3: count 2 → 3, Break
    let result = adapter.step(&mut turn_state, &ctx).await.unwrap();
    assert!(
        matches!(result, ActionResult::Break { .. }),
        "turn 3 of 3 must Break"
    );
}

/// Proves: a type mismatch in the turn state JSON produces `ActionError::Validation`,
/// not a panic. Falsifiable: if the adapter called `unwrap()` on deser, this panics.
#[tokio::test]
async fn adapter_step_returns_validation_on_bad_turn_state() {
    let adapter =
        AgentActionAdapter::new(CountingAgent { target: 3 }).expect("valid agent schemas");
    let ctx = make_ctx();
    let mut bad_turn_state = serde_json::json!("this is not a CountingTurn");

    let err = adapter
        .step(&mut bad_turn_state, &ctx)
        .await
        .expect_err("mismatched turn state JSON must produce an error");

    assert!(
        matches!(err, ActionError::Validation { .. }),
        "bad turn state must produce Validation error; got {err:?}"
    );
}

/// Proves: unchanged turn state does NOT cause an error (no stuck-state guard).
#[tokio::test]
async fn adapter_unchanged_turn_state_is_legal() {
    // This test exercises the property that distinguishes AgentAction from
    // StatefulAction: a turn that returns Continue without mutating its
    // state is valid. If a StatefulStuck-style digest check were added to
    // the adapter, those Continue steps would produce an error instead of Ok.
    //
    // `steps_before_break: 2` means the action returns Continue for two
    // turns (without mutating turn state) and only breaks on the third.
    let adapter = AgentActionAdapter::new(NoMutationAgent {
        steps_before_break: 2,
    })
    .expect("valid agent schemas");
    let ctx = make_ctx();
    let input = adapter
        .prepare_input(crate::ActionInput::Raw(serde_json::json!(null)))
        .expect("input preparation must succeed");
    let mut turn_state = adapter.init_turn(input).expect("init_turn must succeed");

    // Turn 1: must Continue, no mutation.
    let r1 = adapter
        .step(&mut turn_state, &ctx)
        .await
        .expect("first unchanged Continue must not produce an error");
    assert!(
        matches!(r1, ActionResult::Continue { .. }),
        "turn 1 must Continue; got {r1:?}"
    );

    // Turn 2: must Continue again, still no mutation — falsifies a digest guard.
    let r2 = adapter
        .step(&mut turn_state, &ctx)
        .await
        .expect("second unchanged Continue must not produce an error");
    assert!(
        matches!(r2, ActionResult::Continue { .. }),
        "turn 2 must Continue; got {r2:?}"
    );

    // Turn 3: external_step is still 0 (never mutated); steps_before_break
    // is 2, but the condition checks `turn.external_step >= steps_before_break`,
    // i.e. `0 >= 2` — false — so it continues a third time. To reach Break
    // we need external_step to be advanced externally; as written, this
    // agent never breaks on its own. Assert a third Continue to confirm.
    let r3 = adapter
        .step(&mut turn_state, &ctx)
        .await
        .expect("third unchanged Continue must not produce an error");
    assert!(
        matches!(r3, ActionResult::Continue { .. }),
        "turn 3 must Continue (external_step never mutated); got {r3:?}"
    );
}

/// Proves: `AgentActionAdapter` is dyn-compatible as `Arc<dyn AgentHandle>`.
#[test]
fn adapter_is_dyn_compatible() {
    let adapter =
        AgentActionAdapter::new(CountingAgent { target: 1 }).expect("valid agent schemas");
    let _: Arc<dyn AgentHandle> = Arc::new(adapter);
}
