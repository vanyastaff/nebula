# Spec 12 — Expression language (`nebula-expression`)

> **Status:** draft
> **Canon target:** §3.11 (new)
> **Depends on:** 13 (workflow versioning — expressions compiled at save time), 16 (storage — no direct deps but context provides execution state)
> **Depended on by:** 13 (workflow uses expressions), 14 (stateful — expressions in step conditions)

## Problem

Workflow engines need a way to express dynamic values — URL constructed from trigger payload, condition checking credential is still valid, template for notification message. Four real options:

1. **Full programming language** (JavaScript, Python) — Turing-complete, maximally expressive, security risk (sandbox escapes), DoS risk (infinite loops), learning curve
2. **Limited DSL** (Handlebars, Jinja2 subset) — safer but often too weak, ad-hoc syntax
3. **SQL-like** — familiar but awkward for tree navigation
4. **Non-Turing expression language** (CEL, Google's Common Expression Language) — safe by construction, fast, purpose-built for this

n8n chose JavaScript in `vm2`, had several sandbox escape CVEs. Airflow uses Jinja2 with `autoescape=False` default, had SSTI bugs. Zapier uses custom DSL, users complain it's too limited for real needs.

## Decision

**Non-Turing-complete language, CEL-inspired, via `cel-interpreter` or custom implementation.** Two surfaces: expressions (conditions, computations) and templates (string interpolation). Expressions see only an explicit `EvalContext` — no filesystem, network, credentials, or process environment. Compiled once at workflow save time, cached bytecode executed at runtime. Authors extend via pure deterministic functions; non-deterministic results persisted for replay safety.

## Two surfaces

### Expression — for conditions and computations

```
user.age >= 18 && user.country in ['US', 'UK', 'DE']

items.filter(i, i.price > 100).map(i, i.name)

timestamp() - last_run_at > duration('1h')

trigger.payload.amount > 1000 ? 'high-value' : 'normal'
```

Used in:

- Edge conditions (`when` clause in DAG — spec 11 workflow)
- Dynamic parameter values (`http_url: "https://api.example.com/users/${user.id}"`)
- Retry policy conditions
- Stateful action step guards

### Template — for string interpolation

```
"Hello ${user.name}, your order ${order.id} is ready"

"Billing amount: ${price * (1 + tax_rate)} USD"

"https://example.com/users/${user.id}?ref=${trigger.source}"
```

**Template compiles down to an expression** internally: `"Hello " + user.name + ", your order " + order.id + " is ready"`. One parser, one evaluator, two syntax surfaces.

## Language design

### Supported operators

```
# Arithmetic
+ - * / % 

# Comparison
== != < <= > >=

# Logical
&& || !

# String
+ (concatenation)
in (substring or collection membership)

# Collection
[index]           # list access
[key]             # map access
.field            # field access
in                # membership
size()            # cardinality

# Conditional
condition ? a : b  # ternary
```

### Supported types

```rust
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(IndexMap<String, Value>),  // ordered for determinism
    Duration(std::time::Duration),
    Timestamp(DateTime<Utc>),
}
```

Unified type system, maps 1:1 to JSON plus `duration` and `timestamp` for schedule-aware operations.

### Built-in functions

**Pure, deterministic functions:**

```
# Strings
string.lower() -> string
string.upper() -> string
string.trim() -> string
string.split(sep) -> list<string>
string.starts_with(prefix) -> bool
string.ends_with(suffix) -> bool
string.contains(substring) -> bool
string.replace(from, to) -> string
string.matches(regex) -> bool
len(value) -> int     # length of string/list/map

# Collections
list.size() -> int
list.filter(x, predicate) -> list
list.map(x, expression) -> list
list.all(x, predicate) -> bool
list.exists(x, predicate) -> bool
list.reduce(x, acc, initial, expression) -> any

# Math
abs(n) -> number
max(a, b) -> number
min(a, b) -> number
floor(n) -> int
ceil(n) -> int
round(n) -> int

# Time (deterministic)
duration('1h30m') -> duration
duration_from_ms(1000) -> duration
timestamp.add(duration) -> timestamp
timestamp.sub(timestamp) -> duration
timestamp.format('RFC3339') -> string

# Encoding
base64_encode(bytes) -> string
base64_decode(string) -> bytes
hex_encode(bytes) -> string
hex_decode(string) -> bytes
sha256(bytes) -> bytes
md5(bytes) -> bytes       # discouraged, only for legacy compat
url_encode(string) -> string
url_decode(string) -> string
json_parse(string) -> any
json_stringify(any) -> string

# Path navigation
jsonpath(value, '$.path') -> any
has(value, 'path') -> bool

# Regex
regex_match(str, pattern) -> bool
regex_capture(str, pattern) -> map<string, string>
regex_replace(str, pattern, replacement) -> string

# Utility
coalesce(a, b, c, ...) -> any    # first non-null
if_null(value, default) -> any
```

**Non-deterministic functions (marked, results persisted for replay):**

```
now() -> timestamp              # current time
random() -> float               # [0, 1)
random_int(min, max) -> int
uuid() -> string                # generates new UUID
```

When expression calls non-deterministic function during execution, result is captured and stored in execution journal. On replay (retry, multi-worker takeover), cached value used instead of re-evaluating.

### No loops, no recursion, no user-defined functions

**No loops:** there is `filter`/`map`/`reduce` on collections (bounded by collection size), but no `while` or `for`.

**No recursion:** functions cannot call themselves or create cycles in the call graph. Static check at compile time.

**No user-defined functions:** authors cannot define `fn foo() { ... }` in expressions. Only built-ins and extension functions (see below).

**Why:** bounded evaluation time. Any expression completes in polynomial time w.r.t. AST size. No infinite loops, no stack overflow. DoS impossible via expression complexity.

## `EvalContext` — what expressions can see

**Security boundary.** Expression sees **only** this context. Nothing else.

```rust
pub struct EvalContext<'a> {
    /// Current node's input (merged from predecessors + params + resolved expressions).
    /// Accessed as `input.field_name` or `$input.field_name`.
    pub input: &'a Value,
    
    /// Outputs of all completed nodes in this execution, keyed by logical_node_id.
    /// Accessed as `nodes.previous_node_id.output` or `$nodes.xyz.output`.
    pub nodes: &'a HashMap<String, Value>,
    
    /// Trigger payload (webhook body, cron context, event data).
    /// Accessed as `trigger.payload` or `$trigger.payload`.
    pub trigger: &'a Value,
    
    /// Execution-wide variables (set by `SetVariable` action).
    /// Accessed as `vars.foo` or `$vars.foo`. Read-only from expressions.
    pub vars: &'a Value,
    
    /// Workflow metadata (id, slug, version number).
    /// Accessed as `workflow.id`, `workflow.version`.
    pub workflow: &'a WorkflowMeta,
    
    /// Execution metadata (id, started_at, source).
    /// Accessed as `execution.id`, `execution.started_at`.
    pub execution: &'a ExecutionMeta,
    
    /// Whitelisted environment values.
    /// NOT process env. Only values explicitly exported by workspace config.
    /// Accessed as `env.allowed_key`.
    pub env: &'a HashMap<String, String>,
}

pub struct WorkflowMeta {
    pub id: String,
    pub slug: String,
    pub version_number: u32,
    pub display_name: String,
}

pub struct ExecutionMeta {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub source: ExecutionSource,
}
```

### What expressions **NEVER** see

- **Filesystem** — no `read_file`, no `ls`, no paths. Not addressable at all.
- **Network** — no `http_get`, no socket, no DNS. If data needs to be fetched, that's an action's job.
- **Credentials** — no `secret('stripe_key')`. Credential resolution is engine's job, passed through action context, never visible to expressions.
- **Process environment** — no `std::env::var`. Whitelisted `env.*` map is populated from workspace config, not from process env.
- **Mutable state** — `vars.*` is read-only. Setting a variable requires explicit `SetVariable` action, not expression side effect.
- **System info** — no hostname, no OS, no IP addresses.
- **Other executions** — an execution's context contains only its own nodes/trigger/vars. No cross-execution access.

### Canon §12.5 compliance

Canon §12.5 forbids secrets in logs, error strings, metrics labels. Expression evaluator respects this:

- Expressions log only expression text + location on error, never context values
- Error messages reference field paths (e.g., «missing field `user.email`»), not contents
- `Display` / `Debug` on `Value` redacts strings containing credential patterns (optional extra safety)

## Parsing and compilation

### Parse tree

```rust
pub enum Expr {
    Literal(Value),
    Variable(String),                          // `user`, `$input`
    FieldAccess(Box<Expr>, String),            // `user.name`
    IndexAccess(Box<Expr>, Box<Expr>),         // `list[0]`, `map['key']`
    BinaryOp(BinOp, Box<Expr>, Box<Expr>),     // `a + b`
    UnaryOp(UnaryOp, Box<Expr>),               // `!flag`, `-n`
    FunctionCall(String, Vec<Expr>),           // `max(a, b)`
    MethodCall(Box<Expr>, String, Vec<Expr>),  // `list.filter(...)`
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),  // `cond ? a : b`
    In(Box<Expr>, Box<Expr>),                  // `x in list`
    List(Vec<Expr>),                           // `[1, 2, 3]`
    Map(Vec<(String, Expr)>),                  // `{a: 1, b: 2}`
    Comprehension {                             // `list.filter(x, predicate)`
        kind: ComprehensionKind,
        source: Box<Expr>,
        var: String,
        predicate: Box<Expr>,
    },
}

pub enum ComprehensionKind { Filter, Map, All, Exists }
```

### Compilation steps

```
Source string
  ↓ parser (nom / pest / lalrpop)
  ↓
Parse tree (Expr enum)
  ↓ validator (check variables exist in context, functions exist, types match loosely)
  ↓
Validated expression
  ↓ compiler (convert to bytecode or keep as tree)
  ↓
Compiled expression (bytecode or tree + type hints)
  ↓ stored in workflow_versions (or cached in memory)
```

**Compile at workflow save time, not at execution time.** Expression errors are caught in UI when user saves, not when workflow runs at 3 AM.

### Caching

Compiled expressions stored alongside workflow version (can be a separate column or embedded in workflow JSON):

```sql
ALTER TABLE workflow_versions
    ADD COLUMN compiled_expressions BYTEA;  -- serialized bytecode cache
```

Runtime loads compiled form, evaluates. If cache missing (older workflow version), re-parse on first load, cache in memory.

**Cache invalidation:** on workflow edit, new version, new compilation, new cache. Old version's cache stays valid as long as that version exists.

## Evaluation

### Evaluator interface

```rust
// nebula-expression/src/lib.rs

pub fn compile(source: &str) -> Result<CompiledExpr, CompileError>;
pub fn eval(compiled: &CompiledExpr, ctx: &EvalContext) -> Result<Value, EvalError>;

/// Template compilation — produces expression that concatenates parts.
pub fn compile_template(source: &str) -> Result<CompiledExpr, CompileError>;

/// Render template to string.
pub fn render(compiled: &CompiledExpr, ctx: &EvalContext) -> Result<String, EvalError>;
```

### Error types

```rust
#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("parse error at {location}: {message}")]
    ParseError { location: Span, message: String },
    
    #[error("unknown function `{name}` at {location}")]
    UnknownFunction { name: String, location: Span },
    
    #[error("unknown variable `{name}` at {location}. Available: {available}")]
    UnknownVariable { name: String, available: String, location: Span },
    
    #[error("wrong number of arguments for `{function}` at {location}: expected {expected}, got {got}")]
    ArgumentMismatch { function: String, expected: usize, got: usize, location: Span },
}

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("type mismatch at {location}: expected {expected}, got {got}")]
    TypeMismatch { expected: &'static str, got: &'static str, location: Span },
    
    #[error("missing field `{path}` at {location}")]
    MissingField { path: String, location: Span },
    
    #[error("division by zero at {location}")]
    DivisionByZero { location: Span },
    
    #[error("index out of bounds at {location}: {index} of length {length}")]
    IndexOutOfBounds { index: i64, length: usize, location: Span },
    
    #[error("function `{name}` at {location} failed: {reason}")]
    FunctionError { name: String, reason: String, location: Span },
}

pub struct Span {
    pub line: u32,
    pub column: u32,
    pub length: u32,
}
```

**Span tracking critical for DX.** User sees «error in expression at column 15: missing field `user.email`», can fix immediately. Without span, expression errors are unattributable.

## Extension functions (for integration authors)

Integration authors can register additional pure functions:

```rust
// nebula-expression/src/extension.rs
pub trait ExprFunction: Send + Sync {
    fn name(&self) -> &str;
    fn signature(&self) -> FunctionSignature;
    fn is_deterministic(&self) -> bool { true }
    fn call(&self, args: &[Value], ctx: &EvalContext) -> Result<Value, EvalError>;
}

pub struct FunctionSignature {
    pub params: Vec<ParamSpec>,
    pub return_type: ValueType,
}

pub struct ParamSpec {
    pub name: String,
    pub ty: ValueType,
    pub optional: bool,
}
```

### Registration

```rust
// In nebula-expression::registry
pub struct FunctionRegistry {
    functions: HashMap<String, Box<dyn ExprFunction>>,
}

impl FunctionRegistry {
    pub fn register(&mut self, f: impl ExprFunction + 'static) {
        self.functions.insert(f.name().to_string(), Box::new(f));
    }
}

// Plugins register functions when loaded
pub fn register_stripe_functions(registry: &mut FunctionRegistry) {
    registry.register(StripeAmountToDollars);
    registry.register(StripeVerifyWebhookSignature);  // deterministic — hash computation
}
```

### Constraints on extensions

- **Must be pure** — same inputs → same output (unless marked non-deterministic)
- **Must be fast** — function called inline during eval, no blocking I/O
- **No side effects** — function cannot emit events, write storage, make network calls
- **No async** — expression evaluation is synchronous by design

If a function violates these, flag in code review. Runtime doesn't enforce (would require too much ceremony), trust extensions to follow rules.

## Non-deterministic functions and replay

Problem: `now()` returns different value each call. If workflow retried, conditions evaluated with different `now()` may behave differently → non-deterministic replay, hard-to-reproduce bugs.

**Solution: persist non-deterministic results in `execution_journal`.**

```rust
// First evaluation
let ts = now();  // = 2026-04-15T10:00:00Z
// Journal entry written: {key: "now_at_node_a_call_0", value: 2026-04-15T10:00:00Z}
if ts.hour() >= 9 { ... }

// On retry (attempt 2 of node_a after first failure)
let ts = now();  // would be different, say 2026-04-15T10:05:00Z
// But journal has {key: "now_at_node_a_call_0", value: 2026-04-15T10:00:00Z}
// Runtime intercepts: returns cached value, not fresh call
// Same condition result, deterministic behavior
```

**Implementation:** runtime passes `ReplayCache` to evaluator. Non-deterministic function first checks cache by (expression_location, call_index), returns cached or computes fresh and caches.

**Cost:** journal entries per non-deterministic call. Measurable for complex workflows. Alternative: mark entire workflow as «non-deterministic mode» and just accept that replays differ.

**v1 decision:** default behavior is to persist non-deterministic results. Opt-out via workflow setting `allow_non_deterministic_replay: true` for workflows that genuinely don't care.

## Template syntax

Templates use `${...}` for expression interpolation:

```
"Hello ${user.name}, order ${order.id} total ${order.amount * 1.2} USD"

Compiles to expression:
  "Hello " + user.name + ", order " + order.id + " total " + (order.amount * 1.2) + " USD"
```

**No control flow in templates** — no `{% if %}`, no `{% for %}`. For conditional strings, use expression ternary:

```
"Status: ${condition ? 'active' : 'inactive'}"
```

**Escaping `${}`:** `$${` produces literal `${`. Rare need.

### Auto-coercion to string

When inside template, non-string values are coerced via standard formatting:

- `int` → decimal
- `float` → `%.*g` style
- `bool` → `"true"` / `"false"`
- `null` → `"null"` (or empty? — design decision; recommend `"null"` for debuggability)
- `list`, `map` → JSON representation
- `timestamp` → RFC3339
- `duration` → Go-style `"1h30m"`

**Explicit formatting available** via `.format()` method:

```
"Total: ${amount.format_currency('USD')}"
"Date: ${timestamp.format('2006-01-02')}"
```

## Security: evaluation only on designated fields

**Critical rule:** expressions are evaluated **only** on fields that the workflow author explicitly designated as expressions. Data flowing through the workflow (trigger payload, action outputs) is **never** re-evaluated as expressions, even if it happens to contain `${...}` syntax.

**Why this matters:**

- Webhook body contains `"description": "Buy ${admin.credentials.stripe_key}"` — this string is **data**, not an expression, because the payload field is `description`, not a dynamic parameter
- Airflow's Jinja2 recursive template rendering caused SSTI (Server-Side Template Injection) because user input was evaluated
- We don't do this

**Implementation:** expression compilation happens **at workflow save time**, on fields that are typed as «dynamic» in the workflow schema (e.g., `NodeDefinition::params: HashMap<String, ParamValue>` where `ParamValue` can be either `Literal(Value)` or `Expression(String)`). Only `Expression(s)` values get compiled. Everything else passes through as data.

## Examples

### Simple parameter interpolation

Workflow definition:

```yaml
nodes:
  send_email:
    action: email.send
    params:
      to: "${trigger.payload.customer.email}"
      subject: "Welcome, ${trigger.payload.customer.name}!"
      body: "Your account ID is ${trigger.payload.customer.id}"
```

### Conditional routing

```yaml
edges:
  - from: classify
    to: high_value_path
    condition:
      when: "nodes.classify.output.score > 0.8"
  - from: classify
    to: normal_path
    condition:
      when: "nodes.classify.output.score <= 0.8"
```

### Collection operations

```yaml
nodes:
  filter_premium_users:
    action: data.filter
    params:
      items: "${trigger.payload.users}"
      predicate: "user.plan == 'premium' && user.active"
      # Evaluated as: trigger.payload.users.filter(user, user.plan == 'premium' && user.active)
```

### String manipulation

```yaml
nodes:
  make_slug:
    action: string.transform
    params:
      input: "${trigger.payload.title.lower().replace(' ', '-')}"
```

## Configuration surface

```toml
[expression]
# Maximum expression AST depth (prevents pathological nesting)
max_depth = 32

# Maximum number of iterations in filter/map/reduce (prevents expensive collection operations)
max_iterations = 10_000

# Timeout per expression evaluation (backup safety net, primary defense is non-Turing-complete design)
eval_timeout_ms = 100

# Allow non-deterministic functions to be replayed fresh (skip journal cache)
allow_non_deterministic_replay = false
```

## Testing criteria

**Unit tests:**
- Parser accepts valid expressions
- Parser rejects malformed expressions with clear error location
- Each built-in function produces expected output for typical inputs
- Type coercion rules in template rendering
- Non-deterministic functions cache results across calls with same key

**Integration tests:**
- Full compile → eval roundtrip for representative expressions
- Template with multiple interpolations
- Collection operations on realistic data
- Missing field error reports correct location
- Extension function registration and use
- Replay consistency: non-deterministic function returns same value on retry

**Security tests:**
- Expression cannot access filesystem (no `fs`, `file`, `path` functions available)
- Expression cannot make network calls
- Expression cannot read env vars not in whitelist
- Expression cannot evaluate strings from trigger payload (no recursive eval)
- Malicious input: deeply nested expression rejected (max_depth)
- Malicious input: expensive collection operation capped (max_iterations)

**Performance tests:**
- Simple expression eval: < 10 µs
- Complex expression (nested filters, maps): < 1 ms
- Template with 10 interpolations: < 50 µs
- Compilation time: < 5 ms per expression

**Property tests:**
- `eval(compile(x)) == eval(compile(x))` for any compilable x (determinism for pure exprs)
- Parser reports span for every error
- Compiled expression size scales linearly with source length

## Performance targets

- Parse + compile: **< 5 ms p99** per expression
- Eval (simple): **< 10 µs**
- Eval (complex with collection ops): **< 1 ms p99**
- Template render: **< 50 µs** for template with 10 interpolations
- Memory: compiled expression < 10× source size

## Module boundaries

| Component | Crate |
|---|---|
| Parser, AST types, compiler, evaluator | `nebula-expression` |
| `Value` type, `EvalContext`, `CompiledExpr`, errors | `nebula-expression` |
| Built-in functions | `nebula-expression::builtins` |
| `ExprFunction` trait for extensions | `nebula-expression` |
| `FunctionRegistry` | `nebula-expression` |
| Non-deterministic replay integration | `nebula-runtime` (passes ReplayCache) |
| Template parser | `nebula-expression::template` |
| Span-tracking, error formatting | `nebula-expression::diagnostics` |

## Dependencies

- **`cel-interpreter` crate** — if we choose to use Google's CEL as basis. Mature, fast, Rust implementation exists.
- **OR custom implementation** — if CEL features don't match (extension functions, async, etc.).
- `nom` / `pest` / `lalrpop` for parsing if custom
- `thiserror` for errors
- `indexmap` for ordered maps (determinism)
- `chrono` / `chrono-tz` for time handling

**Decision on CEL vs custom:** start with evaluation, decide based on fit. CEL has prior art for Kubernetes admission control, Google Cloud IAM conditions — proven at scale. Custom gives flexibility. Recommendation: **start with CEL**, fall back to custom only if we hit limits.

## Migration path

**Greenfield if `nebula-expression` crate exists but is empty.** If it already has a partial implementation, audit and adapt.

**Workflow definition schema:** `ParamValue` enum needs `Expression(String)` variant to distinguish from literal values. Serialization:

```json
{
  "params": {
    "static_field": "hello",           // literal
    "dynamic_field": {"$expr": "user.name"}, // expression
    "template_field": {"$tmpl": "Hello ${user.name}"} // template
  }
}
```

Or simpler: prefix-based (`${...}` anywhere → template, `@{...}` → raw expression). Convention up to UI/UX.

## Open questions

- **Async functions in extensions** — e.g., «look up DNS» — breaks non-blocking eval design. Probably never allow in v1.
- **Type inference at compile time** — full static typing is a big project, probably deferred. v1 is runtime-typed with good error messages.
- **Expression debugger in UI** — step-through evaluation showing intermediate values. Nice-to-have for v1.5.
- **Expression autocomplete in editor** — needs schema introspection. v1.5 or v2.
- **Custom literal types** — e.g., `email("x@y.com")` for validation. Authors want this? Not in v1.
- **Parameterized functions** — `compose(f, g)(x)` for higher-order. Deliberately out of scope — YAGNI for workflow logic.
