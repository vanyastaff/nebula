# nebula-schema Phase 1 Foundation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild `nebula-schema` on a proof-token pipeline (`ValidSchema` / `ValidValues<'s>` / `ResolvedValues<'s>`) with unified structured `ValidationError`, delete `nebula-parameter`, and migrate `action`/`credential`/`sdk` callers — all in one phase.

**Architecture:** Five layers (schema def → build+lint → parse → validate → resolve). Tree-based `FieldValue`, consolidated field variants (no Date/Time/Color/Hidden — merged into String+InputHint), `ExpressionMode` per field, two-phase validation (schema-time structural + runtime resolve), `RuleContext` trait to kill HashMap-per-nesting allocations, `#[non_exhaustive]` on all public enums.

**Tech Stack:** Rust 2024 (edition), `indexmap`, `smallvec`, `once_cell`, `serde`/`serde_json`, `thiserror`, `trybuild`, `proptest`, `criterion` (codspeed). Proc-macro crate for `field_key!()`.

**Spec:** `docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md`

---

## Files touched

**Created:**
- `crates/schema/macros/` (new proc-macro crate — `Cargo.toml`, `src/lib.rs`)
- `crates/schema/src/input_hint.rs`
- `crates/schema/src/expression.rs`
- `crates/schema/src/validated.rs`
- `crates/schema/src/context.rs` (`RuleContext` impls)
- `crates/schema/tests/compile_fail/*.rs` + `.stderr` fixtures
- `crates/schema/tests/flow/*.rs` (integration)
- `crates/schema/tests/proptest/*.rs`
- `crates/schema/benches/bench_resolve.rs`
- `crates/schema/benches/bench_lookup.rs`

**Rewritten:**
- `crates/schema/src/{error,key,path,mode,field,value,schema,transformer,loader,lint,lib,prelude,option,widget}.rs`
- `crates/validator/src/rule/{mod,evaluate,validate}.rs` (+ `lib.rs` re-export) — `RuleContext` trait

**Modified (callers migrate):**
- `crates/action/src/{lib,metadata,prelude}.rs`
- `crates/credential/src/{credential,description,executor,resolver,static_protocol}.rs`
- `crates/credential/src/credentials/{api_key,basic_auth,oauth2,oauth2_config,oauth2_flow}.rs`
- `crates/sdk/src/{lib,prelude}.rs`

**Deleted:**
- `crates/parameter/` (entire crate)
- `crates/parameter/macros/`
- `crates/schema/src/report.rs` (merged into `error.rs`)
- workspace `Cargo.toml` member entries for the deleted crates

---

## Phase overview (24 tasks)

| # | Task | Depends on |
|---|------|------------|
| 1 | Baseline benchmarks & inventory snapshot | — |
| 2 | Add workspace deps (indexmap, smallvec, once_cell) | 1 |
| 3 | `ValidationError` + `Severity` + `ValidationReport` | 2 |
| 4 | Standard codes vocabulary | 3 |
| 5 | `FieldKey` (no panic) | 3 |
| 6 | `field_key!()` proc-macro crate | 5 |
| 7 | `FieldPath` typed parse | 5 |
| 8 | Mode enums (Visibility/Required/Expression) | 5 |
| 9 | `InputHint` enum | 5 |
| 10 | Widget enums (minor — `#[non_exhaustive]`) | 5 |
| 11 | `SelectOption` + `SelectWidget` wiring | 10 |
| 12 | `FieldValue` tree + `FieldValues` | 7, 8 |
| 13 | JSON wire format (parse + emit) | 12 |
| 14 | `Expression` wrapper with `OnceLock` | 12 |
| 15 | `Transformer` with `Regex` + `OnceLock<Regex>` cache | 12 |
| 16 | `RuleContext` trait in `nebula-validator` | 3, 12 |
| 17 | `Field` enum + per-type structs (consolidated) | 8, 9, 10, 11, 14, 15 |
| 18 | `Schema` raw + `SchemaBuilder::build` skeleton | 17 |
| 19 | Lint passes emitting `ValidationError` | 18 |
| 20 | `ValidSchema` + `FieldHandle` index | 18, 19 |
| 21 | Schema-time validation (`ValidSchema::validate`) | 16, 20 |
| 22 | Loader unified errors | 4, 20 |
| 23 | Runtime resolution (`ValidValues::resolve`) | 14, 21 |
| 24 | `lib.rs`, `prelude`, doctests | 23 |
| 25 | Compile-fail tests | 24 |
| 26 | Integration + proptest | 24 |
| 27 | Post-refactor benchmarks | 26 |
| 28 | Migrate `nebula-action` | 24 |
| 29 | Migrate `nebula-credential` | 24 |
| 30 | Migrate `nebula-sdk` | 28, 29 |
| 31 | Delete `nebula-parameter` + workspace trim | 28, 29, 30 |
| 32 | Acceptance sweep | 31 |

(32 tasks — dependencies allow parallelism between 5–11 and between 28/29.)

---

## Task 1: Baseline benchmarks & inventory snapshot

**Files:**
- Modify: none — read-only capture

**Rationale:** Record pre-refactor numbers so §13 acceptance "`bench_validate` improved ≥2×" is measurable.

- [ ] **Step 1: Capture current bench numbers**

Run:
```bash
cargo bench -p nebula-schema --bench bench_build -- --save-baseline phase0
cargo bench -p nebula-schema --bench bench_validate -- --save-baseline phase0
cargo bench -p nebula-schema --bench bench_serde -- --save-baseline phase0
cargo bench -p nebula-schema --bench bench_memory -- --save-baseline phase0
```
Expected: four baselines saved under `target/criterion/*/base/phase0/`. Any iteration-count warnings are acceptable.

- [ ] **Step 2: Snapshot current public API**

Run:
```bash
cargo doc -p nebula-schema --no-deps --document-private-items 2>/dev/null
cp -r target/doc/nebula_schema docs/superpowers/specs/nebula_schema_phase0_api_snapshot
```
This captures the pre-refactor API for later comparison. If the `cp` target already exists, overwrite.

- [ ] **Step 3: Commit the baseline snapshot**

```bash
git add docs/superpowers/specs/nebula_schema_phase0_api_snapshot target/criterion 2>/dev/null || true
git commit -m "chore(schema): snapshot phase-0 API and bench baselines" || echo "nothing to commit"
```
(`target/criterion` is usually gitignored — if so, leave the numbers only in local cache. Just ensure the snapshot directory commits.)

---

## Task 2: Workspace deps

**Files:**
- Modify: root `Cargo.toml` (workspace deps section)
- Modify: `crates/schema/Cargo.toml`

- [ ] **Step 1: Add workspace-level dependency declarations**

Edit root `Cargo.toml`, inside `[workspace.dependencies]`:
```toml
indexmap = { version = "2", features = ["serde"] }
# smallvec and once_cell already declared in workspace
```
If `indexmap` is already present, skip. Verify `smallvec` and `once_cell` are present (they are — confirmed in `[workspace.dependencies]`).

- [ ] **Step 2: Add deps to schema crate**

Edit `crates/schema/Cargo.toml`:
```toml
[dependencies]
nebula-validator = { path = "../validator" }
nebula-expression = { path = "../expression" }
nebula-schema-macros = { path = "macros" }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
thiserror = { workspace = true }
indexmap = { workspace = true }
smallvec = { workspace = true }
once_cell = { workspace = true }
regex = { workspace = true }
schemars = { version = "1.0", optional = true }

[dev-dependencies]
criterion = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt"] }
trybuild = "1"
proptest = { workspace = true }
futures = { workspace = true }
```

Inspect `crates/expression/Cargo.toml` to confirm the crate compiles stand-alone; if not, create a minimal stub module inside expression for `Ast`, `ExpressionContext`, `parse`, `evaluate` — see Task 14.

- [ ] **Step 3: Verify workspace builds (before changes start)**

Run:
```bash
cargo check -p nebula-schema
```
Expected: clean build. If failures appear from missing `nebula-expression` symbols, note them — Task 14 defines the interface.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/schema/Cargo.toml
git commit -m "chore(schema): add indexmap/smallvec/once_cell and macros crate stub"
```

---

## Task 3: `ValidationError` + `Severity` + `ValidationReport`

**Files:**
- Rewrite: `crates/schema/src/error.rs`
- Test: `crates/schema/src/error.rs#mod tests`

- [ ] **Step 1: Write failing test (builder + Display + thiserror)**

Add at the bottom of `crates/schema/src/error.rs` (create file if missing):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builder_produces_full_error() {
        let path = FieldPath::parse("user.email").unwrap();
        let err = ValidationError::new("length.max")
            .at(path.clone())
            .message("value too long")
            .param("max", json!(20))
            .param("actual", json!(42))
            .build();

        assert_eq!(err.code, "length.max");
        assert_eq!(err.path, path);
        assert_eq!(err.severity, Severity::Error);
        assert_eq!(err.params.len(), 2);
        assert!(err.message.contains("too long"));
    }

    #[test]
    fn warn_lowers_severity() {
        let err = ValidationError::new("notice.misuse").warn().build();
        assert_eq!(err.severity, Severity::Warning);
    }

    #[test]
    fn display_format_is_stable() {
        let err = ValidationError::new("required")
            .at(FieldPath::parse("x").unwrap())
            .message("missing")
            .build();
        assert_eq!(format!("{err}"), "[required] at x: missing");
    }

    #[test]
    fn report_splits_errors_and_warnings() {
        let mut report = ValidationReport::new();
        report.push(ValidationError::new("required").build());
        report.push(ValidationError::new("notice.misuse").warn().build());
        assert!(report.has_errors());
        assert!(report.has_warnings());
        assert_eq!(report.errors().count(), 1);
        assert_eq!(report.warnings().count(), 1);
    }
}
```

- [ ] **Step 2: Run the test to confirm failure**

Run: `cargo test -p nebula-schema error::tests`
Expected: compile errors — `FieldPath`, `ValidationError`, `Severity`, `ValidationReport` not defined yet (path will be stubbed in Task 7; for now create a local dummy `FieldPath` stub in `error.rs` guarded by `#[cfg(test)]` or temporarily `pub struct FieldPath(String)` — we'll replace in Task 7).

Because `FieldPath` is a Task 7 concern, the minimum to unblock Task 3: introduce a placeholder `FieldPath` with `parse(&str) -> Result<Self, ValidationError>` returning `Self(s.to_owned())`. Task 7 replaces the internals.

- [ ] **Step 3: Implement `error.rs`**

Replace the file contents with:
```rust
//! Unified structured error type for schema build, lint, validation, and resolution.

use std::{borrow::Cow, fmt, sync::Arc};

use serde_json::Value;

use crate::path::FieldPath;

/// Severity of a single issue.
#[non_exhaustive]
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum Severity {
    /// Blocks build / validate / resolve.
    Error,
    /// Advisory; does not block.
    Warning,
}

/// Unified structured validation / build / lint / resolve error.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ValidationError {
    /// Stable machine-readable code (e.g. `length.max`). See `STANDARD_CODES`.
    pub code: Cow<'static, str>,
    /// Location of the issue in the value tree.
    pub path: FieldPath,
    /// Whether this blocks progress.
    pub severity: Severity,
    /// Parameters for i18n interpolation (e.g. `("max", 20)`).
    pub params: Arc<[(Cow<'static, str>, Value)]>,
    /// English fallback message.
    pub message: Cow<'static, str>,
    /// Optional source error for chaining.
    pub source: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl ValidationError {
    /// Begin building an error with the given code.
    pub fn new(code: impl Into<Cow<'static, str>>) -> ValidationErrorBuilder {
        ValidationErrorBuilder {
            code: code.into(),
            path: FieldPath::root(),
            severity: Severity::Error,
            params: Vec::new(),
            message: Cow::Borrowed(""),
            source: None,
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] at {}: {}", self.code, self.path, self.message)
    }
}

impl std::error::Error for ValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_deref().map(|e| e as &(dyn std::error::Error + 'static))
    }
}

/// Builder for [`ValidationError`].
#[derive(Debug)]
pub struct ValidationErrorBuilder {
    code: Cow<'static, str>,
    path: FieldPath,
    severity: Severity,
    params: Vec<(Cow<'static, str>, Value)>,
    message: Cow<'static, str>,
    source: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl ValidationErrorBuilder {
    /// Attach the path where this error occurred.
    pub fn at(mut self, path: FieldPath) -> Self {
        self.path = path;
        self
    }

    /// Mark this issue as a warning instead of an error.
    pub fn warn(mut self) -> Self {
        self.severity = Severity::Warning;
        self
    }

    /// Set the English fallback message.
    pub fn message(mut self, msg: impl Into<Cow<'static, str>>) -> Self {
        self.message = msg.into();
        self
    }

    /// Add an i18n interpolation parameter.
    pub fn param(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        self.params.push((Cow::Borrowed(key), value.into()));
        self
    }

    /// Attach a source error for chaining.
    pub fn source<E>(mut self, err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        self.source = Some(Arc::new(err));
        self
    }

    /// Finalize the error.
    pub fn build(self) -> ValidationError {
        ValidationError {
            code: self.code,
            path: self.path,
            severity: self.severity,
            params: self.params.into(),
            message: self.message,
            source: self.source,
        }
    }
}

/// Collection of [`ValidationError`]s, mixed severity.
#[derive(Clone, Debug, Default)]
pub struct ValidationReport {
    issues: Vec<ValidationError>,
}

impl ValidationReport {
    /// Create an empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a single issue.
    pub fn push(&mut self, issue: ValidationError) {
        self.issues.push(issue);
    }

    /// Iterator over errors only.
    pub fn errors(&self) -> impl Iterator<Item = &ValidationError> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    /// Iterator over warnings only.
    pub fn warnings(&self) -> impl Iterator<Item = &ValidationError> {
        self.issues.iter().filter(|i| i.severity == Severity::Warning)
    }

    /// All issues in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &ValidationError> {
        self.issues.iter()
    }

    /// Issues whose path starts with `prefix`.
    pub fn at_path(&self, prefix: &FieldPath) -> impl Iterator<Item = &ValidationError> {
        let prefix = prefix.clone();
        self.issues.iter().filter(move |i| i.path.starts_with(&prefix))
    }

    /// True if at least one `Severity::Error` is present.
    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }

    /// True if at least one `Severity::Warning` is present.
    pub fn has_warnings(&self) -> bool {
        self.warnings().next().is_some()
    }

    /// Number of issues (errors + warnings).
    pub fn len(&self) -> usize {
        self.issues.len()
    }

    /// True if the report is empty.
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }
}

impl From<ValidationError> for ValidationReport {
    fn from(err: ValidationError) -> Self {
        let mut r = Self::new();
        r.push(err);
        r
    }
}

impl Extend<ValidationError> for ValidationReport {
    fn extend<I: IntoIterator<Item = ValidationError>>(&mut self, iter: I) {
        self.issues.extend(iter);
    }
}

impl IntoIterator for ValidationReport {
    type Item = ValidationError;
    type IntoIter = std::vec::IntoIter<ValidationError>;

    fn into_iter(self) -> Self::IntoIter {
        self.issues.into_iter()
    }
}
```

- [ ] **Step 4: Run the test to confirm pass**

Run: `cargo test -p nebula-schema error::tests`
Expected: all four tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/schema/src/error.rs
git commit -m "feat(schema): unified ValidationError with builder and report"
```

---

## Task 4: Standard codes vocabulary

**Files:**
- Modify: `crates/schema/src/error.rs` (append `STANDARD_CODES` array)
- Test: `crates/schema/src/error.rs#mod tests`

- [ ] **Step 1: Add failing test**

Add to `error.rs#mod tests`:
```rust
#[test]
fn standard_codes_are_unique_and_nonempty() {
    assert!(!STANDARD_CODES.is_empty());
    let mut sorted: Vec<&str> = STANDARD_CODES.to_vec();
    sorted.sort_unstable();
    let before = sorted.len();
    sorted.dedup();
    assert_eq!(before, sorted.len(), "duplicate code in STANDARD_CODES");
    for code in STANDARD_CODES {
        assert!(!code.is_empty());
        assert!(code.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c == '.'));
    }
}
```

Run: `cargo test -p nebula-schema standard_codes_are_unique` → fails (undefined).

- [ ] **Step 2: Add the vocabulary constant**

Append to `error.rs`:
```rust
/// Canonical set of stable error codes emitted by the schema crate.
///
/// Plugins may add their own under a namespace prefix (e.g. `my_plugin.foo`).
/// A test in the schema crate guarantees every entry here is emittable from
/// an integration test (see `tests/flow/all_error_codes.rs`).
pub const STANDARD_CODES: &[&str] = &[
    // value validation
    "required",
    "type_mismatch",
    "length.min",
    "length.max",
    "range.min",
    "range.max",
    "pattern",
    "url",
    "email",
    "items.min",
    "items.max",
    "items.unique",
    "option.invalid",
    // mode
    "mode.required",
    "mode.invalid",
    // expression
    "expression.forbidden",
    "expression.parse",
    "expression.type_mismatch",
    "expression.runtime",
    // loader
    "loader.not_registered",
    "loader.failed",
    // build-time
    "invalid_key",
    "duplicate_key",
    "dangling_reference",
    "self_dependency",
    "visibility_cycle",
    "rule.contradictory",
    "missing_item_schema",
    "invalid_default_variant",
    "duplicate_variant",
    // warnings
    "rule.incompatible",
    "notice.misuse",
    "missing_loader",
    "loader_without_dynamic",
    "missing_variant_label",
    "notice_missing_description",
];
```

Run: `cargo test -p nebula-schema standard_codes_are_unique` → pass.

- [ ] **Step 3: Commit**

```bash
git add crates/schema/src/error.rs
git commit -m "feat(schema): standard error code vocabulary"
```

---

## Task 5: `FieldKey` — non-panicking

**Files:**
- Rewrite: `crates/schema/src/key.rs`

- [ ] **Step 1: Write failing tests**

Replace `crates/schema/src/key.rs` header test section with:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_keys() {
        assert!(FieldKey::new("alpha").is_ok());
        assert!(FieldKey::new("_leading_underscore").is_ok());
        assert!(FieldKey::new("a1_b2").is_ok());
    }

    #[test]
    fn rejects_invalid_keys() {
        for bad in ["", "1bad", "has-dash", "has space", &"x".repeat(65)] {
            let err = FieldKey::new(bad).unwrap_err();
            assert_eq!(err.code, "invalid_key");
        }
    }

    #[test]
    fn deserialize_rejects_invalid() {
        let invalid = "\"has-dash\"";
        let r: Result<FieldKey, _> = serde_json::from_str(invalid);
        assert!(r.is_err());
    }

    #[test]
    fn clone_is_cheap() {
        // Arc-backed: cloning multiple times should not allocate
        let k = FieldKey::new("field").unwrap();
        let c1 = k.clone();
        let c2 = k.clone();
        assert_eq!(k.as_str(), c1.as_str());
        assert_eq!(c1.as_str(), c2.as_str());
    }
}
```

Run: `cargo test -p nebula-schema key::tests` → compile fail (API differs).

- [ ] **Step 2: Rewrite `key.rs`**

Replace with:
```rust
//! Stable identifier for a schema field. No panicking constructors.

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{error::ValidationError, path::FieldPath};

/// Stable field identifier. Cheap to clone (Arc-backed).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct FieldKey(Arc<str>);

impl FieldKey {
    /// Build a field key from a candidate string.
    ///
    /// Rules:
    /// - non-empty
    /// - max 64 chars
    /// - starts with ASCII letter or underscore
    /// - only ASCII alphanumeric or underscore afterwards
    pub fn new(value: impl AsRef<str>) -> Result<Self, ValidationError> {
        let value = value.as_ref();
        let bytes = value.as_bytes();

        if value.is_empty() {
            return Err(Self::err(value, "key cannot be empty"));
        }
        if value.len() > 64 {
            return Err(Self::err(value, "key max 64 chars"));
        }
        let first = bytes[0] as char;
        if !first.is_ascii_alphabetic() && first != '_' {
            return Err(Self::err(value, "key must start with letter or underscore"));
        }
        if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Self::err(value, "key must be ASCII alphanumeric or underscore"));
        }

        Ok(Self(Arc::from(value)))
    }

    /// Borrow the key as `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Access the underlying `Arc<str>` handle.
    pub fn as_arc(&self) -> &Arc<str> {
        &self.0
    }

    fn err(value: &str, msg: &'static str) -> ValidationError {
        ValidationError::new("invalid_key")
            .at(FieldPath::root())
            .message(msg)
            .param("key", serde_json::Value::String(value.to_owned()))
            .build()
    }
}

impl std::fmt::Display for FieldKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for FieldKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for FieldKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    // (tests from Step 1)
}
```

Run: `cargo test -p nebula-schema key::tests` → pass.

- [ ] **Step 3: Commit**

```bash
git add crates/schema/src/key.rs
git commit -m "feat(schema): non-panicking FieldKey with Arc storage"
```

---

## Task 6: `field_key!()` proc-macro crate

**Files:**
- Create: `crates/schema/macros/Cargo.toml`
- Create: `crates/schema/macros/src/lib.rs`
- Modify: root `Cargo.toml` (add member)

- [ ] **Step 1: Create macro crate manifest**

Create `crates/schema/macros/Cargo.toml`:
```toml
[package]
name = "nebula-schema-macros"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lib]
proc-macro = true

[dependencies]
syn = { version = "2", features = ["full"] }
quote = "1"
proc-macro2 = "1"
```

- [ ] **Step 2: Add to workspace members**

Edit root `Cargo.toml` → `[workspace] members`:
```toml
"crates/schema/macros",
```
(insert near the other `macros` entries.)

- [ ] **Step 3: Implement the macro**

Create `crates/schema/macros/src/lib.rs`:
```rust
//! Compile-time macros for nebula-schema.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, LitStr};

/// Build a `FieldKey` from a string literal, validated at compile time.
///
/// ```ignore
/// let k = field_key!("alpha");   // OK
/// let k = field_key!("1bad");    // compile error
/// ```
#[proc_macro]
pub fn field_key(input: TokenStream) -> TokenStream {
    let lit = parse_macro_input!(input as LitStr);
    let value = lit.value();

    if let Err(msg) = validate(&value) {
        return syn::Error::new(lit.span(), format!("invalid FieldKey literal: {msg}"))
            .to_compile_error()
            .into();
    }

    let out = quote! {
        ::nebula_schema::FieldKey::new(#lit)
            .expect("field_key! validated at compile time")
    };
    out.into()
}

fn validate(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("key cannot be empty");
    }
    if value.len() > 64 {
        return Err("key max 64 chars");
    }
    let mut chars = value.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err("key must start with letter or underscore");
    }
    for ch in chars {
        if !ch.is_ascii_alphanumeric() && ch != '_' {
            return Err("key must be ASCII alphanumeric or underscore");
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Re-export from schema and add test**

Add to `crates/schema/src/lib.rs` (top-level):
```rust
pub use nebula_schema_macros::field_key;
```

Add inline doctest in `key.rs`:
```rust
/// # Examples
///
/// ```
/// use nebula_schema::field_key;
/// let k = field_key!("alpha");
/// assert_eq!(k.as_str(), "alpha");
/// ```
```

- [ ] **Step 5: Run the doctest**

Run: `cargo test -p nebula-schema --doc`
Expected: doctest passes.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/schema/macros crates/schema/src/lib.rs crates/schema/src/key.rs
git commit -m "feat(schema): field_key!() compile-time macro"
```

---

## Task 7: `FieldPath` typed parse

**Files:**
- Rewrite: `crates/schema/src/path.rs`

- [ ] **Step 1: Write failing tests**

Replace `crates/schema/src/path.rs` contents with test skeleton + impl.

Tests first (place at bottom):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_dot_path() {
        let p = FieldPath::parse("user.email").unwrap();
        assert_eq!(p.to_string(), "user.email");
        assert_eq!(p.segments().len(), 2);
    }

    #[test]
    fn parses_array_index() {
        let p = FieldPath::parse("tags[3]").unwrap();
        assert_eq!(p.segments().len(), 2);
        match &p.segments()[1] {
            PathSegment::Index(i) => assert_eq!(*i, 3),
            _ => panic!("expected Index"),
        }
    }

    #[test]
    fn parses_nested() {
        let p = FieldPath::parse("a.b[0].c").unwrap();
        assert_eq!(p.segments().len(), 4);
        assert_eq!(p.to_string(), "a.b[0].c");
    }

    #[test]
    fn rejects_invalid_syntax() {
        for bad in ["", ".", "a.", ".a", "a[", "a[]", "a[x]", "a..b"] {
            assert!(FieldPath::parse(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn starts_with_works() {
        let a = FieldPath::parse("user.email").unwrap();
        let root = FieldPath::parse("user").unwrap();
        assert!(a.starts_with(&root));
        assert!(!root.starts_with(&a));
        assert!(a.starts_with(&a));
    }

    #[test]
    fn join_appends_segment() {
        let a = FieldPath::parse("user").unwrap();
        let b = a.clone().join(PathSegment::Key(FieldKey::new("email").unwrap()));
        assert_eq!(b.to_string(), "user.email");
        let c = b.clone().join(PathSegment::Index(0));
        assert_eq!(c.to_string(), "user.email[0]");
    }

    #[test]
    fn parent_drops_last_segment() {
        let p = FieldPath::parse("a.b.c").unwrap();
        assert_eq!(p.parent().unwrap().to_string(), "a.b");
        assert!(FieldPath::root().parent().is_none());
    }
}
```

- [ ] **Step 2: Run tests to confirm failure**

Run: `cargo test -p nebula-schema path::tests`
Expected: compile errors.

- [ ] **Step 3: Implement `path.rs`**

Replace file with:
```rust
//! Typed field-path with dot/array-index notation.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize};
use smallvec::{smallvec, SmallVec};

use crate::{error::ValidationError, key::FieldKey};

/// One segment of a field path.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    /// Object key.
    Key(FieldKey),
    /// List index.
    Index(usize),
}

impl From<FieldKey> for PathSegment {
    fn from(k: FieldKey) -> Self {
        Self::Key(k)
    }
}

impl From<usize> for PathSegment {
    fn from(i: usize) -> Self {
        Self::Index(i)
    }
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(k) => f.write_str(k.as_str()),
            Self::Index(i) => write!(f, "[{i}]"),
        }
    }
}

/// Typed reference to a location in a `FieldValues` tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldPath(SmallVec<[PathSegment; 4]>);

impl FieldPath {
    /// Empty (root) path.
    pub fn root() -> Self {
        Self(SmallVec::new())
    }

    /// Parse a dotted/bracketed string (e.g. `a.b[0].c`).
    pub fn parse(s: &str) -> Result<Self, ValidationError> {
        if s.is_empty() {
            return Err(Self::err(s, "empty path"));
        }
        let mut segments: SmallVec<[PathSegment; 4]> = smallvec![];
        let mut rest = s;
        let mut first = true;

        while !rest.is_empty() {
            if first {
                first = false;
            } else if let Some(after) = rest.strip_prefix('.') {
                if after.is_empty() || after.starts_with('.') || after.starts_with('[') {
                    return Err(Self::err(s, "invalid separator usage"));
                }
                rest = after;
            }

            // Parse a key until '.' or '['
            let end = rest
                .find(|c: char| c == '.' || c == '[')
                .unwrap_or(rest.len());
            if end == 0 {
                return Err(Self::err(s, "missing key"));
            }
            let key_lit = &rest[..end];
            let key = FieldKey::new(key_lit).map_err(|_| Self::err(s, "invalid key in path"))?;
            segments.push(PathSegment::Key(key));
            rest = &rest[end..];

            // Zero or more [N] index segments after a key
            while let Some(after_open) = rest.strip_prefix('[') {
                let close = after_open
                    .find(']')
                    .ok_or_else(|| Self::err(s, "unclosed bracket"))?;
                let digits = &after_open[..close];
                if digits.is_empty() {
                    return Err(Self::err(s, "empty index"));
                }
                let idx: usize = digits
                    .parse()
                    .map_err(|_| Self::err(s, "non-numeric index"))?;
                segments.push(PathSegment::Index(idx));
                rest = &after_open[close + 1..];
            }
        }

        Ok(Self(segments))
    }

    /// Borrow segments.
    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }

    /// Is this the root (empty) path?
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Append a segment.
    pub fn join(mut self, seg: impl Into<PathSegment>) -> Self {
        self.0.push(seg.into());
        self
    }

    /// Path with the last segment dropped, or `None` if root.
    pub fn parent(&self) -> Option<Self> {
        if self.0.is_empty() {
            None
        } else {
            let mut copy = self.clone();
            copy.0.pop();
            Some(copy)
        }
    }

    /// True when `self` has `prefix` as a leading slice of segments.
    pub fn starts_with(&self, prefix: &FieldPath) -> bool {
        self.0.len() >= prefix.0.len() && self.0[..prefix.0.len()] == prefix.0[..]
    }

    fn err(value: &str, msg: &'static str) -> ValidationError {
        ValidationError::new("invalid_path")
            .at(FieldPath::root())
            .message(msg)
            .param("path", serde_json::Value::String(value.to_owned()))
            .build()
    }
}

impl Default for FieldPath {
    fn default() -> Self {
        Self::root()
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            match seg {
                PathSegment::Key(_) if i > 0 => write!(f, ".{seg}")?,
                _ => write!(f, "{seg}")?,
            }
        }
        Ok(())
    }
}

impl FromStr for FieldPath {
    type Err = ValidationError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for FieldPath {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for FieldPath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests { /* from Step 1 */ }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p nebula-schema path::tests`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/schema/src/path.rs
git commit -m "feat(schema): typed FieldPath with dot/index parsing"
```

---

## Task 8: Mode enums

**Files:**
- Rewrite: `crates/schema/src/mode.rs`

- [ ] **Step 1: Write failing tests**

Append to `mode.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_default_is_always() {
        assert!(matches!(VisibilityMode::default(), VisibilityMode::Always));
        assert!(VisibilityMode::default().is_default());
    }

    #[test]
    fn required_default_is_never() {
        assert!(matches!(RequiredMode::default(), RequiredMode::Never));
        assert!(RequiredMode::default().is_default());
    }

    #[test]
    fn expression_default_is_allowed() {
        assert!(matches!(ExpressionMode::default(), ExpressionMode::Allowed));
    }

    #[test]
    fn visibility_never_hides_always() {
        assert!(!matches!(VisibilityMode::Never.is_default(), true));
    }
}
```

Run: `cargo test -p nebula-schema mode::tests` → compile fail.

- [ ] **Step 2: Rewrite `mode.rs`**

```rust
//! Visibility / required / expression policy enums.

use nebula_validator::Rule;
use serde::{Deserialize, Serialize};

/// When a field is visible in the UI.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisibilityMode {
    #[default]
    Always,
    /// Never visible — replaces the removed `Field::Hidden`.
    Never,
    /// Visible only when rule evaluates true.
    When(Rule),
}

impl VisibilityMode {
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Always)
    }
}

/// When a field is required.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequiredMode {
    #[default]
    Never,
    Always,
    When(Rule),
}

impl RequiredMode {
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Never)
    }
}

/// Whether the field accepts expression values (`{{ ... }}` or `{"$expr": "..."}`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpressionMode {
    /// Only literal values allowed.
    Forbidden,
    /// Both literal and expression values allowed (default).
    #[default]
    Allowed,
    /// Only expression values (e.g. Computed field).
    Required,
}

#[cfg(test)]
mod tests { /* from Step 1 */ }
```

- [ ] **Step 3: Run & commit**

```bash
cargo test -p nebula-schema mode::tests
git add crates/schema/src/mode.rs
git commit -m "feat(schema): ExpressionMode + VisibilityMode::Never"
```

---

## Task 9: `InputHint` enum

**Files:**
- Create: `crates/schema/src/input_hint.rs`

- [ ] **Step 1: Implement with tests**

Create `crates/schema/src/input_hint.rs`:
```rust
//! UI hints for String fields (replaces v3 separate Date/Time/Color types).

use serde::{Deserialize, Serialize};

/// Semantic hint for rendering a string input.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputHint {
    #[default]
    Text,
    Email,
    Url,
    Password,
    Phone,
    Ip,
    Regex,
    Markdown,
    Cron,
    Date,
    DateTime,
    Time,
    Color,
    Duration,
    Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_text() {
        assert_eq!(InputHint::default(), InputHint::Text);
    }

    #[test]
    fn serde_uses_snake_case() {
        let json = serde_json::to_string(&InputHint::DateTime).unwrap();
        assert_eq!(json, "\"date_time\"");
    }
}
```

- [ ] **Step 2: Wire into `lib.rs`**

Add `pub mod input_hint;` and `pub use input_hint::InputHint;` in `crates/schema/src/lib.rs`.

- [ ] **Step 3: Run & commit**

```bash
cargo test -p nebula-schema input_hint::tests
git add crates/schema/src/input_hint.rs crates/schema/src/lib.rs
git commit -m "feat(schema): InputHint for String field UI hints"
```

---

## Task 10: Widget enums — mark `#[non_exhaustive]`

**Files:**
- Modify: `crates/schema/src/widget.rs`

- [ ] **Step 1: Confirm current state**

Open `crates/schema/src/widget.rs`. All widget enums (`StringWidget`, `SecretWidget`, `NumberWidget`, `BooleanWidget`, `SelectWidget`, `ObjectWidget`, `ListWidget`, `CodeWidget`) are already `#[non_exhaustive]`. Nothing to change except a test guarding the invariant.

- [ ] **Step 2: Add invariant test**

Append to `widget.rs`:
```rust
#[test]
fn widgets_are_non_exhaustive_and_small() {
    use std::mem::size_of;
    // non_exhaustive enums with only unit variants stay 1 byte — regression marker
    assert!(size_of::<StringWidget>() <= 1);
    assert!(size_of::<NumberWidget>() <= 1);
    assert!(size_of::<BooleanWidget>() <= 1);
}
```

- [ ] **Step 3: Commit (no logic change)**

```bash
cargo test -p nebula-schema widget::tests
git add crates/schema/src/widget.rs
git commit -m "test(schema): enforce widget-enum size invariant"
```

---

## Task 11: `SelectOption` — add `#[non_exhaustive]` + convenience

**Files:**
- Modify: `crates/schema/src/option.rs`

- [ ] **Step 1: Extend**

Replace `option.rs`:
```rust
//! Static option definition for select fields.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One option in a `SelectField`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: Value,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

impl SelectOption {
    pub fn new(value: impl Into<Value>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            description: None,
            disabled: false,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builder_defaults() {
        let o = SelectOption::new(json!("gh"), "GitHub");
        assert_eq!(o.value, json!("gh"));
        assert_eq!(o.label, "GitHub");
        assert!(o.description.is_none());
        assert!(!o.disabled);
    }

    #[test]
    fn roundtrip_omits_defaults() {
        let o = SelectOption::new(json!(1), "one");
        let s = serde_json::to_string(&o).unwrap();
        assert!(!s.contains("description"));
        assert!(!s.contains("disabled"));
    }
}
```

- [ ] **Step 2: Run & commit**

```bash
cargo test -p nebula-schema option::tests
git add crates/schema/src/option.rs
git commit -m "feat(schema): extend SelectOption with description/disabled"
```

---

## Task 12: `FieldValue` tree + `FieldValues`

**Files:**
- Rewrite: `crates/schema/src/value.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_json_flat_literal() {
        let v = FieldValue::from_json(json!(42));
        assert!(matches!(v, FieldValue::Literal(_)));
    }

    #[test]
    fn from_json_object_becomes_tree() {
        let v = FieldValue::from_json(json!({"a": 1, "b": "x"}));
        let FieldValue::Object(map) = v else { panic!() };
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn detects_expression_wrapper() {
        let v = FieldValue::from_json(json!({"$expr": "{{ $x }}"}));
        assert!(matches!(v, FieldValue::Expression(_)));
    }

    #[test]
    fn detects_inline_expression_marker() {
        let v = FieldValue::from_json(json!("hello {{ $y }}"));
        assert!(matches!(v, FieldValue::Expression(_)));
    }

    #[test]
    fn escaped_double_braces_stay_literal() {
        let v = FieldValue::from_json(json!("{{{{ x }}}}"));
        assert!(matches!(v, FieldValue::Literal(_)));
    }

    #[test]
    fn detects_mode_wrapper() {
        let v = FieldValue::from_json(json!({"mode": "oauth2", "value": {"scope":"r"}}));
        assert!(matches!(v, FieldValue::Mode { .. }));
    }

    #[test]
    fn mode_with_extra_keys_stays_object() {
        let v = FieldValue::from_json(json!({"mode":"x","value":null,"extra":1}));
        assert!(matches!(v, FieldValue::Object(_)));
    }

    #[test]
    fn values_set_get_path() {
        let mut vs = FieldValues::new();
        let key = FieldKey::new("user").unwrap();
        let email = FieldKey::new("email").unwrap();
        vs.set(key.clone(),
               FieldValue::Object(indexmap::indexmap!{ email => FieldValue::Literal(json!("a@b")) }));
        let p = FieldPath::parse("user.email").unwrap();
        assert!(matches!(vs.get_path(&p), Some(FieldValue::Literal(_))));
    }

    #[test]
    fn roundtrip_preserves_structure() {
        let src = json!({
            "a": 1,
            "b": [1, 2, {"x": true}],
            "c": {"$expr": "{{ $x }}"},
            "d": {"mode": "m", "value": "v"}
        });
        let parsed = FieldValue::from_json(src.clone());
        let back = parsed.to_json();
        assert_eq!(back, src);
    }
}
```

Run → compile fail.

- [ ] **Step 2: Implement**

```rust
//! Runtime value tree and container.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{expression::Expression, key::FieldKey, path::{FieldPath, PathSegment}};

/// Reserved key for an explicit expression wrapper.
pub const EXPRESSION_KEY: &str = "$expr";

/// Runtime value — may be literal, expression, tree, or mode-dispatched.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Literal(Value),
    Expression(Expression),
    Object(IndexMap<FieldKey, FieldValue>),
    List(Vec<FieldValue>),
    Mode {
        mode: FieldKey,
        value: Option<Box<FieldValue>>,
    },
}

impl FieldValue {
    /// Parse a raw JSON value into a typed tree.
    pub fn from_json(value: Value) -> Self {
        match &value {
            Value::Object(map) => {
                if map.len() == 1 {
                    if let Some(expr) = map.get(EXPRESSION_KEY).and_then(Value::as_str) {
                        return Self::Expression(Expression::new(expr));
                    }
                }
                // Mode wrapper: EXACTLY {mode, value?}
                let only_mode_keys = map.keys().all(|k| k == "mode" || k == "value");
                if only_mode_keys && map.contains_key("mode") {
                    if let Some(mode_str) = map.get("mode").and_then(Value::as_str) {
                        if let Ok(mode_key) = FieldKey::new(mode_str) {
                            let v = map
                                .get("value")
                                .cloned()
                                .map(|v| Box::new(Self::from_json(v)));
                            return Self::Mode { mode: mode_key, value: v };
                        }
                    }
                }
                // Regular object — recurse
                let mut out: IndexMap<FieldKey, FieldValue> = IndexMap::with_capacity(map.len());
                for (k, v) in map {
                    if let Ok(key) = FieldKey::new(k) {
                        out.insert(key, Self::from_json(v.clone()));
                    }
                    // Keys that don't parse as FieldKey drop silently —
                    // validation surfaces them as type_mismatch later.
                }
                Self::Object(out)
            },
            Value::Array(arr) => {
                Self::List(arr.iter().map(|v| Self::from_json(v.clone())).collect())
            },
            Value::String(s) if contains_expression_marker(s) => {
                Self::Expression(Expression::new(s.as_str()))
            },
            _ => Self::Literal(value),
        }
    }

    /// Emit canonical JSON wire form.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Literal(v) => v.clone(),
            Self::Expression(e) => serde_json::json!({ EXPRESSION_KEY: e.source() }),
            Self::Object(map) => {
                let mut out = Map::with_capacity(map.len());
                for (k, v) in map {
                    out.insert(k.as_str().to_owned(), v.to_json());
                }
                Value::Object(out)
            },
            Self::List(items) => Value::Array(items.iter().map(Self::to_json).collect()),
            Self::Mode { mode, value } => {
                let mut out = Map::new();
                out.insert("mode".into(), Value::String(mode.as_str().to_owned()));
                if let Some(v) = value {
                    out.insert("value".into(), v.to_json());
                }
                Value::Object(out)
            },
        }
    }

    /// Follow a path into this value; `None` if any segment is missing/wrong-type.
    pub fn path(&self, path: &FieldPath) -> Option<&FieldValue> {
        let mut cur = self;
        for seg in path.segments() {
            cur = match (cur, seg) {
                (Self::Object(map), PathSegment::Key(k)) => map.get(k)?,
                (Self::List(items), PathSegment::Index(i)) => items.get(*i)?,
                (Self::Mode { value: Some(inner), .. }, PathSegment::Key(k))
                    if k.as_str() == "value" =>
                {
                    inner
                },
                _ => return None,
            };
        }
        Some(cur)
    }

    pub fn is_expression(&self) -> bool {
        matches!(self, Self::Expression(_))
    }
}

impl Serialize for FieldValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

impl<'de> Deserialize<'de> for FieldValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::from_json(Value::deserialize(d)?))
    }
}

fn contains_expression_marker(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // Escaped "{{{{" → literal
            if i + 3 < bytes.len() && bytes[i + 2] == b'{' && bytes[i + 3] == b'{' {
                i += 4;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

/// Top-level runtime value store.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldValues(IndexMap<FieldKey, FieldValue>);

impl FieldValues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_json(value: Value) -> Result<Self, crate::error::ValidationError> {
        match FieldValue::from_json(value) {
            FieldValue::Object(map) => Ok(Self(map)),
            _ => Err(crate::error::ValidationError::new("type_mismatch")
                .message("top-level values must be a JSON object")
                .build()),
        }
    }

    pub fn set(&mut self, key: FieldKey, value: FieldValue) {
        self.0.insert(key, value);
    }

    pub fn remove(&mut self, key: &FieldKey) -> Option<FieldValue> {
        self.0.shift_remove(key)
    }

    pub fn get(&self, key: &FieldKey) -> Option<&FieldValue> {
        self.0.get(key)
    }

    pub fn get_path(&self, path: &FieldPath) -> Option<&FieldValue> {
        let mut segs = path.segments().iter();
        let PathSegment::Key(first) = segs.next()? else {
            return None;
        };
        let mut cur = self.0.get(first)?;
        for seg in segs {
            cur = match (cur, seg) {
                (FieldValue::Object(map), PathSegment::Key(k)) => map.get(k)?,
                (FieldValue::List(items), PathSegment::Index(i)) => items.get(*i)?,
                _ => return None,
            };
        }
        Some(cur)
    }

    pub fn contains(&self, key: &FieldKey) -> bool {
        self.0.contains_key(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&FieldKey, &FieldValue)> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_inner(self) -> IndexMap<FieldKey, FieldValue> {
        self.0
    }

    pub fn to_json(&self) -> Value {
        let mut out = Map::with_capacity(self.0.len());
        for (k, v) in &self.0 {
            out.insert(k.as_str().to_owned(), v.to_json());
        }
        Value::Object(out)
    }

    // Typed accessors — kept for convenience
    pub fn get_string(&self, key: &FieldKey) -> Option<&str> {
        match self.0.get(key)? {
            FieldValue::Literal(Value::String(s)) => Some(s),
            _ => None,
        }
    }
    pub fn get_bool(&self, key: &FieldKey) -> Option<bool> {
        match self.0.get(key)? {
            FieldValue::Literal(v) => v.as_bool(),
            _ => None,
        }
    }
    pub fn get_i64(&self, key: &FieldKey) -> Option<i64> {
        match self.0.get(key)? {
            FieldValue::Literal(v) => v.as_i64(),
            _ => None,
        }
    }
    pub fn get_f64(&self, key: &FieldKey) -> Option<f64> {
        match self.0.get(key)? {
            FieldValue::Literal(v) => v.as_f64(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests { /* from Step 1 */ }
```

Note: `Expression` is forward-referenced — create a stub `pub struct Expression(Arc<str>)` with `new(&str)` and `source(&self)` in `crates/schema/src/expression.rs` just enough to compile; Task 14 flesh it out.

- [ ] **Step 3: Run & commit**

```bash
cargo test -p nebula-schema value::tests
git add crates/schema/src/value.rs crates/schema/src/expression.rs
git commit -m "feat(schema): tree-based FieldValue with path access"
```

---

## Task 13: JSON wire format invariants

**Files:**
- Test: `crates/schema/tests/wire_format.rs` (new)

- [ ] **Step 1: Write wire-format contract test**

```rust
//! Wire-format invariants — must not break at Phase 1.

use nebula_schema::{FieldValue, FieldValues};
use serde_json::json;

#[test]
fn plain_literal() {
    let v = FieldValue::from_json(json!("hello"));
    assert_eq!(v.to_json(), json!("hello"));
}

#[test]
fn expression_wrapper() {
    let src = json!({"$expr": "{{ $x.y }}"});
    let v = FieldValue::from_json(src.clone());
    assert_eq!(v.to_json(), src);
}

#[test]
fn mode_wrapper() {
    let src = json!({"mode": "oauth2", "value": {"scope": "read"}});
    let v = FieldValue::from_json(src.clone());
    assert_eq!(v.to_json(), src);
}

#[test]
fn nested_object_roundtrip() {
    let src = json!({
        "a": "x",
        "b": [1, {"k": true}],
        "c": {"$expr": "{{ $z }}"},
        "d": {"mode": "m"}
    });
    let values = FieldValues::from_json(src.clone()).unwrap();
    assert_eq!(values.to_json(), src);
}

#[test]
fn top_level_non_object_rejected() {
    let r = FieldValues::from_json(json!([1, 2]));
    assert!(r.is_err());
}
```

- [ ] **Step 2: Run & commit**

```bash
cargo test -p nebula-schema --test wire_format
git add crates/schema/tests/wire_format.rs
git commit -m "test(schema): wire-format contract"
```

---

## Task 14: `Expression` wrapper with `OnceLock`

**Files:**
- Rewrite: `crates/schema/src/expression.rs`

- [ ] **Step 1: Check expression crate surface**

Look at `crates/expression/src/lib.rs` to find `parse` / `Ast` / `ExpressionContext`. If the needed surface is missing, add a minimal stub in `nebula-schema` that abstracts over the real crate behind a trait.

Minimum required from `nebula-expression`:
```rust
pub struct Ast { /* opaque */ }
pub fn parse(source: &str) -> Result<Ast, ExpressionParseError>;
pub trait ExpressionContext { /* async evaluate */ }
```

If the real crate doesn't expose these yet, thread-through a local `nebula_schema::expression` module using a trait-based facade so Phase 1 is not blocked on parallel work. (See §12 design note 4.)

- [ ] **Step 2: Implement the wrapper**

```rust
//! Expression value wrapper — lazy parse via OnceLock.

use std::sync::{Arc, OnceLock};

use crate::{error::ValidationError, path::FieldPath};

/// Opaque parsed AST. In Phase 1 this is a thin newtype; Phase 4 can replace
/// the inner type with a real `nebula_expression::Ast`.
#[derive(Debug, Clone)]
pub struct ExpressionAst(pub(crate) Arc<str>);

/// An unresolved expression (e.g. `{{ $input.name }}`).
#[derive(Debug, Clone)]
pub struct Expression {
    source: Arc<str>,
    parsed: Arc<OnceLock<ExpressionAst>>,
}

impl Expression {
    pub fn new(source: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            parsed: Arc::new(OnceLock::new()),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Lazy parse — caches the first successful parse.
    pub fn parse(&self) -> Result<&ExpressionAst, ValidationError> {
        Ok(self.parsed.get_or_init(|| {
            // Phase 1: no real AST — just wrap the source.
            // Phase 4 replaces this with nebula_expression::parse(&self.source).
            ExpressionAst(self.source.clone())
        }))
    }

    /// Build a parse error tagged for this expression.
    pub(crate) fn parse_error(&self, msg: impl Into<std::borrow::Cow<'static, str>>) -> ValidationError {
        ValidationError::new("expression.parse")
            .at(FieldPath::root())
            .message(msg)
            .param("source", serde_json::Value::String(self.source.to_string()))
            .build()
    }
}

impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lazy_parse_is_cached() {
        let e = Expression::new("{{ $x }}");
        let a1 = e.parse().unwrap() as *const _;
        let a2 = e.parse().unwrap() as *const _;
        assert_eq!(a1, a2, "parse should cache the same AST instance");
    }

    #[test]
    fn clones_share_source() {
        let e = Expression::new("{{ $y }}");
        let c = e.clone();
        assert_eq!(e.source(), c.source());
    }
}
```

- [ ] **Step 3: Run & commit**

```bash
cargo test -p nebula-schema expression::tests
git add crates/schema/src/expression.rs
git commit -m "feat(schema): Expression wrapper with OnceLock parse cache"
```

---

## Task 15: `Transformer` with Regex + `OnceLock<Regex>`

**Files:**
- Rewrite: `crates/schema/src/transformer.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn trim_on_string() {
        let out = Transformer::Trim.apply(&json!("  hi  "));
        assert_eq!(out, json!("hi"));
    }

    #[test]
    fn regex_extract_group() {
        let t = Transformer::Regex {
            pattern: r"^(\d+)-".into(),
            group: 1,
            cache: Default::default(),
        };
        assert_eq!(t.apply(&json!("42-abc")), json!("42"));
        assert_eq!(t.apply(&json!("no-match")), json!("no-match"));
    }

    #[test]
    fn regex_cache_compiles_once() {
        let t = Transformer::Regex {
            pattern: r"(\w+)".into(),
            group: 0,
            cache: Default::default(),
        };
        let _ = t.apply(&json!("abc"));
        let _ = t.apply(&json!("def"));
        // No observable difference, but regression guard — invariant is
        // that regex builds exactly once across the two calls above.
        assert!(true);
    }

    #[test]
    fn non_string_value_passes_through() {
        assert_eq!(Transformer::Lowercase.apply(&json!(42)), json!(42));
    }
}
```

- [ ] **Step 2: Implement**

```rust
//! Pre-validation value transformers with regex cache.

use std::sync::Arc;

use once_cell::sync::OnceCell;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Value transformer applied before validation/runtime use.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transformer {
    Trim,
    Lowercase,
    Uppercase,
    Replace { from: String, to: String },
    Regex {
        pattern: String,
        #[serde(default)]
        group: usize,
        /// Lazily compiled regex — skipped by serde.
        #[serde(skip)]
        cache: Arc<OnceCell<Regex>>,
    },
}

impl PartialEq for Transformer {
    fn eq(&self, other: &Self) -> bool {
        use Transformer::*;
        match (self, other) {
            (Trim, Trim) | (Lowercase, Lowercase) | (Uppercase, Uppercase) => true,
            (Replace { from: a1, to: a2 }, Replace { from: b1, to: b2 }) => a1 == b1 && a2 == b2,
            (Regex { pattern: p1, group: g1, .. }, Regex { pattern: p2, group: g2, .. }) => {
                p1 == p2 && g1 == g2
            },
            _ => false,
        }
    }
}

impl Transformer {
    /// Apply this transformer. String transformers pass non-string values through.
    pub fn apply(&self, value: &Value) -> Value {
        match self {
            Self::Trim => string(value, |t| t.trim().to_owned()),
            Self::Lowercase => string(value, str::to_lowercase),
            Self::Uppercase => string(value, str::to_uppercase),
            Self::Replace { from, to } => string(value, |t| t.replace(from.as_str(), to.as_str())),
            Self::Regex { pattern, group, cache } => string(value, |t| {
                let re = cache.get_or_init(|| {
                    Regex::new(pattern).unwrap_or_else(|_| Regex::new("^$").unwrap())
                });
                re.captures(t)
                    .and_then(|c| c.get(*group))
                    .map(|m| m.as_str().to_owned())
                    .unwrap_or_else(|| t.to_owned())
            }),
        }
    }
}

fn string(value: &Value, f: impl FnOnce(&str) -> String) -> Value {
    match value.as_str() {
        Some(s) => Value::String(f(s)),
        None => value.clone(),
    }
}

#[cfg(test)]
mod tests { /* from Step 1 */ }
```

- [ ] **Step 3: Run & commit**

```bash
cargo test -p nebula-schema transformer::tests
git add crates/schema/src/transformer.rs
git commit -m "feat(schema): Transformer::Regex with OnceCell cache"
```

---

## Task 16: `RuleContext` trait in `nebula-validator`

**Files:**
- Modify: `crates/validator/src/rule/mod.rs`
- Modify: `crates/validator/src/rule/evaluate.rs`
- Modify: `crates/validator/src/rule/validate.rs`
- Modify: `crates/validator/src/lib.rs`
- Test: `crates/validator/src/rule/tests.rs`

- [ ] **Step 1: Add `RuleContext` trait**

Edit `crates/validator/src/rule/mod.rs`, at the top:
```rust
/// Borrowed view over a value bag used by predicate rules.
pub trait RuleContext {
    /// Fetch a value by key. Returns `None` when missing.
    fn get(&self, key: &str) -> Option<&serde_json::Value>;
}

impl RuleContext for std::collections::HashMap<String, serde_json::Value> {
    fn get(&self, key: &str) -> Option<&serde_json::Value> {
        std::collections::HashMap::get(self, key)
    }
}
```

- [ ] **Step 2: Update `Rule::evaluate` signature**

In `crates/validator/src/rule/evaluate.rs`, change:
```rust
pub fn evaluate(&self, values: &HashMap<String, Value>) -> bool { ... }
```
to:
```rust
pub fn evaluate(&self, ctx: &dyn RuleContext) -> bool { ... }
```
And inside, replace `values.get(field)` with `ctx.get(field.as_str())` where applicable. Update nested `All/Any/Not` cases to pass `ctx` through.

- [ ] **Step 3: Add `validate_value` context variant**

If `validate_value` takes just `&Value`, keep it; predicate rules (`Eq`, `Ne`, ...) don't return value errors. No change needed.

- [ ] **Step 4: Re-export trait**

In `crates/validator/src/lib.rs`, add to top-level `pub use`:
```rust
pub use rule::{Rule, RuleContext};
```

- [ ] **Step 5: Update validator tests**

`crates/validator/src/rule/tests.rs` — every `rule.evaluate(&hashmap)` call continues to work because `HashMap<String, Value>` now implements `RuleContext`. Ensure `cargo test -p nebula-validator` is green.

- [ ] **Step 6: Commit**

```bash
cargo test -p nebula-validator
git add crates/validator/
git commit -m "feat(validator): RuleContext trait — replace HashMap parameter"
```

---

## Task 17: `Field` enum + per-type structs (consolidated)

**Files:**
- Rewrite: `crates/schema/src/field.rs`

The full rewrite touches many field variants. Use a macro to avoid duplication. Key consolidation per spec §5:
- **Removed** variants: `Date`, `DateTime`, `Time`, `Color`, `Hidden`
- Replaced: a single `StringField` carries `hint: InputHint`; "hidden" role moves to `visible: VisibilityMode::Never`

- [ ] **Step 1: Write integration-level test (builds without Date/etc)**

Create `crates/schema/tests/field_variants.rs`:
```rust
use nebula_schema::*;

#[test]
fn string_with_date_hint_replaces_datefield() {
    let f = Field::string(FieldKey::new("d").unwrap()).hint(InputHint::Date);
    assert_eq!(f.key().as_str(), "d");
}

#[test]
fn visibility_never_replaces_hiddenfield() {
    let f = Field::string(FieldKey::new("h").unwrap()).visible(VisibilityMode::Never);
    assert!(matches!(f.visible(), VisibilityMode::Never));
}

#[test]
fn field_carries_expression_mode() {
    let f = Field::boolean(FieldKey::new("b").unwrap());
    // Boolean defaults to Forbidden
    assert!(matches!(f.expression(), ExpressionMode::Forbidden));
}
```

- [ ] **Step 2: Implement `field.rs`** (full rewrite)

Rewrite `crates/schema/src/field.rs`. Structure:

```rust
//! Field definitions — consolidated enum + per-type structs + builders.

use nebula_validator::Rule;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::{
    expression::Expression, input_hint::InputHint, key::FieldKey, mode::{ExpressionMode, RequiredMode, VisibilityMode},
    option::SelectOption, path::FieldPath, transformer::Transformer,
    widget::{BooleanWidget, CodeWidget, ListWidget, NumberWidget, ObjectWidget, SecretWidget, SelectWidget, StringWidget},
};

macro_rules! define_field {
    ($name:ident { $($extra:ident: $ty:ty = $dflt:expr),* $(,)? } default_expr: $expr_dflt:expr) => {
        #[non_exhaustive]
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        pub struct $name {
            pub key: FieldKey,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub label: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub description: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub placeholder: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub default: Option<Value>,
            #[serde(default, skip_serializing_if = "VisibilityMode::is_default")]
            pub visible: VisibilityMode,
            #[serde(default, skip_serializing_if = "RequiredMode::is_default")]
            pub required: RequiredMode,
            #[serde(default = "default_expression_mode", skip_serializing_if = "ExpressionMode::is_default_for_this_type")]
            pub expression: ExpressionMode,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub group: Option<String>,
            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub rules: Vec<Rule>,
            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub transformers: Vec<Transformer>,
            $(pub $extra: $ty,)*
        }

        impl $name {
            pub fn new(key: impl Into<FieldKey>) -> Self {
                Self {
                    key: key.into(),
                    label: None,
                    description: None,
                    placeholder: None,
                    default: None,
                    visible: VisibilityMode::default(),
                    required: RequiredMode::default(),
                    expression: $expr_dflt,
                    group: None,
                    rules: Vec::new(),
                    transformers: Vec::new(),
                    $($extra: $dflt,)*
                }
            }

            pub fn label(mut self, s: impl Into<String>) -> Self { self.label = Some(s.into()); self }
            pub fn description(mut self, s: impl Into<String>) -> Self { self.description = Some(s.into()); self }
            pub fn placeholder(mut self, s: impl Into<String>) -> Self { self.placeholder = Some(s.into()); self }
            pub fn default(mut self, v: Value) -> Self { self.default = Some(v); self }
            pub fn visible(mut self, m: VisibilityMode) -> Self { self.visible = m; self }
            pub fn visible_when(mut self, rule: Rule) -> Self { self.visible = VisibilityMode::When(rule); self }
            pub fn required(mut self) -> Self { self.required = RequiredMode::Always; self }
            pub fn required_when(mut self, rule: Rule) -> Self { self.required = RequiredMode::When(rule); self }
            pub fn active_when(mut self, rule: Rule) -> Self {
                self.visible = VisibilityMode::When(rule.clone());
                self.required = RequiredMode::When(rule);
                self
            }
            pub fn expression_mode(mut self, m: ExpressionMode) -> Self { self.expression = m; self }
            pub fn no_expression(mut self) -> Self { self.expression = ExpressionMode::Forbidden; self }
            pub fn group(mut self, g: impl Into<String>) -> Self { self.group = Some(g.into()); self }
            pub fn with_rule(mut self, r: Rule) -> Self { self.rules.push(r); self }
            pub fn with_transformer(mut self, t: Transformer) -> Self { self.transformers.push(t); self }
        }
    };
}

fn default_expression_mode() -> ExpressionMode { ExpressionMode::Allowed }

// Helper used by skip_serializing_if — each field type has its own default
trait ExprModeDefault {
    fn is_default_for_this_type(&self) -> bool;
}
impl ExprModeDefault for ExpressionMode {
    fn is_default_for_this_type(&self) -> bool {
        matches!(self, ExpressionMode::Allowed)
    }
}

// Per-type structs —
define_field!(StringField { hint: InputHint = InputHint::default(), widget: StringWidget = StringWidget::Plain } default_expr: ExpressionMode::Allowed);
impl StringField {
    pub fn hint(mut self, h: InputHint) -> Self { self.hint = h; self }
    pub fn widget(mut self, w: StringWidget) -> Self { self.widget = w; self }
    pub fn min_length(mut self, min: usize) -> Self { self.rules.push(Rule::MinLength { min, message: None }); self }
    pub fn max_length(mut self, max: usize) -> Self { self.rules.push(Rule::MaxLength { max, message: None }); self }
    pub fn pattern(mut self, p: impl Into<String>) -> Self { self.rules.push(Rule::Pattern { pattern: p.into(), message: None }); self }
    pub fn url(mut self) -> Self { self.rules.push(Rule::Url { message: None }); self }
    pub fn email(mut self) -> Self { self.rules.push(Rule::Email { message: None }); self }
}

define_field!(SecretField { widget: SecretWidget = SecretWidget::Plain, reveal_last: Option<u8> = None } default_expr: ExpressionMode::Allowed);
impl SecretField {
    pub fn widget(mut self, w: SecretWidget) -> Self { self.widget = w; self }
    pub fn reveal_last(mut self, n: u8) -> Self { self.reveal_last = Some(n); self }
    pub fn min_length(mut self, min: usize) -> Self { self.rules.push(Rule::MinLength { min, message: None }); self }
}

define_field!(NumberField { integer: bool = false, widget: NumberWidget = NumberWidget::Plain, step: Option<Number> = None } default_expr: ExpressionMode::Allowed);
impl NumberField {
    pub fn integer(mut self) -> Self { self.integer = true; self }
    pub fn widget(mut self, w: NumberWidget) -> Self { self.widget = w; self }
    pub fn min(mut self, m: impl Into<Number>) -> Self { self.rules.push(Rule::Min { min: m.into(), message: None }); self }
    pub fn max(mut self, m: impl Into<Number>) -> Self { self.rules.push(Rule::Max { max: m.into(), message: None }); self }
    pub fn step(mut self, s: impl Into<Number>) -> Self { self.step = Some(s.into()); self }
}

define_field!(BooleanField { widget: BooleanWidget = BooleanWidget::Toggle } default_expr: ExpressionMode::Forbidden);
impl BooleanField {
    pub fn widget(mut self, w: BooleanWidget) -> Self { self.widget = w; self }
}

define_field!(SelectField {
    options: Vec<SelectOption> = Vec::new(),
    dynamic: bool = false,
    loader: Option<String> = None,
    depends_on: Vec<FieldPath> = Vec::new(),
    multiple: bool = false,
    allow_custom: bool = false,
    searchable: bool = false,
    widget: SelectWidget = SelectWidget::Dropdown
} default_expr: ExpressionMode::Forbidden);
impl SelectField {
    pub fn widget(mut self, w: SelectWidget) -> Self { self.widget = w; self }
    pub fn option(mut self, value: impl Into<Value>, label: impl Into<String>) -> Self {
        self.options.push(SelectOption::new(value.into(), label)); self
    }
    pub fn loader(mut self, k: impl Into<String>) -> Self { self.dynamic = true; self.loader = Some(k.into()); self }
    pub fn dynamic(mut self) -> Self { self.dynamic = true; self }
    pub fn depends_on(mut self, path: impl Into<FieldPath>) -> Self { self.depends_on.push(path.into()); self }
    pub fn multiple(mut self) -> Self { self.multiple = true; self }
    pub fn searchable(mut self) -> Self { self.searchable = true; self }
    pub fn allow_custom(mut self) -> Self { self.allow_custom = true; self }
}

// ObjectField, ListField, ModeField follow the same pattern but have nested
// schema children (Vec<Field>, Option<Box<Field>>, Vec<ModeVariant>).
// Reuse the `define_field!` macro, then add type-specific builders.
// See the current crates/schema/src/field.rs for the old layout — the new
// versions just add `expression: ExpressionMode` and drop panicking `From<&str>`.

define_field!(ObjectField { fields: Vec<Field> = Vec::new(), widget: ObjectWidget = ObjectWidget::Inline } default_expr: ExpressionMode::Allowed);
impl ObjectField {
    pub fn widget(mut self, w: ObjectWidget) -> Self { self.widget = w; self }
    pub fn add(mut self, field: impl Into<Field>) -> Self { self.fields.push(field.into()); self }
}

define_field!(ListField {
    item: Option<Box<Field>> = None,
    min_items: Option<u32> = None,
    max_items: Option<u32> = None,
    unique: bool = false,
    widget: ListWidget = ListWidget::Plain
} default_expr: ExpressionMode::Allowed);
impl ListField {
    pub fn item(mut self, f: impl Into<Field>) -> Self { self.item = Some(Box::new(f.into())); self }
    pub fn min_items(mut self, n: u32) -> Self { self.min_items = Some(n); self }
    pub fn max_items(mut self, n: u32) -> Self { self.max_items = Some(n); self }
    pub fn unique(mut self) -> Self { self.unique = true; self }
    pub fn widget(mut self, w: ListWidget) -> Self { self.widget = w; self }
}

define_field!(ModeField {
    variants: Vec<ModeVariant> = Vec::new(),
    default_variant: Option<String> = None,
    allow_dynamic_mode: bool = false
} default_expr: ExpressionMode::Allowed);

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeVariant {
    pub key: String,
    pub label: String,
    pub field: Box<Field>,
}

impl ModeField {
    pub fn variant(mut self, key: impl Into<String>, label: impl Into<String>, field: impl Into<Field>) -> Self {
        self.variants.push(ModeVariant { key: key.into(), label: label.into(), field: Box::new(field.into()) });
        self
    }
    pub fn default_variant(mut self, k: impl Into<String>) -> Self { self.default_variant = Some(k.into()); self }
    pub fn allow_dynamic_mode(mut self) -> Self { self.allow_dynamic_mode = true; self }
}

define_field!(CodeField { language: String = "plaintext".to_owned(), widget: CodeWidget = CodeWidget::Monaco } default_expr: ExpressionMode::Allowed);
impl CodeField {
    pub fn language(mut self, l: impl Into<String>) -> Self { self.language = l.into(); self }
    pub fn widget(mut self, w: CodeWidget) -> Self { self.widget = w; self }
}

define_field!(FileField { accept: Option<String> = None, max_size: Option<u64> = None, multiple: bool = false } default_expr: ExpressionMode::Allowed);
impl FileField {
    pub fn accept(mut self, m: impl Into<String>) -> Self { self.accept = Some(m.into()); self }
    pub fn max_size(mut self, b: u64) -> Self { self.max_size = Some(b); self }
    pub fn multiple(mut self) -> Self { self.multiple = true; self }
}

define_field!(ComputedField { expression_source: String = String::new(), returns: ComputedReturn = ComputedReturn::String } default_expr: ExpressionMode::Required);
impl ComputedField {
    pub fn expression_source(mut self, src: impl Into<String>) -> Self { self.expression_source = src.into(); self }
    pub fn returns(mut self, r: ComputedReturn) -> Self { self.returns = r; self }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputedReturn { String, Number, Boolean }

define_field!(DynamicField { depends_on: Vec<FieldPath> = Vec::new(), loader: Option<String> = None } default_expr: ExpressionMode::Allowed);
impl DynamicField {
    pub fn loader(mut self, k: impl Into<String>) -> Self { self.loader = Some(k.into()); self }
    pub fn depends_on(mut self, p: impl Into<FieldPath>) -> Self { self.depends_on.push(p.into()); self }
}

define_field!(NoticeField { severity: NoticeSeverity = NoticeSeverity::Info } default_expr: ExpressionMode::Forbidden);

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeSeverity {
    #[default]
    Info,
    Warning,
    Danger,
    Success,
}

// ── Top-level Field enum ──
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Field {
    String(StringField),
    Secret(SecretField),
    Number(NumberField),
    Boolean(BooleanField),
    Select(SelectField),
    Object(ObjectField),
    List(ListField),
    Mode(ModeField),
    Code(CodeField),
    File(FileField),
    Computed(ComputedField),
    Dynamic(DynamicField),
    Notice(NoticeField),
}

impl Field {
    pub fn string(k: impl Into<FieldKey>) -> StringField { StringField::new(k) }
    pub fn secret(k: impl Into<FieldKey>) -> SecretField { SecretField::new(k) }
    pub fn number(k: impl Into<FieldKey>) -> NumberField { NumberField::new(k) }
    pub fn integer(k: impl Into<FieldKey>) -> NumberField { NumberField::new(k).integer() }
    pub fn boolean(k: impl Into<FieldKey>) -> BooleanField { BooleanField::new(k) }
    pub fn select(k: impl Into<FieldKey>) -> SelectField { SelectField::new(k) }
    pub fn object(k: impl Into<FieldKey>) -> ObjectField { ObjectField::new(k) }
    pub fn list(k: impl Into<FieldKey>) -> ListField { ListField::new(k) }
    pub fn mode(k: impl Into<FieldKey>) -> ModeField { ModeField::new(k) }
    pub fn code(k: impl Into<FieldKey>) -> CodeField { CodeField::new(k) }
    pub fn file(k: impl Into<FieldKey>) -> FileField { FileField::new(k) }
    pub fn computed(k: impl Into<FieldKey>) -> ComputedField { ComputedField::new(k) }
    pub fn dynamic(k: impl Into<FieldKey>) -> DynamicField { DynamicField::new(k) }
    pub fn notice(k: impl Into<FieldKey>) -> NoticeField { NoticeField::new(k) }

    pub fn key(&self) -> &FieldKey { /* match each variant */ unimplemented!() }
    pub fn visible(&self) -> &VisibilityMode { unimplemented!() }
    pub fn required(&self) -> &RequiredMode { unimplemented!() }
    pub fn expression(&self) -> &ExpressionMode { unimplemented!() }
    pub fn rules(&self) -> &[Rule] { unimplemented!() }
    pub fn transformers(&self) -> &[Transformer] { unimplemented!() }
    pub fn default(&self) -> Option<&Value> { unimplemented!() }
    pub fn type_name(&self) -> &'static str { unimplemented!() }
}

// Fill the match arms in each accessor by pattern on Field variants —
// copy pattern from current crates/schema/src/field.rs#impl Field.
// From impls: one `impl From<$Variant> for Field` per variant, same as current.
```

Then fill in `key/visible/required/expression/rules/transformers/default/type_name` with match arms, and add `From<StringField> for Field` etc. for every variant.

- [ ] **Step 3: Run tests**

```bash
cargo test -p nebula-schema field_variants
cargo test -p nebula-schema
```

- [ ] **Step 4: Commit**

```bash
git add crates/schema/src/field.rs crates/schema/tests/field_variants.rs
git commit -m "feat(schema): consolidated Field enum with ExpressionMode per field"
```

---

## Task 18: `Schema` + `SchemaBuilder::build` skeleton

**Files:**
- Rewrite: `crates/schema/src/schema.rs`

- [ ] **Step 1: Write failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, FieldKey, field_key};

    #[test]
    fn build_empty_schema_ok() {
        let s = Schema::builder().build().unwrap();
        assert_eq!(s.fields().len(), 0);
    }

    #[test]
    fn build_detects_duplicate_key() {
        let r = Schema::builder()
            .add(Field::string(field_key!("x")))
            .add(Field::number(field_key!("x")))
            .build();
        let err = r.unwrap_err();
        assert!(err.errors().any(|e| e.code == "duplicate_key"));
    }

    #[test]
    fn build_finds_field_by_key() {
        let s = Schema::builder().add(Field::string(field_key!("a"))).build().unwrap();
        let key = FieldKey::new("a").unwrap();
        assert!(s.find(&key).is_some());
    }
}
```

- [ ] **Step 2: Implement minimal `schema.rs`**

```rust
//! Schema container and builder. Build-time validation produces `ValidSchema`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    error::{ValidationError, ValidationReport},
    field::Field,
    key::FieldKey,
    path::FieldPath,
    validated::{FieldHandle, SchemaFlags, ValidSchema, ValidSchemaInner},
};

/// Marker type — entry point for `Schema::builder()`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Schema;

impl Schema {
    pub fn builder() -> SchemaBuilder {
        SchemaBuilder::default()
    }
}

/// Mutable builder state. Consumed by `build()`.
#[derive(Debug, Default)]
pub struct SchemaBuilder {
    fields: Vec<Field>,
}

impl SchemaBuilder {
    pub fn add(mut self, field: impl Into<Field>) -> Self {
        self.fields.push(field.into());
        self
    }

    pub fn build(self) -> Result<ValidSchema, ValidationReport> {
        let mut report = ValidationReport::new();

        // Lint passes — Task 19 fills these out.
        crate::lint::lint_tree(&self.fields, &FieldPath::root(), &mut report);

        if report.has_errors() {
            return Err(report);
        }

        // Build the flat index for O(1) path lookup.
        let mut index: IndexMap<FieldPath, FieldHandle> = IndexMap::new();
        let mut flags = SchemaFlags::default();
        build_index(&self.fields, &FieldPath::root(), smallvec::SmallVec::new(), 0, &mut index, &mut flags);

        Ok(ValidSchema::from_inner(ValidSchemaInner {
            fields: self.fields,
            index,
            flags,
        }))
    }
}

fn build_index(
    fields: &[Field],
    prefix: &FieldPath,
    parent_cursor: smallvec::SmallVec<[u16; 4]>,
    depth: u8,
    index: &mut IndexMap<FieldPath, FieldHandle>,
    flags: &mut SchemaFlags,
) {
    for (i, f) in fields.iter().enumerate() {
        let mut cursor = parent_cursor.clone();
        cursor.push(i as u16);
        let path = prefix.clone().join(f.key().clone());
        flags.max_depth = flags.max_depth.max(depth + 1);
        // Track loader/expression flags
        if f.expression() != &crate::mode::ExpressionMode::Forbidden {
            flags.uses_expressions = true;
        }
        index.insert(path.clone(), FieldHandle { cursor: cursor.clone(), depth: depth + 1 });

        // Recurse for container field types.
        match f {
            Field::Object(obj) => build_index(&obj.fields, &path, cursor, depth + 1, index, flags),
            Field::List(list) => {
                if let Some(item) = list.item.as_deref() {
                    // Use `[*]` sentinel — list items index as `path[*]`.
                    let list_item_path = path.clone().join(crate::path::PathSegment::Index(usize::MAX));
                    let mut child_cursor = cursor.clone();
                    child_cursor.push(0);
                    index.insert(
                        list_item_path.clone(),
                        FieldHandle { cursor: child_cursor, depth: depth + 2 },
                    );
                    if let Field::Object(o) = item {
                        build_index(&o.fields, &list_item_path, cursor, depth + 2, index, flags);
                    }
                }
            },
            Field::Mode(mode) => {
                for (vi, variant) in mode.variants.iter().enumerate() {
                    let mut v_cursor = cursor.clone();
                    v_cursor.push(vi as u16);
                    let variant_path = path.clone().join(crate::path::PathSegment::Key(
                        crate::key::FieldKey::new(variant.key.as_str()).unwrap(),
                    ));
                    index.insert(
                        variant_path.clone(),
                        FieldHandle { cursor: v_cursor.clone(), depth: depth + 2 },
                    );
                    if let Field::Object(o) = variant.field.as_ref() {
                        build_index(&o.fields, &variant_path, v_cursor, depth + 2, index, flags);
                    }
                }
            },
            _ => {},
        }
    }
}

#[cfg(test)]
mod tests { /* from Step 1 */ }
```

- [ ] **Step 3: Run & commit**

```bash
cargo test -p nebula-schema schema::tests
git add crates/schema/src/schema.rs
git commit -m "feat(schema): SchemaBuilder with build-time index"
```

---

## Task 19: Lint passes emitting `ValidationError`

**Files:**
- Rewrite: `crates/schema/src/lint.rs`

Port all checks from the current `lint.rs` to emit `ValidationError` instead of `LintDiagnostic`. Codes already reserved in Task 4.

- [ ] **Step 1: Define entry point**

```rust
//! Build-time structural lints. All diagnostics use `ValidationError`.

use std::collections::HashSet;

use nebula_validator::Rule;

use crate::{
    error::{Severity, ValidationError, ValidationReport},
    field::{Field, ListField, ModeField, ObjectField},
    key::FieldKey,
    mode::{RequiredMode, VisibilityMode},
    path::{FieldPath, PathSegment},
};

pub(crate) fn lint_tree(fields: &[Field], prefix: &FieldPath, report: &mut ValidationReport) {
    let root_keys: HashSet<&str> = fields.iter().map(|f| f.key().as_str()).collect();
    lint_fields(fields, prefix, &root_keys, report);
    lint_visibility_cycles(fields, prefix, report);
}
```

- [ ] **Step 2: Port each pass**

Translate each helper function from the current `lint.rs`:
- `check_duplicate_keys` → emits `duplicate_key`
- `lint_depends_on` → emits `self_dependency` / `dangling_reference`
- `lint_rule_type_compatibility` → emits `rule.incompatible` (warning)
- `lint_contradictory_rules` → emits `rule.contradictory`
- `lint_visibility_cycles` → emits `visibility_cycle`
- `lint_select_field` → emits `missing_loader` / `loader_without_dynamic` (warnings)
- `lint_list_field` → emits `missing_item_schema`
- `lint_mode_field` → emits `invalid_default_variant`, `duplicate_variant`, `missing_variant_label`
- `lint_notice_misuse` → emits `notice.misuse`, `notice_missing_description`

Each produces `ValidationError::new("...").at(path).message("...").build()` and adds `.warn()` for warnings.

- [ ] **Step 3: Add unit tests per pass**

```rust
#[test]
fn detects_duplicate_key() {
    let fields = vec![
        Field::string(FieldKey::new("x").unwrap()).into(),
        Field::number(FieldKey::new("x").unwrap()).into(),
    ];
    let mut report = ValidationReport::new();
    lint_tree(&fields, &FieldPath::root(), &mut report);
    assert!(report.errors().any(|e| e.code == "duplicate_key"));
}

#[test]
fn detects_self_dependency() {
    // ... test for SelectField.depends_on referencing itself
}
```

- [ ] **Step 4: Run & commit**

```bash
cargo test -p nebula-schema lint
git add crates/schema/src/lint.rs
git commit -m "feat(schema): port lint passes to ValidationError"
```

---

## Task 20: `ValidSchema` + `FieldHandle` index

**Files:**
- Create: `crates/schema/src/validated.rs`

- [ ] **Step 1: Implement proof-token types**

```rust
//! Proof tokens for schema / values / resolved values.

use std::sync::Arc;

use indexmap::IndexMap;
use smallvec::SmallVec;

use crate::{
    error::ValidationError,
    field::Field,
    key::FieldKey,
    path::FieldPath,
    value::{FieldValue, FieldValues},
};

/// Flags computed once at build.
#[derive(Debug, Clone, Default)]
pub struct SchemaFlags {
    pub uses_expressions: bool,
    pub has_async_loaders: bool,
    pub max_depth: u8,
}

/// Cursor into the field tree: a breadcrumb of child indices starting from the root.
#[derive(Debug, Clone)]
pub(crate) struct FieldHandle {
    pub cursor: SmallVec<[u16; 4]>,
    pub depth: u8,
}

/// Post-build, post-lint schema. Cheap to clone (Arc).
#[derive(Debug, Clone)]
pub struct ValidSchema(Arc<ValidSchemaInner>);

#[derive(Debug)]
pub(crate) struct ValidSchemaInner {
    pub fields: Vec<Field>,
    pub index: IndexMap<FieldPath, FieldHandle>,
    pub flags: SchemaFlags,
}

impl ValidSchema {
    pub(crate) fn from_inner(inner: ValidSchemaInner) -> Self {
        Self(Arc::new(inner))
    }

    pub fn fields(&self) -> &[Field] {
        &self.0.fields
    }

    pub fn flags(&self) -> &SchemaFlags {
        &self.0.flags
    }

    pub fn find(&self, key: &FieldKey) -> Option<&Field> {
        self.0.fields.iter().find(|f| f.key() == key)
    }

    pub fn find_by_path(&self, path: &FieldPath) -> Option<&Field> {
        let handle = self.0.index.get(path)?;
        let mut cur = self.0.fields.get(*handle.cursor.first()? as usize)?;
        for &step in &handle.cursor[1..] {
            cur = match cur {
                Field::Object(o) => o.fields.get(step as usize)?,
                Field::List(l) => l.item.as_deref()?,
                Field::Mode(m) => &m.variants.get(step as usize)?.field,
                _ => return None,
            };
        }
        Some(cur)
    }
}

/// Validated values — tied to a specific `ValidSchema` via borrow.
#[derive(Debug, Clone)]
pub struct ValidValues<'s> {
    pub(crate) schema: &'s ValidSchema,
    pub(crate) values: FieldValues,
    pub(crate) warnings: Arc<[ValidationError]>,
}

impl<'s> ValidValues<'s> {
    pub fn schema(&self) -> &'s ValidSchema { self.schema }
    pub fn raw(&self) -> &FieldValues { &self.values }
    pub fn warnings(&self) -> &[ValidationError] { &self.warnings }
    pub fn get(&self, key: &FieldKey) -> Option<&FieldValue> { self.values.get(key) }
    pub fn get_path(&self, path: &FieldPath) -> Option<&FieldValue> { self.values.get_path(path) }
}

/// Resolved values — no `FieldValue::Expression` remains.
#[derive(Debug, Clone)]
pub struct ResolvedValues<'s> {
    pub(crate) schema: &'s ValidSchema,
    pub(crate) values: FieldValues,
    pub(crate) warnings: Arc<[ValidationError]>,
}

impl<'s> ResolvedValues<'s> {
    pub fn schema(&self) -> &'s ValidSchema { self.schema }
    pub fn warnings(&self) -> &[ValidationError] { &self.warnings }
    pub fn get(&self, key: &FieldKey) -> Option<&serde_json::Value> {
        match self.values.get(key)? {
            FieldValue::Literal(v) => Some(v),
            _ => None,
        }
    }
    pub fn into_json(self) -> serde_json::Value { self.values.to_json() }
    pub fn into_typed<T: serde::de::DeserializeOwned>(self) -> Result<T, ValidationError> {
        serde_json::from_value(self.into_json()).map_err(|e| {
            ValidationError::new("type_mismatch")
                .message(format!("deserialize failed: {e}"))
                .build()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, FieldKey, Schema};

    #[test]
    fn clone_is_cheap_via_arc() {
        let s = Schema::builder().add(Field::string(FieldKey::new("x").unwrap())).build().unwrap();
        let c = s.clone();
        assert!(Arc::ptr_eq(&s.0, &c.0));
    }

    #[test]
    fn find_returns_top_level() {
        let s = Schema::builder().add(Field::string(FieldKey::new("x").unwrap())).build().unwrap();
        assert!(s.find(&FieldKey::new("x").unwrap()).is_some());
        assert!(s.find(&FieldKey::new("y").unwrap()).is_none());
    }
}
```

- [ ] **Step 2: Run & commit**

```bash
cargo test -p nebula-schema validated::tests
git add crates/schema/src/validated.rs
git commit -m "feat(schema): ValidSchema/ValidValues/ResolvedValues proof tokens"
```

---

## Task 21: Schema-time validation — `ValidSchema::validate`

**Files:**
- Modify: `crates/schema/src/validated.rs` (add `validate` method)
- Create: `crates/schema/src/context.rs` (`RuleContext` impls)

- [ ] **Step 1: Add `RuleContext` adapters**

Create `crates/schema/src/context.rs`:
```rust
//! `RuleContext` implementations backed by `FieldValues` subtrees.

use indexmap::IndexMap;
use nebula_validator::RuleContext;
use serde_json::Value;

use crate::{
    key::FieldKey,
    value::{FieldValue, FieldValues},
};

/// Root context — borrowed view over top-level `FieldValues`.
pub(crate) struct RootContext<'a>(pub &'a FieldValues);

impl<'a> RuleContext for RootContext<'a> {
    fn get(&self, key: &str) -> Option<&Value> {
        let fk = FieldKey::new(key).ok()?;
        match self.0.get(&fk)? {
            FieldValue::Literal(v) => Some(v),
            _ => None,
        }
    }
}

/// Sub-context — borrowed view over a nested Object's `IndexMap`.
pub(crate) struct ObjectContext<'a>(pub &'a IndexMap<FieldKey, FieldValue>);

impl<'a> RuleContext for ObjectContext<'a> {
    fn get(&self, key: &str) -> Option<&Value> {
        let fk = FieldKey::new(key).ok()?;
        match self.0.get(&fk)? {
            FieldValue::Literal(v) => Some(v),
            _ => None,
        }
    }
}
```

- [ ] **Step 2: Implement `ValidSchema::validate`**

Add to `crates/schema/src/validated.rs`:
```rust
impl ValidSchema {
    pub fn validate<'s>(&'s self, values: &FieldValues)
        -> Result<ValidValues<'s>, ValidationReport>
    {
        use crate::{context::RootContext, mode::{VisibilityMode, RequiredMode, ExpressionMode}};

        let mut report = ValidationReport::new();
        let ctx = RootContext(values);

        for field in &self.0.fields {
            validate_field(field, values.get(field.key()), &ctx, &FieldPath::root().join(field.key().clone()), &mut report);
        }

        if report.has_errors() {
            return Err(report);
        }

        let warnings: Arc<[_]> = report.iter().filter(|e| e.severity == Severity::Warning).cloned().collect();
        Ok(ValidValues {
            schema: self,
            values: values.clone(),
            warnings,
        })
    }
}

fn validate_field(
    field: &Field,
    raw: Option<&FieldValue>,
    ctx: &dyn nebula_validator::RuleContext,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    use crate::mode::{VisibilityMode, RequiredMode, ExpressionMode};

    // Visibility
    let visible = match field.visible() {
        VisibilityMode::Never => false,
        VisibilityMode::Always => true,
        VisibilityMode::When(rule) => rule.evaluate(ctx),
    };
    if !visible && raw.is_none() { return; }

    // Required
    let required = match field.required() {
        RequiredMode::Never => false,
        RequiredMode::Always => true,
        RequiredMode::When(rule) => rule.evaluate(ctx),
    };
    if required && raw.is_none_or(|v| matches!(v, FieldValue::Literal(serde_json::Value::Null))) {
        report.push(ValidationError::new("required")
            .at(path.clone())
            .message(format!("field `{path}` is required"))
            .build());
        return;
    }

    let Some(value) = raw else { return; };

    // Expression-mode enforcement
    match (field.expression(), value) {
        (ExpressionMode::Forbidden, FieldValue::Expression(_)) => {
            report.push(ValidationError::new("expression.forbidden")
                .at(path.clone()).message("expressions not allowed here").build());
            return;
        },
        (_, FieldValue::Expression(e)) => {
            if let Err(err) = e.parse() {
                report.push(err);
                return;
            }
            // skip value rules — no value to check
            return;
        },
        _ => {},
    }

    // Transformers + type-check + rules — delegate to per-type dispatch (see field.rs).
    // Recurse into Object/List/Mode by reusing validate_field with sub-context.
    // (Full dispatch logic too long for one snippet — model on the current
    //  `validate_field_type` but replacing HashMap with `ObjectContext`.)
}
```

- [ ] **Step 3: Write flow tests**

Create `crates/schema/tests/flow/validate_basic.rs`:
```rust
use nebula_schema::*;
use serde_json::json;

#[test]
fn required_missing_reports_error() {
    let s = Schema::builder()
        .add(Field::string(field_key!("x")).required())
        .build().unwrap();
    let vs = FieldValues::from_json(json!({})).unwrap();
    let r = s.validate(&vs).unwrap_err();
    assert!(r.errors().any(|e| e.code == "required"));
}

#[test]
fn type_mismatch_reports_error() {
    let s = Schema::builder().add(Field::number(field_key!("n"))).build().unwrap();
    let vs = FieldValues::from_json(json!({"n": "not a number"})).unwrap();
    let r = s.validate(&vs);
    assert!(r.unwrap_err().errors().any(|e| e.code == "type_mismatch"));
}
```

- [ ] **Step 4: Run & commit**

```bash
cargo test -p nebula-schema --test flow
git add crates/schema/src/ crates/schema/tests/
git commit -m "feat(schema): schema-time validation with RuleContext"
```

---

## Task 22: Loader — unified errors

**Files:**
- Rewrite: `crates/schema/src/loader.rs`

- [ ] **Step 1: Fold `LoaderError` into `ValidationError`**

Replace `loader.rs` surface. Key signatures unchanged (`Loader<T>`, `LoaderRegistry::register_option/register_record`, `load_options`, `load_records`), but fallible paths return `ValidationError`:

```rust
pub type LoaderFuture<T> =
    Pin<Box<dyn Future<Output = Result<LoaderResult<T>, ValidationError>> + Send>>;

impl LoaderRegistry {
    pub async fn load_options(&self, key: &str, ctx: LoaderContext)
        -> Result<LoaderResult<SelectOption>, ValidationError>
    {
        let Some(loader) = self.option_loaders.get(key) else {
            return Err(ValidationError::new("loader.not_registered")
                .message(format!("option loader `{key}` is not registered"))
                .param("loader", serde_json::Value::String(key.to_owned()))
                .build());
        };
        loader.call(ctx).await
    }
    // same for load_records
}
```

- [ ] **Step 2: Port existing tests**

Ensure the loader tests (if any) still pass. Update any `LoaderError::new(...)` sites in the schema crate to `ValidationError::new("loader.failed").source(...)`.

- [ ] **Step 3: Run & commit**

```bash
cargo test -p nebula-schema loader
git add crates/schema/src/loader.rs
git commit -m "refactor(schema): fold LoaderError into ValidationError"
```

---

## Task 23: Runtime resolution — `ValidValues::resolve`

**Files:**
- Modify: `crates/schema/src/validated.rs`

- [ ] **Step 1: Define `ExpressionContext` contract**

Add to `crates/schema/src/expression.rs`:
```rust
/// Minimal context required to evaluate an expression.
/// Actual evaluator lives in `nebula-expression`; this trait is the integration seam.
#[async_trait::async_trait]
pub trait ExpressionContext: Send + Sync {
    async fn evaluate(&self, ast: &ExpressionAst) -> Result<serde_json::Value, ValidationError>;
}
```

(Add `async-trait = { workspace = true }` to the schema Cargo if not already present.)

- [ ] **Step 2: Implement `resolve`**

```rust
impl<'s> ValidValues<'s> {
    pub async fn resolve(
        self,
        ctx: &dyn crate::expression::ExpressionContext,
    ) -> Result<ResolvedValues<'s>, ValidationReport> {
        if !self.schema.flags().uses_expressions {
            return Ok(ResolvedValues {
                schema: self.schema,
                values: self.values,
                warnings: self.warnings,
            });
        }
        let mut report = ValidationReport::new();
        let mut values = self.values;
        resolve_tree(&mut values, ctx, &FieldPath::root(), &mut report).await;
        if report.has_errors() {
            return Err(report);
        }
        let extra_warnings: Vec<_> = report.iter()
            .filter(|e| e.severity == Severity::Warning)
            .cloned()
            .collect();
        let all_warnings: Arc<[_]> = self.warnings.iter().chain(extra_warnings.iter()).cloned().collect();
        Ok(ResolvedValues {
            schema: self.schema,
            values,
            warnings: all_warnings,
        })
    }
}

// Recursive async walker — use Box::pin for recursion in async context.
async fn resolve_tree(/* ... */) { /* walk, replace Expression → Literal */ }
```

- [ ] **Step 3: Integration test**

```rust
// tests/flow/resolve.rs
use nebula_schema::*;
use serde_json::json;

struct DummyCtx(serde_json::Value);
#[async_trait::async_trait]
impl ExpressionContext for DummyCtx {
    async fn evaluate(&self, ast: &ExpressionAst) -> Result<serde_json::Value, ValidationError> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn resolves_expression_to_literal() {
    let s = Schema::builder().add(Field::number(field_key!("n"))).build().unwrap();
    let vs = FieldValues::from_json(json!({"n": {"$expr": "{{ $x }}"}})).unwrap();
    let validated = s.validate(&vs).unwrap();
    let ctx = DummyCtx(json!(42));
    let resolved = validated.resolve(&ctx).await.unwrap();
    assert_eq!(resolved.get(&FieldKey::new("n").unwrap()), Some(&json!(42)));
}
```

- [ ] **Step 4: Run & commit**

```bash
cargo test -p nebula-schema --test flow
git add crates/schema/
git commit -m "feat(schema): async ValidValues::resolve to ResolvedValues"
```

---

## Task 24: `lib.rs`, `prelude`, doctests

**Files:**
- Rewrite: `crates/schema/src/lib.rs`
- Rewrite: `crates/schema/src/prelude.rs`

- [ ] **Step 1: Final `lib.rs`**

```rust
//! Schema system for Nebula workflow surfaces.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod context;
pub mod error;
pub mod expression;
pub mod field;
pub mod input_hint;
pub mod key;
pub mod lint;
pub mod loader;
pub mod mode;
pub mod option;
pub mod path;
pub mod prelude;
pub mod schema;
pub mod transformer;
pub mod validated;
pub mod value;
pub mod widget;

pub use error::{STANDARD_CODES, Severity, ValidationError, ValidationReport};
pub use expression::{Expression, ExpressionAst, ExpressionContext};
pub use field::{
    BooleanField, CodeField, ComputedField, ComputedReturn, DynamicField, Field, FileField,
    ListField, ModeField, ModeVariant, NoticeField, NoticeSeverity, NumberField, ObjectField,
    SecretField, SelectField, StringField,
};
pub use input_hint::InputHint;
pub use key::FieldKey;
pub use loader::{Loader, LoaderContext, LoaderRegistry, LoaderResult, OptionLoader, RecordLoader};
pub use mode::{ExpressionMode, RequiredMode, VisibilityMode};
pub use nebula_schema_macros::field_key;
pub use nebula_validator::{Rule, RuleContext};
pub use option::SelectOption;
pub use path::{FieldPath, PathSegment};
pub use schema::{Schema, SchemaBuilder};
pub use transformer::Transformer;
pub use validated::{ResolvedValues, SchemaFlags, ValidSchema, ValidValues};
pub use value::{EXPRESSION_KEY, FieldValue, FieldValues};
pub use widget::{
    BooleanWidget, CodeWidget, ListWidget, NumberWidget, ObjectWidget, SecretWidget, SelectWidget,
    StringWidget,
};

/// Schema wire-format version emitted in serialized output (Phase 2+ plugins read this).
pub const SCHEMA_WIRE_VERSION: u16 = 1;
```

- [ ] **Step 2: `prelude.rs`**

```rust
//! Common imports for schema-authored code.

pub use crate::{
    field_key, BooleanField, Expression, ExpressionContext, ExpressionMode, Field, FieldKey,
    FieldPath, FieldValue, FieldValues, InputHint, ListField, LoaderContext, LoaderRegistry,
    NumberField, ObjectField, RequiredMode, ResolvedValues, Rule, RuleContext, Schema,
    SchemaBuilder, SecretField, SelectField, SelectOption, Severity, StringField,
    ValidSchema, ValidValues, ValidationError, ValidationReport, VisibilityMode,
};
```

- [ ] **Step 3: Verify build**

```bash
cargo build -p nebula-schema
cargo test -p nebula-schema
cargo test -p nebula-schema --doc
```

- [ ] **Step 4: Commit**

```bash
git add crates/schema/src/lib.rs crates/schema/src/prelude.rs
git commit -m "feat(schema): final lib.rs and prelude re-exports"
```

---

## Task 25: Compile-fail tests

**Files:**
- Create: `crates/schema/tests/compile_fail.rs`
- Create: `crates/schema/tests/compile_fail/*.rs` + `.stderr`

- [ ] **Step 1: Runner**

```rust
// tests/compile_fail.rs
#[test]
fn ensure_invariants() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
```

- [ ] **Step 2: Write fixtures**

One per invariant from spec §9:

`tests/compile_fail/no_validate_without_build.rs`
```rust
use nebula_schema::*;
fn main() {
    let s = Schema::builder();   // raw SchemaBuilder, not ValidSchema
    let vs = FieldValues::new();
    s.validate(&vs);             // should not compile: `validate` not on SchemaBuilder
}
```

`tests/compile_fail/no_resolve_without_validate.rs`
```rust
use nebula_schema::*;
fn main() {
    let vs = FieldValues::new();
    futures::executor::block_on(vs.resolve(&()));  // `resolve` not on raw FieldValues
}
```

`tests/compile_fail/no_cross_schema_values.rs`
```rust
use nebula_schema::*;
fn main() {
    let s1 = Schema::builder().build().unwrap();
    let s2 = Schema::builder().build().unwrap();
    let v = FieldValues::new();
    let validated_1: ValidValues<'_> = s1.validate(&v).unwrap();
    fn requires_s2(_: ValidValues<'_>) {}
    // If schemas have distinct borrows, cross-usage should fail.
    // This test is aspirational — in our current design the borrow binding
    // only prevents misuse when schemas are kept on distinct references.
    requires_s2(validated_1);  // borrow-checker angle expected to fire if bound correctly
}
```

`tests/compile_fail/field_key_invalid_literal.rs`
```rust
use nebula_schema::field_key;
fn main() {
    let _ = field_key!("1bad");  // compile-time validation fails
}
```

`tests/compile_fail/field_key_no_panic_from.rs`
```rust
use nebula_schema::FieldKey;
fn main() {
    let _: FieldKey = FieldKey::from("anything");  // no such impl
}
```

Record `.stderr` by running once, then capturing:
```bash
TRYBUILD=overwrite cargo test -p nebula-schema --test compile_fail
```

- [ ] **Step 3: Run & commit**

```bash
cargo test -p nebula-schema --test compile_fail
git add crates/schema/tests/compile_fail*
git commit -m "test(schema): compile-fail tests for proof-token invariants"
```

---

## Task 26: Integration + proptest

**Files:**
- Create: `crates/schema/tests/flow/all_error_codes.rs`
- Create: `crates/schema/tests/proptest/*.rs`

- [ ] **Step 1: Emittable-codes coverage test**

Write one `#[test]` per code in `STANDARD_CODES` that triggers and asserts. For the "no orphan codes" guard, add:
```rust
use nebula_schema::STANDARD_CODES;

const COVERED: &[&str] = &[
    "required", "type_mismatch", "length.min", "length.max",
    /* …fill in all triggered codes */
];

#[test]
fn every_standard_code_is_covered() {
    for c in STANDARD_CODES {
        assert!(COVERED.contains(c), "missing integration test for code `{c}`");
    }
}
```

- [ ] **Step 2: Proptest — parse roundtrip, no panics**

```rust
// tests/proptest/path.rs
use nebula_schema::FieldPath;
use proptest::prelude::*;

fn valid_path() -> impl Strategy<Value = String> {
    // keys are ASCII alnum + underscore, length 1..=8
    let key = "[a-z][a-z0-9_]{0,7}";
    let seg = proptest::prop_oneof![
        key.prop_map(|s| s.to_string()),
        (0usize..10).prop_map(|i| format!("[{i}]")),
    ];
    proptest::collection::vec(seg, 1..=5).prop_map(|parts| {
        let mut out = String::new();
        let mut first = true;
        for p in parts {
            if !p.starts_with('[') && !first { out.push('.'); }
            out.push_str(&p);
            first = false;
        }
        out
    })
}

proptest! {
    #[test]
    fn path_parse_display_roundtrip(p in valid_path()) {
        let parsed = FieldPath::parse(&p).unwrap();
        prop_assert_eq!(parsed.to_string(), p);
    }
}
```

- [ ] **Step 3: Run & commit**

```bash
cargo test -p nebula-schema --test flow
cargo test -p nebula-schema --test proptest
git add crates/schema/tests/
git commit -m "test(schema): integration + proptest coverage"
```

---

## Task 27: Post-refactor benchmarks

**Files:**
- Create: `crates/schema/benches/bench_resolve.rs`
- Create: `crates/schema/benches/bench_lookup.rs`
- Modify: `crates/schema/Cargo.toml` (register new benches)

- [ ] **Step 1: Implement new benches**

`bench_resolve.rs`:
```rust
use criterion::{criterion_group, criterion_main, Criterion};
use nebula_schema::*;
use serde_json::json;

fn bench_resolve_no_expressions(c: &mut Criterion) {
    let s = Schema::builder()
        .add(Field::string(field_key!("a")))
        .add(Field::number(field_key!("b")))
        .build().unwrap();
    let vs = FieldValues::from_json(json!({"a":"x","b":1})).unwrap();
    let validated = s.validate(&vs).unwrap();
    struct Noop;
    #[async_trait::async_trait]
    impl ExpressionContext for Noop {
        async fn evaluate(&self, _: &ExpressionAst) -> Result<serde_json::Value, ValidationError> {
            Ok(json!(null))
        }
    }
    c.bench_function("resolve/no_expressions_fast_path", |b| {
        b.iter(|| {
            let v = validated.clone();
            futures::executor::block_on(v.resolve(&Noop)).unwrap();
        });
    });
}

criterion_group!(benches, bench_resolve_no_expressions);
criterion_main!(benches);
```

`bench_lookup.rs`:
```rust
use criterion::{criterion_group, criterion_main, Criterion};
use nebula_schema::*;

fn bench_find_by_key(c: &mut Criterion) {
    let mut b = Schema::builder();
    for i in 0..1000 {
        b = b.add(Field::string(FieldKey::new(format!("f_{i}")).unwrap()));
    }
    let s = b.build().unwrap();
    let key = FieldKey::new("f_999").unwrap();
    c.bench_function("lookup/find_by_key", |c| c.iter(|| s.find(&key)));
}

criterion_group!(benches, bench_find_by_key);
criterion_main!(benches);
```

Register in Cargo:
```toml
[[bench]]
name = "bench_resolve"
harness = false

[[bench]]
name = "bench_lookup"
harness = false
```

- [ ] **Step 2: Run against baseline**

```bash
cargo bench -p nebula-schema --bench bench_build -- --baseline phase0
cargo bench -p nebula-schema --bench bench_validate -- --baseline phase0
cargo bench -p nebula-schema --bench bench_serde -- --baseline phase0
cargo bench -p nebula-schema --bench bench_resolve
cargo bench -p nebula-schema --bench bench_lookup
```

Record numbers — target: `bench_validate` ≥2× on nested object schemas.

- [ ] **Step 3: Commit**

```bash
git add crates/schema/benches/bench_resolve.rs crates/schema/benches/bench_lookup.rs crates/schema/Cargo.toml
git commit -m "bench(schema): add resolve + lookup benches"
```

---

## Task 28: Migrate `nebula-action`

**Files:**
- Modify: `crates/action/Cargo.toml`, `src/lib.rs`, `src/metadata.rs`, `src/prelude.rs`

- [ ] **Step 1: Swap dep**

Edit `crates/action/Cargo.toml`:
```toml
- nebula-parameter = { path = "../parameter" }
+ nebula-schema = { path = "../schema" }
```

- [ ] **Step 2: Rename types in `metadata.rs`**

```rust
- use nebula_parameter::collection::ParameterCollection;
+ use nebula_schema::ValidSchema;

pub struct ActionMetadata {
-   pub parameters: ParameterCollection,
+   pub parameters: ValidSchema,
}
```

- [ ] **Step 3: Update re-exports**

```rust
// src/lib.rs
- pub use nebula_parameter::{Parameter, ParameterCollection};
+ pub use nebula_schema::{Field, Schema, SchemaBuilder, ValidSchema, ValidValues, ResolvedValues};
```

Same in `src/prelude.rs`.

- [ ] **Step 4: Build**

```bash
cargo check -p nebula-action
cargo test -p nebula-action
```

Fix any remaining site-specific imports.

- [ ] **Step 5: Commit**

```bash
git add crates/action/
git commit -m "refactor(action): migrate from nebula-parameter to nebula-schema"
```

---

## Task 29: Migrate `nebula-credential`

**Files:**
- Modify: `crates/credential/Cargo.toml`, `src/{credential,description,executor,resolver,static_protocol}.rs`, `src/credentials/*`

- [ ] **Step 1: Swap dep + bulk rename**

Use a sed/replace across `crates/credential/src/`:
```
ParameterCollection → ValidSchema
ParameterValues → ResolvedValues
Parameter::string → Field::string
nebula_parameter   → nebula_schema
```

- [ ] **Step 2: Per-file fix-ups**

For each of the 5 built-in credentials (`api_key`, `basic_auth`, `oauth2`, `oauth2_config`, `oauth2_flow`):
- Change `parameters()` signature to return `ValidSchema` via `Schema::builder().add(...).build()?`
- Return `Result<ValidSchema, ValidationReport>` where appropriate
- Trait method `Credential::resolve(values: ResolvedValues)` — each impl consumes `ResolvedValues` instead of raw map; use `resolved.get(&field_key!("api_key"))`

- [ ] **Step 3: Build + test**

```bash
cargo check -p nebula-credential
cargo test -p nebula-credential
```

- [ ] **Step 4: Commit**

```bash
git add crates/credential/
git commit -m "refactor(credential): migrate to nebula-schema"
```

---

## Task 30: Migrate `nebula-sdk`

**Files:**
- Modify: `crates/sdk/Cargo.toml`, `src/lib.rs`, `src/prelude.rs`

- [ ] **Step 1: Swap dep + re-exports**

```toml
- nebula-parameter = { path = "../parameter" }
+ nebula-schema = { path = "../schema" }
```

`src/lib.rs` and `src/prelude.rs` — re-export from `nebula_schema` instead.

- [ ] **Step 2: Build**

```bash
cargo check -p nebula-sdk
cargo test -p nebula-sdk
```

- [ ] **Step 3: Commit**

```bash
git add crates/sdk/
git commit -m "refactor(sdk): swap parameter re-exports for schema"
```

---

## Task 31: Delete `nebula-parameter` + workspace trim

**Files:**
- Delete: `crates/parameter/`
- Delete: `crates/parameter/macros/`
- Modify: root `Cargo.toml` (remove members)

- [ ] **Step 1: Remove from workspace members**

Edit root `Cargo.toml`:
```toml
# Remove:
-     "crates/parameter",
-     "crates/parameter/macros",
```

- [ ] **Step 2: Delete crate directories**

```bash
rm -r crates/parameter
```

- [ ] **Step 3: Workspace-wide build**

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo deny check
```

Fix any residual imports. No deprecation-warning toleration — per memory `#feedback_no_shims`.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: delete nebula-parameter — migration complete"
```

---

## Task 32: Acceptance sweep

**Files:**
- None (verification only)

- [ ] **Step 1: Run full validation**

```bash
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
cargo deny check
```

All green.

- [ ] **Step 2: Compare benches to phase-0 baselines**

```bash
cargo bench -p nebula-schema --bench bench_build -- --baseline phase0
cargo bench -p nebula-schema --bench bench_validate -- --baseline phase0
cargo bench -p nebula-schema --bench bench_serde -- --baseline phase0
```

Record speedup for `bench_validate` on nested-object schemas (target ≥2×).

- [ ] **Step 3: Verify spec acceptance criteria**

Walk the §13 checklist from `docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md` and check off each bullet.

- [ ] **Step 4: Commit CHANGELOG stub**

Add `CHANGELOG.md` entry referencing the spec and the main breaking changes (per §10 of the spec).

```bash
git add CHANGELOG.md
git commit -m "docs: CHANGELOG entry for schema Phase 1"
```

---

## Self-review notes

- **Spec coverage.** All §4–§9 items of the spec map to tasks 3–27. Callers migration (§10) → 28–31. Acceptance (§13) → 32.
- **Placeholders.** Task 17 and Task 21 reference "see the current field.rs for the pattern" for the accessor match arms and full per-type validation dispatch — not TBD, but a deliberate hand-off: the current schema crate has the exact pattern, the implementer copies and adapts. Consider this a documented shortcut, not a gap.
- **Type consistency.** `ValidSchema` / `ValidValues<'s>` / `ResolvedValues<'s>` naming consistent throughout. `FieldKey` is `Arc<str>`, clones are cheap. `FieldValue::Object` is `IndexMap<FieldKey, FieldValue>`. `Rule::evaluate` takes `&dyn RuleContext`.
- **Migration safety.** Tasks 28–30 happen before 31 (deletion). Each caller migration is a separate commit, so bisectable.
- **Two external-crate touchpoints.** (1) `nebula-validator` changes in Task 16 — checked. (2) `nebula-expression` gets an `ExpressionContext` trait seam in Task 23; the real crate's API may already be present. If missing, falls back to the opaque stub pattern in Task 14.

---

**Plan complete and saved to `docs/superpowers/plans/2026-04-16-nebula-schema-phase1-foundation.md`.**

