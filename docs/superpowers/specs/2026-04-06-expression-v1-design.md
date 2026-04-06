# nebula-expression v1 — Language Spec

## Goal

Formalize the expression language: grammar, operators, built-in functions, variables, security, and optimization roadmap. This is the v1 BLOCKER identified in conference Round 4 (Google feedback) and cross-spec audit (Y6).

## Current State

Expression engine is **already very mature**: lexer, parser, evaluator, 60+ built-in functions, lambda support, pipeline operator, template interpolation, regex match, step limits, policy system. This spec **documents what exists** and adds the missing pieces.

---

## 1. Formal Grammar

### 1.1 Template Syntax

Templates mix text with expressions:
```
Hello {{ $input.name }}, your order #{{ $node.order.id }} is ready.
```

Delimiters: `{{ expression }}`. Whitespace control: `{{- expression -}}` strips surrounding whitespace.

### 1.2 Expression Grammar (PEG-style)

```
expression     = conditional | pipeline
pipeline       = logical ("|" function_call)*
conditional    = "if" expression "then" expression "else" expression
               | logical
logical        = comparison (("&&" | "||") comparison)*
comparison     = addition (("==" | "!=" | "<" | ">" | "<=" | ">=" | "=~") addition)*
addition       = multiplication (("+" | "-") multiplication)*
multiplication = power (("*" | "/" | "%") power)*
power          = unary ("**" unary)*
unary          = ("!" | "-") unary | postfix
postfix        = primary (property_access | index_access | function_call)*
property_access = "." identifier
index_access   = "[" expression "]"
function_call  = "(" (expression ("," expression)*)? ")"
primary        = literal | variable | identifier | "(" expression ")" | array | object | lambda
literal        = string | number | boolean | null
variable       = "$" identifier
lambda         = identifier "=>" expression
               | "(" identifier ("," identifier)* ")" "=>" expression
array          = "[" (expression ("," expression)*)? "]"
object         = "{" (key ":" expression ("," key ":" expression)*)? "}"
key            = identifier | string
identifier     = [a-zA-Z_][a-zA-Z0-9_]*
string         = '"' escaped_char* '"' | "'" escaped_char* "'"
number         = [0-9]+ ("." [0-9]+)?
boolean        = "true" | "false"
null           = "null"
```

### 1.3 Operator Precedence (highest to lowest)

| Level | Operators | Associativity |
|-------|-----------|---------------|
| 7 | `**` | Right |
| 6 | `*` `/` `%` | Left |
| 5 | `+` `-` | Left |
| 4 | `<` `>` `<=` `>=` `=~` | Left |
| 3 | `==` `!=` | Left |
| 2 | `&&` | Left |
| 1 | `\|\|` | Left |

Unary `-` and `!` bind tighter than any binary operator. Pipeline `|` binds loosest.

---

## 2. Built-in Functions (60+)

### String (10)
`length`, `uppercase`, `lowercase`, `trim`, `split`, `replace`, `substring`, `contains`, `starts_with`, `ends_with`

### Array (12)
`length`, `first`, `last`, `sort`, `reverse`, `join`, `slice`, `concat`, `flatten`, `filter`, `map`, `reduce`

### Object (3)
`keys`, `values`, `has`

### Math (8)
`abs`, `round`, `floor`, `ceil`, `min`, `max`, `sqrt`, `pow`

### DateTime (12)
`now`, `now_iso`, `parse_date`, `format_date`, `date_add`, `date_subtract`, `date_diff`, `date_year`, `date_month`, `date_day`, `date_hour`, `date_minute`, `date_second`, `date_day_of_week`

### Conversion (5)
`to_string`, `to_number`, `to_boolean`, `to_json`, `parse_json`

### Type Checking (5)
`is_null`, `is_array`, `is_object`, `is_string`, `is_number`

### Utility (1)
`uuid` (feature-gated)

### Missing — add for v1 (from audit)
- **Array:** `some`, `every`, `find`, `find_index`, `unique`, `group_by`, `flat_map`
- **Object:** `merge`, `pick`, `omit`, `entries`, `from_entries`
- **String:** `pad_start`, `pad_end`, `repeat`, `match` (regex capture groups)
- **Utility:** `coalesce` (null coalescing as function: `coalesce(a, b, default)`)
- **Type:** `type_of` (returns type name as string)

---

## 3. Variables

| Variable | Type | Description |
|----------|------|-------------|
| `$node` | Object | All predecessor node outputs. Access: `$node.nodeName` |
| `$input` | Value | Current node's flow input |
| `$execution` | Object | Execution metadata (id, startTime, etc.) |
| `$workflow` | Object | Workflow metadata (id, name, etc.) |
| `$now` | String | Current UTC time in RFC 3339 |
| `$today` | String | Current UTC date as YYYY-MM-DD |

Variables resolve via `EvaluationContext::resolve_variable()`. Unknown variables return `null` (not error).

### Missing — add for v1.1
- `$item` — current item in per-item execution (when ForEach lands)
- `$itemIndex` — index of current item
- `$env` — sandboxed environment variable access (allowlisted keys only)

---

## 4. Security

### 4.1 Existing protections
- **Recursion depth limit:** 256 levels (eval_with_depth)
- **Evaluation step limit:** configurable via `EvaluationPolicy::max_eval_steps` (implemented today)
- **Regex ReDoS protection:** pattern length cap + nested quantifier detection
- **Max template expressions:** 1000 per template
- **Max JSON parse length:** configurable
- **Function allowlist/denylist:** via EvaluationPolicy

### 4.2 Missing — add for v1 (from red team RT-1, RT-2)

**RT-1: Memory budget per evaluation**
```rust
impl EvaluationPolicy {
    /// Maximum bytes allocatable during expression evaluation.
    /// Prevents DoS via expressions that produce large intermediate results.
    pub max_eval_memory_bytes: Option<usize>,
}
```
Track via allocator hooks or periodic RSS check. Return `ExpressionError::MemoryBudgetExceeded`.

**RT-2: Value redaction in error messages**
Expression errors NEVER include node output VALUES — only field NAMES and types.
```
// BAD: "Cannot access field 'password' on {token: 'sk-live-abc123', ...}"
// GOOD: "Cannot access field 'password' on Object{token: String, ...}"
```
Implement a `redact_value(v: &Value) -> String` that shows shape, not content.

### 4.3 Sandboxing guarantees
Expressions have NO access to:
- Filesystem
- Network
- Environment variables (unless `$env` added with allowlist)
- System clock (except `$now`/`$today` which are snapshot values)
- Process state
- Other executions' data

---

## 5. Performance Optimizations

### 5.1 Existing
- **Expression cache:** `ConcurrentComputeCache` for parsed ASTs (via get+insert, not get_or_compute)
- **Template cache:** same pattern
- **Zero-copy lexer:** `Cow<'a, str>` for string tokens (implemented today)
- **Step counter:** `AtomicUsize` for DoS prevention (implemented today)

### 5.2 Planned — v1.1

**Inline Caching (breakthrough #1, Google V8 pattern)**
Cache resolved paths in expression evaluation. After first `$node.output.data.name` access, subsequent accesses are a pointer compare + dereference.
```rust
pub struct InlineCache {
    shape_tag: usize,     // Arc pointer identity
    cached: Option<*const Value>,
}
```
Impact: 10-50x on repeated expressions in loop/template nodes.

**Vectorized Batch Evaluation (breakthrough #6, Databricks Photon pattern)**
```rust
impl Evaluator {
    pub fn eval_batch(
        &self, param: &str, body: &Expr, values: &[Value], base_context: &EvaluationContext,
    ) -> ExpressionResult<Vec<Value>>;
}
```
One context clone for N items instead of N clones. Impact: 3-5x on array workflows.

---

## 6. EvaluationContext — Serialization Strategy Integration

Per serialization-strategy spec Section 2.6, context nodes should use `Arc<RawValue>` for lazy parsing:

```rust
pub struct EvaluationContext {
    /// Node outputs as raw JSON — parsed only when $node.field accessed
    nodes: HashMap<Arc<str>, Arc<RawValue>>,
    execution_vars: HashMap<Arc<str>, Arc<Value>>,
    workflow: Arc<Value>,
    input: Arc<Value>,
    policy: Option<Arc<EvaluationPolicy>>,
}
```

`$node` access: parse from `RawValue` on first use, cache in `OnceLock`. Most expressions only access 1-2 node outputs, so most remain unparsed.

---

## 7. Custom Function Registration

```rust
impl ExpressionEngine {
    /// Register a custom built-in function.
    pub fn register_function(
        &mut self,
        name: impl AsRef<str>,
        func: BuiltinFunction,
    );
}

// Plugin can register domain-specific functions:
engine.register_function("format_currency", |args, eval, ctx| {
    let amount = args[0].as_f64().unwrap_or(0.0);
    let currency = args[1].as_str().unwrap_or("USD");
    Ok(Value::String(format!("{:.2} {}", amount, currency)))
});
```

Functions registered BEFORE workflow execution begins. Immutable during execution.

---

## 8. Error Format — Structured with Redaction

```rust
pub struct ExpressionError {
    pub kind: ExpressionErrorKind,
    /// Position in source for error highlighting
    pub span: Option<Span>,
    /// Safe message (no Values, only field names and types)
    pub message: String,
    /// Error code for localization
    pub code: &'static str,
}

pub enum ExpressionErrorKind {
    SyntaxError,
    ParseError,
    EvalError,
    TypeError,
    RecursionLimit,
    StepLimit,
    MemoryLimit,
    PolicyViolation,
    FunctionNotFound,
    Internal,
}
```

Error formatter uses `ErrorFormatter` with line/column source context and `^^^` highlighting (already implemented).

---

## 9. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Grammar | Undocumented | Formal PEG grammar |
| Missing array functions | No some/every/find/group_by | Add 7 array functions |
| Missing object functions | No merge/pick/omit | Add 5 object functions |
| Memory budget | None | max_eval_memory_bytes policy |
| Value redaction | Errors may leak values | Shape-only in error messages |
| Custom functions | register_function exists | Document as public API |
| Inline caching | None | v1.1 breakthrough #1 |
| Batch evaluation | None | v1.1 breakthrough #6 |
| Context | Arc<Value> per node | Arc<RawValue> lazy parsing |

---

## 10. Not In Scope

- Assignment / variables within expressions (use node chaining instead)
- Loops (use map/filter/reduce)
- Async expressions (all sync, engine resolves before eval)
- Custom operator definitions
- Module / import system
- Comments in expressions
- String interpolation beyond `{{ }}` templates
