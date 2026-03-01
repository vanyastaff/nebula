# API

## Public Surface

- stable APIs:
  - `ExpressionEngine` (`new`, `with_cache_size`, `with_cache_sizes`, `restrict_to_functions`, `evaluate`, `parse_template`, `render_template`)
  - `EvaluationContext` + builder
  - `Template` / `MaybeTemplate`
  - `MaybeExpression` / `CachedExpression`
  - `ExpressionError`, `ExpressionResult`
- experimental APIs:
  - advanced hidden internals (`core`, `lexer`, `parser`, `eval`) should not be treated as stable contracts.
- hidden/internal APIs:
  - `#[doc(hidden)]` re-exports for AST/token internals.

## Usage Patterns

- evaluate single expressions in runtime context.
- parse once and re-render templates many times.
- enable cache for high-frequency expression/template workloads.

## Minimal Example

```rust
use nebula_expression::{EvaluationContext, ExpressionEngine};
use serde_json::Value;

let engine = ExpressionEngine::new();
let mut ctx = EvaluationContext::new();
ctx.set_execution_var("id", Value::String("exec-123".into()));

let out = engine.evaluate("$execution.id", &ctx)?;
assert_eq!(out.as_str(), Some("exec-123"));
# Ok::<(), nebula_expression::ExpressionError>(())
```

## Advanced Example

```rust
use nebula_expression::{EvaluationContext, ExpressionEngine};
use serde_json::json;

let engine = ExpressionEngine::with_cache_sizes(1024, 512);
let mut ctx = EvaluationContext::new();
ctx.set_input(json!("alice"));
ctx.set_execution_var("order_id", json!(42));

let tpl = engine.parse_template("Hello {{ $input | uppercase() }} #{{ $execution.order_id }}")?;
let rendered = engine.render_template(&tpl, &ctx)?;
assert_eq!(rendered, "Hello ALICE #42");
# Ok::<(), nebula_expression::ExpressionError>(())
```

## Policy Example (Function Allowlist)

```rust
use nebula_expression::{EvaluationContext, ExpressionEngine};

let engine = ExpressionEngine::new().restrict_to_functions(["uppercase", "length"]);
let ctx = EvaluationContext::new();

let out = engine.evaluate("uppercase('alice')", &ctx)?;
assert_eq!(out.as_str(), Some("ALICE"));
assert!(engine.evaluate("lowercase('ALICE')", &ctx).is_err());
# Ok::<(), nebula_expression::ExpressionError>(())
```

## Error Semantics

- retryable errors:
  - generally non-retryable for deterministic parse/type/eval failures.
  - `Internal`/transient integration failures may be retryable depending on caller policy.
- fatal errors:
  - syntax/parse/type/function-not-found/division-by-zero and explicit validation failures.
- validation errors:
  - malformed expressions/templates and unsafe regex pattern checks.

## Compatibility Rules

- what changes require major version bump:
  - grammar/operator precedence changes
  - built-in function semantic changes
  - context variable resolution contract changes
- deprecation policy:
  - keep compatibility shims and migration notes for at least one minor release where possible
