//! AST evaluation module
//!
//! This module implements the evaluation of parsed expression ASTs.

use crate::ExpressionError;
use crate::builtins::BuiltinRegistry;
use crate::context::EvaluationContext;
use crate::core::ast::{BinaryOp, Expr};
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use parking_lot::Mutex;
#[cfg(feature = "regex")]
use regex::Regex;
use serde_json::Value;
#[cfg(feature = "regex")]
use std::collections::HashMap;
use std::sync::Arc;

/// Maximum recursion depth for expression evaluation
const MAX_RECURSION_DEPTH: usize = 256;

/// Maximum length for regex patterns to prevent ReDoS attacks
#[cfg(feature = "regex")]
const MAX_REGEX_PATTERN_LEN: usize = 1000;

/// Maximum number of cached regex patterns (simple LRU-style eviction)
#[cfg(feature = "regex")]
const MAX_REGEX_CACHE_SIZE: usize = 100;

/// Evaluator for expression ASTs
pub struct Evaluator {
    builtins: Arc<BuiltinRegistry>,
    /// Regex cache (pattern -> compiled Regex)
    /// Using Mutex for thread-safe interior mutability
    #[cfg(feature = "regex")]
    regex_cache: Mutex<HashMap<String, Regex>>,
}

impl Evaluator {
    /// Create a new evaluator with the given builtin registry
    pub fn new(builtins: Arc<BuiltinRegistry>) -> Self {
        Self {
            builtins,
            #[cfg(feature = "regex")]
            regex_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Evaluate an expression in the given context
    #[inline]
    pub fn eval(&self, expr: &Expr, context: &EvaluationContext) -> ExpressionResult<Value> {
        self.eval_with_depth(expr, context, 0)
    }

    /// Evaluate an expression with recursion depth tracking
    #[inline]
    fn eval_with_depth(
        &self,
        expr: &Expr,
        context: &EvaluationContext,
        depth: usize,
    ) -> ExpressionResult<Value> {
        // Check recursion depth limit
        if depth > MAX_RECURSION_DEPTH {
            return Err(ExpressionError::expression_eval_error(format!(
                "Maximum recursion depth ({}) exceeded",
                MAX_RECURSION_DEPTH
            )));
        }
        match expr {
            Expr::Literal(val) => Ok(val.clone()),

            Expr::Variable(name) => context
                .resolve_variable(name)
                .ok_or_else(|| ExpressionError::expression_variable_not_found(&**name)),

            Expr::Identifier(name) => {
                // Try to resolve as a constant or special value
                // Optimize: use Arc<str> directly instead of converting to String
                Ok(Value::String(name.as_ref().to_string()))
            }

            Expr::Negate(expr) => {
                let val = self.eval_with_depth(expr, context, depth + 1)?;
                match val {
                    Value::Number(ref n) => {
                        if let Some(i) = crate::value_utils::number_as_i64(n) {
                            Ok(Value::Number((-i).into()))
                        } else if let Some(f) = crate::value_utils::number_as_f64(n) {
                            Ok(serde_json::json!(-f))
                        } else {
                            Err(ExpressionError::expression_eval_error(
                                "Cannot negate number",
                            ))
                        }
                    }
                    _ => Err(ExpressionError::expression_type_error(
                        "number",
                        crate::value_utils::value_type_name(&val),
                    )),
                }
            }

            Expr::Not(expr) => {
                let val = self.eval_with_depth(expr, context, depth + 1)?;
                Ok(Value::Bool(!crate::value_utils::to_boolean(&val)))
            }

            Expr::Binary { left, op, right } => {
                self.eval_binary_op(*op, left, right, context, depth)
            }

            Expr::PropertyAccess { object, property } => {
                let obj_val = self.eval_with_depth(object, context, depth + 1)?;
                self.access_property(&obj_val, property)
            }

            Expr::IndexAccess { object, index } => {
                let obj_val = self.eval_with_depth(object, context, depth + 1)?;
                let index_val = self.eval_with_depth(index, context, depth + 1)?;
                self.access_index(&obj_val, &index_val)
            }

            Expr::FunctionCall { name, args } => {
                // Try higher-order functions first (they need raw AST args for lambdas)
                if let Some(result) = self.try_higher_order_function(name, args, context, depth) {
                    return result;
                }

                // Regular function: evaluate all args to values
                let mut arg_values = Vec::with_capacity(args.len());
                for arg in args {
                    arg_values.push(self.eval_with_depth(arg, context, depth + 1)?);
                }
                self.call_function(name, &arg_values, context, depth)
            }

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
                    self.try_higher_order_function(function, &full_args, context, depth)
                {
                    return result;
                }

                // Regular function: evaluate all args to values
                let val = self.eval_with_depth(value, context, depth + 1)?;
                let mut arg_values: Vec<Value> = Vec::with_capacity(1 + args.len());
                arg_values.push(val);
                for arg in args {
                    arg_values.push(self.eval_with_depth(arg, context, depth + 1)?);
                }
                self.call_function(function, &arg_values, context, depth)
            }

            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let cond_val = self.eval_with_depth(condition, context, depth + 1)?;
                if crate::value_utils::to_boolean(&cond_val) {
                    self.eval_with_depth(then_expr, context, depth + 1)
                } else {
                    self.eval_with_depth(else_expr, context, depth + 1)
                }
            }

            Expr::Lambda { .. } => {
                // Lambdas are handled specially in higher-order functions
                Err(ExpressionError::expression_eval_error(
                    "Lambda expressions can only be used as function arguments",
                ))
            }

            Expr::Array(elements) => {
                let values: Result<Vec<_>, _> = elements
                    .iter()
                    .map(|e| self.eval_with_depth(e, context, depth + 1))
                    .collect();
                let values = values?;
                Ok(Value::Array(values))
            }

            Expr::Object(pairs) => {
                let mut obj = serde_json::Map::new();
                for (key, expr) in pairs {
                    let value = self.eval_with_depth(expr, context, depth + 1)?;
                    obj.insert(key.to_string(), value);
                }
                Ok(Value::Object(obj))
            }
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
        depth: usize,
    ) -> ExpressionResult<Value> {
        // Short-circuit evaluation for logical operators
        match op {
            BinaryOp::And => {
                let left_val = self.eval_with_depth(left, context, depth + 1)?;
                if !crate::value_utils::to_boolean(&left_val) {
                    // Short-circuit: if left is false, don't evaluate right
                    return Ok(Value::Bool(false));
                }
                let right_val = self.eval_with_depth(right, context, depth + 1)?;
                Ok(Value::Bool(crate::value_utils::to_boolean(&right_val)))
            }
            BinaryOp::Or => {
                let left_val = self.eval_with_depth(left, context, depth + 1)?;
                if crate::value_utils::to_boolean(&left_val) {
                    // Short-circuit: if left is true, don't evaluate right
                    return Ok(Value::Bool(true));
                }
                let right_val = self.eval_with_depth(right, context, depth + 1)?;
                Ok(Value::Bool(crate::value_utils::to_boolean(&right_val)))
            }
            // For all other operators, evaluate both operands
            _ => {
                let left_val = self.eval_with_depth(left, context, depth + 1)?;
                let right_val = self.eval_with_depth(right, context, depth + 1)?;

                match op {
                    BinaryOp::Add => self.add(&left_val, &right_val),
                    BinaryOp::Subtract => self.subtract(&left_val, &right_val),
                    BinaryOp::Multiply => self.multiply(&left_val, &right_val),
                    BinaryOp::Divide => self.divide(&left_val, &right_val),
                    BinaryOp::Modulo => self.modulo(&left_val, &right_val),
                    BinaryOp::Power => self.power(&left_val, &right_val),
                    BinaryOp::Equal => Ok(Value::Bool(left_val == right_val)),
                    BinaryOp::NotEqual => Ok(Value::Bool(left_val != right_val)),
                    BinaryOp::LessThan => self.less_than(&left_val, &right_val),
                    BinaryOp::GreaterThan => self.greater_than(&left_val, &right_val),
                    BinaryOp::LessEqual => self.less_equal(&left_val, &right_val),
                    BinaryOp::GreaterEqual => self.greater_equal(&left_val, &right_val),
                    BinaryOp::RegexMatch => self.regex_match(&left_val, &right_val),
                    BinaryOp::And | BinaryOp::Or => unreachable!(), // Handled above
                }
            }
        }
    }

    /// Addition
    #[inline]
    fn add(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                // Try integer addition with overflow checking
                if let (Some(li), Some(ri)) = (l.as_i64(), r.as_i64()) {
                    li.checked_add(ri)
                        .map(|result| Value::Number(result.into()))
                        .or_else(|| {
                            // Overflow - fall back to float
                            Some(serde_json::json!(li as f64 + ri as f64))
                        })
                        .ok_or_else(|| {
                            ExpressionError::expression_eval_error("Arithmetic overflow")
                        })
                } else {
                    // At least one is float
                    let lf = crate::value_utils::number_as_f64(l).unwrap_or(0.0);
                    let rf = crate::value_utils::number_as_f64(r).unwrap_or(0.0);
                    Ok(serde_json::json!(lf + rf))
                }
            }
            (Value::String(l), Value::String(r)) => {
                // Pre-allocate exact capacity to avoid reallocations
                let mut result = String::with_capacity(l.len() + r.len());
                result.push_str(l);
                result.push_str(r);
                Ok(Value::String(result))
            }
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
                // Try integer subtraction with overflow checking
                if let (Some(li), Some(ri)) = (l.as_i64(), r.as_i64()) {
                    li.checked_sub(ri)
                        .map(|result| Value::Number(result.into()))
                        .or_else(|| Some(serde_json::json!(li as f64 - ri as f64)))
                        .ok_or_else(|| {
                            ExpressionError::expression_eval_error("Arithmetic overflow")
                        })
                } else {
                    let lf = crate::value_utils::number_as_f64(l).unwrap_or(0.0);
                    let rf = crate::value_utils::number_as_f64(r).unwrap_or(0.0);
                    Ok(serde_json::json!(lf - rf))
                }
            }
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
                // Try integer multiplication with overflow checking
                if let (Some(li), Some(ri)) = (l.as_i64(), r.as_i64()) {
                    li.checked_mul(ri)
                        .map(|result| Value::Number(result.into()))
                        .or_else(|| Some(serde_json::json!(li as f64 * ri as f64)))
                        .ok_or_else(|| {
                            ExpressionError::expression_eval_error("Arithmetic overflow")
                        })
                } else {
                    let lf = crate::value_utils::number_as_f64(l).unwrap_or(0.0);
                    let rf = crate::value_utils::number_as_f64(r).unwrap_or(0.0);
                    Ok(serde_json::json!(lf * rf))
                }
            }
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
                let lf = crate::value_utils::number_as_f64(l).unwrap_or(0.0);
                let rf = crate::value_utils::number_as_f64(r).unwrap_or(0.0);

                if rf == 0.0 {
                    return Err(ExpressionError::expression_division_by_zero());
                }

                Ok(serde_json::json!(lf / rf))
            }
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
                    let lf = crate::value_utils::number_as_f64(l).unwrap_or(0.0);
                    let rf = crate::value_utils::number_as_f64(r).unwrap_or(0.0);
                    if rf == 0.0 {
                        return Err(ExpressionError::expression_division_by_zero());
                    }
                    Ok(serde_json::json!(lf % rf))
                }
            }
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
                let lf = crate::value_utils::number_as_f64(l).unwrap_or(0.0);
                let rf = crate::value_utils::number_as_f64(r).unwrap_or(0.0);
                Ok(serde_json::json!(lf.powf(rf)))
            }
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
    fn less_than(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                let lf = crate::value_utils::number_as_f64(l).unwrap_or(0.0);
                let rf = crate::value_utils::number_as_f64(r).unwrap_or(0.0);
                Ok(Value::Bool(lf < rf))
            }
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
    fn greater_than(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                let lf = crate::value_utils::number_as_f64(l).unwrap_or(0.0);
                let rf = crate::value_utils::number_as_f64(r).unwrap_or(0.0);
                Ok(Value::Bool(lf > rf))
            }
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
    fn less_equal(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                let lf = crate::value_utils::number_as_f64(l).unwrap_or(0.0);
                let rf = crate::value_utils::number_as_f64(r).unwrap_or(0.0);
                Ok(Value::Bool(lf <= rf))
            }
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
    fn greater_equal(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => {
                let lf = crate::value_utils::number_as_f64(l).unwrap_or(0.0);
                let rf = crate::value_utils::number_as_f64(r).unwrap_or(0.0);
                Ok(Value::Bool(lf >= rf))
            }
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
                        _ => {}
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
            }
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
            }
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
            }
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
        _depth: usize,
    ) -> ExpressionResult<Value> {
        self.builtins.call(name, args, self, context)
    }

    /// Evaluate a lambda expression with a parameter value
    pub fn eval_lambda(
        &self,
        param: &str,
        body: &Expr,
        value: &Value,
        context: &EvaluationContext,
    ) -> ExpressionResult<Value> {
        // Create a new context with the lambda parameter
        let mut lambda_context = context.clone();
        lambda_context.set_execution_var(param, value.clone());
        self.eval(body, &lambda_context)
    }

    /// Handle higher-order functions that require lambda expressions.
    /// Returns Some(result) if the function was handled, None if it should
    /// be passed to the regular builtin registry.
    fn try_higher_order_function(
        &self,
        name: &str,
        args: &[Expr],
        context: &EvaluationContext,
        depth: usize,
    ) -> Option<ExpressionResult<Value>> {
        match name {
            "filter" => Some(self.eval_filter(args, context, depth)),
            "map" => Some(self.eval_map(args, context, depth)),
            "reduce" => Some(self.eval_reduce(args, context, depth)),
            "find" => Some(self.eval_find(args, context, depth)),
            "every" | "all" => Some(self.eval_every(args, context, depth)),
            "some" | "any" => Some(self.eval_some(args, context, depth)),
            _ => None,
        }
    }

    /// Filter array elements using a lambda predicate
    ///
    /// Usage: `filter(array, x => condition)`
    /// Example: `filter([1, 2, 3, 4, 5], x => x > 2)` returns `[3, 4, 5]`
    fn eval_filter(
        &self,
        args: &[Expr],
        context: &EvaluationContext,
        depth: usize,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "filter",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        // Evaluate the array argument
        let array_val = self.eval_with_depth(&args[0], context, depth + 1)?;
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
            }
        };

        // Filter the array
        let mut result = Vec::with_capacity(array.len());
        for item in array.iter() {
            let predicate_result = self.eval_lambda(param, body, item, context)?;
            if crate::value_utils::to_boolean(&predicate_result) {
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
        depth: usize,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "map",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        // Evaluate the array argument
        let array_val = self.eval_with_depth(&args[0], context, depth + 1)?;
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
            }
        };

        // Map the array
        let mut result = Vec::with_capacity(array.len());
        for item in array.iter() {
            let transformed = self.eval_lambda(param, body, item, context)?;
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
        depth: usize,
    ) -> ExpressionResult<Value> {
        if args.len() != 3 {
            return Err(ExpressionError::expression_invalid_argument(
                "reduce",
                format!("expected 3 arguments, got {}", args.len()),
            ));
        }

        // Evaluate the array argument
        let array_val = self.eval_with_depth(&args[0], context, depth + 1)?;
        let array = array_val.as_array().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "array",
                crate::value_utils::value_type_name(&array_val),
            )
        })?;

        // Evaluate the initial value
        let initial = self.eval_with_depth(&args[1], context, depth + 1)?;

        // Extract the lambda
        let (param, body) = match &args[2] {
            Expr::Lambda { param, body } => (param.as_ref(), body.as_ref()),
            _ => {
                return Err(ExpressionError::expression_type_error(
                    "lambda expression",
                    "non-lambda",
                ));
            }
        };

        // Reduce the array
        let mut accumulator = initial;
        for item in array.iter() {
            // Create context with both accumulator and current item
            let mut reduce_context = context.clone();
            reduce_context.set_execution_var("$acc", accumulator.clone());
            reduce_context.set_execution_var(param, item.clone());
            accumulator = self.eval(body, &reduce_context)?;
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
        depth: usize,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "find",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        let array_val = self.eval_with_depth(&args[0], context, depth + 1)?;
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
            }
        };

        for item in array.iter() {
            let predicate_result = self.eval_lambda(param, body, item, context)?;
            if crate::value_utils::to_boolean(&predicate_result) {
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
        depth: usize,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "every",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        let array_val = self.eval_with_depth(&args[0], context, depth + 1)?;
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
            }
        };

        for item in array.iter() {
            let predicate_result = self.eval_lambda(param, body, item, context)?;
            if !crate::value_utils::to_boolean(&predicate_result) {
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
        depth: usize,
    ) -> ExpressionResult<Value> {
        if args.len() != 2 {
            return Err(ExpressionError::expression_invalid_argument(
                "some",
                format!("expected 2 arguments, got {}", args.len()),
            ));
        }

        let array_val = self.eval_with_depth(&args[0], context, depth + 1)?;
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
            }
        };

        for item in array.iter() {
            let predicate_result = self.eval_lambda(param, body, item, context)?;
            if crate::value_utils::to_boolean(&predicate_result) {
                return Ok(Value::Bool(true));
            }
        }

        Ok(Value::Bool(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::BuiltinRegistry;

    fn create_evaluator() -> Evaluator {
        let registry = Arc::new(BuiltinRegistry::new());
        Evaluator::new(registry)
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
}
