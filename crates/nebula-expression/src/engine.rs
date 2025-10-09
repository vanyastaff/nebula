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
use parking_lot::RwLock;
use std::sync::Arc;

/// Expression engine with parsing and evaluation capabilities
pub struct ExpressionEngine {
    /// Cache for parsed expressions
    expr_cache: Option<Arc<RwLock<ComputeCache<Arc<str>, Expr>>>>,
    /// Cache for parsed templates
    template_cache: Option<Arc<RwLock<ComputeCache<Arc<str>, crate::Template>>>>,
    /// Builtin function registry
    builtins: Arc<BuiltinRegistry>,
    /// Evaluator
    evaluator: Evaluator,
}

impl ExpressionEngine {
    /// Create a new expression engine with default configuration (no caching)
    pub fn new() -> Self {
        let builtins = Arc::new(BuiltinRegistry::new());
        let evaluator = Evaluator::new(Arc::clone(&builtins));

        Self {
            expr_cache: None,
            template_cache: None,
            builtins,
            evaluator,
        }
    }

    /// Create a new expression engine with caching for both expressions and templates
    pub fn with_cache_size(size: usize) -> Self {
        let builtins = Arc::new(BuiltinRegistry::new());
        let evaluator = Evaluator::new(Arc::clone(&builtins));

        let expr_config = CacheConfig::new(size);
        let expr_cache = ComputeCache::with_config(expr_config);

        let template_config = CacheConfig::new(size);
        let template_cache = ComputeCache::with_config(template_config);

        debug!(
            cache_size = size,
            "Created expression engine with expression and template caches"
        );

        Self {
            expr_cache: Some(Arc::new(RwLock::new(expr_cache))),
            template_cache: Some(Arc::new(RwLock::new(template_cache))),
            builtins,
            evaluator,
        }
    }

    /// Create expression engine with separate cache sizes for expressions and templates
    pub fn with_cache_sizes(expr_cache_size: usize, template_cache_size: usize) -> Self {
        let builtins = Arc::new(BuiltinRegistry::new());
        let evaluator = Evaluator::new(Arc::clone(&builtins));

        let expr_config = CacheConfig::new(expr_cache_size);
        let expr_cache = ComputeCache::with_config(expr_config);

        let template_config = CacheConfig::new(template_cache_size);
        let template_cache = ComputeCache::with_config(template_config);

        debug!(
            expr_cache_size = expr_cache_size,
            template_cache_size = template_cache_size,
            "Created expression engine with separate caches"
        );

        Self {
            expr_cache: Some(Arc::new(RwLock::new(expr_cache))),
            template_cache: Some(Arc::new(RwLock::new(template_cache))),
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
        let ast = if let Some(cache) = &self.expr_cache {
            let key: Arc<str> = Arc::from(expression);
            let mut cache_write = cache.write();
            cache_write.get_or_compute(key, || {
                self.parse_expression(expression).map_err(|_| {
                    nebula_memory::MemoryError::from(nebula_memory::MemoryErrorCode::InvalidState)
                })
            })?
        } else {
            self.parse_expression(expression)?
        };

        // Evaluate the AST
        let result = self.evaluator.eval(&ast, context)?;

        trace!(result = ?result, "Expression evaluation completed");
        Ok(result)
    }

    /// Parse a template from a string (with caching if enabled)
    ///
    /// If template caching is enabled, this will return a cached template
    /// for the same source string, avoiding re-parsing.
    pub fn parse_template(&self, source: impl Into<String>) -> ExpressionResult<crate::Template> {
        let source_str = source.into();

        // Use cache if available
        if let Some(cache) = &self.template_cache {
            let key: Arc<str> = Arc::from(source_str.as_str());
            let mut cache_write = cache.write();
            let template = cache_write.get_or_compute(key, || {
                crate::Template::new(&source_str).map_err(|_| {
                    nebula_memory::MemoryError::from(nebula_memory::MemoryErrorCode::InvalidState)
                })
            })?;
            Ok(template)
        } else {
            crate::Template::new(source_str)
        }
    }

    /// Get or parse a template (alias for parse_template with caching)
    pub fn get_template(&self, source: impl Into<String>) -> ExpressionResult<crate::Template> {
        self.parse_template(source)
    }

    /// Render a parsed template with the given context
    pub fn render_template(
        &self,
        template: &crate::Template,
        context: &EvaluationContext,
    ) -> ExpressionResult<String> {
        template.render(self, context)
    }

    /// Parse an expression string into an AST (internal helper)
    fn parse_expression(&self, expression: &str) -> ExpressionResult<Expr> {
        // Handle template delimiters
        let expr_content =
            if expression.trim().starts_with("{{") && expression.trim().ends_with("}}") {
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

    /// Clear all caches (expressions and templates)
    pub fn clear_cache(&self) {
        if let Some(cache) = &self.expr_cache {
            let mut cache_write = cache.write();
            cache_write.clear();
            debug!("Expression cache cleared");
        }
        if let Some(cache) = &self.template_cache {
            let mut cache_write = cache.write();
            cache_write.clear();
            debug!("Template cache cleared");
        }
    }

    /// Clear expression cache only
    pub fn clear_expr_cache(&self) {
        if let Some(cache) = &self.expr_cache {
            let mut cache_write = cache.write();
            cache_write.clear();
            debug!("Expression cache cleared");
        }
    }

    /// Clear template cache only
    pub fn clear_template_cache(&self) {
        if let Some(cache) = &self.template_cache {
            let mut cache_write = cache.write();
            cache_write.clear();
            debug!("Template cache cleared");
        }
    }

    /// Get expression cache statistics
    pub fn expr_cache_stats(&self) -> Option<nebula_memory::cache::CacheMetrics> {
        self.expr_cache.as_ref().map(|cache| {
            let cache_read = cache.read();
            cache_read.metrics()
        })
    }

    /// Get template cache statistics
    pub fn template_cache_stats(&self) -> Option<nebula_memory::cache::CacheMetrics> {
        self.template_cache.as_ref().map(|cache| {
            let cache_read = cache.read();
            cache_read.metrics()
        })
    }

    /// Get cache statistics (legacy - returns expression cache stats)
    #[deprecated(note = "Use expr_cache_stats() or template_cache_stats() instead")]
    pub fn cache_stats(&self) -> Option<nebula_memory::cache::CacheMetrics> {
        self.expr_cache_stats()
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
    fn test_parse_template() {
        let engine = ExpressionEngine::new();
        let template = engine.parse_template("Hello {{ $input }}!").unwrap();
        assert_eq!(template.expression_count(), 1);
    }

    #[test]
    fn test_render_template_simple() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::text("World"));

        let template = engine.parse_template("Hello {{ $input }}!").unwrap();
        let result = engine.render_template(&template, &context).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn test_render_template_multiple() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::text("Alice"));
        context.set_execution_var("order_id", Value::integer(12345));

        let template = engine
            .parse_template("Hello {{ $input }}! Your order #{{ $execution.order_id }} is ready.")
            .unwrap();
        let result = engine.render_template(&template, &context).unwrap();
        assert_eq!(result, "Hello Alice! Your order #12345 is ready.");
    }

    #[test]
    fn test_render_template_with_functions() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::text("john"));

        let template = engine
            .parse_template("User: {{ $input | uppercase() }}, Length: {{ length($input) }}")
            .unwrap();
        let result = engine.render_template(&template, &context).unwrap();
        assert_eq!(result, "User: JOHN, Length: 4");
    }

    #[test]
    fn test_render_template_html() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::integer(42));
        context.set_execution_var("name", Value::text("Alice"));

        let html = r#"<html>
  <h1>Welcome {{ $execution.name }}</h1>
  <p>Your score: {{ $input * 2 }}</p>
  <span>Total: {{ $input + 8 }}</span>
</html>"#;

        let template = engine.parse_template(html).unwrap();
        let result = engine.render_template(&template, &context).unwrap();
        assert!(result.contains("<h1>Welcome Alice</h1>"));
        assert!(result.contains("<p>Your score: 84</p>"));
        assert!(result.contains("<span>Total: 50</span>"));
    }

    #[test]
    fn test_render_template_with_literal_braces() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let template = engine
            .parse_template("Result: {{ 2 + 3 }}, another: {{ 10 * 2 }}")
            .unwrap();
        let result = engine.render_template(&template, &context).unwrap();
        assert_eq!(result, "Result: 5, another: 20");
    }

    #[test]
    fn test_render_template_no_expressions() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let template = engine
            .parse_template("Just plain text without expressions")
            .unwrap();
        let result = engine.render_template(&template, &context).unwrap();
        assert_eq!(result, "Just plain text without expressions");
    }

    #[test]
    fn test_template_reuse() {
        let engine = ExpressionEngine::new();
        let template = engine.parse_template("Hello {{ $input }}!").unwrap();

        let mut context = EvaluationContext::new();

        // First render
        context.set_input(Value::text("Alice"));
        let result1 = engine.render_template(&template, &context).unwrap();
        assert_eq!(result1, "Hello Alice!");

        // Second render with different context
        context.set_input(Value::text("Bob"));
        let result2 = engine.render_template(&template, &context).unwrap();
        assert_eq!(result2, "Hello Bob!");
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

        let result = engine
            .evaluate("{{ \"hello\" | uppercase() }}", &context)
            .unwrap();
        assert_eq!(result.as_str(), Some("HELLO"));
    }

    #[test]
    fn test_template_cache() {
        let engine = ExpressionEngine::with_cache_size(100);
        let mut context = EvaluationContext::new();
        context.set_input(Value::text("World"));

        let source = "Hello {{ $input }}!";

        // First parse (cache miss)
        let template1 = engine.parse_template(source).unwrap();
        let result1 = engine.render_template(&template1, &context).unwrap();
        assert_eq!(result1, "Hello World!");

        // Second parse (cache hit)
        let template2 = engine.parse_template(source).unwrap();
        let result2 = engine.render_template(&template2, &context).unwrap();
        assert_eq!(result2, "Hello World!");

        #[cfg(feature = "std")]
        {
            let stats = engine.template_cache_stats().unwrap();
            assert!(stats.hits > 0);
        }
    }

    #[test]
    fn test_separate_cache_sizes() {
        let engine = ExpressionEngine::with_cache_sizes(50, 100);
        let context = EvaluationContext::new();

        // Test expression cache
        let result = engine.evaluate("2 + 3", &context).unwrap();
        assert_eq!(result.as_integer(), Some(5));

        // Test template cache
        let template = engine.parse_template("Result: {{ 2 + 3 }}").unwrap();
        let result = engine.render_template(&template, &context).unwrap();
        assert_eq!(result, "Result: 5");

        #[cfg(feature = "std")]
        {
            assert!(engine.expr_cache_stats().is_some());
            assert!(engine.template_cache_stats().is_some());
        }
    }

    #[test]
    fn test_clear_template_cache() {
        let engine = ExpressionEngine::with_cache_size(100);
        let context = EvaluationContext::new();

        // Parse a template
        let _template = engine.parse_template("Hello {{ $input }}").unwrap();

        // Clear template cache only
        engine.clear_template_cache();

        // Expression cache should still work
        let result = engine.evaluate("2 + 3", &context).unwrap();
        assert_eq!(result.as_integer(), Some(5));
    }

    #[test]
    fn test_get_template_alias() {
        let engine = ExpressionEngine::new();
        let template1 = engine.parse_template("Test").unwrap();
        let template2 = engine.get_template("Test").unwrap();
        assert_eq!(template1.source(), template2.source());
    }
}
