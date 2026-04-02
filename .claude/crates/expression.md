# nebula-expression
n8n-compatible expression language evaluating `serde_json::Value` — used in workflow node parameter interpolation.

## Invariants
- Syntax is n8n-compatible: `$node.data`, `$execution.id`, `$input`, etc.
- Template delimiter is `{{ expression }}`. Outside delimiters is literal text.
- All values are `serde_json::Value` — no typed coercion at the expression layer.

## Key Decisions
- `ExpressionEngine::with_cache_size(N)` caches parsed ASTs by expression string. Use for hot paths.
- `MaybeExpression` / `MaybeTemplate` are optimization types — skip parsing for static (non-expression) values.
- `EvaluationPolicy` controls error handling on undefined variables.
- `expr_cache_stats()` / `template_cache_stats()` return `Option<CacheStats>` (from nebula-memory) with real hit/miss/eviction data.
- `CacheOverview` includes `expr_hits`, `expr_misses`, `template_hits`, `template_misses` for quick observability.

## Traps
- `ast`, `lexer`, `parser`, `eval`, `token`, `interner`, `span` modules are `#[doc(hidden)]` — unstable, not public API.
- `Template` != `ExpressionEngine::evaluate`. Templates process multiple `{{ }}` in a string; `evaluate` handles one expression.
- `EvaluationContext` is built per-execution, not reused across executions.

## Relations
- Depends on nebula-memory (feature: `cache`) for `ConcurrentComputeCache`, `CacheConfig`, `CacheStats`.
- Used by nebula-workflow and nebula-engine for dynamic parameter resolution.

<!-- reviewed: 2026-03-30 — derive Classify migration -->

<!-- reviewed: 2026-04-02 -->
