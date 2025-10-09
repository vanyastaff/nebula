//! AST evaluation module
//!
//! This module implements the evaluation of parsed expression ASTs.

use crate::builtins::BuiltinRegistry;
use crate::context::EvaluationContext;
use crate::core::ast::{BinaryOp, Expr};
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use nebula_error::NebulaError;
use nebula_value::Value;
use nebula_value::{JsonValueExt, ValueRefExt};
use parking_lot::Mutex;
#[cfg(feature = "regex")]
use regex::Regex;
#[cfg(feature = "regex")]
use std::collections::HashMap;
use std::sync::Arc;

/// Maximum recursion depth for expression evaluation
const MAX_RECURSION_DEPTH: usize = 256;

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
    pub fn eval(&self, expr: &Expr, context: &EvaluationContext) -> ExpressionResult<Value> {
        self.eval_with_depth(expr, context, 0)
    }

    /// Evaluate an expression with recursion depth tracking
    fn eval_with_depth(&self, expr: &Expr, context: &EvaluationContext, depth: usize) -> ExpressionResult<Value> {
        // Check recursion depth limit
        if depth > MAX_RECURSION_DEPTH {
            return Err(NebulaError::expression_eval_error(
                format!("Maximum recursion depth ({}) exceeded", MAX_RECURSION_DEPTH)
            ));
        }
        match expr {
            Expr::Literal(val) => Ok(val.clone()),

            Expr::Variable(name) => context
                .resolve_variable(&**name)
                .ok_or_else(|| NebulaError::expression_variable_not_found(&**name)),

            Expr::Identifier(name) => {
                // Try to resolve as a constant or special value
                // Optimize: use Arc<str> directly instead of converting to String
                Ok(Value::text(name.as_ref()))
            }

            Expr::Negate(expr) => {
                let val = self.eval_with_depth(expr, context, depth + 1)?;
                match val {
                    Value::Integer(i) => Ok(Value::integer(-i.value())),
                    Value::Float(f) => Ok(Value::float(-f.value())),
                    _ => Err(NebulaError::expression_type_error(
                        "number",
                        val.kind().name(),
                    )),
                }
            }

            Expr::Not(expr) => {
                let val = self.eval_with_depth(expr, context, depth + 1)?;
                Ok(Value::boolean(!val.to_boolean()))
            }

            Expr::Binary { left, op, right } => self.eval_binary_op(*op, left, right, context, depth),

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
                // Optimize: pre-allocate Vec with known capacity
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
                let val = self.eval_with_depth(value, context, depth + 1)?;
                // Optimize: pre-allocate Vec with known capacity
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
                if cond_val.to_boolean() {
                    self.eval_with_depth(then_expr, context, depth + 1)
                } else {
                    self.eval_with_depth(else_expr, context, depth + 1)
                }
            }

            Expr::Lambda { .. } => {
                // Lambdas are handled specially in higher-order functions
                Err(NebulaError::expression_eval_error(
                    "Lambda expressions can only be used as function arguments",
                ))
            }

            Expr::Array(elements) => {
                let values: Result<Vec<_>, _> =
                    elements.iter().map(|e| self.eval_with_depth(e, context, depth + 1)).collect();
                let values = values?;
                // Optimize: collect directly into Vec and convert once
                let json_values: Vec<_> = values.into_iter().map(|v| v.to_json()).collect();
                Ok(Value::Array(nebula_value::Array::from_vec(json_values)))
            }

            Expr::Object(pairs) => {
                let mut obj = nebula_value::Object::new();
                for (key, expr) in pairs {
                    let value = self.eval_with_depth(expr, context, depth + 1)?;
                    obj = obj.insert(key.to_string(), value.to_json());
                }
                Ok(Value::Object(obj))
            }
        }
    }

    /// Evaluate a binary operation
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
                if !left_val.to_boolean() {
                    // Short-circuit: if left is false, don't evaluate right
                    return Ok(Value::boolean(false));
                }
                let right_val = self.eval_with_depth(right, context, depth + 1)?;
                Ok(Value::boolean(right_val.to_boolean()))
            }
            BinaryOp::Or => {
                let left_val = self.eval_with_depth(left, context, depth + 1)?;
                if left_val.to_boolean() {
                    // Short-circuit: if left is true, don't evaluate right
                    return Ok(Value::boolean(true));
                }
                let right_val = self.eval_with_depth(right, context, depth + 1)?;
                Ok(Value::boolean(right_val.to_boolean()))
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
                    BinaryOp::Equal => Ok(Value::boolean(left_val == right_val)),
                    BinaryOp::NotEqual => Ok(Value::boolean(left_val != right_val)),
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
    fn add(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => Ok(Value::integer(l.value() + r.value())),
            (Value::Float(l), Value::Float(r)) => Ok(Value::float(l.value() + r.value())),
            (Value::Integer(l), Value::Float(r)) => Ok(Value::float(l.value() as f64 + r.value())),
            (Value::Float(l), Value::Integer(r)) => Ok(Value::float(l.value() + r.value() as f64)),
            (Value::Text(l), Value::Text(r)) => {
                Ok(Value::text(format!("{}{}", l.as_str(), r.as_str())))
            }
            _ => Err(NebulaError::expression_type_error(
                "number or string",
                format!("{} and {}", left.kind().name(), right.kind().name()),
            )),
        }
    }

    /// Subtraction
    fn subtract(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => Ok(Value::integer(l.value() - r.value())),
            (Value::Float(l), Value::Float(r)) => Ok(Value::float(l.value() - r.value())),
            (Value::Integer(l), Value::Float(r)) => Ok(Value::float(l.value() as f64 - r.value())),
            (Value::Float(l), Value::Integer(r)) => Ok(Value::float(l.value() - r.value() as f64)),
            _ => Err(NebulaError::expression_type_error(
                "number",
                format!("{} and {}", left.kind().name(), right.kind().name()),
            )),
        }
    }

    /// Multiplication
    fn multiply(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => Ok(Value::integer(l.value() * r.value())),
            (Value::Float(l), Value::Float(r)) => Ok(Value::float(l.value() * r.value())),
            (Value::Integer(l), Value::Float(r)) => Ok(Value::float(l.value() as f64 * r.value())),
            (Value::Float(l), Value::Integer(r)) => Ok(Value::float(l.value() * r.value() as f64)),
            _ => Err(NebulaError::expression_type_error(
                "number",
                format!("{} and {}", left.kind().name(), right.kind().name()),
            )),
        }
    }

    /// Division
    fn divide(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => {
                if r.value() == 0 {
                    return Err(NebulaError::expression_division_by_zero());
                }
                Ok(Value::integer(l.value() / r.value()))
            }
            (Value::Float(l), Value::Float(r)) => {
                if r.value() == 0.0 {
                    return Err(NebulaError::expression_division_by_zero());
                }
                Ok(Value::float(l.value() / r.value()))
            }
            (Value::Integer(l), Value::Float(r)) => {
                if r.value() == 0.0 {
                    return Err(NebulaError::expression_division_by_zero());
                }
                Ok(Value::float(l.value() as f64 / r.value()))
            }
            (Value::Float(l), Value::Integer(r)) => {
                if r.value() == 0 {
                    return Err(NebulaError::expression_division_by_zero());
                }
                Ok(Value::float(l.value() / r.value() as f64))
            }
            _ => Err(NebulaError::expression_type_error(
                "number",
                format!("{} and {}", left.kind().name(), right.kind().name()),
            )),
        }
    }

    /// Modulo
    fn modulo(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => {
                if r.value() == 0 {
                    return Err(NebulaError::expression_division_by_zero());
                }
                Ok(Value::integer(l.value() % r.value()))
            }
            _ => Err(NebulaError::expression_type_error(
                "integer",
                format!("{} and {}", left.kind().name(), right.kind().name()),
            )),
        }
    }

    /// Power
    fn power(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => {
                if r.value() < 0 {
                    Ok(Value::float((l.value() as f64).powf(r.value() as f64)))
                } else {
                    Ok(Value::integer(l.value().pow(r.value() as u32)))
                }
            }
            (Value::Float(l), Value::Float(r)) => Ok(Value::float(l.value().powf(r.value()))),
            (Value::Integer(l), Value::Float(r)) => {
                Ok(Value::float((l.value() as f64).powf(r.value())))
            }
            (Value::Float(l), Value::Integer(r)) => {
                Ok(Value::float(l.value().powf(r.value() as f64)))
            }
            _ => Err(NebulaError::expression_type_error(
                "number",
                format!("{} and {}", left.kind().name(), right.kind().name()),
            )),
        }
    }

    /// Less than comparison
    fn less_than(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => Ok(Value::boolean(l.value() < r.value())),
            (Value::Float(l), Value::Float(r)) => Ok(Value::boolean(l.value() < r.value())),
            (Value::Integer(l), Value::Float(r)) => {
                Ok(Value::boolean((l.value() as f64) < r.value()))
            }
            (Value::Float(l), Value::Integer(r)) => {
                Ok(Value::boolean(l.value() < (r.value() as f64)))
            }
            (Value::Text(l), Value::Text(r)) => Ok(Value::boolean(l.as_str() < r.as_str())),
            _ => Err(NebulaError::expression_type_error(
                "comparable values",
                format!("{} and {}", left.kind().name(), right.kind().name()),
            )),
        }
    }

    /// Greater than comparison
    fn greater_than(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => Ok(Value::boolean(l.value() > r.value())),
            (Value::Float(l), Value::Float(r)) => Ok(Value::boolean(l.value() > r.value())),
            (Value::Integer(l), Value::Float(r)) => {
                Ok(Value::boolean((l.value() as f64) > r.value()))
            }
            (Value::Float(l), Value::Integer(r)) => {
                Ok(Value::boolean(l.value() > (r.value() as f64)))
            }
            (Value::Text(l), Value::Text(r)) => Ok(Value::boolean(l.as_str() > r.as_str())),
            _ => Err(NebulaError::expression_type_error(
                "comparable values",
                format!("{} and {}", left.kind().name(), right.kind().name()),
            )),
        }
    }

    /// Less than or equal comparison
    fn less_equal(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => Ok(Value::boolean(l.value() <= r.value())),
            (Value::Float(l), Value::Float(r)) => Ok(Value::boolean(l.value() <= r.value())),
            (Value::Integer(l), Value::Float(r)) => {
                Ok(Value::boolean((l.value() as f64) <= r.value()))
            }
            (Value::Float(l), Value::Integer(r)) => {
                Ok(Value::boolean(l.value() <= (r.value() as f64)))
            }
            (Value::Text(l), Value::Text(r)) => Ok(Value::boolean(l.as_str() <= r.as_str())),
            _ => Err(NebulaError::expression_type_error(
                "comparable values",
                format!("{} and {}", left.kind().name(), right.kind().name()),
            )),
        }
    }

    /// Greater than or equal comparison
    fn greater_equal(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => Ok(Value::boolean(l.value() >= r.value())),
            (Value::Float(l), Value::Float(r)) => Ok(Value::boolean(l.value() >= r.value())),
            (Value::Integer(l), Value::Float(r)) => {
                Ok(Value::boolean((l.value() as f64) >= r.value()))
            }
            (Value::Float(l), Value::Integer(r)) => {
                Ok(Value::boolean(l.value() >= (r.value() as f64)))
            }
            (Value::Text(l), Value::Text(r)) => Ok(Value::boolean(l.as_str() >= r.as_str())),
            _ => Err(NebulaError::expression_type_error(
                "comparable values",
                format!("{} and {}", left.kind().name(), right.kind().name()),
            )),
        }
    }

    /// Regex match
    #[cfg(feature = "regex")]
    fn regex_match(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        let text = left
            .as_str()
            .ok_or_else(|| NebulaError::expression_type_error("string", left.kind().name()))?;

        let pattern = right
            .as_str()
            .ok_or_else(|| NebulaError::expression_type_error("string", right.kind().name()))?;

        // Try to get from cache first
        let mut cache = self.regex_cache.lock();
        let regex = if let Some(cached_regex) = cache.get(pattern) {
            // Cache hit - clone the Regex (cheap, Arc internally)
            cached_regex.clone()
        } else {
            // Cache miss - compile and cache
            let new_regex = Regex::new(pattern)
                .map_err(|e| NebulaError::expression_regex_error(e.to_string()))?;
            cache.insert(pattern.to_string(), new_regex.clone());
            new_regex
        };
        drop(cache); // Release borrow before is_match

        Ok(Value::boolean(regex.is_match(text)))
    }

    #[cfg(not(feature = "regex"))]
    fn regex_match(&self, _left: &Value, _right: &Value) -> ExpressionResult<Value> {
        Err(NebulaError::expression_eval_error(
            "Regex matching is not enabled (feature 'regex' not enabled)"
        ))
    }

    /// Access a property of an object
    fn access_property(&self, obj: &Value, property: &str) -> ExpressionResult<Value> {
        match obj {
            Value::Object(o) => {
                let json_val = o.get(property).ok_or_else(|| {
                    NebulaError::expression_eval_error(format!("Property '{}' not found", property))
                })?;
                Ok(json_val.to_nebula_value_or_null())
            }
            _ => Err(NebulaError::expression_type_error(
                "object",
                obj.kind().name(),
            )),
        }
    }

    /// Access an element of an array or object by index
    fn access_index(&self, obj: &Value, index: &Value) -> ExpressionResult<Value> {
        match obj {
            Value::Array(arr) => {
                let idx = index.to_integer()?;
                let len = arr.len() as i64;
                let actual_idx = if idx < 0 { len + idx } else { idx };

                if actual_idx < 0 || actual_idx >= len {
                    return Err(NebulaError::expression_index_out_of_bounds(
                        actual_idx as usize,
                        len as usize,
                    ));
                }

                let json_val = arr.get(actual_idx as usize).ok_or_else(|| {
                    NebulaError::expression_index_out_of_bounds(actual_idx as usize, len as usize)
                })?;
                Ok(json_val.to_nebula_value_or_null())
            }
            Value::Object(o) => {
                let key = index.as_str().ok_or_else(|| {
                    NebulaError::expression_type_error("string", index.kind().name())
                })?;
                let json_val = o.get(key).ok_or_else(|| {
                    NebulaError::expression_eval_error(format!("Key '{}' not found", key))
                })?;
                Ok(json_val.to_nebula_value_or_null())
            }
            _ => Err(NebulaError::expression_type_error(
                "array or object",
                obj.kind().name(),
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
        let expr = Expr::Literal(Value::integer(42));
        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_integer(), Some(42));
    }

    #[test]
    fn test_eval_arithmetic() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::integer(10))),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Value::integer(5))),
        };
        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_integer(), Some(15));
    }

    #[test]
    fn test_deep_nesting_within_limit() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // Create moderately nested expression (safe for both construction and evaluation)
        let mut expr = Expr::Literal(Value::integer(1));
        for _ in 0..50 {  // 50 levels is safe and tests recursion tracking works
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::Add,
                right: Box::new(Expr::Literal(Value::integer(1))),
            };
        }

        // Should succeed (50 << 256)
        let result = evaluator.eval(&expr, &context);
        assert!(result.is_ok(), "50-level deep expression should succeed");
        assert_eq!(result.unwrap().as_integer(), Some(51));
    }

    #[test]
    fn test_short_circuit_and_false() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // false && <anything> should short-circuit and not evaluate right side
        // Using a division by zero on the right to prove it's not evaluated
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::boolean(false))),
            op: BinaryOp::And,
            right: Box::new(Expr::Binary {
                left: Box::new(Expr::Literal(Value::integer(1))),
                op: BinaryOp::Divide,
                right: Box::new(Expr::Literal(Value::integer(0))),
            }),
        };

        // Should succeed without dividing by zero (short-circuit)
        let result = evaluator.eval(&expr, &context);
        assert!(result.is_ok(), "Short-circuit should prevent division by zero");
        assert_eq!(result.unwrap().as_boolean(), Some(false));
    }

    #[test]
    fn test_short_circuit_or_true() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // true || <anything> should short-circuit and not evaluate right side
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::boolean(true))),
            op: BinaryOp::Or,
            right: Box::new(Expr::Binary {
                left: Box::new(Expr::Literal(Value::integer(1))),
                op: BinaryOp::Divide,
                right: Box::new(Expr::Literal(Value::integer(0))),
            }),
        };

        // Should succeed without dividing by zero (short-circuit)
        let result = evaluator.eval(&expr, &context);
        assert!(result.is_ok(), "Short-circuit should prevent division by zero");
        assert_eq!(result.unwrap().as_boolean(), Some(true));
    }

    #[test]
    fn test_and_evaluates_both_when_left_true() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // true && false should evaluate both
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::boolean(true))),
            op: BinaryOp::And,
            right: Box::new(Expr::Literal(Value::boolean(false))),
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_boolean(), Some(false));
    }

    #[test]
    fn test_or_evaluates_both_when_left_false() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // false || true should evaluate both
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::boolean(false))),
            op: BinaryOp::Or,
            right: Box::new(Expr::Literal(Value::boolean(true))),
        };

        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_boolean(), Some(true));
    }

    #[test]
    #[cfg(feature = "regex")]
    fn test_regex_caching() {
        let evaluator = create_evaluator();
        let context = EvaluationContext::new();

        // First regex match - should compile and cache
        let expr1 = Expr::Binary {
            left: Box::new(Expr::Literal(Value::text("hello world"))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::text("hello.*"))),
        };
        let result1 = evaluator.eval(&expr1, &context).unwrap();
        assert_eq!(result1.as_boolean(), Some(true));

        // Second regex match with same pattern - should use cached regex
        let expr2 = Expr::Binary {
            left: Box::new(Expr::Literal(Value::text("hello universe"))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::text("hello.*"))),
        };
        let result2 = evaluator.eval(&expr2, &context).unwrap();
        assert_eq!(result2.as_boolean(), Some(true));

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
            left: Box::new(Expr::Literal(Value::text("goodbye world"))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::text("^hello"))),
        };
        let result = evaluator.eval(&expr, &context).unwrap();
        assert_eq!(result.as_boolean(), Some(false));
    }
}
