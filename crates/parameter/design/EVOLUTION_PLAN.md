# nebula-parameter — Evolution Plan

> **Status**: Draft  
> **Base**: RFC 0005 (Parameter v2 — Final Design, Accepted)  
> **Scope**: v2.1 → v3.0 evolution roadmap  
> **Breaking changes**: Allowed

---

## Executive Summary

`nebula-parameter` v2 (RFC 0005) delivers a solid 16-variant field model with declarative
validation, conditions, and async loaders. This plan addresses **six architectural gaps**
uncovered through analysis of n8n, Windmill, Activepieces, and Prefect, and through
mapping cross-crate integration requirements with `nebula-validator`, `nebula-expression`,
`nebula-credential`, `nebula-action`, and `nebula-runtime`.

### Gap Summary

| # | Gap | Impact |
|---|-----|--------|
| G1 | No expression resolution pipeline | Actions can't evaluate `{{ }}` values |
| G2 | Rules engine is parallel to `nebula-validator` | Duplicated validation logic |
| G3 | No type coercion or secret redaction utilities | Every consumer re-implements these |

### Guiding Principles: SOLID / DRY / KISS

Every proposed change is evaluated against:

- **SRP** — Each module has one responsibility. The parameter crate defines schemas;
  validation delegates to `nebula-validator`; expression evaluation delegates to
  `nebula-expression`.
- **OCP** — `Rule` and `ParameterError` are `#[non_exhaustive]` — new variants extend
  without breaking consumers.
- **DRY** — No new field types that duplicate existing ones. `Code` already covers JSON
  editing. `Mode` already covers resource-locator patterns. `List` already covers
  key-value assignment collections.
- **KISS** — No layout/UI hints (`FieldWidth`, `order`) in the data model. The parameter
  crate describes *what* data is needed, not *how* to render it. UI layout is the
  renderer's concern.
- **ISP** — `ExpressionResolver` is a narrow single-purpose trait.
- **DIP** — The parameter crate depends on abstractions (traits), not concrete engines.

---

## Competitive Analysis Summary

### n8n (TypeScript, 400+ nodes)
- **ResourceLocator**: Multi-mode resource finding (by ID, URL, name, list search).
- **ResourceMapper**: Dynamically maps fields from external API schemas.
- **AssignmentCollection**: Key-value pair editing for data transformation.
- **Routing declarations**: Declarative HTTP mapping per field (`request.body`, `request.query`, etc.).
- **displayOptions**: Conditional visibility with rich operator set (eq, gte, startsWith, includes, exists).
- **loadOptionsMethod**: String reference to method name + `loadOptionsDependsOn` for dependency tracking.
- **noDataExpression**: Per-field opt-out from expression support.
- **Codex AI descriptions**: Separate AI-optimized descriptions for LLM consumers.

### Windmill (Rust/Svelte, script-driven)
- **JSON Schema as wire format**: Maximum interop with external tools.
- **Script-inferred schemas**: Auto-generates parameter schema from function signatures.
- **Resource type references**: `format: "resource-postgresql"` links params to resource types.
- **Variable/secret injection**: Path-based injection (`wmill.get_resource()`).

### Activepieces (TypeScript, piece-based)
- **Refreshers system**: Explicit `refreshers: ["field_a", "field_b"]` array per dynamic property.
- **DynamicProperties**: Entire property groups regenerated based on other values.
- **Property processors**: Transform chain applied before validation.
- **TypeBox schema generation**: Runtime validation from property definitions.

### Prefect (Python, flow-based)  
- **Pydantic integration**: Parameter schemas from type annotations + Pydantic models.
- **OpenAPI schema export**: Parameter schemas as standard OpenAPI for UI rendering.
- **Runtime resolution**: `visit_collection` + `resolve_to_final_result` pipeline.

---

## Phase 1 — Expression Integration

**Priority**: Critical  
**Breaking**: Yes  
**Depends on**: `nebula-expression`  
**Crate impact**: `nebula-parameter`, `nebula-expression`, `nebula-action`, `nebula-runtime`

### 1.1 ExpressionMode enum

Replace `expression: bool` in `FieldMetadata` with a richer mode:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExpressionMode {
    /// Expressions are not allowed for this field.
    Disabled,
    /// Field accepts both literal and expression values (default).
    #[default]
    Enabled,
    /// Field MUST contain an expression (computed/derived fields).
    Required,
}
```

**Migration**: `expression: true` → `ExpressionMode::Enabled`,
`expression: false` → `ExpressionMode::Disabled`.

### 1.2 ExpressionResolver trait

Define in `nebula-parameter` so it stays decoupled from the expression engine:

```rust
/// Trait for resolving `{{ expression }}` values at runtime.
///
/// Implemented by `nebula-expression::ExpressionEngine`.
pub trait ExpressionResolver: Send + Sync {
    /// Resolve a single expression string to a concrete JSON value.
    fn resolve(
        &self,
        expression: &str,
        context: &serde_json::Value,
    ) -> Result<serde_json::Value, ExpressionResolveError>;

    /// Validate expression syntax without evaluation.
    fn validate_syntax(&self, expression: &str) -> Result<(), ExpressionResolveError>;
}

#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum ExpressionResolveError {
    #[error("syntax error in `{expression}`: {message}")]
    Syntax { expression: String, message: String },
    #[error("resolution failed for `{expression}`: {message}")]
    Resolution { expression: String, message: String },
    #[error("type mismatch in `{expression}`: expected {expected}, got {actual}")]
    TypeError { expression: String, expected: String, actual: String },
}
```

### 1.3 Schema expression methods

```rust
impl Schema {
    /// Resolve all expression-backed values in the given FieldValues.
    pub fn resolve_expressions(
        &self,
        values: &FieldValues,
        resolver: &dyn ExpressionResolver,
        context: &serde_json::Value,
    ) -> Result<FieldValues, Vec<ParameterError>>;

    /// Validate expression syntax for all expression-backed values.
    pub fn validate_expressions(
        &self,
        values: &FieldValues,
        resolver: &dyn ExpressionResolver,
    ) -> Vec<ParameterError>;
}
```

### 1.4 ParameterError extensions

```rust
// New variants:
pub enum ParameterError {
    // ... existing ...

    /// An expression failed to resolve.
    #[error("expression error for `{key}`: {message}")]
    ExpressionError { key: String, message: String },

    /// A required expression field has a literal value.
    #[error("field `{key}` requires an expression")]
    ExpressionRequired { key: String },
}
```

---

## Phase 2 — Validation Unification with nebula-validator

**Priority**: High  
**Breaking**: Yes (additive to Rule enum)  
**Depends on**: `nebula-validator`  
**Crate impact**: `nebula-parameter`, `nebula-validator`

### 2.1 Rule enum extensions

```rust
pub enum Rule {
    // ... existing 9 variants ...

    /// Cross-field validation expressed as an expression.
    /// Fields are referenced by id; the expression must evaluate to a boolean.
    CrossField {
        fields: Vec<String>,
        expression: String,
        message: Option<String>,
    },

    /// Composed validation using `nebula-validator` composable validators.
    /// Not serializable — runtime-only bridge.
    #[serde(skip)]
    Composed(ComposedRule),
}
```

### 2.2 Bridge to nebula-validator

```rust
/// A composed rule that wraps a `nebula-validator` validator.
/// Not serializable — exists only at runtime.
pub struct ComposedRule {
    name: String,
    validator: Box<dyn Fn(&serde_json::Value) -> Result<(), String> + Send + Sync>,
}

impl ComposedRule {
    pub fn new(
        name: impl Into<String>,
        validator: impl Fn(&serde_json::Value) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self;

    pub fn validate(&self, value: &serde_json::Value) -> Result<(), String>;
}
```

### 2.3 Rule categorization

```rust
impl Rule {
    /// Whether this rule can be evaluated without runtime context.
    pub fn is_static(&self) -> bool;

    /// Whether this rule needs expression engine (Custom, CrossField).
    pub fn needs_expression_engine(&self) -> bool;

    /// Whether this rule is serializable to JSON.
    pub fn is_serializable(&self) -> bool;
}
```

### 2.4 ValidationContext for expression-aware validation

```rust
/// Extended validation context for expression-backed rules.
pub struct ValidationContext<'a> {
    pub profile: ValidationProfile,
    pub expression_resolver: Option<&'a dyn ExpressionResolver>,
    pub expression_context: Option<&'a serde_json::Value>,
}

impl Schema {
    /// Full validation with optional expression engine.
    pub fn validate_with_context(
        &self,
        values: &FieldValues,
        ctx: &ValidationContext<'_>,
    ) -> ValidationReport;
}
```

---

## Phase 3 — Runtime Resolution Pipeline

**Priority**: High  
**Breaking**: Yes (ValidatedValues → richer type)  
**Depends on**: Phase 1, Phase 2  
**Crate impact**: `nebula-parameter`, `nebula-runtime`, `nebula-action`

### 3.1 Type coercion

```rust
impl Schema {
    /// Attempt to coerce values to match field types.
    ///
    /// Examples: `"42"` → `42` for Number fields, `"true"` → `true` for Boolean.
    pub fn coerce_values(
        &self,
        values: &FieldValues,
    ) -> Result<FieldValues, Vec<ParameterError>>;
}
```

### 3.2 Secret redaction

```rust
impl Schema {
    /// Return a copy with all secret-marked field values replaced
    /// by `"[REDACTED]"`.
    pub fn redact_secrets(&self, values: &FieldValues) -> FieldValues;

    /// Collect the ids of all fields marked as `secret`.
    pub fn secret_field_ids(&self) -> Vec<&str>;
}
```

### 3.3 Prepare-for-execution pipeline

```rust
/// The full resolution result, ready for action execution.
#[derive(Debug, Clone)]
pub struct PreparedValues {
    /// Fully resolved, coerced, and validated values.
    values: FieldValues,
    /// Same values with secrets replaced by "[REDACTED]".
    redacted: FieldValues,
}

impl PreparedValues {
    /// The execution-ready values.
    pub fn values(&self) -> &FieldValues;
    /// Values safe for logging and audit.
    pub fn redacted(&self) -> &FieldValues;
    /// Consume and return the execution-ready values.
    pub fn into_values(self) -> FieldValues;
}

/// Configuration for the preparation pipeline.
pub struct PrepareConfig<'a> {
    pub profile: ValidationProfile,
    pub expression_resolver: Option<&'a dyn ExpressionResolver>,
    pub expression_context: Option<&'a serde_json::Value>,
}

impl Schema {
    /// Full resolution pipeline: normalize → resolve expressions →
    /// coerce types → validate → redact secrets.
    pub fn prepare(
        &self,
        values: &FieldValues,
        config: &PrepareConfig<'_>,
    ) -> Result<PreparedValues, Vec<ParameterError>>;
}
```

### 3.4 ValidatedValues as a type-state wrapper

```rust
/// Marker for values that passed validation.
#[derive(Debug, Clone)]
pub struct ValidatedValues {
    values: FieldValues,
    profile: ValidationProfile,
}

impl ValidatedValues {
    /// Only constructible through `Schema::validate()`.
    pub(crate) fn new(values: FieldValues, profile: ValidationProfile) -> Self;

    pub fn raw(&self) -> &FieldValues;
    pub fn into_inner(self) -> FieldValues;
    pub fn profile(&self) -> ValidationProfile;
}
```

---

## Phase 4 — Metadata & Default Value Enhancements

**Priority**: Medium  
**Breaking**: Yes (metadata field type changes)  
**Depends on**: Phase 1 (ExpressionMode)  
**Crate impact**: `nebula-parameter`

### 4.1 No new field types (DRY / KISS)

The existing 16 field variants already cover the patterns found in competitors:

| Competitor concept | Nebula equivalent | Rationale |
|--------------------|-------------------|----------|
| n8n `json` | `Field::Code { language: "json" }` | Same editing UX, just a language hint |
| n8n `resourceLocator` | `Field::Mode` with text/list variants | Mode already models multi-variant selection |
| n8n `assignmentCollection` | `Field::List` with `Object` items | List of `{key,value}` objects |
| n8n `credentialsSelect` | Not a field — credential binding is a schema-level concern | Avoid coupling parameter types to credential internals |

Adding parallel types would violate **DRY** (duplicate behavior) and **KISS** (more
variants = more match arms everywhere).

### 4.2 FieldMetadata — only `expression_mode` change

```rust
pub struct FieldMetadata {
    // ... all existing fields unchanged ...

    /// Expression evaluation mode for this field.
    /// Replaces `expression: bool`.
    pub expression_mode: ExpressionMode,
}
```

**NOT added** (by design):

- ~~`width: FieldWidth`~~ — Layout is the UI renderer's concern (KISS, SRP).
  The parameter crate defines *what* data is needed, not *how* to arrange it on screen.
- ~~`order: Option<i32>`~~ — Field order is already implicit in `Vec<Field>` position.
- ~~`deprecated: Option<String>`~~ — Needs deeper analysis: deprecation lifecycle,
  migration tooling, version tracking. Deferred to a focused RFC.
- ~~`ai_description: Option<String>`~~ — Needs deeper analysis: semantic annotation
  system for AI consumers, embedding strategies, structured vs. free-text.
  Deferred to a focused RFC after AI integration patterns are clearer.

### 4.3 DefaultValue enum

```rust
/// Default value strategies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DefaultValue {
    /// A static JSON value.
    Static(serde_json::Value),
    /// An expression evaluated at form-load time.
    Expression(String),
}
```

Replaces `default: Option<serde_json::Value>` in `FieldMetadata`.

**Use cases for `Expression`**: Pre-populating fields with dynamic system
defaults evaluated at form-load time — e.g. `{{ $date.now }}` for a timestamp
field, `{{ $workflow.name }}` for a label field. The expression is evaluated once
when the form opens, not on every change. If the user later edits the field,
their literal value takes precedence.

**Migration**: `default: Some(v)` → `default: Some(DefaultValue::Static(v))`.

---

## Phase 5 — Cross-Crate Integration Contracts

**Priority**: Medium  
**Breaking**: Varies  
**Crate impact**: All five dependent crates

### 5.1 nebula-credential integration

Credential types have their own `Schema` that describes the form fields a user
must fill in (API key, OAuth tokens, connection strings, etc.). This is already
the case: `CredentialDescription.properties: Schema`.

Credential binding to actions is a **schema-level concern** — the parameter crate
never knows about credential semantics, it only sees opaque `serde_json::Value`.

```rust
// CredentialDescription already uses Schema for its form:
pub struct CredentialDescription {
    pub credential_type: String,
    pub display_name: String,
    /// Schema defining the credential form fields.
    pub properties: Schema,
    // ...
}

// Credential resolution happens BEFORE parameter preparation:
//   1. Action declares required credential types (on ActionMetadata).
//   2. CredentialManager resolves credentials → serde_json::Value.
//   3. Resolved values are merged into raw FieldValues.
//   4. Schema::prepare(merged_values, config) runs the full pipeline.
```

### 5.2 nebula-action integration

```rust
// ActionMetadata already holds `parameters: Schema`.
// No routing declarations — HTTP routing is handled by nebula-resource.
pub struct ActionMetadata {
    // ... existing ...
    pub parameters: Schema,
}

// The action crate re-exports Schema/Field for convenience —
// no additional parameter-specific types needed.
```

### 5.3 nebula-expression integration

```rust
// nebula-expression implements ExpressionResolver:
impl ExpressionResolver for ExpressionEngine {
    fn resolve(
        &self,
        expression: &str,
        context: &serde_json::Value,
    ) -> Result<serde_json::Value, ExpressionResolveError> {
        // Delegate to existing engine
    }

    fn validate_syntax(&self, expression: &str) -> Result<(), ExpressionResolveError> {
        // Parse without evaluation
    }
}
```

### 5.4 nebula-runtime integration

```rust
// In nebula-runtime, the ActionRuntime uses Schema::prepare():
impl ActionRuntime {
    async fn execute_action(
        &self,
        action: &dyn Action,
        raw_values: FieldValues,
        ctx: &ExecutionContext,
    ) -> Result<ActionOutput> {
        let schema = action.metadata().parameters;
        let prepared = schema.prepare(&raw_values, &PrepareConfig {
            profile: ValidationProfile::Strict,
            expression_resolver: Some(&ctx.expression_engine),
            expression_context: Some(&ctx.execution_data),
        })?;

        // Log with redacted values
        tracing::info!(params = ?prepared.redacted(), "executing action");

        // Execute with resolved values
        action.execute(prepared.into_values(), ctx).await
    }
}
```

---

## Phase 6 — JSON Schema Interop via `schemars`

**Priority**: Low  
**Breaking**: No (additive, feature-gated)  
**Depends on**: Phase 4  
**Crate impact**: `nebula-parameter`

### 6.1 Why `schemars`, not hand-rolled

`schemars` v1 is already in the workspace (`nebula-credential` uses it for K8s CRDs).
Deriving `JsonSchema` on our types is **DRY**: the schema definition IS the Rust type,
not a separate hand-maintained function.

```toml
# Cargo.toml
[dependencies]
schemars = { workspace = true, optional = true }

[features]
json-schema = ["schemars"]
```

### 6.2 Derive on core types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub struct FieldMetadata { /* ... */ }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum Field { /* ... */ }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum Condition { /* ... */ }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "json-schema", derive(schemars::JsonSchema))]
pub enum Rule { /* ... */ }
```

Non-serializable types (`OptionLoader`, `RecordLoader`, `ComposedRule`) are
skipped via `#[schemars(skip)]` or excluded from the derive.

### 6.3 Schema generation

```rust
#[cfg(feature = "json-schema")]
impl Schema {
    /// Generate a JSON Schema (draft 2020-12) for the Schema type itself.
    ///
    /// This describes the *shape of a parameter schema definition*, not
    /// the shape of the values it validates.
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(Schema))
            .expect("schema serialization cannot fail")
    }
}
```

### 6.4 Value schema generation

For generating a JSON Schema that describes the *values* a given parameter
Schema expects (i.e., the shape of `FieldValues`), a separate method is
provided. This one IS hand-rolled because it depends on the field definitions:

```rust
impl Schema {
    /// Generate a JSON Schema describing the valid shape of `FieldValues`
    /// for this particular schema instance.
    pub fn values_json_schema(&self) -> serde_json::Value;
}
```

This maps each field to its JSON Schema equivalent:
- `Field::Text` → `{ "type": "string" }`
- `Field::Number { integer: true }` → `{ "type": "integer" }`
- `Field::Boolean` → `{ "type": "boolean" }`
- `Field::Object { fields }` → recursive `{ "type": "object", "properties": { ... } }`
- etc.

---

## Implementation Order

```
Phase 1.1 (ExpressionMode)           ──┐
Phase 4.2 (FieldMetadata changes)     ──┤── v2.1 (metadata breaking changes)
Phase 4.3 (DefaultValue enum)         ──┘

Phase 1.2-1.4 (ExpressionResolver)   ──┐
Phase 2.1-2.4 (Validation unification)─┤── v2.2 (resolution + validation)
Phase 3.1-3.2 (Coercion + Redaction) ──┘

Phase 3.3-3.4 (PreparedValues)       ──┐
Phase 5 (Cross-crate contracts)       ──┤── v3.0 (full integration)
                                       ┘

Phase 6 (schemars JSON Schema)       ──── v3.1 (interop, feature-gated)
```

---

## Design Principles

1. **SOLID — Single Responsibility** — The parameter crate defines schemas and orchestrates
   the resolution pipeline. Validation logic lives in `nebula-validator`. Expression
   evaluation lives in `nebula-expression`. Each crate has one job.

2. **DRY — Don't Repeat Yourself** — No new field types that duplicate existing ones.
   `Code` covers JSON editing, `Mode` covers multi-variant resource location, `List`
   covers key-value collections. Rules bridge to `nebula-validator` instead of
   re-implementing validators.

3. **KISS — Keep It Simple** — No UI layout hints in the data model (`FieldWidth`, `order`,
   grid positioning). The schema describes *what* data is needed. 16 field variants is the
   ceiling — new "types" are expressed through existing variants with different configuration.

4. **Serializable core** — `Schema`, `Field`, `Rule`, `Condition` must all
   round-trip through `serde_json`. Loaders (`OptionLoader`, `RecordLoader`) are
   intentionally non-serializable runtime-only closures.

5. **Runtime traits at the boundary (DIP)** — `ExpressionResolver` is a
   trait object injected at runtime. The parameter crate defines the trait, consumers
   implement it. No concrete dependencies upward.

6. **Progressive complexity** — A simple schema (`Field::text("name").required()`) requires
   no imports beyond `Field` and `Schema`. Expression resolution, cross-field validation,
   and provider protocols are opt-in.

7. **Type-state guarantees** — `FieldValues` → `ValidatedValues` → `PreparedValues` encode
   processing state at the type level. You can't pass unvalidated values where validated
   ones are required.

8. **Credential decoupling** — The parameter crate references credentials as opaque
   `serde_json::Value`. No credential-specific field types — credential binding is a
   schema-level or action-level concern, not a field type concern.

---

## Migration Impact

### Breaking changes by crate:

| Crate | Breaking change | Migration |
|-------|----------------|-----------|
| `nebula-parameter` | `expression: bool` → `expression_mode: ExpressionMode` | Mechanical rename |
| `nebula-parameter` | `default: Option<Value>` → `default: Option<DefaultValue>` | Wrap in `DefaultValue::Static()` |
| `nebula-parameter` | New `Rule` variants | Non-exhaustive, additive |
| `nebula-parameter` | New `ParameterError` variants | Non-exhaustive, additive |
| `nebula-credential` | `CredentialDescription.properties` type change | Adapt to new metadata fields |
| `nebula-action` | `ActionMetadata.parameters` type change | Adapt to new Schema methods |
| `nebula-runtime` | Uses `Schema::prepare()` pipeline | New integration point |
| `nebula-expression` | Implements `ExpressionResolver` trait | New trait impl |
### Deferred to future RFCs:

| Topic | Reason |
|-------|--------|
| `deprecated` field flag | Needs lifecycle design: version tracking, migration tooling, sunset policy |
| AI/semantic annotations | Needs analysis of AI integration patterns, embedding strategies, structured vs. free-text |
| New field types beyond 16 | Current variants cover all identified patterns (DRY) |
| UI layout hints | Out of scope — renderer's concern (SRP, KISS) |