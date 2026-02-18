# Credential Protocol System Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement a layered credential protocol system with `StaticProtocol`, `FlowProtocol`, `CredentialResource`, and built-in implementations for `BasicAuth`, `HeaderAuth`, `Database`, `OAuth2`, `LDAP`, and stub protocols for `SAML`/`Kerberos`/`mTLS`.

**Architecture:** Rename existing `CredentialProtocol` → `StaticProtocol` (sync, no IO). Add `FlowProtocol` (async, configurable via `Config`). Add `CredentialResource` trait linking Resources to credential State. Wire all through updated `#[derive(Credential)]` macro with `#[oauth2(...)]`, `#[ldap(...)]`, `#[saml(...)]` sub-attributes.

**Tech Stack:** Rust 2024 / MSRV 1.92, `async-trait`, `serde`, `thiserror`, `chrono`, `reqwest` (already in `nebula-credential`), `base64` (already in `nebula-credential`)

---

## Task 1: Rename `CredentialProtocol` → `StaticProtocol`

**Files:**
- Modify: `crates/credential/src/traits/credential.rs`
- Modify: `crates/credential/src/traits/mod.rs`
- Modify: `crates/credential/src/protocols/api_key.rs`
- Modify: `crates/credential/src/protocols/mod.rs`
- Modify: `crates/macros/src/credential.rs`
- Modify: `crates/sdk/src/prelude.rs`
- Modify: `plugins/github/src/credentials/github_api.rs`

**Step 1: Write the failing test**

In `crates/credential/src/protocols/api_key.rs`, the existing tests use `ApiKeyProtocol` which implements `CredentialProtocol`. After renaming, they should compile with `StaticProtocol`. No new test needed — existing 6 tests serve as regression guard. Run them first to confirm baseline.

Run: `cargo test -p nebula-credential -- --nocapture`
Expected: All tests PASS

**Step 2: Rename `CredentialProtocol` → `StaticProtocol` in `credential.rs`**

In `crates/credential/src/traits/credential.rs`, find:

```rust
pub trait CredentialProtocol: Send + Sync + 'static {
    type State: CredentialState;

    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError>
    where
        Self: Sized;
}
```

Replace the trait name with `StaticProtocol`. Keep the doc comment — update it to say "Synchronous form-to-State protocol. No IO, no async." Add a type alias for backwards compatibility:

```rust
/// Synchronous form-to-State protocol. No IO, no async.
///
/// Use for: API keys, Basic Auth, database credentials, header auth.
/// Implement via `#[credential(extends = MyProtocol)]` macro attribute.
pub trait StaticProtocol: Send + Sync + 'static {
    type State: CredentialState;

    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError>
    where
        Self: Sized;
}

/// Backwards-compatible alias — prefer [`StaticProtocol`].
#[deprecated(since = "0.1.0", note = "use `StaticProtocol` instead")]
pub type CredentialProtocol = dyn StaticProtocol<State = ()>;
```

Actually — NO type alias: the old trait was not object-safe in that form, and `CredentialProtocol` is only used internally. Just rename everywhere.

**Step 3: Update `mod.rs` re-export**

In `crates/credential/src/traits/mod.rs`, change:

```rust
pub use credential::{
    CredentialProtocol, CredentialType, InteractiveCredential, Refreshable, Revocable,
};
```

To:

```rust
pub use credential::{
    StaticProtocol, CredentialType, InteractiveCredential, Refreshable, Revocable,
};
```

**Step 4: Update `api_key.rs` impl**

In `crates/credential/src/protocols/api_key.rs`, change:

```rust
use crate::traits::CredentialProtocol;
```

to:

```rust
use crate::traits::StaticProtocol;
```

And change:

```rust
impl CredentialProtocol for ApiKeyProtocol {
```

to:

```rust
impl StaticProtocol for ApiKeyProtocol {
```

**Step 5: Update macro**

In `crates/macros/src/credential.rs`, change all 3 occurrences of `CredentialProtocol` to `StaticProtocol`:

```rust
// line: extends_type resolution
let resolved_state = match (&explicit_state, &extends_type) {
    (None, Some(proto)) => quote! {
        <#proto as ::nebula_credential::traits::StaticProtocol>::State
    },
    // ...
};

let properties_expr = match &extends_type {
    Some(proto) => quote! {
        <#proto as ::nebula_credential::traits::StaticProtocol>::parameters()
    },
    // ...
};

let initialize_body = match &extends_type {
    Some(proto) => quote! {
        let state = <#proto as ::nebula_credential::traits::StaticProtocol>::build_state(input)?;
        // ...
    },
    // ...
};
```

**Step 6: Update SDK prelude**

In `crates/sdk/src/prelude.rs`, change `CredentialProtocol` to `StaticProtocol`:

```rust
pub use nebula_credential::{
    // ...
    traits::StaticProtocol,
    // ...
};
```

**Step 7: Run tests to verify rename compiles**

Run: `cargo check --workspace`
Expected: No errors

Run: `cargo test -p nebula-credential -- --nocapture`
Expected: All 6 api_key tests PASS

**Step 8: Commit**

```bash
git add crates/credential/src/traits/ crates/credential/src/protocols/api_key.rs \
        crates/macros/src/credential.rs crates/sdk/src/prelude.rs \
        plugins/github/src/credentials/
git commit -m "refactor(credential): rename CredentialProtocol to StaticProtocol"
```

---

## Task 2: Add `FlowProtocol` trait + `CredentialResource` trait

**Files:**
- Modify: `crates/credential/src/traits/credential.rs`
- Modify: `crates/credential/src/traits/mod.rs`

**Step 1: Write the failing compile test**

In `crates/credential/src/traits/credential.rs`, add a compile-time doc-test at the bottom that uses `FlowProtocol`:

```rust
/// ```compile_fail
/// // FlowProtocol must not be importable before this task is done
/// use nebula_credential::traits::FlowProtocol;
/// ```
```

Run: `cargo test -p nebula-credential --doc`
Expected: FAIL (FlowProtocol not found)

**Step 2: Add `FlowProtocol` trait**

Append to `crates/credential/src/traits/credential.rs` (after the existing `CredentialProtocol`/`StaticProtocol` trait):

```rust
use crate::core::CredentialContext;

/// Async multi-step protocol. Configurable per provider.
///
/// Use for: OAuth2, LDAP, SAML, Kerberos, mTLS.
/// Plugin implements `Config` type and uses macro attributes to wire it up.
pub trait FlowProtocol: Send + Sync + 'static {
    /// Provider-specific configuration (endpoints, scopes, options)
    type Config: Send + Sync + 'static;

    /// State produced after successful flow completion
    type State: CredentialState;

    /// Parameters shown to user in UI (client_id, client_secret, etc.)
    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    /// Execute the authentication flow
    async fn initialize(
        config: &Self::Config,
        values: &ParameterValues,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>
    where
        Self: Sized;

    /// Refresh an expired credential (default: no-op)
    async fn refresh(
        config: &Self::Config,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>
    where
        Self: Sized,
    {
        let _ = (config, state, ctx);
        Ok(())
    }

    /// Revoke an active credential (default: no-op)
    async fn revoke(
        config: &Self::Config,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>
    where
        Self: Sized,
    {
        let _ = (config, state, ctx);
        Ok(())
    }
}
```

Note: `async fn` in traits requires Rust 1.75+ — already in MSRV 1.92, so no `async-trait` needed for `FlowProtocol` (it is a free-standing trait with `Sized` bounds, not an object-safe trait). Confirm with `cargo check`. If AFIT causes issues, add `#[async_trait::async_trait]` temporarily.

**Step 3: Add `CredentialResource` trait**

Append after `FlowProtocol` in the same file. This requires a `Resource` bound — but `Resource` lives in a different crate not yet linked. Use a simpler form that does not depend on `nebula-resource` to avoid circular deps. The trait is a marker for the pattern:

```rust
/// Links a resource client to its required credential type.
///
/// The runtime retrieves the credential State automatically and calls
/// `authorize()` when creating or refreshing the resource instance.
pub trait CredentialResource {
    /// The credential type required by this resource
    type Credential: CredentialType;

    /// Apply credential state to authorize this resource's client
    fn authorize(
        &mut self,
        state: &<Self::Credential as CredentialType>::State,
    );
}
```

**Step 4: Update `mod.rs` re-export**

```rust
pub use credential::{
    StaticProtocol, FlowProtocol, CredentialResource,
    CredentialType, InteractiveCredential, Refreshable, Revocable,
};
```

**Step 5: Fix the doc-test to be a passing test**

Change the compile_fail doc test to:

```rust
/// ```
/// use nebula_credential::traits::FlowProtocol;
/// // FlowProtocol is now importable
/// ```
```

Run: `cargo test -p nebula-credential --doc`
Expected: PASS

**Step 6: Run full workspace check**

Run: `cargo check --workspace`
Expected: No errors

**Step 7: Commit**

```bash
git add crates/credential/src/traits/
git commit -m "feat(credential): add FlowProtocol and CredentialResource traits"
```

---

## Task 3: Add `BasicAuthProtocol`, `HeaderAuthProtocol`, `DatabaseProtocol`

**Files:**
- Create: `crates/credential/src/protocols/basic_auth.rs`
- Create: `crates/credential/src/protocols/header_auth.rs`
- Create: `crates/credential/src/protocols/database.rs`
- Modify: `crates/credential/src/protocols/mod.rs`

These are all `StaticProtocol` implementations — pattern identical to `api_key.rs`.

**Step 1: Write failing tests for `BasicAuthProtocol`**

Create `crates/credential/src/protocols/basic_auth.rs` with tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn basic_auth_parameters_has_username_and_password() {
        let params = BasicAuthProtocol::parameters();
        assert!(params.contains("username"));
        assert!(params.contains("password"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn basic_auth_build_state_produces_state() {
        let mut values = ParameterValues::new();
        values.set("username", json!("alice"));
        values.set("password", json!("s3cr3t"));
        let state = BasicAuthProtocol::build_state(&values).unwrap();
        assert_eq!(state.username, "alice");
        assert_eq!(state.password, "s3cr3t");
    }

    #[test]
    fn basic_auth_encoded_produces_base64() {
        let state = BasicAuthState {
            username: "alice".into(),
            password: "s3cr3t".into(),
        };
        let encoded = state.encoded();
        // base64("alice:s3cr3t") = "YWxpY2U6czNjcjN0"
        assert_eq!(encoded, "YWxpY2U6czNjcjN0");
    }

    #[test]
    fn basic_auth_missing_username_returns_error() {
        let mut values = ParameterValues::new();
        values.set("password", json!("s3cr3t"));
        assert!(BasicAuthProtocol::build_state(&values).is_err());
    }
}
```

Run: `cargo test -p nebula-credential protocols::basic_auth`
Expected: FAIL (BasicAuthProtocol not defined)

**Step 2: Implement `basic_auth.rs`**

```rust
//! BasicAuth protocol — username + password credential block.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};

use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{SecretParameter, TextParameter};
use nebula_parameter::values::ParameterValues;

use crate::core::{CredentialError, CredentialState, ValidationError};
use crate::traits::StaticProtocol;

/// State produced by [`BasicAuthProtocol`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicAuthState {
    pub username: String,
    pub password: String,
}

impl BasicAuthState {
    /// Base64-encoded "username:password" for `Authorization: Basic` header.
    pub fn encoded(&self) -> String {
        BASE64.encode(format!("{}:{}", self.username, self.password))
    }
}

impl CredentialState for BasicAuthState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "basic_auth";

    fn scrub_ephemeral(&mut self) {}
}

pub struct BasicAuthProtocol;

impl StaticProtocol for BasicAuthProtocol {
    type State = BasicAuthState;

    fn parameters() -> ParameterCollection {
        let mut username = TextParameter::new("username", "Username");
        username.metadata.required = true;

        let mut password = SecretParameter::new("password", "Password");
        password.metadata.required = true;

        ParameterCollection::new()
            .with(ParameterDef::Text(username))
            .with(ParameterDef::Secret(password))
    }

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError> {
        let username = values
            .get_string("username")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: username".into()),
            })?
            .to_owned();

        let password = values
            .get_string("password")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: password".into()),
            })?
            .to_owned();

        Ok(BasicAuthState { username, password })
    }
}
```

**Step 3: Run tests for `basic_auth`**

Run: `cargo test -p nebula-credential protocols::basic_auth -- --nocapture`
Expected: 4 tests PASS

**Step 4: Write and implement `header_auth.rs`**

Write tests first (same pattern):

```rust
// Tests
#[test]
fn header_auth_parameters_has_header_name_and_value() {
    let params = HeaderAuthProtocol::parameters();
    assert!(params.contains("header_name"));
    assert!(params.contains("header_value"));
}

#[test]
fn header_auth_build_state_produces_state() {
    let mut values = ParameterValues::new();
    values.set("header_name", json!("X-Auth-Token"));
    values.set("header_value", json!("tok_123"));
    let state = HeaderAuthProtocol::build_state(&values).unwrap();
    assert_eq!(state.header_name, "X-Auth-Token");
    assert_eq!(state.header_value, "tok_123");
}
```

Implementation:

```rust
//! HeaderAuth protocol — arbitrary header name + secret value.

use serde::{Deserialize, Serialize};
use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{SecretParameter, TextParameter};
use nebula_parameter::values::ParameterValues;
use crate::core::{CredentialError, CredentialState, ValidationError};
use crate::traits::StaticProtocol;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderAuthState {
    pub header_name: String,
    pub header_value: String,
}

impl CredentialState for HeaderAuthState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "header_auth";
    fn scrub_ephemeral(&mut self) {}
}

pub struct HeaderAuthProtocol;

impl StaticProtocol for HeaderAuthProtocol {
    type State = HeaderAuthState;

    fn parameters() -> ParameterCollection {
        let mut name = TextParameter::new("header_name", "Header Name");
        name.metadata.required = true;
        name.metadata.placeholder = Some("X-Auth-Token".into());

        let mut value = SecretParameter::new("header_value", "Header Value");
        value.metadata.required = true;

        ParameterCollection::new()
            .with(ParameterDef::Text(name))
            .with(ParameterDef::Secret(value))
    }

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError> {
        let header_name = values
            .get_string("header_name")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: header_name".into()),
            })?
            .to_owned();

        let header_value = values
            .get_string("header_value")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: header_value".into()),
            })?
            .to_owned();

        Ok(HeaderAuthState { header_name, header_value })
    }
}
```

**Step 5: Write and implement `database.rs`**

Write tests:

```rust
#[test]
fn database_parameters_are_complete() {
    let params = DatabaseProtocol::parameters();
    assert!(params.contains("host"));
    assert!(params.contains("port"));
    assert!(params.contains("database"));
    assert!(params.contains("username"));
    assert!(params.contains("password"));
    assert!(params.contains("ssl_mode"));
}

#[test]
fn database_build_state_with_defaults() {
    let mut values = ParameterValues::new();
    values.set("host", json!("localhost"));
    values.set("port", json!("5432"));
    values.set("database", json!("mydb"));
    values.set("username", json!("admin"));
    values.set("password", json!("pass"));
    // ssl_mode omitted — should default to "disable"
    let state = DatabaseProtocol::build_state(&values).unwrap();
    assert_eq!(state.host, "localhost");
    assert_eq!(state.port, 5432);
    assert_eq!(state.ssl_mode, "disable");
}
```

Implementation:

```rust
//! Database protocol — host, port, database, username, password, ssl_mode.

use serde::{Deserialize, Serialize};
use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{SecretParameter, TextParameter};
use nebula_parameter::values::ParameterValues;
use crate::core::{CredentialError, CredentialState, ValidationError};
use crate::traits::StaticProtocol;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseState {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub ssl_mode: String,
}

impl CredentialState for DatabaseState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "database";
    fn scrub_ephemeral(&mut self) {}
}

pub struct DatabaseProtocol;

impl StaticProtocol for DatabaseProtocol {
    type State = DatabaseState;

    fn parameters() -> ParameterCollection {
        let mut host = TextParameter::new("host", "Host");
        host.metadata.required = true;
        host.metadata.placeholder = Some("localhost".into());

        let mut port = TextParameter::new("port", "Port");
        port.metadata.placeholder = Some("5432".into());

        let mut database = TextParameter::new("database", "Database");
        database.metadata.required = true;

        let mut username = TextParameter::new("username", "Username");
        username.metadata.required = true;

        let mut password = SecretParameter::new("password", "Password");
        password.metadata.required = true;

        let mut ssl_mode = TextParameter::new("ssl_mode", "SSL Mode");
        ssl_mode.metadata.placeholder = Some("disable".into());

        ParameterCollection::new()
            .with(ParameterDef::Text(host))
            .with(ParameterDef::Text(port))
            .with(ParameterDef::Text(database))
            .with(ParameterDef::Text(username))
            .with(ParameterDef::Secret(password))
            .with(ParameterDef::Text(ssl_mode))
    }

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError> {
        let host = values.get_string("host")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: host".into()),
            })?
            .to_owned();

        let port_str = values.get_string("port").unwrap_or("5432");
        let port = port_str.parse::<u16>().map_err(|_| CredentialError::Validation {
            source: ValidationError::InvalidFormat(format!("invalid port: {port_str}")),
        })?;

        let database = values.get_string("database")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: database".into()),
            })?
            .to_owned();

        let username = values.get_string("username")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: username".into()),
            })?
            .to_owned();

        let password = values.get_string("password")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: password".into()),
            })?
            .to_owned();

        let ssl_mode = values.get_string("ssl_mode")
            .unwrap_or("disable")
            .to_owned();

        Ok(DatabaseState { host, port, database, username, password, ssl_mode })
    }
}
```

**Step 6: Register in `protocols/mod.rs`**

```rust
pub mod api_key;
pub mod basic_auth;
pub mod database;
pub mod header_auth;

pub use api_key::{ApiKeyProtocol, ApiKeyState};
pub use basic_auth::{BasicAuthProtocol, BasicAuthState};
pub use database::{DatabaseProtocol, DatabaseState};
pub use header_auth::{HeaderAuthProtocol, HeaderAuthState};
```

**Step 7: Run tests**

Run: `cargo test -p nebula-credential -- --nocapture`
Expected: All tests PASS (6 api_key + 4 basic_auth + 2 header_auth + 2 database)

**Step 8: Commit**

```bash
git add crates/credential/src/protocols/
git commit -m "feat(credential): add BasicAuthProtocol, HeaderAuthProtocol, DatabaseProtocol"
```

---

## Task 4: Add OAuth2 supporting types (`GrantType`, `AuthStyle`, `OAuth2Config`, `OAuth2State`)

**Files:**
- Create: `crates/credential/src/protocols/oauth2/mod.rs`
- Create: `crates/credential/src/protocols/oauth2/config.rs`
- Create: `crates/credential/src/protocols/oauth2/state.rs`
- Modify: `crates/credential/src/protocols/mod.rs`

No HTTP logic yet — pure data types and builder. `OAuth2Protocol::initialize()` will be added in Task 5.

**Step 1: Write failing tests**

Create `crates/credential/src/protocols/oauth2/config.rs` with tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_requires_auth_url_and_token_url() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();
        assert_eq!(config.auth_url, "https://example.com/auth");
        assert_eq!(config.token_url, "https://example.com/token");
        assert_eq!(config.grant_type, GrantType::AuthorizationCode);
        assert_eq!(config.auth_style, AuthStyle::Header);
        assert!(!config.pkce);
    }

    #[test]
    fn default_scopes_is_empty() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .build();
        assert!(config.scopes.is_empty());
    }

    #[test]
    fn scopes_can_be_set() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .scopes(["read", "write"])
            .build();
        assert_eq!(config.scopes, vec!["read", "write"]);
    }
}
```

For `state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::time::Duration;

    #[test]
    fn bearer_header_format() {
        let state = OAuth2State {
            access_token: "tok_abc".into(),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
        };
        assert_eq!(state.bearer_header(), "Bearer tok_abc");
    }

    #[test]
    fn expired_token_detected() {
        let state = OAuth2State {
            access_token: "tok".into(),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at: Some(Utc::now() - chrono::Duration::seconds(60)),
            scopes: vec![],
        };
        assert!(state.is_expired(Duration::from_secs(0)));
    }

    #[test]
    fn valid_token_not_expired() {
        let state = OAuth2State {
            access_token: "tok".into(),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at: Some(Utc::now() + chrono::Duration::seconds(300)),
            scopes: vec![],
        };
        assert!(!state.is_expired(Duration::from_secs(0)));
    }

    #[test]
    fn none_expires_at_never_expired() {
        let state = OAuth2State {
            access_token: "tok".into(),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
        };
        assert!(!state.is_expired(Duration::from_secs(9999)));
    }
}
```

Run: `cargo test -p nebula-credential protocols::oauth2`
Expected: FAIL (module not found)

**Step 2: Create `oauth2/config.rs`**

```rust
//! OAuth2 provider configuration — built via builder, const-friendly.

use serde::{Deserialize, Serialize};

/// How client credentials are sent in the OAuth2 token request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AuthStyle {
    /// RFC 6749: Authorization: Basic base64(client_id:client_secret) — default
    #[default]
    Header,
    /// client_id + client_secret as POST body form fields
    /// Required by: GitHub, Slack
    PostBody,
}

/// OAuth2 grant type (RFC 6749 / RFC 8628).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GrantType {
    /// Authorization Code flow — user browser redirect (default)
    #[default]
    AuthorizationCode,
    /// Client Credentials — server-to-server, no user
    ClientCredentials,
    /// Device Authorization Grant (RFC 8628)
    DeviceCode,
}

/// Provider-specific OAuth2 configuration.
///
/// Build via [`OAuth2Config::authorization_code()`] or other constructors.
/// Generated as a `const` by the `#[oauth2(...)]` macro attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Config {
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub grant_type: GrantType,
    pub auth_style: AuthStyle,
    pub pkce: bool,
}

impl OAuth2Config {
    pub fn authorization_code() -> OAuth2ConfigBuilder {
        OAuth2ConfigBuilder::new(GrantType::AuthorizationCode)
    }

    pub fn client_credentials() -> OAuth2ConfigBuilder {
        OAuth2ConfigBuilder::new(GrantType::ClientCredentials)
    }

    pub fn device_code() -> OAuth2ConfigBuilder {
        OAuth2ConfigBuilder::new(GrantType::DeviceCode)
    }
}

/// Builder for [`OAuth2Config`].
pub struct OAuth2ConfigBuilder {
    grant_type: GrantType,
    auth_url: String,
    token_url: String,
    scopes: Vec<String>,
    auth_style: AuthStyle,
    pkce: bool,
}

impl OAuth2ConfigBuilder {
    fn new(grant_type: GrantType) -> Self {
        Self {
            grant_type,
            auth_url: String::new(),
            token_url: String::new(),
            scopes: Vec::new(),
            auth_style: AuthStyle::Header,
            pkce: false,
        }
    }

    pub fn auth_url(mut self, url: impl Into<String>) -> Self {
        self.auth_url = url.into();
        self
    }

    pub fn token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    pub fn scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    pub fn auth_style(mut self, style: AuthStyle) -> Self {
        self.auth_style = style;
        self
    }

    pub fn pkce(mut self, pkce: bool) -> Self {
        self.pkce = pkce;
        self
    }

    pub fn build(self) -> OAuth2Config {
        OAuth2Config {
            auth_url: self.auth_url,
            token_url: self.token_url,
            scopes: self.scopes,
            grant_type: self.grant_type,
            auth_style: self.auth_style,
            pkce: self.pkce,
        }
    }
}
```

**Step 3: Create `oauth2/state.rs`**

```rust
//! OAuth2 State — access token, refresh token, expiry.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::core::CredentialState;

/// Persisted state after a successful OAuth2 flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2State {
    pub access_token: String,
    /// Typically "Bearer"
    pub token_type: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
}

impl OAuth2State {
    /// Returns `true` if the access token is expired or expires within `margin`.
    pub fn is_expired(&self, margin: Duration) -> bool {
        match self.expires_at {
            None => false,
            Some(exp) => {
                let margin = chrono::Duration::from_std(margin).unwrap_or_default();
                Utc::now() + margin >= exp
            }
        }
    }

    /// `Authorization: Bearer <access_token>` header value.
    pub fn bearer_header(&self) -> String {
        format!("Bearer {}", self.access_token)
    }
}

impl CredentialState for OAuth2State {
    const VERSION: u16 = 1;
    const KIND: &'static str = "oauth2";

    fn scrub_ephemeral(&mut self) {
        // access_token and refresh_token must be stored — no scrub
    }
}
```

**Step 4: Create `oauth2/mod.rs`**

```rust
//! OAuth2 protocol — FlowProtocol implementation (HTTP flow in flow.rs, Task 5).

pub mod config;
pub mod state;

pub use config::{AuthStyle, GrantType, OAuth2Config, OAuth2ConfigBuilder};
pub use state::OAuth2State;
```

**Step 5: Register in `protocols/mod.rs`**

```rust
pub mod api_key;
pub mod basic_auth;
pub mod database;
pub mod header_auth;
pub mod oauth2;

pub use api_key::{ApiKeyProtocol, ApiKeyState};
pub use basic_auth::{BasicAuthProtocol, BasicAuthState};
pub use database::{DatabaseProtocol, DatabaseState};
pub use header_auth::{HeaderAuthProtocol, HeaderAuthState};
pub use oauth2::{AuthStyle, GrantType, OAuth2Config, OAuth2ConfigBuilder, OAuth2State};
```

**Step 6: Run tests**

Run: `cargo test -p nebula-credential protocols::oauth2 -- --nocapture`
Expected: 7 tests PASS (3 config + 4 state)

**Step 7: Commit**

```bash
git add crates/credential/src/protocols/oauth2/
git commit -m "feat(credential): add OAuth2Config, OAuth2State, GrantType, AuthStyle"
```

---

## Task 5: Implement `OAuth2Protocol` (FlowProtocol — HTTP token exchange)

**Files:**
- Create: `crates/credential/src/protocols/oauth2/flow.rs`
- Modify: `crates/credential/src/protocols/oauth2/mod.rs`

This is the `FlowProtocol` impl that performs the actual token exchange via `reqwest` (already in `Cargo.toml`). For `AuthorizationCode`, it returns `RequiresInteraction(Redirect { ... })`. For `ClientCredentials`, it performs the token exchange directly.

**Step 1: Write failing tests**

Create `crates/credential/src/protocols/oauth2/flow.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::oauth2::config::{AuthStyle, GrantType, OAuth2Config};

    #[test]
    fn oauth2_protocol_parameters_has_client_id_and_secret() {
        let params = OAuth2Protocol::parameters();
        assert!(params.contains("client_id"));
        assert!(params.contains("client_secret"));
    }

    #[tokio::test]
    async fn authorization_code_returns_redirect() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .scopes(["read"])
            .build();

        let mut values = ParameterValues::new();
        values.set("client_id", serde_json::json!("my_client"));
        values.set("client_secret", serde_json::json!("my_secret"));

        let mut ctx = CredentialContext::default();
        let result = OAuth2Protocol::initialize(&config, &values, &mut ctx).await.unwrap();

        match result {
            InitializeResult::RequiresInteraction(InteractionRequest::Redirect { url, .. }) => {
                assert!(url.contains("example.com/auth"));
                assert!(url.contains("client_id=my_client"));
            }
            other => panic!("expected Redirect, got: {other:?}"),
        }
    }
}
```

Run: `cargo test -p nebula-credential protocols::oauth2::flow`
Expected: FAIL (OAuth2Protocol not defined)

**Step 2: Implement `flow.rs`**

```rust
//! OAuth2Protocol — FlowProtocol implementation.

use std::collections::HashMap;

use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{SecretParameter, TextParameter};
use nebula_parameter::values::ParameterValues;

use crate::core::{CredentialContext, CredentialError, ValidationError};
use crate::core::result::{InitializeResult, InteractionRequest};
use crate::traits::FlowProtocol;

use super::config::{AuthStyle, GrantType, OAuth2Config};
use super::state::OAuth2State;

/// OAuth2 flow protocol.
///
/// Supports Authorization Code, Client Credentials, and Device Code grant types.
/// Use via `#[credential(extends = OAuth2Protocol)]` + `#[oauth2(...)]`.
pub struct OAuth2Protocol;

impl FlowProtocol for OAuth2Protocol {
    type Config = OAuth2Config;
    type State = OAuth2State;

    fn parameters() -> ParameterCollection {
        let mut client_id = TextParameter::new("client_id", "Client ID");
        client_id.metadata.required = true;

        let mut client_secret = SecretParameter::new("client_secret", "Client Secret");
        client_secret.metadata.required = true;

        ParameterCollection::new()
            .with(ParameterDef::Text(client_id))
            .with(ParameterDef::Secret(client_secret))
    }

    async fn initialize(
        config: &Self::Config,
        values: &ParameterValues,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        let client_id = values.get_string("client_id")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing client_id".into()),
            })?;

        match config.grant_type {
            GrantType::AuthorizationCode => {
                let redirect_url = build_auth_url(config, client_id);
                Ok(InitializeResult::RequiresInteraction(
                    InteractionRequest::Redirect {
                        url: redirect_url,
                        validation_params: HashMap::new(),
                        metadata: HashMap::new(),
                    },
                ))
            }
            GrantType::ClientCredentials => {
                let client_secret = values.get_string("client_secret")
                    .ok_or_else(|| CredentialError::Validation {
                        source: ValidationError::InvalidFormat("missing client_secret".into()),
                    })?;
                let state = exchange_client_credentials(config, client_id, client_secret).await?;
                Ok(InitializeResult::Complete(state))
            }
            GrantType::DeviceCode => {
                // Device flow: request device code, return DisplayInfo
                let device_resp = request_device_code(config, client_id).await?;
                Ok(InitializeResult::RequiresInteraction(device_resp))
            }
        }
    }

    async fn refresh(
        config: &Self::Config,
        state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        let refresh_token = state.refresh_token.clone().ok_or_else(|| {
            CredentialError::Validation {
                source: ValidationError::InvalidFormat("no refresh_token available".into()),
            }
        })?;

        // POST to token_url with grant_type=refresh_token
        let client = reqwest::Client::new();
        let mut params = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token),
        ];
        if matches!(config.auth_style, AuthStyle::PostBody) {
            // client_id/secret not available here — caller must pass via Config extension
            // For now: PostBody auth requires refresh to be called with client creds in state metadata
            // This is a known limitation noted in docs
        }
        let resp = client
            .post(&config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| CredentialError::Validation {
                source: ValidationError::InvalidFormat(format!("refresh request failed: {e}")),
            })?;

        let token_resp: serde_json::Value = resp.json().await.map_err(|e| {
            CredentialError::Validation {
                source: ValidationError::InvalidFormat(format!("invalid token response: {e}")),
            }
        })?;

        update_state_from_token_response(state, &token_resp);
        Ok(())
    }
}

fn build_auth_url(config: &OAuth2Config, client_id: &str) -> String {
    let scopes = config.scopes.join(" ");
    let mut url = format!(
        "{}?response_type=code&client_id={}",
        config.auth_url,
        urlencoding(client_id),
    );
    if !scopes.is_empty() {
        url.push_str(&format!("&scope={}", urlencoding(&scopes)));
    }
    if config.pkce {
        // PKCE: code_challenge would be generated per-request — stub for now
        url.push_str("&code_challenge_method=S256");
    }
    url
}

fn urlencoding(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

async fn exchange_client_credentials(
    config: &OAuth2Config,
    client_id: &str,
    client_secret: &str,
) -> Result<OAuth2State, CredentialError> {
    let client = reqwest::Client::new();
    let scopes = config.scopes.join(" ");
    let mut form: Vec<(&str, String)> = vec![("grant_type", "client_credentials".into())];
    if !scopes.is_empty() {
        form.push(("scope", scopes));
    }

    let req = match config.auth_style {
        AuthStyle::Header => {
            use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
            let credentials = BASE64.encode(format!("{client_id}:{client_secret}"));
            client
                .post(&config.token_url)
                .header("Authorization", format!("Basic {credentials}"))
                .form(&form)
        }
        AuthStyle::PostBody => {
            form.push(("client_id", client_id.into()));
            form.push(("client_secret", client_secret.into()));
            client.post(&config.token_url).form(&form)
        }
    };

    let resp = req.send().await.map_err(|e| CredentialError::Validation {
        source: ValidationError::InvalidFormat(format!("token request failed: {e}")),
    })?;

    let token_resp: serde_json::Value = resp.json().await.map_err(|e| {
        CredentialError::Validation {
            source: ValidationError::InvalidFormat(format!("invalid token response: {e}")),
        }
    })?;

    Ok(state_from_token_response(&token_resp))
}

async fn request_device_code(
    config: &OAuth2Config,
    client_id: &str,
) -> Result<InteractionRequest, CredentialError> {
    use crate::core::result::DisplayData;

    let client = reqwest::Client::new();
    let scopes = config.scopes.join(" ");
    let mut form = vec![("client_id", client_id.to_string())];
    if !scopes.is_empty() {
        form.push(("scope", scopes));
    }

    // device_authorization_endpoint is conventionally derived from auth_url
    // Many providers expose it at /device/code — use auth_url as-is for now
    let resp = client
        .post(&config.auth_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| CredentialError::Validation {
            source: ValidationError::InvalidFormat(format!("device code request failed: {e}")),
        })?;

    let device_resp: serde_json::Value = resp.json().await.map_err(|e| {
        CredentialError::Validation {
            source: ValidationError::InvalidFormat(format!("invalid device response: {e}")),
        }
    })?;

    let user_code = device_resp["user_code"].as_str().unwrap_or("").to_string();
    let verification_url = device_resp["verification_uri"]
        .as_str()
        .or_else(|| device_resp["verification_url"].as_str())
        .unwrap_or("")
        .to_string();
    let expires_in = device_resp["expires_in"].as_u64();

    Ok(InteractionRequest::DisplayInfo {
        display_data: DisplayData::UserCode { code: user_code, verification_url },
        instructions: Some("Visit the URL and enter the code to authorize".into()),
        expires_in,
    })
}

fn state_from_token_response(resp: &serde_json::Value) -> OAuth2State {
    use chrono::Utc;

    let expires_at = resp["expires_in"].as_u64().map(|secs| {
        Utc::now() + chrono::Duration::seconds(secs as i64)
    });

    OAuth2State {
        access_token: resp["access_token"].as_str().unwrap_or("").to_string(),
        token_type: resp["token_type"].as_str().unwrap_or("Bearer").to_string(),
        refresh_token: resp["refresh_token"].as_str().map(str::to_string),
        expires_at,
        scopes: resp["scope"]
            .as_str()
            .map(|s| s.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
    }
}

fn update_state_from_token_response(state: &mut OAuth2State, resp: &serde_json::Value) {
    let new = state_from_token_response(resp);
    state.access_token = new.access_token;
    state.token_type = new.token_type;
    state.expires_at = new.expires_at;
    if let Some(rt) = new.refresh_token {
        state.refresh_token = Some(rt);
    }
    if !new.scopes.is_empty() {
        state.scopes = new.scopes;
    }
}
```

**Step 3: Update `oauth2/mod.rs`**

```rust
pub mod config;
pub mod flow;
pub mod state;

pub use config::{AuthStyle, GrantType, OAuth2Config, OAuth2ConfigBuilder};
pub use flow::OAuth2Protocol;
pub use state::OAuth2State;
```

**Step 4: Run tests**

Run: `cargo test -p nebula-credential protocols::oauth2 -- --nocapture`
Expected: All tests PASS (including authorization_code_returns_redirect)

Run: `cargo check --workspace`
Expected: No errors

**Step 5: Commit**

```bash
git add crates/credential/src/protocols/oauth2/
git commit -m "feat(credential): implement OAuth2Protocol with FlowProtocol"
```

---

## Task 6: Add LDAP and SAML/Kerberos/mTLS stub protocols

**Files:**
- Create: `crates/credential/src/protocols/ldap/mod.rs`
- Create: `crates/credential/src/protocols/ldap/config.rs`
- Create: `crates/credential/src/protocols/saml/mod.rs`
- Create: `crates/credential/src/protocols/kerberos/mod.rs`
- Create: `crates/credential/src/protocols/mtls/mod.rs`
- Modify: `crates/credential/src/protocols/mod.rs`

**Step 1: Write failing tests for LDAP**

```rust
// In ldap/mod.rs tests
#[test]
fn ldap_parameters_has_required_fields() {
    let params = LdapProtocol::parameters();
    assert!(params.contains("host"));
    assert!(params.contains("port"));
    assert!(params.contains("bind_dn"));
    assert!(params.contains("bind_password"));
}

#[test]
fn ldap_config_defaults() {
    let config = LdapConfig::default();
    assert_eq!(config.tls, TlsMode::None);
    assert_eq!(config.timeout.as_secs(), 30);
}
```

Run: `cargo test -p nebula-credential protocols::ldap`
Expected: FAIL

**Step 2: Implement `ldap/config.rs`**

```rust
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// TLS mode for LDAP connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TlsMode {
    /// Plaintext (development only)
    #[default]
    None,
    /// TLS from connection start (ldaps://, port 636)
    Tls,
    /// STARTTLS upgrade on plaintext connection (port 389)
    StartTls,
}

/// LDAP-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapConfig {
    pub tls: TlsMode,
    pub timeout: Duration,
    pub ca_cert: Option<String>,
}

impl Default for LdapConfig {
    fn default() -> Self {
        Self {
            tls: TlsMode::None,
            timeout: Duration::from_secs(30),
            ca_cert: None,
        }
    }
}
```

**Step 3: Implement `ldap/mod.rs`**

```rust
//! LDAP protocol — FlowProtocol stub.
//!
//! Full implementation (using ldap3 crate) is Phase 6.
//! This provides the trait wiring, config types, and State for macro use.

pub mod config;

pub use config::{LdapConfig, TlsMode};

use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{SecretParameter, TextParameter};
use nebula_parameter::values::ParameterValues;
use serde::{Deserialize, Serialize};

use crate::core::{CredentialContext, CredentialError, CredentialState, ValidationError};
use crate::core::result::InitializeResult;
use crate::traits::FlowProtocol;

/// Persisted state after successful LDAP bind.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapState {
    pub host: String,
    pub port: u16,
    pub bind_dn: String,
    pub bind_password: String,
    pub tls: TlsMode,
}

impl CredentialState for LdapState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "ldap";
    fn scrub_ephemeral(&mut self) {}
}

/// LDAP bind protocol.
///
/// # Stub
/// Currently stores credentials for later use by an LDAP Resource.
/// Does not perform network bind yet (planned: Phase 6 with ldap3 crate).
pub struct LdapProtocol;

impl FlowProtocol for LdapProtocol {
    type Config = LdapConfig;
    type State = LdapState;

    fn parameters() -> ParameterCollection {
        let mut host = TextParameter::new("host", "LDAP Host");
        host.metadata.required = true;

        let mut port = TextParameter::new("port", "Port");
        port.metadata.placeholder = Some("389".into());

        let mut bind_dn = TextParameter::new("bind_dn", "Bind DN");
        bind_dn.metadata.required = true;
        bind_dn.metadata.placeholder = Some("cn=admin,dc=example,dc=com".into());

        let mut bind_password = SecretParameter::new("bind_password", "Bind Password");
        bind_password.metadata.required = true;

        ParameterCollection::new()
            .with(ParameterDef::Text(host))
            .with(ParameterDef::Text(port))
            .with(ParameterDef::Text(bind_dn))
            .with(ParameterDef::Secret(bind_password))
    }

    async fn initialize(
        config: &Self::Config,
        values: &ParameterValues,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        let host = values.get_string("host")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing host".into()),
            })?
            .to_owned();

        let port_str = values.get_string("port").unwrap_or("389");
        let port = port_str.parse::<u16>().unwrap_or(389);

        let bind_dn = values.get_string("bind_dn")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing bind_dn".into()),
            })?
            .to_owned();

        let bind_password = values.get_string("bind_password")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing bind_password".into()),
            })?
            .to_owned();

        Ok(InitializeResult::Complete(LdapState {
            host,
            port,
            bind_dn,
            bind_password,
            tls: config.tls,
        }))
    }
}
```

**Step 4: Create SAML, Kerberos, mTLS stubs**

`saml/mod.rs`:

```rust
//! SAML protocol stub — Phase 7.
//!
//! Full implementation requires samael or similar crate.

use serde::{Deserialize, Serialize};

/// SAML request binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SamlBinding {
    #[default]
    HttpPost,
    HttpRedirect,
}

/// SAML configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SamlConfig {
    pub binding: SamlBinding,
    pub sign_requests: bool,
}

// SamlProtocol and SamlState will be added when samael dependency is introduced.
```

`kerberos/mod.rs`:

```rust
//! Kerberos protocol stub — Phase 7.
//!
//! Full implementation requires cross3des or libkrb5 FFI.
```

`mtls/mod.rs`:

```rust
//! mTLS protocol stub — Phase 7.
//!
//! Full implementation requires rustls client cert support.
```

**Step 5: Register in `protocols/mod.rs`**

```rust
pub mod api_key;
pub mod basic_auth;
pub mod database;
pub mod header_auth;
pub mod kerberos;
pub mod ldap;
pub mod mtls;
pub mod oauth2;
pub mod saml;

pub use api_key::{ApiKeyProtocol, ApiKeyState};
pub use basic_auth::{BasicAuthProtocol, BasicAuthState};
pub use database::{DatabaseProtocol, DatabaseState};
pub use header_auth::{HeaderAuthProtocol, HeaderAuthState};
pub use ldap::{LdapConfig, LdapProtocol, LdapState, TlsMode};
pub use oauth2::{AuthStyle, GrantType, OAuth2Config, OAuth2ConfigBuilder, OAuth2Protocol, OAuth2State};
pub use saml::{SamlBinding, SamlConfig};
```

**Step 6: Run tests**

Run: `cargo test -p nebula-credential -- --nocapture`
Expected: All tests PASS

**Step 7: Commit**

```bash
git add crates/credential/src/protocols/
git commit -m "feat(credential): add LdapProtocol, SAML/Kerberos/mTLS stubs"
```

---

## Task 7: Update `#[derive(Credential)]` macro for `FlowProtocol` + sub-attributes

**Files:**
- Modify: `crates/macros/src/credential.rs`

The macro must detect whether `extends = X` refers to a `StaticProtocol` or `FlowProtocol`, and parse `#[oauth2(...)]`, `#[ldap(...)]`, `#[saml(...)]` sub-attributes to generate `OAuth2Config` / `LdapConfig` as local constants and wire the `initialize()` / `Refreshable` impls.

**Step 1: Write failing UI tests**

Create `crates/macros/tests/ui/oauth2_pass.rs`:

```rust
use nebula_macros::Credential;
use nebula_credential::protocols::{OAuth2Protocol, OAuth2Config, AuthStyle, GrantType};
use nebula_credential::traits::CredentialType;

#[derive(Credential)]
#[credential(key = "gh-oauth2", name = "GitHub OAuth2", extends = OAuth2Protocol)]
#[oauth2(
    auth_url   = "https://github.com/login/oauth/authorize",
    token_url  = "https://github.com/login/oauth/access_token",
    scopes     = ["repo", "user"],
    auth_style = PostBody,
)]
pub struct GithubOauth2;

fn _check_type() {
    let _desc = GithubOauth2::description();
}
```

Run: `cargo test -p nebula-macros --test compile_tests -- oauth2_pass`
Expected: FAIL (no `#[oauth2(...)]` support)

**Step 2: Add `#[oauth2(...)]` attribute parsing to macro**

In `crates/macros/src/credential.rs`, the expand function currently parses only `#[credential(...)]`. Add parsing for `#[oauth2(...)]`, `#[ldap(...)]`, `#[saml(...)]` as secondary attribute blocks.

Key changes to `expand()`:

```rust
// After parsing `cred_attrs`, detect protocol family:
// If `extends` is `OAuth2Protocol` — look for #[oauth2(...)] attribute
// If `extends` is `LdapProtocol` — look for #[ldap(...)] attribute
// If `extends` is `SamlProtocol` — look for #[saml(...)] attribute

// Parse #[oauth2(...)] attribute if present
let oauth2_attrs = attrs::parse_attrs(&input.attrs, "oauth2")?;
let ldap_attrs = attrs::parse_attrs(&input.attrs, "ldap")?;

// For OAuth2 FlowProtocol, generate:
// 1. const CONFIG: OAuth2Config = ...;
// 2. impl CredentialType { type State = OAuth2State; initialize() -> delegates to OAuth2Protocol::initialize(&CONFIG, ...) }
// 3. impl Refreshable { refresh() -> delegates to OAuth2Protocol::refresh(&CONFIG, ...) }

let is_oauth2 = extends_type.as_ref()
    .map(|t| quote!(#t).to_string().contains("OAuth2Protocol"))
    .unwrap_or(false);

let is_ldap = extends_type.as_ref()
    .map(|t| quote!(#t).to_string().contains("LdapProtocol"))
    .unwrap_or(false);
```

For OAuth2 — generate config const and impl blocks:

```rust
if is_oauth2 {
    let auth_url = oauth2_attrs.require_string("auth_url", struct_name)?;
    let token_url = oauth2_attrs.require_string("token_url", struct_name)?;
    let scopes = oauth2_attrs.get_string_list("scopes");
    let auth_style = oauth2_attrs.get_ident("auth_style")
        .map(|i| {
            let s = i.to_string();
            match s.as_str() {
                "PostBody" => quote! { ::nebula_credential::protocols::oauth2::AuthStyle::PostBody },
                _ => quote! { ::nebula_credential::protocols::oauth2::AuthStyle::Header },
            }
        })
        .unwrap_or_else(|| quote! { ::nebula_credential::protocols::oauth2::AuthStyle::Header });
    let pkce = oauth2_attrs.get_bool("pkce").unwrap_or(false);

    // resolved_state = OAuth2State
    // resolved_input = ParameterValues
    // config const generation
    // initialize_body delegates to OAuth2Protocol::initialize
    // Refreshable impl block
}
```

The `attrs::parse_attrs` helper in `crates/macros/src/support/attrs.rs` already exists — check if it supports list parsing (`scopes = ["a", "b"]`). If not, add `get_string_list` that parses bracket-delimited string literals.

**Step 3: Add `get_string_list` to attrs support if missing**

In `crates/macros/src/support/attrs.rs`, add:

```rust
/// Parse `key = ["a", "b", "c"]` as `Vec<String>`
pub fn get_string_list(&self, key: &str) -> Vec<String> {
    // Parse from the raw token stream stored in self
    // Look for key followed by `=` followed by `[`, then string literals separated by `,`
    // Return empty vec if not found
    todo!()
}
```

Look at how `attrs.rs` currently works (read it), then add the method appropriately.

Run: `cargo test -p nebula-macros --test compile_tests -- oauth2_pass`
Expected: PASS

**Step 4: Add `#[ldap(...)]` attribute support**

Same pattern as OAuth2 but for `LdapProtocol`:

```rust
if is_ldap {
    let tls_str = ldap_attrs.get_ident("tls").map(|i| i.to_string());
    let tls = match tls_str.as_deref() {
        Some("Tls") => quote! { ::nebula_credential::protocols::ldap::TlsMode::Tls },
        Some("StartTls") => quote! { ::nebula_credential::protocols::ldap::TlsMode::StartTls },
        _ => quote! { ::nebula_credential::protocols::ldap::TlsMode::None },
    };
    let timeout = ldap_attrs.get_u64("timeout_secs").unwrap_or(30);
    // generate LdapConfig const, impl CredentialType delegating to LdapProtocol::initialize
}
```

**Step 5: Run UI tests for all new cases**

Run: `cargo test -p nebula-macros --test compile_tests -- --nocapture`
Expected: All UI tests PASS

**Step 6: Commit**

```bash
git add crates/macros/src/credential.rs crates/macros/src/support/ \
        crates/macros/tests/ui/
git commit -m "feat(macros): add FlowProtocol support with #[oauth2(...)], #[ldap(...)] attributes"
```

---

## Task 8: Update SDK prelude + exports

**Files:**
- Modify: `crates/sdk/src/prelude.rs`

**Step 1: Update prelude**

Current credential exports:

```rust
pub use nebula_credential::{
    core::CredentialContext, core::CredentialDescription, core::CredentialError,
    core::CredentialState, protocols::ApiKeyProtocol, protocols::ApiKeyState,
    traits::CredentialProtocol, traits::CredentialType, traits::Refreshable, traits::Revocable,
};
```

Updated:

```rust
pub use nebula_credential::{
    // Core
    core::CredentialContext,
    core::CredentialDescription,
    core::CredentialError,
    core::CredentialState,
    // Traits
    traits::StaticProtocol,
    traits::FlowProtocol,
    traits::CredentialResource,
    traits::CredentialType,
    traits::Refreshable,
    traits::Revocable,
    // Protocols — StaticProtocol
    protocols::ApiKeyProtocol,
    protocols::ApiKeyState,
    protocols::BasicAuthProtocol,
    protocols::BasicAuthState,
    protocols::HeaderAuthProtocol,
    protocols::HeaderAuthState,
    protocols::DatabaseProtocol,
    protocols::DatabaseState,
    // Protocols — FlowProtocol
    protocols::OAuth2Protocol,
    protocols::OAuth2Config,
    protocols::OAuth2ConfigBuilder,
    protocols::OAuth2State,
    protocols::AuthStyle,
    protocols::GrantType,
    protocols::LdapProtocol,
    protocols::LdapConfig,
    protocols::LdapState,
    protocols::TlsMode,
    protocols::SamlBinding,
    protocols::SamlConfig,
};
```

**Step 2: Run check**

Run: `cargo check -p nebula-sdk`
Expected: No errors

**Step 3: Commit**

```bash
git add crates/sdk/src/prelude.rs
git commit -m "feat(sdk): export all credential protocols in prelude"
```

---

## Task 9: Update github plugin — add `GithubOauth2` credential

**Files:**
- Modify: `plugins/github/src/credentials/github_api.rs` (or add `github_oauth2.rs`)
- Modify: `plugins/github/src/credentials/mod.rs`
- Modify: `plugins/github/src/lib.rs`
- Modify: `plugins/github/Cargo.toml` (if new deps needed)

**Step 1: Create `plugins/github/src/credentials/github_oauth2.rs`**

```rust
use nebula_sdk::prelude::{Credential, OAuth2Protocol};

#[derive(Credential)]
#[credential(
    key = "github-oauth2",
    name = "GitHub OAuth2",
    description = "Authenticate with GitHub via OAuth2 authorization code flow",
    extends = OAuth2Protocol,
)]
#[oauth2(
    auth_url   = "https://github.com/login/oauth/authorize",
    token_url  = "https://github.com/login/oauth/access_token",
    scopes     = ["repo", "user", "workflow"],
    auth_style = PostBody,
)]
pub struct GithubOauth2;
```

**Step 2: Update `credentials/mod.rs`**

```rust
pub mod github_api;
pub mod github_oauth2;

pub use github_api::GithubApi;
pub use github_oauth2::GithubOauth2;
```

**Step 3: Update `lib.rs`**

```rust
pub mod credentials;
pub use credentials::{GithubApi, GithubOauth2};
```

**Step 4: Run tests**

Run: `cargo check -p nebula-github`
Expected: No errors

Run: `cargo test -p nebula-github -- --nocapture`
Expected: All tests PASS

**Step 5: Commit**

```bash
git add plugins/github/
git commit -m "feat(github): add GithubOauth2 credential using OAuth2Protocol"
```

---

## Task 10: Full workspace verification

**Files:** None (verification only)

**Step 1: Format**

Run: `cargo fmt --all`
Expected: No output (already formatted)

**Step 2: Clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings or errors

**Step 3: Full test suite**

Run: `cargo test --workspace -- --nocapture`
Expected: All tests PASS

**Step 4: Doc build**

Run: `cargo doc --no-deps --workspace`
Expected: No errors

**Step 5: Audit**

Run: `cargo audit`
Expected: No vulnerabilities

**Step 6: Final commit if any fmt/clippy fixes were needed**

```bash
git add -A
git commit -m "chore: fmt and clippy fixes after protocol system implementation"
```

---

## Summary

| Task | Scope | Tests Added |
|------|-------|------------|
| 1 | Rename `CredentialProtocol` → `StaticProtocol` | regression (existing 6) |
| 2 | Add `FlowProtocol` + `CredentialResource` traits | doc-test compile check |
| 3 | `BasicAuthProtocol`, `HeaderAuthProtocol`, `DatabaseProtocol` | 8 unit tests |
| 4 | `OAuth2Config`, `OAuth2State`, enums | 7 unit tests |
| 5 | `OAuth2Protocol` (FlowProtocol, HTTP) | 2 async tests |
| 6 | `LdapProtocol`, SAML/Kerberos/mTLS stubs | 2 unit tests |
| 7 | Macro: `#[oauth2(...)]`, `#[ldap(...)]` | UI compile tests |
| 8 | SDK prelude exports | cargo check |
| 9 | GitHub plugin `GithubOauth2` | cargo check + test |
| 10 | Full workspace verification | all |
