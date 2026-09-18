---
name: nebula-expression
role: Expression Evaluator (dynamic field resolution for workflow parameters)
status: stable
last-reviewed: 2026-09-18
canon-invariants: []
related: [nebula-schema, nebula-validator, nebula-core]
---

# nebula-expression

## Purpose

Workflow fields often need dynamic values — the output of a previous node, the current
execution id, or a date computed at runtime. Hardcoding those values in the workflow
definition is not feasible, and ad-hoc string interpolation in each integration author's
action is fragile. `nebula-expression` provides a single shared expression evaluator:
a small expression language (n8n-compatible syntax) that resolves `{{ expression }}`
interpolation, `{% if %}` / `{% for %}` control flow, and `{# #}` comments against
execution-time context, with a parse-once LRU cache for hot paths, typed runtime
values, and wrapper types that let callers declare whether a field may carry an
expression or is always a literal.

## Install

```toml
[dependencies]
nebula-expression = "0.12"
```

Or from this workspace:

```sh
cargo add -p your-crate --path crates/expression
```

## Features

`default = ["cache", "regex", "datetime", "uuid"]`.

| Feature    | Adds |
|------------|------|
| `cache`    | LRU parse cache for expressions and templates (via `moka`) |
| `regex`    | `=~` and `match()` regex support; pulls in `moka` for the pattern cache |
| `datetime` | Date/time builtins with optional IANA timezone arguments (`chrono-tz`) |
| `uuid`     | The `uuid()` builtin |
| `full`     | All of the above |

Disabling `cache` turns off only the AST/template cache; the regex pattern cache
is gated by `regex`.

## Role

**Expression Evaluator.** The resolution backend that `nebula-schema`'s proof-token
pipeline calls at the `ValidValues::resolve` step. Callers supply an `EvaluationContext`
(built from execution state at runtime); the engine evaluates the expression AST against
it. Evaluation works on `RuntimeValue` — JSON shapes plus typed date-times and
`Undefined` — and renders to `serde_json::Value` at the crate boundary. Use
`evaluate_runtime` when the caller wants the typed value instead.

## Public API

- `CompiledProgram` — immutable retained syntax: `compile(source)` (auto),
  `compile_expression(source)` (raw), `compile_template(source)` (text),
  `compile_with_syntax(source, syntax)`, `source()`, and `syntax()`.
- `ProgramSyntax` - closed authored grammar: `Auto`, `Expression` (raw), or
  `Template` (always string). Retained independently of the resulting AST/body.
- `has_expression_marker(source)` — lexical shorthand classification, including
  malformed unescaped openers; neither a `$` nor a closing delimiter is required.
- `ExpressionEngine` — main engine: `evaluate(source, ctx)` and
  `evaluate_runtime(source, ctx)`, `evaluate_compiled(&program, ctx)` and
  `evaluate_compiled_runtime`, `parse_template(source)`,
  `render_template(&template, ctx)`, `register_function(name, f)` for custom
  builtins. Cache constructors require `cache`.
- `EvaluationContext` — runtime variable bindings: `$node`, `$execution`, `$workflow`,
  `$input`, `$json`, `$now`, `$today`; `resolve_variable` returns shared
  `Arc<RuntimeValue>` snapshots, and `EvaluationContextBuilder` provides fluent
  construction.
- `EvaluationPolicy` — function restrictions, coercion rules, work and JSON input
  limits, and `MissingLookup` (missing lookup errors by default, or yields
  `Undefined`).
- `BuiltinFunction`, `Argument`, `BuiltinView` — the custom-builtin contract; see
  [Extending the evaluator](#extending-the-evaluator).
- `BuiltinOutput`, `BuiltinOutputBuilder`, `BuiltinOutputBound`, `BuiltinOutputLimits` — mandatory bounded
  construction for outputs returned by public custom builtins.
- `RuntimeValue` — evaluator value model: JSON shapes plus typed date-times and
  `Undefined`.
- `Template` — pre-parsed `{{ ... }}` / `{% ... %}` template; call `.render(engine, ctx)` to evaluate.
- `MaybeExpression<T>` — typed wrapper: either a literal `T` or an expression string that
  resolves to `T`. Used in `serde` structs for action/credential config parameters.
- `MaybeTemplate` — like `MaybeExpression` but for text templates (`{{ }}` delimiters).
- `MissingLookup` — whether a missing lookup errors or yields `Undefined`.
- `CachedExpression` — opaque lazy program storage inside `MaybeExpression`; clones
  share the compiled program independently of the optional engine cache.
- `ExpressionError`, `ExpressionResult` — typed error and result alias. Parse failures
  carry a structured `Position`; render source context with
  `error_formatter::ErrorFormatter`, not by parsing the message. Runtime lookup keys
  never appear in diagnostics (`KeyNotFound` carries no payload).
- `Position`, `TemplatePart` — template source positions and parsed parts, for
  diagnostics and editor tooling.
- `CacheOverview` — cache hit/miss statistics snapshot.

See the [crate-level rustdoc](https://docs.rs/nebula-expression) for the quick-start and custom-builtin examples.

## Contract

- **Expression variables:** `$node`, `$execution`, `$workflow`, `$input`, plus `$json`
  (the n8n spelling of the current item, which is exactly `$input` because this crate
  resolves one item at a time), `$now`, and `$today`. Seam:
  `crates/expression/src/context.rs`. `$node`/`$execution` members resolve directly
  without building the aggregate view object. The n8n item model (`$item`, `$items()`,
  `$position`, `$itemIndex`) is not implemented: it needs multiple outputs per node,
  which is an engine/workflow contract.
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
- **Missing versus null:** missing variables/properties are lookup errors under the
  default policy; explicit JSON null remains `Value::Null`. No missing-value sentinel is
  collapsed into null. `EvaluationPolicy::with_missing_lookup(MissingLookup::Undefined)`
  opts into n8n-style authoring, where a miss yields `Undefined` that `??` and `?.` can
  consume. `?.` guards a nullish *receiver*, not a missing key: `a.missing?.b` still
  follows the missing policy for `missing`.
- **Methods and namespaces:** `items.filter(x => x > 1)` is the same call as
  `filter(items, x => x > 1)` — the receiver becomes the first argument. JavaScript
  names (`toUpperCase`, `includes`, `startsWith`) and Luxon date methods (`plus`, `diff`,
  `toFormat`) map onto the registered builtins; there is no second library. `reduce`
  honors the JavaScript `(fn, initial)` order. `Math`, `JSON`, `Object`, `Number`, and
  `Array` are namespace libraries that dispatch to receiverless builtins.
- **Coalescing:** `??` returns the left side unless it is `Null` or `Undefined`, so
  `false`, `0`, and `""` survive. It binds looser than `||`.
- **DoS guard:** source is capped at 1 MiB, with at most 65,536 tokens and 16,384 AST
  nodes per embedded expression, depth 256, and 1,000 template expressions. Evaluation
  has a default 100,000-unit work ceiling shared by all template parts, pipelines,
  higher-order bodies, materialized values, string bytes, and builtin work. Context
  restrictions intersect engine restrictions and cannot raise engine limits, including
  the default 1 MiB JSON input limit. String expansion is checked before allocation in
  `split`, `join`, `to_json`, `replace`, `repeat`, padding, and other expanding
  string operations; template output is capped at 1 MiB. Every builtin result also
  has finite total-byte, single-string, collection-item, value-node, and depth bounds.
- **Shared context reads:** stored variables resolve through O(1) `Arc<RuntimeValue>` clones. The
  evaluator retains borrows through property/index chains and passes borrowed values to
  builtins, materializing ownership only when an expression produces a new value or at
  the existing top-level owned-`Value` boundary.
- **Numbers:** signed/unsigned integer and float ordering uses `num-cmp` exact mixed
  comparison, including values above 2^53; numeric scalar equality uses the same order.
  Integer `+`, `-`, `*`, `%` and integral math preserve representable JSON integers or
  fail on overflow. Fractional/out-of-range integer conversions and non-finite results
  fail instead of truncating, saturating, or returning null. Floating arithmetic remains
  IEEE-754, not decimal arithmetic. Arrays/objects retain structural JSON equality.
- **Type coercion:** evaluation produces a `RuntimeValue` and renders to
  `serde_json::Value` at the boundary; `MaybeExpression<T>` calls `resolve_as_*`,
  which coerces the JSON result to `T` and returns a typed error on mismatch.

## Non-goals

- Not a validation rules engine — see `nebula-validator` for `Rule` and `Validate<T>`.
- Not a schema system — see `nebula-schema` for field definitions and the proof-token
  pipeline.
- The template engine supports `{{ }}` interpolation, `{% if %}` / `{% for %}`
  control flow, and `{# #}` comments with structured source positions. It is not a
  general-purpose HTML/JS templating language: there is no template inheritance,
  macro system, or evaluation inside text other than the documented delimiters.
- Not a full JavaScript sandbox: expressions are parsed and evaluated by this crate,
  not by a JS engine.

### BuiltinFunction signature (no frame reset)

`BuiltinFunction` is typed as:

```rust
pub type BuiltinFunction =
    fn(
        &[Argument<'_>],
        BuiltinView<'_>,
        &EvaluationContext,
        BuiltinOutputBuilder,
    ) -> ExpressionResult<BuiltinOutput>;
```

`Argument<'_>` is either an evaluated value or an unevaluated lambda. Lambdas stay
unevaluated until the builtin calls `BuiltinView::invoke_lambda`, which binds the
lambda's parameters positionally and evaluates its body against the **caller's frame**.
A registered builtin therefore cannot reset the step budget or recursion depth, and
every lambda invocation is charged against the calling program — the historical
step-budget bypass (issue #252) stays type-enforced. The pitfall is documented in
[`docs/pitfalls.md`](https://github.com/vanyastaff/nebula/blob/main/docs/pitfalls.md) for historical context.

`BuiltinView<'_>` also exposes the call's effective policy (the engine and
context policies already intersected) plus `charge_work` and
`check_output_bytes` against the calling program's shared budget.
The mandatory `BuiltinOutputBuilder` is the only public way to construct the opaque
result, and validates total bytes, string bytes, collection size, value nodes, and
depth. Registered functions should charge work before loops and use builder methods
that preflight allocations.

Higher-order combinators (`filter`, `map`, `reduce`, `flat_map`, `group_by`, `find`,
`find_index`, `some`, `every`) are ordinary registered builtins built on this surface.
`reduce` accepts both shapes: a single-parameter lambda with `$acc` bound in context,
or `(acc, x) => …` with positional binding.

Custom callbacks are trusted in-process code: work charging is cooperative and cannot
preempt a callback that ignores the view or blocks. Output bounds are mandatory because
callbacks cannot construct `BuiltinOutput` without the supplied builder. This is not an
isolation boundary.

## Maturity

See the [crate maturity dashboard](https://github.com/vanyastaff/nebula/blob/main/docs/MATURITY.md).

- The retained-program boundary intentionally changes internal APIs: `CachedExpression`
  fields are private; `BuiltinRegistry::call` receives a `BuiltinView` instead of an
  evaluator; template constructors accept `AsRef<str>`. Callers caching source-only
  syntax should cache `CompiledProgram` and call `evaluate_compiled` instead.
- `datetime` functions are feature-gated (`feature = "datetime"`); include if date
  arithmetic is needed.

## Related

- Canon: [PRODUCT_CANON.md §3.5](https://github.com/vanyastaff/nebula/blob/main/docs/PRODUCT_CANON.md) (expression context used by the resolution step).
- Siblings: `nebula-schema` (calls expression context via `ValidValues::resolve`),
  `nebula-validator` (rule engine), `nebula-core` (base types).

## Extending the evaluator

Register a custom builtin with `ExpressionEngine::register_function`. The callback
receives evaluated values or unevaluated lambdas (`Argument`), a `BuiltinView` for
work charging and lambda invocation, and a mandatory `BuiltinOutputBuilder`:

```rust
use nebula_expression::{
    Argument, BuiltinOutput, BuiltinOutputBuilder, BuiltinView, EvaluationContext,
    ExpressionEngine, ExpressionResult,
};

fn triple(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _context: &EvaluationContext,
    output: BuiltinOutputBuilder,
) -> ExpressionResult<BuiltinOutput> {
    let value = args[0].as_value().and_then(|value| value.as_i64()).unwrap_or_default();
    output.signed_integer(value * 3)
}

fn main() -> Result<(), nebula_expression::ExpressionError> {
    let mut engine = ExpressionEngine::new();
    engine.register_function("triple", triple);
    assert_eq!(engine.evaluate("triple(7)", &EvaluationContext::new())?.as_i64(), Some(21));
    Ok(())
}
```

Lambdas are invoked through `BuiltinView::invoke_lambda`, which evaluates the body
against the caller's frame; a custom builtin cannot reset the step budget or
recursion depth. Custom callbacks are trusted, cooperative in-process code — the
output bounds are mandatory, but work charging is not an isolation boundary.

## Contributing

See [CONTRIBUTING.md](https://github.com/vanyastaff/nebula/blob/main/CONTRIBUTING.md)
for the toolchain, the layered dependency rules, and the PR process. In short:

- `task dev:check` — the pre-PR gate (fmt + clippy + nextest + doctests + deny).
- `cargo nextest run -p nebula-expression` — this crate's tests.
- `cargo test -p nebula-expression --doc` — doctests.
- Design notes: [`docs/DESIGN.md`](docs/DESIGN.md); local agent orientation:
  [`AGENTS.md`](AGENTS.md).
- Fuzzing: [`fuzz/README.md`](fuzz/README.md).
- Benchmarks: [`benches/README.md`](benches/README.md).

Changes to the evaluation path, builtins, retained compilation, or numeric behavior
have a documented evidence bar — see the *Change checks* table in `AGENTS.md`.

## License

Licensed under `MIT OR Apache-2.0` (see the workspace
[`LICENSE`](https://github.com/vanyastaff/nebula/blob/main/LICENSE)). Unless you
explicitly state otherwise, any contribution you intentionally submit for inclusion
in this crate is dual-licensed as above, without additional terms.

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
    ├── eval/             # AST evaluator (Evaluator, EvalFrame, Argument, BuiltinView)
    ├── value.rs          # RuntimeValue: JSON shapes + typed date-times + Undefined
    ├── builtins/         # BuiltinFunction registry (higher_order.rs, output.rs, …)
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
