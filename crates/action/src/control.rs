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
//! `ControlAction` is erased to [`StatelessHandler`] via
//! [`ControlActionAdapter`]. This mirrors the
//! [`PollTriggerAdapter`](crate::poll::PollTriggerAdapter) and
//! [`WebhookTriggerAdapter`](crate::webhook::WebhookTriggerAdapter) pattern
//! for `TriggerAction` DX families: author writes a typed trait, adapter
//! wraps and bridges to the dyn-compat handler contract. Registration:
//!
//! ```rust,ignore
//! use nebula_action::{ControlAction, ControlActionAdapter, StatelessHandler};
//! use std::sync::Arc;
//!
//! let adapter = ControlActionAdapter::new(MyIf::new());
//! let handler: Arc<dyn StatelessHandler> = Arc::new(adapter);
//! registry.register(handler);
//! ```
//!
//! # Example: writing an `If` node
//!
//! ```rust,ignore
//! use nebula_action::{
//!     Action, ActionCategory, DeclaresDependencies, ActionError,
//!     ActionMetadata, ControlAction, ControlInput, ControlOutcome,
//! };
//! use nebula_core::action_key;
//!
//! pub struct MyIf {
//!     metadata: ActionMetadata,
//! }
//!
//! impl MyIf {
//!     pub fn new() -> Self {
//!         Self {
//!             metadata: ActionMetadata::new(action_key!("control.if"), "If", "Binary branch")
//!                 .with_category(ActionCategory::Control),
//!         }
//!     }
//! }
//!
//! impl DeclaresDependencies for MyIf {}
//! impl Action for MyIf {
//!     fn metadata(&self) -> &ActionMetadata { &self.metadata }
//! }
//!
//! impl ControlAction for MyIf {
//!     async fn evaluate(
//!         &self,
//!         input: ControlInput,
//!         _ctx: &(impl nebula_action::ActionContext + ?Sized),
//!     ) -> Result<ControlOutcome, ActionError> {
//!         let condition = input.get_bool("/condition")?;
//!         let selected = if condition { "true" } else { "false" };
//!         Ok(ControlOutcome::Branch {
//!             selected: selected.into(),
//!             output: input.into_value(),
//!         })
//!     }
//! }
//! ```

use std::{fmt, future::Future, pin::Pin, sync::Arc};

use serde_json::Value;

use crate::{
    action::Action,
    context::ActionContext,
    error::{ActionError, ValidationReason},
    metadata::{ActionCategory, ActionMetadata},
    port::PortKey,
    result::{ActionResult, TerminationReason},
    stateless::StatelessHandler,
};

// ── ControlInput ────────────────────────────────────────────────────────────

/// Owned wrapper around the JSON input passed to a [`ControlAction`].
///
/// Provides convenient typed accessors so control authors don't reinvent
/// the same `serde_json::Value::pointer(...).and_then(...)` incantation
/// for every If/Switch/Filter node.
///
/// The wrapper is **owned**, not borrowed. This is deliberate — the
/// [`StatelessHandler::execute`] path returns a `Send + 'static` future,
/// which cannot carry a borrow tied to the input. A borrowed wrapper
/// would force every author to `async move` into owned copies anyway,
/// so owning the value up front is both simpler and zero-cost.
///
/// Wrapper is `#[non_exhaustive]`; future versions may add fields
/// (pre-parsed JSON pointer cache, metrics hooks) without breaking
/// external implementors.
#[derive(Debug, Clone)]
pub struct ControlInput {
    value: Value,
}

impl ControlInput {
    /// Wrap a raw JSON value.
    #[must_use]
    pub fn from_value(value: Value) -> Self {
        Self { value }
    }

    /// Borrow the underlying JSON value without consuming the wrapper.
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    /// Consume the wrapper and return the underlying value.
    ///
    /// Used in passthrough cases — e.g.
    /// `ControlOutcome::Pass { output: input.into_value() }`.
    #[must_use]
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Look up an arbitrary sub-value at a JSON pointer.
    ///
    /// Thin wrapper around [`Value::pointer`]; returns `None` if the
    /// path is missing or empty.
    #[must_use]
    pub fn get(&self, pointer: &str) -> Option<&Value> {
        self.value.pointer(pointer)
    }

    /// Read a boolean at a JSON pointer.
    ///
    /// Returns [`ActionError::Validation`] with
    /// [`ValidationReason::MissingField`] if the field is absent, or
    /// [`ValidationReason::WrongType`] if the field exists but is not
    /// a boolean.
    pub fn get_bool(&self, pointer: &str) -> Result<bool, ActionError> {
        let v = self.require(pointer)?;
        v.as_bool().ok_or_else(|| {
            ActionError::validation(
                "control_input",
                ValidationReason::WrongType,
                Some(format!(
                    "expected boolean at `{pointer}`, got {}",
                    value_kind(v)
                )),
            )
        })
    }

    /// Read a string slice at a JSON pointer.
    ///
    /// Returns the same validation errors as [`get_bool`](Self::get_bool)
    /// for missing / wrong-type fields.
    pub fn get_str(&self, pointer: &str) -> Result<&str, ActionError> {
        let v = self.require(pointer)?;
        v.as_str().ok_or_else(|| {
            ActionError::validation(
                "control_input",
                ValidationReason::WrongType,
                Some(format!(
                    "expected string at `{pointer}`, got {}",
                    value_kind(v)
                )),
            )
        })
    }

    /// Read a signed 64-bit integer at a JSON pointer.
    ///
    /// Accepts any JSON number that fits in `i64`. Returns validation
    /// errors for missing fields, non-numeric values, and numbers that
    /// don't fit in `i64`.
    pub fn get_i64(&self, pointer: &str) -> Result<i64, ActionError> {
        let v = self.require(pointer)?;
        v.as_i64().ok_or_else(|| {
            ActionError::validation(
                "control_input",
                ValidationReason::WrongType,
                Some(format!(
                    "expected i64 at `{pointer}`, got {}",
                    value_kind(v)
                )),
            )
        })
    }

    /// Read an `f64` at a JSON pointer.
    ///
    /// Accepts any JSON number. Returns validation errors for missing
    /// fields or non-numeric values.
    pub fn get_f64(&self, pointer: &str) -> Result<f64, ActionError> {
        let v = self.require(pointer)?;
        v.as_f64().ok_or_else(|| {
            ActionError::validation(
                "control_input",
                ValidationReason::WrongType,
                Some(format!(
                    "expected f64 at `{pointer}`, got {}",
                    value_kind(v)
                )),
            )
        })
    }

    fn require(&self, pointer: &str) -> Result<&Value, ActionError> {
        self.value.pointer(pointer).ok_or_else(|| {
            ActionError::validation(
                "control_input",
                ValidationReason::MissingField,
                Some(format!("field `{pointer}` is required")),
            )
        })
    }
}

/// Classify a JSON value by its type for use in validation error messages.
///
/// Returns a static string describing the JSON shape (`"null"`, `"bool"`,
/// `"number"`, `"string"`, `"array"`, `"object"`). Deliberately does NOT
/// include the actual value — a control node's input may carry secrets
/// (API keys, passwords, PII) and the `ActionError::Validation.detail`
/// field flows into logs.
fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

impl From<Value> for ControlInput {
    fn from(value: Value) -> Self {
        Self::from_value(value)
    }
}

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
pub enum ControlOutcome {
    /// Route the input to one selected output port.
    ///
    /// Used by `If` (2-way), `Switch` (N-way static), and `Router` in
    /// first-match mode. `selected` must match a port key declared in
    /// [`ActionMetadata::outputs`].
    Branch {
        /// Key of the chosen output port.
        selected: PortKey,
        /// Value to emit on the selected port.
        output: Value,
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
        ports: std::collections::HashMap<PortKey, Value>,
    },

    /// Pass the input through unchanged to the single main output.
    ///
    /// Used by `NoOp` and `Filter` in the "match" case. Desugars to
    /// [`ActionResult::Success`].
    Pass {
        /// Value to emit on the main output port.
        output: Value,
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

impl From<ControlOutcome> for ActionResult<Value> {
    fn from(outcome: ControlOutcome) -> Self {
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
    /// ```ignore
    /// // Sugar form — `async fn` in trait impls is stable and
    /// // desugars via RPITIT to the explicit return-type form below.
    /// async fn evaluate(
    ///     &self,
    ///     input: ControlInput,
    ///     ctx: &(impl ActionContext + ?Sized),
    /// ) -> Result<ControlOutcome, ActionError> { /* ... */ }
    /// ```
    ///
    /// ```ignore
    /// // Explicit form — use this if you want to spell out bounds
    /// // or match the existing StatelessAction::execute convention.
    /// fn evaluate(
    ///     &self,
    ///     input: ControlInput,
    ///     ctx: &(impl ActionContext + ?Sized),
    /// ) -> impl Future<Output = Result<ControlOutcome, ActionError>> + Send { /* ... */ }
    /// ```
    ///
    /// Both compile to equivalent code. If your impl accidentally
    /// captures a non-`Send` value the compiler will flag it at the
    /// adapter instantiation site, which is the right place to notice.
    fn evaluate(
        &self,
        input: ControlInput,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ControlOutcome, ActionError>> + Send;
}

// ── ControlActionAdapter ────────────────────────────────────────────────────

/// Wraps a [`ControlAction`] as a [`dyn StatelessHandler`].
///
/// The adapter caches a copy of the action's [`ActionMetadata`] with
/// the [`ActionCategory`] field stamped automatically based on whether
/// the action declares output ports:
///
/// - Zero outputs → [`ActionCategory::Terminal`] (Stop, Fail)
/// - One or more outputs → [`ActionCategory::Control`]
///
/// Authors cannot forget to set the category; the adapter does it for
/// them at registration time.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::{ControlActionAdapter, StatelessHandler};
/// use std::sync::Arc;
///
/// let adapter = ControlActionAdapter::new(MyIf::new());
/// let handler: Arc<dyn StatelessHandler> = Arc::new(adapter);
/// ```
pub struct ControlActionAdapter<A: ControlAction> {
    action: A,
    cached_metadata: Arc<ActionMetadata>,
}

impl<A: ControlAction> ControlActionAdapter<A> {
    /// Wrap a typed control action.
    ///
    /// The adapter clones the action's metadata, stamps the appropriate
    /// [`ActionCategory`] (Control or Terminal), and caches the result
    /// in an `Arc` so subsequent `metadata()` calls are cheap.
    #[must_use]
    pub fn new(action: A) -> Self {
        let mut meta = action.metadata().clone();
        meta.category = derive_category(&meta);
        Self {
            action,
            cached_metadata: Arc::new(meta),
        }
    }

    /// Consume the adapter, returning the inner action.
    #[must_use]
    pub fn into_inner(self) -> A {
        self.action
    }
}

impl<A> StatelessHandler for ControlActionAdapter<A>
where
    A: ControlAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.cached_metadata
    }

    fn execute<'life0, 'life1, 'a>(
        &'life0 self,
        input: Value,
        ctx: &'life1 dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionResult<Value>, ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
    {
        Box::pin(async move {
            let outcome = self
                .action
                .evaluate(ControlInput::from_value(input), ctx)
                .await?;
            Ok(outcome.into())
        })
    }
}

impl<A: ControlAction> fmt::Debug for ControlActionAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlActionAdapter")
            .field("action", &self.cached_metadata.base.key)
            .field("category", &self.cached_metadata.category)
            .finish_non_exhaustive()
    }
}

/// Infer the `ActionCategory` for a control action based on its declared
/// output ports.
///
/// - Zero outputs → `Terminal` (Stop, Fail)
/// - One or more outputs → `Control`
///
/// Called automatically by [`ControlActionAdapter::new`]. Exposed for
/// testing only — authors should not override this.
fn derive_category(meta: &ActionMetadata) -> ActionCategory {
    if meta.outputs.is_empty() {
        ActionCategory::Terminal
    } else {
        ActionCategory::Control
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use nebula_core::{DeclaresDependencies, action_key};

    use super::*;
    use crate::{
        port::{OutputPort, default_input_ports, default_output_ports},
        testing::{TestActionContext, TestContextBuilder},
    };

    fn make_ctx() -> TestActionContext {
        TestContextBuilder::new().build()
    }

    // ── ControlInput ────────────────────────────────────────────────

    #[test]
    fn control_input_get_bool_ok() {
        let input = ControlInput::from_value(serde_json::json!({ "flag": true }));
        assert!(input.get_bool("/flag").unwrap());
    }

    #[test]
    fn control_input_get_bool_missing() {
        let input = ControlInput::from_value(serde_json::json!({}));
        let err = input.get_bool("/flag").unwrap_err();
        match err {
            ActionError::Validation { reason, .. } => {
                assert!(matches!(reason, ValidationReason::MissingField));
            },
            _ => panic!("expected Validation"),
        }
    }

    #[test]
    fn control_input_get_bool_wrong_type() {
        let input = ControlInput::from_value(serde_json::json!({ "flag": "yes" }));
        let err = input.get_bool("/flag").unwrap_err();
        match err {
            ActionError::Validation { reason, .. } => {
                assert!(matches!(reason, ValidationReason::WrongType));
            },
            _ => panic!("expected Validation"),
        }
    }

    #[test]
    fn control_input_get_str_ok() {
        let input = ControlInput::from_value(serde_json::json!({ "name": "alice" }));
        assert_eq!(input.get_str("/name").unwrap(), "alice");
    }

    #[test]
    fn control_input_get_i64_ok() {
        let input = ControlInput::from_value(serde_json::json!({ "n": 42 }));
        assert_eq!(input.get_i64("/n").unwrap(), 42);
    }

    #[test]
    fn control_input_get_f64_ok() {
        let input = ControlInput::from_value(serde_json::json!({ "x": 2.5 }));
        assert!((input.get_f64("/x").unwrap() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn control_input_into_value_passthrough() {
        let original = serde_json::json!({ "foo": "bar" });
        let input = ControlInput::from_value(original.clone());
        assert_eq!(input.into_value(), original);
    }

    #[test]
    fn control_input_from_value_impl() {
        let v: Value = serde_json::json!(42);
        let input: ControlInput = v.into();
        assert_eq!(input.as_value(), &serde_json::json!(42));
    }

    // ── ControlOutcome → ActionResult ──────────────────────────────

    #[test]
    fn outcome_branch_desugars_to_action_result_branch() {
        let outcome = ControlOutcome::Branch {
            selected: "true".into(),
            output: serde_json::json!({"v": 1}),
        };
        let result: ActionResult<Value> = outcome.into();
        match result {
            ActionResult::Branch {
                selected,
                output,
                alternatives,
            } => {
                assert_eq!(selected, "true");
                assert_eq!(output.as_value(), Some(&serde_json::json!({"v": 1})));
                assert!(alternatives.is_empty());
            },
            _ => panic!("expected Branch"),
        }
    }

    #[test]
    fn outcome_route_desugars_to_multi_output() {
        let outcome = ControlOutcome::Route {
            ports: std::collections::HashMap::from([
                ("high".into(), serde_json::json!(1)),
                ("low".into(), serde_json::json!(2)),
            ]),
        };
        let result: ActionResult<Value> = outcome.into();
        match result {
            ActionResult::MultiOutput {
                outputs,
                main_output,
            } => {
                assert_eq!(outputs.len(), 2);
                assert!(outputs.contains_key("high"));
                assert!(outputs.contains_key("low"));
                assert!(main_output.is_none());
            },
            _ => panic!("expected MultiOutput"),
        }
    }

    #[test]
    fn outcome_pass_desugars_to_success() {
        let outcome = ControlOutcome::Pass {
            output: serde_json::json!({"ok": true}),
        };
        let result: ActionResult<Value> = outcome.into();
        match result {
            ActionResult::Success { output } => {
                assert_eq!(output.as_value(), Some(&serde_json::json!({"ok": true})));
            },
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn outcome_drop_desugars_to_drop() {
        let outcome = ControlOutcome::Drop {
            reason: Some("rate limit".into()),
        };
        let result: ActionResult<Value> = outcome.into();
        match result {
            ActionResult::Drop { reason } => {
                assert_eq!(reason.as_deref(), Some("rate limit"));
            },
            _ => panic!("expected Drop"),
        }
    }

    #[test]
    fn outcome_terminate_success_desugars_to_terminate() {
        let outcome = ControlOutcome::Terminate {
            reason: TerminationReason::Success {
                note: Some("done".into()),
            },
        };
        let result: ActionResult<Value> = outcome.into();
        match result {
            ActionResult::Terminate { reason } => match reason {
                TerminationReason::Success { note } => assert_eq!(note.as_deref(), Some("done")),
                TerminationReason::Failure { .. } => panic!("expected Success"),
            },
            _ => panic!("expected Terminate"),
        }
    }

    #[test]
    fn outcome_terminate_failure_desugars_to_terminate() {
        let outcome = ControlOutcome::Terminate {
            reason: TerminationReason::Failure {
                code: "E_BAD".into(),
                message: "nope".into(),
            },
        };
        let result: ActionResult<Value> = outcome.into();
        match result {
            ActionResult::Terminate { reason } => match reason {
                TerminationReason::Failure { code, message } => {
                    assert_eq!(code.as_str(), "E_BAD");
                    assert_eq!(message, "nope");
                },
                TerminationReason::Success { .. } => panic!("expected Failure"),
            },
            _ => panic!("expected Terminate"),
        }
    }

    // ── derive_category ────────────────────────────────────────────

    #[test]
    fn derive_category_control_for_nodes_with_outputs() {
        let meta = ActionMetadata::new(action_key!("test.if"), "If", "Binary branch");
        // ActionMetadata::new uses default_output_ports() which has one main output.
        assert!(!meta.outputs.is_empty());
        assert_eq!(derive_category(&meta), ActionCategory::Control);
    }

    #[test]
    fn derive_category_terminal_for_zero_output_nodes() {
        let meta = ActionMetadata::new(action_key!("test.stop"), "Stop", "Terminate")
            .with_outputs(Vec::new());
        assert_eq!(derive_category(&meta), ActionCategory::Terminal);
    }

    // ── ControlActionAdapter smoke test ────────────────────────────

    /// Minimal control action used for smoke tests.
    struct TestIf {
        metadata: ActionMetadata,
    }

    impl TestIf {
        fn new() -> Self {
            Self {
                metadata: ActionMetadata::new(action_key!("test.if"), "TestIf", "Binary branch")
                    .with_inputs(default_input_ports())
                    .with_outputs(vec![OutputPort::flow("true"), OutputPort::flow("false")]),
            }
        }
    }

    impl DeclaresDependencies for TestIf {}

    impl Action for TestIf {
        fn metadata(&self) -> &ActionMetadata {
            &self.metadata
        }
    }

    impl ControlAction for TestIf {
        async fn evaluate(
            &self,
            input: ControlInput,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ControlOutcome, ActionError> {
            let condition = input.get_bool("/condition")?;
            let selected = if condition { "true" } else { "false" };
            Ok(ControlOutcome::Branch {
                selected: selected.into(),
                output: input.into_value(),
            })
        }
    }

    /// Terminal-only action for category-inference smoke tests.
    struct TestStop {
        metadata: ActionMetadata,
    }

    impl TestStop {
        fn new() -> Self {
            Self {
                metadata: ActionMetadata::new(action_key!("test.stop"), "TestStop", "Terminate")
                    .with_outputs(Vec::new()),
            }
        }
    }

    impl DeclaresDependencies for TestStop {}

    impl Action for TestStop {
        fn metadata(&self) -> &ActionMetadata {
            &self.metadata
        }
    }

    impl ControlAction for TestStop {
        async fn evaluate(
            &self,
            _input: ControlInput,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ControlOutcome, ActionError> {
            Ok(ControlOutcome::Terminate {
                reason: TerminationReason::Success {
                    note: Some("stopped".into()),
                },
            })
        }
    }

    #[test]
    fn adapter_stamps_control_category() {
        let adapter = ControlActionAdapter::new(TestIf::new());
        assert_eq!(adapter.metadata().category, ActionCategory::Control);
    }

    #[test]
    fn adapter_stamps_terminal_category_for_zero_output_action() {
        let adapter = ControlActionAdapter::new(TestStop::new());
        assert_eq!(adapter.metadata().category, ActionCategory::Terminal);
    }

    #[test]
    fn adapter_preserves_action_key() {
        let adapter = ControlActionAdapter::new(TestIf::new());
        assert_eq!(adapter.metadata().base.key, action_key!("test.if"));
    }

    #[tokio::test]
    async fn adapter_executes_through_stateless_handler() {
        let adapter = ControlActionAdapter::new(TestIf::new());
        let ctx = make_ctx();

        let result = StatelessHandler::execute(
            &adapter,
            serde_json::json!({ "condition": true, "payload": 42 }),
            &ctx,
        )
        .await
        .unwrap();

        match result {
            ActionResult::Branch {
                selected, output, ..
            } => {
                assert_eq!(selected, "true");
                assert_eq!(
                    output.as_value(),
                    Some(&serde_json::json!({ "condition": true, "payload": 42 }))
                );
            },
            _ => panic!("expected Branch"),
        }
    }

    #[tokio::test]
    async fn adapter_evaluates_false_branch() {
        let adapter = ControlActionAdapter::new(TestIf::new());
        let ctx = make_ctx();

        let result =
            StatelessHandler::execute(&adapter, serde_json::json!({ "condition": false }), &ctx)
                .await
                .unwrap();

        match result {
            ActionResult::Branch { selected, .. } => assert_eq!(selected, "false"),
            _ => panic!("expected Branch"),
        }
    }

    #[tokio::test]
    async fn adapter_propagates_validation_error_on_missing_field() {
        let adapter = ControlActionAdapter::new(TestIf::new());
        let ctx = make_ctx();

        let err = StatelessHandler::execute(&adapter, serde_json::json!({}), &ctx)
            .await
            .unwrap_err();

        assert!(matches!(err, ActionError::Validation { .. }));
    }

    #[tokio::test]
    async fn adapter_stop_action_returns_terminate() {
        let adapter = ControlActionAdapter::new(TestStop::new());
        let ctx = make_ctx();

        let result = StatelessHandler::execute(&adapter, serde_json::json!({}), &ctx)
            .await
            .unwrap();

        match result {
            ActionResult::Terminate { reason } => match reason {
                TerminationReason::Success { note } => {
                    assert_eq!(note.as_deref(), Some("stopped"));
                },
                TerminationReason::Failure { .. } => panic!("expected Success"),
            },
            _ => panic!("expected Terminate"),
        }
    }

    #[test]
    fn adapter_is_dyn_compatible() {
        let adapter = ControlActionAdapter::new(TestIf::new());
        let _: Arc<dyn StatelessHandler> = Arc::new(adapter);
    }

    #[test]
    fn adapter_into_inner_returns_action() {
        let adapter = ControlActionAdapter::new(TestIf::new());
        let action = adapter.into_inner();
        assert_eq!(action.metadata().base.key, action_key!("test.if"));
    }

    #[test]
    fn adapter_preserves_original_outputs_after_stamp() {
        // The adapter only rewrites `category`; it must not touch `outputs`
        // or any other metadata field.
        let original_outputs = TestIf::new().metadata.outputs;
        let adapter = ControlActionAdapter::new(TestIf::new());
        assert_eq!(adapter.metadata().outputs, original_outputs);
    }

    // ── default_output_ports parity ────────────────────────────────

    #[test]
    fn derive_category_default_output_ports_is_control() {
        // Actions constructed with default ports (one main output) must
        // land in Control, never Terminal.
        assert!(!default_output_ports().is_empty());
    }
}
