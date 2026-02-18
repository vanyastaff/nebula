# Credential Type & Protocol Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rename `Credential` trait to `CredentialType`, split `refresh`/`revoke` into opt-in traits, introduce `CredentialProtocol` as a reusable base block, implement `ApiKeyProtocol` as the first protocol, and wire `extends = ApiKeyProtocol` into the `#[derive(Credential)]` macro.

**Architecture:** `CredentialType` describes a concrete credential (schema + initialize). `CredentialProtocol` is a static reusable block (parameters + state type) that a `CredentialType` can extend via `extends = XyzProtocol` in the macro attribute — the macro merges parameters and delegates `initialize`. Opt-in traits (`Refreshable`, `Revocable`, `Testable`, `Rotatable`) replace the forced `refresh`/`revoke` stubs.

**Tech Stack:** Rust 2024, async-trait, serde, nebula-parameter (`ParameterCollection`), nebula-macros (proc-macro2 + syn + quote)

---

## Task 1: Rename `Credential` → `CredentialType` in `nebula-credential`

**Files:**
- Modify: `crates/credential/src/traits/credential.rs`
- Modify: `crates/credential/src/traits/mod.rs`
- Modify: `crates/credential/src/lib.rs`

**Context:** The trait is currently called `Credential` which conflicts with the `#[derive(Credential)]` macro name. We rename the trait to `CredentialType`. All existing supertrait chains (`TestableCredential: Credential`, `RotatableCredential: TestableCredential`) must be updated too.

**Step 1: Rename trait in `credential.rs`**

In `crates/credential/src/traits/credential.rs`, rename both occurrences:

```rust
// BEFORE
pub trait Credential: Send + Sync + 'static { ... }
pub trait InteractiveCredential: Credential { ... }

// AFTER
pub trait CredentialType: Send + Sync + 'static { ... }
pub trait InteractiveCredential: CredentialType { ... }
```

**Step 2: Update `mod.rs` re-exports**

In `crates/credential/src/traits/mod.rs`:

```rust
// BEFORE
pub use credential::{Credential, InteractiveCredential};

// AFTER
pub use credential::{CredentialType, InteractiveCredential};
```

**Step 3: Update `testable.rs` supertraits**

In `crates/credential/src/traits/testable.rs`:

```rust
// BEFORE
use super::credential::Credential;
pub trait TestableCredential: Credential { ... }

// AFTER
use super::credential::CredentialType;
pub trait TestableCredential: CredentialType { ... }
```

**Step 4: Update `lib.rs` re-exports**

In `crates/credential/src/lib.rs`, find the traits re-export block and update:

```rust
// BEFORE
pub use crate::traits::{DistributedLock, LockError, LockGuard, StateStore, StorageProvider};

// AFTER  
pub use crate::traits::{
    CredentialType, InteractiveCredential,
    DistributedLock, LockError, LockGuard,
    StateStore, StorageProvider,
};
```

Also update the `prelude` block — remove the commented-out `Credential, InteractiveCredential` lines and add `CredentialType`.

**Step 5: Verify it compiles**

```bash
cargo check -p nebula-credential
```

Expected: no errors about `Credential` trait.

**Step 6: Commit**

```bash
git add crates/credential/src/traits/
git add crates/credential/src/lib.rs
git commit -m "refactor(credential): rename Credential trait to CredentialType"
```

---

## Task 2: Split `refresh` / `revoke` into opt-in traits

**Files:**
- Modify: `crates/credential/src/traits/credential.rs`
- Modify: `crates/credential/src/traits/mod.rs`
- Modify: `crates/credential/src/lib.rs`

**Context:** Currently `CredentialType` forces every impl to write `refresh` and `revoke` stubs returning `Ok(())`. API keys don't expire — they don't need refresh. We split these into separate opt-in traits.

**Step 1: Remove `refresh` and `revoke` from `CredentialType`**

In `crates/credential/src/traits/credential.rs`, the new `CredentialType` trait should be:

```rust
#[async_trait]
pub trait CredentialType: Send + Sync + 'static {
    /// Input type — matches the `ParameterCollection` from `description()`
    type Input: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Persisted state type
    type State: CredentialState;

    /// Static description: key, name, icon, parameter schema
    fn description() -> CredentialDescription
    where
        Self: Sized;

    /// Initialize credential from user input
    async fn initialize(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>;
}
```

Note two changes vs before:
- `fn description(&self)` → `fn description()` (no `self`, static — avoids constructing a dummy instance)
- `refresh` and `revoke` removed

**Step 2: Add `Refreshable` opt-in trait**

At the bottom of the same file, add:

```rust
/// Opt-in: credential supports token/secret refresh (OAuth2, JWT, etc.)
#[async_trait]
pub trait Refreshable: CredentialType {
    async fn refresh(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>;
}
```

**Step 3: Add `Revocable` opt-in trait**

```rust
/// Opt-in: credential supports explicit revocation (OAuth2 token revoke, etc.)
#[async_trait]
pub trait Revocable: CredentialType {
    async fn revoke(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>;
}
```

**Step 4: Update `mod.rs` to export new traits**

```rust
pub use credential::{CredentialType, InteractiveCredential, Refreshable, Revocable};
```

**Step 5: Update `testable.rs` — remove `refresh`/`revoke` dependency**

`TestableCredential` already only requires `CredentialType` as supertrait (updated in Task 1), no further changes needed.

**Step 6: Verify**

```bash
cargo check -p nebula-credential
```

**Step 7: Commit**

```bash
git add crates/credential/src/traits/
git commit -m "refactor(credential): split refresh/revoke into opt-in Refreshable/Revocable traits"
```

---

## Task 3: Add `CredentialProtocol` trait

**Files:**
- Create: `crates/credential/src/protocols/mod.rs`
- Modify: `crates/credential/src/traits/mod.rs`
- Modify: `crates/credential/src/lib.rs`

**Context:** `CredentialProtocol` is a static reusable block. It does NOT have `&self` — it is purely type-level. Plugin authors write `extends = ApiKeyProtocol` and the macro pulls `ApiKeyProtocol::parameters()` and merges them into the description, plus delegates `initialize`.

**Step 1: Create `protocols/mod.rs`**

```rust
//! Built-in credential protocols for reuse across plugins.
//!
//! A `CredentialProtocol` is a static building block that defines:
//! - A fixed set of `ParameterCollection` fields (e.g. server, token)
//! - A `State` type for those fields
//! - A default `initialize` implementation
//!
//! Plugin authors extend a protocol via:
//! ```ignore
//! #[derive(Credential)]
//! #[credential(key = "github-api", name = "GitHub API", extends = ApiKeyProtocol)]
//! pub struct GithubApi {
//!     #[param(name = "User", required)]
//!     pub user: String,
//! }
//! ```

pub mod api_key;

pub use api_key::ApiKeyProtocol;
```

**Step 2: Add `CredentialProtocol` trait to `crates/credential/src/traits/credential.rs`**

Add below the `Revocable` trait:

```rust
/// A reusable credential building block.
///
/// Protocols are purely static (no `self`). They define a fixed schema and
/// a default initialize implementation that concrete `CredentialType`s can
/// inherit via `#[credential(extends = XyzProtocol)]`.
pub trait CredentialProtocol: Send + Sync + 'static {
    /// The state this protocol produces after initialization
    type State: CredentialState;

    /// Parameters this protocol contributes (merged first, before own params)
    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    /// Default initialize logic for this protocol
    ///
    /// Called by the macro-generated `initialize()` when no custom impl exists.
    /// `values` contains the full flat input (protocol fields + own fields).
    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError>
    where
        Self: Sized;
}
```

Add the required import at the top of the file:

```rust
use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::values::ParameterValues;
```

**Step 3: Export from `traits/mod.rs`**

```rust
pub use credential::{CredentialType, CredentialProtocol, InteractiveCredential, Refreshable, Revocable};
```

**Step 4: Export from `lib.rs`**

Add to re-exports:

```rust
pub use crate::traits::CredentialProtocol;
```

And add to `prelude`:

```rust
pub use crate::traits::CredentialProtocol;
```

**Step 5: Verify**

```bash
cargo check -p nebula-credential
```

**Step 6: Commit**

```bash
git add crates/credential/src/traits/credential.rs
git add crates/credential/src/protocols/
git add crates/credential/src/lib.rs
git commit -m "feat(credential): add CredentialProtocol trait and protocols module"
```

---

## Task 4: Implement `ApiKeyProtocol`

**Files:**
- Create: `crates/credential/src/protocols/api_key.rs`

**Context:** This is the first concrete protocol. It contributes two fields — `server` (URL, required) and `token` (secret, required). Its state is `ApiKeyState`. The `build_state` function just copies fields from `ParameterValues`.

**Step 1: Write the test first**

At the bottom of `crates/credential/src/protocols/api_key.rs`, add a `#[cfg(test)]` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_parameter::values::ParameterValues;
    use serde_json::json;

    #[test]
    fn parameters_contains_server_and_token() {
        let params = ApiKeyProtocol::parameters();
        assert!(params.contains("server"));
        assert!(params.contains("token"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn server_is_required() {
        let param = ApiKeyProtocol::parameters();
        assert!(param.get_by_key("server").unwrap().is_required());
    }

    #[test]
    fn token_is_secret_and_required() {
        let param = ApiKeyProtocol::parameters();
        let token = param.get_by_key("token").unwrap();
        assert!(token.is_required());
        // SecretParameter maps to ParameterDef::Secret variant
        assert!(matches!(token, nebula_parameter::def::ParameterDef::Secret(_)));
    }

    #[test]
    fn build_state_produces_correct_state() {
        let mut values = ParameterValues::new();
        values.set("server", json!("https://api.github.com"));
        values.set("token", json!("ghp_secret123"));

        let state = ApiKeyProtocol::build_state(&values).unwrap();
        assert_eq!(state.server, "https://api.github.com");
        assert_eq!(state.token.expose(), "ghp_secret123");
    }

    #[test]
    fn build_state_missing_server_returns_error() {
        let mut values = ParameterValues::new();
        values.set("token", json!("ghp_secret123"));

        let result = ApiKeyProtocol::build_state(&values);
        assert!(result.is_err());
    }

    #[test]
    fn build_state_missing_token_returns_error() {
        let mut values = ParameterValues::new();
        values.set("server", json!("https://api.github.com"));

        let result = ApiKeyProtocol::build_state(&values);
        assert!(result.is_err());
    }
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p nebula-credential protocols::api_key -- --nocapture
```

Expected: compile error — `api_key` module doesn't exist yet.

**Step 3: Implement `ApiKeyState`**

In `crates/credential/src/protocols/api_key.rs`:

```rust
//! ApiKey protocol — reusable server + token credential block.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{SecretParameter, TextParameter};
use nebula_parameter::values::ParameterValues;

use crate::core::{CredentialError, CredentialState, ValidationError};
use crate::traits::CredentialProtocol;

/// State produced by `ApiKeyProtocol` after initialization.
///
/// Stored encrypted in the credential store.
/// Accessible in nodes via `ctx.credential::<MyApi>().await?`
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize)]
#[zeroize(drop)]
pub struct ApiKeyState {
    /// Base URL of the service (e.g. "https://api.github.com")
    pub server: String,
    /// Secret API token — zeroized on drop
    pub token: String,
}

impl ApiKeyState {
    /// Expose the token for use in HTTP headers.
    /// Returns a reference — never clone unnecessarily.
    pub fn expose(&self) -> &str {
        &self.token
    }
}

impl CredentialState for ApiKeyState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "api_key";

    fn scrub_ephemeral(&mut self) {
        // token is the secret — zeroize handles it on drop,
        // but we can also scrub it from memory eagerly if needed
    }
}
```

**Step 4: Implement `ApiKeyProtocol`**

Continue in the same file:

```rust
/// Protocol that contributes `server` + `token` fields.
///
/// # Usage in plugins
///
/// ```ignore
/// #[derive(Credential)]
/// #[credential(
///     key = "github-api",
///     name = "GitHub API",
///     extends = ApiKeyProtocol,
/// )]
/// pub struct GithubApi {
///     #[param(name = "User", required)]
///     pub user: String,
/// }
/// ```
pub struct ApiKeyProtocol;

impl CredentialProtocol for ApiKeyProtocol {
    type State = ApiKeyState;

    fn parameters() -> ParameterCollection {
        let mut server = TextParameter::new("server", "Server URL");
        server.metadata.description = Some("Base URL of the service (e.g. https://api.github.com)".into());
        server.metadata.required = true;

        let mut token = SecretParameter::new("token", "API Token");
        token.metadata.description = Some("Secret API token or personal access token".into());
        token.metadata.required = true;

        ParameterCollection::new()
            .with(ParameterDef::Text(server))
            .with(ParameterDef::Secret(token))
    }

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError> {
        let server = values
            .get_string("server")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat(
                    "missing required field: server".into(),
                ),
            })?
            .to_owned();

        let token = values
            .get_string("token")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat(
                    "missing required field: token".into(),
                ),
            })?
            .to_owned();

        Ok(ApiKeyState { server, token })
    }
}
```

**Step 5: Run tests**

```bash
cargo test -p nebula-credential protocols::api_key -- --nocapture
```

Expected: all 6 tests pass.

**Step 6: Commit**

```bash
git add crates/credential/src/protocols/
git commit -m "feat(credential): implement ApiKeyProtocol with server+token fields"
```

---

## Task 5: Update `#[derive(Credential)]` macro

**Files:**
- Modify: `crates/macros/src/credential.rs`

**Context:** The macro needs three changes:
1. Generate `impl CredentialType` instead of `impl Credential`
2. `description()` becomes a static method (no `&self`)
3. Support optional `extends = ApiKeyProtocol` — merge its `parameters()` into `description()` and delegate `initialize` to `Protocol::build_state`

**Step 1: Write a trybuild UI test for `extends`**

In `crates/macros/tests/ui/`, create `credential_extends_pass.rs`:

```rust
use nebula_credential::protocols::ApiKeyProtocol;
use nebula_macros::Credential;
use nebula_parameter::Parameters;

#[derive(Parameters)]
pub struct GithubApiInput {
    #[param(name = "User", required)]
    pub user: String,
}

#[derive(Credential)]
#[credential(
    key = "github-api",
    name = "GitHub API",
    description = "GitHub API credentials",
    extends = ApiKeyProtocol,
    input = GithubApiInput,
)]
pub struct GithubApi;

fn main() {}
```

**Step 2: Update macro `expand` function**

In `crates/macros/src/credential.rs`, update `expand`:

```rust
fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let cred_attrs = attrs::parse_attrs(&input.attrs, "credential")?;
    let key = cred_attrs.require_string("key", struct_name)?;
    let name = cred_attrs.require_string("name", struct_name)?;
    let description = cred_attrs
        .get_string("description")
        .or_else(|| Some(utils::doc_string(&input.attrs)))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| name.clone());

    // `extends` is optional — a CredentialProtocol type path
    let extends_type = cred_attrs.get_type("extends")?;

    // `input` is optional when `extends` is set — protocol provides all fields
    let input_type = cred_attrs.get_type("input")?;
    // `state` is optional when `extends` is set — protocol provides state
    let state_type = cred_attrs.get_type("state")?;

    match &input.data {
        Data::Struct(_) => {}
        _ => return Err(syn::Error::new(
            input.ident.span(),
            "Credential derive can only be used on structs",
        )),
    }

    // Build the properties expression:
    // - if extends: start with Protocol::parameters(), then merge own params
    // - if no extends: empty ParameterCollection (own params come from Input type)
    let properties_expr = match &extends_type {
        Some(proto) => quote! {
            <#proto as ::nebula_credential::traits::CredentialProtocol>::parameters()
        },
        None => quote! {
            ::nebula_parameter::collection::ParameterCollection::new()
        },
    };

    // Build the State type:
    // - if extends and no explicit state: use Protocol::State
    // - if explicit state: use that
    let resolved_state = match (&state_type, &extends_type) {
        (Some(s), _) => quote! { #s },
        (None, Some(proto)) => quote! {
            <#proto as ::nebula_credential::traits::CredentialProtocol>::State
        },
        (None, None) => return Err(diag::error_spanned(
            struct_name,
            "missing `state = Type` — required when no `extends` is set",
        )),
    };

    // Build the Input type:
    // - if explicit input: use that
    // - if extends and no explicit input: use ParameterValues as flat input
    let resolved_input = match &input_type {
        Some(t) => quote! { #t },
        None => quote! { ::nebula_parameter::values::ParameterValues },
    };

    // Build initialize body:
    // - if extends: delegate to Protocol::build_state(values)
    // - if no extends: todo!() as before
    let initialize_body = match &extends_type {
        Some(proto) => quote! {
            let state = <#proto as ::nebula_credential::traits::CredentialProtocol>::build_state(
                // input must be ParameterValues or implement Into<ParameterValues>
                &::nebula_parameter::values::ParameterValues::from(input),
            )?;
            Ok(::nebula_credential::core::result::InitializeResult::Complete(state))
        },
        None => quote! {
            ::std::todo!(
                "implement `initialize` for credential `{}`",
                stringify!(#struct_name)
            )
        },
    };

    let expanded = quote! {
        #[::async_trait::async_trait]
        impl #impl_generics ::nebula_credential::traits::CredentialType
            for #struct_name #ty_generics #where_clause
        {
            type Input = #resolved_input;
            type State = #resolved_state;

            fn description() -> ::nebula_credential::core::CredentialDescription {
                use ::std::sync::OnceLock;
                static DESC: OnceLock<::nebula_credential::core::CredentialDescription> =
                    OnceLock::new();
                DESC.get_or_init(|| {
                    ::nebula_credential::core::CredentialDescription {
                        key: #key.to_string(),
                        name: #name.to_string(),
                        description: #description.to_string(),
                        icon: None,
                        icon_url: None,
                        documentation_url: None,
                        properties: #properties_expr,
                    }
                })
                .clone()
            }

            async fn initialize(
                &self,
                input: &Self::Input,
                _ctx: &mut ::nebula_credential::core::CredentialContext,
            ) -> ::std::result::Result<
                ::nebula_credential::core::result::InitializeResult<Self::State>,
                ::nebula_credential::core::CredentialError,
            > {
                #initialize_body
            }
        }
    };

    Ok(expanded.into())
}
```

**Step 3: Verify macro compiles**

```bash
cargo build -p nebula-macros
```

**Step 4: Run trybuild tests**

```bash
cargo test -p nebula-macros
```

**Step 5: Commit**

```bash
git add crates/macros/src/credential.rs
git add crates/macros/tests/ui/
git commit -m "feat(macros): update Credential derive — CredentialType + extends support"
```

---

## Task 6: Update `nebula-sdk` prelude

**Files:**
- Modify: `crates/sdk/src/prelude.rs`

**Context:** The prelude currently aliases `traits::Credential as CredentialTrait` — this was the workaround for the name conflict. Now that the trait is `CredentialType`, we can export it cleanly without an alias.

**Step 1: Update the credential re-exports**

In `crates/sdk/src/prelude.rs`, find:

```rust
// Credential types
pub use nebula_credential::{
    core::CredentialContext, core::CredentialDescription, core::CredentialError,
    core::CredentialState, traits::Credential as CredentialTrait,
};
```

Replace with:

```rust
// Credential types
pub use nebula_credential::{
    core::CredentialContext, core::CredentialDescription, core::CredentialError,
    core::CredentialState,
    traits::CredentialType,
    traits::CredentialProtocol,
    traits::Refreshable,
    traits::Revocable,
    protocols::ApiKeyProtocol,
};
```

**Step 2: Verify full SDK compiles**

```bash
cargo check -p nebula-sdk
```

**Step 3: Commit**

```bash
git add crates/sdk/src/prelude.rs
git commit -m "refactor(sdk): clean up credential prelude exports — CredentialType, CredentialProtocol"
```

---

## Task 7: Update `plugins/github` to use new API

**Files:**
- Modify: `plugins/github/src/credentials/github_api.rs`
- Modify: `plugins/github/src/credentials/github_oauth2.rs`
- Modify: `plugins/github/src/lib.rs`

**Context:** The github plugin currently references the old `Credential` trait name and broken imports. We update it to use `extends = ApiKeyProtocol` — the cleanest real-world usage example.

**Step 1: Rewrite `github_api.rs`**

```rust
use nebula_sdk::prelude::{ApiKeyProtocol, Credential, Parameters};

/// GitHub API credentials (personal access token)
///
/// Extends `ApiKeyProtocol` which contributes:
/// - `server`: GitHub server URL (e.g. https://api.github.com)
/// - `token`: Personal access token (secret)
///
/// Adds:
/// - `user`: GitHub username
#[derive(Parameters)]
pub struct GithubApiInput {
    #[param(name = "User", description = "GitHub username", required)]
    pub user: String,
}

#[derive(Credential)]
#[credential(
    key = "github-api",
    name = "GitHub API",
    description = "Authenticate with GitHub using a personal access token",
    extends = ApiKeyProtocol,
    input = GithubApiInput,
)]
pub struct GithubApi;
```

Note: `state` is omitted — the macro infers it as `ApiKeyProtocol::State` = `ApiKeyState`.

**Step 2: Fix `github_oauth2.rs`** (stub for now)

```rust
/// GitHub OAuth2 credentials — to be implemented in OAuth2Protocol task
pub struct GithubOauth2;
```

**Step 3: Fix `lib.rs`**

```rust
pub mod credentials;
```

Remove the broken `mod credential;` line and the placeholder `add` function.

**Step 4: Verify plugin compiles**

```bash
cargo check -p github
```

**Step 5: Commit**

```bash
git add plugins/github/src/
git commit -m "feat(github): update credentials to use ApiKeyProtocol extends"
```

---

## Task 8: Full workspace check

**Step 1: Format**

```bash
cargo fmt --all
```

**Step 2: Clippy**

```bash
cargo clippy --workspace -- -D warnings
```

Fix any warnings before proceeding.

**Step 3: Tests**

```bash
cargo test --workspace
```

Expected: all tests pass.

**Step 4: Docs**

```bash
cargo doc --no-deps -p nebula-credential --open
```

Verify `CredentialType`, `CredentialProtocol`, `ApiKeyProtocol`, `Refreshable`, `Revocable` all appear and are documented.

**Step 5: Final commit**

```bash
git add -A
git commit -m "chore: workspace-wide fmt + clippy fixes after credential refactor"
```

---

## Summary of changes

| What | Before | After |
|---|---|---|
| Core trait | `Credential` (conflicts with macro) | `CredentialType` |
| `description()` | `fn description(&self)` instance method | `fn description()` static |
| `refresh` / `revoke` | forced stubs in `CredentialType` | opt-in `Refreshable` / `Revocable` traits |
| Reuse block | doesn't exist | `CredentialProtocol` trait |
| First protocol | — | `ApiKeyProtocol` (server + token) |
| Macro `extends` | not supported | `extends = ApiKeyProtocol` merges params + delegates initialize |
| SDK prelude | `Credential as CredentialTrait` alias hack | clean `CredentialType`, `CredentialProtocol` |
| Github plugin | broken imports | `extends = ApiKeyProtocol` + own `user` field |
