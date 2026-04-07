//! Expression engine with caching support
//!
//! This module provides the main ExpressionEngine that parses and evaluates expressions,
//! with optional caching of parsed ASTs for improved performance.

use crate::ast::Expr;
use crate::builtins::BuiltinRegistry;
use crate::context::EvaluationContext;
use crate::error::ExpressionResult;
use crate::eval::Evaluator;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::policy::EvaluationPolicy;
use nebula_log::{debug, trace};
use nebula_memory::cache::{CacheConfig, CacheStats, ConcurrentComputeCache};
use serde_json::Value;
use std::sync::Arc;

/// Lightweight cache observability snapshot for `ExpressionEngine`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CacheOverview {
    /// Whether expression parsing cache is enabled.
    pub expr_cache_enabled: bool,
    /// Whether template parsing cache is enabled.
    pub template_cache_enabled: bool,
    /// Current number of entries in expression cache.
    pub expr_entries: usize,
    /// Current number of entries in template cache.
    pub template_entries: usize,
    /// Expression cache hits since last reset.
    pub expr_hits: u64,
    /// Expression cache misses since last reset.
    pub expr_misses: u64,
    /// Template cache hits since last reset.
    pub template_hits: u64,
    /// Template cache misses since last reset.
    pub template_misses: u64,
}

/// Expression engine with parsing and evaluation capabilities
pub struct ExpressionEngine {
    /// Cache for parsed expressions (lock-free concurrent cache)
    expr_cache: Option<ConcurrentComputeCache<Arc<str>, Expr>>,
    /// Cache for parsed templates (lock-free concurrent cache)
    template_cache: Option<ConcurrentComputeCache<Arc<str>, crate::Template>>,
    /// Builtin function registry
    builtins: Arc<BuiltinRegistry>,
    /// Optional engine-level evaluation policy.
    policy: Option<Arc<EvaluationPolicy>>,
    /// Evaluator
    evaluator: Evaluator,
}

impl ExpressionEngine {
    fn create(
        expr_cache: Option<ConcurrentComputeCache<Arc<str>, Expr>>,
        template_cache: Option<ConcurrentComputeCache<Arc<str>, crate::Template>>,
        policy: Option<Arc<EvaluationPolicy>>,
    ) -> Self {
        let builtins = Arc::new(BuiltinRegistry::new());
        let evaluator = Evaluator::with_policy(Arc::clone(&builtins), policy.clone());

        Self {
            expr_cache,
            template_cache,
            builtins,
            policy,
            evaluator,
        }
    }

    /// Create a new expression engine with default configuration (no caching)
    pub fn new() -> Self {
        Self::create(None, None, None)
    }

    /// Create a new expression engine with caching for both expressions and templates
    pub fn with_cache_size(size: usize) -> Self {
        let expr_config = CacheConfig::new(size);
        let expr_cache = ConcurrentComputeCache::with_config(expr_config);

        let template_config = CacheConfig::new(size);
        let template_cache = ConcurrentComputeCache::with_config(template_config);

        debug!(
            cache_size = size,
            "Created expression engine with lock-free concurrent caches"
        );

        Self::create(Some(expr_cache), Some(template_cache), None)
    }

    /// Create expression engine with separate cache sizes for expressions and templates
    pub fn with_cache_sizes(expr_cache_size: usize, template_cache_size: usize) -> Self {
        let expr_config = CacheConfig::new(expr_cache_size);
        let expr_cache = ConcurrentComputeCache::with_config(expr_config);

        let template_config = CacheConfig::new(template_cache_size);
        let template_cache = ConcurrentComputeCache::with_config(template_config);

        debug!(
            expr_cache_size = expr_cache_size,
            template_cache_size = template_cache_size,
            "Created expression engine with lock-free concurrent caches"
        );

        Self::create(Some(expr_cache), Some(template_cache), None)
    }

    /// Restrict the engine to a specific set of allowed builtin function names.
    ///
    /// When set, any function call outside this allowlist fails with an evaluation error.
    /// Use canonical names (`every`, `some`) or aliases (`all`, `any`) for higher-order functions.
    pub fn restrict_to_functions<I, S>(mut self, allowed_functions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.policy = Some(Arc::new(EvaluationPolicy::allow_only(allowed_functions)));
        self.rebuild_evaluator();
        self
    }

    /// Set an engine-level policy.
    pub fn with_policy(mut self, policy: EvaluationPolicy) -> Self {
        self.policy = Some(Arc::new(policy));
        self.rebuild_evaluator();
        self
    }

    /// Return the current engine-level policy, if configured.
    pub fn policy(&self) -> Option<&EvaluationPolicy> {
        self.policy.as_deref()
    }

    fn rebuild_evaluator(&mut self) {
        self.evaluator = Evaluator::with_policy(Arc::clone(&self.builtins), self.policy.clone());
    }

    /// Register a custom builtin function.
    ///
    /// This method is safe to call after the engine has been used. Internally,
    /// it performs copy-on-write on the builtin registry when needed and then
    /// rebuilds the evaluator so subsequent evaluations observe the new function.
    pub fn register_function(
        &mut self,
        name: impl AsRef<str>,
        func: crate::builtins::BuiltinFunction,
    ) {
        Arc::make_mut(&mut self.builtins).register(name, func);
        self.rebuild_evaluator();
    }

    /// Evaluate an expression string in the given context
    pub fn evaluate(
        &self,
        expression: &str,
        context: &EvaluationContext,
    ) -> ExpressionResult<Value> {
        trace!(expression = expression, "Evaluating expression");

        // Parse the expression (with lock-free caching if enabled)
        let ast = if let Some(cache) = &self.expr_cache {
            let key: Arc<str> = Arc::from(expression);
            cache.get_or_compute(key, || {
                self.parse_expression(expression).map_err(|_| {
                    nebula_memory::MemoryError::invalid_layout("parse expression failed")
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

        // Use lock-free cache if available
        if let Some(cache) = &self.template_cache {
            let key: Arc<str> = Arc::from(source_str.as_str());
            let template = cache.get_or_compute(key, || {
                crate::Template::new(&source_str).map_err(|_| {
                    nebula_memory::MemoryError::invalid_layout("template creation failed")
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
                trimmed[2..trimmed.len() - 2].trim()
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
            cache.clear();
            debug!("Expression cache cleared");
        }
        if let Some(cache) = &self.template_cache {
            cache.clear();
            debug!("Template cache cleared");
        }
    }

    /// Clear expression cache only
    pub fn clear_expr_cache(&self) {
        if let Some(cache) = &self.expr_cache {
            cache.clear();
            debug!("Expression cache cleared");
        }
    }

    /// Clear template cache only
    pub fn clear_template_cache(&self) {
        if let Some(cache) = &self.template_cache {
            cache.clear();
            debug!("Template cache cleared");
        }
    }

    /// Get expression cache size
    pub fn expr_cache_size(&self) -> Option<usize> {
        self.expr_cache.as_ref().map(|cache| cache.len())
    }

    /// Get template cache size
    pub fn template_cache_size(&self) -> Option<usize> {
        self.template_cache.as_ref().map(|cache| cache.len())
    }

    /// Return a lightweight cache snapshot for observability.
    pub fn cache_overview(&self) -> CacheOverview {
        let expr_stats = self.expr_cache.as_ref().map(|c| c.stats());
        let tmpl_stats = self.template_cache.as_ref().map(|c| c.stats());

        CacheOverview {
            expr_cache_enabled: self.expr_cache.is_some(),
            template_cache_enabled: self.template_cache.is_some(),
            expr_entries: self.expr_cache.as_ref().map_or(0, |c| c.len()),
            template_entries: self.template_cache.as_ref().map_or(0, |c| c.len()),
            expr_hits: expr_stats.as_ref().map_or(0, |s| s.hits),
            expr_misses: expr_stats.as_ref().map_or(0, |s| s.misses),
            template_hits: tmpl_stats.as_ref().map_or(0, |s| s.hits),
            template_misses: tmpl_stats.as_ref().map_or(0, |s| s.misses),
        }
    }

    /// Get a point-in-time snapshot of expression cache statistics.
    ///
    /// Returns `None` if expression caching is disabled.
    pub fn expr_cache_stats(&self) -> Option<CacheStats> {
        self.expr_cache.as_ref().map(|c| c.stats())
    }

    /// Get a point-in-time snapshot of template cache statistics.
    ///
    /// Returns `None` if template caching is disabled.
    pub fn template_cache_stats(&self) -> Option<CacheStats> {
        self.template_cache.as_ref().map(|c| c.stats())
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
    use crate::EvaluationPolicy;

    fn constant_one(
        _args: &[Value],
        _evaluator: &crate::eval::Evaluator,
        _context: &EvaluationContext,
    ) -> ExpressionResult<Value> {
        Ok(Value::from(1))
    }

    #[test]
    fn test_evaluate_literal() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine.evaluate("42", &context).unwrap();
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn test_evaluate_arithmetic() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine.evaluate("2 + 3 * 4", &context).unwrap();
        assert_eq!(result.as_i64(), Some(14));
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
        assert_eq!(result.as_i64(), Some(5));
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
        context.set_input(Value::String("World".to_string()));

        let template = engine.parse_template("Hello {{ $input }}!").unwrap();
        let result = engine.render_template(&template, &context).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn test_render_template_multiple() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("Alice".to_string()));
        context.set_execution_var("order_id", Value::Number(12345.into()));

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
        context.set_input(Value::String("john".to_string()));

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
        context.set_input(Value::Number(42.into()));
        context.set_execution_var("name", Value::String("Alice".to_string()));

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
        context.set_input(Value::String("Alice".to_string()));
        let result1 = engine.render_template(&template, &context).unwrap();
        assert_eq!(result1, "Hello Alice!");

        // Second render with different context
        context.set_input(Value::String("Bob".to_string()));
        let result2 = engine.render_template(&template, &context).unwrap();
        assert_eq!(result2, "Hello Bob!");
    }

    #[test]
    fn test_evaluate_variable() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_execution_var("id", Value::String("test-123".to_string()));

        let result = engine.evaluate("$execution.id", &context).unwrap();
        assert_eq!(result.as_str(), Some("test-123"));
    }

    #[test]
    fn test_with_cache() {
        let engine = ExpressionEngine::with_cache_size(100);
        let context = EvaluationContext::new();

        // First evaluation
        let result1 = engine.evaluate("2 + 3", &context).unwrap();
        assert_eq!(result1.as_i64(), Some(5));

        // Second evaluation (should use cache)
        let result2 = engine.evaluate("2 + 3", &context).unwrap();
        assert_eq!(result2.as_i64(), Some(5));
    }

    #[test]
    fn test_conditional() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let result = engine.evaluate("if true then 1 else 2", &context).unwrap();
        assert_eq!(result.as_i64(), Some(1));

        let result = engine.evaluate("if false then 1 else 2", &context).unwrap();
        assert_eq!(result.as_i64(), Some(2));
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
        context.set_input(Value::String("World".to_string()));

        let source = "Hello {{ $input }}!";

        // First parse (cache miss)
        let template1 = engine.parse_template(source).unwrap();
        let result1 = engine.render_template(&template1, &context).unwrap();
        assert_eq!(result1, "Hello World!");

        // Second parse (cache hit)
        let template2 = engine.parse_template(source).unwrap();
        let result2 = engine.render_template(&template2, &context).unwrap();
        assert_eq!(result2, "Hello World!");
    }

    #[test]
    fn test_separate_cache_sizes() {
        let engine = ExpressionEngine::with_cache_sizes(50, 100);
        let context = EvaluationContext::new();

        // Test expression cache
        let result = engine.evaluate("2 + 3", &context).unwrap();
        assert_eq!(result.as_i64(), Some(5));

        // Test template cache
        let template = engine.parse_template("Result: {{ 2 + 3 }}").unwrap();
        let result = engine.render_template(&template, &context).unwrap();
        assert_eq!(result, "Result: 5");
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
        assert_eq!(result.as_i64(), Some(5));
    }

    #[test]
    fn test_get_template_alias() {
        let engine = ExpressionEngine::new();
        let template1 = engine.parse_template("Test").unwrap();
        let template2 = engine.get_template("Test").unwrap();
        assert_eq!(template1.source(), template2.source());
    }

    #[test]
    fn test_register_function_before_evaluate() {
        let mut engine = ExpressionEngine::new();
        engine.register_function("constant_one", constant_one);

        let context = EvaluationContext::new();
        let result = engine.evaluate("constant_one()", &context).unwrap();
        assert_eq!(result.as_i64(), Some(1));
    }

    #[test]
    fn test_register_function_after_evaluate() {
        let mut engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let baseline = engine.evaluate("1 + 1", &context).unwrap();
        assert_eq!(baseline.as_i64(), Some(2));

        engine.register_function("constant_one", constant_one);

        let result = engine.evaluate("constant_one()", &context).unwrap();
        assert_eq!(result.as_i64(), Some(1));
    }

    #[test]
    fn test_function_allowlist_permits_allowed_function() {
        let engine = ExpressionEngine::new().restrict_to_functions(["uppercase"]);
        let context = EvaluationContext::new();

        let result = engine.evaluate("uppercase('hello')", &context).unwrap();
        assert_eq!(result.as_str(), Some("HELLO"));
    }

    #[test]
    fn test_function_allowlist_blocks_disallowed_function() {
        let engine = ExpressionEngine::new().restrict_to_functions(["uppercase"]);
        let context = EvaluationContext::new();

        let err = engine.evaluate("lowercase('HELLO')", &context).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not allowed by policy"));
        assert!(msg.contains("lowercase"));
    }

    #[test]
    fn test_policy_deny_takes_precedence_over_allow() {
        let policy = EvaluationPolicy::new()
            .with_allowed_functions(["uppercase", "lowercase"])
            .with_denied_functions(["lowercase"]);
        let engine = ExpressionEngine::new().with_policy(policy);
        let context = EvaluationContext::new();

        let err = engine.evaluate("lowercase('HELLO')", &context).unwrap_err();
        assert!(err.to_string().contains("denied by policy"));
    }

    #[test]
    fn test_context_policy_restricts_engine_policy() {
        let engine = ExpressionEngine::new()
            .with_policy(EvaluationPolicy::new().with_allowed_functions(["uppercase", "length"]));

        let mut context = EvaluationContext::new();
        context.set_policy(EvaluationPolicy::allow_only(["length"]));

        let err = engine.evaluate("uppercase('hello')", &context).unwrap_err();
        assert!(err.to_string().contains("not allowed by policy"));
    }

    #[test]
    fn test_engine_policy_strict_mode_flag_exposed() {
        let engine =
            ExpressionEngine::new().with_policy(EvaluationPolicy::new().with_strict_mode(true));
        assert!(engine.policy().unwrap().strict_mode());
    }

    #[test]
    fn test_strict_mode_requires_boolean_condition() {
        let engine =
            ExpressionEngine::new().with_policy(EvaluationPolicy::new().with_strict_mode(true));
        let context = EvaluationContext::new();

        let err = engine
            .evaluate("if 1 then 'yes' else 'no'", &context)
            .unwrap_err();
        assert!(err.to_string().contains("expected boolean"));
    }

    #[test]
    fn test_strict_mode_requires_boolean_logical_operands() {
        let engine =
            ExpressionEngine::new().with_policy(EvaluationPolicy::new().with_strict_mode(true));
        let context = EvaluationContext::new();

        let err = engine.evaluate("1 && true", &context).unwrap_err();
        assert!(err.to_string().contains("expected boolean"));
    }

    #[test]
    fn test_strict_mode_requires_boolean_higher_order_predicates() {
        let engine =
            ExpressionEngine::new().with_policy(EvaluationPolicy::new().with_strict_mode(true));
        let context = EvaluationContext::new();

        let err = engine
            .evaluate("filter([1,2,3], x => x)", &context)
            .unwrap_err();
        assert!(err.to_string().contains("expected boolean"));
    }

    #[test]
    fn test_strict_mode_rejects_numeric_string_for_math_builtin() {
        let engine =
            ExpressionEngine::new().with_policy(EvaluationPolicy::new().with_strict_mode(true));
        let context = EvaluationContext::new();

        let err = engine.evaluate("sqrt('9')", &context).unwrap_err();
        assert!(err.to_string().contains("strict mode"));
    }

    #[test]
    fn test_non_strict_mode_allows_numeric_string_for_math_builtin() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let value = engine.evaluate("sqrt('9')", &context).unwrap();
        assert_eq!(value.as_f64(), Some(3.0));
    }

    #[test]
    fn test_strict_mode_still_allows_explicit_to_number_by_default() {
        let engine =
            ExpressionEngine::new().with_policy(EvaluationPolicy::new().with_strict_mode(true));
        let context = EvaluationContext::new();

        let value = engine.evaluate("to_number('42')", &context).unwrap();
        assert_eq!(value.as_f64(), Some(42.0));
    }

    #[test]
    fn test_strict_conversion_functions_reject_non_numeric_to_number() {
        let policy = EvaluationPolicy::new()
            .with_strict_mode(true)
            .with_strict_conversion_functions(true);
        let engine = ExpressionEngine::new().with_policy(policy);
        let context = EvaluationContext::new();

        let err = engine.evaluate("to_number('42')", &context).unwrap_err();
        assert!(err.to_string().contains("expected number"));
    }

    #[test]
    fn test_strict_conversion_functions_reject_non_boolean_to_boolean() {
        let policy = EvaluationPolicy::new().with_strict_conversion_functions(true);
        let engine = ExpressionEngine::new().with_policy(policy);
        let context = EvaluationContext::new();

        let err = engine.evaluate("to_boolean(1)", &context).unwrap_err();
        assert!(err.to_string().contains("expected boolean"));
    }

    #[test]
    fn test_strict_conversion_functions_reject_complex_to_string() {
        let policy = EvaluationPolicy::new().with_strict_conversion_functions(true);
        let engine = ExpressionEngine::new().with_policy(policy);
        let context = EvaluationContext::new();

        let err = engine.evaluate("to_string([1,2,3])", &context).unwrap_err();
        assert!(err.to_string().contains("scalar"));
    }

    #[test]
    fn test_non_strict_conversion_allows_complex_to_string() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let value = engine.evaluate("to_string([1,2,3])", &context).unwrap();
        assert_eq!(value.as_str(), Some("[1,2,3]"));
    }

    #[test]
    fn test_strict_conversion_functions_reject_scalar_parse_json_result() {
        let policy = EvaluationPolicy::new().with_strict_conversion_functions(true);
        let engine = ExpressionEngine::new().with_policy(policy);
        let context = EvaluationContext::new();

        let err = engine.evaluate("parse_json('42')", &context).unwrap_err();
        assert!(err.to_string().contains("expected object or array"));
    }

    #[test]
    fn test_policy_max_json_parse_length_restricts_parse_json() {
        let policy = EvaluationPolicy::new().with_max_json_parse_length(5);
        let engine = ExpressionEngine::new().with_policy(policy);
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("{\"a\":1}".to_string()));

        let err = engine.evaluate("parse_json($input)", &context).unwrap_err();
        assert!(err.to_string().contains("JSON string too large"));
    }

    #[test]
    fn test_default_allows_string_relational_comparison() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let value = engine.evaluate("'b' > 'a'", &context).unwrap();
        assert_eq!(value.as_bool(), Some(true));
    }

    #[test]
    fn test_strict_numeric_comparisons_reject_string_relational_comparison() {
        let policy = EvaluationPolicy::new().with_strict_numeric_comparisons(true);
        let engine = ExpressionEngine::new().with_policy(policy);
        let context = EvaluationContext::new();

        let err = engine.evaluate("'b' > 'a'", &context).unwrap_err();
        assert!(err.to_string().contains("expected number"));
    }

    #[test]
    fn test_strict_numeric_comparisons_allow_numeric_relational_comparison() {
        let policy = EvaluationPolicy::new().with_strict_numeric_comparisons(true);
        let engine = ExpressionEngine::new().with_policy(policy);
        let context = EvaluationContext::new();

        let value = engine.evaluate("3 > 2", &context).unwrap();
        assert_eq!(value.as_bool(), Some(true));
    }

    #[test]
    fn test_cache_overview_no_cache() {
        let engine = ExpressionEngine::new();
        let overview = engine.cache_overview();
        assert!(!overview.expr_cache_enabled);
        assert!(!overview.template_cache_enabled);
        assert_eq!(overview.expr_entries, 0);
        assert_eq!(overview.template_entries, 0);
        assert_eq!(overview.expr_hits, 0);
        assert_eq!(overview.expr_misses, 0);
    }

    #[test]
    fn test_cache_overview_with_cache_entries() {
        let engine = ExpressionEngine::with_cache_size(100);
        let context = EvaluationContext::new();

        let _ = engine.evaluate("2 + 3", &context).unwrap();
        let _ = engine.parse_template("Hello {{ $input }}!").unwrap();

        let overview = engine.cache_overview();
        assert!(overview.expr_cache_enabled);
        assert!(overview.template_cache_enabled);
        assert!(overview.expr_entries >= 1);
        assert!(overview.template_entries >= 1);
    }

    #[test]
    fn test_cache_stats_populated_after_evaluation() {
        let engine = ExpressionEngine::with_cache_size(100);
        let context = EvaluationContext::new();

        // First eval = miss, second eval = hit
        let _ = engine.evaluate("2 + 3", &context).unwrap();
        let _ = engine.evaluate("2 + 3", &context).unwrap();

        let stats = engine.expr_cache_stats().expect("cache should be enabled");
        assert!(
            stats.hits >= 1,
            "expected at least 1 hit, got {}",
            stats.hits
        );
        assert!(
            stats.misses >= 1,
            "expected at least 1 miss, got {}",
            stats.misses
        );

        let overview = engine.cache_overview();
        assert!(overview.expr_hits >= 1);
        assert!(overview.expr_misses >= 1);
    }
}
