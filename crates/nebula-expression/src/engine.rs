//! Expression engine with caching support
//!
//! This module provides the main ExpressionEngine that parses and evaluates expressions,
//! with optional caching of parsed ASTs for improved performance.

use crate::builtins::BuiltinRegistry;
use crate::context::EvaluationContext;
use crate::core::ast::Expr;
use crate::core::error::ExpressionResult;
use crate::eval::Evaluator;
use crate::lexer::Lexer;
use crate::parser::Parser;
use nebula_log::{debug, trace};
use nebula_memory::cache::{CacheConfig, ComputeCache};
use nebula_value::Value;
use std::sync::{Arc, Mutex};

/// Expression engine with parsing and evaluation capabilities
pub struct ExpressionEngine {
    /// Cache for parsed expressions
    cache: Option<Arc<Mutex<ComputeCache<String, Expr>>>>,
    /// Builtin function registry
    builtins: Arc<BuiltinRegistry>,
    /// Evaluator
    evaluator: Evaluator,
}

impl ExpressionEngine {
    /// Create a new expression engine with default configuration
    pub fn new() -> Self {
        let builtins = Arc::new(BuiltinRegistry::new());
        let evaluator = Evaluator::new(Arc::clone(&builtins));

        Self {
            cache: None,
            builtins,
            evaluator,
        }
    }

    /// Create a new expression engine with a cache of the specified size
    pub fn with_cache_size(size: usize) -> Self {
        let builtins = Arc::new(BuiltinRegistry::new());
        let evaluator = Evaluator::new(Arc::clone(&builtins));

        let config = CacheConfig::new(size);
        let cache = ComputeCache::with_config(config);

        debug!(cache_size = size, "Created expression engine with cache");

        Self {
            cache: Some(Arc::new(Mutex::new(cache))),
            builtins,
            evaluator,
        }
    }

    /// Register a custom builtin function
    pub fn register_function(&mut self, name: &str, func: crate::builtins::BuiltinFunction) {
        Arc::get_mut(&mut self.builtins)
            .expect("Cannot register function after builtins have been shared")
            .register(name, func);
    }

    /// Evaluate an expression string in the given context
    pub fn evaluate(
        &self,
        expression: &str,
        context: &EvaluationContext,
    ) -> ExpressionResult<Value> {
        trace!(expression = expression, "Evaluating expression");

        // Parse the expression (with caching if enabled)
        let ast = if let Some(cache) = &self.cache {
            let mut cache_guard = cache.lock().unwrap();
            cache_guard.get_or_compute(expression.to_string(), || {
                self.parse_expression(expression)
                    .map_err(|_| nebula_memory::MemoryError::from(nebula_memory::MemoryErrorCode::InvalidState))
            })?
        } else {
            self.parse_expression(expression)?
        };

        // Evaluate the AST
        let result = self.evaluator.eval(&ast, context)?;

        trace!(result = ?result, "Expression evaluation completed");
        Ok(result)
    }

    /// Evaluate a template string with multiple {{ }} expressions
    /// All {{ expression }} patterns will be replaced with their evaluated results
    /// Returns the final string with all expressions replaced
    pub fn evaluate_template(
        &self,
        template: &str,
        context: &EvaluationContext,
    ) -> ExpressionResult<String> {
        let mut result = String::new();
        let mut last_pos = 0;
        let chars: Vec<char> = template.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            // Look for opening {{
            if i + 1 < len && chars[i] == '{' && chars[i + 1] == '{' {
                // Add text before the expression
                result.push_str(&template[last_pos..i]);

                // Find closing }}
                let mut j = i + 2;
                let mut depth = 1;
                while j + 1 < len {
                    if chars[j] == '{' && chars[j + 1] == '{' {
                        depth += 1;
                        j += 2;
                    } else if chars[j] == '}' && chars[j + 1] == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        j += 2;
                    } else {
                        j += 1;
                    }
                }

                if depth == 0 && j + 1 < len {
                    // Extract and evaluate the expression
                    let expr_start = i + 2;
                    let expr_end = j;
                    let expr: String = chars[expr_start..expr_end].iter().collect();

                    trace!(expression = expr.as_str(), "Evaluating template expression");

                    let value = self.evaluate(expr.trim(), context)?;
                    result.push_str(&value.to_string());

                    i = j + 2;
                    last_pos = i;
                } else {
                    // No matching closing }}
                    result.push(chars[i]);
                    i += 1;
                    last_pos = i;
                }
            } else {
                i += 1;
            }
        }

        // Add remaining text
        result.push_str(&template[last_pos..]);

        trace!(result = result.as_str(), "Template evaluation completed");
        Ok(result)
    }

    /// Parse an expression string into an AST (internal helper)
    fn parse_expression(&self, expression: &str) -> ExpressionResult<Expr> {
        // Handle template delimiters
        let expr_content = if expression.trim().starts_with("{{") && expression.trim().ends_with("}}") {
            let trimmed = expression.trim();
            &trimmed[2..trimmed.len() - 2].trim()
        } else {
            expression
        };

        // Tokenize
        let mut lexer = Lexer::new(expr_content);
        let tokens = lexer.tokenize()?;

        // Parse
        let mut parser = Parser::new(tokens);
        parser.parse()
    }

    /// Clear the cache (if caching is enabled)
    pub fn clear_cache(&self) {
        if let Some(cache) = &self.cache {
            let mut cache_guard = cache.lock().unwrap();
            cache_guard.clear();
            debug!("Expression cache cleared");
        }
    }

    /// Get cache statistics (if caching is enabled)
    #[cfg(feature = "std")]
    pub fn cache_stats(&self) -> Option<nebula_memory::cache::CacheMetrics> {
        self.cache.as_ref().map(|cache| {
            let cache_guard = cache.lock().unwrap();
            cache_guard.metrics()
        })
    }
}

impl Default for ExpressionEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_literal() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine.evaluate("42", &context).unwrap();
        assert_eq!(result.as_integer(), Some(42));
    }

    #[test]
    fn test_evaluate_arithmetic() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine.evaluate("2 + 3 * 4", &context).unwrap();
        assert_eq!(result.as_integer(), Some(14));
    }

    #[test]
    fn test_evaluate_string_function() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine.evaluate("uppercase('hello')", &context).unwrap();
        assert_eq!(result.as_str(), Some("HELLO"));
    }

    #[test]
    fn test_evaluate_single_template() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine.evaluate("{{ 2 + 3 }}", &context).unwrap();
        assert_eq!(result.as_integer(), Some(5));
    }

    #[test]
    fn test_evaluate_template_simple() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::text("World"));

        let result = engine.evaluate_template("Hello {{ $input }}!", &context).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn test_evaluate_template_multiple() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::text("Alice"));
        context.set_execution_var("order_id", Value::integer(12345));

        let result = engine
            .evaluate_template(
                "Hello {{ $input }}! Your order #{{ $execution.order_id }} is ready.",
                &context,
            )
            .unwrap();
        assert_eq!(result, "Hello Alice! Your order #12345 is ready.");
    }

    #[test]
    fn test_evaluate_template_with_functions() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::text("john"));

        let result = engine
            .evaluate_template(
                "User: {{ $input | uppercase() }}, Length: {{ length($input) }}",
                &context,
            )
            .unwrap();
        assert_eq!(result, "User: JOHN, Length: 4");
    }

    #[test]
    fn test_evaluate_template_html() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::integer(42));
        context.set_execution_var("name", Value::text("Alice"));

        let html = r#"<html>
  <h1>Welcome {{ $execution.name }}</h1>
  <p>Your score: {{ $input * 2 }}</p>
  <span>Total: {{ $input + 8 }}</span>
</html>"#;

        let result = engine.evaluate_template(html, &context).unwrap();
        assert!(result.contains("<h1>Welcome Alice</h1>"));
        assert!(result.contains("<p>Your score: 84</p>"));
        assert!(result.contains("<span>Total: 50</span>"));
    }

    #[test]
    fn test_evaluate_template_with_literal_braces() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine
            .evaluate_template("Result: {{ 2 + 3 }}, another: {{ 10 * 2 }}", &context)
            .unwrap();
        assert_eq!(result, "Result: 5, another: 20");
    }

    #[test]
    fn test_evaluate_template_no_expressions() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine
            .evaluate_template("Just plain text without expressions", &context)
            .unwrap();
        assert_eq!(result, "Just plain text without expressions");
    }

    #[test]
    fn test_evaluate_variable() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_execution_var("id", Value::text("test-123"));

        let result = engine.evaluate("$execution.id", &context).unwrap();
        assert_eq!(result.as_str(), Some("test-123"));
    }

    #[test]
    fn test_with_cache() {
        let engine = ExpressionEngine::with_cache_size(100);
        let context = EvaluationContext::new();

        // First evaluation
        let result1 = engine.evaluate("2 + 3", &context).unwrap();
        assert_eq!(result1.as_integer(), Some(5));

        // Second evaluation (should use cache)
        let result2 = engine.evaluate("2 + 3", &context).unwrap();
        assert_eq!(result2.as_integer(), Some(5));

        #[cfg(feature = "std")]
        {
            let stats = engine.cache_stats().unwrap();
            assert!(stats.hits > 0 || stats.misses > 0);
        }
    }

    #[test]
    fn test_conditional() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine.evaluate("if true then 1 else 2", &context).unwrap();
        assert_eq!(result.as_integer(), Some(1));

        let result = engine.evaluate("if false then 1 else 2", &context).unwrap();
        assert_eq!(result.as_integer(), Some(2));
    }

    #[test]
    fn test_pipeline() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine.evaluate("{{ \"hello\" | uppercase() }}", &context).unwrap();
        assert_eq!(result.as_str(), Some("HELLO"));
    }
}
