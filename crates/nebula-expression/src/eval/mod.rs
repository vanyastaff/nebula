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
use regex::Regex;
use std::sync::Arc;

/// Evaluator for expression ASTs
pub struct Evaluator {
    builtins: Arc<BuiltinRegistry>,
}

impl Evaluator {
    /// Create a new evaluator with the given builtin registry
    pub fn new(builtins: Arc<BuiltinRegistry>) -> Self {
        Self { builtins }
    }

    /// Evaluate an expression in the given context
    pub fn eval(&self, expr: &Expr, context: &EvaluationContext) -> ExpressionResult<Value> {
        match expr {
            Expr::Literal(val) => Ok(val.clone()),

            Expr::Variable(name) => context
                .resolve_variable(name)
                .ok_or_else(|| NebulaError::expression_variable_not_found(name)),

            Expr::Identifier(name) => {
                // Try to resolve as a constant or special value
                Ok(Value::text(name.clone()))
            }

            Expr::Negate(expr) => {
                let val = self.eval(expr, context)?;
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
                let val = self.eval(expr, context)?;
                Ok(Value::boolean(!val.to_boolean()))
            }

            Expr::Binary { left, op, right } => self.eval_binary_op(*op, left, right, context),

            Expr::PropertyAccess { object, property } => {
                let obj_val = self.eval(object, context)?;
                self.access_property(&obj_val, property)
            }

            Expr::IndexAccess { object, index } => {
                let obj_val = self.eval(object, context)?;
                let index_val = self.eval(index, context)?;
                self.access_index(&obj_val, &index_val)
            }

            Expr::FunctionCall { name, args } => {
                let arg_values: Result<Vec<_>, _> =
                    args.iter().map(|arg| self.eval(arg, context)).collect();
                let arg_values = arg_values?;
                self.call_function(name, &arg_values, context)
            }

            Expr::Pipeline {
                value,
                function,
                args,
            } => {
                let val = self.eval(value, context)?;
                let mut arg_values: Vec<Value> = vec![val];
                let additional_args: Result<Vec<_>, _> =
                    args.iter().map(|arg| self.eval(arg, context)).collect();
                arg_values.extend(additional_args?);
                self.call_function(function, &arg_values, context)
            }

            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let cond_val = self.eval(condition, context)?;
                if cond_val.to_boolean() {
                    self.eval(then_expr, context)
                } else {
                    self.eval(else_expr, context)
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
                    elements.iter().map(|e| self.eval(e, context)).collect();
                let values = values?;
                let mut arr = nebula_value::Array::new();
                for val in values {
                    arr = arr.push(val.to_json());
                }
                Ok(Value::Array(arr))
            }

            Expr::Object(pairs) => {
                let mut obj = nebula_value::Object::new();
                for (key, expr) in pairs {
                    let value = self.eval(expr, context)?;
                    obj = obj.insert(key.clone(), value.to_json());
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
    ) -> ExpressionResult<Value> {
        let left_val = self.eval(left, context)?;
        let right_val = self.eval(right, context)?;

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
            BinaryOp::And => Ok(Value::boolean(
                left_val.to_boolean() && right_val.to_boolean(),
            )),
            BinaryOp::Or => Ok(Value::boolean(
                left_val.to_boolean() || right_val.to_boolean(),
            )),
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
    fn regex_match(&self, left: &Value, right: &Value) -> ExpressionResult<Value> {
        let text = left
            .as_str()
            .ok_or_else(|| NebulaError::expression_type_error("string", left.kind().name()))?;

        let pattern = right
            .as_str()
            .ok_or_else(|| NebulaError::expression_type_error("string", right.kind().name()))?;

        let regex =
            Regex::new(pattern).map_err(|e| NebulaError::expression_regex_error(e.to_string()))?;

        Ok(Value::boolean(regex.is_match(text)))
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
}
