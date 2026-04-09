# Phase 2a: Derive Macro — Action + Dependencies — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `#[derive(Action)]` generates Action + ActionDependencies with type-based credential/resource declarations. `ctx.credential::<T>()` returns `CredentialGuard<S>`. Unblocks Phase 3 (handler adapters) and Phase 2b (Parameters integration).

**Architecture:** Three layers of change: (1) `CredentialGuard<S>` + type-based credential access on `ActionContext`, (2) `ScopedCredentialAccessor` for runtime enforcement, (3) derive macro improvements to support non-unit structs and generate `credential_types() -> Vec<TypeId>`. The existing macro crate (`crates/action/macros/`) already parses `#[action(credential = Type)]` — we extend it, not rewrite.

**Tech Stack:** Rust 1.94, `proc-macro2`, `quote`, `syn`, `anyhow`, `zeroize`, `secrecy`

**Prerequisites:** Phase 10 (ErrorCode + ActionResultExt) should be completed first — this plan assumes `Arc<anyhow::Error>` and `ErrorCode` are already on `ActionError`.

---

### Task 1: Add CredentialGuard\<S\>

**Files:**
- Create: `crates/action/src/guard.rs`
- Modify: `crates/action/src/lib.rs` (add module + re-export)
- Modify: `crates/action/Cargo.toml` (add `zeroize` dependency if not present)
- Test: `crates/action/src/guard.rs` (inline mod tests)

**Step 1: Write the failing test**

Create `crates/action/src/guard.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::SecretString;

    // Minimal test AuthScheme
    #[derive(Clone, zeroize::Zeroize)]
    struct TestSecret {
        token: String,
    }

    #[test]
    fn deref_to_inner() {
        let guard = CredentialGuard::new(TestSecret {
            token: "abc".into(),
        });
        // Deref gives access to inner fields
        assert_eq!(guard.token, "abc");
    }

    #[test]
    fn guard_is_not_serialize() {
        // This is a compile-time guarantee — CredentialGuard does NOT impl Serialize.
        // We can't test "does not compile" at runtime, but we verify the type exists
        // and works as expected.
        let guard = CredentialGuard::new(TestSecret {
            token: "secret".into(),
        });
        assert_eq!(guard.token, "secret");
    }

    #[test]
    fn clone_works() {
        let guard = CredentialGuard::new(TestSecret {
            token: "abc".into(),
        });
        let cloned = guard.clone();
        assert_eq!(cloned.token, "abc");
    }

    #[test]
    fn debug_redacts() {
        let guard = CredentialGuard::new(TestSecret {
            token: "super-secret".into(),
        });
        let debug = format!("{:?}", guard);
        assert!(!debug.contains("super-secret"));
        assert!(debug.contains("CredentialGuard"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo nextest run -p nebula-action -- guard`
Expected: FAIL — module not found

**Step 3: Implement CredentialGuard**

```rust
//! Credential guard — secure wrapper for credential access.
//!
//! `CredentialGuard<S>` wraps an `AuthScheme` value with three guarantees:
//! 1. **Transparent access** via `Deref<Target = S>`
//! 2. **Zeroize on drop** — secret material wiped from memory
//! 3. **Not Serialize** — prevents accidental inclusion in action output or state
//!
//! Similar to the resource guard pattern used in `nebula-resource`.

use std::fmt;
use std::ops::Deref;

use zeroize::Zeroize;

/// Secure wrapper for credential values returned by `ctx.credential::<S>()`.
///
/// Provides transparent access to the inner `AuthScheme` value via `Deref`,
/// zeroizes the value on drop, and intentionally does NOT implement `Serialize`
/// to prevent accidental persistence of secret material.
///
/// # Examples
///
/// ```rust,ignore
/// let cred = ctx.credential::<BearerSecret>()?;
/// // Use via Deref — cred.token, cred.expose_secret(), etc.
/// client.bearer_auth(cred.token.expose_secret());
/// // Dropped here — zeroized automatically
/// ```
pub struct CredentialGuard<S: Zeroize> {
    inner: S,
}

impl<S: Zeroize> CredentialGuard<S> {
    /// Create a new credential guard wrapping the given scheme value.
    pub(crate) fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S: Zeroize> Deref for CredentialGuard<S> {
    type Target = S;

    fn deref(&self) -> &S {
        &self.inner
    }
}

impl<S: Zeroize> Drop for CredentialGuard<S> {
    fn drop(&mut self) {
        self.inner.zeroize();
    }
}

impl<S: Zeroize + Clone> Clone for CredentialGuard<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<S: Zeroize> fmt::Debug for CredentialGuard<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialGuard")
            .field("type", &std::any::type_name::<S>())
            .finish_non_exhaustive()
    }
}

// NOTE: Intentionally NO Serialize/Deserialize impl.
// This prevents CredentialGuard from being included in action Output or State types,
// which would persist secret material to storage.
```

**Step 4: Register module**

In `crates/action/src/lib.rs`:
```rust
mod guard;
pub use guard::CredentialGuard;
```

In `crates/action/Cargo.toml`, ensure `zeroize` dependency:
```toml
zeroize = "1"
```

**Step 5: Run tests**

Run: `cargo nextest run -p nebula-action -- guard`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/action/src/guard.rs crates/action/src/lib.rs crates/action/Cargo.toml
git commit -m "feat(action): add CredentialGuard<S> — Deref + Zeroize + !Serialize"
```

---

### Task 2: Add type-based credential access to ActionContext

**Files:**
- Modify: `crates/action/src/context.rs`
- Modify: `crates/action/src/capability.rs` (add `get_by_type` to CredentialAccessor)
- Test: `crates/action/src/context.rs` (inline mod tests)

**Step 1: Write the failing test**

Add to `mod tests` in `context.rs`:

```rust
#[tokio::test]
async fn credential_by_type_returns_guard() {
    use nebula_credential::{CredentialMetadata, SecretToken};
    use nebula_core::SecretString;

    let snapshot = CredentialSnapshot::new(
        "api_key",
        CredentialMetadata::new(),
        SecretToken::new(SecretString::new("test-token")),
    );

    let ctx = ActionContext::new(
        ExecutionId::new(),
        NodeId::new(),
        WorkflowId::new(),
        CancellationToken::new(),
    )
    .with_credentials(Arc::new(TestTypedCredentialAccessor::new(snapshot)));

    let guard = ctx.credential::<SecretToken>().await.unwrap();
    // Deref to inner type
    assert_eq!(guard.token().expose_secret(), "test-token");
}
```

**Step 2: Add `get_by_type` to CredentialAccessor trait**

In `capability.rs`, add method to `CredentialAccessor`:

```rust
#[async_trait]
pub trait CredentialAccessor: Send + Sync {
    /// Retrieve a credential snapshot by string id (legacy API).
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, ActionError>;

    /// Check whether a credential exists for the given id.
    async fn has(&self, id: &str) -> bool;

    /// Retrieve a credential snapshot by TypeId of the AuthScheme.
    ///
    /// Used by type-based credential access: `ctx.credential::<T>()`.
    /// Default implementation returns an error — implementations that support
    /// type-based access must override this.
    async fn get_by_type(&self, type_id: std::any::TypeId) -> Result<CredentialSnapshot, ActionError> {
        let _ = type_id;
        Err(ActionError::fatal(
            "type-based credential access not supported by this accessor",
        ))
    }
}
```

Also update `NoopCredentialAccessor` and `TestCredentialAccessor` to include default.

**Step 3: Add `credential::<S>()` method to ActionContext**

In `context.rs`, add new method:

```rust
use crate::guard::CredentialGuard;

impl ActionContext {
    /// Retrieve a typed credential by AuthScheme type.
    ///
    /// Type IS the key — no string identifier needed. The credential accessor
    /// resolves the correct credential by `TypeId`.
    ///
    /// Returns a `CredentialGuard<S>` that:
    /// - Derefs to `S` for transparent access
    /// - Zeroizes secret material on drop
    /// - Does NOT implement Serialize (prevents leakage to output/state)
    ///
    /// # Errors
    ///
    /// - `ActionError::Fatal` if no credential of type `S` is configured
    /// - `ActionError::Fatal` if the stored scheme does not match `S`
    /// - `ActionError::SandboxViolation` if the action did not declare this credential type
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let cred = ctx.credential::<BearerSecret>()?;
    /// client.bearer_auth(cred.token.expose_secret());
    /// ```
    pub async fn credential<S>(&self) -> Result<CredentialGuard<S>, ActionError>
    where
        S: nebula_core::AuthScheme + zeroize::Zeroize,
    {
        let type_id = std::any::TypeId::of::<S>();
        let snapshot = self.credentials.get_by_type(type_id).await?;
        let scheme = snapshot
            .into_project::<S>()
            .map_err(|e| ActionError::fatal(format!("credential type mismatch: {e}")))?;
        Ok(CredentialGuard::new(scheme))
    }
}
```

**Step 4: Run tests**

Run: `cargo nextest run -p nebula-action -- credential_by_type`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/action/src/context.rs crates/action/src/capability.rs
git commit -m "feat(action): add type-based credential access ctx.credential::<S>()

Returns CredentialGuard<S> via TypeId-based lookup on CredentialAccessor.
Legacy string-based credential_typed() remains for backward compat."
```

---

### Task 3: Add ScopedCredentialAccessor

**Files:**
- Create: `crates/action/src/scoped.rs`
- Modify: `crates/action/src/lib.rs` (add module + re-export)
- Test: `crates/action/src/scoped.rs` (inline mod tests)

**Step 1: Write the failing test**

Create `crates/action/src/scoped.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_credential::{CredentialMetadata, CredentialSnapshot, SecretToken};
    use nebula_core::SecretString;
    use crate::capability::NoopCredentialAccessor;

    #[tokio::test]
    async fn allowed_type_passes_through() {
        let snapshot = CredentialSnapshot::new(
            "key",
            CredentialMetadata::new(),
            SecretToken::new(SecretString::new("test")),
        );
        let inner = Arc::new(SingleCredentialAccessor::new(snapshot));
        let scoped = ScopedCredentialAccessor::new(
            inner,
            vec![TypeId::of::<SecretToken>()],
        );

        let result = scoped.get_by_type(TypeId::of::<SecretToken>()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn disallowed_type_returns_sandbox_violation() {
        let inner: Arc<dyn CredentialAccessor> = Arc::new(NoopCredentialAccessor);
        let scoped = ScopedCredentialAccessor::new(
            inner,
            vec![], // no types allowed
        );

        let result = scoped.get_by_type(TypeId::of::<SecretToken>()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ActionError::SandboxViolation { capability, .. } => {
                assert!(capability.contains("credential"));
            }
            other => panic!("expected SandboxViolation, got {:?}", other),
        }
    }
}
```

**Step 2: Implement ScopedCredentialAccessor**

```rust
//! Scoped credential accessor — enforces type-based access control.
//!
//! Wraps a real `CredentialAccessor` and restricts access to only the
//! credential types declared in the action's `ActionDependencies`.

use std::any::TypeId;
use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_credential::CredentialSnapshot;

use crate::capability::CredentialAccessor;
use crate::error::ActionError;

/// Credential accessor that enforces type-based access scoping.
///
/// Engine wraps the real accessor with this at `ActionContext` construction time.
/// The `allowed_types` set is populated from `ActionDependencies::credential_types()`.
///
/// Any `get_by_type()` call for a TypeId not in the set returns
/// `ActionError::SandboxViolation`.
pub struct ScopedCredentialAccessor {
    inner: Arc<dyn CredentialAccessor>,
    allowed_types: HashSet<TypeId>,
}

impl ScopedCredentialAccessor {
    /// Create a scoped accessor from the real accessor and declared types.
    pub fn new(inner: Arc<dyn CredentialAccessor>, allowed_types: Vec<TypeId>) -> Self {
        Self {
            inner,
            allowed_types: allowed_types.into_iter().collect(),
        }
    }
}

#[async_trait]
impl CredentialAccessor for ScopedCredentialAccessor {
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, ActionError> {
        // Legacy string-based access — delegate without type check.
        // ScopedCredentialAccessor is primarily for type-based access enforcement.
        self.inner.get(id).await
    }

    async fn has(&self, id: &str) -> bool {
        self.inner.has(id).await
    }

    async fn get_by_type(&self, type_id: TypeId) -> Result<CredentialSnapshot, ActionError> {
        if !self.allowed_types.contains(&type_id) {
            return Err(ActionError::SandboxViolation {
                capability: format!("credential type {:?}", type_id),
                action_id: String::new(), // engine fills this from context
            });
        }
        self.inner.get_by_type(type_id).await
    }
}
```

**Step 3: Register module**

In `lib.rs`: `mod scoped; pub use scoped::ScopedCredentialAccessor;`

**Step 4: Run tests**

Run: `cargo nextest run -p nebula-action -- scoped`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/action/src/scoped.rs crates/action/src/lib.rs
git commit -m "feat(action): add ScopedCredentialAccessor for type-based access enforcement

Wraps CredentialAccessor, restricts get_by_type() to declared TypeIds.
Returns SandboxViolation for undeclared credential types."
```

---

### Task 4: Add credential_types() to ActionDependencies

**Files:**
- Modify: `crates/action/src/dependency.rs`
- Test: inline in dependency.rs

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;
    use nebula_credential::SecretToken;

    struct TestAction;

    impl ActionDependencies for TestAction {
        fn credential_types() -> Vec<TypeId>
        where
            Self: Sized,
        {
            vec![TypeId::of::<SecretToken>()]
        }
    }

    #[test]
    fn credential_types_returns_declared_types() {
        let types = TestAction::credential_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0], TypeId::of::<SecretToken>());
    }

    #[test]
    fn default_credential_types_is_empty() {
        struct NoCredAction;
        impl ActionDependencies for NoCredAction {}

        assert!(NoCredAction::credential_types().is_empty());
    }
}
```

**Step 2: Add method to ActionDependencies trait**

In `dependency.rs`, add:

```rust
use std::any::TypeId;

pub trait ActionDependencies {
    // ... existing methods ...

    /// TypeIds of credential types this action requires.
    ///
    /// Used by `ScopedCredentialAccessor` to enforce that actions can only
    /// access credentials they declared. Populated by `#[derive(Action)]`
    /// from `#[action(credential = Type)]` attributes.
    fn credential_types() -> Vec<TypeId>
    where
        Self: Sized,
    {
        vec![]
    }
}
```

**Step 3: Run tests**

Run: `cargo nextest run -p nebula-action -- credential_types`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/action/src/dependency.rs
git commit -m "feat(action): add credential_types() to ActionDependencies

Returns Vec<TypeId> of declared credential types for ScopedCredentialAccessor."
```

---

### Task 5: Update derive macro to generate credential_types()

**Files:**
- Modify: `crates/action/macros/src/action_attrs.rs`
- Modify: `crates/action/macros/src/action.rs`
- Test: integration test in `crates/action/tests/`

**Step 1: Write the failing integration test**

Create or add to `crates/action/tests/derive_action.rs`:

```rust
use nebula_action::prelude::*;
use std::any::TypeId;

// Mock credential type for testing
#[derive(Default, Clone, Debug)]
struct TestApiKey;

// Need to impl AuthScheme somehow — check what nebula_credential requires

#[derive(Action)]
#[action(key = "test_action", name = "Test", description = "test")]
#[action(credential = TestApiKey)]
struct MyTestAction;

#[test]
fn derive_generates_credential_types() {
    let types = MyTestAction::credential_types();
    assert_eq!(types.len(), 1);
    assert_eq!(types[0], TypeId::of::<TestApiKey>());
}

#[test]
fn derive_generates_metadata() {
    let action = MyTestAction;
    let meta = action.metadata();
    assert_eq!(meta.key().as_str(), "test_action");
    assert_eq!(meta.name(), "Test");
}
```

**Step 2: Update macro to generate credential_types()**

In `action_attrs.rs`, update `dependencies_impl_expr()` to include:

```rust
let credential_types_method = {
    let all_creds = self.all_credentials();
    if all_creds.is_empty() {
        quote! {}
    } else {
        let type_ids: Vec<_> = all_creds.iter().map(|ty| {
            quote! { ::std::any::TypeId::of::<#ty>() }
        }).collect();
        quote! {
            fn credential_types() -> ::std::vec::Vec<::std::any::TypeId>
            where
                Self: Sized,
            {
                vec![ #(#type_ids),* ]
            }
        }
    }
};
```

Add this to the generated `ActionDependencies` impl block alongside `credential_method` and `resources_method`.

**Step 3: Add duplicate type detection**

In `action_attrs.rs`, in `ActionAttrs::parse()`, after collecting all credentials, add:

```rust
// Check for duplicate credential types
let all_cred_strs: Vec<String> = all_creds.iter().map(|t| quote!(#t).to_string()).collect();
let mut seen = std::collections::HashSet::new();
for (i, s) in all_cred_strs.iter().enumerate() {
    if !seen.insert(s) {
        return Err(syn::Error::new(
            all_creds[i].span(),
            format!("duplicate credential type `{s}` — each type can only be declared once per action"),
        ));
    }
}
```

**Step 4: Run tests**

Run: `cargo nextest run -p nebula-action -- derive_generates`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/action/macros/src/action_attrs.rs crates/action/macros/src/action.rs crates/action/tests/
git commit -m "feat(action): derive macro generates credential_types() with duplicate check

#[action(credential = Type)] now generates ActionDependencies::credential_types()
returning Vec<TypeId>. Duplicate credential types are compile errors."
```

---

### Task 6: Lift unit struct restriction (prepare for Phase 2b)

**Files:**
- Modify: `crates/action/macros/src/action.rs` (remove `validate_unit_struct`)
- Test: integration test with struct-with-fields

**Step 1: Write the failing test**

```rust
#[derive(Action, Clone, serde::Deserialize)]
#[action(key = "with_fields", name = "With Fields", description = "has fields")]
struct ActionWithFields {
    url: String,
    timeout: u32,
}

#[test]
fn derive_works_on_struct_with_fields() {
    let action = ActionWithFields {
        url: "https://example.com".into(),
        timeout: 30,
    };
    let meta = action.metadata();
    assert_eq!(meta.key().as_str(), "with_fields");
}
```

**Step 2: Remove unit struct validation**

In `action.rs`, remove the `validate_unit_struct(&input)?;` call and the `validate_unit_struct` function. The derive macro should work on any named struct (unit or with fields).

**Important:** The `OnceLock<ActionMetadata>` pattern for `metadata()` still works because metadata is derived from attributes, not from fields. The static OnceLock is initialized once regardless of how many instances exist.

**Step 3: Run tests**

Run: `cargo nextest run -p nebula-action`
Expected: PASS (both old unit struct tests and new fields test)

**Step 4: Commit**

```bash
git add crates/action/macros/src/action.rs crates/action/tests/
git commit -m "feat(action): lift unit struct restriction on #[derive(Action)]

Structs with fields now supported — enables type Input = Self pattern in Phase 2b.
Metadata is still attribute-driven, not field-driven."
```

---

### Task 7: cargo expand reference output

**Files:**
- Create: `crates/action/docs/derive-expand-reference.rs` (or `examples/derive_expanded.rs`)

**Step 1: Generate expanded output**

Run `cargo expand` on a canonical example (requires `cargo-expand` installed):

```bash
cd crates/action
cargo expand --test derive_action 2>&1 | head -200
```

Or manually write the expanded output based on what the macro generates.

**Step 2: Create reference file**

Create `crates/action/docs/derive-expand-reference.rs` with the expanded output of a canonical `#[derive(Action)]` example, annotated with comments explaining each generated section.

**Step 3: Commit**

```bash
git add crates/action/docs/derive-expand-reference.rs
git commit -m "docs(action): add cargo expand reference output for #[derive(Action)]

Shows what the derive macro generates — Action impl, ActionDependencies impl,
credential_types() with TypeId. Builds trust with plugin authors (C6 feedback)."
```

---

### Task 8: Full workspace validation + context file update

**Files:**
- Various (fix any downstream compilation errors)
- Modify: `.claude/crates/action.md`

**Step 1: Full workspace check**

```bash
cargo fmt && cargo clippy --workspace -- -D warnings
```

Fix any warnings or errors in downstream crates that depend on nebula-action.

**Step 2: Full test suite**

```bash
cargo nextest run --workspace
```

**Step 3: Doc tests**

```bash
cargo test --workspace --doc
```

**Step 4: Update context file**

Update `.claude/crates/action.md` with:
- `CredentialGuard<S: Zeroize>` — Deref + Zeroize on drop + !Serialize
- `ctx.credential::<S>()` — type-based credential access via TypeId
- `ScopedCredentialAccessor` — runtime enforcement of declared types
- `credential_types()` on `ActionDependencies` — Vec<TypeId> from derive
- Unit struct restriction lifted — structs with fields allowed
- Derive macro generates duplicate credential type check

**Step 5: Commit**

```bash
git add .claude/crates/action.md
# Plus any downstream fixes
git commit -m "feat(action): complete Phase 2a — derive macro + type-based credentials

CredentialGuard<S>, ScopedCredentialAccessor, credential_types(),
unit struct restriction lifted, cargo expand reference output."
```

---

## Summary

| Task | What | Effort |
|------|------|--------|
| 1 | CredentialGuard\<S\> | 1 hour |
| 2 | Type-based credential access on ActionContext | 1-2 hours |
| 3 | ScopedCredentialAccessor | 1 hour |
| 4 | credential_types() on ActionDependencies | 30 min |
| 5 | Derive macro generates credential_types() + duplicate check | 2-3 hours |
| 6 | Lift unit struct restriction | 30 min |
| 7 | cargo expand reference output | 30 min |
| 8 | Workspace validation + context update | 1 hour |

**Total estimated effort: 7-10 hours (2-3 days)**

**Exit criteria from roadmap:**
- [ ] `#[derive(Action)]` compiles on structs with fields
- [ ] `#[action(credential = Type)]` generates `credential_types()` with TypeId
- [ ] Duplicate credential types produce compile error
- [ ] `ctx.credential::<T>()` returns `CredentialGuard<S>`
- [ ] `ScopedCredentialAccessor` enforces type-based access
- [ ] `cargo expand` reference output committed
- [ ] Manual registration path still works (backward compat)
- [ ] Full workspace builds and tests pass
- [ ] Context file updated

## Dependency on Phase 10

Tasks 1-8 assume Phase 10 is complete (`ActionError` has `Arc<anyhow::Error>` + `ErrorCode`). If running in parallel, Task 2 (credential access error handling) may need adjustment for the old `String`-based error — but the plan is written for the target API.
