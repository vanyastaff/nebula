//! # nebula-sdk — Integration Author SDK
//!
//! Single-crate façade for writing Nebula integrations. Its current external
//! one-dependency proof covers `WorkflowBuilder`, `ActionBuilder`, and credential
//! `TestResult`; other manual/prelude workflows require focused proofs.
//! Procedural derives remain an explicit SDK gap.
//!
//! Internal workspace crates are not re-exported. Use curated persona modules
//! and the prelude so integrations remain insulated from crate-boundary
//! refactors. In particular, use
//! `nebula_sdk::integration::credential::{TestFailureCode, TestResult}` for
//! credential-test outcomes instead of `nebula_sdk::nebula_credential`.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use nebula_sdk::prelude::*;
//!
//! let metadata = ActionBuilder::new(action_key!("example.greet"), "Greet")
//!.with_description("A simple greeting action")
//!.build();
//!
//! let workflow = WorkflowBuilder::new("example_workflow")
//!.add_node("greet", "core", "example_greet")
//!.build();
//!
//! assert_eq!(metadata.base.name, "Greet");
//! assert!(workflow.is_ok());
//! ```
//!
//! ## Modules
//!
//! - `prelude` — one-stop import for common types and traits.
//! - `integration` — curated contracts for integration authors.
//! - `action` — `ActionBuilder` for programmatic action metadata.
//! - `workflow` — `WorkflowBuilder` for programmatic workflow construction.
//! - `runtime` — `TestRuntime`, `RunReport` — in-process test harness.
//! - `testing` (feature `testing`) — test helpers and fixtures.
//!
//! ## Canon
//!
//! - §3.5 integration model: Action, Credential, Resource, Schema, Plugin.
//! - §4.4 DX: curated persona APIs and builders are public contracts.
//! - §7 open source contract: breaking changes need explicit announcement.
//!
//! See `crates/sdk/README.md` for persona APIs and maturity notes.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

// Ecosystem conveniences intentionally re-exported by the SDK. Nebula's
// internal crate topology is not: curated modules and the prelude are the
// supported surface.
pub use serde;
pub use serde_json;
pub use thiserror;
// Re-export tokio when needed for async
#[cfg(feature = "testing")]
pub use tokio;

/// Macro implementation paths. This is public only because exported macros
/// expand in downstream crates; it is hidden from documentation and is not a
/// supported integration persona.
#[doc(hidden)]
pub mod __private {
    /// Exact action items required by exported macro expansions.
    #[doc(hidden)]
    pub mod action {
        pub use nebula_action::{
            Action, ActionContext, ActionError, ActionMetadata, ActionResult, StatelessAction,
        };
    }

    /// Exact core items required by exported macro expansions.
    #[doc(hidden)]
    pub mod core {
        pub use nebula_core::{Dependencies, action_key};
    }

    /// Exact schema items required by exported macro expansions.
    #[doc(hidden)]
    pub mod schema {
        pub use nebula_schema::value::FieldValues;
    }
}

pub mod action;
pub mod integration;
pub mod prelude;
pub mod runtime;
pub mod workflow;

pub use runtime::{RunReport, TestRuntime};

#[cfg(feature = "testing")]
pub mod testing;

/// Result type alias for SDK operations.
pub type Result<T> = std::result::Result<T, Error>;

/// SDK error type.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Workflow building error.
    #[error("workflow error: {0}")]
    Workflow(String),

    /// Action execution error.
    #[error("action error: {0}")]
    Action(String),

    /// Parameter validation error.
    #[error("parameter error: {0}")]
    Parameter(String),

    /// Serialization error.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Generic error.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Create a new workflow error.
    pub fn workflow(msg: impl Into<String>) -> Self {
        Self::Workflow(msg.into())
    }

    /// Create a new action error.
    pub fn action(msg: impl Into<String>) -> Self {
        Self::Action(msg.into())
    }

    /// Create a new parameter error.
    pub fn parameter(msg: impl Into<String>) -> Self {
        Self::Parameter(msg.into())
    }
}

/// Helper macro for creating JSON values.
///
/// # Examples
///
/// ```
/// use nebula_sdk::json;
///
/// let value = json!({
/// "name": "test",
/// "count": 42
/// });
/// ```
pub use serde_json::json;

/// Helper macro for creating parameter collections.
///
/// # Examples
///
/// ```
/// use nebula_sdk::params;
///
/// let values = params! {
///     "name" => "test",
///     "count" => 42,
/// };
/// assert_eq!(values.len(), 2);
/// ```
#[macro_export]
macro_rules! params {
    ($($key:expr => $value:expr),* $(,)?) => {{
        use $crate::__private::schema::FieldValues;
        use $crate::serde_json::json;

        let mut values = FieldValues::new();
        $(
            values.try_set_raw($key, json!($value))
                .expect("params! macro: invalid FieldKey or nested key");
        )*
        values
    }};
}

/// Macro for defining a workflow.
///
/// Each node entry is `node_key: "plugin_key", "action_key"`, registered via
/// [`WorkflowBuilder::add_node`](crate::workflow::WorkflowBuilder::add_node).
/// Use `=> next_node` to declare a default downstream connection (compiled
/// to [`WorkflowBuilder::connect`](crate::workflow::WorkflowBuilder::connect)).
///
/// # Examples
///
/// ```
/// use nebula_sdk::workflow;
///
/// let builder = workflow! {
///     name: "my_workflow",
///     nodes: [
///         fetch: "http", "get" => transform,
///         transform: "json", "map" => store,
///         store: "db", "insert",
///     ]
/// };
/// let wf = builder.build().expect("valid workflow");
/// assert_eq!(wf.nodes.len(), 3);
/// assert_eq!(wf.connections.len(), 2);
/// ```
#[macro_export]
macro_rules! workflow {
    (
        name: $name:expr,
        nodes: [
            $($node_key:ident: $plugin:literal, $action:literal $(=> $next:ident)?),* $(,)?
        ]
    ) => {{
        use $crate::workflow::WorkflowBuilder;

        let mut builder = WorkflowBuilder::new($name);
        $(
            builder = builder.add_node(
                stringify!($node_key),
                $plugin,
                $action,
            );
            $(
                builder = builder.connect(stringify!($node_key), stringify!($next));
            )?
        )*
        builder
    }};
}

/// Macro for defining a simple stateless action with a unit struct.
///
/// Generates a unit `struct $name`, implements [`Action`](nebula_action::Action)
/// (with static metadata + schemas + slot-binding [`Dependencies`](nebula_core::Dependencies)),
/// and implements
/// [`StatelessAction`](nebula_action::StatelessAction) over the supplied
/// `input` / `output` types.
///
/// # Requirements
///
/// `Input` and `Output` must implement [`HasSchema`](nebula_schema::HasSchema).
///
/// # Examples
///
/// See `examples/hello_action.rs` for a runnable end-to-end demo.
///
/// ```
/// use nebula_sdk::{prelude::*, simple_action};
///
/// simple_action! {
///     name: GreetAction,
///     key: "demo.greet",
///     input: serde_json::Value,
///     output: serde_json::Value,
///     async fn execute(&self, input, _ctx) {
///         let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("world");
///         Ok(ActionResult::success(serde_json::json!({
///             "message": format!("Hello, {name}!"),
///         })))
///     }
/// }
///
/// // The generated type carries static metadata; `execute` runs under an engine.
/// assert_eq!(<GreetAction as Action>::metadata().base.name, "GreetAction");
/// ```
#[macro_export]
macro_rules! simple_action {
    (
        name: $name:ident,
        key: $key:literal,
        input: $input:ty,
        output: $output:ty,
        async fn execute(&$self:tt, $input_param:ident, $ctx_param:ident) $body:block
    ) => {
        pub struct $name;

        impl $crate::__private::action::Action for $name {
            type Input = $input;
            type Output = $output;

            fn metadata() -> $crate::__private::action::ActionMetadata {
                $crate::__private::action::ActionMetadata::for_action::<$name>(
                    $crate::__private::core::action_key!($key),
                    stringify!($name),
                    "",
                )
            }

            fn dependencies() -> &'static $crate::__private::core::Dependencies {
                static DEPS: ::std::sync::OnceLock<$crate::__private::core::Dependencies> =
                    ::std::sync::OnceLock::new();
                DEPS.get_or_init($crate::__private::core::Dependencies::new)
            }
        }

        impl $crate::__private::action::StatelessAction for $name {
            async fn execute(
                &$self,
                $input_param: <Self as $crate::__private::action::Action>::Input,
                $ctx_param: &(impl $crate::__private::action::ActionContext + ?Sized),
            ) -> ::std::result::Result<
                $crate::__private::action::ActionResult<<Self as $crate::__private::action::Action>::Output>,
                $crate::__private::action::ActionError,
            > {
                $body
            }
        }
    };
}
