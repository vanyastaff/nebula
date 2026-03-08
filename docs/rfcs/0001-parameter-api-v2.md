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
        variants: HashMap<String, ModeVariant>,
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
    pub rules: Vec<ValidationRule>,
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
```

**Key Changes:**
- `FieldId` is explicit newtype (not raw `String`)
- `ValueSpec` is pure type definition (no validation, no default)
- `NumberKind` explicitly models integer vs decimal
- `SubtypeRef` preserves custom subtypes (no silent fallback)
- `ModeVariant` includes nested fields for proper validation

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

## Migration Strategy

### Phase 1: Foundation (v0.9.0, non-breaking)

**Timeline:** 30 days

**Goals:**
- Introduce new core types alongside existing API
- Add deprecation warnings
- Provide parallel path for new adopters

**Changes:**
1. Add `schema_core` module:
   - `FieldId`, `FieldDef`, `ValueSpec`, `SubtypeRef`, `NumberKind`
2. Add `validation` module:
   - `ValidationPlan`, `ValidationPolicy`, `ParameterError`, `ValuePath`
3. Add `ValidatedValues` wrapper
4. Implement `TryFrom<ParameterDef> for FieldDef`
5. Add `Schema::compile()` method
6. Add `#[deprecated]` attributes to existing number/subtype APIs
7. Update docs with migration guide

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

### Performance
- [ ] Pre-compiled validation ≥20% faster than baseline for deep schemas
- [ ] Path access without full clone for nested values
- [ ] Regex compilation cached in `ValidationPlan`

### Compatibility
- [ ] Full round-trip: `ParameterDef → FieldDef → ParameterDef` (with warnings)
- [ ] Legacy JSON deserializes without data loss
- [ ] Migration tool handles 100% of existing schemas

### Developer Experience
- [ ] Typed extractors for common Rust types (i64, u16, String, Vec<T>, etc.)
- [ ] Clear error messages with field paths
- [ ] Comprehensive migration guide with examples
- [ ] Deprecation warnings in v0.9+ guide users to new API

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
- **Expression Language:** Safe subset for `Custom` validation rules
- **Async Validation:** Support for remote option providers and async validators

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

**Next Steps:**
1. Team review and feedback (7 days)
2. RFC refinement based on comments
3. Approval decision
4. Implementation tracking issue
5. Phased rollout per migration strategy
