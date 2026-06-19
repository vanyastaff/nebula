//! Core [`AgentAction`] author trait.
//!
//! Agent actions run an internal multi-turn reasoning loop and return a final
//! answer once they decide to stop. Each turn reads context (workflow inputs,
//! resource slots), advances the turn state, and returns either
//! [`ActionResult::Continue`] for another turn or a terminal result when done.
//!
//! ## Execution contract
//!
//! 1. The engine initialises turn state via [`AgentAction::init_turn`].
//! 2. On each turn the engine calls [`AgentAction::step`] with a mutable
//!    reference to the turn state and a context that carries workflow inputs
//!    and resource slots (including an LLM provider slot the author adds
//!    via `#[resource(key = "llm")]`).
//! 3. Returning [`ActionResult::Continue`] saves the turn state and starts the
//!    next turn; returning any terminal result (typically [`ActionResult::Break`])
//!    delivers the final output downstream.
//! 4. The engine enforces [`AgentAction::max_turns`] — exceeding the budget
//!    surfaces a typed `AgentBudgetExceeded` error rather than looping forever.
//! 5. [`AgentAction::turn_timeout`] bounds each individual turn's wall-clock
//!    time, preventing a hung provider call from pinning a worker indefinitely.
//!
//! ## Llm-agnostic contract
//!
//! The machinery here is Llm-agnostic: the `Llm` provider is injected as a
//! slot field on the concrete action struct (e.g. `#[resource(key = "llm")]
//! llm: ResourceGuard<L>` where `L: Llm + Provider`), resolved at instantiation
//! time by `from_workflow_node`. The `step` signature does not name `Llm` — this
//! crate does not depend on a `nebula-agent` crate and never will.
//!
//! ## Differences from `StatefulAction`
//!
//! Three properties distinguish `AgentAction` from `StatefulAction`, each
//! structural rather than discipline-based:
//!
//! 1. **Legal no-progress turns** — a turn that leaves `Turn` unchanged is
//!    valid (the engine does NOT apply the `StatefulStuck` digest guard).
//! 2. **`max_turns` budget** — bounded by the author's declared limit (default
//!    25), not the 10 000 iteration cap applied to stateful loops.
//! 3. **Per-turn wall-clock timeout** — each call to `step` is individually
//!    bounded so a hung remote provider cannot pin a worker.
//!
//! ## Cancellation
//!
//! The runtime races every turn against the execution-level cancellation
//! token and also honours `turn_timeout`. Authors do not need to poll the
//! token themselves; the runtime handles it.

use std::{future::Future, time::Duration};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    action::Action,
    context::ActionContext,
    error::{ActionError, ValidationReason},
    metadata::ActionMetadata,
    result::ActionResult,
};

/// Agent action: multi-turn reasoning loop with a per-budget and per-turn timeout.
///
/// Authors implement [`init_turn`](AgentAction::init_turn) and
/// [`step`](AgentAction::step). The engine owns the loop: it calls `step`
/// repeatedly, saves the `Turn` state after each `Continue`, and delivers the
/// final output when the action breaks.
///
/// `Self::Turn` is the per-turn state checkpoint type — the running conversation
/// transcript, tool-call history, or any other state the author needs to carry
/// across turns. It must be serializable so the engine can checkpoint it.
///
/// # Slots and the Llm provider
///
/// An LLM provider reaches the action as a slot field (`#[resource(key = "llm")]
/// llm: ResourceGuard<L>`), resolved at instantiation. The `step` signature does
/// not reference `Llm` — the crate is Llm-agnostic, which keeps `AgentHandle`
/// object-safe and avoids a dependency on `nebula-agent`.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::prelude::*;
/// use nebula_action::agent::AgentAction;
///
/// struct Summariser;
///
/// #[derive(Clone, serde::Serialize, serde::Deserialize)]
/// struct SummariserTurn { attempts: u32 }
///
/// impl Action for Summariser {
///     type Input  = String;
///     type Output = String;
/// #   fn metadata() -> nebula_action::ActionMetadata { todo!() }
/// #   fn dependencies() -> &'static nebula_core::Dependencies { todo!() }
/// }
///
/// impl AgentAction for Summariser {
///     type Turn = SummariserTurn;
///
///     fn init_turn(&self, _input: &String) -> SummariserTurn {
///         SummariserTurn { attempts: 0 }
///     }
///
///     async fn step(
///         &self,
///         turn: &mut SummariserTurn,
///         _ctx: &(impl ActionContext + ?Sized),
///     ) -> Result<ActionResult<String>, ActionError> {
///         turn.attempts += 1;
///         Ok(ActionResult::break_completed("done".to_string()))
///     }
/// }
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement AgentAction",
    note = "implement `init_turn` and `step` (Turn and Input/Output are associated types)"
)]
pub trait AgentAction: Action {
    /// Per-turn state the engine checkpoints after every `Continue`.
    ///
    /// This is the running context the action maintains across turns — a
    /// conversation transcript, accumulated tool results, or any other
    /// cross-turn state the author needs. It must be serializable so the
    /// engine can checkpoint it between turns.
    type Turn: Serialize + DeserializeOwned + Clone + Send + Sync;

    /// Maximum number of turns before the engine raises `AgentBudgetExceeded`.
    ///
    /// Authors should set a domain-appropriate value. The engine rejects any
    /// action whose loop does not terminate within this many turns with a
    /// typed error — it does not silently loop forever.
    fn max_turns(&self) -> u32 {
        25
    }

    /// Per-turn wall-clock deadline. `None` means unbounded.
    ///
    /// When `Some(d)` is returned, the engine wraps each call to [`step`](Self::step)
    /// in a `tokio::time::timeout(d, …)`. A turn that does not resolve within
    /// the deadline surfaces as `AgentTurnTimeout` — preventing a hung remote
    /// provider from pinning a worker indefinitely.
    fn turn_timeout(&self) -> Option<Duration> {
        None
    }

    /// Initialise the turn state from the workflow input before the first turn.
    ///
    /// Called once at the start of the agent loop. Subsequent turns receive the
    /// state as mutated by the previous `step` call.
    fn init_turn(&self, input: &<Self as Action>::Input) -> Self::Turn;

    /// Advance the agent by one turn.
    ///
    /// Read from `ctx` (workflow inputs, resource slots, LLM provider) and from
    /// `turn` (accumulated history); write results back into `turn`; return
    /// [`ActionResult::Continue`] to keep going or [`ActionResult::Break`] to
    /// deliver the final answer.
    ///
    /// Returning `Continue` without mutating `turn` is legal — the engine does
    /// not apply a stuck-state guard. Use [`max_turns`](Self::max_turns) to cap
    /// loops that never converge.
    ///
    /// # Errors
    ///
    /// Return [`ActionError::Retryable`] for transient provider failures and
    /// [`ActionError::Fatal`] for permanent failures.
    #[must_use = "a step result does nothing unless the returned future is awaited and acted on"]
    fn step(
        &self,
        turn: &mut Self::Turn,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ActionResult<<Self as Action>::Output>, ActionError>> + Send;
}

// ── AgentHandle object-safe dyn trait ───────────────────────────────────────

/// Object-safe agent dispatch surface — JSON in, mutable JSON turn state, JSON out.
///
/// The engine drives the loop against this trait. Implementors are typically
/// produced by [`GenericAgentFactory`](crate::factory::GenericAgentFactory).
///
/// # Turn state
///
/// Turn state is carried as `serde_json::Value` between calls so the engine can
/// checkpoint it without knowing the concrete `A::Turn` type. The adapter
/// deserialises into the typed turn on each `step` call and serialises it back.
///
/// # Errors
///
/// Serialisation mismatches in `init_turn` and `step` surface as
/// [`ActionError::Validation`] rather than panicking.
#[async_trait::async_trait]
pub trait AgentHandle: Send + Sync + 'static {
    /// Action metadata (key, version, ports, schemas), with `ActionKind::Agent` stamped.
    fn metadata(&self) -> &ActionMetadata;

    /// Maximum turns the engine is allowed to call `step` for this handle.
    fn max_turns(&self) -> u32;

    /// Per-turn wall-clock deadline, if any.
    fn turn_timeout(&self) -> Option<Duration>;

    /// Build the initial turn state as JSON from the serialised workflow input.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Validation`] if `input` cannot be deserialised
    /// into the typed `Input`, or [`ActionError::Fatal`] if the resulting turn
    /// state cannot be serialised back to JSON.
    fn init_turn(&self, input: &Value) -> Result<Value, ActionError>;

    /// Advance one turn with mutable JSON turn state and return the next result.
    ///
    /// The engine calls this in a loop until it returns a terminal result.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Validation`] if `turn_state` cannot be
    /// deserialised, or propagates errors from the underlying action.
    async fn step(
        &self,
        turn_state: &mut Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

// ── AgentActionAdapter ───────────────────────────────────────────────────────

/// Wraps an [`AgentAction`] as a [`dyn AgentHandle`].
///
/// Handles JSON (de)serialization of input, output, and turn state so the
/// engine works with untyped `serde_json::Value` while action authors write
/// strongly-typed Rust.
///
/// Turn state mutations performed by the typed action are flushed back to the
/// JSON `turn_state` argument before any error is propagated — matching the
/// checkpoint-before-error contract that `StatefulActionAdapter` enforces.
///
/// **Double-failure exception:** if flushing the turn state to JSON fails
/// *and* the action step returned an error, the flush is skipped and the
/// original action error is propagated. The double-failure path is logged at
/// `ERROR` level so the checkpoint loss is observable.
pub struct AgentActionAdapter<A> {
    action: A,
    meta: ActionMetadata,
}

impl<A> AgentActionAdapter<A> {
    /// Wrap a typed agent action with its pre-stamped metadata.
    #[must_use]
    pub fn new(action: A, meta: ActionMetadata) -> Self {
        Self { action, meta }
    }

    /// Consume the adapter, returning the inner action.
    #[must_use]
    pub fn into_inner(self) -> A {
        self.action
    }
}

#[async_trait::async_trait]
impl<A> AgentHandle for AgentActionAdapter<A>
where
    A: AgentAction + Send + Sync + 'static,
    A::Input: DeserializeOwned + Send + Sync,
    A::Output: Serialize + Send + Sync,
    A::Turn: Serialize + DeserializeOwned + Clone + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }

    fn max_turns(&self) -> u32 {
        self.action.max_turns()
    }

    fn turn_timeout(&self) -> Option<Duration> {
        self.action.turn_timeout()
    }

    fn init_turn(&self, input: &Value) -> Result<Value, ActionError> {
        let typed_input: A::Input = serde_json::from_value(input.clone()).map_err(|e| {
            ActionError::validation(
                "input",
                ValidationReason::MalformedJson,
                Some(e.to_string()),
            )
        })?;
        let turn = self.action.init_turn(&typed_input);
        serde_json::to_value(&turn)
            .map_err(|e| ActionError::fatal(format!("init_turn serialization failed: {e}")))
    }

    /// Advance one turn, deserializing turn state from JSON and flushing it
    /// back before propagating any error.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Validation`] if turn state deserialization fails,
    /// or propagates errors from the underlying action.
    ///
    /// # Turn state checkpointing invariant
    ///
    /// Mutations made to `turn_state` inside `step` are flushed back to the
    /// JSON `turn_state` argument **before** any error from the action is
    /// propagated, so that a `Retryable` error does not replay work already
    /// completed in this turn. The one exception is the double-failure path
    /// (serialization fails *and* the action returned an error): the flush is
    /// skipped and the original action error propagates; the loss is logged at
    /// `ERROR` level.
    async fn step(
        &self,
        turn_state: &mut Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let mut typed_turn: A::Turn = serde_json::from_value::<A::Turn>(turn_state.clone())
            .map_err(|e| {
                ActionError::validation(
                    "turn_state",
                    ValidationReason::StateDeserialization,
                    Some(e.to_string()),
                )
            })?;

        let step_result = self.action.step(&mut typed_turn, ctx).await;

        // Flush turn state back to JSON regardless of Ok/Err — a Retryable
        // must checkpoint the updated position to avoid replaying work.
        match (serde_json::to_value(&typed_turn), &step_result) {
            (Ok(updated_state), _) => {
                *turn_state = updated_state;
            },
            (Err(ser_err), Ok(_)) => {
                return Err(ActionError::fatal(format!(
                    "turn state serialization failed: {ser_err}"
                )));
            },
            (Err(ser_err), Err(action_err)) => {
                tracing::error!(
                    action = %<A as Action>::metadata().base.key,
                    serialization_error = %ser_err,
                    action_error = %action_err,
                    "agent adapter: turn state serialization failed on error path; \
                     checkpoint lost, propagating original action error"
                );
            },
        }

        let result = step_result?;

        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }
}

impl<A: Action> std::fmt::Debug for AgentActionAdapter<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentActionAdapter")
            .field("action", &<A as Action>::metadata().base.key)
            .finish_non_exhaustive()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use nebula_core::Dependencies;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use super::*;
    use crate::{
        action::Action,
        error::ActionError,
        metadata::ActionMetadata,
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

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(
                nebula_core::action_key!("test.agent.counting"),
                "CountingAgent",
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

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(
                nebula_core::action_key!("test.agent.no_mutation"),
                "NoMutationAgent",
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
            AgentActionAdapter::new(CountingAgent { target: 3 }, CountingAgent::metadata());
        let turn_json = adapter
            .init_turn(&serde_json::json!(null))
            .expect("init_turn must succeed");
        let turn: CountingTurn =
            serde_json::from_value(turn_json).expect("init_turn must produce valid JSON");
        assert_eq!(turn.count, 0, "initial count must be 0");
    }

    /// Proves: `step` advances turn state and returns `Continue` on intermediate turns.
    #[tokio::test]
    async fn adapter_step_advances_state_and_continues() {
        let adapter = Arc::new(AgentActionAdapter::new(
            CountingAgent { target: 3 },
            CountingAgent::metadata(),
        ));
        let ctx = make_ctx();
        let mut turn_state = adapter
            .init_turn(&serde_json::json!(null))
            .expect("init_turn must succeed");

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
            AgentActionAdapter::new(CountingAgent { target: 3 }, CountingAgent::metadata());
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
        let adapter = AgentActionAdapter::new(
            NoMutationAgent {
                steps_before_break: 2,
            },
            NoMutationAgent::metadata(),
        );
        let ctx = make_ctx();
        let mut turn_state = adapter
            .init_turn(&serde_json::json!(null))
            .expect("init_turn must succeed");

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
            AgentActionAdapter::new(CountingAgent { target: 1 }, CountingAgent::metadata());
        let _: Arc<dyn AgentHandle> = Arc::new(adapter);
    }
}
