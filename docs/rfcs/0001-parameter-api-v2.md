# RFC 0001: Parameter API v2 — Type-Safe Schema Architecture

**Status:** Draft  
**Created:** 2026-03-07  
**Authors:** AI Code Review (Claude 4.6 + GPT 5.4 + GPT 5.3 Codex synthesis)  
**Target:** `nebula-parameter` v0.x → v1.0  

---

## Summary

This RFC proposes a breaking architectural redesign of `nebula-parameter` to achieve:
- Clean separation of schema definition, runtime values, validation execution, and UI metadata
- Type-safe numeric semantics (integer/decimal split, no silent `f64` coercion)
- Lossless subtype preservation (no `unwrap_or_default` fallback)
- Policy-driven validation with deterministic error reporting
- Unified expression model for validation and conditional UI behavior
- Credential-safe value handling (redaction and secret references)
- Legacy JSON wire-format compatibility through explicit adapters

**Core principle:** Schema is the source of truth; `ParameterDef` becomes a legacy compatibility layer.

---

## Motivation

### Current Pain Points

1. **Semantic Loss in Typed → Legacy Conversion**
   - `Number::<Port>::new()` stores `u16`, but converts to `f64` in `ParameterDef::Number`
   - Unknown subtypes silently degrade: `from_name(...).unwrap_or_default()`
   - Loss of precision for large integers and decimals

2. **Validation Model Gaps**
   - `Mode` validation doesn't check active variant's nested fields
   - `Custom` validation rules skipped silently when expression engine unavailable
   - No policy for unknown keys in `Object`
   - Non-deterministic error order

3. **Mixed Responsibilities**
   - Schema, validation, UI hints, and runtime values tangled in same types
   - `ParameterDef` enum serves as both schema and wire format
   - No pre-compiled validation plan

4. **Runtime Value Access**
   - `ParameterValues` is raw `HashMap<String, Value>` with no schema awareness
   - No typed path accessors (`values.get_path("db.host")`)
   - Full clone required for nested access

---

## Design

### Architecture: 4 Clean Layers

```
┌─────────────────────────────────────────────────┐
│  1. Schema Core (canonical model)              │
│     FieldDef, ValueSpec, Constraints            │
└─────────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────────┐
│  2. Validation Engine (pre-compiled)           │
│     ValidationPlan, CompiledSchema              │
└─────────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────────┐
│  3. Runtime Values (typed access)              │
│     ParameterValues (dynamic storage)           │
│     ValidatedValues (schema-bound view)         │
└─────────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────────┐
│  4. UI Layer (presentation hints)              │
│     UiHints, DisplayRules, LocalizedText        │
└─────────────────────────────────────────────────┘

         Legacy ParameterDef ←→ Schema Core
              (adapter layer)
```

---

## Detailed Proposal

### Terminology

- `ConstraintRule`: schema-level validation constraint attached to `Constraints`.
- `ExpressionRule`: expression rule (`ExprTarget + Expression`) used for validation conditions and UI conditions.
- `OptionLoadStrategy`: declaration of how options are loaded (`load_options` vs `list_search`).
- `OptionProvider`: runtime execution interface that resolves options using the declared strategy.

### Relationship to Existing Systems

- `Expression` (this RFC): declarative, serializable schema rules for validation and UI conditions.
- `nebula-expression`: runtime expression parser/evaluator for `ValueSource::Expression { code }` and delegated custom rules.
- Existing display API (`DisplayCondition`, `DisplayRuleSet`): compatibility surface that maps to `ExpressionRule` in migration phases.
- `ValidationPlan` ownership: `nebula-parameter` owns schema compilation and deterministic traversal metadata; `nebula-validator` executes compiled `ConstraintRule` sets and emits structured `ParameterError` values.

### Rust Naming Conventions

- Public types use `PascalCase`; fields and methods use `snake_case`.
- Public API names prefer full domain terms over short aliases.
- Wire naming stays language-neutral via `serde(rename_all = "snake_case")`.

### 1. Schema Core

```rust
/// Stable field identifier (newtype for clarity).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FieldId(String);

/// Canonical field definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub id: FieldId,
    pub value_spec: ValueSpec,
    pub constraints: Constraints,
    pub default: Option<serde_json::Value>,
    pub ui_hints: Option<UiHints>,
}

/// Value type specification (no validation, no UI).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ValueSpec {
    Text {
        subtype: SubtypeRef<TextSubtype>,
        multiline: bool,
        sensitive: bool,
    },
    Number {
        subtype: SubtypeRef<NumberSubtype>,
        kind: NumberKind,
    },
    Boolean {
        subtype: SubtypeRef<BooleanSubtype>,
    },
    Select {
        options: Vec<SelectOption>,
        option_source: OptionSource,
        multiple: bool,
    },
    Object {
        fields: Vec<FieldDef>,
        allow_unknown: bool,
    },
    List {
        item_spec: Box<ValueSpec>,
    },
    Mode {
        variants: IndexMap<String, ModeVariant>,
    },
    // ... other types
}

/// Numeric domain specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NumberKind {
    Integer {
        bits: IntBits,
        signed: bool,
    },
    Decimal {
        scale: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum IntBits {
    I8, I16, I32, I64,
    U8, U16, U32, U64,
}

/// Subtype reference: known enum or custom string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SubtypeRef<T> {
    Known(T),
    Custom(String),
}

/// Validation constraints (domain-agnostic).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Constraints {
    pub required: bool,
    pub rules: Vec<ConstraintRule>,
}

/// Schema-level validation rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConstraintRule {
    Standard { name: String },
    Expr { rule: ExpressionRule },
}

/// Presentation metadata only; no validation semantics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiHints {
    pub label: Option<String>,
    pub description: Option<String>,
    pub placeholder: Option<String>,
    pub group: Option<String>,
    pub hint: Option<String>,
}

/// Canonical select option shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    pub key: String,
    pub name: String,
    pub value: serde_json::Value,
    pub description: Option<String>,
    pub disabled: bool,
}

/// Mode variant definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeVariant {
    pub label: String,
    pub description: Option<String>,
    pub fields: Vec<FieldDef>,
}

/// Schema is a collection of fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub fields: Vec<FieldDef>,
}

/// Option source for select-like fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum OptionSource {
    Static,
    Dynamic {
        provider: String,
        cache_ttl_ms: Option<u64>,
    },
}
```

**Key Changes:**
- `FieldId` is explicit newtype (not raw `String`)
- `ValueSpec` is pure type definition (no validation, no default)
- `NumberKind` explicitly models integer vs decimal
- `SubtypeRef` preserves custom subtypes (no silent fallback)
- `ModeVariant` includes nested fields for proper validation
- `IndexMap` preserves deterministic variant traversal order
- `OptionSource` enables static and dynamic option providers

---

### 2. Validation Engine

```rust
/// Pre-compiled validation plan.
#[derive(Debug)]
pub struct ValidationPlan {
    fields: Vec<CompiledField>,
    topology: Vec<FieldId>, // deterministic traversal order
}

#[derive(Debug)]
struct CompiledField {
    id: FieldId,
    validators: Vec<Box<dyn Validator>>,
    depends_on: Vec<FieldId>,
}

/// Validation policy.
#[derive(Debug, Clone)]
pub struct ValidationPolicy {
    pub unknown_keys: UnknownKeysPolicy,
    pub custom_rules: CustomRulePolicy,
    pub collect_all_errors: bool,
    pub max_depth: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum UnknownKeysPolicy {
    Ignore,
    Warn,
    Reject,
}

#[derive(Debug, Clone, Copy)]
pub enum CustomRulePolicy {
    Skip,
    Error,
    Delegate,
}

impl Schema {
    /// Compile schema into optimized validation plan.
    pub fn compile(&self, policy: ValidationPolicy) -> Result<ValidationPlan, SchemaError> {
        // Pre-compile regexes, sort fields topologically, etc.
    }
}

impl ValidationPlan {
    /// Execute validation with deterministic error ordering.
    pub fn validate(&self, values: &ParameterValues) 
        -> Result<ValidatedValues<'_>, Vec<ParameterError>> 
    {
        // Fast-path validation with pre-compiled validators
    }
}

/// Structured validation error with path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterError {
    pub path: ValuePath,
    pub code: Cow<'static, str>,
    pub message: String,
    pub severity: Severity,
}

/// JSON-pointer-style path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValuePath(Vec<PathSegment>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathSegment {
    Field(String),
    Index(usize),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
}
```

**Key Changes:**
- Validation is pre-compiled for performance and determinism
- Errors include structured `ValuePath` (not just flat key)
- Policy-driven behavior (unknown keys, custom rules)
- Deterministic error order via topological sort

---

### 3. Runtime Values

```rust
/// Dynamic value storage (unchanged externally, but internal semantics clarified).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterValues {
    #[serde(flatten)]
    values: HashMap<String, serde_json::Value>,
}

/// Schema-validated view with typed accessors.
#[derive(Debug)]
pub struct ValidatedValues<'s> {
    schema: &'s ValidationPlan,
    raw: ParameterValues,
}

impl<'s> ValidatedValues<'s> {
    /// Get typed value for field (no re-validation).
    pub fn get<T: ExtractValue>(&self, id: &FieldId) -> Result<T, ValueError> {
        T::extract(&self.raw, id, &self.schema)
    }

    /// Get value by path (e.g., "db.connection.host").
    pub fn get_path<T: ExtractValue>(&self, path: &str) -> Result<T, ValueError> {
        let segments = ValuePath::parse(path)?;
        T::extract_path(&self.raw, &segments, &self.schema)
    }

    /// Borrow raw values (escape hatch).
    pub fn raw(&self) -> &ParameterValues {
        &self.raw
    }
}

/// Trait for typed value extraction.
pub trait ExtractValue: Sized {
    fn extract(
        values: &ParameterValues, 
        id: &FieldId,
        plan: &ValidationPlan,
    ) -> Result<Self, ValueError>;

    fn extract_path(
        values: &ParameterValues,
        path: &ValuePath,
        plan: &ValidationPlan,
    ) -> Result<Self, ValueError>;
}

// Implementations for String, i64, u16, f64, bool, Vec<T>, etc.
```

**Key Changes:**
- `ParameterValues` remains dynamic (JSON-compatible)
- `ValidatedValues` provides safe typed access post-validation
- `ExtractValue` trait for extensible typed accessors
- Path-based access for nested fields

---

### 4. Numeric Semantics

```rust
/// Runtime numeric value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NumberValue {
    Int(i64),
    UInt(u64),
    Decimal(rust_decimal::Decimal),
}

/// Numeric range bound.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberRange {
    pub min: Option<NumberBound>,
    pub max: Option<NumberBound>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberBound {
    pub value: NumberValue,
    pub inclusive: bool,
}

impl ExtractValue for i64 {
    fn extract(values: &ParameterValues, id: &FieldId, plan: &ValidationPlan) 
        -> Result<Self, ValueError> 
    {
        let value = values.get(id.as_str())?;
        match value {
            Value::Number(n) if n.is_i64() => Ok(n.as_i64().unwrap()),
            Value::Number(n) if n.is_u64() => {
                i64::try_from(n.as_u64().unwrap())
                    .map_err(|_| ValueError::overflow(id))
            }
            _ => Err(ValueError::type_mismatch(id, "integer")),
        }
    }
}

impl ExtractValue for u16 {
    fn extract(values: &ParameterValues, id: &FieldId, plan: &ValidationPlan) 
        -> Result<Self, ValueError> 
    {
        let value = values.get(id.as_str())?;
        match value {
            Value::Number(n) if n.is_u64() => {
                u16::try_from(n.as_u64().unwrap())
                    .map_err(|_| ValueError::overflow(id))
            }
            _ => Err(ValueError::type_mismatch(id, "u16")),
        }
    }
}
```

**Key Changes:**
- Explicit `NumberValue` enum (not `f64` everywhere)
- Typed extractors preserve integer semantics
- Range bounds are inclusive/exclusive-aware
- No precision loss for large integers

---

### 5. Legacy Compatibility

```rust
/// Legacy parameter definition (wire format only).
#[deprecated(since = "0.9.0", note = "Use Schema/FieldDef instead")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParameterDef {
    Text(TextParameter),
    Number(NumberParameter),
    // ... existing variants
}

/// Conversion with warnings.
#[derive(Debug, Clone)]
pub struct ConversionWarning {
    pub field: String,
    pub kind: WarningKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum WarningKind {
    UnknownSubtype,
    PrecisionLoss,
    UnsupportedFeature,
}

impl TryFrom<ParameterDef> for FieldDef {
    type Error = ConversionError;

    fn try_from(legacy: ParameterDef) -> Result<Self, Self::Error> {
        // Lossless conversion where possible, error on incompatible constructs
    }
}

impl From<FieldDef> for (ParameterDef, Vec<ConversionWarning>) {
    fn from(def: FieldDef) -> (ParameterDef, Vec<ConversionWarning>) {
        // Best-effort conversion with warnings for custom subtypes, etc.
    }
}

/// Compatibility API.
pub mod compat {
    pub fn from_legacy_json(json: &str) -> Result<(Schema, Vec<ConversionWarning>), ParseError>;
    pub fn to_legacy_json(schema: &Schema) -> Result<(String, Vec<ConversionWarning>), SerializeError>;
}
```

**Key Changes:**
- `ParameterDef` becomes `#[deprecated]` wire format
- Explicit conversion with `ConversionWarning` tracking
- No silent `unwrap_or_default()` fallback
- Compatibility module for migration

---

### 6. Expression Integration

```rust
/// Where an expression is evaluated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExprTarget {
    /// Validate or evaluate the current field value.
    Local,
    /// Evaluate another field by path for cross-field checks.
    Field(ValuePath),
}

/// Unified expression used by validation and UI conditions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Expression {
    Eq { value: serde_json::Value },
    Ne { value: serde_json::Value },
    MinLength { value: usize },
    MaxLength { value: usize },
    Min { value: NumberValue },
    Max { value: NumberValue },
    Contains { value: serde_json::Value },
    Matches { pattern: String },
    IsTrue,
    IsFalse,
    Required,
    And { items: Vec<Expression> },
    Or { items: Vec<Expression> },
    Not { item: Box<Expression> },
}

/// Expression rule ties target and expression together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpressionRule {
    pub target: ExprTarget,
    pub expr: Expression,
}

/// Expression policy controls where expressions are allowed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ExpressionPolicy {
    Forbidden,
    Allowed,
    /// Allows expressions with a restricted capability set:
    /// - no secret plaintext materialization
    /// - no cross-tenant/runtime-global side-channel access
    /// - cross-field access only to declared dependencies
    Restricted,
}

/// Runtime source of a field value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ValueSource {
    Literal,
    Expression { code: String },
    SecretRef { key: String },
    EnvironmentRef { key: String },
}

impl ExpressionRule {
    pub fn local(expr: Expression) -> Self {
        Self {
            target: ExprTarget::Local,
            expr,
        }
    }

    pub fn field(path: ValuePath, expr: Expression) -> Self {
        Self {
            target: ExprTarget::Field(path),
            expr,
        }
    }
}
```

**Key Changes:**
- One expression model for validation, `visible_if`, and `required_if`
- Cross-field rules are path-based and schema-aware
- Value source is explicit (`Literal`, `Expression`, `SecretRef`, `EnvironmentRef`)
- Expression usage is controlled by per-field policy

---

### 7. Deterministic Ordering Contract

Deterministic behavior is a hard requirement for reproducible validation and stable UX.

```rust
/// Ordering contract for schema collections.
pub struct OrderingContract {
    /// Root field order must be preserved exactly as declared.
    pub preserve_field_declaration_order: bool,
    /// Object field traversal must be stable.
    pub preserve_object_field_order: bool,
    /// Mode variant traversal must be stable.
    pub preserve_mode_variant_order: bool,
    /// Error emission order must be stable across runs.
    pub stable_error_order: bool,
}
```

**Contract Rules:**
- All schema maps that affect traversal use deterministic map types (`IndexMap` or `BTreeMap`)
- `ValidationPlan::topology` is canonical and stable for identical input schema
- For nodes with no dependency edge between them, tie-break by schema declaration order
- Error ordering is defined as `topology order -> constraint/expression rule order -> path lexical tie-break`
- Cycles in expression or dynamic dependencies fail schema compilation with explicit `SchemaError`
- Legacy compatibility conversion must preserve declaration order where representable

---

### 8. Credential Security Model

```rust
/// Secret value handle; never stores plaintext in schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRef {
    pub provider: String,
    pub key: String,
}

/// Controls how values may be exposed in APIs and logs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SecretExposurePolicy {
    Never,
    Redacted,
    Explicit,
}

/// Field-level runtime security settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPolicy {
    pub secret_exposure: SecretExposurePolicy,
    pub allow_expression: ExpressionPolicy,
    pub include_in_telemetry: bool,
}
```

**Security Guarantees:**
- Secret-backed fields never serialize plaintext by default
- Validation and expression errors for secret fields use redacted messages
- Telemetry/events include structural metadata but exclude secret payloads
- Credential fields default to `ExpressionPolicy::Restricted`
- Redaction applies consistently across: validation errors, action logs, traces, and runtime telemetry events

---

### 9. Dynamic Option Providers

```rust
/// Request context for resolving dynamic options.
#[derive(Debug, Clone)]
pub struct OptionRequest {
    pub field: FieldId,
    pub tenant: Option<String>,
    pub user: Option<String>,
    pub values: ParameterValues,
}

/// Dynamic option provider interface.
#[async_trait::async_trait]
pub trait OptionProvider: Send + Sync {
    fn key(&self) -> &str;

    async fn resolve(
        &self,
        request: &OptionRequest,
        query: Option<&OptionQuery>,
    ) -> Result<OptionPage, OptionProviderError>;
}
```

**Key Changes:**
- Select-like fields can use static or provider-backed options
- Providers are schema-keyed for plugin/extension interoperability
- Cache behavior is explicit and policy-driven
- `OptionProvider` is the runtime executor for declared loading strategies

---

### 10. n8n-Compatible Dynamic Loading Contracts

To support resource/operation UX patterns similar to n8n, dynamic options require explicit
dependency, search, and routing contracts.

```rust
/// Dynamic option loading strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OptionLoadStrategy {
    /// Provider resolves a full option list.
    LoadOptions {
        method: String,
        depends_on: Vec<ValuePath>,
    },
    /// Provider resolves searchable list pages.
    ListSearch {
        method: String,
        filter_required: bool,
    },
}

/// Search and pagination context for dynamic options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OptionQuery {
    pub filter: Option<String>,
    pub pagination_token: Option<String>,
}

/// Provider response with paging metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionPage {
    pub options: Vec<SelectOption>,
    pub pagination_token: Option<String>,
    pub has_more: bool,
}

/// Resource locator mode (n8n-compatible mental model).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceLocatorMode {
    Id,
    Url,
    List,
    Custom(String),
}

/// Resource locator value shape for runtime/UI interoperability.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ResourceLocatorValue {
    Id {
        value: String,
        cached_result_name: Option<String>,
    },
    Url {
        value: String,
        cached_result_name: Option<String>,
        cached_result_url: Option<String>,
    },
    List {
        value: String,
        cached_result_name: Option<String>,
        cached_result_url: Option<String>,
    },
    Custom {
        id: String,
        value: String,
    },
}

/// Field-level dynamic UX options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicUiPolicy {
    pub searchable: bool,
    pub allow_arbitrary_values: bool,
    pub resolvable_field: bool,
    pub slow_load_notice: Option<SlowLoadNotice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlowLoadNotice {
    pub message: String,
    pub timeout_ms: u64,
}
```

**Compatibility Notes:**
- `depends_on` supports deterministic invalidation of cached option pages
- Any `depends_on` change invalidates only dependent field caches (not global cache state)
- If a dependency becomes hidden or unresolved, dependent options are reset to empty until re-resolved
- `allow_arbitrary_values` allows expressions or manually entered IDs not currently in list
- `allow_arbitrary_values` bypasses membership checks only; type, shape, and security validation still apply
- `resolvable_field` allows deferred resolution during credential setup/runtime hydration
- `ResourceLocatorValue` provides stable wire format for `id/url/list` mode switching
- `OptionLoadStrategy` declares invocation semantics; `OptionProvider` performs resolution

---

## Migration Strategy

### Phase 1: Foundation (v0.9.0, non-breaking)

**Timeline:** 30 days

**Goals:**
- Introduce new core types alongside existing API
- Add deprecation warnings
- Provide parallel path for new adopters

**Changes:**
1. Add `schema_core` module:
    - `FieldId`, `FieldDef`, `ValueSpec`, `SubtypeRef`, `NumberKind`, `OptionSource`
2. Add `validation` module:
   - `ValidationPlan`, `ValidationPolicy`, `ParameterError`, `ValuePath`
3. Add `expression` module:
    - `Expression`, `ExpressionRule`, `ExprTarget`, `ExpressionPolicy`, `ValueSource`
4. Add `security` module:
    - `SecretRef`, `SecurityPolicy`, `SecretExposurePolicy`
5. Add `ValidatedValues` wrapper
6. Implement `TryFrom<ParameterDef> for FieldDef`
7. Add `Schema::compile()` method
8. Add `#[deprecated]` attributes to existing number/subtype APIs
9. Update docs with migration guide
10. Add n8n-compatible dynamic contracts:
    - `OptionLoadStrategy`, `OptionQuery`, `OptionPage`, `ResourceLocatorValue`, `DynamicUiPolicy`
11. Add missing schema primitives to make RFC type-complete:
    - `ConstraintRule`, `UiHints`, `SelectOption`, `FieldDef::default`

**Compatibility:**
- All existing APIs remain functional
- New APIs are opt-in
- Feature flag `legacy-v1` enables old behavior without warnings

---

### Phase 2: Dual Run (v0.10.0, minor breaking)

**Timeline:** 30 days

**Goals:**
- Switch default to new API
- Validate correctness via parallel execution
- Collect metrics on conversion warnings

**Changes:**
1. Typed builders produce `FieldDef` by default
2. Legacy `ParameterDef` constructors behind `legacy` module
3. CI validates both paths produce equivalent validation results
4. Log `ConversionWarning` in production with telemetry
5. Numeric extractors use typed paths (no automatic `f64` coercion)
6. Subtype preservation enforced (fail on unknown subtype in strict mode)
7. Enable unified expression runtime for both validation and visibility
8. Add dynamic option provider adapter and cache policy
9. Move dynamic provider execution to async contract (`OptionProvider::resolve`)
10. Enforce credential-safe redaction in logs and API responses
11. Add resource-locator mode switching and list-search paging support

**Compatibility:**
- Legacy constructors still available via `nebula_parameter::legacy::`
- Automatic migration lint: `cargo fix --lib --allow-dirty`
- Feature flag `strict-validation` enables new error policies

---

### Phase 3: Major Release (v1.0.0, breaking)

**Timeline:** 30 days

**Goals:**
- Remove deprecated APIs
- Finalize stable contracts
- Lock wire format

**Changes:**
1. Remove `ParameterDef` constructors (keep only as serde shape)
2. Remove `unwrap_or_default()` fallback paths
3. Make `ValidationPolicy::Reject` default for unknown keys
4. Remove `#[deprecated]` shims
5. Stabilize `Schema` serde representation
6. Publish crate-level SemVer policy
7. Stabilize expression and security contracts (`Expression`, `ValueSource`, `SecurityPolicy`)
8. Stabilize dynamic loading contracts (`OptionLoadStrategy`, `ResourceLocatorValue`)
9. Keep `ParameterDef` in `compat::legacy` as serde-facing legacy shape

**Compatibility:**
- `nebula-parameter-legacy` compatibility crate (separate package)
- Automated migration tool: `nebula-parameter-migrate`
- Stable 1.0 API with SemVer guarantees

---

## Acceptance Criteria

### Correctness
- [ ] Zero silent subtype downgrades in conversion
- [ ] Deterministic validation error order (stable between runs)
- [ ] No precision loss for integer-only domains (ports, timestamps, indices)
- [ ] Mode validation checks active variant fields
- [ ] Expression behavior is consistent across validation and UI conditions
- [ ] Credential fields do not leak secrets via errors, logs, or telemetry
- [ ] Dynamic option providers do not change deterministic validation outcomes
- [ ] `depends_on` invalidation reloads only affected dynamic fields
- [ ] Resource locator mode switches (`id/url/list`) preserve semantic value integrity

### Performance
- [ ] Pre-compiled validation ≥20% faster than baseline for deep schemas
- [ ] Path access without full clone for nested values
- [ ] Regex compilation cached in `ValidationPlan`
- [ ] Visibility/expression recomputation is incremental based on rule dependencies
- [ ] Option provider caching reduces repeated lookups under identical context
- [ ] List-search pagination handles large catalogs without blocking UI render

### Compatibility
- [ ] Full round-trip: `ParameterDef → FieldDef → ParameterDef` (with warnings)
- [ ] Legacy JSON deserializes without data loss
- [ ] Migration tool handles 100% of existing schemas
- [ ] Existing action/credential schemas can opt into expression policies without rewrites
- [ ] n8n-style dynamic option patterns map to `OptionLoadStrategy` without ad-hoc adapters

### Developer Experience
- [ ] Typed extractors for common Rust types (i64, u16, String, Vec<T>, etc.)
- [ ] Clear error messages with field paths
- [ ] Comprehensive migration guide with examples
- [ ] Deprecation warnings in v0.9+ guide users to new API
- [ ] Fluent conditional API (`when(...).eq(...)`) for `visible_if` and `required_if`
- [ ] Clear docs for secure credential handling with `SecretRef`
- [ ] Dynamic field APIs clearly document `load options` vs `list search` tradeoffs

---

## Alternatives Considered

### 1. Keep Flat `ParameterDef` Enum
**Rejected:** Mixing schema/validation/UI in one type limits extensibility and creates lifetime issues.

### 2. Fully Generic `ParameterValues<S: Schema>`
**Rejected:** Would break JSON interop and complicate dynamic use cases (plugins, REPL, etc.).

### 3. Use `f64` for All Numerics
**Rejected:** Silent precision loss for integers and decimals is a correctness bug.

### 4. Silent Subtype Coercion
**Rejected:** Data loss in plugin/extension scenarios is unacceptable for production systems.

---

## Future Extensions (Post-1.0)

- **Schema Linting:** `Schema::lint()` detects duplicate keys, unreachable fields, circular dependencies
- **Derive Macros:** `#[derive(ParameterSchema)]` for struct → schema generation
- **Plugin Registry:** `SubtypeRegistry`, `ValidatorPlugin`, `OptionProvider` with versioned contracts
- **Expression Language:** Safe subset for `Custom` validation rules with static analysis and cost limits
- **Async Validation:** Support for remote option providers and async validators
- **Credential Vault Integrations:** Native secret providers with rotation and lease semantics

---

## References

- Current implementation: `crates/parameter/`
- Inspiration: [paramdef](https://github.com/vanyastaff/paramdef), Blender RNA, JSON Schema
- Related RFCs: (none yet)

---

## Appendix: Example Migration

### Before (v0.8.x)
```rust
use nebula_parameter::types::NumberParameter;
use nebula_parameter::subtype::NumberSubtype;

let port = NumberParameter::new("port", "Port")
    .subtype(NumberSubtype::Port)
    .default_value(8080.0)
    .range(1.0, 65535.0);
```

### After (v1.0)
```rust
use nebula_parameter::schema::{FieldDef, ValueSpec, NumberKind, IntBits, SubtypeRef};
use nebula_parameter::subtype::std_subtypes::Port;

let port = FieldDef::builder(FieldId::new("port"))
    .label("Port")
    .value_spec(ValueSpec::Number {
        subtype: SubtypeRef::Known(Port),
        kind: NumberKind::Integer {
            bits: IntBits::U16,
            signed: false,
        },
    })
    .default_value(8080u16)
    .range(1u16..=65535u16)
    .build();
```

### Or with Type Alias
```rust
use nebula_parameter::typed::{PortParam, FieldId};

let port = PortParam::builder(FieldId::new("port"))
    .label("Port")
    .default_value(8080)
    .build();
```

---

## Status: DRAFT → Review → Accepted → Implemented

## Implementation Plan (Documentation-Only)

This section captures planning decisions and delivery checkpoints only. No implementation
work is started until this plan is explicitly approved.

### Scope Freeze (v1 Mandatory)

1. Schema/runtime separation with immutable schema contracts.
2. Unified expression model for validation and UI conditions.
3. Deterministic ordering for traversal, validation, and error emission.
4. Dynamic option loading contracts (`load options` and `list search`).
5. Resource locator runtime value (`id`/`url`/`list` mode shape).
6. Credential-safe security contracts (redaction and secret references).

### Nebula Crate Integration Plan

This RFC defines contracts. Execution responsibilities are split across existing Nebula crates.

1. `nebula-parameter`:
Owns schema authoring model (`FieldDef`, `ValueSpec`, `UiHints`, `ExpressionRule` declarations),
compat adapters, and deterministic traversal metadata.
2. `nebula-validator`:
Owns validation execution for `ConstraintRule` and schema policies. `nebula-parameter`
provides compiled plans/inputs (ordered fields, dependency graph, precompiled regex metadata);
`nebula-validator` interprets those plans and returns structured `ParameterError` values.
3. `nebula-expression`:
Owns runtime parsing/evaluation for expression-backed values (`ValueSource::Expression`) and delegated
custom expression rules. `ExpressionRule` in this RFC remains declarative schema data.
Evaluation is policy-gated by `ExpressionPolicy` declared in schema/security settings.
4. `nebula-action`:
Consumes compiled parameter contracts for node/action configuration and runtime parameter resolution.
5. `nebula-credential`:
Consumes security-aware parameter contracts for secrets and resolvable credential fields.
6. `nebula-runtime`:
Coordinates evaluation order: resolve expressions, load dynamic options, validate inputs, then execute actions.

### Display System Plan

Display behavior is schema-driven and deterministic, with evaluation delegated to expression and validation engines.

1. `UiHints` stores presentation metadata only (labels, placeholders, grouping, notices, editor hints).
2. `ExpressionRule` drives `visible_if`, `required_if`, and `enabled_if` conditions.
3. `OptionLoadStrategy` + `OptionProvider` drive dynamic option widgets (`load options`, `list search`).
4. `DynamicUiPolicy` defines UX semantics (`searchable`, `slow_load_notice`, `allow_arbitrary_values`).
5. Display recomputation is dependency-scoped using `depends_on` and expression field dependencies.

### Validation and Expression Execution Order

Order is fixed to avoid ambiguous behavior between UI, expressions, and hard validation.

1. Apply defaults and source resolution (`Literal`, `SecretRef`, `EnvironmentRef`, `Expression`).
2. Execute expression evaluation through `nebula-expression` under `ExpressionPolicy`.
3. Refresh dynamic options for affected fields (`depends_on` invalidation).
4. Run structural and constraint validation through `nebula-validator`.
5. Emit deterministic errors and UI state updates.
6. For async providers, complete option resolution before final validation pass for affected fields.

### Integration Nuances (Plan-Level)

1. Expression failures on secret-backed fields must produce redacted errors.
2. Dynamic option failures must not invalidate unrelated fields.
3. `allow_arbitrary_values` applies to option-membership checks, not to type checks.
4. `resolvable_field` values may be unresolved at design-time but must be resolvable by execution-time policy.
5. Unknown-key behavior must be consistent between display hydration and validator execution.

### Industry Pattern Mapping

| Industry Pattern | Source System | Nebula Contract |
|---|---|---|
| Subtype + Unit semantics | Blender RNA | `SubtypeRef<T>` + unit-aware numeric metadata |
| Soft/Hard numeric limits | Blender RNA, Houdini | UI hints (`soft_*`) separated from enforced constraints (`hard_*`) |
| Conditional field visibility | n8n, Houdini, Unreal | Unified `ExpressionRule`/`Expression` for `visible_if` and `required_if` |
| Resource/operation dynamic loading | n8n | `OptionLoadStrategy`, `depends_on`, `OptionQuery`, `OptionPage` |
| Resource locator value modes | n8n | `ResourceLocatorValue` with `id`/`url`/`list` mode contract |
| Sensitive parameter handling | NiFi, n8n | `SecurityPolicy`, `SecretRef`, redacted errors/logs/telemetry |
| Reset to default behavior | Qt, Houdini | Schema-declared defaults and explicit reset semantics |
| Value coercion hooks | WPF | Transformer/coercion stage prior to final validation |
| Change notifications | Qt, WPF, Unreal | Event hooks for value/visibility/validation changes |

### Phase Gates

1. Plan Gate:
Define final contracts and non-goals. Lock API names at RFC level.
2. Design Gate:
Review domain parity for Action/Credential and approve security defaults.
3. Verification Gate:
Approve test matrix and acceptance criteria before writing production code.

### Test Matrix (Planning)

1. Determinism:
Stable ordering for validation and errors across repeated runs.
2. Numeric correctness:
No precision loss for integer and decimal domains.
3. Dynamic loading:
Dependency-based invalidation and paginated list-search correctness.
4. Resource locator:
`id`/`url`/`list` mode switching preserves semantic identity.
5. Security:
No secret leakage via serialization, error messages, logs, or telemetry.
6. Compatibility:
Legacy-to-v2 conversion with explicit warnings and documented caveats.

### Non-Goals for v1

1. Full plugin marketplace/runtime registry implementation.
2. Cross-service distributed cache for dynamic options.
3. Expression language static optimizer beyond safety/cost limits.
4. Automated migration CLI with full codemod support.

**Next Steps:**
1. Team review and feedback (7 days)
2. RFC refinement based on comments
3. Approval decision
4. Implementation tracking issue
5. Phased rollout per migration strategy
