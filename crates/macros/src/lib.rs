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
mod credential;
mod parameter;
mod plugin;
mod resource;
mod support;

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
/// - `action_type = "..."` - One of: process, stateful, trigger, streaming, transactional, interactive (default: process)
/// - `isolation = "..."` - Isolation level: none, sandbox, process, vm (default: none)
/// - `credential = "..."` - Required credential type key (optional)
///
/// ## Field attributes
///
/// - `#[action(config)]` - Marks the config field
/// - `#[action(resource)]` - Marks a resource to be injected
/// - `#[action(skip)]` - Skips this field
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
///     credential = "slack_oauth"
/// )]
/// pub struct SlackSendAction {
///     #[action(config)]
///     config: SlackConfig,
///     
///     #[action(resource)]
///     http_client: HttpClient,
/// }
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
#[proc_macro_derive(Credential, attributes(credential))]
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
