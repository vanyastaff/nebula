# nebula-metadata Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract generic compat rules from `ActionMetadata` into `nebula-metadata::compat`, wire action/credential/resource to the shared checks, and document the crate (README, MATURITY row, ADR-0018 for the future plugin→manifest migration).

**Architecture:** `BaseCompatError<K>` + `validate_base_compat` live in `nebula-metadata`; each consumer wraps the shared error in its own thin enum (or delegates directly when no entity-specific rules exist). `PluginManifest` reshape stays docs-only (ADR-0018).

**Tech Stack:** Rust 2024, `thiserror`, `semver`, `cargo nextest`, workspace clippy gate.

**Spec:** [`docs/superpowers/specs/2026-04-19-nebula-metadata-completion-design.md`](../specs/2026-04-19-nebula-metadata-completion-design.md)

---

## File Structure

| Path | Change | Responsibility |
|---|---|---|
| `crates/metadata/Cargo.toml` | Modify | Add `thiserror` workspace dep. |
| `crates/metadata/src/compat.rs` | Modify | Replace stub with `BaseCompatError<K>` + `validate_base_compat` + tests. |
| `crates/metadata/src/lib.rs` | Modify | Re-export `BaseCompatError` and `validate_base_compat`. |
| `crates/action/src/metadata.rs` | Modify | Convert `MetadataCompatibilityError` into wrapper over `BaseCompatError<ActionKey>` + action-specific `PortsChangeWithoutMajorBump`. Update affected tests. |
| `crates/credential/src/metadata.rs` | Modify | New `MetadataCompatibilityError` (wraps base + `PatternChangeWithoutMajorBump`) + `validate_compatibility`. |
| `crates/resource/src/resource.rs` | Modify | New `ResourceMetadata::validate_compatibility` (direct delegation). |
| `crates/metadata/README.md` | Create | Workspace-style crate README (purpose / core types / composition / consumers / crosslinks). |
| `docs/MATURITY.md` | Modify | Insert `nebula-metadata` row; bump `Last targeted revision`. |
| `docs/adr/0018-plugin-metadata-to-manifest.md` | Create | ADR capturing the `PluginMetadata` → `PluginManifest` decision (docs-only). |
| `docs/adr/README.md` | Modify | Add `0018` row to the ADR index. |

---

## Task 1: Compat primitives in `nebula-metadata`

**Files:**
- Modify: `crates/metadata/Cargo.toml`
- Modify: `crates/metadata/src/compat.rs` (currently doc-only stub)
- Modify: `crates/metadata/src/lib.rs`
- Test: `crates/metadata/src/compat.rs` (inline `#[cfg(test)] mod tests`)

- [ ] **Step 1.1: Add `thiserror` to `crates/metadata/Cargo.toml`**

Edit `crates/metadata/Cargo.toml`. Insert `thiserror = { workspace = true }` into `[dependencies]` so the block reads:

```toml
[dependencies]
nebula-schema = { path = "../schema" }
semver = { workspace = true, features = ["serde"] }
serde = { workspace = true, features = ["derive"] }
thiserror = { workspace = true }
```

- [ ] **Step 1.2: Write failing tests in `crates/metadata/src/compat.rs`**

Replace the entire contents of `crates/metadata/src/compat.rs` with the tests-first skeleton below. `use super::*;` is not needed yet because the types do not exist — the tests reference them directly from `crate::compat`, giving a compile error we will remove when the implementation lands.

```rust
//! Generic compatibility rules shared by every catalog citizen.
//!
//! Entity-specific rules (action ports, credential auth pattern, future
//! plugin-manifest contents) compose *on top of* the shared checks here.
//! Keeping the base rules here prevents action/credential/resource from
//! copy-drifting `key immutable / version monotonic / schema-break-requires-
//! major-bump` three times with slightly different spellings.

#[cfg(test)]
mod tests {
    use nebula_schema::{FieldCollector, Schema, ValidSchema};
    use semver::Version;

    use super::{BaseCompatError, validate_base_compat};
    use crate::BaseMetadata;

    fn empty_schema() -> ValidSchema {
        Schema::builder()
            .build()
            .expect("empty schema always valid")
    }

    fn schema_with_one_field() -> ValidSchema {
        // Real API: `SchemaBuilder: FieldCollector` exposes a closure-style
        // `.string(key, |s| s)` child — see `crates/schema/src/builder/mod.rs:56`.
        // `FieldCollector` is re-exported at `nebula_schema::FieldCollector`
        // (see `crates/schema/src/lib.rs:72`).
        Schema::builder()
            .string("extra", |s| s)
            .build()
            .expect("single-string schema always valid")
    }

    // Tests use a tiny newtype key so we don't pull nebula-core here.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestKey(&'static str);

    impl std::fmt::Display for TestKey {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    fn md(key: &'static str, major: u64, minor: u64) -> BaseMetadata<TestKey> {
        BaseMetadata::new(TestKey(key), "n", "d", empty_schema())
            .with_version(Version::new(major, minor, 0))
    }

    #[test]
    fn minor_bump_same_schema_ok() {
        let prev = md("k", 1, 0);
        let next = md("k", 1, 1);
        assert!(validate_base_compat(&next, &prev).is_ok());
    }

    #[test]
    fn major_bump_same_schema_ok() {
        let prev = md("k", 1, 0);
        let next = md("k", 2, 0);
        assert!(validate_base_compat(&next, &prev).is_ok());
    }

    #[test]
    fn key_change_rejected() {
        let prev = md("old", 1, 0);
        let next = md("new", 1, 0);
        let err = validate_base_compat(&next, &prev).unwrap_err();
        assert!(matches!(err, BaseCompatError::KeyChanged { .. }));
    }

    #[test]
    fn version_regression_rejected() {
        let prev = md("k", 2, 1);
        let next = md("k", 2, 0);
        let err = validate_base_compat(&next, &prev).unwrap_err();
        assert!(matches!(err, BaseCompatError::VersionRegressed { .. }));
    }

    #[test]
    fn schema_change_without_major_rejected() {
        let prev = md("k", 1, 0);
        // Same key + minor bump but a *different* schema triggers the rule.
        let next_base =
            BaseMetadata::new(TestKey("k"), "n", "d", schema_with_one_field())
                .with_version(Version::new(1, 1, 0));
        let err = validate_base_compat(&next_base, &prev).unwrap_err();
        assert_eq!(err, BaseCompatError::SchemaChangeWithoutMajorBump);
    }

    #[test]
    fn schema_change_with_major_accepted() {
        let prev = md("k", 1, 0);
        let next_base =
            BaseMetadata::new(TestKey("k"), "n", "d", schema_with_one_field())
                .with_version(Version::new(2, 0, 0));
        assert!(validate_base_compat(&next_base, &prev).is_ok());
    }

    #[test]
    fn display_includes_keys() {
        let prev = md("old", 1, 0);
        let next = md("new", 1, 0);
        let err = validate_base_compat(&next, &prev).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("old") && msg.contains("new"), "got: {msg}");
    }
}
```

Builder API reference: `SchemaBuilder: FieldCollector` — see
`crates/schema/src/builder/mod.rs:36-113` for the trait methods
(`.string / .number / .boolean / …`) and `crates/schema/src/schema.rs:262`
for the `impl`. `FieldCollector` is re-exported at
`nebula_schema::FieldCollector` per `crates/schema/src/lib.rs:72`. The
closure form is `.string(key, |s| s)` (returns the builder unchanged) —
any other method from the trait produces a `ValidSchema != empty_schema()`
and also works. Do **not** invent new method names; if the signatures
above do not match current source, stop and open an issue before
continuing.

- [ ] **Step 1.3: Run tests — expect compile failure**

Run: `cargo nextest run -p nebula-metadata`
Expected: compile error — `BaseCompatError` and `validate_base_compat` unresolved.

- [ ] **Step 1.4: Implement `BaseCompatError<K>` + `validate_base_compat`**

Insert the production code at the top of `crates/metadata/src/compat.rs`, above the `#[cfg(test)] mod tests` block you wrote in Step 1.2. Final file shape:

```rust
//! Generic compatibility rules shared by every catalog citizen.
//!
//! Entity-specific rules (action ports, credential auth pattern, future
//! plugin-manifest contents) compose *on top of* the shared checks here.
//! Keeping the base rules here prevents action/credential/resource from
//! copy-drifting `key immutable / version monotonic / schema-break-requires-
//! major-bump` three times with slightly different spellings.

use semver::Version;

use crate::BaseMetadata;

/// Entity-agnostic compatibility errors reported by
/// [`validate_base_compat`].
///
/// `K` is the concrete catalog-entity key type (e.g. `ActionKey`,
/// `CredentialKey`, `ResourceKey`). It must be `Display` so the error
/// message can name the keys in each variant and `Debug + Clone + Eq` so
/// the error composes inside a `thiserror`-derived parent enum.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BaseCompatError<K>
where
    K: std::fmt::Debug + std::fmt::Display + Clone + PartialEq + Eq,
{
    /// Typed entity key was replaced across revisions — identity is
    /// immutable across a version history.
    #[error("metadata key changed from `{previous}` to `{current}`")]
    KeyChanged {
        /// Previous key value.
        previous: K,
        /// Current (rejected) key value.
        current: K,
    },

    /// Interface version went backwards.
    #[error("interface version regressed from {previous} to {current}")]
    VersionRegressed {
        /// Previous version.
        previous: Version,
        /// Current (rejected) version.
        current: Version,
    },

    /// Input schema changed shape but the major version was not bumped.
    #[error("breaking schema change detected without a major version bump")]
    SchemaChangeWithoutMajorBump,
}

/// Validate that `current` is a backwards-compatible revision of `previous`
/// with respect to the shared `BaseMetadata` prefix.
///
/// Rules:
/// - `key` must be equal.
/// - `version` must be `>= previous.version` (full `semver::Version`
///   ordering, including pre-release + build).
/// - If `schema` changed, `version.major` must exceed `previous.version.major`.
///
/// Entity-specific rules (ports, auth pattern) are **not** checked here —
/// each concrete metadata type layers its own rules on top.
pub fn validate_base_compat<K>(
    current: &BaseMetadata<K>,
    previous: &BaseMetadata<K>,
) -> Result<(), BaseCompatError<K>>
where
    K: std::fmt::Debug + std::fmt::Display + Clone + PartialEq + Eq,
{
    if current.key != previous.key {
        return Err(BaseCompatError::KeyChanged {
            previous: previous.key.clone(),
            current: current.key.clone(),
        });
    }
    if current.version < previous.version {
        return Err(BaseCompatError::VersionRegressed {
            previous: previous.version.clone(),
            current: current.version.clone(),
        });
    }
    if current.schema != previous.schema
        && current.version.major == previous.version.major
    {
        return Err(BaseCompatError::SchemaChangeWithoutMajorBump);
    }
    Ok(())
}
```

- [ ] **Step 1.5: Re-export from `crates/metadata/src/lib.rs`**

Edit `crates/metadata/src/lib.rs`. Update the re-exports block near the bottom of the file so it reads:

```rust
pub use base::{BaseMetadata, Metadata};
pub use compat::{BaseCompatError, validate_base_compat};
pub use deprecation::DeprecationNotice;
pub use icon::Icon;
pub use maturity::MaturityLevel;
```

Also update the module doc comment on `pub mod compat;` above, replacing the current line with:

```rust
/// [`BaseCompatError`] + [`validate_base_compat`] — generic compat rules
/// shared by every catalog citizen.
pub mod compat;
```

- [ ] **Step 1.6: Run tests — expect green**

Run: `cargo nextest run -p nebula-metadata`
Expected: all 6 new compat tests pass; existing tests in `base.rs`, `deprecation.rs`, `icon.rs`, `maturity.rs` stay green.

- [ ] **Step 1.7: Run clippy (crate-local)**

Run: `cargo clippy -p nebula-metadata -- -D warnings`
Expected: no warnings.

- [ ] **Step 1.8: Commit**

```bash
git add crates/metadata/Cargo.toml crates/metadata/src/compat.rs crates/metadata/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(metadata): extract shared compat rules to BaseCompatError

Generic compat checks (key immutable / version monotonic /
schema-break-requires-major) now live on BaseMetadata, so action /
credential / resource can compose them instead of each re-implementing
the same three rules with slightly different spellings.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Migrate `nebula-action` to the shared error

**Files:**
- Modify: `crates/action/src/metadata.rs`

- [ ] **Step 2.1: Update existing tests in `crates/action/src/metadata.rs` to the new matcher shape (failing)**

Locate `fn key_change_is_rejected` in the `#[cfg(test)] mod tests` block. Replace its final `assert!(matches!(...))` with:

```rust
use nebula_metadata::BaseCompatError;

let err = next.validate_compatibility(&prev).unwrap_err();
assert!(matches!(
    err,
    MetadataCompatibilityError::Base(BaseCompatError::KeyChanged { .. })
));
```

Locate `fn version_regression_is_rejected`. Replace the matcher similarly:

```rust
assert!(matches!(
    err,
    MetadataCompatibilityError::Base(BaseCompatError::VersionRegressed { .. })
));
```

Locate `fn schema_change_requires_major_bump`. Its current body differs the
`outputs` of the two metadata objects, so after migration it exercises the
**ports** rule, not the schema rule. Replace the assertion:

```rust
let err = next.validate_compatibility(&prev).unwrap_err();
assert_eq!(err, MetadataCompatibilityError::PortsChangeWithoutMajorBump);
```

Add a new test below it that exercises the schema rule via `BaseCompatError`:

```rust
#[test]
fn schema_field_change_requires_major_bump() {
    use nebula_schema::{FieldCollector, Schema};

    let prev = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc")
        .with_version(1, 0);
    let next = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc")
        .with_version(1, 1)
        .with_schema(
            Schema::builder()
                .string("added", |s| s)
                .build()
                .unwrap(),
        );

    let err = next.validate_compatibility(&prev).unwrap_err();
    assert!(matches!(
        err,
        MetadataCompatibilityError::Base(BaseCompatError::SchemaChangeWithoutMajorBump)
    ));
}
```

`Schema::builder().string(key, |s| s).build()` uses the real
`FieldCollector::string` method — see the builder API reference in
Step 1.2. Do not invent a different method name.

Reviewer note: this task intentionally changes the *semantics* of the
existing `schema_change_requires_major_bump` test (its body differs
outputs, so under the new rules it exercises ports, not schema) and adds
a fresh `schema_field_change_requires_major_bump` test for the
schema-rule path. The commit message in Step 2.7 calls this out in
plain English.

- [ ] **Step 2.2: Run tests — expect compile failure**

Run: `cargo nextest run -p nebula-action --no-run` (then `run` if you want to see failures directly)
Expected: compile errors — `MetadataCompatibilityError::Base`, `::PortsChangeWithoutMajorBump`, and the `BaseCompatError` import unresolved.

- [ ] **Step 2.3: Rewrite `MetadataCompatibilityError` in `crates/action/src/metadata.rs`**

Replace the current enum definition (lines 71–93 in the current file) with the wrapper:

```rust
/// Compatibility validation errors for action metadata evolution.
///
/// Wraps [`nebula_metadata::BaseCompatError`] (shared catalog-entity rules)
/// and layers the action-specific port-change rule on top.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    /// A generic catalog-citizen rule fired (key / version / schema).
    #[error(transparent)]
    Base(#[from] nebula_metadata::BaseCompatError<ActionKey>),

    /// Input or output ports changed without a major version bump.
    #[error("action ports changed without a major version bump")]
    PortsChangeWithoutMajorBump,
}
```

Then replace `ActionMetadata::validate_compatibility` (currently around lines 324–356) with:

```rust
/// Validate that this metadata update is version-compatible with `previous`.
///
/// Delegates `key immutable / version monotonic / schema-break-requires-
/// major` to [`nebula_metadata::validate_base_compat`]; layers the action-
/// specific port-change rule on top.
pub fn validate_compatibility(
    &self,
    previous: &Self,
) -> Result<(), MetadataCompatibilityError> {
    nebula_metadata::validate_base_compat(&self.base, &previous.base)?;

    let ports_changed =
        self.inputs != previous.inputs || self.outputs != previous.outputs;
    if ports_changed && self.base.version.major == previous.base.version.major {
        return Err(MetadataCompatibilityError::PortsChangeWithoutMajorBump);
    }

    Ok(())
}
```

- [ ] **Step 2.4: Remove the now-orphan imports**

Scroll to the top of `crates/action/src/metadata.rs`. The `use semver::Version;` import stays (used for `with_version_full`). The `thiserror` import is already there via the enum derive attribute — if clippy flags a duplicate, remove the redundant line. No other cleanup is expected.

- [ ] **Step 2.5: Run tests — expect green**

Run: `cargo nextest run -p nebula-action`
Expected: all action tests pass including the updated matchers and the new `schema_field_change_requires_major_bump`.

- [ ] **Step 2.6: Run workspace clippy over touched crates**

Run: `cargo clippy -p nebula-metadata -p nebula-action -- -D warnings`
Expected: no warnings.

- [ ] **Step 2.7: Commit**

```bash
git add crates/action/src/metadata.rs
git commit -m "$(cat <<'EOF'
refactor(action): use BaseCompatError via wrapper in MetadataCompatibilityError

MetadataCompatibilityError now wraps nebula_metadata::BaseCompatError and
adds a single action-specific variant (PortsChangeWithoutMajorBump).
Port-change semantics stay identical; schema-change semantics now surface
as Base(SchemaChangeWithoutMajorBump) so credential and resource can
reuse the same rule without copy-drift.

Note: the existing `schema_change_requires_major_bump` test in
crates/action/src/metadata.rs is repurposed — its body differs the
outputs of the two metadata objects, so post-migration it exercises the
PortsChangeWithoutMajorBump rule, not the schema rule. A new
`schema_field_change_requires_major_bump` test added alongside covers
the schema rule via Base(SchemaChangeWithoutMajorBump). This is a
test-semantics change, not a pure refactor.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add compat to `nebula-credential`

**Files:**
- Modify: `crates/credential/src/metadata.rs`

- [ ] **Step 3.1: Write failing tests at the bottom of `crates/credential/src/metadata.rs`**

Append a `#[cfg(test)] mod tests` block (if one does not already exist — check the file first; if it does, add inside it):

```rust
#[cfg(test)]
mod compat_tests {
    use nebula_core::{AuthPattern, credential_key};
    use nebula_metadata::BaseCompatError;
    use nebula_schema::Schema;
    use semver::Version;

    use super::{CredentialMetadata, MetadataCompatibilityError};

    fn empty_schema() -> nebula_schema::ValidSchema {
        Schema::builder().build().unwrap()
    }

    fn cred(pattern: AuthPattern, major: u64, minor: u64) -> CredentialMetadata {
        let mut m = CredentialMetadata::new(
            credential_key!("cred"),
            "C",
            "d",
            empty_schema(),
            pattern,
        );
        m.base.version = Version::new(major, minor, 0);
        m
    }

    #[test]
    fn pattern_change_requires_major_bump() {
        let prev = cred(AuthPattern::OpaqueSecret, 1, 0);
        let next = cred(AuthPattern::OAuth2, 1, 1);
        let err = next.validate_compatibility(&prev).unwrap_err();
        assert_eq!(err, MetadataCompatibilityError::PatternChangeWithoutMajorBump);
    }

    #[test]
    fn pattern_change_with_major_accepted() {
        let prev = cred(AuthPattern::OpaqueSecret, 1, 0);
        let next = cred(AuthPattern::OAuth2, 2, 0);
        assert!(next.validate_compatibility(&prev).is_ok());
    }

    #[test]
    fn key_change_via_base_rejected() {
        let prev = CredentialMetadata::new(
            credential_key!("a"), "A", "d", empty_schema(), AuthPattern::OpaqueSecret,
        );
        let next = CredentialMetadata::new(
            credential_key!("b"), "A", "d", empty_schema(), AuthPattern::OpaqueSecret,
        );
        let err = next.validate_compatibility(&prev).unwrap_err();
        assert!(matches!(
            err,
            MetadataCompatibilityError::Base(BaseCompatError::KeyChanged { .. })
        ));
    }
}
```

If `credential_key!` macro isn't available from `nebula_core`, grep `crates/credential/src/` for how tests build a `CredentialKey` (likely `CredentialKey::new("cred").unwrap()`) and substitute.

Both `AuthPattern::OpaqueSecret` and `AuthPattern::OAuth2` are present in
`crates/core/src/auth.rs` (the `OpaqueSecret` variant is Nebula's name for
API-key / bearer-token / session-token style credentials; there is no
`ApiKey` variant).

- [ ] **Step 3.2: Run tests — expect compile failure**

Run: `cargo nextest run -p nebula-credential`
Expected: compile error — `MetadataCompatibilityError` and `validate_compatibility` undefined.

- [ ] **Step 3.3: Implement `MetadataCompatibilityError` + `validate_compatibility`**

Add below the `CredentialMetadataBuilder` impl block in `crates/credential/src/metadata.rs`:

```rust
/// Compatibility validation errors for credential metadata evolution.
///
/// Wraps [`nebula_metadata::BaseCompatError`] and layers the credential-
/// specific auth-pattern rule on top.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    /// A generic catalog-citizen rule fired (key / version / schema).
    #[error(transparent)]
    Base(#[from] nebula_metadata::BaseCompatError<nebula_core::CredentialKey>),

    /// Auth pattern changed without a major version bump.
    #[error("credential auth pattern changed without a major version bump")]
    PatternChangeWithoutMajorBump,
}

impl CredentialMetadata {
    /// Validate that this metadata update is version-compatible with `previous`.
    ///
    /// Delegates `key immutable / version monotonic / schema-break-requires-
    /// major` to [`nebula_metadata::validate_base_compat`]; layers the
    /// credential-specific auth-pattern rule on top.
    pub fn validate_compatibility(
        &self,
        previous: &Self,
    ) -> Result<(), MetadataCompatibilityError> {
        nebula_metadata::validate_base_compat(&self.base, &previous.base)?;

        if self.pattern != previous.pattern
            && self.base.version.major == previous.base.version.major
        {
            return Err(MetadataCompatibilityError::PatternChangeWithoutMajorBump);
        }

        Ok(())
    }
}
```

- [ ] **Step 3.4: Run tests — expect green**

Run: `cargo nextest run -p nebula-credential`
Expected: all three new compat tests pass; existing tests stay green.

- [ ] **Step 3.5: Run clippy**

Run: `cargo clippy -p nebula-credential -- -D warnings`
Expected: no warnings.

- [ ] **Step 3.6: Commit**

```bash
git add crates/credential/src/metadata.rs
git commit -m "$(cat <<'EOF'
feat(credential): add validate_compatibility using shared BaseCompatError

MetadataCompatibilityError wraps nebula_metadata::BaseCompatError and
adds PatternChangeWithoutMajorBump for the credential-specific auth
pattern rule. Fills the parity gap vs nebula-action where credential
metadata previously had no compat validator at all.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Add compat to `nebula-resource`

**Files:**
- Modify: `crates/resource/src/resource.rs`

- [ ] **Step 4.1: Write failing test in `crates/resource/src/resource.rs`**

Append inside the file's `#[cfg(test)] mod tests` block (if none, add one at the bottom):

```rust
#[cfg(test)]
mod compat_tests {
    use nebula_core::resource_key;
    use nebula_metadata::BaseCompatError;
    use nebula_schema::Schema;
    use semver::Version;

    use super::ResourceMetadata;

    fn empty_schema() -> nebula_schema::ValidSchema {
        Schema::builder().build().unwrap()
    }

    fn md(major: u64, minor: u64) -> ResourceMetadata {
        let mut m = ResourceMetadata::new(
            resource_key!("postgres"), "pg", "d", empty_schema(),
        );
        m.base.version = Version::new(major, minor, 0);
        m
    }

    #[test]
    fn version_monotonic_accepted() {
        let prev = md(1, 0);
        let next = md(1, 1);
        assert!(next.validate_compatibility(&prev).is_ok());
    }

    #[test]
    fn version_regression_rejected() {
        let prev = md(2, 1);
        let next = md(2, 0);
        let err = next.validate_compatibility(&prev).unwrap_err();
        assert!(matches!(err, BaseCompatError::VersionRegressed { .. }));
    }
}
```

If `resource_key!` macro is not exposed, use `ResourceKey::new("postgres").unwrap()` directly.

- [ ] **Step 4.2: Run — expect compile failure**

Run: `cargo nextest run -p nebula-resource`
Expected: compile error — `validate_compatibility` undefined on `ResourceMetadata`.

- [ ] **Step 4.3: Implement `ResourceMetadata::validate_compatibility`**

Insert inside the existing `impl ResourceMetadata { ... }` block in `crates/resource/src/resource.rs` (currently ending around line 116):

```rust
    /// Validate that this metadata update is version-compatible with `previous`.
    ///
    /// Resource metadata has no entity-specific fields beyond the shared
    /// base, so this is a direct delegation to
    /// [`nebula_metadata::validate_base_compat`]. If a future
    /// `ResourceMetadata` gains entity-specific fields, wrap the result in
    /// a `MetadataCompatibilityError` like `nebula-action` does.
    pub fn validate_compatibility(
        &self,
        previous: &Self,
    ) -> Result<(), nebula_metadata::BaseCompatError<nebula_core::ResourceKey>> {
        nebula_metadata::validate_base_compat(&self.base, &previous.base)
    }
```

- [ ] **Step 4.4: Run tests — expect green**

Run: `cargo nextest run -p nebula-resource`
Expected: both new compat tests pass; existing tests stay green.

- [ ] **Step 4.5: Run clippy**

Run: `cargo clippy -p nebula-resource -- -D warnings`
Expected: no warnings.

- [ ] **Step 4.6: Commit**

```bash
git add crates/resource/src/resource.rs
git commit -m "$(cat <<'EOF'
feat(resource): add validate_compatibility using shared BaseCompatError

Direct delegation to nebula_metadata::validate_base_compat. Resource has
no entity-specific rules today, so the consumer signature returns the
bare BaseCompatError<ResourceKey>; swap to a MetadataCompatibilityError
wrapper the moment resource-specific rules appear.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `crates/metadata/README.md`

**Files:**
- Create: `crates/metadata/README.md`

- [ ] **Step 5.1: Write the README**

Create `crates/metadata/README.md` with the exact content:

````markdown
---
name: nebula-metadata
role: Shared catalog-citizen metadata (BaseMetadata + Metadata trait + Icon / MaturityLevel / DeprecationNotice + compat rules)
status: frontier
last-reviewed: 2026-04-19
canon-invariants: [L2-3.5]
related: [nebula-action, nebula-credential, nebula-resource, nebula-plugin]
---

# nebula-metadata

## Purpose

Every catalog citizen in Nebula — an action, a credential, a resource, a
future plugin — shares the same surface: a typed key, a human-readable
name and description, a canonical input schema, optional catalog
ornaments (icon, documentation URL, tags), a declared maturity level,
and an optional deprecation notice. `nebula-metadata` owns those shared
concerns as concrete types and a small trait, so each business-layer
crate composes them instead of redeclaring the same prefix with
incompatible field names.

## Role

**Core-layer support crate.** Cross-cutting, no upward dependencies.
Only depends on `nebula-schema` (for `ValidSchema`), `semver`, `serde`,
and `thiserror`. Every other crate in the business layer
(`nebula-action`, `nebula-credential`, `nebula-resource`) composes
`BaseMetadata<K>` via `#[serde(flatten)]` on its own concrete metadata
struct.

## Public API

- `BaseMetadata<K>` — shared catalog prefix (`key`, `name`, `description`,
  `schema`, `version`, `icon`, `documentation_url`, `tags`, `maturity`,
  `deprecation`). Composed on each concrete entity metadata.
- `Metadata` trait — one-line impl on each concrete metadata
  (`fn base(&self) -> &BaseMetadata<Self::Key>`); all other accessors
  default-delegate through it.
- `Icon` — `None` / `Inline(String)` / `Url { url: String }` enum;
  replaces the earlier `icon: Option<String>` + `icon_url: Option<String>`
  pair.
- `MaturityLevel` — `Experimental` / `Beta` / `Stable` / `Deprecated`.
- `DeprecationNotice` — `since` / `sunset` / `replacement` / `reason`.
- `BaseCompatError<K>` + `validate_base_compat` — entity-agnostic compat
  rules shared by every catalog citizen (`key` immutable, `version`
  monotonic, schema-break-requires-major-bump). Each consumer layers
  entity-specific rules on top via a thin wrapper enum.

## Composition

```rust
use nebula_metadata::{BaseMetadata, Metadata};
use nebula_schema::{Schema, ValidSchema};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyKey(&'static str);

pub struct MyEntityMetadata {
    pub base: BaseMetadata<MyKey>,
    pub extra_field: u32,
}

impl Metadata for MyEntityMetadata {
    type Key = MyKey;
    fn base(&self) -> &BaseMetadata<Self::Key> {
        &self.base
    }
}

fn empty_schema() -> ValidSchema {
    Schema::builder().build().unwrap()
}

let md = MyEntityMetadata {
    base: BaseMetadata::new(MyKey("k"), "My Entity", "desc", empty_schema()),
    extra_field: 7,
};
assert_eq!(md.name(), "My Entity");
```

## Consumers

- `nebula-action::ActionMetadata` — composes `BaseMetadata<ActionKey>`;
  adds `inputs`, `outputs`, `isolation_level`, `category`; wraps
  `BaseCompatError<ActionKey>` in its own `MetadataCompatibilityError`.
- `nebula-credential::CredentialMetadata` — composes
  `BaseMetadata<CredentialKey>`; adds `pattern`; wraps `BaseCompatError`
  similarly.
- `nebula-resource::ResourceMetadata` — composes
  `BaseMetadata<ResourceKey>`; no entity-specific fields today, direct
  delegation.
- `nebula-plugin::PluginMetadata` — **does not** compose `BaseMetadata`
  today. Reshape to `PluginManifest` (bundle descriptor) tracked in
  [ADR-0018](../../docs/adr/0018-plugin-metadata-to-manifest.md);
  manifest will reuse `Icon` / `MaturityLevel` / `DeprecationNotice` but
  not `BaseMetadata<K>` (plugin is a container, not a schematized leaf).

## Canon

- `docs/PRODUCT_CANON.md §3.5` — integration model (one pattern, five concepts).
- `docs/MATURITY.md` — crate-state dashboard row.
- `docs/STYLE.md` — idioms, naming, error taxonomy.
````

- [ ] **Step 5.2: Verify doctests compile**

The composition example is a fenced `rust` block (not `no_run`), so it
runs under `cargo test --doc`.

Run: `cargo test --doc -p nebula-metadata`
Expected: the composition example compiles and passes. If it fails because
of a `Schema::builder` API mismatch, adjust the example to whatever form
`crates/metadata/src/base.rs` tests currently use.

- [ ] **Step 5.3: Commit**

```bash
git add crates/metadata/README.md
git commit -m "$(cat <<'EOF'
docs(metadata): add crate README

Workspace-standard README with purpose, public API surface, composition
example (runnable doctest), and consumer crosslinks. References ADR-0018
for the upcoming PluginMetadata → PluginManifest reshape.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `docs/MATURITY.md` row

**Files:**
- Modify: `docs/MATURITY.md`

- [ ] **Step 6.1: Insert the `nebula-metadata` row**

Open `docs/MATURITY.md`. Find the existing row for `nebula-log` (currently
around line 30) and insert directly below it — between `nebula-log` and
`nebula-metrics` — keeping alphabetical order:

```
| nebula-metadata      | frontier | stable  | stable | n/a | n/a |
```

Column-align the pipes so the new row matches the table formatting. Run
`cargo +nightly fmt --all` will not touch this markdown file — alignment is
manual.

- [ ] **Step 6.2: Update `Last targeted revision`**

Scroll to the bottom of `docs/MATURITY.md`. Update the line that starts
with `Last targeted revision:` to include the new entry. Replace the
current bottom line with:

```
Last targeted revision: 2026-04-19 (nebula-metadata row added; `compat.rs` extracted to BaseCompatError + validate_base_compat; action / credential / resource wired to the shared check).
```

(Keep the existing `Last full sweep:` line above it untouched.)

- [ ] **Step 6.3: Commit**

```bash
git add docs/MATURITY.md
git commit -m "$(cat <<'EOF'
docs(maturity): add nebula-metadata row

frontier / stable / stable / n/a / n/a. Frontier because compat.rs
landed in this cycle and the plugin→manifest ADR may imply further
nebula-metadata additions. Promote to stable after one release cycle
without surface changes.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: ADR-0018 — plugin metadata → manifest

**Files:**
- Create: `docs/adr/0018-plugin-metadata-to-manifest.md`
- Modify: `docs/adr/README.md` (index row)

- [ ] **Step 7.1: Write the ADR**

Create `docs/adr/0018-plugin-metadata-to-manifest.md` with the exact content:

````markdown
---
id: 0018
title: plugin-metadata-to-manifest
status: proposed
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [plugin, metadata, canon-3.5]
related:
  - crates/plugin/src/metadata.rs
  - crates/metadata/src/lib.rs
  - docs/PRODUCT_CANON.md#35-integration-model-one-pattern-five-concepts
linear: []
---

# ADR-0018 — `PluginMetadata` → `PluginManifest`

## Context

`nebula-plugin::PluginMetadata` was introduced before `nebula-metadata`
existed and still carries the pre-consolidation shape:

- `icon: Option<String>` + `icon_url: Option<String>` — the two-field
  invalid-combination problem that `nebula_metadata::Icon` was introduced
  to solve.
- `version: u32` — conflicts with `semver::Version` used everywhere else
  in Nebula (and with ADR-0007 identifier conventions spirit).
- No `maturity`, no `deprecation` — a plugin can never be marked
  experimental or scheduled for removal.
- `author`, `license`, `homepage`, `repository`, `nebula_version`,
  `group`, `color` — all bundle-level / provenance fields that do not
  apply to leaf entities.

Meanwhile `nebula_metadata::BaseMetadata<K>` is the canonical shape for
catalog citizens (§3.5). It requires `schema: ValidSchema`, which a
plugin — being a **container** for actions / credentials / resources —
does not have: user input lives on the leaves it bundles, not on the
container itself. Forcing a plugin into `BaseMetadata` would require
either making `schema` optional (uglifying every leaf consumer's
accessors) or passing `ValidSchema::empty()` (misleading semantics:
"empty schema" ≠ "no schema applies").

The right fix is not to bend the leaf shape around a container — it is
to give the container its own, honest type.

## Decision

1. Rename `nebula-plugin::PluginMetadata` → `nebula-plugin::PluginManifest`
   (a **bundle descriptor**, not entity metadata).

2. `PluginManifest` **does not compose `BaseMetadata<K>`.** A plugin is
   not a schematized leaf.

3. `PluginManifest` **reuses the small types** from `nebula-metadata`:

   - `Icon` — replaces `icon: Option<String>` + `icon_url: Option<String>`.
   - `MaturityLevel` — new for plugins (experimental / beta / stable /
     deprecated).
   - `DeprecationNotice` — new for plugins.

4. `PluginManifest::version` adopts `semver::Version` (consistency with
   `BaseMetadata::version`).

5. Plugin-specific fields stay on the manifest: `author`, `license`,
   `homepage`, `repository`, `nebula_version`, `group`, `color`,
   `description`, `name`, `key`, `tags`. Builder + `normalize_key`
   behavior is preserved.

## Consequences

**Positive.**
- Manifest stops advertising invalid icon combinations.
- Plugins can declare `MaturityLevel::Experimental` / mark deprecations.
- Semver is used uniformly.
- The conceptual split (container vs. leaf) becomes visible in types.

**Negative.**
- Wire format breaks for any persisted plugin metadata. Scope is small:
  no known production deployments, `nebula-plugin` is `frontier` per
  `docs/MATURITY.md`.
- `nebula-plugin` gains a direct dep on `nebula-metadata` (for `Icon` /
  `MaturityLevel` / `DeprecationNotice`).

**Neutral.**
- `Plugin::metadata() → Plugin::manifest()` rename propagates through
  `nebula-plugin::macros`, `nebula-engine::lib.rs` re-exports, and every
  plugin test fixture.

## Alternatives considered

- **(A) Make `BaseMetadata::schema` optional** (`Option<ValidSchema>`).
  *Rejected:* every leaf consumer — action, credential, resource, and
  any future leaf — would gain an `Option<ValidSchema>` in its accessor
  to support a single non-leaf case.

- **(B) Plugin uses `BaseMetadata` with `ValidSchema::empty()`.**
  *Rejected:* misleading semantics. "Empty schema" implies "this entity
  has a schema, and it happens to have zero fields", not "this entity
  has no schema concept".

- **(C) Split `CatalogInfo<K>` (cosmetic prefix: name / description /
  icon / documentation_url / tags / maturity / deprecation) from
  `BaseMetadata<K>` (= `CatalogInfo<K>` + schema).** *Rejected* for now:
  adds a new abstraction layer for a single non-leaf consumer. Revisit
  only if a second container type (bundle, pack, preset) appears.

- **(D) Leave `PluginMetadata` as-is; extract only the shared small types
  (`Icon`, `MaturityLevel`, `DeprecationNotice`) and adopt them
  field-by-field without renaming the struct or touching its accessors.**
  *Rejected:* the shape problem is not only cosmetic. `version: u32`
  contradicts semver everywhere else in Nebula, `icon` + `icon_url` as
  two `Option<String>` fields is exactly the invalid-state bug `Icon`
  was introduced to fix, and `PluginMetadata` as a *name* now mis-signals
  that a plugin is a catalog-leaf like `ActionMetadata` /
  `CredentialMetadata` / `ResourceMetadata` — which it is not. Piecemeal
  extraction leaves a type whose name lies about its role; the rename to
  `PluginManifest` is the part that prevents the next contributor from
  composing `BaseMetadata` into it on reflex. The ADR scope is
  intentionally wider than "swap the small types".

## Migration plan (executed in a follow-up PR)

1. Introduce `PluginManifest` alongside `PluginMetadata`; mark the old
   type `#[deprecated]`.
2. Rename `Plugin::metadata() → Plugin::manifest()` directly — no shim.
   `nebula-plugin` is `frontier` per `docs/MATURITY.md`; the
   `CLAUDE.md` quick-win trap catalog explicitly discourages shim-naming.
3. Update `nebula-plugin::macros` so `#[plugin]` emits
   `PluginManifest::builder(...)`.
4. Update `nebula-engine` re-exports (`crates/engine/src/lib.rs:70`) and
   `crates/engine/README.md:63`.
5. Delete `PluginMetadata` in the following cycle.

## Follow-ups

- Track migration PR against this ADR (issue created at implementation
  time).
- Revisit alternative (C) when a second container-shape entity arrives.
````

- [ ] **Step 7.2: Add row to `docs/adr/README.md` index**

Open `docs/adr/README.md`. In the index table, append a new row after the
`0017` line:

```
| [0018](./0018-plugin-metadata-to-manifest.md) | `PluginMetadata` → `PluginManifest` (bundle descriptor, reuse small types from `nebula-metadata`) | proposed | 2026-04-19 |
```

Update the "Writing a new ADR" section — change "currently **0018**" to
"currently **0019**".

- [ ] **Step 7.3: Commit**

```bash
git add docs/adr/0018-plugin-metadata-to-manifest.md docs/adr/README.md
git commit -m "$(cat <<'EOF'
docs(adr): file ADR-0018 plugin-metadata → plugin-manifest

Records the decision to rename PluginMetadata → PluginManifest (bundle
descriptor) and to reuse only the small types from nebula-metadata
(Icon / MaturityLevel / DeprecationNotice), not BaseMetadata<K>. A
plugin is a container, not a schematized leaf; forcing it into the
leaf shape was the original misfit. Implementation lands in a follow-up
PR per the migration plan in the ADR body.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final validation

- [ ] **Step F.1: Full local gate**

Run: `cargo +nightly fmt --all && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace && cargo test --workspace --doc`
Expected: all green. No formatting diffs, no warnings, all tests + doctests pass.

- [ ] **Step F.2: Lefthook mirror (PR pre-flight)**

Run: `lefthook run pre-push`
Expected: all mirrored CI jobs pass (fmt, clippy, tests, doctests, taplo, MSRV 1.94, `--all-features`, `--no-default-features`).

- [ ] **Step F.3: Push branch, open PR**

Branch: `claude/beautiful-archimedes-34c6aa` (already on it). Push:

```bash
git push -u origin claude/beautiful-archimedes-34c6aa
```

Open PR with title `feat(metadata): extract shared compat rules + README + ADR-0018 plugin manifest` and a body linking to the spec and the ADR.

---

## Self-review notes

Plan covers every spec section:

- Artifact 1 (compat.rs) → Tasks 1–4.
- Artifact 2 (README) → Task 5.
- Artifact 3 (MATURITY row) → Task 6.
- Artifact 4 (ADR-0018) → Task 7.
- Testing strategy → per-task `cargo nextest` + Step F.1 workspace gate.
- Out-of-scope (plugin reshape code) → stays docs-only per ADR-0018 migration plan.

Type-consistency check done: `BaseCompatError<K>` bounds (`Debug + Display +
Clone + PartialEq + Eq`) verified against `ActionKey` / `CredentialKey` /
`ResourceKey` via `domain-key::Key<T>` trait impls. `AuthPattern: PartialEq +
Eq` confirmed in `crates/core/src/auth.rs`. No placeholder text.
