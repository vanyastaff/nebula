//! Expression engine with caching support
//!
//! This module provides the main ExpressionEngine that parses and evaluates expressions,
//! with optional caching of parsed ASTs for improved performance.

use std::sync::Arc;
#[cfg(feature = "cache")]
use std::sync::atomic::{AtomicU64, Ordering};

use nebula_log::{debug, trace};
use serde_json::Value;

use crate::{
    ast::Expr, builtins::BuiltinRegistry, context::EvaluationContext, error::ExpressionResult,
    eval::Evaluator, lexer::Lexer, parser::Parser, policy::EvaluationPolicy,
};

/// Cache hit/miss statistics snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Total cache hits.
    pub hits: u64,
    /// Total cache misses.
    pub misses: u64,
}

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

/// Wrapper around moka cache with hit/miss counters.
#[cfg(feature = "cache")]
struct TrackedCache<
    K: std::hash::Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
> {
    inner: moka::sync::Cache<K, V>,
    hits: AtomicU64,
    misses: AtomicU64,
}

#[cfg(feature = "cache")]
impl<K: std::hash::Hash + Eq + Send + Sync + 'static, V: Clone + Send + Sync + 'static>
    TrackedCache<K, V>
{
    fn new(capacity: u64) -> Self {
        Self {
            inner: moka::sync::Cache::new(capacity),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    fn get(&self, key: &K) -> Option<V> {
        match self.inner.get(key) {
            Some(v) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                Some(v)
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    fn insert(&self, key: K, value: V) {
        self.inner.insert(key, value);
    }

    fn clear(&self) {
        self.inner.invalidate_all();
    }

    fn len(&self) -> usize {
        self.inner.entry_count() as usize
    }

    fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }
}

/// Expression engine with parsing and evaluation capabilities
pub struct ExpressionEngine {
    /// Cache for parsed expressions
    #[cfg(feature = "cache")]
    expr_cache: Option<TrackedCache<Arc<str>, Expr>>,
    /// Cache for parsed templates
    #[cfg(feature = "cache")]
    template_cache: Option<TrackedCache<Arc<str>, crate::Template>>,
    /// Builtin function registry
    builtins: Arc<BuiltinRegistry>,
    /// Optional engine-level evaluation policy.
    policy: Option<Arc<EvaluationPolicy>>,
    /// Evaluator
    evaluator: Evaluator,
}

impl ExpressionEngine {
    #[cfg(feature = "cache")]
    fn create(
        expr_cache: Option<TrackedCache<Arc<str>, Expr>>,
        template_cache: Option<TrackedCache<Arc<str>, crate::Template>>,
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

    #[cfg(not(feature = "cache"))]
    fn create(policy: Option<Arc<EvaluationPolicy>>) -> Self {
        let builtins = Arc::new(BuiltinRegistry::new());
        let evaluator = Evaluator::with_policy(Arc::clone(&builtins), policy.clone());

        Self {
            builtins,
            policy,
            evaluator,
        }
    }

    /// Create a new expression engine with default configuration (no caching)
    pub fn new() -> Self {
        #[cfg(feature = "cache")]
        {
            Self::create(None, None, None)
        }
        #[cfg(not(feature = "cache"))]
        {
            Self::create(None)
        }
    }

    /// Create a new expression engine with caching for both expressions and templates
    #[cfg(feature = "cache")]
    pub fn with_cache_size(size: usize) -> Self {
        let expr_cache = TrackedCache::new(size as u64);
        let template_cache = TrackedCache::new(size as u64);

        debug!(
            cache_size = size,
            "Created expression engine with moka caches"
        );

        Self::create(Some(expr_cache), Some(template_cache), None)
    }

    /// Create expression engine with separate cache sizes for expressions and templates
    #[cfg(feature = "cache")]
    pub fn with_cache_sizes(expr_cache_size: usize, template_cache_size: usize) -> Self {
        let expr_cache = TrackedCache::new(expr_cache_size as u64);
        let template_cache = TrackedCache::new(template_cache_size as u64);

        debug!(
            expr_cache_size = expr_cache_size,
            template_cache_size = template_cache_size,
            "Created expression engine with moka caches"
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

        // Parse the expression (with caching if enabled)
        #[cfg(feature = "cache")]
        let ast = if let Some(cache) = &self.expr_cache {
            let key: Arc<str> = Arc::from(expression);
            if let Some(cached) = cache.get(&key) {
                cached
            } else {
                let parsed = self.parse_expression(expression)?;
                cache.insert(key, parsed.clone());
                parsed
            }
        } else {
            self.parse_expression(expression)?
        };

        #[cfg(not(feature = "cache"))]
        let ast = self.parse_expression(expression)?;

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

        #[cfg(feature = "cache")]
        if let Some(cache) = &self.template_cache {
            let key: Arc<str> = Arc::from(source_str.as_str());
            if let Some(cached) = cache.get(&key) {
                return Ok(cached);
            }
            let template = crate::Template::new(&source_str)?;
            cache.insert(key, template.clone());
            return Ok(template);
        }

        crate::Template::new(source_str)
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
        #[cfg(feature = "cache")]
        {
            if let Some(cache) = &self.expr_cache {
                cache.clear();
                debug!("Expression cache cleared");
            }
            if let Some(cache) = &self.template_cache {
                cache.clear();
                debug!("Template cache cleared");
            }
        }
    }

    /// Clear expression cache only
    pub fn clear_expr_cache(&self) {
        #[cfg(feature = "cache")]
        if let Some(cache) = &self.expr_cache {
            cache.clear();
            debug!("Expression cache cleared");
        }
    }

    /// Clear template cache only
    pub fn clear_template_cache(&self) {
        #[cfg(feature = "cache")]
        if let Some(cache) = &self.template_cache {
            cache.clear();
            debug!("Template cache cleared");
        }
    }

    /// Get expression cache size
    pub fn expr_cache_size(&self) -> Option<usize> {
        #[cfg(feature = "cache")]
        {
            self.expr_cache.as_ref().map(|cache| cache.len())
        }
        #[cfg(not(feature = "cache"))]
        {
            None
        }
    }

    /// Get template cache size
    pub fn template_cache_size(&self) -> Option<usize> {
        #[cfg(feature = "cache")]
        {
            self.template_cache.as_ref().map(|cache| cache.len())
        }
        #[cfg(not(feature = "cache"))]
        {
            None
        }
    }

    /// Return a lightweight cache snapshot for observability.
    pub fn cache_overview(&self) -> CacheOverview {
        #[cfg(feature = "cache")]
        {
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
        #[cfg(not(feature = "cache"))]
        {
            CacheOverview {
                expr_cache_enabled: false,
                template_cache_enabled: false,
                expr_entries: 0,
                template_entries: 0,
                expr_hits: 0,
                expr_misses: 0,
                template_hits: 0,
                template_misses: 0,
            }
        }
    }

    /// Get a point-in-time snapshot of expression cache statistics.
    ///
    /// Returns `None` if expression caching is disabled.
    pub fn expr_cache_stats(&self) -> Option<CacheStats> {
        #[cfg(feature = "cache")]
        {
            self.expr_cache.as_ref().map(|c| c.stats())
        }
        #[cfg(not(feature = "cache"))]
        {
            None
        }
    }

    /// Get a point-in-time snapshot of template cache statistics.
    ///
    /// Returns `None` if template caching is disabled.
    pub fn template_cache_stats(&self) -> Option<CacheStats> {
        #[cfg(feature = "cache")]
        {
            self.template_cache.as_ref().map(|c| c.stats())
        }
        #[cfg(not(feature = "cache"))]
        {
            None
        }
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
    fn test_custom_function_registration() {
        let mut engine = ExpressionEngine::new();
        engine.register_function("constant_one", constant_one);

        let context = EvaluationContext::new();
        let result = engine.evaluate("constant_one()", &context).unwrap();
        assert_eq!(result.as_i64(), Some(1));
    }

    #[test]
    fn test_function_allowlist_blocks_disallowed() {
        let engine = ExpressionEngine::new().restrict_to_functions(["length"]);
        let ctx = EvaluationContext::new();

        // length is allowed
        assert!(engine.evaluate("length('hi')", &ctx).is_ok());

        // uppercase is not
        assert!(engine.evaluate("uppercase('hi')", &ctx).is_err());
    }

    #[test]
    fn test_policy_getter() {
        let engine = ExpressionEngine::new();
        assert!(engine.policy().is_none());

        let engine = engine.with_policy(EvaluationPolicy::allow_only(["length"]));
        assert!(engine.policy().is_some());
    }

    #[test]
    #[cfg(feature = "cache")]
    fn test_cache_operations() {
        let engine = ExpressionEngine::with_cache_size(100);
        let context = EvaluationContext::new();

        // First call — miss, parses and caches
        let r1 = engine.evaluate("1 + 1", &context).unwrap();
        // Second call — hit, returns cached AST
        let r2 = engine.evaluate("1 + 1", &context).unwrap();
        assert_eq!(r1, r2);

        let stats = engine.expr_cache_stats().unwrap();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);

        // Clear and verify stats reset doesn't affect counters
        engine.clear_cache();
    }

    #[test]
    fn test_cache_overview_no_cache() {
        let engine = ExpressionEngine::new();
        let overview = engine.cache_overview();
        assert!(!overview.expr_cache_enabled);
        assert!(!overview.template_cache_enabled);
    }
}
