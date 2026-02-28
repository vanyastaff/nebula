# nebula-macros

Proc-macros for the Nebula workflow engine.

## Overview

This crate provides derive macros for simplifying the implementation of core Nebula traits:

- `Action` - For workflow actions
- `Resource` - For resource providers
- `Plugin` - For plugin definitions
- `Credential` - For credential types
- `Parameters` - For parameter definitions
- `Validator` - For field-based validation rules
- `Config` - For env-backed config loading + validation

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-macros = { path = "../crates/macros" }
```

## Derive Macros

### Action

Derive the `Action` trait for a struct:

```rust
use nebula_macros::Action;

#[derive(Action)]
#[action(
    key = "http.request",
    name = "HTTP Request",
    description = "Make HTTP requests",
    version = "2.1",
    action_type = "process",
    isolation = "sandbox",
    credential = "api_key"
)]
pub struct HttpRequestAction {
    #[action(config)]
    config: HttpConfig,
}
```

**Attributes:**

- `key` - Unique identifier (required)
- `name` - Human-readable name (required)
- `description` - Description (optional, defaults to doc comments)
- `version` - Interface version (default: "1.0")
- `action_type` - One of: `process`, `stateful`, `trigger`, `streaming`, `transactional`, `interactive` (default: `process`)
- `isolation` - Isolation level: `none`, `sandbox`, `process`, `vm` (default: `none`)
- `credential` - Required credential type key (optional)

**Field attributes:**

- `#[action(config)]` - Marks the configuration field
- `#[action(resource)]` - Marks a resource to be injected
- `#[action(skip)]` - Skips the field

### Resource

Derive the `Resource` trait:

```rust
use nebula_macros::Resource;

#[derive(Resource)]
#[resource(
    id = "postgres",
    config = PgConfig,
    instance = PgPool
)]
pub struct PostgresResource;
```

**Attributes:**

- `id` - Unique resource identifier (required)
- `config` - Associated configuration type (required)
- `instance` - Associated instance type (default: `Self`)

### Plugin

Derive the `Plugin` trait:

```rust
use nebula_macros::Plugin;

#[derive(Plugin)]
#[plugin(
    key = "http",
    name = "HTTP",
    description = "HTTP request actions",
    version = 2,
    group = ["network", "api"]
)]
pub struct HttpPlugin;
```

**Attributes:**

- `key` - Unique plugin key (required)
- `name` - Human-readable name (required)
- `description` - Description (optional)
- `version` - Version number (default: 1)
- `group` - Group hierarchy for UI categorization (optional)

### Credential

Derive the `Credential` trait:

```rust
use nebula_macros::Credential;
use serde::{Serialize, Deserialize};

#[derive(Credential)]
#[credential(
    key = "api_key",
    name = "API Key",
    description = "Simple API key authentication",
    input = ApiKeyInput,
    state = ApiKeyState
)]
pub struct ApiKeyCredential;

#[derive(Serialize, Deserialize)]
pub struct ApiKeyInput {
    pub key: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKeyState {
    pub key: String,
}
```

**Attributes:**

- `key` - Unique credential type key (required)
- `name` - Human-readable name (required)
- `description` - Description (required)
- `input` - Input type for initialization (required)
- `state` - State type for persistence (required)

### Parameters

Generate parameter definitions from struct fields:

```rust
use nebula_macros::Parameters;

#[derive(Parameters)]
pub struct DatabaseConfig {
    #[param(description = "Database host", required, default = "localhost")]
    host: String,
    
    #[param(description = "Port number", default = 5432)]
    port: u16,
    
    #[param(description = "Password", secret, required)]
    password: String,
}
```

**Field attributes:**

- `description` - Field description
- `required` - Marks as required
- `secret` - Marks as sensitive data
- `default` - Default value
- `options = [...]` - Select options

### Validator

Generate field-based validation with `nebula-validator`:

```rust
use nebula_macros::Validator;

#[derive(Validator)]
#[validator(message = "input is invalid")]
pub struct UserInput {
    #[validate(required, min_length = 3, max_length = 32)]
    username: Option<String>,

    #[validate(min = 18, max = 120)]
    age: u8,
}
```

**Container attributes:**

- `message` - Root error message for aggregated validation errors

**Field attributes:**

- `required` - Requires `Option<T>` to be `Some`
- `min_length = N` - Requires `len() >= N`
- `max_length = N` - Requires `len() <= N`
- `min = N` - Requires numeric value `>= N`
- `max = N` - Requires numeric value `<= N`

### Config

Generate env-backed config loading and field validation:

```rust
use nebula_macros::Config;

#[derive(Config, Default)]
#[config(
    sources = ["dotenv", "file", "env"],
    path = ".env",
    prefix = "NEBULA_APP",
    profile_var = "APP_ENV"
)]
pub struct AppConfig {
    #[validate(min = 1, max = 65535)]
    port: u16,

    #[config(key = "NEBULA_APP_ADMIN_EMAIL", default = "admin@example.com")]
    #[validate(email)]
    admin_email: String,
}
```

Generated API:

- `from_env() -> Result<Self, String>` (requires `Default`)
- `from_env_with_prefix(Option<&str>) -> Result<Self, String>`
- `validate_fields() -> Result<(), ValidationErrors>`
- `load() -> Result<Self, String>` (ordered loaders chain)
- `load_with_profile(Option<&str>) -> Result<Self, String>`

Container attribute names (standard + compatibility aliases):
- `source` (`from`)
- `sources` (`loaders`)
- `path` (`file`)
- `profile_var` (`profile_env`)

Field attribute names:
- `key` (`name`/`env`)
- `default`

Profile naming follows suffix style:
- `.env` -> `.env.dev`
- `config.json` -> `config.dev.json`

## License

MIT OR Apache-2.0
