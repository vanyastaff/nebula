# API

## Public Surface

- **Stable APIs:**
  - `ExpressionEngine` — `new`, `with_cache_size`, `with_cache_sizes`, `with_policy`, `evaluate`, `parse_template`, `render_template`, `cache_overview`
  - `EvaluationContext` + `EvaluationContextBuilder`
  - `EvaluationPolicy` — function allowlist/denylist + strict mode flags
  - `CacheOverview`
  - `Template` / `MaybeTemplate`
  - `MaybeExpression<T>` / `CachedExpression`
  - `ExpressionError` (#[non_exhaustive]), `ExpressionErrorExt`, `ExpressionResult<T>`
  - `Value` (re-export of `serde_json::Value`)
  - `prelude::*` — convenient single-import of all stable types
- **Internal/hidden:** `ast`, `eval`, `lexer`, `parser`, `interner`, `span`, `token` — `#[doc(hidden)]`, may change between versions.

## `ExpressionError` (#[non_exhaustive])

All variants are `#[non_exhaustive]` — match arms must include `_` or use `..`.

| Variant | Fields | `code()` | `is_retryable()` |
|---|---|---|---|
| `SyntaxError` | `message` | `EXPR:SYNTAX` | false |
| `ParseError` | `message` | `EXPR:PARSE` | false |
| `EvalError` | `message` | `EXPR:EVAL` | false |
| `TypeError` | `expected`, `actual` | `EXPR:TYPE` | false |
| `VariableNotFound` | `name` | `EXPR:VAR_NOT_FOUND` | false |
| `FunctionNotFound` | `name` | `EXPR:FUNC_NOT_FOUND` | false |
| `InvalidArgument` | `function`, `message` | `EXPR:INVALID_ARG` | false |
| `DivisionByZero` | — | `EXPR:DIV_ZERO` | false |
| `RegexError` | `message` | `EXPR:REGEX` | false |
| `IndexOutOfBounds` | `index`, `length` | `EXPR:INDEX_OOB` | false |
| `Validation` | `message` | `EXPR:VALIDATION` | false |
| `NotFound` | `resource_type`, `resource_id` | `EXPR:NOT_FOUND` | false |
| `Internal` | `message` | `EXPR:INTERNAL` | **true** |
| `Json` | `serde_json::Error` | `EXPR:JSON` | **true** |
| `InvalidDate` | `chrono::format::ParseError` | `EXPR:INVALID_DATE` | false |

Convenience constructors: `syntax_error(msg)`, `parse_error(msg)`, `eval_error(msg)`, `type_error(expected, actual)`, `variable_not_found(name)`, `function_not_found(name)`, `invalid_argument(fn, msg)`, `division_by_zero()`, `regex_error(msg)`, `index_out_of_bounds(i, len)`, `validation(msg)`, `internal(msg)`.

`ExpressionErrorExt` trait — adds `expression_*` static constructors as trait methods (must be in scope to call via trait syntax).

## `EvaluationContext` and Variable Namespaces

Context maps 4 runtime namespaces to expression syntax:

| Expression syntax | Context method | Notes |
|---|---|---|
| `$execution.id` | `set_execution_var("id", v)` | arbitrary key-value pairs |
| `$node["name"]` | `set_node_data("name", v)` | per-node output data |
| `$workflow.id` | `set_workflow(v)` | single JSON object |
| `$input` | `set_input(v)` | single JSON value |

Getters: `get_execution_var(name)`, `get_node_data(name)`, `get_workflow()`, `get_input()` — all return `Arc<Value>`.

Per-context policy override: `set_policy(policy)` / `policy() -> Option<&EvaluationPolicy>`.

Builder pattern:

```rust
let ctx = EvaluationContext::builder()
    .execution_var("id", json!("exec-123"))
    .node("http_node", json!({"response": 200}))
    .workflow(json!({"id": "wf-1"}))
    .input(json!("alice"))
    .policy(EvaluationPolicy::allow_only(["uppercase"]))
    .build();
```

## `EvaluationPolicy`

Constrains which builtins are callable and enables strict semantic flags:

```rust
let policy = EvaluationPolicy::new()
    .with_allowed_functions(["uppercase", "length"])  // allowlist (Option<HashSet>)
    .with_denied_functions(["uuid"])                   // denylist (HashSet)
    .with_strict_mode(true)                            // future coercion-hardening flag
    .with_strict_conversion_functions(true)            // to_number/to_boolean require native types
    .with_strict_numeric_comparisons(true)             // <, >, <=, >= only for numbers
    .with_max_json_parse_length(4096);                 // parse_json input size limit
```

Shortcut: `EvaluationPolicy::allow_only(["fn1", "fn2"])`.

Policy can be set at engine level (`engine.with_policy(p)`) or per-context (`ctx.set_policy(p)`). Context policy overrides engine policy.

## `MaybeExpression<T>`

A parameter type that accepts either a concrete value or an expression string resolved at runtime.

```rust
pub enum MaybeExpression<T> {
    Value(T),
    Expression(CachedExpression),   // source string + OnceCell<Expr> (lazy AST)
}
```

Constructors: `MaybeExpression::value(v)`, `MaybeExpression::expression("{{ expr }}")`.

Check: `is_value()`, `is_expression()`, `as_value()`, `as_expression()`.

Convert: `into_value()`, `into_expression()`.

**Serde behavior:** transparent — `Value(T)` serializes as `T`; `Expression` serializes as its string. On deserialization, strings containing `{{ }}` are auto-detected as expressions; everything else deserializes as `T`.

**Typed resolve methods:**

```rust
maybe.resolve_as_value(&engine, &ctx)    -> Result<Value, ExpressionError>
maybe.resolve_as_string(&engine, &ctx)   -> Result<String, ExpressionError>
maybe.resolve_as_integer(&engine, &ctx)  -> Result<i64, ExpressionError>
maybe.resolve_as_float(&engine, &ctx)    -> Result<f64, ExpressionError>
maybe.resolve_as_bool(&engine, &ctx)     -> Result<bool, ExpressionError>
// Generic: T: TryFrom<Value>, <T::Error>: Into<ExpressionError>
maybe.resolve(&engine, &ctx)             -> Result<T, ExpressionError>
```

`From<T>` and `Default` (where `T: Default`) implemented.

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

## Policy Example (Strict Mode)

```rust
use nebula_expression::{EvaluationContext, EvaluationPolicy, ExpressionEngine};

let engine = ExpressionEngine::new().with_policy(EvaluationPolicy::new().with_strict_mode(true));
let ctx = EvaluationContext::new();

assert!(engine.evaluate("if 1 then 'yes' else 'no'", &ctx).is_err());
```

## Policy Example (Strict Conversion Builtins)

```rust
use nebula_expression::{EvaluationContext, EvaluationPolicy, ExpressionEngine};

let policy = EvaluationPolicy::new().with_strict_conversion_functions(true);
let engine = ExpressionEngine::new().with_policy(policy);
let ctx = EvaluationContext::new();

assert!(engine.evaluate("to_number('42')", &ctx).is_err());
assert!(engine.evaluate("to_boolean(1)", &ctx).is_err());
assert!(engine.evaluate("to_string([1,2,3])", &ctx).is_err());
assert!(engine.evaluate("parse_json('42')", &ctx).is_err());
```

```rust
use nebula_expression::{EvaluationContext, EvaluationPolicy, ExpressionEngine, Value};

let policy = EvaluationPolicy::new().with_max_json_parse_length(5);
let engine = ExpressionEngine::new().with_policy(policy);
let mut ctx = EvaluationContext::new();
ctx.set_input(Value::String("{\"a\":1}".to_string()));

assert!(engine.evaluate("parse_json($input)", &ctx).is_err());
```

```rust
use nebula_expression::{EvaluationContext, EvaluationPolicy, ExpressionEngine};

let policy = EvaluationPolicy::new().with_strict_numeric_comparisons(true);
let engine = ExpressionEngine::new().with_policy(policy);
let ctx = EvaluationContext::new();

assert!(engine.evaluate("'b' > 'a'", &ctx).is_err());
assert_eq!(engine.evaluate("3 > 2", &ctx)?.as_bool(), Some(true));
# Ok::<(), nebula_expression::ExpressionError>(())
```

## Cache Observability Example

```rust
use nebula_expression::{EvaluationContext, ExpressionEngine};

let engine = ExpressionEngine::with_cache_size(128);
let ctx = EvaluationContext::new();
let _ = engine.evaluate("2 + 3", &ctx)?;

let cache = engine.cache_overview();
assert!(cache.expr_cache_enabled);
assert!(cache.expr_entries >= 1);
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
