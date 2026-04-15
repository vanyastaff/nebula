//! AST evaluation module
//!
//! This module implements the evaluation of parsed expression ASTs.

#[cfg(feature = "regex")]
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "regex")]
use parking_lot::Mutex;
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
    steps: usize,
    max_steps: Option<usize>,
}

impl EvalFrame {
    /// Create a fresh frame with the given step cap (snapshotted once
    /// from the effective policy at the top-level `eval` entry).
    #[inline]
    fn new(max_steps: Option<usize>) -> Self {
        Self {
            depth: 0,
            steps: 0,
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
    fn tick(&mut self) -> ExpressionResult<()> {
        self.steps += 1;
        if let Some(max) = self.max_steps
            && self.steps > max
        {
            return Err(ExpressionError::expression_eval_error(format!(
                "Maximum evaluation steps ({max}) exceeded",
            )));
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
            return Err(ExpressionError::expression_eval_error(format!(
                "Maximum recursion depth ({MAX_RECURSION_DEPTH}) exceeded",
            )));
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

/// Evaluator for expression ASTs
pub struct Evaluator {
    builtins: Arc<BuiltinRegistry>,
    policy: Option<Arc<EvaluationPolicy>>,
    /// Regex cache (pattern -> compiled Regex)
    /// Using Mutex for thread-safe interior mutability
    #[cfg(feature = "regex")]
    regex_cache: Mutex<HashMap<String, Regex>>,
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
            regex_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve the effective `max_eval_steps` for this `eval` call.
    ///
    /// Policy can be attached either on the evaluator itself or on the
    /// `EvaluationContext`; the context wins when both are set (matches
    /// the precedence already used by `ensure_function_allowed`).
    #[inline]
    fn resolve_max_steps(&self, context: &EvaluationContext) -> Option<usize> {
        context
            .policy()
            .and_then(EvaluationPolicy::max_eval_steps)
            .or_else(|| {
                self.policy
                    .as_deref()
                    .and_then(EvaluationPolicy::max_eval_steps)
            })
    }

    /// Evaluate an expression in the given context.
    ///
    /// This is the sole place where a fresh [`EvalFrame`] is constructed.
    /// All recursive paths inside the evaluator reuse the caller's frame
    /// via [`eval_with_frame`], so the step budget defined by
    /// [`EvaluationPolicy::max_eval_steps`] is enforced across ALL
    /// nested work — lambdas, reduces, pipelines, higher-order combinators.
    ///
    /// # CO-C1-01 footgun (builtins)
    ///
    /// `BuiltinRegistry::call` currently hands builtins `&Evaluator`
    /// without the caller's [`EvalFrame`]. A builtin that recurses by
    /// invoking `evaluator.eval(...)` will build a fresh frame and
    /// reset the step budget mid-traversal — which is exactly the DoS
    /// bypass this refactor closes for the intrinsic higher-order
    /// combinators (`map`, `filter`, `reduce`, ...).
    ///
    /// Today no shipping builtin does this, but before stabilising the
    /// public builtin API plumb `&mut EvalFrame` through
    /// `BuiltinRegistry::call` and add a pitfalls note under
    /// `.project/context/pitfalls.md`. See issue #252 / audit memory
    /// `pitfall_expression_builtin_frame`.
    #[inline]
    pub fn eval(&self, expr: &Expr, context: &EvaluationContext) -> ExpressionResult<Value> {
        let mut frame = EvalFrame::new(self.resolve_max_steps(context));
        self.eval_with_frame(expr, context, &mut frame)
    }

    /// Evaluate an expression using the caller's step/depth frame.
    ///
    /// Internal recursive paths MUST use this method — calling
    /// `self.eval(...)` from within the evaluator would construct a
    /// fresh frame mid-traversal and reset the step budget, reopening
    /// the CO-C1-01 lambda DoS bypass.
    #[inline]
    fn eval_with_frame(
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
            Expr::Literal(val) => Ok(val.clone()),

            Expr::Variable(name) => context
                .resolve_variable(name)
                .ok_or_else(|| ExpressionError::expression_variable_not_found(&**name)),

            Expr::Identifier(name) => {
                // Check if this identifier is a bound lambda parameter
                if let Some(val) = context.get_lambda_var(name) {
                    return Ok((*val).clone());
                }
                // Otherwise treat as a string constant
                Ok(Value::String(name.as_ref().to_string()))
            },

            Expr::Negate(expr) => {
                let val = self.eval_with_frame(expr, context, frame)?;
                match val {
                    Value::Number(ref n) => {
                        // Dispatch on the concrete representation: floats must never be
                        // routed through the i64 path (silent truncation of `-3.7` → `-3`),
                        // and i64 negation must be checked to surface `-(i64::MIN)` as a
                        // typed error instead of panicking in debug / wrapping in release.
                        if n.is_f64() {
                            let f = n.as_f64().ok_or_else(|| {
                                ExpressionError::expression_eval_error("Cannot negate number")
                            })?;
                            Ok(serde_json::json!(-f))
                        } else if let Some(i) = n.as_i64() {
                            let neg = i.checked_neg().ok_or_else(|| {
                                ExpressionError::expression_eval_error(
                                    "Integer overflow: cannot negate i64::MIN",
                                )
                            })?;
                            Ok(Value::Number(neg.into()))
                        } else if let Some(u) = n.as_u64() {
                            // u64 values above i64::MAX cannot be represented as negated i64.
                            let as_i: i64 = u.try_into().map_err(|_| {
                                ExpressionError::expression_eval_error(
                                    "Integer overflow: unsigned value exceeds i64 range",
                                )
                            })?;
                            let neg = as_i.checked_neg().ok_or_else(|| {
                                ExpressionError::expression_eval_error(
                                    "Integer overflow: cannot negate i64::MIN",
                                )
                            })?;
                            Ok(Value::Number(neg.into()))
                        } else {
                            Err(ExpressionError::expression_eval_error(
                                "Cannot negate number",
                            ))
                        }
                    },
                    _ => Err(ExpressionError::expression_type_error(
                        "number",
                        crate::value_utils::value_type_name(&val),
                    )),
                }
            },

            Expr::Not(expr) => {
                let val = self.eval_with_frame(expr, context, frame)?;
                Ok(Value::Bool(!self.coerce_boolean(&val, context)?))
            },

            Expr::Binary { left, op, right } => {
                self.eval_binary_op(*op, left, right, context, frame)
            },

            Expr::PropertyAccess { object, property } => {
                let obj_val = self.eval_with_frame(object, context, frame)?;
                self.access_property(&obj_val, property)
            },

            Expr::IndexAccess { object, index } => {
                let obj_val = self.eval_with_frame(object, context, frame)?;
                let index_val = self.eval_with_frame(index, context, frame)?;
                self.access_index(&obj_val, &index_val)
            },

            Expr::FunctionCall { name, args } => {
                // Try higher-order functions first (they need raw AST args for lambdas)
                if let Some(result) = self.try_higher_order_function(name, args, context, frame) {
                    return result;
                }

                // Regular function: evaluate all args to values
                let mut arg_values = Vec::with_capacity(args.len());
                for arg in args {
                    arg_values.push(self.eval_with_frame(arg, context, frame)?);
                }
                self.call_function(name, &arg_values, context, frame)
            },

            Expr::Pipeline {
                value,
                function,
                args,
            } => {
                // For higher-order functions in pipelines, prepend the value as first arg
                let mut full_args = Vec::with_capacity(1 + args.len());
                full_args.push(value.as_ref().clone());
                full_args.extend(args.iter().cloned());

                // Try higher-order functions first
                if let Some(result) =
                    self.try_higher_order_function(function, &full_args, context, frame)
                {
                    return result;
                }

                // Regular function: evaluate all args to values
                let val = self.eval_with_frame(value, context, frame)?;
                let mut arg_values: Vec<Value> = Vec::with_capacity(1 + args.len());
                arg_values.push(val);
                for arg in args {
                    arg_values.push(self.eval_with_frame(arg, context, frame)?);
                }
                self.call_function(function, &arg_values, context, frame)
            },

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

            Expr::Array(elements) => {
                let values: Result<Vec<_>, _> = elements
                    .iter()
                    .map(|e| self.eval_with_frame(e, context, frame))
                    .collect();
                let values = values?;
                Ok(Value::Array(values))
            },

            Expr::Object(pairs) => {
                let mut obj = serde_json::Map::new();
                for (key, expr) in pairs {
                    let value = self.eval_with_frame(expr, context, frame)?;
                    obj.insert(key.to_string(), value);
                }
                Ok(Value::Object(obj))
            },
        }
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
                let left_val = self.eval_with_frame(left, context, frame)?;
                if !self.coerce_boolean(&left_val, context)? {
                    // Short-circuit: if left is false, don't evaluate right
                    return Ok(Value::Bool(false));
                }
                let right_val = self.eval_with_frame(right, context, frame)?;
                Ok(Value::Bool(self.coerce_boolean(&right_val, context)?))
            },
            BinaryOp::Or => {
                let left_val = self.eval_with_frame(left, context, frame)?;
                if self.coerce_boolean(&left_val, context)? {
                    // Short-circuit: if left is true, don't evaluate right
                    return Ok(Value::Bool(true));
                }
                let right_val = self.eval_with_frame(right, context, frame)?;
                Ok(Value::Bool(self.coerce_boolean(&right_val, context)?))
            },
            // For all other operators, evaluate both operands
            _ => {
                let left_val = self.eval_with_frame(left, context, frame)?;
                let right_val = self.eval_with_frame(right, context, frame)?;

                match op {
                    BinaryOp::Add => self.add(&left_val, &right_val),
                    BinaryOp::Subtract => self.subtract(&left_val, &right_val),
                    BinaryOp::Multiply => self.multiply(&left_val, &right_val),
                    BinaryOp::Divide => self.divide(&left_val, &right_val),
                    BinaryOp::Modulo => self.modulo(&left_val, &right_val),
                    BinaryOp::Power => self.power(&left_val, &right_val),
                    BinaryOp::Equal => Ok(Value::Bool(left_val == right_val)),
                    BinaryOp::NotEqual => Ok(Value::Bool(left_val != right_val)),
                    BinaryOp::LessThan => self.less_than(&left_val, &right_val, context),
                    BinaryOp::GreaterThan => self.greater_than(&left_val, &right_val, context),
                    BinaryOp::LessEqual => self.less_equal(&left_val, &right_val, context),
                    BinaryOp::GreaterEqual => self.greater_equal(&left_val, &right_val, context),
                    BinaryOp::RegexMatch => self.regex_match(&left_val, &right_val),
                    BinaryOp::And | BinaryOp::Or => unreachable!(), // Handled above
                }
            },
        }
    }

    /// Addition
    #[inline]
    fn add(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                // Try integer addition with overflow checking; on overflow,
                // fall back to f64 arithmetic (lossy for values above 2^53).
                if let (Some(li), Some(ri)) = (l.as_i64(), r.as_i64()) {
                    let result = li.checked_add(ri).map_or_else(
                        || serde_json::json!(li as f64 + ri as f64),
                        |v| Value::Number(v.into()),
                    );
                    Ok(result)
                } else {
                    // At least one is float
                    let lf = self.number_to_f64(l)?;
                    let rf = self.number_to_f64(r)?;
                    Ok(serde_json::json!(lf + rf))
                }
            },
            (Value::String(l), Value::String(r)) => {
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
                // Try integer subtraction with overflow checking; on overflow
                // fall back to f64 (lossy above 2^53).
                if let (Some(li), Some(ri)) = (l.as_i64(), r.as_i64()) {
                    let result = li.checked_sub(ri).map_or_else(
                        || serde_json::json!(li as f64 - ri as f64),
                        |v| Value::Number(v.into()),
                    );
                    Ok(result)
                } else {
                    let lf = self.number_to_f64(l)?;
                    let rf = self.number_to_f64(r)?;
                    Ok(serde_json::json!(lf - rf))
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
                // Try integer multiplication with overflow checking; on
                // overflow fall back to f64 (lossy above 2^53).
                if let (Some(li), Some(ri)) = (l.as_i64(), r.as_i64()) {
                    let result = li.checked_mul(ri).map_or_else(
                        || serde_json::json!(li as f64 * ri as f64),
                        |v| Value::Number(v.into()),
                    );
                    Ok(result)
                } else {
                    let lf = self.number_to_f64(l)?;
                    let rf = self.number_to_f64(r)?;
                    Ok(serde_json::json!(lf * rf))
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
                if let (Some(li), Some(ri)) = (l.as_i64(), r.as_i64()) {
                    if ri == 0 {
                        return Err(ExpressionError::expression_division_by_zero());
                    }
                    Ok(Value::Number((li % ri).into()))
                } else {
                    // Fall back to float modulo
                    let lf = self.number_to_f64(l)?;
                    let rf = self.number_to_f64(r)?;
                    if rf == 0.0 {
                        return Err(ExpressionError::expression_division_by_zero());
                    }
                    Ok(serde_json::json!(lf % rf))
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
                Ok(serde_json::json!(lf.powf(rf)))
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
                let lf = self.number_to_f64(l)?;
                let rf = self.number_to_f64(r)?;
                Ok(Value::Bool(lf < rf))
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
                let lf = self.number_to_f64(l)?;
                let rf = self.number_to_f64(r)?;
                Ok(Value::Bool(lf > rf))
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
                let lf = self.number_to_f64(l)?;
                let rf = self.number_to_f64(r)?;
                Ok(Value::Bool(lf <= rf))
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
                let lf = self.number_to_f64(l)?;
                let rf = self.number_to_f64(r)?;
                Ok(Value::Bool(lf >= rf))
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

        // Try to get from cache first
        let mut cache = self.regex_cache.lock();
        let regex = if let Some(cached_regex) = cache.get(pattern) {
            // Cache hit - clone the Regex (cheap, Arc internally)
            cached_regex.clone()
        } else {
            // Cache miss - compile and cache
            let new_regex = Regex::new(pattern)
                .map_err(|e| ExpressionError::expression_regex_error(e.to_string()))?;

            // Enforce cache size limit with simple eviction
            if cache.len() >= MAX_REGEX_CACHE_SIZE {
                // Remove first entry (simple eviction strategy)
                if let Some(key) = cache.keys().next().cloned() {
                    cache.remove(&key);
                }
            }

            cache.insert(pattern.to_string(), new_regex.clone());
            new_regex
        };
        drop(cache); // Release lock before is_match

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
                let mut depth = 1;
                i += 1;

                // Find matching closing parenthesis
                while i < len && depth > 0 {
                    match chars[i] {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        '\\' => i += 1, // Skip escaped character
                        _ => {},
                    }
                    i += 1;
                }

                // Check if group is followed by a quantifier
                if i < len && (chars[i] == '+' || chars[i] == '*') {
                    // Check if the group contains a quantifier
                    let group_content: String = chars[group_start + 1..i - 1].iter().collect();
                    if group_content.contains('+')
                        || group_content.contains('*')
                        || group_content.contains('{')
                    {
                        // Nested quantifiers detected - potentially dangerous
                        return true;
                    }
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

    #[cfg(not(feature = "regex"))]
    fn regex_match(&self, _left: &Value, _right: &Value) -> ExpressionResult<Value> {
        Err(ExpressionError::expression_eval_error(
            "Regex matching is not enabled (feature 'regex' not enabled)",
        ))
    }

    /// Access a property of an object
    fn access_property(&self, obj: &Value, property: &str) -> ExpressionResult<Value> {
        match obj {
            Value::Object(o) => {
                let json_val = o.get(property).ok_or_else(|| {
                    ExpressionError::expression_eval_error(format!(
                        "Property '{}' not found",
                        property
                    ))
                })?;
                Ok(json_val.clone())
            },
            _ => Err(ExpressionError::expression_type_error(
                "object",
                crate::value_utils::value_type_name(obj),
            )),
        }
    }

    /// Access an element of an array or object by index
    fn access_index(&self, obj: &Value, index: &Value) -> ExpressionResult<Value> {
        match obj {
            Value::Array(arr) => {
                let idx = index.as_i64().ok_or_else(|| {
                    ExpressionError::expression_type_error(
                        "integer",
                        crate::value_utils::value_type_name(index),
                    )
                })?;
                let len = arr.len() as i64;
                let actual_idx = if idx < 0 { len + idx } else { idx };

                if actual_idx < 0 || actual_idx >= len {
                    return Err(ExpressionError::expression_index_out_of_bounds(
                        actual_idx as usize,
                        len as usize,
                    ));
                }

                let json_val = arr.get(actual_idx as usize).ok_or_else(|| {
                    ExpressionError::expression_index_out_of_bounds(
                        actual_idx as usize,
                        len as usize,
                    )
                })?;
                Ok(json_val.clone())
            },
            Value::Object(o) => {
                let key = index.as_str().ok_or_else(|| {
                    ExpressionError::expression_type_error(
                        "string",
                        crate::value_utils::value_type_name(index),
                    )
                })?;
                let json_val = o.get(key).ok_or_else(|| {
                    ExpressionError::expression_eval_error(format!("Key '{}' not found", key))
                })?;
                Ok(json_val.clone())
            },
            _ => Err(ExpressionError::expression_type_error(
                "array or object",
                crate::value_utils::value_type_name(obj),
            )),
        }
    }

    /// Call a builtin function
    fn call_function(
        &self,
        name: &str,
        args: &[Value],
        context: &EvaluationContext,
        _frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        self.ensure_function_allowed(name, context)?;
        self.builtins.call(name, args, self, context)
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
                    "Function '{}' is denied by policy",
                    name
                )));
            }
        }

        for policy in policies.into_iter().flatten() {
            if self.is_allowed_by_policy(policy, name, canonical) {
                continue;
            }
            return Err(ExpressionError::expression_eval_error(format!(
                "Function '{}' is not allowed by policy",
                name
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
        context
            .policy()
            .and_then(EvaluationPolicy::max_json_parse_length)
            .or_else(|| {
                self.policy
                    .as_deref()
                    .and_then(EvaluationPolicy::max_json_parse_length)
            })
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
        for item in array.iter() {
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
        let mut result = Vec::with_capacity(array.len());
        for item in array.iter() {
            let transformed = self.eval_lambda(param, body, item, context, frame)?;
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
        for item in array.iter() {
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

        for item in array.iter() {
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

        for item in array.iter() {
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

        for item in array.iter() {
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

        let mut groups = serde_json::Map::new();
        for item in array.iter() {
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

        let mut result = Vec::new();
        for item in array.iter() {
            let transformed = self.eval_lambda(param, body, item, context, frame)?;
            match transformed {
                Value::Array(inner) => result.extend(inner),
                other => result.push(other),
            }
        }

        Ok(Value::Array(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{builtins::BuiltinRegistry, policy::EvaluationPolicy};

    fn create_evaluator() -> Evaluator {
        let registry = Arc::new(BuiltinRegistry::new());
        Evaluator::new(registry)
    }

    fn create_evaluator_with_allowlist(functions: &[&str]) -> Evaluator {
        let registry = Arc::new(BuiltinRegistry::new());
        let policy = EvaluationPolicy::allow_only(functions.iter().copied());
        Evaluator::with_policy(registry, Some(Arc::new(policy)))
    }

    #[test]
    fn test_eval_literal() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();
        let expr = Expr::Literal(Value::Number(42.into()));
        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn test_eval_arithmetic() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::Number(10.into()))),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Value::Number(5.into()))),
        };
        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_i64(), Some(15));
    }

    #[test]
    fn test_deep_nesting_within_limit() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // Create moderately nested expression (safe for both construction and evaluation)
        let mut expr = Expr::Literal(Value::Number(1.into()));
        for _ in 0..50 {
            // 50 levels is safe and tests recursion tracking works
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::Add,
                right: Box::new(Expr::Literal(Value::Number(1.into()))),
            };
        }

        // Should succeed (50 << 256)
        let result = evaluator.eval(&expr, &context);
        assert!(result.is_ok(), "50-level deep expression should succeed");
        assert_eq!(result.unwrap().as_i64(), Some(51));
    }

    #[test]
    fn test_short_circuit_and_false() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // false && <anything> should short-circuit and not evaluate right side
        // Using a division by zero on the right to prove it's not evaluated
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::Bool(false))),
            op: BinaryOp::And,
            right: Box::new(Expr::Binary {
                left: Box::new(Expr::Literal(Value::Number(1.into()))),
                op: BinaryOp::Divide,
                right: Box::new(Expr::Literal(Value::Number(0.into()))),
            }),
        };

        // Should succeed without dividing by zero (short-circuit)
        let result = evaluator.eval(&expr, &context);
        assert!(
            result.is_ok(),
            "Short-circuit should prevent division by zero"
        );
        assert_eq!(result.unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_short_circuit_or_true() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // true || <anything> should short-circuit and not evaluate right side
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::Bool(true))),
            op: BinaryOp::Or,
            right: Box::new(Expr::Binary {
                left: Box::new(Expr::Literal(Value::Number(1.into()))),
                op: BinaryOp::Divide,
                right: Box::new(Expr::Literal(Value::Number(0.into()))),
            }),
        };

        // Should succeed without dividing by zero (short-circuit)
        let result = evaluator.eval(&expr, &context);
        assert!(
            result.is_ok(),
            "Short-circuit should prevent division by zero"
        );
        assert_eq!(result.unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_and_evaluates_both_when_left_true() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // true && false should evaluate both
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::Bool(true))),
            op: BinaryOp::And,
            right: Box::new(Expr::Literal(Value::Bool(false))),
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_or_evaluates_both_when_left_false() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // false || true should evaluate both
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::Bool(false))),
            op: BinaryOp::Or,
            right: Box::new(Expr::Literal(Value::Bool(true))),
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_regex_caching() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // First regex match - should compile and cache
        let expr1 = Expr::Binary {
            left: Box::new(Expr::Literal(Value::String("hello world".to_string()))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::String("hello.*".to_string()))),
        };
        let result1 = evaluator.eval(&expr1, &context).unwrap();
        assert_eq!(result1.as_bool(), Some(true));

        // Second regex match with same pattern - should use cached regex
        let expr2 = Expr::Binary {
            left: Box::new(Expr::Literal(Value::String("hello universe".to_string()))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::String("hello.*".to_string()))),
        };
        let result2 = evaluator.eval(&expr2, &context).unwrap();
        assert_eq!(result2.as_bool(), Some(true));

        // Verify cache has the pattern
        assert_eq!(evaluator.regex_cache.lock().len(), 1);
        assert!(evaluator.regex_cache.lock().contains_key("hello.*"));
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_regex_no_match() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::String("goodbye world".to_string()))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::String("^hello".to_string()))),
        };
        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    // ReDoS protection tests

    #[test]
    #[cfg(feature = "regex")]
    fn test_redos_pattern_length_limit() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // Create a pattern that exceeds the maximum length
        let long_pattern = "a".repeat(MAX_REGEX_PATTERN_LEN + 1);

        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::String("test".to_string()))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::String(long_pattern))),
        };

        let result = evaluator.eval(&expr, &context);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("too long"));
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_redos_nested_quantifiers_plus_plus() {
        // Test pattern like (a+)+ which can cause catastrophic backtracking
        assert!(Evaluator::is_potentially_dangerous_regex("(a+)+"));
        assert!(Evaluator::is_potentially_dangerous_regex("(a+)+b"));
        assert!(Evaluator::is_potentially_dangerous_regex("^(a+)+$"));
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_redos_nested_quantifiers_star_star() {
        // Test pattern like (a*)* which can cause catastrophic backtracking
        assert!(Evaluator::is_potentially_dangerous_regex("(a*)*"));
        assert!(Evaluator::is_potentially_dangerous_regex("(.*)*"));
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_redos_nested_quantifiers_mixed() {
        // Test mixed quantifier patterns
        assert!(Evaluator::is_potentially_dangerous_regex("(a+)*"));
        assert!(Evaluator::is_potentially_dangerous_regex("(a*)+"));
        assert!(Evaluator::is_potentially_dangerous_regex("([a-z]+)*"));
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_redos_nested_quantifiers_with_braces() {
        // Test patterns with curly brace quantifiers
        assert!(Evaluator::is_potentially_dangerous_regex("(a{2,})+"));
        assert!(Evaluator::is_potentially_dangerous_regex("(a{1,5})*"));
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_redos_safe_patterns() {
        // These patterns should NOT be flagged as dangerous
        assert!(!Evaluator::is_potentially_dangerous_regex("hello.*"));
        assert!(!Evaluator::is_potentially_dangerous_regex("^[a-z]+$"));
        assert!(!Evaluator::is_potentially_dangerous_regex("\\d{3}-\\d{4}"));
        assert!(!Evaluator::is_potentially_dangerous_regex("(abc)+"));
        assert!(!Evaluator::is_potentially_dangerous_regex("a+b+c+"));
        assert!(!Evaluator::is_potentially_dangerous_regex("(foo|bar)+"));
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_redos_rejection_in_eval() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // This dangerous pattern should be rejected
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::String(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaa!".to_string(),
            ))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::String("(a+)+$".to_string()))),
        };

        let result = evaluator.eval(&expr, &context);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("nested quantifiers"));
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_regex_cache_size_limit() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // Fill the cache with many patterns
        for i in 0..MAX_REGEX_CACHE_SIZE + 10 {
            let pattern = format!("pattern_{}", i);
            let expr = Expr::Binary {
                left: Box::new(Expr::Literal(Value::String("test".to_string()))),
                op: BinaryOp::RegexMatch,
                right: Box::new(Expr::Literal(Value::String(pattern))),
            };
            let _ = evaluator.eval(&expr, &context);
        }

        // Cache should not exceed MAX_REGEX_CACHE_SIZE
        let cache_size = evaluator.regex_cache.lock().len();
        assert!(
            cache_size <= MAX_REGEX_CACHE_SIZE,
            "Cache size {} exceeds limit {}",
            cache_size,
            MAX_REGEX_CACHE_SIZE
        );
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_redos_escaped_characters() {
        // Escaped parentheses and quantifiers should not trigger false positives
        assert!(!Evaluator::is_potentially_dangerous_regex(r"\(a+\)+"));
        assert!(!Evaluator::is_potentially_dangerous_regex(r"\+\*"));
    }

    // Higher-order function tests

    #[test]
    fn test_filter_with_lambda() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // filter([1, 2, 3, 4, 5], x => x > 2) should return [3, 4, 5]
        let expr = Expr::FunctionCall {
            name: Arc::from("filter"),
            args: vec![
                Expr::Array(vec![
                    Expr::Literal(Value::Number(1.into())),
                    Expr::Literal(Value::Number(2.into())),
                    Expr::Literal(Value::Number(3.into())),
                    Expr::Literal(Value::Number(4.into())),
                    Expr::Literal(Value::Number(5.into())),
                ]),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("x"))),
                        op: BinaryOp::GreaterThan,
                        right: Box::new(Expr::Literal(Value::Number(2.into()))),
                    }),
                },
            ],
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.first().unwrap().as_i64(), Some(3));
        assert_eq!(arr.get(1).unwrap().as_i64(), Some(4));
        assert_eq!(arr.get(2).unwrap().as_i64(), Some(5));
    }

    #[test]
    fn test_map_with_lambda() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // map([1, 2, 3], x => x * 2) should return [2, 4, 6]
        let expr = Expr::FunctionCall {
            name: Arc::from("map"),
            args: vec![
                Expr::Array(vec![
                    Expr::Literal(Value::Number(1.into())),
                    Expr::Literal(Value::Number(2.into())),
                    Expr::Literal(Value::Number(3.into())),
                ]),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("x"))),
                        op: BinaryOp::Multiply,
                        right: Box::new(Expr::Literal(Value::Number(2.into()))),
                    }),
                },
            ],
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.first().unwrap().as_i64(), Some(2));
        assert_eq!(arr.get(1).unwrap().as_i64(), Some(4));
        assert_eq!(arr.get(2).unwrap().as_i64(), Some(6));
    }

    #[test]
    fn test_reduce_with_lambda() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // reduce([1, 2, 3], 0, x => $acc + x) should return 6
        let expr = Expr::FunctionCall {
            name: Arc::from("reduce"),
            args: vec![
                Expr::Array(vec![
                    Expr::Literal(Value::Number(1.into())),
                    Expr::Literal(Value::Number(2.into())),
                    Expr::Literal(Value::Number(3.into())),
                ]),
                Expr::Literal(Value::Number(0.into())),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("$acc"))),
                        op: BinaryOp::Add,
                        right: Box::new(Expr::Variable(Arc::from("x"))),
                    }),
                },
            ],
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_i64(), Some(6));
    }

    #[test]
    fn test_find_with_lambda() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // find([1, 2, 3, 4], x => x > 2) should return 3
        let expr = Expr::FunctionCall {
            name: Arc::from("find"),
            args: vec![
                Expr::Array(vec![
                    Expr::Literal(Value::Number(1.into())),
                    Expr::Literal(Value::Number(2.into())),
                    Expr::Literal(Value::Number(3.into())),
                    Expr::Literal(Value::Number(4.into())),
                ]),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("x"))),
                        op: BinaryOp::GreaterThan,
                        right: Box::new(Expr::Literal(Value::Number(2.into()))),
                    }),
                },
            ],
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_i64(), Some(3));
    }

    #[test]
    fn test_every_with_lambda() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // every([2, 4, 6], x => x % 2 == 0) should return true
        let expr = Expr::FunctionCall {
            name: Arc::from("every"),
            args: vec![
                Expr::Array(vec![
                    Expr::Literal(Value::Number(2.into())),
                    Expr::Literal(Value::Number(4.into())),
                    Expr::Literal(Value::Number(6.into())),
                ]),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Binary {
                            left: Box::new(Expr::Variable(Arc::from("x"))),
                            op: BinaryOp::Modulo,
                            right: Box::new(Expr::Literal(Value::Number(2.into()))),
                        }),
                        op: BinaryOp::Equal,
                        right: Box::new(Expr::Literal(Value::Number(0.into()))),
                    }),
                },
            ],
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_some_with_lambda() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // some([1, 2, 3], x => x > 2) should return true
        let expr = Expr::FunctionCall {
            name: Arc::from("some"),
            args: vec![
                Expr::Array(vec![
                    Expr::Literal(Value::Number(1.into())),
                    Expr::Literal(Value::Number(2.into())),
                    Expr::Literal(Value::Number(3.into())),
                ]),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("x"))),
                        op: BinaryOp::GreaterThan,
                        right: Box::new(Expr::Literal(Value::Number(2.into()))),
                    }),
                },
            ],
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_bool(), Some(true));

        // some([1, 2, 3], x => x > 5) should return false
        let expr2 = Expr::FunctionCall {
            name: Arc::from("some"),
            args: vec![
                Expr::Array(vec![
                    Expr::Literal(Value::Number(1.into())),
                    Expr::Literal(Value::Number(2.into())),
                    Expr::Literal(Value::Number(3.into())),
                ]),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("x"))),
                        op: BinaryOp::GreaterThan,
                        right: Box::new(Expr::Literal(Value::Number(5.into()))),
                    }),
                },
            ],
        };

        let result2 = evaluator.eval(&expr2, &context).unwrap();
        assert_eq!(result2.as_bool(), Some(false));
    }

    #[test]
    fn test_allowlist_alias_for_higher_order_function() {
        let evaluator = create_evaluator_with_allowlist(&["all"]);
        let context = EvaluationContext::new();

        let expr = Expr::FunctionCall {
            name: Arc::from("every"),
            args: vec![
                Expr::Array(vec![
                    Expr::Literal(Value::Number(2.into())),
                    Expr::Literal(Value::Number(4.into())),
                    Expr::Literal(Value::Number(6.into())),
                ]),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Binary {
                            left: Box::new(Expr::Variable(Arc::from("x"))),
                            op: BinaryOp::Modulo,
                            right: Box::new(Expr::Literal(Value::Number(2.into()))),
                        }),
                        op: BinaryOp::Equal,
                        right: Box::new(Expr::Literal(Value::Number(0.into()))),
                    }),
                },
            ],
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    // ────────────────────────────────────────────────────────────────
    // CO-C1-01 / issue #252 regression guards — step-budget enforcement
    // across higher-order combinators, with thread-safety under a
    // shared Arc<Evaluator>.
    // ────────────────────────────────────────────────────────────────

    /// Build an `Evaluator` with a hard step budget.
    fn create_evaluator_with_step_budget(max_steps: usize) -> Evaluator {
        let registry = Arc::new(BuiltinRegistry::new());
        let policy = EvaluationPolicy::new().with_max_eval_steps(max_steps);
        Evaluator::with_policy(registry, Some(Arc::new(policy)))
    }

    /// Build a literal array `[0, 1, ..., n-1]`.
    fn literal_array(n: usize) -> Expr {
        Expr::Array(
            (0..n)
                .map(|i| Expr::Literal(Value::Number((i as i64).into())))
                .collect(),
        )
    }

    /// `x => x + 1` — one lambda body evaluation = ~3 steps (binary op +
    /// two operands). Used as a cheap predicate that nonetheless multiplies
    /// out under higher-order traversal.
    fn increment_lambda() -> Expr {
        Expr::Lambda {
            param: Arc::from("x"),
            body: Box::new(Expr::Binary {
                left: Box::new(Expr::Variable(Arc::from("x"))),
                op: BinaryOp::Add,
                right: Box::new(Expr::Literal(Value::Number(1.into()))),
            }),
        }
    }

    #[test]
    fn step_budget_bounds_linear_expression() {
        // Sanity check: a trivial top-level expression still fires the
        // step-limit error when the cap is tight.
        let evaluator = create_evaluator_with_step_budget(2);
        let context = EvaluationContext::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Binary {
                left: Box::new(Expr::Literal(Value::Number(1.into()))),
                op: BinaryOp::Add,
                right: Box::new(Expr::Literal(Value::Number(2.into()))),
            }),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Value::Number(3.into()))),
        };
        let err = evaluator.eval(&expr, &context).unwrap_err();
        assert!(
            err.to_string().contains("Maximum evaluation steps"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn step_budget_bounds_map_over_large_array() {
        // Pre-fix: `eval_lambda` called `self.eval(...)` which reset the
        // step counter per element, so this test passed. After the fix
        // the lambda reuses the caller's frame and the cap stops the
        // traversal after a handful of elements.
        let evaluator = create_evaluator_with_step_budget(50);
        let context = EvaluationContext::new();
        let expr = Expr::FunctionCall {
            name: Arc::from("map"),
            args: vec![literal_array(1000), increment_lambda()],
        };
        let err = evaluator
            .eval(&expr, &context)
            .expect_err("map over 1000 elements must exceed a 50-step budget");
        assert!(
            err.to_string().contains("Maximum evaluation steps"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn step_budget_bounds_nested_higher_order() {
        // `map(arr, x => filter(arr2, y => y > x))` — nested lambdas
        // used to double-reset the counter. Now the budget is honoured
        // across the whole traversal.
        let evaluator = create_evaluator_with_step_budget(80);
        let context = EvaluationContext::new();
        let inner_filter = Expr::FunctionCall {
            name: Arc::from("filter"),
            args: vec![
                literal_array(20),
                Expr::Lambda {
                    param: Arc::from("y"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("y"))),
                        op: BinaryOp::GreaterThan,
                        right: Box::new(Expr::Variable(Arc::from("x"))),
                    }),
                },
            ],
        };
        let expr = Expr::FunctionCall {
            name: Arc::from("map"),
            args: vec![
                literal_array(20),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(inner_filter),
                },
            ],
        };
        let err = evaluator
            .eval(&expr, &context)
            .expect_err("nested map/filter must exceed an 80-step budget");
        assert!(err.to_string().contains("Maximum evaluation steps"));
    }

    #[test]
    fn step_budget_bounds_reduce_across_iterations() {
        // `reduce` clones the context per iteration — before the fix
        // the clone carried a reset step counter. Afterwards, each
        // iteration reuses the caller's frame.
        let evaluator = create_evaluator_with_step_budget(30);
        let context = EvaluationContext::new();
        let expr = Expr::FunctionCall {
            name: Arc::from("reduce"),
            args: vec![
                literal_array(100),
                Expr::Literal(Value::Number(0.into())),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("$acc"))),
                        op: BinaryOp::Add,
                        right: Box::new(Expr::Variable(Arc::from("x"))),
                    }),
                },
            ],
        };
        let err = evaluator
            .eval(&expr, &context)
            .expect_err("reduce over 100 elements must exceed a 30-step budget");
        assert!(err.to_string().contains("Maximum evaluation steps"));
    }

    #[test]
    fn step_budget_bounds_flat_map() {
        let evaluator = create_evaluator_with_step_budget(40);
        let context = EvaluationContext::new();
        let expr = Expr::FunctionCall {
            name: Arc::from("flat_map"),
            args: vec![literal_array(200), increment_lambda()],
        };
        let err = evaluator
            .eval(&expr, &context)
            .expect_err("flat_map over 200 elements must exceed a 40-step budget");
        assert!(err.to_string().contains("Maximum evaluation steps"));
    }

    #[test]
    fn step_budget_bounds_group_by() {
        let evaluator = create_evaluator_with_step_budget(40);
        let context = EvaluationContext::new();
        let expr = Expr::FunctionCall {
            name: Arc::from("group_by"),
            args: vec![literal_array(200), increment_lambda()],
        };
        let err = evaluator
            .eval(&expr, &context)
            .expect_err("group_by over 200 elements must exceed a 40-step budget");
        assert!(err.to_string().contains("Maximum evaluation steps"));
    }

    #[test]
    fn step_budget_resets_between_successive_eval_calls() {
        // Guard against a future mistake that would move the step
        // counter onto `EvaluationContext` (or `Evaluator`) and have
        // it leak across top-level calls. Two back-to-back `eval`
        // calls on the same evaluator and context must each start
        // from a fresh budget.
        let evaluator = create_evaluator_with_step_budget(10);
        let context = EvaluationContext::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::Number(1.into()))),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Value::Number(2.into()))),
        };
        // First call: 3 steps, well under the cap.
        let r1 = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(r1.as_i64(), Some(3));
        // Second call: also starts at 0 steps, must also succeed.
        let r2 = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(r2.as_i64(), Some(3));
    }

    #[test]
    fn step_budget_respects_context_policy_when_evaluator_has_none() {
        // An evaluator with no policy can still be bounded via the
        // `EvaluationContext` builder's policy override.
        let evaluator = create_evaluator();
        let policy = EvaluationPolicy::new().with_max_eval_steps(5);
        let context = EvaluationContext::builder().policy(policy).build();
        let expr = Expr::FunctionCall {
            name: Arc::from("map"),
            args: vec![literal_array(100), increment_lambda()],
        };
        let err = evaluator
            .eval(&expr, &context)
            .expect_err("context-level budget of 5 must also bound a map over 100 elements");
        assert!(err.to_string().contains("Maximum evaluation steps"));
    }

    #[test]
    fn step_budget_error_path_does_not_leak_depth_into_next_call() {
        // A recursion-depth error on one `eval` call must not
        // contaminate the next call's depth tracking — each top-level
        // call builds a fresh `EvalFrame` on the caller's stack.
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // Build a deeply nested unary-not chain that exceeds MAX_RECURSION_DEPTH.
        let mut deep_expr = Expr::Literal(Value::Bool(true));
        for _ in 0..(MAX_RECURSION_DEPTH + 10) {
            deep_expr = Expr::Not(Box::new(deep_expr));
        }
        let err = evaluator.eval(&deep_expr, &context).unwrap_err();
        assert!(err.to_string().contains("Maximum recursion depth"));

        // Next call on the same evaluator must start fresh.
        let ok = evaluator
            .eval(&Expr::Literal(Value::Bool(true)), &context)
            .expect("fresh call after a depth error must succeed");
        assert_eq!(ok.as_bool(), Some(true));
    }

    #[test]
    fn step_budget_concurrent_arc_evaluator_is_independent_per_task() {
        // Thread-safety regression: N threads sharing a single
        // `Arc<Evaluator>` must each get their own stack-local
        // `EvalFrame`. Before the fix, all threads shared a single
        // `AtomicUsize` counter on the evaluator and could see
        // spurious "step limit exceeded" errors caused by another
        // thread's work.
        use std::thread;

        let evaluator = Arc::new(create_evaluator_with_step_budget(500));
        let expr = Arc::new(Expr::FunctionCall {
            name: Arc::from("map"),
            args: vec![literal_array(50), increment_lambda()],
        });

        let mut handles = Vec::new();
        for _ in 0..8 {
            let evaluator = Arc::clone(&evaluator);
            let expr = Arc::clone(&expr);
            handles.push(thread::spawn(move || {
                let context = EvaluationContext::new();
                // 50 elements × ~3-step body + overhead is well under
                // 500. Every thread should succeed. If the counters
                // were shared, some threads would see 500 exceeded.
                for _ in 0..10 {
                    evaluator
                        .eval(&expr, &context)
                        .expect("per-thread budget must be independent");
                }
            }));
        }
        for h in handles {
            h.join().expect("worker thread panicked");
        }
    }

    #[test]
    fn step_budget_bounds_reduce_nested_in_map() {
        // Reduce has its own per-iteration context clone path (the
        // `$acc` lambda var lives on a fresh clone per element). The
        // other nested test exercises map + filter; this one exercises
        // map-of-reduce so the reduce-specific clone cannot become a
        // hidden counter reset in future refactors.
        let evaluator = create_evaluator_with_step_budget(80);
        let context = EvaluationContext::new();
        let inner_reduce = Expr::FunctionCall {
            name: Arc::from("reduce"),
            args: vec![
                literal_array(10),
                Expr::Literal(Value::Number(0.into())),
                Expr::Lambda {
                    param: Arc::from("y"),
                    body: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("$acc"))),
                        op: BinaryOp::Add,
                        right: Box::new(Expr::Variable(Arc::from("y"))),
                    }),
                },
            ],
        };
        let expr = Expr::FunctionCall {
            name: Arc::from("map"),
            args: vec![
                literal_array(10),
                Expr::Lambda {
                    param: Arc::from("x"),
                    body: Box::new(inner_reduce),
                },
            ],
        };
        let err = evaluator
            .eval(&expr, &context)
            .expect_err("map-of-reduce over 10x10 must exceed an 80-step budget");
        assert!(err.to_string().contains("Maximum evaluation steps"));
    }

    #[test]
    fn step_budget_permissive_budget_still_completes_large_map() {
        // Smoke test that a reasonable budget does NOT spuriously
        // reject a realistic higher-order expression — guards against
        // off-by-one or arithmetic regressions in `tick`.
        let evaluator = create_evaluator_with_step_budget(10_000);
        let context = EvaluationContext::new();
        let expr = Expr::FunctionCall {
            name: Arc::from("map"),
            args: vec![literal_array(100), increment_lambda()],
        };
        let result = evaluator.eval(&expr, &context).unwrap();
        let arr = result.as_array().expect("map returns an array");
        assert_eq!(arr.len(), 100);
        assert_eq!(arr.first().and_then(|v| v.as_i64()), Some(1));
        assert_eq!(arr.last().and_then(|v| v.as_i64()), Some(100));
    }

    #[test]
    fn test_negate_integer() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();
        let expr = Expr::Negate(Box::new(Expr::Literal(Value::Number(42.into()))));
        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_i64(), Some(-42));
    }

    #[test]
    fn test_negate_float_preserves_fraction() {
        // Regression for #280: `-3.7` must NOT truncate to `-3`.
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();
        let expr = Expr::Negate(Box::new(Expr::Literal(serde_json::json!(3.7))));
        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_f64(), Some(-3.7));
    }

    #[test]
    fn test_negate_negative_float() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();
        let expr = Expr::Negate(Box::new(Expr::Literal(serde_json::json!(-2.5))));
        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_f64(), Some(2.5));
    }

    #[test]
    fn test_negate_i64_min_errors() {
        // Regression for #280: negating `i64::MIN` must surface a typed error,
        // not panic (debug) or silently wrap (release).
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();
        let expr = Expr::Negate(Box::new(Expr::Literal(Value::Number(i64::MIN.into()))));
        let err = evaluator.eval(&expr, &context).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.to_lowercase().contains("overflow"),
            "expected overflow error, got: {msg}"
        );
    }

    #[test]
    fn test_negate_u64_above_i64_max_errors() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();
        let big = (i64::MAX as u64) + 1;
        let expr = Expr::Negate(Box::new(Expr::Literal(Value::Number(big.into()))));
        let err = evaluator.eval(&expr, &context).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.to_lowercase().contains("overflow"),
            "expected overflow error, got: {msg}"
        );
    }

    #[test]
    fn test_negate_non_number_type_error() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();
        let expr = Expr::Negate(Box::new(Expr::Literal(Value::Bool(true))));
        let err = evaluator.eval(&expr, &context).unwrap_err();
        assert!(format!("{err}").to_lowercase().contains("type"));
    }
}
