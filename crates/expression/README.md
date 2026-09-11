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

- `CompiledProgram` — immutable retained syntax: `compile(source)` (auto),
  `compile_expression(source)` (raw), `compile_template(source)` (text),
  `compile_with_syntax(source, syntax)`, `source()`, and `syntax()`.
- `ProgramSyntax` - closed authored grammar: `Auto`, `Expression` (raw), or
  `Template` (always string). Retained independently of the resulting AST/body.
- `has_expression_marker(source)` — lexical shorthand classification, including
  malformed unescaped openers; neither a `$` nor a closing delimiter is required.
- `ExpressionEngine` — main engine: `evaluate(source, ctx)`,
  `evaluate_compiled(&program, ctx)`, `parse_template(source)`,
  `render_template(&template, ctx)`. Cache constructors require `cache`.
- `EvaluationContext` — runtime variable bindings: `$node`, `$execution`, `$workflow`,
  `$input`; `resolve_variable` returns shared `Arc<Value>` snapshots, and
  `EvaluationContextBuilder` provides fluent construction.
- `EvaluationPolicy` — function restrictions, coercion rules, work and JSON input limits.
- `BuiltinOutput`, `BuiltinOutputBuilder`, `BuiltinOutputBound`, `BuiltinOutputLimits` — mandatory bounded
  construction for outputs returned by public custom builtins.
- `Template` — pre-parsed `{{ ... }}` template; call `.render(engine, ctx)` to evaluate.
- `MaybeExpression<T>` — typed wrapper: either a literal `T` or an expression string that
  resolves to `T`. Used in `serde` structs for action/credential config parameters.
- `MaybeTemplate` — like `MaybeExpression` but for text templates (`{{ }}` delimiters).
- `CachedExpression` — opaque lazy program storage inside `MaybeExpression`; clones
  share the compiled program independently of the optional engine cache.
- `ExpressionError`, `ExpressionResult` — typed error and result alias.
- `CacheOverview` — cache hit/miss statistics snapshot.

See `src/lib.rs` rustdoc for the quick-start example.

## Contract

- **Expression variables:** `$node`, `$execution`, `$workflow`, `$input` — the four
  standard execution-time variable namespaces. Seam: `crates/expression/src/context.rs`.
- **Compilation boundary:** compile once and retain `CompiledProgram`; evaluation never
  reparses its source. Compilation checks syntax, not variable existence, builtin
  availability, policy, or expected result type. Those remain runtime checks.
  Clones preserve exact source and authored `ProgramSyntax`; AUTO remains AUTO
  even when compilation selects a template body. Persist both source and syntax
  when reconstructing authoring input, never infer syntax from the body or source.
- **Grammar:** auto compilation tries raw expressions first, so quoted `{{` and `}}`
  remain string contents. A lone envelope with surrounding whitespace preserves its
  JSON type; mixed text returns a string. Explicit template compilation always returns
  a string, including static text and lone envelopes. Delimiters respect quotes and
  nested object braces. `parse_expression` uses the same auto compiler and discards
  the program. Template text accepts `\{{` and `{{{{` as escaped literal openers;
  an even number of preceding backslashes leaves the opener active. Marker
  classification and template parsing share these lexical rules.
- **Missing versus null:** missing variables/properties are lookup errors; explicit
  JSON null remains `Value::Null`. No missing-value sentinel is collapsed into null.
- **DoS guard:** source is capped at 1 MiB, with at most 65,536 tokens and 16,384 AST
  nodes per embedded expression, depth 256, and 1,000 template expressions. Evaluation
  has a default 100,000-unit work ceiling shared by all template parts, pipelines,
  higher-order bodies, materialized values, string bytes, and builtin work. Context
  restrictions intersect engine restrictions and cannot raise engine limits, including
  the default 1 MiB JSON input limit. String expansion is checked before allocation in
  `split`, `join`, `to_json`, `replace`, `repeat`, padding, and other expanding
  string operations; template output is capped at 1 MiB. Every builtin result also
  has finite total-byte, single-string, collection-item, value-node, and depth bounds.
- **Shared context reads:** stored variables resolve through O(1) `Arc` clones. The
  evaluator retains borrows through property/index chains and passes borrowed values to
  builtins, materializing ownership only when an expression produces a new value or at
  the existing top-level owned-`Value` boundary.
- **Numbers:** signed/unsigned integer and float ordering uses `num-cmp` exact mixed
  comparison, including values above 2^53; numeric scalar equality uses the same order.
  Integer `+`, `-`, `*`, `%` and integral math preserve representable JSON integers or
  fail on overflow. Fractional/out-of-range integer conversions and non-finite results
  fail instead of truncating, saturating, or returning null. Floating arithmetic remains
  IEEE-754, not decimal arithmetic. Arrays/objects retain structural JSON equality.
- **Type coercion:** expressions evaluate to `serde_json::Value`; `MaybeExpression<T>`
  calls `resolve_as_*` which coerces the JSON result to `T` and returns a typed error on
  mismatch.

## Non-goals

- Not a validation rules engine — see `nebula-validator` for `Rule` and `Validate<T>`.
- Not a schema system — see `nebula-schema` for field definitions and the proof-token
  pipeline.
- Not a template engine for HTML rendering — it resolves `{{ }}` in workflow field strings;
  full HTML templating with control flow is out of scope.

### BuiltinFunction signature (no re-entry)

`BuiltinFunction` is typed as:

```rust
pub type BuiltinFunction =
    fn(
        &[&Value],
        BuiltinView<'_>,
        &EvaluationContext,
        BuiltinOutputBuilder,
    ) -> ExpressionResult<BuiltinOutput>;
```

`BuiltinView<'_>` exposes policy queries plus
`charge_work` and `check_output_bytes` against the calling program's shared budget.
The mandatory `BuiltinOutputBuilder` is the only public way to construct the opaque
result, and validates total bytes, string bytes, collection size, value nodes, and
depth. Registered functions should charge work before loops and use builder methods
that preflight allocations. `BuiltinView` does not expose
`Evaluator::eval` — a registered builtin physically cannot recurse back into AST
evaluation, so the historical step-budget bypass that was previously a "discipline-only"
rule (issue #252) is now type-enforced. The pitfall is documented in
`docs/pitfalls.md` for historical context.

Higher-order combinators (`filter`, `map`, `reduce`, `flat_map`, `group_by`, `find`,
`find_index`, `some`, `every`) are NOT registered through this surface. They live
inside the evaluator module and call `eval_with_frame` directly with the caller's
`EvalFrame`, so the step budget stays accumulated across every iteration.

Custom callbacks are trusted in-process code: work charging is cooperative and cannot
preempt a callback that ignores the view or blocks. Output bounds are mandatory because
callbacks cannot construct `BuiltinOutput` without the supplied builder. This is not an
isolation boundary.

## Maturity

See `docs/MATURITY.md` row for `nebula-expression`.

- The retained-program boundary intentionally changes internal APIs: `CachedExpression`
  fields are private; `BuiltinRegistry::call` receives a `BuiltinView` instead of an
  evaluator; template constructors accept `AsRef<str>`. Callers caching source-only
  syntax should cache `CompiledProgram` and call `evaluate_compiled` instead.
- `datetime` functions are feature-gated (`feature = "datetime"`); include if date
  arithmetic is needed.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5 (expression context used by resolution step).
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
    ├── program.rs        # Immutable compiled expressions/templates
    ├── limits.rs         # Shared compilation and allocation bounds
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
