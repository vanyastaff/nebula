# nebula-parameter v2 → v3 Migration Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rewrite nebula-parameter from v2 (Field enum + FieldMetadata + Schema) to v3 (Parameter struct + ParameterType enum + ParameterCollection) per HLD v3.

**Architecture:** Big-bang rewrite of the parameter crate, then update all 6 consumers. The core change is Field enum (16 variants each carrying flattened FieldMetadata) → Parameter struct (shared fields inline) + ParameterType enum (19 variants, type-specific data only). Schema → ParameterCollection. FieldValues → ParameterValues. Condition becomes its own enum separate from Rule. New types: Transformer, DisplayMode, Computed, Notice, FilterField, FilterFieldLoader, LoaderResult, ParameterPath.

**Tech Stack:** Rust 1.93, serde/serde_json, thiserror, regex, nebula-validator

**Rename map (v2 → v3):**
| v2 | v3 |
|---|---|
| `Field` enum | `Parameter` struct + `ParameterType` enum |
| `FieldMetadata` struct | fields inline on `Parameter` |
| `Schema` | `ParameterCollection` |
| `FieldValues` | `ParameterValues` |
| `FieldValue` | `ParameterValue` |
| `FieldValuesSnapshot` | `ParameterValuesSnapshot` |
| `FieldValuesDiff` | `ParameterValuesDiff` |
| `ValidatedValues` (wraps FieldValues) | `ValidatedValues` (wraps ParameterValues) |
| `ModeValueRef` | `ModeValueRef` (unchanged) |
| `Condition` (alias for `Rule`) | `Condition` (own enum) |
| `FieldSpec` (4-variant subset) | `FieldSpec` (updated to use Parameter) |
| `ModeVariant` { key, label, content } | removed — variants are `Vec<Parameter>` |
| `UiElement::Notice` | `ParameterType::Notice` |
| `UiElement::Button` | removed (not in v3) |
| `Schema.ui` / `Schema.groups` | removed (groups via `Parameter.group` + Sections display mode) |
| `Group` | removed |
| `Severity` | `NoticeSeverity` |
| `OptionSource` enum | flattened into Select variant fields |
| `DynamicFieldsMode` | removed |
| `UnknownFieldPolicy` | removed (handled by ValidationProfile) |
| `LoaderCtx` | `LoaderContext` |
| `LoaderFuture<T>` | inline in loader types |

**Consumer impact:**
| Crate | Types used | Migration effort |
|---|---|---|
| nebula-action | `Field`, `Schema` → `Parameter`, `ParameterCollection` | Low — rename imports + re-exports |
| nebula-credential | `Field`, `Schema`, `FieldValues` → `Parameter`, `ParameterCollection`, `ParameterValues` | Medium — 6 protocol files + traits |
| nebula-sdk | re-exports all | Low — update re-exports |
| nebula-macros | generates `Field`, `Schema` code | Medium — update codegen paths |
| nebula-resource | dev-dep, Schema in tests | Trivial |
| nebula-auth | Schema | Trivial |

---

## Phase 1: Core Types (Tasks 1–7)

Foundation types that everything else depends on.

### Task 1: ParameterPath + NoticeSeverity + DisplayMode + ComputedReturn

**Files:**
- Create: `crates/parameter/src/path.rs`
- Create: `crates/parameter/src/notice.rs`
- Create: `crates/parameter/src/display_mode.rs`

**Step 1: Create `path.rs`**

```rust
//! Typed reference to a parameter within a schema.

use serde::{Deserialize, Serialize};

/// Typed reference to a parameter within a schema.
///
/// Supports sibling references (`"field_name"`), nested paths (`"obj.field"`),
/// and absolute root references (`"$root.field"`).
///
/// # Examples
///
/// ```
/// use nebula_parameter::path::ParameterPath;
///
/// let sibling = ParameterPath::sibling("email");
/// assert!(!sibling.is_absolute());
///
/// let root = ParameterPath::root("auth_mode");
/// assert!(root.is_absolute());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParameterPath(String);

impl ParameterPath {
    /// Reference a sibling parameter in the same scope.
    #[must_use]
    pub fn sibling(id: &str) -> Self {
        Self(id.to_owned())
    }

    /// Reference a nested parameter via dot-separated path.
    #[must_use]
    pub fn nested(path: &str) -> Self {
        Self(path.to_owned())
    }

    /// Absolute reference from the root collection.
    #[must_use]
    pub fn root(id: &str) -> Self {
        Self(format!("$root.{id}"))
    }

    /// Whether this is an absolute root reference.
    #[must_use]
    pub fn is_absolute(&self) -> bool {
        self.0.starts_with("$root.")
    }

    /// Split the path into segments.
    #[must_use]
    pub fn segments(&self) -> Vec<&str> {
        let s = self.0.strip_prefix("$root.").unwrap_or(&self.0);
        s.split('.').collect()
    }

    /// The raw path string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ParameterPath {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl std::fmt::Display for ParameterPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
```

**Step 2: Create `notice.rs`**

```rust
//! Notice severity for display-only parameter blocks.

use serde::{Deserialize, Serialize};

/// Severity level for a [`ParameterType::Notice`](crate::parameter_type::ParameterType::Notice).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeSeverity {
    /// Informational.
    #[default]
    Info,
    /// Warning.
    Warning,
    /// Success.
    Success,
    /// Danger / error.
    Danger,
}

impl NoticeSeverity {
    /// Returns `true` for the default variant ([`Info`](Self::Info)).
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Info)
    }
}
```

**Step 3: Create `display_mode.rs`**

```rust
//! Object display mode controlling UI presentation and normalization behavior.

use serde::{Deserialize, Serialize};

/// Controls how an Object parameter renders its sub-parameters.
///
/// Affects both UI presentation and normalization behavior:
/// - `Inline` / `Collapsed`: all sub-parameter defaults are backfilled.
/// - `PickFields` / `Sections`: only explicitly added fields appear in values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    /// All sub-parameters rendered inline, always visible.
    #[default]
    Inline,
    /// Collapsible section with expand/collapse toggle.
    Collapsed,
    /// "Add Field" dropdown. Only added fields present in values.
    PickFields,
    /// Like PickFields, but dropdown grouped by `Parameter.group`.
    Sections,
}

impl DisplayMode {
    /// Whether this is the default display mode.
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Inline)
    }

    /// Whether this mode uses pick-style field selection.
    #[must_use]
    pub fn is_pick_mode(&self) -> bool {
        matches!(self, Self::PickFields | Self::Sections)
    }
}

/// Return type for computed parameter fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputedReturn {
    /// Computed field returns a string.
    String,
    /// Computed field returns a number.
    Number,
    /// Computed field returns a boolean.
    Boolean,
}
```

**Step 4: Run `cargo check -p nebula-parameter`** (will fail — modules not wired yet, that's ok)

**Step 5: Commit**
```
feat(parameter): add ParameterPath, NoticeSeverity, DisplayMode, ComputedReturn types
```

---

### Task 2: Condition enum (separate from Rule)

**Files:**
- Rewrite: `crates/parameter/src/conditions.rs`

**Step 1: Rewrite `conditions.rs` with the new Condition enum**

```rust
//! Declarative conditions for field visibility and required logic.
//!
//! `Condition` is a predicate on sibling field values — it controls
//! when a field is visible, required, or disabled. It is distinct from
//! `Rule`, which validates a field's own value.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::path::ParameterPath;

/// A predicate over sibling parameter values.
///
/// Used for `visible_when`, `required_when`, and `disabled_when`.
///
/// # Examples
///
/// ```
/// use nebula_parameter::conditions::Condition;
/// use serde_json::json;
///
/// let cond = Condition::eq("auth_mode", json!("bearer"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Condition {
    /// Field value equals the given value.
    Eq {
        /// Reference to the field to check.
        field: ParameterPath,
        /// Expected value.
        value: serde_json::Value,
    },
    /// Field value does not equal the given value.
    Ne {
        /// Reference to the field to check.
        field: ParameterPath,
        /// Value that should not match.
        value: serde_json::Value,
    },
    /// Field value is one of the given values.
    OneOf {
        /// Reference to the field to check.
        field: ParameterPath,
        /// Accepted values.
        values: Vec<serde_json::Value>,
    },
    /// Field has a non-null, non-empty value.
    Set {
        /// Reference to the field to check.
        field: ParameterPath,
    },
    /// Field is null, empty, or absent.
    NotSet {
        /// Reference to the field to check.
        field: ParameterPath,
    },
    /// Field value is boolean `true`.
    IsTrue {
        /// Reference to the field to check.
        field: ParameterPath,
    },
    /// Field value is greater than the given value.
    Gt {
        /// Reference to the field to check.
        field: ParameterPath,
        /// Threshold.
        value: serde_json::Value,
    },
    /// Field value is less than the given value.
    Lt {
        /// Reference to the field to check.
        field: ParameterPath,
        /// Threshold.
        value: serde_json::Value,
    },
    /// All sub-conditions must be true.
    All {
        /// Sub-conditions.
        conditions: Vec<Condition>,
    },
    /// At least one sub-condition must be true.
    Any {
        /// Sub-conditions.
        conditions: Vec<Condition>,
    },
    /// Negates a sub-condition.
    Not {
        /// Sub-condition.
        condition: Box<Condition>,
    },
}

impl Condition {
    /// Shorthand for `Condition::Eq`.
    #[must_use]
    pub fn eq(field: impl Into<ParameterPath>, value: serde_json::Value) -> Self {
        Self::Eq { field: field.into(), value }
    }

    /// Shorthand for `Condition::Ne`.
    #[must_use]
    pub fn ne(field: impl Into<ParameterPath>, value: serde_json::Value) -> Self {
        Self::Ne { field: field.into(), value }
    }

    /// Shorthand for `Condition::OneOf`.
    #[must_use]
    pub fn one_of(field: impl Into<ParameterPath>, values: Vec<serde_json::Value>) -> Self {
        Self::OneOf { field: field.into(), values }
    }

    /// Shorthand for `Condition::Set`.
    #[must_use]
    pub fn set(field: impl Into<ParameterPath>) -> Self {
        Self::Set { field: field.into() }
    }

    /// Shorthand for `Condition::NotSet`.
    #[must_use]
    pub fn not_set(field: impl Into<ParameterPath>) -> Self {
        Self::NotSet { field: field.into() }
    }

    /// Shorthand for `Condition::All`.
    #[must_use]
    pub fn all(conditions: Vec<Condition>) -> Self {
        Self::All { conditions }
    }

    /// Shorthand for `Condition::Any`.
    #[must_use]
    pub fn any(conditions: Vec<Condition>) -> Self {
        Self::Any { conditions }
    }

    /// Shorthand for `Condition::Not`.
    #[must_use]
    pub fn not(condition: Condition) -> Self {
        Self::Not { condition: Box::new(condition) }
    }

    /// Evaluate this condition against a values map.
    #[must_use]
    pub fn evaluate(&self, values: &HashMap<String, serde_json::Value>) -> bool {
        match self {
            Self::Eq { field, value } => {
                values.get(field.as_str()).is_some_and(|v| v == value)
            }
            Self::Ne { field, value } => {
                values.get(field.as_str()).is_none_or(|v| v != value)
            }
            Self::OneOf { field, values: expected } => {
                values.get(field.as_str()).is_some_and(|v| expected.contains(v))
            }
            Self::Set { field } => {
                values.get(field.as_str()).is_some_and(|v| !v.is_null())
            }
            Self::NotSet { field } => {
                values.get(field.as_str()).is_none_or(serde_json::Value::is_null)
            }
            Self::IsTrue { field } => {
                values.get(field.as_str()).is_some_and(|v| v.as_bool() == Some(true))
            }
            Self::Gt { field, value } => {
                values.get(field.as_str()).is_some_and(|v| {
                    v.as_f64().zip(value.as_f64()).is_some_and(|(a, b)| a > b)
                })
            }
            Self::Lt { field, value } => {
                values.get(field.as_str()).is_some_and(|v| {
                    v.as_f64().zip(value.as_f64()).is_some_and(|(a, b)| a < b)
                })
            }
            Self::All { conditions } => conditions.iter().all(|c| c.evaluate(values)),
            Self::Any { conditions } => conditions.iter().any(|c| c.evaluate(values)),
            Self::Not { condition } => !condition.evaluate(values),
        }
    }

    /// Collect all field references in this condition.
    pub fn field_references<'a>(&'a self, refs: &mut Vec<&'a str>) {
        match self {
            Self::Eq { field, .. }
            | Self::Ne { field, .. }
            | Self::OneOf { field, .. }
            | Self::Set { field }
            | Self::NotSet { field }
            | Self::IsTrue { field }
            | Self::Gt { field, .. }
            | Self::Lt { field, .. } => refs.push(field.as_str()),
            Self::All { conditions } | Self::Any { conditions } => {
                for c in conditions {
                    c.field_references(refs);
                }
            }
            Self::Not { condition } => condition.field_references(refs),
        }
    }
}
```

**Step 2: Commit**
```
feat(parameter): rewrite Condition as independent enum with ParameterPath
```

---

### Task 3: Transformer enum

**Files:**
- Create: `crates/parameter/src/transformer.rs`

**Step 1: Create `transformer.rs`**

```rust
//! Declarative value transformation pipeline.
//!
//! Transformers are applied lazily when action code reads values via
//! `get_transformed()`. They do NOT affect validation or normalization.

use serde::{Deserialize, Serialize};

fn default_group() -> usize { 1 }

/// A declarative value transform.
///
/// Applied in order via `ParameterValues::get_transformed()`.
/// If a transformer fails to match (e.g. regex with no capture), the value
/// passes through unchanged.
///
/// # Examples
///
/// ```
/// use nebula_parameter::transformer::Transformer;
///
/// let chain = Transformer::Chain {
///     transformers: vec![Transformer::Trim, Transformer::Lowercase],
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transformer {
    /// Remove leading/trailing whitespace.
    Trim,
    /// Convert to lowercase.
    Lowercase,
    /// Convert to uppercase.
    Uppercase,
    /// Replace all occurrences of `from` with `to`.
    Replace {
        /// Substring to find.
        from: String,
        /// Replacement.
        to: String,
    },
    /// Remove a prefix if present.
    StripPrefix {
        /// Prefix to strip.
        prefix: String,
    },
    /// Remove a suffix if present.
    StripSuffix {
        /// Suffix to strip.
        suffix: String,
    },
    /// Extract a regex capture group. No match → pass-through.
    Regex {
        /// Regex pattern.
        pattern: String,
        /// Capture group index (default: 1).
        #[serde(default = "default_group")]
        group: usize,
    },
    /// Extract value at a JSON dot-path.
    JsonPath {
        /// Dot-separated path.
        path: String,
    },
    /// Apply transformers in sequence.
    Chain {
        /// Ordered transformer pipeline.
        transformers: Vec<Transformer>,
    },
    /// First transformer that changes the value wins.
    FirstMatch {
        /// Candidate transformers.
        transformers: Vec<Transformer>,
    },
}

impl Transformer {
    /// Apply this transformer to a JSON value.
    ///
    /// Returns the transformed value. Non-string values pass through
    /// string-only transformers unchanged.
    #[must_use]
    pub fn apply(&self, value: &serde_json::Value) -> serde_json::Value {
        match self {
            Self::Trim => match value.as_str() {
                Some(s) => serde_json::Value::String(s.trim().to_owned()),
                None => value.clone(),
            },
            Self::Lowercase => match value.as_str() {
                Some(s) => serde_json::Value::String(s.to_lowercase()),
                None => value.clone(),
            },
            Self::Uppercase => match value.as_str() {
                Some(s) => serde_json::Value::String(s.to_uppercase()),
                None => value.clone(),
            },
            Self::Replace { from, to } => match value.as_str() {
                Some(s) => serde_json::Value::String(s.replace(from.as_str(), to.as_str())),
                None => value.clone(),
            },
            Self::StripPrefix { prefix } => match value.as_str() {
                Some(s) => serde_json::Value::String(
                    s.strip_prefix(prefix.as_str()).unwrap_or(s).to_owned(),
                ),
                None => value.clone(),
            },
            Self::StripSuffix { suffix } => match value.as_str() {
                Some(s) => serde_json::Value::String(
                    s.strip_suffix(suffix.as_str()).unwrap_or(s).to_owned(),
                ),
                None => value.clone(),
            },
            Self::Regex { pattern, group } => match value.as_str() {
                Some(s) => {
                    let Ok(re) = regex::Regex::new(pattern) else {
                        return value.clone();
                    };
                    match re.captures(s).and_then(|c| c.get(*group)) {
                        Some(m) => serde_json::Value::String(m.as_str().to_owned()),
                        None => value.clone(),
                    }
                }
                None => value.clone(),
            },
            Self::JsonPath { path } => {
                let mut current = value;
                for segment in path.split('.') {
                    match current.get(segment) {
                        Some(next) => current = next,
                        None => return value.clone(),
                    }
                }
                current.clone()
            }
            Self::Chain { transformers } => {
                let mut result = value.clone();
                for t in transformers {
                    result = t.apply(&result);
                }
                result
            }
            Self::FirstMatch { transformers } => {
                for t in transformers {
                    let result = t.apply(value);
                    if result != *value {
                        return result;
                    }
                }
                value.clone()
            }
        }
    }
}
```

**Step 2: Commit**
```
feat(parameter): add Transformer enum with apply() logic
```

---

### Task 4: SelectOption (add icon) + FilterField + FilterFieldType

**Files:**
- Rewrite: `crates/parameter/src/option.rs`
- Create: `crates/parameter/src/filter_field.rs`

**Step 1: Update `option.rs`** — remove `OptionSource`, add `icon` to `SelectOption`

```rust
//! Option models for select-like parameters.

use serde::{Deserialize, Serialize};

/// A single option in a select or multi-select parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    /// The value produced when this option is selected.
    pub value: serde_json::Value,
    /// Human-readable display label.
    pub label: String,
    /// Optional tooltip or help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this option is shown but not selectable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    /// Icon key, URI, or emoji. Frontend-only rendering hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl SelectOption {
    /// Creates a new enabled option.
    #[must_use]
    pub fn new(value: serde_json::Value, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            description: None,
            disabled: false,
            icon: None,
        }
    }

    /// Sets the description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the icon.
    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Marks the option as disabled.
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}
```

**Step 2: Create `filter_field.rs`**

```rust
//! Typed field definitions for the Filter condition builder.

use crate::option::SelectOption;
use serde::{Deserialize, Serialize};

/// Describes a filterable field for the Filter condition builder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterField {
    /// Field identifier.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Data type — determines applicable operators and value input widget.
    #[serde(default, skip_serializing_if = "FilterFieldType::is_default")]
    pub field_type: FilterFieldType,
}

/// Data type of a filterable field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterFieldType {
    /// Free-form text.
    #[default]
    String,
    /// Numeric value.
    Number,
    /// Boolean.
    Boolean,
    /// Calendar date.
    Date,
    /// Date and time.
    DateTime,
    /// Enum with predefined options.
    Enum {
        /// Available options.
        options: Vec<SelectOption>,
    },
}

impl FilterFieldType {
    /// Returns `true` for the default variant.
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::String)
    }
}
```

**Step 3: Commit**
```
feat(parameter): update SelectOption with icon, add FilterField/FilterFieldType
```

---

### Task 5: LoaderResult + updated loaders (OptionLoader, RecordLoader, FilterFieldLoader)

**Files:**
- Create: `crates/parameter/src/loader_result.rs`
- Rewrite: `crates/parameter/src/loader.rs`

**Step 1: Create `loader_result.rs`**

```rust
//! Paginated result type for async loaders.

use serde::{Deserialize, Serialize};

/// Paginated result from any loader.
///
/// # Examples
///
/// ```
/// use nebula_parameter::loader_result::LoaderResult;
///
/// let result = LoaderResult::done(vec!["a", "b", "c"]);
/// assert!(!result.has_more());
/// assert_eq!(result.items.len(), 3);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoaderResult<T> {
    /// Items in this page.
    pub items: Vec<T>,
    /// Cursor for the next page, if more results are available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Total number of items across all pages, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

impl<T> LoaderResult<T> {
    /// Create a complete (non-paginated) result.
    #[must_use]
    pub fn done(items: Vec<T>) -> Self {
        Self { items, next_cursor: None, total: None }
    }

    /// Create a paginated result with a cursor for the next page.
    #[must_use]
    pub fn page(items: Vec<T>, next_cursor: impl Into<String>) -> Self {
        Self { items, next_cursor: Some(next_cursor.into()), total: None }
    }

    /// Set the total count.
    #[must_use]
    pub fn with_total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }

    /// Whether more pages are available.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some()
    }
}

impl<T> From<Vec<T>> for LoaderResult<T> {
    fn from(items: Vec<T>) -> Self {
        Self::done(items)
    }
}
```

**Step 2: Rewrite `loader.rs`** — rename LoaderCtx → LoaderContext, return LoaderResult, add FilterFieldLoader

```rust
//! Async loader types for dynamic parameter resolution.
//!
//! Loaders are closure-based async functions attached to parameters.
//! They are NOT serialized — they exist only in-process.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::filter_field::FilterField;
use crate::loader_result::LoaderResult;
use crate::option::SelectOption;

// NOTE: ParameterValues and Parameter are forward-declared here.
// They will be defined in later tasks. For now these use FieldValues/FieldSpec
// as a temporary compilation placeholder.
// After Task 6+7, replace with the real types.

/// Context passed to loader functions when the UI requests dynamic data.
#[derive(Debug, Clone)]
pub struct LoaderContext {
    /// The id of the parameter requesting a load.
    pub parameter_id: String,
    /// Current parameter values at the time of the request.
    pub values: crate::values::ParameterValues,
    /// Optional text filter entered by the user (for searchable selects).
    pub filter: Option<String>,
    /// Pagination cursor returned from a previous load.
    pub cursor: Option<String>,
    /// Resolved credential value, engine-populated.
    pub credential: Option<serde_json::Value>,
    /// Additional metadata from the runtime.
    pub metadata: Option<serde_json::Value>,
}

/// Error returned by a loader when it cannot resolve data.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct LoaderError {
    /// Human-readable description of the failure.
    pub message: String,
    /// Optional underlying cause.
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl LoaderError {
    /// Creates a loader error with a message.
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), source: None }
    }

    /// Creates a loader error wrapping a source error.
    pub fn with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self { message: message.into(), source: Some(Box::new(source)) }
    }
}

// ── OptionLoader ──

/// Async loader that resolves [`SelectOption`]s for select parameters.
pub struct OptionLoader(
    Arc<dyn Fn(LoaderContext) -> Pin<Box<dyn Future<Output = Result<LoaderResult<SelectOption>, LoaderError>> + Send>> + Send + Sync>,
);

impl OptionLoader {
    /// Wraps an async closure as an [`OptionLoader`].
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<SelectOption>, LoaderError>> + Send + 'static,
    {
        Self(Arc::new(move |ctx| Box::pin(f(ctx))))
    }

    /// Invokes the loader.
    ///
    /// # Errors
    ///
    /// Returns [`LoaderError`] on failure.
    pub async fn call(&self, ctx: LoaderContext) -> Result<LoaderResult<SelectOption>, LoaderError> {
        (self.0)(ctx).await
    }
}

impl Clone for OptionLoader {
    fn clone(&self) -> Self { Self(Arc::clone(&self.0)) }
}

impl PartialEq for OptionLoader {
    fn eq(&self, _: &Self) -> bool { true }
}

impl std::fmt::Debug for OptionLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OptionLoader(<async fn>)")
    }
}

// ── RecordLoader ──

// NOTE: Returns Vec<Parameter> in v3. Until Parameter is defined (Task 6),
// we use a forward reference. The actual signature:
// Result<LoaderResult<crate::parameter::Parameter>, LoaderError>
// For now, use a placeholder that will be fixed in Task 6.

/// Async loader that resolves parameters for Dynamic fields.
pub struct RecordLoader(
    Arc<dyn Fn(LoaderContext) -> Pin<Box<dyn Future<Output = Result<LoaderResult<serde_json::Value>, LoaderError>> + Send>> + Send + Sync>,
);

impl RecordLoader {
    /// Wraps an async closure as a [`RecordLoader`].
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<serde_json::Value>, LoaderError>> + Send + 'static,
    {
        Self(Arc::new(move |ctx| Box::pin(f(ctx))))
    }

    /// Invokes the loader.
    ///
    /// # Errors
    ///
    /// Returns [`LoaderError`] on failure.
    pub async fn call(&self, ctx: LoaderContext) -> Result<LoaderResult<serde_json::Value>, LoaderError> {
        (self.0)(ctx).await
    }
}

impl Clone for RecordLoader {
    fn clone(&self) -> Self { Self(Arc::clone(&self.0)) }
}

impl PartialEq for RecordLoader {
    fn eq(&self, _: &Self) -> bool { true }
}

impl std::fmt::Debug for RecordLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecordLoader(<async fn>)")
    }
}

// ── FilterFieldLoader ──

/// Async loader that resolves [`FilterField`]s for Filter parameters.
pub struct FilterFieldLoader(
    Arc<dyn Fn(LoaderContext) -> Pin<Box<dyn Future<Output = Result<LoaderResult<FilterField>, LoaderError>> + Send>> + Send + Sync>,
);

impl FilterFieldLoader {
    /// Wraps an async closure as a [`FilterFieldLoader`].
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<FilterField>, LoaderError>> + Send + 'static,
    {
        Self(Arc::new(move |ctx| Box::pin(f(ctx))))
    }

    /// Invokes the loader.
    ///
    /// # Errors
    ///
    /// Returns [`LoaderError`] on failure.
    pub async fn call(&self, ctx: LoaderContext) -> Result<LoaderResult<FilterField>, LoaderError> {
        (self.0)(ctx).await
    }
}

impl Clone for FilterFieldLoader {
    fn clone(&self) -> Self { Self(Arc::clone(&self.0)) }
}

impl PartialEq for FilterFieldLoader {
    fn eq(&self, _: &Self) -> bool { true }
}

impl std::fmt::Debug for FilterFieldLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FilterFieldLoader(<async fn>)")
    }
}
```

**Step 3: Commit**
```
feat(parameter): add LoaderResult, LoaderContext, FilterFieldLoader; update loaders to return LoaderResult
```

---

### Task 6: ParameterType enum + Parameter struct + fluent builders

This is the core rewrite. Delete `field.rs` and `metadata.rs`, create `parameter.rs` and `parameter_type.rs`.

**Files:**
- Delete: `crates/parameter/src/field.rs`
- Delete: `crates/parameter/src/metadata.rs`
- Create: `crates/parameter/src/parameter_type.rs`
- Create: `crates/parameter/src/parameter.rs`

**Step 1: Create `parameter_type.rs`** — the 19-variant enum with type-specific data only

The enum should contain these variants:
`String`, `Number`, `Boolean`, `Select`, `Object`, `List`, `Mode`, `Code`, `Date`, `DateTime`, `Time`, `Color`, `File`, `Hidden`, `Filter`, `Computed`, `Dynamic`, `Notice`

Each variant carries ONLY type-specific fields (not shared metadata). Follow the HLD v3 type definitions exactly.

Key differences from v2:
- `Select` has `options: Vec<SelectOption>` directly (no `OptionSource` wrapper), `dynamic: bool`, `depends_on: Vec<ParameterPath>`
- `Object` has `parameters: Vec<Parameter>` (not `fields: Vec<Field>`), `display_mode: DisplayMode`
- `List` gains `unique: bool`, `sortable: bool`
- `Mode` variants are `Vec<Parameter>` (not `Vec<ModeVariant>`)
- `Filter` gains `fields: Vec<FilterField>`, `dynamic_fields: bool`, `depends_on`, `fields_loader`
- New: `Computed { expression, returns }`, `Dynamic { depends_on, loader }`, `Notice { severity }`

**Step 2: Create `parameter.rs`** — the Parameter struct with ALL shared fields inline + fluent builder methods

The struct has all fields from the HLD: `id`, `param_type`, `label`, `description`, `placeholder`, `hint`, `default`, `required`, `secret`, `expression`, `input_type`, `rules`, `visible_when`, `required_when`, `disabled_when`, `transformers`, `group`.

Fluent builders: `Parameter::string(id)`, `Parameter::number(id)`, `Parameter::integer(id)`, `Parameter::boolean(id)`, `Parameter::select(id)`, `Parameter::object(id)`, `Parameter::list(id)`, `Parameter::mode(id)`, `Parameter::code(id)`, `Parameter::date(id)`, `Parameter::datetime(id)`, `Parameter::time(id)`, `Parameter::color(id)`, `Parameter::file(id)`, `Parameter::hidden(id)`, `Parameter::filter(id)`, `Parameter::computed(id)`, `Parameter::dynamic(id)`, `Parameter::notice(id)`, `Parameter::warning(id)`, `Parameter::danger(id)`, `Parameter::success(id)`.

Shared builder methods (return `Self`): `.label()`, `.description()`, `.placeholder()`, `.hint()`, `.required()`, `.secret()`, `.default()`, `.with_rule()`, `.visible_when()`, `.required_when()`, `.disabled_when()`, `.active_when()`, `.input_type()`, `.group()`.

Type-specific builder methods that access `param_type` variants:
- String: `.multiline()`
- Number: `.integer()`, `.min()`, `.max()`, `.step()`
- Select: `.option()`, `.option_with()`, `.multiple()`, `.allow_custom()`, `.searchable()`, `.depends_on()`, `.loader()` (OptionLoader)
- Object: `.add()`, `.collapsed()`, `.pick_fields()`, `.sections()`
- List: `.item()`, `.min_items()`, `.max_items()`, `.unique()`, `.sortable()`
- Mode: `.variant()`, `.default_variant()`
- Code: `.language()`
- File: `.accept()`, `.max_size()`, `.multiple()` (File variant)
- Filter: `.operators()`, `.allow_groups()`, `.max_depth()`, `.field()`, `.dynamic_fields()`, `.fields_loader()`
- Computed: `.expression()`, `.returns_string()`, `.returns_number()`, `.returns_boolean()`
- Dynamic: `.depends_on()`, `.loader()` (RecordLoader)
- Transformer helpers: `.trim()`, `.lowercase()`, `.uppercase()`, `.extract_regex()`, `.transformer()`

**Step 3: Commit**
```
feat(parameter): add Parameter struct + ParameterType enum replacing Field + FieldMetadata
```

---

### Task 7: ParameterValues (rename from FieldValues) + get_transformed()

**Files:**
- Rewrite: `crates/parameter/src/values.rs`
- Rewrite: `crates/parameter/src/runtime.rs`

Rename `FieldValues` → `ParameterValues`, `FieldValue` → `ParameterValue`, `FieldValuesSnapshot` → `ParameterValuesSnapshot`, `FieldValuesDiff` → `ParameterValuesDiff`.

Add `get_transformed(key, collection) -> Option<Value>` method that:
1. Looks up the parameter definition in the collection by key
2. Gets the raw value
3. Applies each transformer in the parameter's transformer list via `Transformer::apply()`
4. Returns the transformed value

Keep all existing functionality: typed accessors, snapshot/restore, diff, mode/expression accessors.

Update `runtime.rs` — `ValidatedValues` wraps `ParameterValues` instead of `FieldValues`.

**Step: Commit**
```
feat(parameter): rename FieldValues → ParameterValues, add get_transformed()
```

---

## Phase 2: Collection + Validation + Normalization + Lint (Tasks 8–11)

### Task 8: ParameterCollection (replace Schema)

**Files:**
- Rewrite: `crates/parameter/src/schema.rs` → rename to `crates/parameter/src/collection.rs`

`ParameterCollection` has:
- `parameters: Vec<Parameter>`
- `fn new()`, `fn add(Parameter)` (builder), `fn len()`, `fn is_empty()`, `fn get(id)`, `fn contains(id)`
- `fn validate(&self, &ParameterValues) -> Result<ValidatedValues, Vec<ParameterError>>`
- `fn validate_with_profile(&self, &ParameterValues, ValidationProfile) -> ValidationReport`
- `fn normalize(&self, &ParameterValues) -> ParameterValues`

No `ui`, no `groups` — those are gone in v3.

**Step: Commit**
```
feat(parameter): add ParameterCollection replacing Schema
```

---

### Task 9: Rewrite validation engine

**Files:**
- Rewrite: `crates/parameter/src/validate.rs`

Update to work with `Parameter` + `ParameterType` instead of `Field`. Key changes:
- Use `Condition::evaluate()` instead of `Rule::evaluate()` for visible_when/required_when
- Skip `Computed` and `Notice` parameter types
- Object validation respects `DisplayMode::is_pick_mode()` — absent pick-mode keys skip entirely
- Mode validation: variants are `Vec<Parameter>`, match by `Parameter.id`
- Filter validation: check field references, operator applicability, value types
- Dynamic: skip (resolved at runtime)

**Step: Commit**
```
feat(parameter): rewrite validation engine for Parameter + ParameterType
```

---

### Task 10: Rewrite normalization

**Files:**
- Rewrite: `crates/parameter/src/normalize.rs`

Update for:
- Mode variants are `Vec<Parameter>` — find by `param.id`
- Object with PickFields/Sections: absent key → `{}`, sub-keys absent → skip defaults
- Skip `Computed`, `Notice`, `Hidden` defaults
- Recurse into Object sub-parameters, List items, Mode active variant

**Step: Commit**
```
feat(parameter): rewrite normalization for Parameter + ParameterType
```

---

### Task 11: Rewrite lint

**Files:**
- Rewrite: `crates/parameter/src/lint.rs`

Update to use `Parameter` + `ParameterType`. Add new diagnostics from v3:
- Sections Object with sub-parameters missing `group`
- `group` on parameter inside non-Sections Object
- `required` on sub-parameter of PickFields Object (warning)
- PickFields/Sections Object with ≤2 sub-parameters
- Transformer on non-string parameter
- Invalid regex in `Transformer::Regex`
- Regex capture group 0 (warning)
- Chain/FirstMatch with single transformer
- Notice with `required`, `secret`, `default`, or `rules` set
- Notice without `description`
- Filter with no static fields and no fields_loader
- Filter with duplicate field IDs

**Step: Commit**
```
feat(parameter): rewrite lint with new diagnostics for v3
```

---

## Phase 3: FieldSpec + module wiring + lib.rs (Task 12–13)

### Task 12: Update FieldSpec + filter types

**Files:**
- Rewrite: `crates/parameter/src/spec.rs`

`FieldSpec` stays as a restricted 4-variant subset but uses Parameter-style types. Keep `TryFrom<&Parameter>` and `From<FieldSpec> for Parameter` conversions.

Keep `FilterExpr`, `FilterGroup`, `FilterRule`, `FilterOp`, `FilterCombinator` — these don't change.

Remove `ModeVariant`, `DynamicFieldsMode`, `UnknownFieldPolicy`, `FieldSpecConvertError` (update to use Parameter).

**Step: Commit**
```
refactor(parameter): update FieldSpec for Parameter types, remove ModeVariant
```

---

### Task 13: Wire up lib.rs + prelude + delete old files

**Files:**
- Rewrite: `crates/parameter/src/lib.rs`
- Delete: `crates/parameter/src/field.rs` (if not already)
- Delete: `crates/parameter/src/metadata.rs` (if not already)
- Delete: `crates/parameter/src/schema.rs` (replaced by collection.rs)

Update `lib.rs` to:
- Export new modules: `parameter`, `parameter_type`, `collection`, `path`, `notice`, `display_mode`, `transformer`, `filter_field`, `loader_result`
- Remove old modules: `field`, `metadata`, `schema`
- Update top-level re-exports: `Parameter`, `ParameterType`, `ParameterCollection`, `ParameterValues`, `Condition`, etc.
- Update prelude

**Step 1: Run `cargo check -p nebula-parameter`** — fix all compile errors

**Step 2: Commit**
```
feat(parameter): wire up lib.rs for v3 types, remove v2 modules
```

---

## Phase 4: Tests (Tasks 14–17)

### Task 14: Rewrite integration tests

**Files:**
- Rewrite: `crates/parameter/tests/serde_roundtrip.rs`
- Rewrite: `crates/parameter/tests/validation.rs`
- Rewrite: `crates/parameter/tests/normalize.rs`
- Rewrite: `crates/parameter/tests/lint.rs`

Update all tests to use v3 API. The patterns change:
- `Field::text("id").with_label("L")` → `Parameter::string("id").label("L")`
- `Schema::new().field(f)` → `ParameterCollection::new().add(f)`
- `FieldValues` → `ParameterValues`
- `schema.validate(&values)` → `collection.validate(&values)`
- `ModeVariant { key, label, content }` → `Parameter::hidden("key").label("Label")` or `Parameter::string("key").label("Label")`

**Step: Run `cargo nextest run -p nebula-parameter`** — all tests pass

**Step: Commit**
```
test(parameter): rewrite integration tests for v3 API
```

---

### Task 15: Rewrite examples

**Files:**
- Rewrite: `crates/parameter/examples/telegram_node.rs`
- Rewrite: `crates/parameter/examples/core_actions.rs`
- Rewrite: `crates/parameter/examples/jsonplaceholder_provider.rs`

Update to use v3 API.

**Step: Run `cargo check --examples -p nebula-parameter`**

**Step: Commit**
```
docs(parameter): update examples for v3 API
```

---

### Task 16: Add new v3-specific tests

**Files:**
- Add tests to existing test files or create new ones

Test new functionality:
- Transformer::apply() for each variant
- get_transformed() integration
- Condition::evaluate() for each variant
- DisplayMode normalization differences (PickFields vs Inline)
- Computed/Notice skip in validation
- FilterField validation
- LoaderResult pagination
- ParameterPath construction and segments

**Step: Commit**
```
test(parameter): add tests for Transformer, Condition, DisplayMode, FilterField
```

---

### Task 17: Run full workspace check

**Step 1:** `cargo check -p nebula-parameter`
**Step 2:** `cargo nextest run -p nebula-parameter`
**Step 3:** `cargo test --doc -p nebula-parameter`
**Step 4:** `cargo clippy -p nebula-parameter -- -D warnings`

Fix any issues.

**Step: Commit**
```
chore(parameter): fix clippy warnings and doc tests
```

---

## Phase 5: Consumer Migration (Tasks 18–23)

### Task 18: Migrate nebula-action

**Files:**
- Modify: `crates/action/src/lib.rs` — change `Field, Schema` → `Parameter, ParameterCollection`
- Modify: `crates/action/src/metadata.rs` — `Schema` → `ParameterCollection`
- Modify: `crates/action/src/prelude.rs` — update re-exports

Simple rename: `Field` → `Parameter`, `Schema` → `ParameterCollection`.

**Step: Run `cargo check -p nebula-action`**
**Step: Commit**
```
refactor(action): migrate to parameter v3 API
```

---

### Task 19: Migrate nebula-credential

**Files:**
- Modify: `crates/credential/src/traits/credential.rs`
- Modify: `crates/credential/src/core/description.rs`
- Modify: `crates/credential/src/core/reference.rs` (test)
- Modify: `crates/credential/src/manager/registry.rs`
- Modify: `crates/credential/src/protocols/api_key.rs`
- Modify: `crates/credential/src/protocols/basic_auth.rs`
- Modify: `crates/credential/src/protocols/database.rs`
- Modify: `crates/credential/src/protocols/header_auth.rs`
- Modify: `crates/credential/src/protocols/oauth2/flow.rs`
- Modify: `crates/credential/src/protocols/ldap/mod.rs`
- Modify: `crates/credential/tests/manager_schema_validation.rs`
- Modify: `crates/credential/tests/manager_create.rs`
- Modify: `crates/credential/examples/credential_description.rs`

Pattern in each protocol file:
```rust
// Before:
use nebula_parameter::values::FieldValues;
use nebula_parameter::{Field, Schema};
// ... Schema::new().field(Field::text("token").with_label("Token").required().secret())

// After:
use nebula_parameter::values::ParameterValues;
use nebula_parameter::{Parameter, ParameterCollection};
// ... ParameterCollection::new().add(Parameter::string("token").label("Token").required().secret())
```

**Step: Run `cargo check -p nebula-credential`**
**Step: Commit**
```
refactor(credential): migrate to parameter v3 API
```

---

### Task 20: Migrate nebula-sdk

**Files:**
- Modify: `crates/sdk/src/lib.rs` — update re-export
- Modify: `crates/sdk/src/prelude.rs` — update prelude re-exports

**Step: Run `cargo check -p nebula-sdk`**
**Step: Commit**
```
refactor(sdk): migrate to parameter v3 API
```

---

### Task 21: Migrate nebula-macros

**Files:**
- Modify: `crates/macros/src/parameter.rs` — update generated code paths
- Modify: `crates/macros/src/credential.rs` — update generated code paths
- Modify: `crates/macros/src/types/param_attrs.rs` — `Field::text` → `Parameter::string`, `Schema` → `ParameterCollection`

Generated code changes:
```rust
// Before: ::nebula_parameter::schema::Field::text(#key)
// After:  ::nebula_parameter::parameter::Parameter::string(#key)

// Before: ::nebula_parameter::schema::Schema::new()
// After:  ::nebula_parameter::collection::ParameterCollection::new()

// Before: ::nebula_parameter::values::ParameterValues (already correct in macros!)
// After:  ::nebula_parameter::values::ParameterValues
```

**Step: Run `cargo check -p nebula-macros`**
**Step: Commit**
```
refactor(macros): migrate to parameter v3 API
```

---

### Task 22: Migrate nebula-resource + nebula-auth

**Files:**
- Modify: `crates/resource/src/handler.rs` — update test code
- Check: `crates/auth/` — update if needed

**Step: Run `cargo check -p nebula-resource -p nebula-auth`**
**Step: Commit**
```
refactor(resource,auth): migrate to parameter v3 API
```

---

### Task 23: Full workspace verification

**Step 1:** `cargo fmt`
**Step 2:** `cargo clippy --workspace -- -D warnings`
**Step 3:** `cargo nextest run --workspace`
**Step 4:** `cargo test --workspace --doc`

Fix any remaining issues.

**Step: Commit**
```
chore: fix workspace-wide issues after parameter v3 migration
```

---

## Phase 6: Cleanup (Task 24–25)

### Task 24: Update context files

**Files:**
- Modify: `.claude/crates/parameter.md` — update for v3 types and invariants
- Modify: `.claude/active-work.md` — mark parameter v3 migration as complete

**Step: Commit**
```
docs(claude): update parameter context file for v3
```

---

### Task 25: Delete stale docs

**Files:**
- Check if `docs/crates/parameter/` still exists — if so, note for deletion
- The v3 HLD in `docs/designs/` stays as reference

**Step: Commit**
```
chore(parameter): cleanup stale v1 documentation references
```

---

## Summary

| Phase | Tasks | Description |
|-------|-------|-------------|
| 1 | 1–7 | Core types: ParameterPath, Condition, Transformer, SelectOption, FilterField, LoaderResult, loaders, Parameter, ParameterType, ParameterValues |
| 2 | 8–11 | ParameterCollection, validation, normalization, lint |
| 3 | 12–13 | FieldSpec update, lib.rs wiring |
| 4 | 14–17 | Tests: rewrite existing + add new + full check |
| 5 | 18–23 | Consumer migration: action, credential, sdk, macros, resource, auth |
| 6 | 24–25 | Context files, cleanup |

**Total: 25 tasks across 6 phases.**
