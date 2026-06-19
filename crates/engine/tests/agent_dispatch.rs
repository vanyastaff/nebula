//! Engine-level integration tests for the Agent kind dispatch path.
//!
//! Each test exercises `execute_agent_handle` via the real `dispatch_action`
//! path and asserts on concrete output values or error variants — never
//! tautologies or bare `is_ok()`.
//!
//! Red-on-revert falsifiability: removing the `ActionHandle::Agent` dispatch
//! arm from `dispatch_action` makes every test here hit the `_ => UNKNOWN_VARIANT`
//! arm and return `RuntimeError::Internal`, which fails every assertion.

use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_action::{
    ActionContext, ActionError, AgentAction,
    action::Action,
    metadata::ActionMetadata,
    output::ActionOutput,
    result::{ActionResult, BreakReason, WaitCondition},
    testing::TestContextBuilder,
};
use nebula_core::{Dependencies, action_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner, RuntimeError,
};
use nebula_metrics::MetricsRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Shared fixture helpers ───────────────────────────────────────────────────

/// Build a minimal `ActionRuntime` for testing — no capability runner, in-process only.
fn make_runtime(registry: Arc<ActionRegistry>) -> ActionRuntime {
    let metrics = MetricsRegistry::new();
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runner = Arc::new(InProcessRunner::new(executor));
    ActionRuntime::try_new(registry, runner, DataPassingPolicy::default(), metrics)
        .expect("ActionRuntime::try_new must succeed in tests")
}

/// Build a test context with no special slots or credentials wired.
fn make_ctx() -> nebula_action::TestActionContext {
    TestContextBuilder::new().build()
}

// ── Fixture: TwoTurnAgent ────────────────────────────────────────────────────

/// Runs exactly 2 `Continue` turns then `Break`s with `{"turns": 2}`.
/// Used to prove end-to-end dispatch and correct final output shape.
struct TwoTurnAgent;

#[derive(Clone, Serialize, Deserialize)]
struct TurnCounter {
    turns_completed: u32,
}

impl Action for TwoTurnAgent {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.agent.two_turn"),
            "TwoTurnAgent",
            "continues twice then breaks",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for TwoTurnAgent {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(TwoTurnAgent)
    }
}

impl AgentAction for TwoTurnAgent {
    type Turn = TurnCounter;

    fn init_turn(&self, _input: &Value) -> TurnCounter {
        TurnCounter { turns_completed: 0 }
    }

    async fn step(
        &self,
        turn: &mut TurnCounter,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        turn.turns_completed += 1;
        if turn.turns_completed >= 2 {
            Ok(ActionResult::Break {
                output: ActionOutput::Value(serde_json::json!({ "turns": turn.turns_completed })),
                reason: BreakReason::Completed,
            })
        } else {
            Ok(ActionResult::Continue {
                output: ActionOutput::Value(Value::Null),
                progress: None,
                delay: None,
            })
        }
    }
}

// ── Fixture: StubbornContinueAgent ──────────────────────────────────────────

/// Returns `Continue` for exactly `continue_turns` steps WITHOUT touching the
/// `Turn` value, then `Break`s. The step counter lives in an `AtomicU32` on
/// the action struct itself — deliberately kept out of `Turn` so that `Turn`
/// stays bit-for-bit identical between every step call.
///
/// This proves the engine does NOT apply a stuck-state digest guard to agent
/// Continue paths: a StatefulStuck-equivalent guard would hash `Turn` before
/// and after each step, detect the unchanged bytes, and return an error.
/// This action would trigger it on the very first Continue.
struct StubbornContinueAgent {
    /// Number of Continue turns before the agent decides to Break.
    continue_turns: u32,
    /// Step counter kept OUTSIDE `Turn` to ensure `Turn` is never mutated.
    steps_taken: AtomicU32,
}

/// Turn type whose fields are never written by [`StubbornContinueAgent::step`].
/// The engine serialises this to JSON between turns; unchanged bytes are what
/// would trigger a StatefulStuck guard.
#[derive(Clone, Serialize, Deserialize)]
struct StubbornTurn {
    /// Stamp set once at `init_turn`. Never overwritten.
    initial_marker: u32,
}

impl Action for StubbornContinueAgent {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.agent.stubborn_continue"),
            "StubbornContinueAgent",
            "continues N times without mutating turn state, then breaks",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for StubbornContinueAgent {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(StubbornContinueAgent {
            continue_turns: 3,
            steps_taken: AtomicU32::new(0),
        })
    }
}

impl AgentAction for StubbornContinueAgent {
    type Turn = StubbornTurn;

    fn init_turn(&self, _input: &Value) -> StubbornTurn {
        StubbornTurn { initial_marker: 42 }
    }

    async fn step(
        &self,
        turn: &mut StubbornTurn,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        // `turn.initial_marker` is intentionally never written in this method.
        // A StatefulStuck-style digest guard would see identical bytes and error.
        let _ = turn.initial_marker; // read to prove it's accessible, not mutated
        let taken = self.steps_taken.fetch_add(1, Ordering::Relaxed);
        if taken >= self.continue_turns {
            Ok(ActionResult::break_completed(serde_json::json!({
                "steps_without_turn_mutation": taken,
            })))
        } else {
            Ok(ActionResult::Continue {
                output: ActionOutput::Value(Value::Null),
                progress: None,
                delay: None,
            })
        }
    }
}

// ── Fixture: LoopForeverAgent ────────────────────────────────────────────────

/// Always returns `Continue`, never terminates. Used to prove `max_turns`
/// budget is enforced with the exact declared limit.
struct LoopForeverAgent {
    max: u32,
}

#[derive(Clone, Serialize, Deserialize)]
struct LoopTurn {
    steps: u32,
}

impl Action for LoopForeverAgent {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.agent.loop_forever"),
            "LoopForeverAgent",
            "loops until budget is exceeded",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for LoopForeverAgent {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(LoopForeverAgent { max: 3 })
    }
}

impl AgentAction for LoopForeverAgent {
    type Turn = LoopTurn;

    fn max_turns(&self) -> u32 {
        self.max
    }

    fn init_turn(&self, _input: &Value) -> LoopTurn {
        LoopTurn { steps: 0 }
    }

    async fn step(
        &self,
        turn: &mut LoopTurn,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        turn.steps += 1;
        Ok(ActionResult::Continue {
            output: ActionOutput::Value(Value::Null),
            progress: None,
            delay: None,
        })
    }
}

// ── Fixture: SlowTurnAgent ───────────────────────────────────────────────────

/// Each step sleeps for `step_delay` (50 ms) while `turn_timeout()` returns
/// 10 ms, so every step exceeds the per-turn deadline and fires
/// `AgentTurnTimeout`. Paired with [`FastTurnAgent`] for the negative case.
struct SlowTurnAgent {
    step_delay: Duration,
}

#[derive(Clone, Serialize, Deserialize)]
struct SlowTurn {
    attempts: u32,
}

impl Action for SlowTurnAgent {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.agent.slow_turn"),
            "SlowTurnAgent",
            "sleeps per turn to exercise the per-turn timeout",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for SlowTurnAgent {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(SlowTurnAgent {
            step_delay: Duration::from_millis(50),
        })
    }
}

impl AgentAction for SlowTurnAgent {
    type Turn = SlowTurn;

    fn turn_timeout(&self) -> Option<Duration> {
        // Tight deadline — real slow steps will exceed it.
        Some(Duration::from_millis(10))
    }

    fn init_turn(&self, _input: &Value) -> SlowTurn {
        SlowTurn { attempts: 0 }
    }

    async fn step(
        &self,
        turn: &mut SlowTurn,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        turn.attempts += 1;
        tokio::time::sleep(self.step_delay).await;
        Ok(ActionResult::break_completed(serde_json::json!("done")))
    }
}

/// Fast variant: step completes well within the turn_timeout.
struct FastTurnAgent;

impl Action for FastTurnAgent {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.agent.fast_turn"),
            "FastTurnAgent",
            "completes instantly — per-turn timeout must NOT fire",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for FastTurnAgent {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(FastTurnAgent)
    }
}

impl AgentAction for FastTurnAgent {
    type Turn = SlowTurn;

    fn turn_timeout(&self) -> Option<Duration> {
        Some(Duration::from_millis(200))
    }

    fn init_turn(&self, _input: &Value) -> SlowTurn {
        SlowTurn { attempts: 0 }
    }

    async fn step(
        &self,
        turn: &mut SlowTurn,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        turn.attempts += 1;
        // No sleep — resolves immediately, well within the 200 ms window.
        Ok(ActionResult::break_completed(
            serde_json::json!({ "attempts": turn.attempts }),
        ))
    }
}

// ── Fixture: WaitReturningAgent ──────────────────────────────────────────────

/// Returns `ActionResult::Wait` on the first step — proves the engine rejects
/// the Wait arm with `AgentWaitNotSupported` (durable park/resume is not yet wired).
struct WaitReturningAgent;

#[derive(Clone, Serialize, Deserialize)]
struct WaitTurn;

impl Action for WaitReturningAgent {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.agent.wait_returning"),
            "WaitReturningAgent",
            "returns Wait — engine must reject with AgentWaitNotSupported",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for WaitReturningAgent {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(WaitReturningAgent)
    }
}

impl AgentAction for WaitReturningAgent {
    type Turn = WaitTurn;

    fn init_turn(&self, _input: &Value) -> WaitTurn {
        WaitTurn
    }

    async fn step(
        &self,
        _turn: &mut WaitTurn,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Webhook {
                callback_id: "test-callback".to_owned(),
            },
            timeout: None,
            partial_output: None,
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Proves: an agent action that runs 2 Continue turns then Breaks dispatches
/// end-to-end through the engine and delivers the exact final output.
///
/// Falsifiable: removing `ActionHandle::Agent` from the dispatch arm sends this
/// to the `_ => UNKNOWN_VARIANT` arm → `RuntimeError::Internal` → test fails on
/// the `match result` below.
#[tokio::test]
async fn agent_dispatches_through_engine() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_agent_factory::<TwoTurnAgent>();

    let runtime = make_runtime(registry);
    let ctx = make_ctx();

    let result = runtime
        .execute_action("test.agent.two_turn", serde_json::json!(null), &ctx)
        .await
        .expect("agent dispatch must succeed");

    match result {
        ActionResult::Break {
            output,
            reason: BreakReason::Completed,
        } => {
            let value = output.into_value().expect("output must be an inline Value");
            assert_eq!(
                value,
                serde_json::json!({ "turns": 2 }),
                "final output must reflect both turns completed"
            );
        },
        other => panic!("expected ActionResult::Break(Completed), got {other:?}"),
    }
}

/// Proves: the engine does NOT apply a stuck-state digest guard to agent
/// Continue paths — returning `Continue` with bit-for-bit identical `Turn`
/// state across multiple turns must NOT produce an error.
///
/// `StubbornContinueAgent` returns Continue for 3 turns without ever writing
/// to its `Turn` value, then Breaks. A StatefulStuck-equivalent guard in the
/// engine loop would detect the unchanged bytes and return an error on the
/// first (or subsequent) Continue steps.
///
/// # Falsifiability
///
/// Add the following guard at the top of the Continue arm inside
/// `execute_agent_handle`:
/// ```ignore
/// if serde_json::to_vec(&turn_state_before).ok()
///     == serde_json::to_vec(&turn_state).ok()
/// {
///     return Err(RuntimeError::Internal("agent stuck".into()));
/// }
/// ```
/// With that guard the test fails with `"agent stuck"` on the first Continue.
/// Remove the guard and the test passes — confirming the loop tolerates
/// unchanged state. Red evidence was collected and the guard removed before
/// this commit.
#[tokio::test]
async fn no_progress_turn_is_legal() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_agent_factory::<StubbornContinueAgent>();

    let runtime = make_runtime(registry);
    let ctx = make_ctx();

    let result = runtime
        .execute_action(
            "test.agent.stubborn_continue",
            serde_json::json!(null),
            &ctx,
        )
        .await
        .expect("3 Continue turns with unchanged Turn state must not produce a stuck-state error");

    match result {
        ActionResult::Break {
            output,
            reason: BreakReason::Completed,
        } => {
            let value = output.into_value().expect("output must be an inline Value");
            // `steps_without_turn_mutation` == 3 proves the engine went through
            // all 3 Continue iterations without raising a stuck-state error.
            assert_eq!(
                value["steps_without_turn_mutation"],
                serde_json::json!(3),
                "must have completed exactly 3 unchanged-state Continue turns"
            );
        },
        other => panic!("expected ActionResult::Break(Completed), got {other:?}"),
    }
}

/// Proves: when `max_turns()` is exhausted, the engine returns
/// `RuntimeError::AgentBudgetExceeded` with the exact declared limit.
///
/// Falsifiable: the wrong limit or wrong variant causes the assertion to fail.
#[tokio::test]
async fn max_turns_budget_enforced() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_agent_factory::<LoopForeverAgent>();

    let runtime = make_runtime(registry);
    let ctx = make_ctx();

    let err = runtime
        .execute_action("test.agent.loop_forever", serde_json::json!(null), &ctx)
        .await
        .expect_err("a non-terminating agent must hit its budget and return an error");

    match err {
        RuntimeError::AgentBudgetExceeded { max_turns, .. } => {
            assert_eq!(
                max_turns, 3,
                "budget error must carry the exact declared max_turns"
            );
        },
        other => panic!("expected RuntimeError::AgentBudgetExceeded, got {other:?}"),
    }
}

/// Proves: when `turn_timeout()` fires, the engine returns
/// `RuntimeError::AgentTurnTimeout` with the correct turn index and timeout.
/// A companion fast-turn case proves the path is NOT always triggered.
///
/// Falsifiable: if the timeout were not wired, the slow test would complete
/// and the `expect_err` would panic.
#[tokio::test(start_paused = true)]
async fn per_turn_timeout_fires() {
    // ── Slow case: turn exceeds the 10 ms deadline ──
    let registry = Arc::new(ActionRegistry::new());
    registry.register_agent_factory::<SlowTurnAgent>();

    let runtime = make_runtime(registry);
    let ctx = make_ctx();

    let err = runtime
        .execute_action("test.agent.slow_turn", serde_json::json!(null), &ctx)
        .await
        .expect_err("a turn that sleeps past its deadline must time out");

    match err {
        RuntimeError::AgentTurnTimeout { turn, timeout, .. } => {
            assert_eq!(turn, 0, "timeout must fire on the first (zeroth) turn");
            assert_eq!(
                timeout,
                Duration::from_millis(10),
                "timeout must carry the declared per-turn deadline"
            );
        },
        other => panic!("expected RuntimeError::AgentTurnTimeout, got {other:?}"),
    }

    // ── Fast case: turn completes well within the 200 ms deadline ──
    let fast_registry = Arc::new(ActionRegistry::new());
    fast_registry.register_agent_factory::<FastTurnAgent>();

    let fast_runtime = make_runtime(fast_registry);
    let fast_ctx = make_ctx();

    let fast_result = fast_runtime
        .execute_action("test.agent.fast_turn", serde_json::json!(null), &fast_ctx)
        .await
        .expect("a fast turn must complete without hitting the per-turn timeout");

    assert!(
        matches!(fast_result, ActionResult::Break { .. }),
        "fast turn must reach Break, not timeout; got {fast_result:?}"
    );
}

/// Proves: returning `ActionResult::Wait` from an agent step yields
/// `RuntimeError::AgentWaitNotSupported` — the engine does not yet support Wait.
///
/// Falsifiable: if Wait were silently mishandled (e.g. mapped to Break or ignored),
/// no error would be returned and `expect_err` would panic.
#[tokio::test]
async fn wait_step_rejected_in_a_s1() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_agent_factory::<WaitReturningAgent>();

    let runtime = make_runtime(registry);
    let ctx = make_ctx();

    let err = runtime
        .execute_action("test.agent.wait_returning", serde_json::json!(null), &ctx)
        .await
        .expect_err("returning Wait from an agent step must produce AgentWaitNotSupported");

    assert!(
        matches!(err, RuntimeError::AgentWaitNotSupported { .. }),
        "expected RuntimeError::AgentWaitNotSupported, got {err:?}"
    );
}
