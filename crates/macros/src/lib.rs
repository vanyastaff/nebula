//! # Nebula Macros
//!
//! Proc-macros for the Nebula workflow engine.
//!
//! ## Derive Macros
//!
//! | Macro | Description |
//! |-------|-------------|
//! | [`Action`](derive@Action) | Implements the `Action` trait |
//! | [`Resource`](derive@Resource) | Implements the `Resource` trait |
//! | [`Plugin`](derive@Plugin) | Implements the `Plugin` trait |
//! | [`Credential`](derive@Credential) | Implements the `Credential` trait |
//! | [`Parameters`](derive@Parameters) | Generates parameter definitions |
//! | [`Validator`](derive@Validator) | Implements field-based validation |
//! | [`Config`](derive@Config) | Loads from env and validates fields |
//!
//! ## Examples
//!
//! ```ignore
//! use nebula_macros::{Action, Parameters};
//!
//! #[derive(Action)]
//! #[action(
//!     key = "http.request",
//!     name = "HTTP Request",
//!     description = "Make HTTP requests to external APIs"
//! )]
//! pub struct HttpRequestAction {
//!     #[action(config)]
//!     config: HttpConfig,
//! }
//!
//! #[derive(Parameters)]
//! pub struct HttpRequestInput {
//!     #[param(description = "URL to request", required = true)]
//!     url: String,
//!
//!     #[param(description = "HTTP method", default = "GET")]
//!     method: String,
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod action;
mod config;
mod credential;
mod parameter;
mod plugin;
mod resource;
mod support;
mod types;
mod validator;

/// Derive macro for the `Action` trait.
///
/// # Attributes
///
/// ## Container attributes (`#[action(...)]` on the struct)
///
/// - `key = "..."` - Unique identifier for this action (required)
/// - `name = "..."` - Human-readable name (required)
/// - `description = "..."` - Short description (required)
/// - `version = "..."` - Interface version, e.g., "1.0" (default: "1.0")
/// - `credential = Type` - Single credential type for `ActionComponents` (optional)
/// - `credentials = [Type1, Type2]` - Multiple credential types (optional)
/// - `resource = Type` - Single resource type for `ActionComponents` (optional)
/// - `resources = [Type1, Type2]` - Multiple resource types (optional)
/// - `parameters = Type` - Type with `parameters()` for `ActionMetadata` (optional)
///
/// Note: `credential = "key"` (string) is ignored; use `credential = CredentialType` for type-based refs.
///
/// Action structs must be unit structs with no fields (e.g. `struct MyAction;`).
///
/// # Example
///
/// ```ignore
/// #[derive(Action)]
/// #[action(
///     key = "slack.send",
///     name = "Send Slack Message",
///     description = "Sends a message to a Slack channel",
///     version = "2.1",
///     credential = SlackOAuthCredential,
///     resources = [HttpClient]
/// )]
/// pub struct SlackSendAction;
/// ```
#[proc_macro_derive(Action, attributes(action, nebula))]
pub fn derive_action(input: TokenStream) -> TokenStream {
    action::derive(input)
}

/// Derive macro for the `Resource` trait.
///
/// # Attributes
///
/// ## Container attributes (`#[resource(...)]` on the struct)
///
/// - `id = "..."` - Unique resource identifier (required)
/// - `config = Type` - Associated config type (required)
/// - `instance = Type` - Associated instance type (default: Self)
///
/// # Example
///
/// ```ignore
/// #[derive(Resource)]
/// #[resource(
///     id = "postgres",
///     config = PgConfig,
///     instance = PgPool
/// )]
/// pub struct PostgresResource;
/// ```
#[proc_macro_derive(Resource, attributes(resource))]
pub fn derive_resource(input: TokenStream) -> TokenStream {
    resource::derive(input)
}

/// Derive macro for the `Plugin` trait.
///
/// # Attributes
///
/// ## Container attributes (`#[plugin(...)]` on the struct)
///
/// - `key = "..."` - Unique plugin key (required)
/// - `name = "..."` - Human-readable name (required)
/// - `description = "..."` - Short description (optional)
/// - `version = N` - Version number (default: 1)
/// - `group = [...]` - Group hierarchy for UI (optional)
///
/// # Example
///
/// ```ignore
/// #[derive(Plugin)]
/// #[plugin(
///     key = "http",
///     name = "HTTP",
///     description = "HTTP request actions",
///     version = 2,
///     group = ["network", "api"]
/// )]
/// pub struct HttpPlugin;
/// ```
#[proc_macro_derive(Plugin, attributes(plugin))]
pub fn derive_plugin(input: TokenStream) -> TokenStream {
    plugin::derive(input)
}

/// Derive macro for the `Credential` trait.
///
/// # Attributes
///
/// ## Container attributes (`#[credential(...)]` on the struct)
///
/// - `key = "..."` - Unique credential type key (required)
/// - `name = "..."` - Human-readable name (required)
/// - `description = "..."` - Short description (required)
/// - `input = Type` - Input type for initialization (required)
/// - `state = Type` - State type for persistence (required)
///
/// # Example
///
/// ```ignore
/// #[derive(Credential)]
/// #[credential(
///     key = "api_key",
///     name = "API Key",
///     description = "Simple API key authentication",
///     input = ApiKeyInput,
///     state = ApiKeyState
/// )]
/// pub struct ApiKeyCredential;
///
/// #[derive(Serialize, Deserialize)]
/// pub struct ApiKeyInput {
///     pub key: String,
/// }
///
/// #[derive(Clone, Serialize, Deserialize)]
/// pub struct ApiKeyState {
///     pub key: String,
///     pub created_at: DateTime<Utc>,
/// }
/// ```
#[proc_macro_derive(Credential, attributes(credential, oauth2, ldap))]
pub fn derive_credential(input: TokenStream) -> TokenStream {
    credential::derive(input)
}

/// Derive macro for generating parameter definitions.
///
/// Generates `ParameterCollection` from struct fields and their attributes.
///
/// # Field Attributes
///
/// - `#[param(description = "...")]` - Field description (optional)
/// - `#[param(required)]` - Marks the field as required (default: optional)
/// - `#[param(secret)]` - Marks the field as sensitive data
/// - `#[param(default = ...)]` - Default value for the field
/// - `#[param(validation = "...")]` - Validation rule (email, url, regex, range)
/// - `#[param(options = [...])]` - Select options
///
/// # Example
///
/// ```ignore
/// #[derive(Parameters)]
/// pub struct DatabaseConfig {
///     #[param(description = "Database host", required, default = "localhost")]
///     host: String,
///
///     #[param(description = "Port number", validation = "range(1, 65535)", default = 5432)]
///     port: u16,
///
///     #[param(description = "Password", secret)]
///     password: String,
///
///     #[param(description = "Log level", options = ["debug", "info", "warn", "error"])]
///     log_level: String,
/// }
/// ```
#[proc_macro_derive(Parameters, attributes(param))]
pub fn derive_parameters(input: TokenStream) -> TokenStream {
    parameter::derive(input)
}

/// Derive macro for generating field-based validators.
///
/// Implements `nebula_validator::foundation::Validate` for the struct and
/// generates an inherent `validate_fields()` helper.
///
/// # Container attributes (`#[validator(...)]`)
///
/// - `message = "..."` - Root error message when multiple field errors are aggregated
///
/// # Field attributes (`#[validate(...)]`)
///
/// ## Size / length
/// - `required` - `Option<T>` must be `Some`
/// - `min_length = N` - string `len()` must be at least `N`
/// - `max_length = N` - string `len()` must be at most `N`
/// - `exact_length = N` - string `len()` must be exactly `N`
/// - `length_range(min = A, max = B)` - string `len()` must be within range `[A, B]`
/// - `min = N` - numeric value must be `>= N`
/// - `max = N` - numeric value must be `<= N`
/// - `min_size = N` - collection length must be at least `N` (`Vec<T>` / `Option<Vec<T>>`)
/// - `max_size = N` - collection length must be at most `N` (`Vec<T>` / `Option<Vec<T>>`)
/// - `exact_size = N` - collection length must be exactly `N` (`Vec<T>` / `Option<Vec<T>>`)
/// - `not_empty_collection` - collection must not be empty (`Vec<T>` / `Option<Vec<T>>`)
/// - `size_range(min = A, max = B)` - collection length must be within range `[A, B]`
///
/// ## Format flags (operate on `String` / `Option<String>` fields)
/// - `not_empty` - string must not be empty
/// - `alphanumeric` - letters and digits only
/// - `alphabetic` - letters only
/// - `numeric` - digits only
/// - `lowercase` - alphabetic characters must be lowercase
/// - `uppercase` - alphabetic characters must be uppercase
/// - `email` - valid email address
/// - `url` - valid HTTP/HTTPS URL
/// - `ipv4` - valid IPv4 address
/// - `ipv6` - valid IPv6 address
/// - `ip_addr` - valid IPv4 or IPv6 address
/// - `hostname` - valid hostname (RFC 1123)
/// - `uuid` - valid UUID (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`)
/// - `date` - valid date (`YYYY-MM-DD`)
/// - `date_time` - valid RFC 3339 date-time
/// - `time` - valid time (`HH:MM:SS`)
///
/// ## Pattern
/// - `regex = "pattern"` - string must match the given regex
/// - `contains = "..."` - string must contain substring
/// - `starts_with = "..."` - string must start with prefix
/// - `ends_with = "..."` - string must end with suffix
/// - `message = "..."` - override message for this field's validation errors
/// - `is_true` - boolean field must be `true`
/// - `is_false` - boolean field must be `false`
///
/// ## Advanced
/// - `nested` - validates nested fields via `SelfValidating::check()`
/// - `custom = path::to::fn` or `custom = "path::to::fn"` - custom validator fn
///   with signature `fn(&T) -> Result<(), ValidationError>`
/// - `each(...)` - applies rules to each element of `Vec<T>` / `Option<Vec<T>>`
///   (supports the same flags and key-value entries as field rules where applicable, including
///   `exact_length`, `contains`, `starts_with`, `ends_with`, and `not_empty` for string elements)
///
/// The macro also generates `SelfValidating` for the struct automatically.
///
/// # Example
///
/// ```ignore
/// use nebula_macros::Validator;
/// use nebula_validator::foundation::Validate;
///
/// #[derive(Validator)]
/// #[validator(message = "invalid webhook config")]
/// struct WebhookConfig {
///     #[validate(url)]
///     endpoint: String,
///
///     #[validate(min_length = 8)]
///     secret: String,
///
///     #[validate(email)]
///     notify: Option<String>,
///
///     #[validate(regex = r"^v\d+$")]
///     version: String,
///
///     #[validate(custom = "crate::must_be_even")]
///     retries: u32,
/// }
/// ```
#[proc_macro_derive(Validator, attributes(validator, validate))]
pub fn derive_validator(input: TokenStream) -> TokenStream {
    validator::derive(input)
}

/// Derive macro for env-backed configuration types with field validation.
///
/// Generates:
/// - `from_env()` and `from_env_with_prefix(prefix)` constructors
/// - `validate_fields()` helper
/// - `nebula_validator::foundation::Validate<Self>` implementation
///
/// # Container attributes (`#[config(...)]`)
///
/// - `source = "env" | "dotenv" | "file"` - single source selector (`from` also accepted)
/// - `sources = ["dotenv", "file", "env"]` - ordered loader chain (`loaders` also accepted)
/// - `prefix = "..."` - env prefix (for example `NEBULA_APP`)
/// - `path = "..."` - base file path for `dotenv`/`file` loaders (`file` also accepted)
/// - `profile_var = "APP_ENV"` - env var used to resolve active profile (`profile_env` also accepted)
/// - `profile = "dev"` - default profile when profile env var is missing
/// - `separator = "..."` - segment separator between prefix and key (default: `_`)
/// - `file` loader format by extension: `.json`, `.toml`, `.yaml`/`.yml`, `.env`
///
/// # Field attributes
///
/// - `#[config(key = "...")]` - explicit config key (`name`/`env` also accepted)
/// - `#[config(default = ...)]` - field-level default value override
/// - `#[validate(...)]` - same rules as `#[derive(Validator)]`
///
/// # Example
///
/// ```ignore
/// use nebula_macros::Config;
///
/// #[derive(Config, Default)]
/// #[config(sources = ["dotenv", "env"], path = ".env", prefix = "NEBULA_APP")]
/// struct AppConfig {
///     #[validate(min = 1, max = 65535)]
///     port: u16,
///
///     #[config(key = "NEBULA_APP_ADMIN_EMAIL")]
///     #[validate(email)]
///     admin_email: String,
/// }
/// ```
#[proc_macro_derive(Config, attributes(config, validator, validate))]
pub fn derive_config(input: TokenStream) -> TokenStream {
    config::derive(input)
}
