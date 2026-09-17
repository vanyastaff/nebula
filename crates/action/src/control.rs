//! [`ControlAction`] — DX family for synchronous flow-control nodes.
//!
//! Control actions make decisions on a single input and return a
//! [`ControlOutcome`] describing how execution should proceed: route to
//! a specific output port, drop an item, pass through, or terminate the
//! execution. Implementors do **not** hold state across calls, do not
//! wait on external signals, and do not iterate — those use cases belong
//! to [`StatefulAction`](crate::stateful::StatefulAction).
//!
//! The 7 canonical control nodes (`IfAction`, `SwitchAction`, `RouterAction`,
//! `FilterAction`, `NoOpAction`, `StopAction`, `FailAction`) are **not**
//! shipped in this crate — they live downstream in a reference-implementation
//! crate, and community plugin crates may add their own. `nebula-action`
//! owns only the trait contract, types, and adapter.
//!
//! # Adapter pattern
//!
//! `ControlAction` is erased to [`ControlHandle`] via
//! [`ControlActionAdapter`]. This mirrors the
//! [`PollTriggerAdapter`](crate::poll::PollTriggerAdapter) and
//! [`WebhookTriggerAdapter`](crate::webhook::WebhookTriggerAdapter) pattern
//! for `TriggerAction` DX families: author writes a typed trait, adapter
//! wraps and bridges to the dyn-compat handler contract. Registration:
//!
//! ```rust
//! # use std::sync::OnceLock;
//! # use nebula_action::{
//! #     Action, ActionContext, ActionError, ActionKind, ActionMetadata,
//! #     ControlAction, ControlOutcome, branch_key,
//! # };
//! # use nebula_core::{Dependencies, action_key};
//! use std::sync::Arc;
//! use nebula_action::{ControlActionAdapter, ControlHandle};
//! # struct MyIf;
//! # impl MyIf { fn new() -> Self { Self } }
//! # impl Action for MyIf {
//! #     type Input = bool;
//! #     type Output = bool;
//! #     fn metadata() -> nebula_action::ActionMetadataDraft {
//! #         nebula_action::ActionMetadataDraft::new(action_key!("control.if"), nebula_action::metadata_name!("If"), "Binary branch")
//! #     }
//! #     fn dependencies() -> &'static Dependencies {
//! #         static D: OnceLock<Dependencies> = OnceLock::new();
//! #         D.get_or_init(Dependencies::new)
//! #     }
//! # }
//! # impl ControlAction for MyIf {
//! #     async fn evaluate(
//! #         &self,
//! #         input: bool,
//! #         _ctx: &(impl ActionContext + ?Sized),
//! #     ) -> Result<ControlOutcome<bool>, ActionError> {
//! #         let selected = if input { branch_key!("true") } else { branch_key!("false") };
//! #         Ok(ControlOutcome::Branch { selected, output: input })
//! #     }
//! # }
//! let adapter = ControlActionAdapter::new(MyIf::new()).expect("valid metadata");
//! let handler: Arc<dyn ControlHandle> = Arc::new(adapter);
//! // The adapter stamps `ActionKind::Control`, so the registry can classify
//! // the erased handler without the author tagging the node by hand.
//! assert_eq!(handler.metadata().kind(), ActionKind::Control);
//! ```
//!
//! # Example: writing an `If` node
//!
//! ```rust
//! use std::sync::OnceLock;
//! use nebula_action::{
//!     Action, ActionContext, ActionError, ActionInput, ActionResult,
//!     ControlAction, ControlActionAdapter, ControlOutcome,
//!     ControlHandle, branch_key, port_key,
//!     port::{OutputPort, default_input_ports},
//! };
//! use nebula_action::testing::TestContextBuilder;
//! use nebula_core::{Dependencies, action_key};
//!
//! pub struct MyIf;
//!
//! impl Action for MyIf {
//!     type Input = bool;
//!     type Output = bool;
//!
//!     // The `ControlActionAdapter` stamps `ActionKind::Control` automatically;
//!     // authors do not classify the node by hand.
//!     fn metadata() -> nebula_action::ActionMetadataDraft {
//!         nebula_action::ActionMetadataDraft::new(action_key!("control.if"), nebula_action::metadata_name!("If"), "Binary branch")
//!             .with_inputs(default_input_ports())
//!             .with_outputs(vec![
//!                 OutputPort::flow(port_key!("true")),
//!                 OutputPort::flow(port_key!("false")),
//!             ])
//!     }
//!
//!     fn dependencies() -> &'static Dependencies {
//!         static D: OnceLock<Dependencies> = OnceLock::new();
//!         D.get_or_init(Dependencies::new)
//!     }
//! }
//!
//! impl ControlAction for MyIf {
//!     async fn evaluate(
//!         &self,
//!         input: bool,
//!         _ctx: &(impl ActionContext + ?Sized),
//!     ) -> Result<ControlOutcome<bool>, ActionError> {
//!         let selected = if input { branch_key!("true") } else { branch_key!("false") };
//!         Ok(ControlOutcome::Branch {
//!             selected,
//!             output: input,
//!         })
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     // Wrap the typed action and drive it through the erased handler path.
//!     let adapter = ControlActionAdapter::new(MyIf).expect("valid metadata");
//!     let ctx = TestContextBuilder::new().build();
//!     let input = adapter
//!         .prepare_input(ActionInput::Raw(serde_json::json!(true)))
//!         .unwrap();
//!     let result = adapter.dispatch(input, &ctx).await.unwrap();
//!     assert!(matches!(
//!         result,
//!         ActionResult::Branch { selected, .. } if selected.as_str() == "true"
//!     ));
//! }
//! ```

use std::{fmt, future::Future, sync::Arc};

use serde_json::Value;

use crate::{
    ActionInput,
    action::Action,
    branch_key::BranchKey,
    context::ActionContext,
    error::ActionError,
    handle::ControlHandle,
    input::{ActionInputContract, PreparedActionInput},
    metadata::{ActionKind, ActionMetadata},
    port_key::PortKey,
    result::{ActionResult, TerminationReason},
};

// ── ControlOutcome ──────────────────────────────────────────────────────────

/// The decision returned by a [`ControlAction::evaluate`] call.
///
/// Each variant corresponds to a flow-control semantic that cannot be
/// expressed safely through the broader [`ActionResult`] surface. The
/// adapter desugars each variant to the corresponding `ActionResult`
/// variant via [`From`] impl.
///
/// Marked `#[non_exhaustive]` — only this crate may add variants. New
/// variants preserve backward compatibility for author trait
/// implementations, but pattern matches on `ControlOutcome` in external
/// code must include a wildcard arm.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ControlOutcome<T> {
    /// Route the input to one selected output port.
    ///
    /// Used by `If` (2-way), `Switch` (N-way static), and `Router` in
    /// first-match mode. `selected` must match a port key declared in
    /// [`ActionMetadata::outputs`].
    Branch {
        /// Key of the chosen branch output port.
        selected: BranchKey,
        /// Value to emit on the selected port.
        output: T,
    },

    /// Route the input to multiple output ports in one call.
    ///
    /// Used by `Router` in all-match mode. Desugars to
    /// [`ActionResult::MultiOutput`]. Downstream join semantics follow
    /// the `all_success` rule documented on that variant.
    ///
    /// Carries a `HashMap` rather than a `Vec<(PortKey, Value)>` so that
    /// duplicate port keys are unrepresentable — an earlier `Vec` shape
    /// would silently overwrite on collision, which is a quiet footgun
    /// in routers that build up port lists dynamically.
    Route {
        /// Per-port outputs. Ports not present in this map are not
        /// emitted this cycle.
        ports: std::collections::HashMap<PortKey, T>,
    },

    /// Pass the input through unchanged to the single main output.
    ///
    /// Used by `NoOp` and `Filter` in the "match" case. Desugars to
    /// [`ActionResult::Success`].
    Pass {
        /// Value to emit on the main output port.
        output: T,
    },

    /// Drop this item without stopping the branch.
    ///
    /// Used by `Filter` in the "no-match" case. Desugars to
    /// [`ActionResult::Drop`]. Unlike [`ActionResult::Skip`], the
    /// broader execution continues; only this item is silently removed
    /// from the main output.
    Drop {
        /// Optional human-readable reason for dropping this item.
        reason: Option<String>,
    },

    /// Terminate the entire workflow execution.
    ///
    /// Used by `Stop` (success) and `Fail` (error). Desugars to
    /// [`ActionResult::Terminate`], which the engine recognises as a
    /// whole-execution terminal state, not a per-node skip.
    Terminate {
        /// Why the execution is ending.
        reason: TerminationReason,
    },
}

impl<T> From<ControlOutcome<T>> for ActionResult<T> {
    fn from(outcome: ControlOutcome<T>) -> Self {
        match outcome {
            ControlOutcome::Branch { selected, output } => ActionResult::Branch {
                selected,
                output: crate::output::ActionOutput::Value(output),
                alternatives: std::collections::HashMap::new(),
            },
            ControlOutcome::Route { ports } => {
                let outputs = ports
                    .into_iter()
                    .map(|(k, v)| (k, crate::output::ActionOutput::Value(v)))
                    .collect();
                ActionResult::MultiOutput {
                    outputs,
                    main_output: None,
                }
            },
            ControlOutcome::Pass { output } => ActionResult::Success {
                output: crate::output::ActionOutput::Value(output),
            },
            ControlOutcome::Drop { reason } => ActionResult::Drop { reason },
            ControlOutcome::Terminate { reason } => ActionResult::Terminate { reason },
        }
    }
}

// ── ControlAction trait ────────────────────────────────────────────────────

/// DX trait for flow-control nodes — synchronous decisions on a single
/// input.
///
/// Implementors make a decision based on the input and return a
/// [`ControlOutcome`] describing how execution should proceed. The
/// trait is **public and non-sealed** — community plugin crates may
/// implement it directly and register their own control primitives via
/// [`ControlActionAdapter`].
///
/// # When to implement this
///
/// - Node routes, filters, or terminates based on a synchronous decision over a single input.
/// - No **engine-persisted** state between calls (no `State` associated type, no checkpointing, no
///   serialization). In-memory `&self` state for local concerns like rate-limit counters, caches,
///   or metrics is fine — it just does not survive process restarts. If you need state that *does*
///   survive restarts, reach for [`StatefulAction`](crate::StatefulAction) instead.
/// - No waiting on external signals, no iteration.
///
/// # When NOT to implement this
///
/// - Needs cursor / counter between calls → [`StatefulAction`](crate::StatefulAction) (see DX
///   families `BatchAction`, `PaginatedAction`).
/// - Waits for time or external signal → [`StatefulAction`](crate::StatefulAction) with
///   `ActionResult::Wait` (or future `DelayAction` DX).
/// - Starts new executions from outside the graph →
///   [`TriggerAction`](crate::trigger::TriggerAction).
/// - Fan-outs to parallel branches → not an action at all; DAG topology concern.
/// - Waits for N upstream branches to complete → scheduler `trigger_rule`, not an action.
///
/// # Contract
///
/// `evaluate` must not block on external resources or persist state
/// between calls. It must not panic. The returned future must be
/// `Send` — the runtime runs it in `tokio::select!` against
/// cancellation.
pub trait ControlAction: Action {
    /// Evaluate the control decision for a single input.
    ///
    /// Returns a [`ControlOutcome`] on success, or [`ActionError`] if
    /// the input fails validation or an unrecoverable error occurs.
    ///
    /// The returned future must be `Send` because the runtime drives
    /// evaluation in `tokio::select!` against cancellation. Either of
    /// these forms is fine:
    ///
    /// ```rust
    /// # use std::sync::OnceLock;
    /// # use nebula_action::{
    /// #     Action, ActionContext, ActionError, ActionMetadata,
    /// #     ControlAction, ControlOutcome,
    /// # };
    /// # use nebula_core::{Dependencies, action_key};
    /// struct SugarPass;
    /// # impl Action for SugarPass {
    /// #     type Input = bool;
    /// #     type Output = bool;
    /// #     fn metadata() -> nebula_action::ActionMetadataDraft {
    /// #         nebula_action::ActionMetadataDraft::new(action_key!("control.pass"), nebula_action::metadata_name!("Pass"), "Pass through")
    /// #     }
    /// #     fn dependencies() -> &'static Dependencies {
    /// #         static D: OnceLock<Dependencies> = OnceLock::new();
    /// #         D.get_or_init(Dependencies::new)
    /// #     }
    /// # }
    /// impl ControlAction for SugarPass {
    ///     // Sugar form — `async fn` in trait impls is stable and
    ///     // desugars via RPITIT to the explicit return-type form below.
    ///     async fn evaluate(
    ///         &self,
    ///         input: bool,
    ///         _ctx: &(impl ActionContext + ?Sized),
    ///     ) -> Result<ControlOutcome<bool>, ActionError> {
    ///         Ok(ControlOutcome::Pass { output: input })
    ///     }
    /// }
    /// # #[tokio::main]
    /// # async fn main() {
    /// #     use nebula_action::testing::TestContextBuilder;
    /// #     let ctx = TestContextBuilder::new().build();
    /// #     let outcome = SugarPass
    /// #         .evaluate(true, &ctx)
    /// #         .await
    /// #         .unwrap();
    /// #     assert!(matches!(outcome, ControlOutcome::Pass { .. }));
    /// # }
    /// ```
    ///
    /// ```rust
    /// # use std::future::Future;
    /// # use std::sync::OnceLock;
    /// # use nebula_action::{
    /// #     Action, ActionContext, ActionError, ActionMetadata,
    /// #     ControlAction, ControlOutcome,
    /// # };
    /// # use nebula_core::{Dependencies, action_key};
    /// struct ExplicitPass;
    /// # impl Action for ExplicitPass {
    /// #     type Input = bool;
    /// #     type Output = bool;
    /// #     fn metadata() -> nebula_action::ActionMetadataDraft {
    /// #         nebula_action::ActionMetadataDraft::new(action_key!("control.pass"), nebula_action::metadata_name!("Pass"), "Pass through")
    /// #     }
    /// #     fn dependencies() -> &'static Dependencies {
    /// #         static D: OnceLock<Dependencies> = OnceLock::new();
    /// #         D.get_or_init(Dependencies::new)
    /// #     }
    /// # }
    /// impl ControlAction for ExplicitPass {
    ///     // Explicit form — use this if you want to spell out bounds
    ///     // or match the existing `StatelessAction::execute` convention.
    ///     fn evaluate(
    ///         &self,
    ///         input: bool,
    ///         _ctx: &(impl ActionContext + ?Sized),
    ///     ) -> impl Future<Output = Result<ControlOutcome<bool>, ActionError>> + Send {
    ///         async move { Ok(ControlOutcome::Pass { output: input }) }
    ///     }
    /// }
    /// # #[tokio::main]
    /// # async fn main() {
    /// #     use nebula_action::testing::TestContextBuilder;
    /// #     let ctx = TestContextBuilder::new().build();
    /// #     let outcome = ExplicitPass
    /// #         .evaluate(true, &ctx)
    /// #         .await
    /// #         .unwrap();
    /// #     assert!(matches!(outcome, ControlOutcome::Pass { .. }));
    /// # }
    /// ```
    ///
    /// Both compile to equivalent code. If your impl accidentally
    /// captures a non-`Send` value the compiler will flag it at the
    /// adapter instantiation site, which is the right place to notice.
    fn evaluate(
        &self,
        input: Self::Input,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ControlOutcome<Self::Output>, ActionError>> + Send;
}

// ── ControlActionAdapter ────────────────────────────────────────────────────

/// Wraps a [`ControlAction`] as a [`dyn ControlHandle`].
///
/// The adapter caches a copy of the action's [`ActionMetadata`] with the
/// [`ActionKind::Control`] node kind stamped automatically, so authors cannot
/// forget to classify a control node. Terminal control nodes (Stop, Fail)
/// declare an empty `outputs` set and keep [`ActionKind::Control`] —
/// terminality is carried structurally by the empty port set, not by a
/// distinct kind, so the workflow validator detects the graph sink from the
/// ports.
///
/// # Example
///
/// ```rust
/// # use std::sync::OnceLock;
/// # use nebula_action::{
/// #     Action, ActionContext, ActionError, ActionKind, ActionMetadata,
/// #     ControlAction, ControlOutcome,
/// # };
/// # use nebula_core::{Dependencies, action_key};
/// use std::sync::Arc;
/// use nebula_action::{ControlActionAdapter, ControlHandle};
/// # struct MyIf;
/// # impl MyIf { fn new() -> Self { Self } }
/// # impl Action for MyIf {
/// #     type Input = bool;
/// #     type Output = bool;
/// #     fn metadata() -> nebula_action::ActionMetadataDraft {
/// #         nebula_action::ActionMetadataDraft::new(action_key!("control.if"), nebula_action::metadata_name!("If"), "Binary branch")
/// #     }
/// #     fn dependencies() -> &'static Dependencies {
/// #         static D: OnceLock<Dependencies> = OnceLock::new();
/// #         D.get_or_init(Dependencies::new)
/// #     }
/// # }
/// # impl ControlAction for MyIf {
/// #     async fn evaluate(
/// #         &self,
/// #         input: bool,
/// #         _ctx: &(impl ActionContext + ?Sized),
/// #     ) -> Result<ControlOutcome<bool>, ActionError> {
/// #         Ok(ControlOutcome::Pass { output: input })
/// #     }
/// # }
/// let adapter = ControlActionAdapter::new(MyIf::new()).expect("valid metadata");
/// let handler: Arc<dyn ControlHandle> = Arc::new(adapter);
/// // The kind is stamped at construction, regardless of the inner action.
/// assert_eq!(handler.metadata().kind(), ActionKind::Control);
/// ```
pub struct ControlActionAdapter<A: ControlAction> {
    action: A,
    cached_metadata: Arc<ActionMetadata>,
    input_contract: ActionInputContract,
}

impl<A: ControlAction> crate::handle::sealed::Control for ControlActionAdapter<A> {}

impl<A: ControlAction> ControlActionAdapter<A> {
    /// Wrap a typed control action.
    ///
    /// The adapter takes the action's metadata, stamps the
    /// [`ActionKind::Control`] node kind, and caches the result in an `Arc` so
    /// subsequent `metadata()` calls are cheap.
    ///
    /// A terminal control node (empty `outputs`) keeps
    /// [`ActionKind::Control`] — terminality is carried by the empty port set,
    /// not by a distinct kind.
    ///
    /// # Errors
    /// Returns a typed catalog error if metadata or an associated schema is invalid.
    #[tracing::instrument(name = "action.metadata.admit", skip_all, err)]
    pub fn new(action: A) -> Result<Self, crate::ActionMetadataAdmissionError> {
        let meta = <A as Action>::metadata().admit_for::<A>(ActionKind::Control)?;
        let cached_metadata = Arc::new(meta);
        let input_contract = ActionInputContract::new(cached_metadata.base().schema());
        Ok(Self {
            action,
            cached_metadata,
            input_contract,
        })
    }

    /// Consume the adapter, returning the inner action.
    #[must_use]
    pub fn into_inner(self) -> A {
        self.action
    }
}

#[async_trait::async_trait]
impl<A> ControlHandle for ControlActionAdapter<A>
where
    A: ControlAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.cached_metadata
    }

    fn prepare_input(&self, input: ActionInput) -> Result<PreparedActionInput, ActionError> {
        self.input_contract.prepare::<A::Input>(input)
    }

    async fn dispatch(
        &self,
        input: PreparedActionInput,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let input = input.into_typed::<A::Input>(&self.input_contract)?;
        let outcome: ActionResult<A::Output> = self.action.evaluate(input, ctx).await?.into();
        outcome.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|_| ActionError::fatal("control output cannot be serialized as declared"))
        })
    }
}

impl<A: ControlAction> fmt::Debug for ControlActionAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlActionAdapter")
            .field("action", self.cached_metadata.base().key())
            .field("kind", &self.cached_metadata.kind())
            .finish_non_exhaustive()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
