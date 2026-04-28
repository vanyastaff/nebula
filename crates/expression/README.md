---
name: nebula-expression
role: Expression Evaluator (dynamic field resolution for workflow parameters)
status: stable
last-reviewed: 2026-04-28
canon-invariants: []
related: [nebula-schema, nebula-validator, nebula-core]
---

# nebula-expression

## Purpose

Workflow fields often need dynamic values — the output of a previous node, the current
execution id, or a date computed at runtime. Hardcoding those values in the workflow
definition is not feasible, and ad-hoc string interpolation in each integration author's
action is fragile. `nebula-expression` provides a single shared expression evaluator:
a small expression language (compatible with n8n syntax) that resolves `{{ expression }}`
templates against execution-time context, with a parse-once LRU cache for hot paths,
and typed wrapper types that let callers declare whether a field may carry an expression
or is always a literal.

## Role

**Expression Evaluator.** The resolution backend that `nebula-schema`'s proof-token
pipeline calls at the `ValidValues::resolve` step. Callers supply an `EvaluationContext`
(built from execution state at runtime); the engine evaluates the expression AST against
it and returns a `serde_json::Value`.

## Public API

- `ExpressionEngine` — main engine: `new()`, `with_cache_size(n)`, `evaluate(expr, ctx)`,
  `evaluate_template(tmpl, ctx)`, `parse_template(tmpl)`, `cache_overview()`.
- `EvaluationContext` — runtime variable bindings: `$node`, `$execution`, `$workflow`,
  `$input`; `EvaluationContextBuilder` for fluent construction.
- `EvaluationPolicy` — DoS budget (max steps, max recursion depth).
- `Template` — pre-parsed `{{ ... }}` template; call `.render(engine, ctx)` to evaluate.
- `MaybeExpression<T>` — typed wrapper: either a literal `T` or an expression string that
  resolves to `T`. Used in `serde` structs for action/credential config parameters.
- `MaybeTemplate` — like `MaybeExpression` but for text templates (`{{ }}` delimiters).
- `CachedExpression` — pre-compiled expression for reuse across evaluations.
- `ExpressionError`, `ExpressionResult` — typed error and result alias.
- `CacheOverview` — cache hit/miss statistics snapshot.

See `src/lib.rs` rustdoc for the quick-start example.

## Contract

- **Expression variables:** `$node`, `$execution`, `$workflow`, `$input` — the four
  standard execution-time variable namespaces. Seam: `crates/expression/src/context.rs`.
- **DoS guard:** `EvaluationPolicy` caps recursion depth (default 256) and step budget
  per evaluation call. Exceeding either returns `ExpressionError` rather than panicking
  or looping indefinitely.
- **Type coercion:** expressions evaluate to `serde_json::Value`; `MaybeExpression<T>`
  calls `resolve_as_*` which coerces the JSON result to `T` and returns a typed error on
  mismatch.

## Non-goals

- Not a validation rules engine — see `nebula-validator` for `Rule` and `Validate<T>`.
- Not a schema system — see `nebula-schema` for field definitions and the proof-token
  pipeline.
- Not a template engine for HTML rendering — it resolves `{{ }}` in workflow field strings;
  full HTML templating with control flow is out of scope.

### Known limitation: BuiltinFunction signature and evaluator re-entry

`BuiltinFunction` is typed as:

```rust
pub type BuiltinFunction = fn(&[Value], &Evaluator, &EvaluationContext) -> ExpressionResult<Value>;
```

The `&Evaluator` parameter lets built-in functions call `Evaluator::eval` recursively.
This is an intentional design choice for lambda-accepting functions (`filter`, `map`,
`reduce`) but it means a malicious or buggy built-in can re-enter the evaluator
arbitrarily. The `EvaluationPolicy` step budget partially guards against runaway
recursion, but cannot prevent all re-entry patterns. **Built-in functions must not be
authored by untrusted code** — they are first-party only. See memory note
`pitfall_expression_builtin_frame.md` for the full pitfall description. This constraint
should be promoted to `docs/pitfalls.md` before 1.0.

## Maturity

See `docs/MATURITY.md` row for `nebula-expression`.

- API stability: `stable` — `ExpressionEngine`, `EvaluationContext`, `Template`,
  `MaybeExpression`, and `MaybeTemplate` are in active use; no known planned breaking changes.
- `datetime` functions are feature-gated (`feature = "datetime"`); include if date
  arithmetic is needed.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5 (expression context used by resolution step).
- Glossary: `docs/GLOSSARY.md` §5 (`ResolvedValues` — produced after expression resolution).
- Siblings: `nebula-schema` (calls expression context via `ValidValues::resolve`),
  `nebula-validator` (rule engine), `nebula-core` (base types).

## Appendix

### Architecture overview

```
nebula-expression/
└── src/
    ├── lexer.rs          # Tokenizer
    ├── parser.rs         # Expression → AST
    ├── ast.rs            # Expression AST node types
    ├── eval.rs           # AST evaluator (Evaluator, EvalFrame)
    ├── builtins.rs       # BuiltinFunction registry
    ├── context.rs        # EvaluationContext + builder
    ├── template.rs       # Template / MaybeTemplate
    ├── engine.rs         # ExpressionEngine + LRU cache
    ├── maybe.rs          # MaybeExpression<T>
    ├── policy.rs         # EvaluationPolicy (DoS budget)
    └── error_formatter.rs  # Pretty error display with source context
```

Runnable examples live at the workspace root in `examples/expression_*.rs`,
not under `crates/expression/examples/`. Run them with:

```bash
cargo run -p nebula-examples --example expression_template_rendering
cargo run -p nebula-examples --example expression_maybe_vs_template
cargo run -p nebula-examples --example expression_template_advanced
cargo run -p nebula-examples --example expression_error_messages
```

### Whitespace control

Templates support `{{-` (strip left whitespace) and `-}}` (strip right whitespace):

```rust
let template = Template::new("Hello   {{- $input -}}!").unwrap();
// renders "HelloWorld!" — surrounding whitespace stripped
```
