//! AST evaluation module
//!
//! This module implements the evaluation of parsed expression ASTs.

use std::{borrow::Cow, cell::Cell, sync::Arc};

#[cfg(feature = "regex")]
use regex::Regex;
use serde_json::{Number, Value};

use crate::{
    ExpressionError,
    ast::{BinaryOp, Expr},
    builtins::BuiltinRegistry,
    context::EvaluationContext,
    error::{ExpressionErrorExt, ExpressionResult},
    policy::EvaluationPolicy,
};

/// Maximum recursion depth for expression evaluation
const MAX_RECURSION_DEPTH: usize = 256;

/// Maximum length for regex patterns to prevent ReDoS attacks
#[cfg(feature = "regex")]
const MAX_REGEX_PATTERN_LEN: usize = 1000;

/// Maximum number of cached regex patterns (simple LRU-style eviction)
#[cfg(feature = "regex")]
const MAX_REGEX_CACHE_SIZE: usize = 100;

pub(crate) type EvalValue<'a> = Cow<'a, Value>;

/// Per-call evaluation frame that tracks recursion depth and the DoS
/// step budget for a single top-level [`Evaluator::eval`] invocation.
///
/// Lives on the caller's stack (never on `Evaluator`, never on
/// [`EvaluationContext`]). Every recursive path inside the evaluator
/// threads `&mut EvalFrame` instead of a bare `depth: usize`, so:
///
/// - Concurrent `Arc<Evaluator>` users each get their own frame with zero synchronization — no
///   shared atomics, no thread-local state.
/// - Nested lambda evaluation cannot accidentally reset the counter (the old `self.eval(...)`
///   re-entry pattern that did `self.steps.store(0)` at the top of every call is gone).
/// - One top-level `eval` call = one step budget, regardless of how many lambdas / reduces /
///   pipelines it recurses through.
///
/// Closes CO-C1-01 (issue #252): `max_eval_steps` bypass via lambdas.
pub(crate) struct EvalFrame {
    depth: usize,
    steps: Cell<usize>,
    max_steps: Option<usize>,
}

impl EvalFrame {
    /// Create a fresh frame with the given step cap (snapshotted once
    /// from the effective policy at the top-level `eval` entry).
    #[inline]
    fn new(max_steps: Option<usize>) -> Self {
        Self {
            depth: 0,
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

    fn charge_value(&self, value: &Value) -> ExpressionResult<()> {
        crate::limits::check_value_limits(value)?;
        self.tick()?;
        let mut pending = vec![(value, 1)];
        while let Some((value, depth)) = pending.pop() {
            crate::limits::check_limit("value depth", depth, crate::limits::MAX_AST_DEPTH)?;
            match value {
                Value::String(text) => self.charge(text.len())?,
                Value::Array(values) => {
                    self.charge(values.len())?;
                    pending.extend(values.iter().map(|value| (value, depth + 1)));
                },
                Value::Object(values) => {
                    self.charge(values.len())?;
                    for (key, value) in values {
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
    fn enter(&mut self) -> ExpressionResult<()> {
        if self.depth >= MAX_RECURSION_DEPTH {
            tracing::warn!(
                target: "nebula_expression::dos",
                limit = MAX_RECURSION_DEPTH,
                actual = self.depth,
                "recursion depth exceeded"
            );
            return Err(ExpressionError::depth_exceeded(
                MAX_RECURSION_DEPTH,
                self.depth,
            ));
        }
        self.depth += 1;
        Ok(())
    }

    /// Leave a recursion level previously entered via [`enter`].
    #[inline]
    fn leave(&mut self) {
        debug_assert!(self.depth > 0, "leave called without matching enter");
        self.depth -= 1;
    }
}

/// Read-only handle on the evaluator's policy state, exposed to
/// registered builtins.
///
/// Replaces the old `&Evaluator` parameter that builtins used to
/// receive. The crucial difference: `BuiltinView` does not expose
/// `eval()` or `eval_with_frame()`, so a registered builtin literally
/// cannot recurse back into AST evaluation. The CO-C1-01 step-budget
/// bypass therefore becomes type-impossible for first-party builtins,
/// closing the discipline-only contract documented in the crate
/// `lib.rs` "Known limitation" note (issue #252).
///
/// Higher-order combinators (`filter`, `map`, `reduce`, …) live inside
/// the evaluator module itself and continue to call `eval_with_frame`
/// directly with the caller's `EvalFrame`. They never go through this
/// view; the type-enforced boundary applies only to the
/// `BuiltinRegistry`'s callable surface.
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
    /// program's budget without exposing evaluator re-entry or budget resets.
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

    /// Evaluate an expression in the given context.
    ///
    /// Top-level AST and compiled-program calls construct a fresh [`EvalFrame`].
    /// All recursive paths inside the evaluator reuse the caller's frame
    /// via [`eval_with_frame`], so the step budget defined by
    /// [`EvaluationPolicy::max_eval_steps`] is enforced across ALL
    /// nested work — lambdas, reduces, pipelines, higher-order combinators.
    ///
    /// # CO-C1-01 footgun (builtins) — closed
    ///
    /// `BuiltinRegistry::call` now hands builtins a [`BuiltinView`]
    /// instead of `&Evaluator`. The view exposes policy and work-charging
    /// methods, so a registered builtin cannot recurse back into AST
    /// evaluation — the historical step-budget bypass (issue #252) is
    /// type-enforced shut. Higher-order combinators (`map`, `filter`,
    /// `reduce`, ...) live inside this module and continue to use
    /// `eval_with_frame` with the caller's `EvalFrame` directly, so
    /// their iteration budget stays accumulated. See the `lib.rs`
    /// crate-level docs and `docs/pitfalls.md` for the historical
    /// context.
    #[inline]
    pub fn eval(&self, expr: &Expr, context: &EvaluationContext) -> ExpressionResult<Value> {
        let mut frame = EvalFrame::new(self.resolve_max_steps(context));
        let result = self.eval_with_frame(expr, context, &mut frame)?;
        crate::limits::check_value_limits(&result)?;
        Ok(result)
    }

    pub(crate) fn eval_program(
        &self,
        program: &crate::CompiledProgram,
        context: &EvaluationContext,
    ) -> ExpressionResult<Value> {
        let mut frame = EvalFrame::new(self.resolve_max_steps(context));
        let result = program.evaluate(self, context, &mut frame)?;
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
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
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
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        match expr {
            Expr::Literal(val) => {
                if matches!(val, Value::String(_) | Value::Array(_) | Value::Object(_)) {
                    frame.charge_value(val)?;
                }
                Ok(val.clone())
            },

            Expr::Variable(name) => {
                let value = context
                    .resolve_variable_value(name)?
                    .ok_or_else(|| ExpressionError::expression_variable_not_found(&**name))?;
                frame.charge_value(&value)?;
                Ok(value.into_owned())
            },

            Expr::Identifier(name) => {
                // Check if this identifier is a bound lambda parameter
                if let Some(value) = context.get_lambda_var(name) {
                    frame.charge_value(&value)?;
                    return Ok((*value).clone());
                }
                // Otherwise treat as a string constant
                frame.charge(name.len())?;
                Ok(Value::String(name.as_ref().to_string()))
            },

            Expr::Negate(expr) => {
                let val = self.eval_with_frame(expr, context, frame)?;
                self.negate(&val)
            },

            Expr::Not(expr) => {
                let val = self.eval_with_frame(expr, context, frame)?;
                Ok(Value::Bool(!self.coerce_boolean(&val, context)?))
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
                // Lambdas are handled specially in higher-order functions
                Err(ExpressionError::expression_eval_error(
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
        frame: &mut EvalFrame,
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
        frame: &mut EvalFrame,
    ) -> ExpressionResult<EvalValue<'a>> {
        match expr {
            Expr::Literal(value) => {
                if matches!(value, Value::String(_) | Value::Array(_) | Value::Object(_)) {
                    frame.charge_value(value)?;
                }
                Ok(Cow::Borrowed(value))
            },
            Expr::Variable(name) => {
                let value = context
                    .resolve_variable_value(name)?
                    .ok_or_else(|| ExpressionError::expression_variable_not_found(&**name))?;
                frame.charge_value(&value)?;
                Ok(value)
            },
            Expr::Identifier(name) => {
                if let Some(value) = context.resolve_lambda_value(name) {
                    frame.charge_value(value)?;
                    return Ok(Cow::Borrowed(value));
                }
                frame.charge(name.len())?;
                Ok(Cow::Owned(Value::String(name.as_ref().to_string())))
            },
            Expr::PropertyAccess { object, property } => {
                if let Expr::Variable(name) = object.as_ref()
                    && name.as_ref() == "node"
                {
                    frame.tick()?;
                    frame.enter()?;
                    let value = context.resolve_node_value(property).ok_or_else(|| {
                        ExpressionError::expression_eval_error(format!(
                            "Property '{property}' not found"
                        ))
                    });
                    frame.leave();
                    let value = value?;
                    frame.charge_value(value)?;
                    return Ok(Cow::Borrowed(value));
                }
                if let Expr::Variable(name) = object.as_ref()
                    && name.as_ref() == "execution"
                {
                    frame.tick()?;
                    frame.enter()?;
                    let value = context.resolve_execution_value(property).ok_or_else(|| {
                        ExpressionError::expression_eval_error(format!(
                            "Property '{property}' not found"
                        ))
                    });
                    frame.leave();
                    let value = value?;
                    frame.charge_value(value)?;
                    return Ok(Cow::Borrowed(value));
                }
                let object = self.eval_borrowed_with_frame(object, context, frame)?;
                self.access_property(object, property)
            },
            Expr::IndexAccess { object, index } => {
                if let Expr::Variable(name) = object.as_ref()
                    && name.as_ref() == "node"
                {
                    frame.tick()?;
                    frame.enter()?;
                    frame.leave();
                    let index = self.eval_borrowed_with_frame(index, context, frame)?;
                    let key = index.as_str().ok_or_else(|| {
                        ExpressionError::expression_type_error(
                            "string",
                            crate::value_utils::value_type_name(&index),
                        )
                    })?;
                    let value = context.resolve_node_value(key).ok_or_else(|| {
                        ExpressionError::expression_eval_error("Object key not found")
                    })?;
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
                    let key = index.as_str().ok_or_else(|| {
                        ExpressionError::expression_type_error(
                            "string",
                            crate::value_utils::value_type_name(&index),
                        )
                    })?;
                    let value = context.resolve_execution_value(key).ok_or_else(|| {
                        ExpressionError::expression_eval_error("Object key not found")
                    })?;
                    frame.charge_value(value)?;
                    return Ok(Cow::Borrowed(value));
                }
                let object = self.eval_borrowed_with_frame(object, context, frame)?;
                let index = self.eval_borrowed_with_frame(index, context, frame)?;
                self.access_index(object, &index)
            },
            _ => self.eval_with_frame(expr, context, frame).map(Cow::Owned),
        }
    }

    fn eval_function(
        &self,
        name: &str,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        if let Some(result) = self.try_higher_order_function(name, args, context, frame) {
            return result;
        }
        let mut values = Vec::with_capacity(args.len());
        for argument in args {
            values.push(self.eval_borrowed_with_frame(argument, context, frame)?);
        }
        self.call_function(name, &values, context, frame)
    }

    fn eval_pipeline(
        &self,
        value: &Expr,
        function: &str,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        let mut full_args = Vec::with_capacity(1 + args.len());
        full_args.push(value.clone());
        full_args.extend(args.iter().cloned());
        if let Some(result) = self.try_higher_order_function(function, &full_args, context, frame) {
            return result;
        }
        let mut values = Vec::with_capacity(1 + args.len());
        values.push(self.eval_borrowed_with_frame(value, context, frame)?);
        for argument in args {
            values.push(self.eval_borrowed_with_frame(argument, context, frame)?);
        }
        self.call_function(function, &values, context, frame)
    }

    fn eval_array(
        &self,
        elements: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        let mut budget = crate::limits::AggregateValueBudget::new()?;
        let mut values = Vec::with_capacity(elements.len());
        for element in elements {
            let value = self.eval_with_frame(element, context, frame)?;
            budget.push_array_value(&value)?;
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn eval_object(
        &self,
        pairs: &[(Arc<str>, Expr)],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        let mut object = serde_json::Map::new();
        let mut budget = crate::limits::AggregateValueBudget::new()?;
        for (key, expression) in pairs {
            frame.charge(key.len())?;
            let value = self.eval_with_frame(expression, context, frame)?;
            budget.insert_object_value(key, &value, object.get(key.as_ref()))?;
            object.insert(key.to_string(), value);
        }
        Ok(Value::Object(object))
    }

    // Keep numeric conversion/error temporaries out of every recursive dispatch frame.
    fn negate(&self, value: &Value) -> ExpressionResult<Value> {
        let number = value.as_number().ok_or_else(|| {
            ExpressionError::type_error("number", crate::value_utils::value_type_name(value))
        })?;
        if number.is_f64() {
            return crate::value_utils::finite_result(-self.number_to_f64(number)?, "negation");
        }
        let integer = crate::value_utils::integer_value(number).ok_or_else(|| {
            ExpressionError::eval_error("Integer cannot be represented in the JSON integer range")
        })?;
        crate::value_utils::integer_result(integer.checked_neg(), "negation")
    }

    /// Evaluate a binary operation
    #[inline]
    fn eval_binary_op(
        &self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        // Short-circuit evaluation for logical operators
        match op {
            BinaryOp::And => {
                let left_val = self.eval_borrowed_with_frame(left, context, frame)?;
                if !self.coerce_boolean(&left_val, context)? {
                    // Short-circuit: if left is false, don't evaluate right
                    return Ok(Value::Bool(false));
                }
                let right_val = self.eval_borrowed_with_frame(right, context, frame)?;
                Ok(Value::Bool(self.coerce_boolean(&right_val, context)?))
            },
            BinaryOp::Or => {
                let left_val = self.eval_borrowed_with_frame(left, context, frame)?;
                if self.coerce_boolean(&left_val, context)? {
                    // Short-circuit: if left is true, don't evaluate right
                    return Ok(Value::Bool(true));
                }
                let right_val = self.eval_borrowed_with_frame(right, context, frame)?;
                Ok(Value::Bool(self.coerce_boolean(&right_val, context)?))
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
                        let equal = match (left_val.as_ref(), right_val.as_ref()) {
                            (Value::Number(left), Value::Number(right)) => {
                                crate::value_utils::compare_numbers(left, right)
                                    == Some(std::cmp::Ordering::Equal)
                            },
                            _ => left_val == right_val,
                        };
                        Ok(Value::Bool(if op == BinaryOp::Equal {
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
    fn add(&self, left: &Value, right: &Value, frame: &EvalFrame) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                if let (Some(left), Some(right)) = (
                    crate::value_utils::integer_value(l),
                    crate::value_utils::integer_value(r),
                ) {
                    crate::value_utils::integer_result(left.checked_add(right), "addition")
                } else {
                    let lf = self.number_to_f64(l)?;
                    let rf = self.number_to_f64(r)?;
                    crate::value_utils::finite_result(lf + rf, "addition")
                }
            },
            (Value::String(l), Value::String(r)) => {
                crate::limits::check_limit(
                    "string output bytes",
                    l.len().saturating_add(r.len()),
                    crate::limits::MAX_RESULT_BYTES,
                )?;
                frame.charge(l.len() + r.len())?;
                // Pre-allocate exact capacity to avoid reallocations
                let mut result = String::with_capacity(l.len() + r.len());
                result.push_str(l);
                result.push_str(r);
                Ok(Value::String(result))
            },
            _ => Err(ExpressionError::expression_type_error(
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
    fn subtract(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                if let (Some(left), Some(right)) = (
                    crate::value_utils::integer_value(l),
                    crate::value_utils::integer_value(r),
                ) {
                    crate::value_utils::integer_result(left.checked_sub(right), "subtraction")
                } else {
                    let lf = self.number_to_f64(l)?;
                    let rf = self.number_to_f64(r)?;
                    crate::value_utils::finite_result(lf - rf, "subtraction")
                }
            },
            _ => Err(ExpressionError::expression_type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            )),
        }
    }

    /// Multiplication
    #[inline]
    fn multiply(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                if let (Some(left), Some(right)) = (
                    crate::value_utils::integer_value(l),
                    crate::value_utils::integer_value(r),
                ) {
                    crate::value_utils::integer_result(left.checked_mul(right), "multiplication")
                } else {
                    let lf = self.number_to_f64(l)?;
                    let rf = self.number_to_f64(r)?;
                    crate::value_utils::finite_result(lf * rf, "multiplication")
                }
            },
            _ => Err(ExpressionError::expression_type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            )),
        }
    }

    /// Division
    #[inline]
    fn divide(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                // Always use floating point for division
                let lf = self.number_to_f64(l)?;
                let rf = self.number_to_f64(r)?;

                if rf == 0.0 {
                    return Err(ExpressionError::expression_division_by_zero());
                }
                // Reject non-finite divisor (NaN, ±∞). `serde_json::json!(NaN)`
                // silently converts to `Value::Null`, which would surface as
                // `1 / NaN = null` instead of an error.
                if !rf.is_finite() {
                    return Err(ExpressionError::expression_eval_error(
                        "division by non-finite number",
                    ));
                }

                let result = lf / rf;
                if !result.is_finite() {
                    return Err(ExpressionError::expression_eval_error(
                        "division produced a non-finite result",
                    ));
                }
                Ok(serde_json::json!(result))
            },
            _ => Err(ExpressionError::expression_type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            )),
        }
    }

    /// Modulo
    #[inline]
    fn modulo(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                // Try integer modulo first
                if let (Some(li), Some(ri)) = (
                    crate::value_utils::integer_value(l),
                    crate::value_utils::integer_value(r),
                ) {
                    if ri == 0 {
                        return Err(ExpressionError::expression_division_by_zero());
                    }
                    crate::value_utils::integer_result(li.checked_rem(ri), "remainder")
                } else {
                    // Fall back to float modulo
                    let lf = self.number_to_f64(l)?;
                    let rf = self.number_to_f64(r)?;
                    if rf == 0.0 {
                        return Err(ExpressionError::expression_division_by_zero());
                    }
                    crate::value_utils::finite_result(lf % rf, "remainder")
                }
            },
            _ => Err(ExpressionError::expression_type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            )),
        }
    }

    /// Power
    #[inline]
    fn power(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                // Always use floating point for power operations
                let lf = self.number_to_f64(l)?;
                let rf = self.number_to_f64(r)?;
                let result = lf.powf(rf);
                crate::value_utils::finite_result(result, "power")
            },
            _ => Err(ExpressionError::expression_type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            )),
        }
    }

    /// Less than comparison
    #[inline]
    fn less_than(
        &self,
        left: &Value,
        right: &Value,
        context: &EvaluationContext,
    ) -> ExpressionResult<Value> {
        if self.strict_numeric_comparisons_enabled(context)
            && (!left.is_number() || !right.is_number())
        {
            return Err(ExpressionError::expression_type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            ));
        }
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                Ok(Value::Bool(self.number_ordering(l, r)?.is_lt()))
            },
            (Value::String(l), Value::String(r)) => Ok(Value::Bool(l < r)),
            _ => Err(ExpressionError::expression_type_error(
                "comparable values",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            )),
        }
    }

    /// Greater than comparison
    #[inline]
    fn greater_than(
        &self,
        left: &Value,
        right: &Value,
        context: &EvaluationContext,
    ) -> ExpressionResult<Value> {
        if self.strict_numeric_comparisons_enabled(context)
            && (!left.is_number() || !right.is_number())
        {
            return Err(ExpressionError::expression_type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            ));
        }
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                Ok(Value::Bool(self.number_ordering(l, r)?.is_gt()))
            },
            (Value::String(l), Value::String(r)) => Ok(Value::Bool(l > r)),
            _ => Err(ExpressionError::expression_type_error(
                "comparable values",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            )),
        }
    }

    /// Less than or equal comparison
    fn less_equal(
        &self,
        left: &Value,
        right: &Value,
        context: &EvaluationContext,
    ) -> ExpressionResult<Value> {
        if self.strict_numeric_comparisons_enabled(context)
            && (!left.is_number() || !right.is_number())
        {
            return Err(ExpressionError::expression_type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            ));
        }
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                Ok(Value::Bool(self.number_ordering(l, r)?.is_le()))
            },
            (Value::String(l), Value::String(r)) => Ok(Value::Bool(l <= r)),
            _ => Err(ExpressionError::expression_type_error(
                "comparable values",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            )),
        }
    }

    /// Greater than or equal comparison
    fn greater_equal(
        &self,
        left: &Value,
        right: &Value,
        context: &EvaluationContext,
    ) -> ExpressionResult<Value> {
        if self.strict_numeric_comparisons_enabled(context)
            && (!left.is_number() || !right.is_number())
        {
            return Err(ExpressionError::expression_type_error(
                "number",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            ));
        }
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                Ok(Value::Bool(self.number_ordering(l, r)?.is_ge()))
            },
            (Value::String(l), Value::String(r)) => Ok(Value::Bool(l >= r)),
            _ => Err(ExpressionError::expression_type_error(
                "comparable values",
                format!(
                    "{} and {}",
                    crate::value_utils::value_type_name(left),
                    crate::value_utils::value_type_name(right)
                ),
            )),
        }
    }

    /// Regex match with ReDoS protection
    ///
    /// Security measures:
    /// - Pattern length limit (MAX_REGEX_PATTERN_LEN)
    /// - Detection of potentially dangerous nested quantifiers
    /// - Cache size limit with eviction (MAX_REGEX_CACHE_SIZE)
    #[cfg(feature = "regex")]
    fn regex_match(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        let text = left.as_str().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "string",
                crate::value_utils::value_type_name(left),
            )
        })?;

        let pattern = right.as_str().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "string",
                crate::value_utils::value_type_name(right),
            )
        })?;

        // ReDoS protection: check pattern length
        if pattern.len() > MAX_REGEX_PATTERN_LEN {
            return Err(ExpressionError::expression_regex_error(format!(
                "Regex pattern too long: {} chars (max {})",
                pattern.len(),
                MAX_REGEX_PATTERN_LEN
            )));
        }

        // ReDoS protection: detect potentially dangerous patterns
        if Self::is_potentially_dangerous_regex(pattern) {
            return Err(ExpressionError::expression_regex_error(
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
                .map_err(|_| ExpressionError::expression_regex_error("Regex pattern is invalid"))?;
            let arc = Arc::new(compiled);
            self.regex_cache
                .insert(Arc::from(pattern), Arc::clone(&arc));
            arc
        };

        Ok(Value::Bool(regex.is_match(text)))
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
    fn regex_match(&self, _left: &Value, _right: &Value) -> ExpressionResult<Value> {
        Err(ExpressionError::expression_eval_error(
            "Regex matching is not enabled (feature 'regex' not enabled)",
        ))
    }

    /// Access a property of an object.
    fn access_property<'a>(
        &self,
        object: EvalValue<'a>,
        property: &str,
    ) -> ExpressionResult<EvalValue<'a>> {
        let missing =
            || ExpressionError::expression_eval_error(format!("Property '{property}' not found"));
        match object {
            Cow::Borrowed(Value::Object(entries)) => {
                entries.get(property).map(Cow::Borrowed).ok_or_else(missing)
            },
            Cow::Owned(Value::Object(entries)) => entries
                .get(property)
                .cloned()
                .map(Cow::Owned)
                .ok_or_else(missing),
            Cow::Borrowed(other) => Err(ExpressionError::expression_type_error(
                "object",
                crate::value_utils::value_type_name(other),
            )),
            Cow::Owned(other) => Err(ExpressionError::expression_type_error(
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
            return Err(ExpressionError::expression_index_out_of_bounds(
                actual as usize,
                len,
            ));
        }
        Ok(actual as usize)
    }

    /// Access an element of an array or object by index.
    fn access_index<'a>(
        &self,
        object: EvalValue<'a>,
        index: &Value,
    ) -> ExpressionResult<EvalValue<'a>> {
        let missing_object = || ExpressionError::expression_eval_error("Object key not found");
        match object {
            Cow::Borrowed(Value::Array(array)) => {
                let pos = Self::resolve_array_index(index, array.len())?;
                array.get(pos).map(Cow::Borrowed).ok_or_else(|| {
                    ExpressionError::expression_index_out_of_bounds(pos, array.len())
                })
            },
            Cow::Owned(Value::Array(array)) => {
                let pos = Self::resolve_array_index(index, array.len())?;
                array.get(pos).cloned().map(Cow::Owned).ok_or_else(|| {
                    ExpressionError::expression_index_out_of_bounds(pos, array.len())
                })
            },
            Cow::Borrowed(Value::Object(entries)) => entries
                .get(index.as_str().ok_or_else(|| {
                    ExpressionError::expression_type_error(
                        "string",
                        crate::value_utils::value_type_name(index),
                    )
                })?)
                .map(Cow::Borrowed)
                .ok_or_else(missing_object),
            Cow::Owned(Value::Object(entries)) => entries
                .get(index.as_str().ok_or_else(|| {
                    ExpressionError::expression_type_error(
                        "string",
                        crate::value_utils::value_type_name(index),
                    )
                })?)
                .cloned()
                .map(Cow::Owned)
                .ok_or_else(missing_object),
            Cow::Borrowed(other) => Err(ExpressionError::expression_type_error(
                "array or object",
                crate::value_utils::value_type_name(other),
            )),
            Cow::Owned(other) => Err(ExpressionError::expression_type_error(
                "array or object",
                crate::value_utils::value_type_name(&other),
            )),
        }
    }

    /// Resolve an array index value (integer, allowing negative offsets) into
    /// a `0..len` position.
    fn resolve_array_index(index: &Value, len: usize) -> ExpressionResult<usize> {
        let idx = index.as_i64().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "integer",
                crate::value_utils::value_type_name(index),
            )
        })?;
        Self::normalize_index(idx, len)
    }

    /// Call a builtin function
    fn call_function(
        &self,
        name: &str,
        args: &[EvalValue<'_>],
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<Value> {
        self.ensure_function_allowed(name, context)?;
        for argument in args {
            frame.charge_value(argument)?;
        }
        let arguments = args.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        let result =
            self.builtins
                .call(name, &arguments, BuiltinView::new(self, frame), context)?;
        let result = crate::BuiltinOutputBuilder::new(self.builtin_output_limits(context))
            .value(result)?
            .into_value();
        frame.charge_value(&result)?;
        Ok(result)
    }

    /// Evaluate a lambda expression with a parameter value.
    ///
    /// Visibility is `pub(crate)` — external callers cannot construct
    /// an [`EvalFrame`], and exposing a wrapper that would create a
    /// fresh frame would reopen the CO-C1-01 lambda DoS bypass.
    pub(crate) fn eval_lambda(
        &self,
        param: &str,
        body: &Expr,
        value: &Value,
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        // Create a new context with the lambda parameter. Note: we
        // reuse the caller's `frame` so the step budget accumulates
        // across every lambda application. Do NOT switch this to
        // `self.eval(...)` — doing so would construct a fresh frame
        // and defeat the whole budget.
        let mut lambda_context = context.clone();
        lambda_context.set_lambda_var(param, value.clone());
        self.eval_with_frame(body, &lambda_context, frame)
    }

    /// Handle higher-order functions that require lambda expressions.
    /// Returns Some(result) if the function was handled, None if it should
    /// be passed to the regular builtin registry.
    fn try_higher_order_function(
        &self,
        name: &str,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> Option<ExpressionResult<Value>> {
        if let Err(err) = self.ensure_function_allowed(name, context) {
            return Some(Err(err));
        }

        match name {
            "filter" => Some(self.eval_filter(args, context, frame)),
            "map" => Some(self.eval_map(args, context, frame)),
            "reduce" => Some(self.eval_reduce(args, context, frame)),
            "find" => Some(self.eval_find(args, context, frame)),
            "find_index" => Some(self.eval_find_index(args, context, frame)),
            "every" | "all" => Some(self.eval_every(args, context, frame)),
            "some" | "any" => Some(self.eval_some(args, context, frame)),
            "group_by" => Some(self.eval_group_by(args, context, frame)),
            "flat_map" => Some(self.eval_flat_map(args, context, frame)),
            _ => None,
        }
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
                return Err(ExpressionError::expression_eval_error(format!(
                    "Function '{name}' is denied by policy"
                )));
            }
        }

        for policy in policies.into_iter().flatten() {
            if self.is_allowed_by_policy(policy, name, canonical) {
                continue;
            }
            return Err(ExpressionError::expression_eval_error(format!(
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

    fn coerce_boolean(&self, value: &Value, context: &EvaluationContext) -> ExpressionResult<bool> {
        if self.strict_mode_enabled(context) && !value.is_boolean() {
            return Err(ExpressionError::expression_type_error(
                "boolean",
                crate::value_utils::value_type_name(value),
            ));
        }
        Ok(crate::value_utils::to_boolean(value))
    }

    fn number_to_f64(&self, num: &Number) -> ExpressionResult<f64> {
        crate::value_utils::number_as_f64(num).ok_or_else(|| {
            ExpressionError::expression_eval_error("Number cannot be represented as float")
        })
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

    fn number_ordering(
        &self,
        left: &Number,
        right: &Number,
    ) -> ExpressionResult<std::cmp::Ordering> {
        crate::value_utils::compare_numbers(left, right).ok_or(ExpressionError::NonFiniteNumber {
            operation: "comparison",
        })
    }

    /// Filter array elements using a lambda predicate
    ///
    /// Usage: `filter(array, x => condition)`
    /// Example: `filter([1, 2, 3, 4, 5], x => x > 2)` returns `[3, 4, 5]`
    fn eval_filter(
        &self,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "filter",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        // Evaluate the array argument
        let array_val = self.eval_with_frame(&args[0], context, frame)?;
        let array = array_val.as_array().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "array",
                crate::value_utils::value_type_name(&array_val),
            )
        })?;

        // Extract the lambda
        let (param, body) = match &args[1] {
            Expr::Lambda { param, body } => (param.as_ref(), body.as_ref()),
            _ => {
                return Err(ExpressionError::expression_type_error(
                    "lambda expression",
                    "non-lambda",
                ));
            },
        };

        // Filter the array
        let mut result = Vec::with_capacity(array.len());
        for item in array {
            let predicate_result = self.eval_lambda(param, body, item, context, frame)?;
            if self.coerce_boolean(&predicate_result, context)? {
                result.push(item.clone());
            }
        }

        Ok(Value::Array(result))
    }

    /// Map over array elements using a lambda transformer
    ///
    /// Usage: `map(array, x => transform)`
    /// Example: `map([1, 2, 3], x => x * 2)` returns `[2, 4, 6]`
    fn eval_map(
        &self,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "map",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        // Evaluate the array argument
        let array_val = self.eval_with_frame(&args[0], context, frame)?;
        let array = array_val.as_array().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "array",
                crate::value_utils::value_type_name(&array_val),
            )
        })?;

        // Extract the lambda
        let (param, body) = match &args[1] {
            Expr::Lambda { param, body } => (param.as_ref(), body.as_ref()),
            _ => {
                return Err(ExpressionError::expression_type_error(
                    "lambda expression",
                    "non-lambda",
                ));
            },
        };

        // Map the array
        let output = crate::BuiltinOutputBuilder::new(self.builtin_output_limits(context));
        output.ensure_collection_items(array.len())?;
        let mut budget = crate::builtins::ArrayOutputBudget::new(output)?;
        let mut result = Vec::with_capacity(array.len());
        for item in array {
            let transformed = self.eval_lambda(param, body, item, context, frame)?;
            budget.push(&transformed)?;
            result.push(transformed);
        }

        Ok(Value::Array(result))
    }

    /// Reduce array elements using a lambda accumulator
    ///
    /// Usage: `reduce(array, initial, (acc, x) => expression)`
    /// Note: Since we only support single-parameter lambdas, we use a special syntax:
    /// `reduce(array, initial, x => expression)` where `$acc` is available in context
    ///
    /// Example: `reduce([1, 2, 3], 0, x => $acc + x)` returns `6`
    fn eval_reduce(
        &self,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        if args.len() != 3 {
            return Err(ExpressionError::expression_invalid_argument(
                "reduce",
                format!("expected 3 arguments, got {}", args.len()),
            ));
        }

        // Evaluate the array argument
        let array_val = self.eval_with_frame(&args[0], context, frame)?;
        let array = array_val.as_array().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "array",
                crate::value_utils::value_type_name(&array_val),
            )
        })?;

        // Evaluate the initial value
        let initial = self.eval_with_frame(&args[1], context, frame)?;

        // Extract the lambda
        let (param, body) = match &args[2] {
            Expr::Lambda { param, body } => (param.as_ref(), body.as_ref()),
            _ => {
                return Err(ExpressionError::expression_type_error(
                    "lambda expression",
                    "non-lambda",
                ));
            },
        };

        // Reduce the array. Each iteration reuses the caller's frame
        // so the step budget is enforced across every element — the
        // previous `self.eval(body, ...)` pattern reset the counter on
        // every element and was the CO-C1-01 DoS bypass.
        let mut accumulator = initial;
        for item in array {
            // Create context with both accumulator and current item
            let mut reduce_context = context.clone();
            reduce_context.set_lambda_var("$acc", accumulator.clone());
            reduce_context.set_lambda_var(param, item.clone());
            accumulator = self.eval_with_frame(body, &reduce_context, frame)?;
        }

        Ok(accumulator)
    }

    /// Find the first element matching a predicate
    ///
    /// Usage: `find(array, x => condition)`
    /// Example: `find([1, 2, 3, 4], x => x > 2)` returns `3`
    fn eval_find(
        &self,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "find",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        let array_val = self.eval_with_frame(&args[0], context, frame)?;
        let array = array_val.as_array().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "array",
                crate::value_utils::value_type_name(&array_val),
            )
        })?;

        let (param, body) = match &args[1] {
            Expr::Lambda { param, body } => (param.as_ref(), body.as_ref()),
            _ => {
                return Err(ExpressionError::expression_type_error(
                    "lambda expression",
                    "non-lambda",
                ));
            },
        };

        for item in array {
            let predicate_result = self.eval_lambda(param, body, item, context, frame)?;
            if self.coerce_boolean(&predicate_result, context)? {
                return Ok(item.clone());
            }
        }

        Ok(Value::Null)
    }

    /// Check if all elements match a predicate
    ///
    /// Usage: `every(array, x => condition)` or `all(array, x => condition)`
    /// Example: `every([2, 4, 6], x => x % 2 == 0)` returns `true`
    fn eval_every(
        &self,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "every",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        let array_val = self.eval_with_frame(&args[0], context, frame)?;
        let array = array_val.as_array().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "array",
                crate::value_utils::value_type_name(&array_val),
            )
        })?;

        let (param, body) = match &args[1] {
            Expr::Lambda { param, body } => (param.as_ref(), body.as_ref()),
            _ => {
                return Err(ExpressionError::expression_type_error(
                    "lambda expression",
                    "non-lambda",
                ));
            },
        };

        for item in array {
            let predicate_result = self.eval_lambda(param, body, item, context, frame)?;
            if !self.coerce_boolean(&predicate_result, context)? {
                return Ok(Value::Bool(false));
            }
        }

        Ok(Value::Bool(true))
    }

    /// Check if any element matches a predicate
    ///
    /// Usage: `some(array, x => condition)` or `any(array, x => condition)`
    /// Example: `some([1, 2, 3], x => x > 2)` returns `true`
    fn eval_some(
        &self,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "some",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        let array_val = self.eval_with_frame(&args[0], context, frame)?;
        let array = array_val.as_array().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "array",
                crate::value_utils::value_type_name(&array_val),
            )
        })?;

        let (param, body) = match &args[1] {
            Expr::Lambda { param, body } => (param.as_ref(), body.as_ref()),
            _ => {
                return Err(ExpressionError::expression_type_error(
                    "lambda expression",
                    "non-lambda",
                ));
            },
        };

        for item in array {
            let predicate_result = self.eval_lambda(param, body, item, context, frame)?;
            if self.coerce_boolean(&predicate_result, context)? {
                return Ok(Value::Bool(true));
            }
        }

        Ok(Value::Bool(false))
    }

    /// Return the index of the first element matching a predicate, or -1
    ///
    /// Usage: `find_index(array, x => condition)`
    /// Example: `find_index([1, 2, 3], x => x > 1)` returns `1`
    fn eval_find_index(
        &self,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "find_index",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        let array_val = self.eval_with_frame(&args[0], context, frame)?;
        let array = array_val.as_array().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "array",
                crate::value_utils::value_type_name(&array_val),
            )
        })?;

        let (param, body) = match &args[1] {
            Expr::Lambda { param, body } => (param.as_ref(), body.as_ref()),
            _ => {
                return Err(ExpressionError::expression_type_error(
                    "lambda expression",
                    "non-lambda",
                ));
            },
        };

        for (i, item) in array.iter().enumerate() {
            let predicate_result = self.eval_lambda(param, body, item, context, frame)?;
            if self.coerce_boolean(&predicate_result, context)? {
                return Ok(Value::Number((i as i64).into()));
            }
        }

        Ok(Value::Number((-1_i64).into()))
    }

    /// Group array elements by a key returned by a lambda
    ///
    /// Usage: `group_by(array, x => key_expr)`
    /// Example: `group_by([{name:"a",age:1},{name:"b",age:1}], x => x.age)`
    ///   returns `{"1": [{name:"a",age:1},{name:"b",age:1}]}`
    fn eval_group_by(
        &self,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "group_by",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        let array_val = self.eval_with_frame(&args[0], context, frame)?;
        let array = array_val.as_array().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "array",
                crate::value_utils::value_type_name(&array_val),
            )
        })?;

        let (param, body) = match &args[1] {
            Expr::Lambda { param, body } => (param.as_ref(), body.as_ref()),
            _ => {
                return Err(ExpressionError::expression_type_error(
                    "lambda expression",
                    "non-lambda",
                ));
            },
        };

        let output = crate::BuiltinOutputBuilder::new(self.builtin_output_limits(context));
        let mut budget = crate::builtins::GroupOutputBudget::new(output)?;
        let mut groups = serde_json::Map::new();
        for item in array {
            let key_val = self.eval_lambda(param, body, item, context, frame)?;
            let key = match &key_val {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => "null".to_string(),
                _ => {
                    return Err(ExpressionError::expression_eval_error(
                        "group_by key must be a string, number, boolean, or null",
                    ));
                },
            };
            let existing_items = groups
                .get(&key)
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            budget.push(&key, existing_items, item)?;
            let group_entry = groups
                .entry(key)
                .or_insert_with(|| Value::Array(Vec::new()));

            match group_entry {
                Value::Array(items) => items.push(item.clone()),
                other => {
                    return Err(ExpressionError::expression_type_error(
                        "array",
                        crate::value_utils::value_type_name(other),
                    ));
                },
            }
        }

        Ok(Value::Object(groups))
    }

    /// Map then flatten one level
    ///
    /// Usage: `flat_map(array, x => transform)`
    /// Example: `flat_map([[1,2],[3,4]], x => x)` returns `[1,2,3,4]`
    fn eval_flat_map(
        &self,
        args: &[Expr],
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "flat_map",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        let array_val = self.eval_with_frame(&args[0], context, frame)?;
        let array = array_val.as_array().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "array",
                crate::value_utils::value_type_name(&array_val),
            )
        })?;

        let (param, body) = match &args[1] {
            Expr::Lambda { param, body } => (param.as_ref(), body.as_ref()),
            _ => {
                return Err(ExpressionError::expression_type_error(
                    "lambda expression",
                    "non-lambda",
                ));
            },
        };

        let output = crate::BuiltinOutputBuilder::new(self.builtin_output_limits(context));
        let mut budget = crate::builtins::ArrayOutputBudget::new(output)?;
        let mut result = Vec::new();
        for item in array {
            let transformed = self.eval_lambda(param, body, item, context, frame)?;
            match transformed {
                Value::Array(inner) => {
                    for value in inner {
                        budget.push(&value)?;
                        result.push(value);
                    }
                },
                other => {
                    budget.push(&other)?;
                    result.push(other);
                },
            }
        }

        Ok(Value::Array(result))
    }
}

#[cfg(test)]
mod tests;
