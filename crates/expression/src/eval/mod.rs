//! AST evaluation module
//!
//! This module implements the evaluation of parsed expression ASTs.
//!
//! Evaluation works on [`RuntimeValue`], not on `serde_json::Value`:
//! typed values such as date-times survive property/index chains and
//! builtin dispatch. Plain JSON appears only at the crate boundary
//! (`ExpressionEngine::evaluate` and friends call `RuntimeValue::to_json`).

use std::{borrow::Cow, cell::Cell, sync::Arc};

#[cfg(feature = "regex")]
use regex::Regex;

use crate::{
    ExpressionError,
    ast::{BinaryOp, Expr},
    builtins::BuiltinRegistry,
    context::EvaluationContext,
    error::ExpressionResult,
    limits::MAX_AST_DEPTH,
    policy::{EvaluationPolicy, MissingLookup},
    value::RuntimeValue,
};

/// Maximum length for regex patterns to prevent ReDoS attacks
#[cfg(feature = "regex")]
const MAX_REGEX_PATTERN_LEN: usize = 1000;

/// Maximum number of cached regex patterns (simple LRU-style eviction)
#[cfg(feature = "regex")]
const MAX_REGEX_CACHE_SIZE: usize = 100;

/// A value borrowed from the AST or context, or owned by evaluation.
pub(crate) type EvalValue<'a> = Cow<'a, RuntimeValue>;

/// One evaluated function argument.
///
/// Lambdas stay unevaluated: a registered builtin receives the expression and
/// decides when (and how many times) to invoke it through
/// [`BuiltinView::invoke_lambda`]. Every other argument is a value.
pub enum Argument<'a> {
    /// An evaluated value.
    Value(EvalValue<'a>),
    /// An unevaluated lambda expression (`Expr::Lambda`).
    Lambda(&'a Expr),
}

impl Argument<'_> {
    /// Borrow the value, if this argument is one.
    pub fn as_value(&self) -> Option<&RuntimeValue> {
        match self {
            Self::Value(value) => Some(value.as_ref()),
            Self::Lambda(_) => None,
        }
    }

    /// Borrow the lambda expression, if this argument is one.
    pub fn as_lambda(&self) -> Option<&Expr> {
        match self {
            Self::Value(_) => None,
            Self::Lambda(lambda) => Some(lambda),
        }
    }
}

/// Per-call evaluation frame that tracks recursion depth and the DoS
/// step budget for a single top-level [`Evaluator::eval`] invocation.
///
/// Lives on the caller's stack (never on `Evaluator`, never on
/// [`EvaluationContext`]). Every recursive path inside the evaluator
/// threads `&EvalFrame` instead of a bare `depth: usize`, so:
///
/// - Concurrent `Arc<Evaluator>` users each get their own frame with zero synchronization — no
///   shared atomics, no thread-local state.
/// - Nested lambda evaluation cannot accidentally reset the counter (the old `self.eval(...)`
///   re-entry pattern that did `self.steps.store(0)` at the top of every call is gone).
/// - One top-level `eval` call = one step budget, regardless of how many lambdas / reduces /
///   pipelines it recurses through.
///
/// The frame is shared by reference, not by `&mut`: a registered builtin holds
/// a [`BuiltinView`] over the same frame when it invokes a lambda argument, so
/// the step budget and depth still accumulate across every invocation. What a
/// builtin must not be able to do — construct a fresh frame — remains
/// type-impossible.
///
/// Closes CO-C1-01 (issue #252): `max_eval_steps` bypass via lambdas.
pub(crate) struct EvalFrame {
    depth: Cell<usize>,
    steps: Cell<usize>,
    max_steps: Option<usize>,
}

impl EvalFrame {
    /// Create a fresh frame with the given step cap (snapshotted once
    /// from the effective policy at the top-level `eval` entry).
    #[inline]
    fn new(max_steps: Option<usize>) -> Self {
        Self {
            depth: Cell::new(0),
            steps: Cell::new(0),
            max_steps,
        }
    }

    /// Count one AST-node evaluation against the step budget.
    ///
    /// Called from the top of [`Evaluator::eval_with_frame`] exactly
    /// once per AST node. Returns an error the moment the cap is
    /// exceeded, so a hostile `map(range, x => expensive)` traversal
    /// aborts deterministically instead of running to completion.
    #[inline]
    fn tick(&self) -> ExpressionResult<()> {
        self.charge(1)
    }

    pub(crate) fn charge(&self, steps: usize) -> ExpressionResult<()> {
        let actual = self.steps.get().saturating_add(steps);
        self.steps.set(actual);
        if let Some(max) = self.max_steps
            && actual > max
        {
            // Emit a structured warning so dashboards can spot DoS attempts
            // before the typed error has reached the user. `actual` is the
            // step that tripped the budget, not the budget itself.
            tracing::warn!(
                target: "nebula_expression::dos",
                limit = max,
                actual,
                "step budget exceeded"
            );
            return Err(ExpressionError::step_limit_exceeded(max, actual));
        }
        Ok(())
    }

    /// Charge a value's shape (string bytes and container entries) plus one
    /// step for materializing it. Recurses iteratively, never on the stack.
    fn charge_value(&self, value: &RuntimeValue) -> ExpressionResult<()> {
        crate::limits::check_value_limits(value)?;
        self.tick()?;
        let mut pending = vec![(value, 1)];
        while let Some((value, depth)) = pending.pop() {
            crate::limits::check_limit("value depth", depth, MAX_AST_DEPTH)?;
            match value {
                RuntimeValue::String(text) => self.charge(text.len())?,
                RuntimeValue::DateTime(_) => self.charge(35)?,
                RuntimeValue::Array(values) => {
                    self.charge(values.len())?;
                    pending.extend(values.iter().map(|value| (value, depth + 1)));
                },
                RuntimeValue::Object(values) => {
                    self.charge(values.len())?;
                    for (key, value) in values.iter() {
                        self.charge(key.len())?;
                        pending.push((value, depth + 1));
                    }
                },
                _ => {},
            }
        }
        Ok(())
    }

    /// Enter a deeper recursion level.
    ///
    /// Each recursive `eval_with_frame` call increments `depth`; the
    /// matching decrement happens after the dispatch returns via the
    /// symmetric `leave` call, wired unconditionally (both success and
    /// error paths) from `eval_with_frame`. Frames are per-call and
    /// stack-local, so depth cannot leak across top-level `eval` calls
    /// even if a recursive path bails mid-traversal.
    #[inline]
    fn enter(&self) -> ExpressionResult<()> {
        let depth = self.depth.get();
        if depth >= MAX_AST_DEPTH {
            tracing::warn!(
                target: "nebula_expression::dos",
                limit = MAX_AST_DEPTH,
                actual = depth,
                "recursion depth exceeded"
            );
            return Err(ExpressionError::depth_exceeded(MAX_AST_DEPTH, depth));
        }
        self.depth.set(depth + 1);
        Ok(())
    }

    /// Leave a recursion level previously entered via [`enter`].
    #[inline]
    fn leave(&self) {
        let depth = self.depth.get();
        debug_assert!(depth > 0, "leave called without matching enter");
        self.depth.set(depth.saturating_sub(1));
    }
}

/// Read-only handle on the evaluator's frame and policy state, exposed to
/// registered builtins.
///
/// Replaces the old `&Evaluator` parameter that builtins used to
/// receive. The crucial difference: `BuiltinView` does not expose
/// `eval()` or construct a fresh frame, so a registered builtin cannot reset
/// the step budget. Lambda arguments are invoked through
/// [`BuiltinView::invoke_lambda`], which reuses the caller's frame.
///
/// Higher-order combinators (`filter`, `map`, `reduce`, …) are registered
/// builtins too and use the same path; the shared-budget invariant is
/// enforced by this view, not by convention.
#[derive(Copy, Clone)]
pub struct BuiltinView<'a> {
    eval: &'a Evaluator,
    frame: &'a EvalFrame,
}

impl<'a> BuiltinView<'a> {
    /// Only evaluator dispatch can lend out its current frame.
    #[inline(always)]
    fn new(eval: &'a Evaluator, frame: &'a EvalFrame) -> Self {
        Self { eval, frame }
    }

    /// Charge work before a builtin loop or allocation. This shares the calling
    /// program's budget without exposing a frame reset.
    pub fn charge_work(&self, units: usize) -> ExpressionResult<()> {
        self.frame.charge(units)
    }

    /// Bound and charge a string allocation before allocating it.
    pub fn check_output_bytes(&self, bytes: usize) -> ExpressionResult<()> {
        crate::limits::check_limit(
            "builtin output bytes",
            bytes,
            crate::limits::MAX_RESULT_BYTES,
        )?;
        self.charge_work(bytes)
    }

    /// Invoke a lambda argument with the caller's frame.
    ///
    /// The lambda's parameters are bound positionally; a parameter-count
    /// mismatch is an argument error, not a silent partial binding. The
    /// invocation shares the calling program's step budget and recursion
    /// depth, so a lambda cannot be used to reset either.
    ///
    /// # Errors
    /// Returns an argument error when `lambda` is not a lambda expression or
    /// `arguments` does not match its parameter count.
    pub fn invoke_lambda(
        &self,
        lambda: &Expr,
        arguments: &[RuntimeValue],
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        let Expr::Lambda { params, body } = lambda else {
            return Err(ExpressionError::type_error(
                "lambda expression",
                "non-lambda",
            ));
        };
        if params.len() != arguments.len() {
            return Err(ExpressionError::invalid_argument(
                "lambda",
                format!(
                    "expected {} argument(s), got {}",
                    params.len(),
                    arguments.len()
                ),
            ));
        }
        let mut lambda_context = context.clone();
        for (param, argument) in params.iter().zip(arguments) {
            lambda_context.set_lambda_var(param, argument.clone());
        }
        self.eval.eval_with_frame(body, &lambda_context, self.frame)
    }

    /// Invoke a lambda body with an already-prepared context.
    ///
    /// Used by combinators that bind extra names beyond the lambda's own
    /// parameters (for example `$acc` in `reduce`). Shares the caller's frame
    /// exactly like [`Self::invoke_lambda`].
    pub fn eval_body(
        &self,
        body: &Expr,
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        self.eval.eval_with_frame(body, context, self.frame)
    }

    /// Whether strict mode is enabled for this evaluation (engine-level
    /// or context-level policy).
    #[inline(always)]
    pub fn is_strict_mode(&self, context: &EvaluationContext) -> bool {
        self.eval.is_strict_mode(context)
    }

    /// Whether strict-coercion mode is enabled for conversion builtins.
    #[inline(always)]
    pub fn strict_conversions_enabled(&self, context: &EvaluationContext) -> bool {
        self.eval.strict_conversions_enabled(context)
    }

    /// Optional max JSON parse length cap for `parse_json`.
    #[inline(always)]
    pub fn max_json_parse_length(&self, context: &EvaluationContext) -> Option<usize> {
        self.eval.max_json_parse_length(context)
    }

    pub(crate) fn output_builder(self, context: &EvaluationContext) -> crate::BuiltinOutputBuilder {
        crate::BuiltinOutputBuilder::new(self.eval.builtin_output_limits(context))
    }
}

/// Evaluator for expression ASTs
pub struct Evaluator {
    builtins: Arc<BuiltinRegistry>,
    policy: Option<Arc<EvaluationPolicy>>,
    /// Compiled regex cache (pattern → `Arc<Regex>`).
    ///
    /// Backed by `moka::sync::Cache` — concurrent, true-LRU eviction.
    /// Replaces the previous `parking_lot::Mutex<HashMap>` implementation
    /// whose `keys().next()` eviction was order-undefined and could throw
    /// out the hottest pattern under load (ROADMAP #590).
    #[cfg(feature = "regex")]
    regex_cache: moka::sync::Cache<Arc<str>, Arc<Regex>>,
}

impl Evaluator {
    /// Create a new evaluator with the given builtin registry
    pub fn new(builtins: Arc<BuiltinRegistry>) -> Self {
        Self::with_policy(builtins, None)
    }

    /// Create a new evaluator with an optional policy.
    pub fn with_policy(
        builtins: Arc<BuiltinRegistry>,
        policy: Option<Arc<EvaluationPolicy>>,
    ) -> Self {
        Self {
            builtins,
            policy,
            #[cfg(feature = "regex")]
            regex_cache: moka::sync::Cache::new(MAX_REGEX_CACHE_SIZE as u64),
        }
    }

    /// Resolve the effective `max_eval_steps` for this `eval` call.
    ///
    /// Context limits can tighten the engine ceiling, never raise it.
    #[inline]
    fn resolve_max_steps(&self, context: &EvaluationContext) -> Option<usize> {
        let engine_limit = self
            .policy
            .as_deref()
            .and_then(EvaluationPolicy::max_eval_steps)
            .unwrap_or(crate::limits::DEFAULT_MAX_EVAL_STEPS);
        Some(
            context
                .policy()
                .and_then(EvaluationPolicy::max_eval_steps)
                .map_or(engine_limit, |limit| limit.min(engine_limit)),
        )
    }

    /// Evaluate an expression and convert the result to plain JSON.
    #[inline]
    pub fn eval(
        &self,
        expr: &Expr,
        context: &EvaluationContext,
    ) -> ExpressionResult<serde_json::Value> {
        self.eval_runtime(expr, context)
            .map(|value| value.to_json())
    }

    /// Evaluate an expression, returning its runtime value.
    #[inline]
    pub fn eval_runtime(
        &self,
        expr: &Expr,
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        let frame = EvalFrame::new(self.resolve_max_steps(context));
        let result = self.eval_with_frame(expr, context, &frame)?;
        crate::limits::check_value_limits(&result)?;
        Ok(result)
    }

    pub(crate) fn eval_program(
        &self,
        program: &crate::CompiledProgram,
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        let frame = EvalFrame::new(self.resolve_max_steps(context));
        let result = program.evaluate(self, context, &frame)?;
        crate::limits::check_value_limits(&result)?;
        Ok(result)
    }

    /// Evaluate an expression using the caller's step/depth frame.
    ///
    /// Internal recursive paths MUST use this method — calling
    /// `self.eval(...)` from within the evaluator would construct a
    /// fresh frame mid-traversal and reset the step budget, reopening
    /// the CO-C1-01 lambda DoS bypass.
    pub(crate) fn eval_with_frame(
        &self,
        expr: &Expr,
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<RuntimeValue> {
        frame.tick()?;
        frame.enter()?;
        let result = self.eval_node(expr, context, frame);
        frame.leave();
        result
    }

    /// Dispatch on the AST node kind. Split from `eval_with_frame` so
    /// `frame.leave()` still runs on the success path without having to
    /// sprinkle early returns through every match arm.
    fn eval_node(
        &self,
        expr: &Expr,
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<RuntimeValue> {
        match expr {
            Expr::Literal(value) => {
                if matches!(
                    value,
                    RuntimeValue::String(_) | RuntimeValue::Array(_) | RuntimeValue::Object(_)
                ) {
                    frame.charge_value(value)?;
                }
                Ok(value.clone())
            },

            Expr::Variable(name) => {
                let value = match context.resolve_variable_value(name)? {
                    Some(value) => value,
                    None => return self.missing_variable(name, context),
                };
                frame.charge_value(&value)?;
                Ok(value.into_owned())
            },

            Expr::Identifier(name) => {
                // Check if this identifier is a bound lambda parameter
                if let Some(value) = context.get_lambda_var(name) {
                    frame.charge_value(&value)?;
                    return Ok((*value).clone());
                }
                // Otherwise treat as a string constant. The name is already an
                // `Arc<str>`, so this is a refcount bump, not an allocation.
                frame.charge(name.len())?;
                Ok(RuntimeValue::String(Arc::clone(name)))
            },

            Expr::Negate(expr) => {
                let val = self.eval_with_frame(expr, context, frame)?;
                self.negate(&val)
            },

            Expr::Not(expr) => {
                let val = self.eval_with_frame(expr, context, frame)?;
                Ok(RuntimeValue::Bool(!self.coerce_boolean(&val, context)?))
            },

            Expr::Binary { left, op, right } => {
                self.eval_binary_op(*op, left, right, context, frame)
            },

            Expr::PropertyAccess { .. } => self
                .eval_borrowable_node(expr, context, frame)
                .map(Cow::into_owned),

            Expr::IndexAccess { .. } => self
                .eval_borrowable_node(expr, context, frame)
                .map(Cow::into_owned),

            Expr::FunctionCall { name, args } => self.eval_function(name, args, context, frame),

            Expr::Pipeline {
                value,
                function,
                args,
            } => self.eval_pipeline(value, function, args, context, frame),

            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let cond_val = self.eval_with_frame(condition, context, frame)?;
                if self.coerce_boolean(&cond_val, context)? {
                    self.eval_with_frame(then_expr, context, frame)
                } else {
                    self.eval_with_frame(else_expr, context, frame)
                }
            },

            Expr::Lambda { .. } => {
                // A lambda evaluates only through `BuiltinView::invoke_lambda`.
                Err(ExpressionError::eval_error(
                    "Lambda expressions can only be used as function arguments",
                ))
            },

            Expr::Array(elements) => self.eval_array(elements, context, frame),
            Expr::Object(pairs) => self.eval_object(pairs, context, frame),
        }
    }

    pub(crate) fn eval_borrowed_with_frame<'a>(
        &self,
        expr: &'a Expr,
        context: &'a EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<EvalValue<'a>> {
        match expr {
            Expr::Literal(_)
            | Expr::Variable(_)
            | Expr::Identifier(_)
            | Expr::PropertyAccess { .. }
            | Expr::IndexAccess { .. } => {
                frame.tick()?;
                frame.enter()?;
                let result = self.eval_borrowable_node(expr, context, frame);
                frame.leave();
                result
            },
            _ => self.eval_with_frame(expr, context, frame).map(Cow::Owned),
        }
    }

    fn eval_borrowable_node<'a>(
        &self,
        expr: &'a Expr,
        context: &'a EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<EvalValue<'a>> {
        match expr {
            Expr::Literal(value) => {
                if matches!(
                    value,
                    RuntimeValue::String(_) | RuntimeValue::Array(_) | RuntimeValue::Object(_)
                ) {
                    frame.charge_value(value)?;
                }
                Ok(Cow::Borrowed(value))
            },
            Expr::Variable(name) => {
                let value = match context.resolve_variable_value(name)? {
                    Some(value) => value,
                    None => return self.missing_variable(name, context).map(Cow::Owned),
                };
                frame.charge_value(&value)?;
                Ok(value)
            },
            Expr::Identifier(name) => {
                if let Some(value) = context.resolve_lambda_value(name) {
                    frame.charge_value(value)?;
                    return Ok(Cow::Borrowed(value));
                }
                frame.charge(name.len())?;
                Ok(Cow::Owned(RuntimeValue::String(Arc::clone(name))))
            },
            Expr::PropertyAccess { object, property } => {
                if let Expr::Variable(name) = object.as_ref()
                    && name.as_ref() == "node"
                {
                    frame.tick()?;
                    frame.enter()?;
                    let value = context.resolve_node_value(property);
                    frame.leave();
                    let Some(value) = value else {
                        return self.missing_property(property, context).map(Cow::Owned);
                    };
                    frame.charge_value(value)?;
                    return Ok(Cow::Borrowed(value));
                }
                if let Expr::Variable(name) = object.as_ref()
                    && name.as_ref() == "execution"
                {
                    frame.tick()?;
                    frame.enter()?;
                    let value = context.resolve_execution_value(property);
                    frame.leave();
                    let Some(value) = value else {
                        return self.missing_property(property, context).map(Cow::Owned);
                    };
                    frame.charge_value(value)?;
                    return Ok(Cow::Borrowed(value));
                }
                let object = self.eval_borrowed_with_frame(object, context, frame)?;
                self.access_property(object, property, context)
            },
            Expr::IndexAccess { object, index } => {
                if let Expr::Variable(name) = object.as_ref()
                    && name.as_ref() == "node"
                {
                    frame.tick()?;
                    frame.enter()?;
                    frame.leave();
                    let index = self.eval_borrowed_with_frame(index, context, frame)?;
                    let Some(key) = index.as_str() else {
                        return Err(ExpressionError::type_error(
                            "string",
                            crate::value_utils::value_type_name(&index),
                        ));
                    };
                    let Some(value) = context.resolve_node_value(key) else {
                        return self.missing_key(context).map(Cow::Owned);
                    };
                    frame.charge_value(value)?;
                    return Ok(Cow::Borrowed(value));
                }
                if let Expr::Variable(name) = object.as_ref()
                    && name.as_ref() == "execution"
                {
                    frame.tick()?;
                    frame.enter()?;
                    frame.leave();
                    let index = self.eval_borrowed_with_frame(index, context, frame)?;
                    let Some(key) = index.as_str() else {
                        return Err(ExpressionError::type_error(
                            "string",
                            crate::value_utils::value_type_name(&index),
                        ));
                    };
                    let Some(value) = context.resolve_execution_value(key) else {
                        return self.missing_key(context).map(Cow::Owned);
                    };
                    frame.charge_value(value)?;
                    return Ok(Cow::Borrowed(value));
                }
                let object = self.eval_borrowed_with_frame(object, context, frame)?;
                let index = self.eval_borrowed_with_frame(index, context, frame)?;
                self.access_index(object, &index, context)
            },
            _ => self.eval_with_frame(expr, context, frame).map(Cow::Owned),
        }
    }

    fn eval_function(
        &self,
        name: &str,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<RuntimeValue> {
        let mut arguments = Vec::with_capacity(args.len());
        for argument in args {
            arguments.push(self.eval_argument(argument, context, frame)?);
        }
        self.call_function(name, &arguments, context, frame)
    }

    fn eval_pipeline(
        &self,
        value: &Expr,
        function: &str,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<RuntimeValue> {
        let mut arguments = Vec::with_capacity(1 + args.len());
        arguments.push(self.eval_argument(value, context, frame)?);
        for argument in args {
            arguments.push(self.eval_argument(argument, context, frame)?);
        }
        self.call_function(function, &arguments, context, frame)
    }

    /// Evaluate one call argument: lambdas stay unevaluated, values evaluate.
    fn eval_argument<'a>(
        &'a self,
        argument: &'a Expr,
        context: &'a EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<Argument<'a>> {
        match argument {
            Expr::Lambda { .. } => Ok(Argument::Lambda(argument)),
            _ => self
                .eval_borrowed_with_frame(argument, context, frame)
                .map(Argument::Value),
        }
    }

    fn eval_array(
        &self,
        elements: &[Expr],
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<RuntimeValue> {
        let mut budget = crate::limits::AggregateValueBudget::new()?;
        let mut values = Vec::with_capacity(elements.len());
        for element in elements {
            let value = self.eval_with_frame(element, context, frame)?;
            budget.push_array_value(&value)?;
            values.push(value);
        }
        Ok(RuntimeValue::Array(values.into()))
    }

    fn eval_object(
        &self,
        pairs: &[(Arc<str>, Expr)],
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<RuntimeValue> {
        let mut object = std::collections::BTreeMap::new();
        let mut budget = crate::limits::AggregateValueBudget::new()?;
        for (key, expression) in pairs {
            frame.charge(key.len())?;
            let value = self.eval_with_frame(expression, context, frame)?;
            budget.insert_object_value(key, &value, object.get(key.as_ref()))?;
            object.insert(Arc::clone(key), value);
        }
        Ok(RuntimeValue::Object(Arc::new(object)))
    }

    // Keep numeric conversion/error temporaries out of every recursive dispatch frame.
    fn negate(&self, value: &RuntimeValue) -> ExpressionResult<RuntimeValue> {
        match value {
            RuntimeValue::Float(value) => {
                return crate::value_utils::finite_result(-value, "negation");
            },
            RuntimeValue::Integer(value) => {
                return crate::value_utils::integer_result(
                    i128::from(*value).checked_neg(),
                    "negation",
                );
            },
            RuntimeValue::Unsigned(value) => {
                return crate::value_utils::integer_result(
                    i128::from(*value).checked_neg(),
                    "negation",
                );
            },
            _ => {},
        }
        Err(ExpressionError::type_error(
            "number",
            crate::value_utils::value_type_name(value),
        ))
    }

    /// Evaluate a binary operation
    #[inline]
    fn eval_binary_op(
        &self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<RuntimeValue> {
        // Short-circuit evaluation for logical operators
        match op {
            BinaryOp::And => {
                let left_val = self.eval_borrowed_with_frame(left, context, frame)?;
                if !self.coerce_boolean(&left_val, context)? {
                    // Short-circuit: if left is false, don't evaluate right
                    return Ok(RuntimeValue::Bool(false));
                }
                let right_val = self.eval_borrowed_with_frame(right, context, frame)?;
                Ok(RuntimeValue::Bool(
                    self.coerce_boolean(&right_val, context)?,
                ))
            },
            BinaryOp::Or => {
                let left_val = self.eval_borrowed_with_frame(left, context, frame)?;
                if self.coerce_boolean(&left_val, context)? {
                    // Short-circuit: if left is true, don't evaluate right
                    return Ok(RuntimeValue::Bool(true));
                }
                let right_val = self.eval_borrowed_with_frame(right, context, frame)?;
                Ok(RuntimeValue::Bool(
                    self.coerce_boolean(&right_val, context)?,
                ))
            },
            // For all other operators, evaluate both operands
            _ => {
                let left_val = self.eval_borrowed_with_frame(left, context, frame)?;
                let right_val = self.eval_borrowed_with_frame(right, context, frame)?;

                match op {
                    BinaryOp::Add => self.add(&left_val, &right_val, frame),
                    BinaryOp::Subtract => self.subtract(&left_val, &right_val),
                    BinaryOp::Multiply => self.multiply(&left_val, &right_val),
                    BinaryOp::Divide => self.divide(&left_val, &right_val),
                    BinaryOp::Modulo => self.modulo(&left_val, &right_val),
                    BinaryOp::Power => self.power(&left_val, &right_val),
                    BinaryOp::Equal | BinaryOp::NotEqual => {
                        // Structural equality already compares numbers exactly
                        // across representations; see `RuntimeValue::eq`.
                        let equal = left_val == right_val;
                        Ok(RuntimeValue::Bool(if op == BinaryOp::Equal {
                            equal
                        } else {
                            !equal
                        }))
                    },
                    BinaryOp::LessThan => self.less_than(&left_val, &right_val, context),
                    BinaryOp::GreaterThan => self.greater_than(&left_val, &right_val, context),
                    BinaryOp::LessEqual => self.less_equal(&left_val, &right_val, context),
                    BinaryOp::GreaterEqual => self.greater_equal(&left_val, &right_val, context),
                    BinaryOp::RegexMatch => self.regex_match(&left_val, &right_val),
                    BinaryOp::And | BinaryOp::Or => Err(ExpressionError::internal(
                        "logical operator escaped short-circuit dispatch",
                    )),
                }
            },
        }
    }

    /// Addition
    #[inline]
    fn add(
        &self,
        left: &RuntimeValue,
        right: &RuntimeValue,
        frame: &EvalFrame,
    ) -> ExpressionResult<RuntimeValue> {
        match (left, right) {
            (RuntimeValue::String(left), RuntimeValue::String(right)) => {
                crate::limits::check_limit(
                    "string output bytes",
                    left.len().saturating_add(right.len()),
                    crate::limits::MAX_RESULT_BYTES,
                )?;
                frame.charge(left.len() + right.len())?;
                // Pre-allocate exact capacity to avoid reallocations
                let mut result = String::with_capacity(left.len() + right.len());
                result.push_str(left);
                result.push_str(right);
                Ok(RuntimeValue::string(result))
            },
            _ if left.is_number() && right.is_number() => {
                if let (Some(left), Some(right)) = (left.as_i128(), right.as_i128()) {
                    crate::value_utils::integer_result(left.checked_add(right), "addition")
                } else {
                    let lf = left.as_f64().unwrap_or_default();
                    let rf = right.as_f64().unwrap_or_default();
                    crate::value_utils::finite_result(lf + rf, "addition")
                }
            },
            _ => Err(ExpressionError::type_error(
                "number or string",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            )),
        }
    }

    /// Subtraction
    #[inline]
    fn subtract(
        &self,
        left: &RuntimeValue,
        right: &RuntimeValue,
    ) -> ExpressionResult<RuntimeValue> {
        if left.is_number() && right.is_number() {
            if let (Some(left), Some(right)) = (left.as_i128(), right.as_i128()) {
                crate::value_utils::integer_result(left.checked_sub(right), "subtraction")
            } else {
                let lf = left.as_f64().unwrap_or_default();
                let rf = right.as_f64().unwrap_or_default();
                crate::value_utils::finite_result(lf - rf, "subtraction")
            }
        } else {
            Err(ExpressionError::type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            ))
        }
    }

    /// Multiplication
    #[inline]
    fn multiply(
        &self,
        left: &RuntimeValue,
        right: &RuntimeValue,
    ) -> ExpressionResult<RuntimeValue> {
        if left.is_number() && right.is_number() {
            if let (Some(left), Some(right)) = (left.as_i128(), right.as_i128()) {
                crate::value_utils::integer_result(left.checked_mul(right), "multiplication")
            } else {
                let lf = left.as_f64().unwrap_or_default();
                let rf = right.as_f64().unwrap_or_default();
                crate::value_utils::finite_result(lf * rf, "multiplication")
            }
        } else {
            Err(ExpressionError::type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            ))
        }
    }

    /// Division
    #[inline]
    fn divide(&self, left: &RuntimeValue, right: &RuntimeValue) -> ExpressionResult<RuntimeValue> {
        if left.is_number() && right.is_number() {
            // Always use floating point for division
            let lf = crate::value_utils::to_float(left).map_err(ExpressionError::eval_error)?;
            let rf = crate::value_utils::to_float(right).map_err(ExpressionError::eval_error)?;

            if rf == 0.0 {
                return Err(ExpressionError::division_by_zero());
            }
            let result = lf / rf;
            crate::value_utils::finite_result(result, "division")
        } else {
            Err(ExpressionError::type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            ))
        }
    }

    /// Modulo
    #[inline]
    fn modulo(&self, left: &RuntimeValue, right: &RuntimeValue) -> ExpressionResult<RuntimeValue> {
        if left.is_number() && right.is_number() {
            // Try integer modulo first
            if let (Some(left), Some(right)) = (left.as_i128(), right.as_i128()) {
                if right == 0 {
                    return Err(ExpressionError::division_by_zero());
                }
                crate::value_utils::integer_result(left.checked_rem(right), "remainder")
            } else {
                // Fall back to float modulo
                let lf = crate::value_utils::to_float(left).map_err(ExpressionError::eval_error)?;
                let rf =
                    crate::value_utils::to_float(right).map_err(ExpressionError::eval_error)?;
                if rf == 0.0 {
                    return Err(ExpressionError::division_by_zero());
                }
                crate::value_utils::finite_result(lf % rf, "remainder")
            }
        } else {
            Err(ExpressionError::type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            ))
        }
    }

    /// Power
    #[inline]
    fn power(&self, left: &RuntimeValue, right: &RuntimeValue) -> ExpressionResult<RuntimeValue> {
        if left.is_number() && right.is_number() {
            // Always use floating point for power operations
            let lf = crate::value_utils::to_float(left).map_err(ExpressionError::eval_error)?;
            let rf = crate::value_utils::to_float(right).map_err(ExpressionError::eval_error)?;
            crate::value_utils::finite_result(lf.powf(rf), "power")
        } else {
            Err(ExpressionError::type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            ))
        }
    }

    /// Less than comparison
    #[inline]
    fn less_than(
        &self,
        left: &RuntimeValue,
        right: &RuntimeValue,
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        self.compare_operands(left, right, context, std::cmp::Ordering::is_lt)
    }

    /// Greater than comparison
    #[inline]
    fn greater_than(
        &self,
        left: &RuntimeValue,
        right: &RuntimeValue,
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        self.compare_operands(left, right, context, std::cmp::Ordering::is_gt)
    }

    /// Less than or equal comparison
    fn less_equal(
        &self,
        left: &RuntimeValue,
        right: &RuntimeValue,
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        self.compare_operands(left, right, context, std::cmp::Ordering::is_le)
    }

    /// Greater than or equal comparison
    fn greater_equal(
        &self,
        left: &RuntimeValue,
        right: &RuntimeValue,
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        self.compare_operands(left, right, context, std::cmp::Ordering::is_ge)
    }

    /// Shared ordering seam for the four comparison operators.
    ///
    /// Both numbers and strings reduce to a [`std::cmp::Ordering`]; `accept`
    /// supplies the operator-specific predicate. Non-finite numbers surface as
    /// [`ExpressionError::NonFiniteNumber`] from
    /// [`crate::value_utils::compare_numbers`] regardless of which operator
    /// asked, so error behaviour stays uniform.
    fn compare_operands(
        &self,
        left: &RuntimeValue,
        right: &RuntimeValue,
        context: &EvaluationContext,
        accept: fn(std::cmp::Ordering) -> bool,
    ) -> ExpressionResult<RuntimeValue> {
        if self.strict_numeric_comparisons_enabled(context)
            && (!left.is_number() || !right.is_number())
        {
            return Err(ExpressionError::type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            ));
        }
        let ordering = match (left, right) {
            (RuntimeValue::String(left), RuntimeValue::String(right)) => left.cmp(right),
            _ if left.is_number() && right.is_number() => crate::value_utils::compare_numbers(
                left, right,
            )
            .ok_or(ExpressionError::NonFiniteNumber {
                operation: "comparison",
            })?,
            _ => {
                return Err(ExpressionError::type_error(
                    "comparable values",
                    format!(
                        "{} and {}",
                        crate::value_utils::value_type_name(left),
                        crate::value_utils::value_type_name(right)
                    ),
                ));
            },
        };
        Ok(RuntimeValue::Bool(accept(ordering)))
    }

    /// Regex match with ReDoS protection
    ///
    /// Security measures:
    /// - Pattern length limit (MAX_REGEX_PATTERN_LEN)
    /// - Detection of potentially dangerous nested quantifiers
    /// - Cache size limit with eviction (MAX_REGEX_CACHE_SIZE)
    #[cfg(feature = "regex")]
    fn regex_match(
        &self,
        left: &RuntimeValue,
        right: &RuntimeValue,
    ) -> ExpressionResult<RuntimeValue> {
        let text = left.as_str().ok_or_else(|| {
            ExpressionError::type_error("string", crate::value_utils::value_type_name(left))
        })?;

        let pattern = right.as_str().ok_or_else(|| {
            ExpressionError::type_error("string", crate::value_utils::value_type_name(right))
        })?;

        // ReDoS protection: check pattern length
        if pattern.len() > MAX_REGEX_PATTERN_LEN {
            return Err(ExpressionError::regex_error(format!(
                "Regex pattern too long: {} chars (max {})",
                pattern.len(),
                MAX_REGEX_PATTERN_LEN
            )));
        }

        // ReDoS protection: detect potentially dangerous patterns
        if Self::is_potentially_dangerous_regex(pattern) {
            return Err(ExpressionError::regex_error(
                "Regex pattern rejected: contains potentially dangerous nested quantifiers",
            ));
        }

        // Hot path: moka concurrent get is lock-free. Compilation happens
        // outside any lock; concurrent insertions are tolerated (last writer
        // wins; redundant compiles are wasted but not incorrect).
        let regex = if let Some(cached) = self.regex_cache.get(pattern) {
            cached
        } else {
            let compiled = Regex::new(pattern)
                .map_err(|_| ExpressionError::regex_error("Regex pattern is invalid"))?;
            let arc = Arc::new(compiled);
            self.regex_cache
                .insert(Arc::from(pattern), Arc::clone(&arc));
            arc
        };

        Ok(RuntimeValue::Bool(regex.is_match(text)))
    }

    /// Check if a regex pattern contains potentially dangerous constructs
    /// that could lead to catastrophic backtracking (ReDoS).
    ///
    /// Detects patterns like `(a+)+`, `(a*)*`, `(a+)*` which can cause
    /// exponential time complexity.
    #[cfg(feature = "regex")]
    fn is_potentially_dangerous_regex(pattern: &str) -> bool {
        let chars: Vec<char> = pattern.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            // Look for opening parenthesis
            if chars[i] == '(' {
                let group_start = i;

                // Find matching closing parenthesis
                match Self::find_group_end(&chars, i + 1) {
                    Some(end) => {
                        if Self::group_quantified_dangerously(&chars, group_start, end) {
                            return true;
                        }
                        i = end;
                    },
                    // Unbalanced group: the scan ran past end of input, so
                    // the caller's loop could not find another '(' in the
                    // tail — stop instead of resuming.
                    None => break,
                }
            } else if chars[i] == '\\' {
                // Skip escaped character
                i += 2;
            } else {
                i += 1;
            }
        }

        false
    }

    /// Index just after the ')' matching the '(' directly before `from`,
    /// scanning `chars` from `from` with depth and escape tracking.
    /// Returns `None` when the group never closes before end of input
    /// (the scan has consumed the whole tail by then).
    #[cfg(feature = "regex")]
    fn find_group_end(chars: &[char], from: usize) -> Option<usize> {
        let len = chars.len();
        let mut i = from;
        let mut depth = 1;

        while i < len && depth > 0 {
            match chars[i] {
                '(' => depth += 1,
                ')' => depth -= 1,
                '\\' => i += 1, // Skip escaped character
                _ => {},
            }
            i += 1;
        }

        if depth == 0 { Some(i) } else { None }
    }

    /// `true` when the group ending at `group_end` is followed by `+` or `*`
    /// and its content itself contains a quantifier.
    #[cfg(feature = "regex")]
    fn group_quantified_dangerously(chars: &[char], group_start: usize, group_end: usize) -> bool {
        let len = chars.len();

        // Check if group is followed by a quantifier.
        if group_end >= len || !matches!(chars[group_end], '+' | '*') {
            return false;
        }

        // Check if the group content contains a nested quantifier.
        // `group_end - 1` is the closing `)`; content lives between the parens.
        chars[group_start + 1..group_end - 1]
            .iter()
            .any(|ch| matches!(ch, '+' | '*' | '{'))
    }

    #[cfg(not(feature = "regex"))]
    fn regex_match(
        &self,
        _left: &RuntimeValue,
        _right: &RuntimeValue,
    ) -> ExpressionResult<RuntimeValue> {
        Err(ExpressionError::eval_error(
            "Regex matching is not enabled (feature 'regex' not enabled)",
        ))
    }

    /// The value produced for a missing variable under the current policy.
    fn missing_variable(
        &self,
        name: &str,
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        if self.missing_lookup_is_undefined(context) {
            Ok(RuntimeValue::Undefined)
        } else {
            Err(ExpressionError::variable_not_found(name))
        }
    }

    /// The value produced for a missing authored property under the current policy.
    ///
    /// The name comes from the template source, so echoing it is safe.
    fn missing_property(
        &self,
        property: &str,
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        if self.missing_lookup_is_undefined(context) {
            Ok(RuntimeValue::Undefined)
        } else {
            Err(ExpressionError::eval_error(format!(
                "Property '{property}' not found"
            )))
        }
    }

    /// The value produced for a missing runtime index key under the current policy.
    ///
    /// The key was computed from input data, so it may hold credential material
    /// and must never appear in a diagnostic.
    fn missing_key(&self, context: &EvaluationContext) -> ExpressionResult<RuntimeValue> {
        if self.missing_lookup_is_undefined(context) {
            Ok(RuntimeValue::Undefined)
        } else {
            Err(ExpressionError::eval_error("Object key not found"))
        }
    }

    /// Access a property of an object.
    fn access_property<'a>(
        &self,
        object: EvalValue<'a>,
        property: &str,
        context: &EvaluationContext,
    ) -> ExpressionResult<EvalValue<'a>> {
        match object {
            Cow::Borrowed(RuntimeValue::Object(entries)) => match entries.get(property) {
                Some(value) => Ok(Cow::Borrowed(value)),
                None => self.missing_property(property, context).map(Cow::Owned),
            },
            Cow::Owned(RuntimeValue::Object(entries)) => match entries.get(property) {
                Some(value) => Ok(Cow::Owned(value.clone())),
                None => self.missing_property(property, context).map(Cow::Owned),
            },
            Cow::Borrowed(other) => Err(ExpressionError::type_error(
                "object",
                crate::value_utils::value_type_name(other),
            )),
            Cow::Owned(other) => Err(ExpressionError::type_error(
                "object",
                crate::value_utils::value_type_name(&other),
            )),
        }
    }

    /// Resolve a signed integer index into a `0..len` position, supporting
    /// negative indices (Python-style).
    fn normalize_index(idx: i64, len: usize) -> ExpressionResult<usize> {
        let len_i64 = len as i64;
        let actual = if idx < 0 { len_i64 + idx } else { idx };
        if actual < 0 || actual >= len_i64 {
            return Err(ExpressionError::index_out_of_bounds(actual as usize, len));
        }
        Ok(actual as usize)
    }

    /// Access an element of an array or object by index.
    fn access_index<'a>(
        &self,
        object: EvalValue<'a>,
        index: &RuntimeValue,
        context: &EvaluationContext,
    ) -> ExpressionResult<EvalValue<'a>> {
        match object {
            Cow::Borrowed(RuntimeValue::Array(array)) => {
                let pos = Self::resolve_array_index(index, array.len())?;
                array
                    .get(pos)
                    .map(Cow::Borrowed)
                    .ok_or_else(|| ExpressionError::index_out_of_bounds(pos, array.len()))
            },
            Cow::Owned(RuntimeValue::Array(array)) => {
                let pos = Self::resolve_array_index(index, array.len())?;
                array
                    .get(pos)
                    .cloned()
                    .map(Cow::Owned)
                    .ok_or_else(|| ExpressionError::index_out_of_bounds(pos, array.len()))
            },
            Cow::Borrowed(RuntimeValue::Object(entries)) => {
                let key = index.as_str().ok_or_else(|| {
                    ExpressionError::type_error(
                        "string",
                        crate::value_utils::value_type_name(index),
                    )
                })?;
                match entries.get(key) {
                    Some(value) => Ok(Cow::Borrowed(value)),
                    None => self.missing_key(context).map(Cow::Owned),
                }
            },
            Cow::Owned(RuntimeValue::Object(entries)) => {
                let key = index.as_str().ok_or_else(|| {
                    ExpressionError::type_error(
                        "string",
                        crate::value_utils::value_type_name(index),
                    )
                })?;
                match entries.get(key) {
                    Some(value) => Ok(Cow::Owned(value.clone())),
                    None => self.missing_key(context).map(Cow::Owned),
                }
            },
            Cow::Borrowed(other) => Err(ExpressionError::type_error(
                "array or object",
                crate::value_utils::value_type_name(other),
            )),
            Cow::Owned(other) => Err(ExpressionError::type_error(
                "array or object",
                crate::value_utils::value_type_name(&other),
            )),
        }
    }

    /// Resolve an array index value (integer, allowing negative offsets) into
    /// a `0..len` position.
    fn resolve_array_index(index: &RuntimeValue, len: usize) -> ExpressionResult<usize> {
        let idx = index.as_i64().ok_or_else(|| {
            ExpressionError::type_error("integer", crate::value_utils::value_type_name(index))
        })?;
        Self::normalize_index(idx, len)
    }

    /// Call a builtin function
    fn call_function(
        &self,
        name: &str,
        args: &[Argument<'_>],
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<RuntimeValue> {
        self.ensure_function_allowed(name, context)?;
        for argument in args {
            if let Argument::Value(value) = argument {
                frame.charge_value(value)?;
            }
        }
        let result = self
            .builtins
            .call(name, args, BuiltinView::new(self, frame), context)?;
        let result = crate::BuiltinOutputBuilder::new(self.builtin_output_limits(context))
            .value(result)?
            .into_value();
        frame.charge_value(&result)?;
        Ok(result)
    }

    fn canonical_function_name<'a>(&self, name: &'a str) -> &'a str {
        match name {
            "all" => "every",
            "any" => "some",
            _ => name,
        }
    }

    fn ensure_function_allowed(
        &self,
        name: &str,
        context: &EvaluationContext,
    ) -> ExpressionResult<()> {
        let canonical = self.canonical_function_name(name);
        let policies = [self.policy.as_deref(), context.policy()];

        for policy in policies.into_iter().flatten() {
            let denied = policy.denied_functions();
            if denied.contains(name) || denied.contains(canonical) {
                return Err(ExpressionError::eval_error(format!(
                    "Function '{name}' is denied by policy"
                )));
            }
        }

        for policy in policies.into_iter().flatten() {
            if self.is_allowed_by_policy(policy, name, canonical) {
                continue;
            }
            return Err(ExpressionError::eval_error(format!(
                "Function '{name}' is not allowed by policy"
            )));
        }

        Ok(())
    }

    fn is_allowed_by_policy(&self, policy: &EvaluationPolicy, name: &str, canonical: &str) -> bool {
        let Some(allowed) = policy.allowed_functions() else {
            return true;
        };

        if allowed.contains(name) || allowed.contains(canonical) {
            return true;
        }

        matches!(
            canonical,
            "every" if allowed.contains("all")
        ) || matches!(
            canonical,
            "some" if allowed.contains("any")
        )
    }

    fn strict_mode_enabled(&self, context: &EvaluationContext) -> bool {
        let engine_strict = self
            .policy
            .as_deref()
            .is_some_and(EvaluationPolicy::strict_mode);
        let context_strict = context.policy().is_some_and(EvaluationPolicy::strict_mode);
        engine_strict || context_strict
    }

    pub(crate) fn is_strict_mode(&self, context: &EvaluationContext) -> bool {
        self.strict_mode_enabled(context)
    }

    pub(crate) fn strict_conversions_enabled(&self, context: &EvaluationContext) -> bool {
        let engine_strict = self
            .policy
            .as_deref()
            .is_some_and(EvaluationPolicy::strict_conversion_functions);
        let context_strict = context
            .policy()
            .is_some_and(EvaluationPolicy::strict_conversion_functions);
        engine_strict || context_strict
    }

    fn strict_numeric_comparisons_enabled(&self, context: &EvaluationContext) -> bool {
        let engine_strict = self
            .policy
            .as_deref()
            .is_some_and(EvaluationPolicy::strict_numeric_comparisons);
        let context_strict = context
            .policy()
            .is_some_and(EvaluationPolicy::strict_numeric_comparisons);
        engine_strict || context_strict
    }

    /// Whether a missing lookup yields `Undefined`.
    ///
    /// Enabling is additive: either the engine or the context policy can
    /// switch it on, and neither can switch the other off.
    fn missing_lookup_is_undefined(&self, context: &EvaluationContext) -> bool {
        let engine_undefined = self
            .policy
            .as_deref()
            .is_some_and(|policy| policy.missing_lookup() == MissingLookup::Undefined);
        let context_undefined = context
            .policy()
            .is_some_and(|policy| policy.missing_lookup() == MissingLookup::Undefined);
        engine_undefined || context_undefined
    }

    pub(crate) fn max_json_parse_length(&self, context: &EvaluationContext) -> Option<usize> {
        let engine_limit = self
            .policy
            .as_deref()
            .and_then(EvaluationPolicy::max_json_parse_length)
            .unwrap_or(crate::limits::MAX_SOURCE_BYTES);
        Some(
            context
                .policy()
                .and_then(EvaluationPolicy::max_json_parse_length)
                .map_or(engine_limit, |limit| limit.min(engine_limit)),
        )
    }

    fn coerce_boolean(
        &self,
        value: &RuntimeValue,
        context: &EvaluationContext,
    ) -> ExpressionResult<bool> {
        if self.strict_mode_enabled(context) && value.as_bool().is_none() {
            return Err(ExpressionError::type_error(
                "boolean",
                crate::value_utils::value_type_name(value),
            ));
        }
        Ok(crate::value_utils::to_boolean(value))
    }

    pub(crate) fn builtin_output_limits(
        &self,
        context: &EvaluationContext,
    ) -> crate::BuiltinOutputLimits {
        let engine_limits = self
            .policy
            .as_deref()
            .map_or_else(
                crate::BuiltinOutputLimits::default,
                EvaluationPolicy::builtin_output_limits,
            )
            .most_restrictive(crate::BuiltinOutputLimits::default());
        context.policy().map_or(engine_limits, |policy| {
            engine_limits.most_restrictive(policy.builtin_output_limits())
        })
    }
}

#[cfg(test)]
mod tests;
