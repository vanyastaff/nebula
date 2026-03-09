# Working Paper: Parameter API v2 — Type-Safe Schema Architecture

**Type:** Working Paper  
**Status:** Draft  
**Created:** 2026-03-07  
**Updated:** 2026-03-08  
**Authors:** AI Code Review (Claude 4.6 + GPT 5.4 + GPT 5.3 Codex synthesis)  
**Canonical RFC:** RFC 0001 (`0001-parameter-schema-v2.md`)  
**Target:** `nebula-parameter` v0.x → v1.0  

---

## Summary

This working paper explores a breaking architectural redesign of
`nebula-parameter` to achieve:
- Clean separation of schema definition, runtime values, validation execution, and UI metadata
- Type-safe numeric semantics (integer/decimal split, no silent `f64` coercion)
- No subtype field in schema surface (pattern-rule shortcuts only)
- Policy-driven validation with deterministic error reporting
- Unified expression model for validation and conditional UI behavior
- Credential-safe value handling (redaction and secret references)
- Legacy JSON wire-format compatibility through explicit adapters

**Core principle:** Schema core is the source of truth for the internal model;
`ParameterDef` remains a boundary compatibility layer.

## Status In The RFC Set

This document is non-normative for the public JSON shape.

Use it to reason about internal Rust layering, validation architecture, and
migration boundaries. If it conflicts with RFC 0001, RFC 0001 wins for the
wire contract.

## Implementation Bridge To RFC 0001

Use this working paper as the internal implementation guide behind RFC 0001,
not as a competing public contract.

Recommended realization order:
1. Implement richer internal schema/value/validation types as described here.
2. Emit the canonical HTTP schema defined by RFC 0001 at API boundaries.
3. Use adapters only in one-time migration/import tooling.
4. For dynamic providers, follow the shared versioned envelope standardized in
    RFC 0002 and consumed by RFC 0004.

For v1, any internal type that cannot be represented in the RFC 0001 wire
contract must stay internal.

## Naming Decision

The crate remains `nebula-parameter` in v1.

Naming split:
- `parameter`: domain boundary, crate/package naming, runtime payload concepts,
    and migration concepts
- `field`: canonical schema-definition unit in the v2 authoring model

Recommended public module layout:

```rust
nebula_parameter::schema::{Schema, Field, UiElement, Group, Rule, Condition}
nebula_parameter::runtime::{ParameterValues, ValidatedValues}
nebula_parameter::providers::{OptionProvider, DynamicRecordProvider}
nebula_parameter::migration::{import_v1_json}
```

Internal implementation may still use richer types such as `FieldDef`, but the
public v2 authoring surface should prefer `Field` to match RFC 0001.

---

## Motivation

### Current Pain Points

1. **Semantic Loss in Typed → Legacy Conversion**
   - `Number::<Port>::new()` stores `u16`, but converts to `f64` in `ParameterDef::Number`
   - Legacy subtype shortcuts silently degrade: `from_name(...).unwrap_or_default()`
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

    Boundary compatibility: Legacy ParameterDef ←→ Schema Core
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
        multiline: bool,
        sensitive: bool,
    },
    Number {
        kind: NumberKind,
    },
    Boolean,
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

/// Nebula does not model subtype as a first-class schema field.
/// Semantic checks are represented explicitly in `Constraints.rules`
/// (for example `Rule::Pattern` with predefined regex shortcuts).

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
- No dedicated subtype field in schema; semantic checks are explicit rules
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

### 5. One-Time Migration Input

```rust
/// Migration warnings returned by v1 -> v2 import.
#[derive(Debug, Clone)]
pub struct ConversionWarning {
    pub field: String,
    pub kind: WarningKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum WarningKind {
    DeprecatedSubtypeShortcut,
    PrecisionLoss,
    UnsupportedFeature,
}

/// Import-only API used during migration.
pub mod migration {
    pub fn import_v1_json(json: &str) -> Result<(Schema, Vec<ConversionWarning>), ParseError>;
}
```

**Key Changes:**
- Explicit import with `ConversionWarning` tracking
- No silent `unwrap_or_default()` fallback
- No runtime `legacy` module in v1 public API

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

/// Shared provider response envelope used across dynamic option and
/// field-schema providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicProviderEnvelope<T> {
    pub response_version: u16,
    pub kind: DynamicResponseKind,
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub schema_version: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicResponseKind {
    Options,
    Fields,
}

/// Dynamic option provider interface.
#[async_trait::async_trait]
pub trait OptionProvider: Send + Sync {
    fn key(&self) -> &str;

    async fn resolve(
        &self,
        request: &OptionRequest,
        query: Option<&OptionQuery>,
    ) -> Result<DynamicProviderEnvelope<SelectOption>, OptionProviderError>;
}

/// Dynamic field-schema provider interface used by `Field::dynamic_record`.
#[async_trait::async_trait]
pub trait DynamicRecordProvider: Send + Sync {
    fn key(&self) -> &str;

    async fn resolve_fields(
        &self,
        request: &OptionRequest,
    ) -> Result<DynamicProviderEnvelope<DynamicFieldSpec>, OptionProviderError>;
}
```

**Key Changes:**
- Select-like fields can use static or provider-backed options
- Providers are schema-keyed for plugin/extension interoperability
- Cache behavior is explicit and policy-driven
- `OptionProvider` and `DynamicRecordProvider` share one response envelope
- v1 keeps separate object-safe provider traits to avoid associated-type
  complexity at plugin boundaries

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
- `OptionLoadStrategy` declares invocation semantics; providers resolve through
    the shared versioned envelope
- `depends_on` values are part of provider cache keys and invalidation scope

---

## Migration Strategy

### Phase 1: Cutover Preparation (v0.9.x)

**Timeline:** 30 days

**Goals:**
- Finalize v2 surface and migration importer
- Remove dependency on legacy constructors in active codepaths

**Changes:**
1. Add `schema_core` module:
    - `FieldId`, `FieldDef`, `ValueSpec`, `NumberKind`, `OptionSource`
2. Add `validation` module:
   - `ValidationPlan`, `ValidationPolicy`, `ParameterError`, `ValuePath`
3. Add `expression` module:
    - `Expression`, `ExpressionRule`, `ExprTarget`, `ExpressionPolicy`, `ValueSource`
4. Add `security` module:
    - `SecretRef`, `SecurityPolicy`, `SecretExposurePolicy`
5. Add `ValidatedValues` wrapper
6. Implement import-only `migration::import_v1_json`
7. Add `Schema::compile()` method
8. Remove new usage of legacy subtype shortcut APIs in first-party schemas
9. Update docs with migration guide
10. Add shared dynamic-provider contracts:
    - `DynamicProviderEnvelope`, `DynamicResponseKind`, `OptionLoadStrategy`, `OptionQuery`, `ResourceLocatorValue`, `DynamicUiPolicy`
11. Add missing schema primitives to make RFC type-complete:
    - `ConstraintRule`, `UiHints`, `SelectOption`, `FieldDef::default`

### Phase 2: Clean Cut Release (v1.0.0, breaking)

**Timeline:** 30 days

**Goals:**
- Ship only canonical v2 runtime API
- Keep migration importer as tooling entrypoint

**Changes:**
1. Remove `ParameterDef` and `ParameterCollection` from the public runtime API
2. Remove `unwrap_or_default()` fallback paths
3. Make `ValidationPolicy::Reject` default for unknown keys
4. Remove deprecated subtype shortcuts from public constructors
5. Stabilize `Schema` serde representation
6. Publish crate-level SemVer policy
7. Stabilize expression and security contracts (`Expression`, `ValueSource`, `SecurityPolicy`)
8. Stabilize dynamic loading contracts (`DynamicProviderEnvelope`, `OptionLoadStrategy`, `ResourceLocatorValue`)
9. Keep migration importer as an offline/tooling surface only

**Migration Tooling:**
- Automated migration tool: `nebula-parameter-migrate`
- Import-only API for existing JSON snapshots
- Stable 1.0 API with SemVer guarantees

---

## Acceptance Criteria

### Correctness
- [ ] Zero silent subtype-shortcut downgrades in conversion
- [ ] Deterministic validation error order (stable between runs)
- [ ] No precision loss for integer-only domains (ports, timestamps, indices)
- [ ] Mode validation checks active variant fields
- [ ] Expression behavior is consistent across validation and UI conditions
- [ ] Credential fields do not leak secrets via errors, logs, or telemetry
- [ ] Dynamic providers do not change deterministic validation outcomes
- [ ] `depends_on` invalidation reloads only affected dynamic fields
- [ ] Resource locator mode switches (`id/url/list`) preserve semantic value integrity

### Performance
- [ ] Pre-compiled validation ≥20% faster than baseline for deep schemas
- [ ] Path access without full clone for nested values
- [ ] Regex compilation cached in `ValidationPlan`
- [ ] Visibility/expression recomputation is incremental based on rule dependencies
- [ ] Provider caching reduces repeated lookups under identical context
- [ ] List-search pagination handles large catalogs without blocking UI render

### Compatibility
- [ ] v1 JSON imports into canonical schema with explicit warnings and no silent coercion
- [ ] Migration tool handles 100% of existing schemas
- [ ] Existing action/credential schemas can opt into expression policies without rewrites
- [ ] n8n-style dynamic option patterns map to `OptionLoadStrategy` and the
    shared versioned provider envelope without ad-hoc adapters

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

### 4. Legacy Subtype Shortcut Coercion
**Rejected:** Data loss in plugin/extension scenarios is unacceptable for production systems.

---

## Future Extensions (Post-1.0)

- **Schema Linting:** `Schema::lint()` detects duplicate keys, unreachable fields, circular dependencies
- **Derive Macros:** `#[derive(ParameterSchema)]` for struct → schema generation
- **Plugin Registry:** `ValidatorPresetRegistry`, `ValidatorPlugin`, `OptionProvider` with versioned contracts
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
use nebula_parameter::schema::Rule;

let port = NumberParameter::new("port", "Port")
    .rule(Rule::pattern("^([1-9][0-9]{0,4})$", "Must be a valid TCP port"))
    .default_value(8080.0)
    .range(1.0, 65535.0);
```

### After (v1.0)
```rust
use nebula_parameter::schema::{FieldDef, ValueSpec, NumberKind, IntBits};

let port = FieldDef::builder(FieldId::new("port"))
    .label("Port")
    .value_spec(ValueSpec::Number {
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
It must not be used for base form `Condition` evaluation, which stays a small
deterministic evaluator shared with the frontend.
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
| Validation shortcut semantics | Blender RNA inspiration | explicit `Rule::Pattern` + unit-aware numeric metadata |
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
6. Migration:
One-time v1 JSON import with explicit warnings and documented caveats.

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

