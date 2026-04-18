//! # nebula-sdk — Integration Author SDK
//!
//! Single-crate façade for writing Nebula integrations. Re-exports the full
//! integration surface (`nebula-action`, `nebula-credential`, `nebula-resource`,
//! `nebula-schema`, `nebula-workflow`, `nebula-plugin`, `nebula-validator`) and
//! adds `prelude`, `WorkflowBuilder`, `ActionBuilder`, and `TestRuntime`.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use nebula_core::action_key;
//! use nebula_sdk::prelude::*;
//!
//! let metadata = ActionBuilder::new(action_key!("example.greet"), "Greet")
//!     .with_description("A simple greeting action")
//!     .build();
//!
//! let workflow = WorkflowBuilder::new("example_workflow")
//!     .add_node("greet", "example_greet")
//!     .build();
//!
//! assert_eq!(metadata.base.name, "Greet");
//! assert!(workflow.is_ok());
//! ```
//!
//! ## Modules
//!
//! - `prelude` — one-stop import for common types and traits.
//! - `action` — `ActionBuilder` for programmatic action metadata.
//! - `workflow` — `WorkflowBuilder` for programmatic workflow construction.
//! - `runtime` — `TestRuntime`, `RunReport` — in-process test harness.
//! - `testing` (feature `testing`) — test helpers and fixtures.
//!
//! ## Canon
//!
//! - §3.5 integration model: Action, Credential, Resource, Schema, Plugin.
//! - §4.4 DX: stable `prelude` + `WorkflowBuilder` API is a public contract.
//! - §7 open source contract: breaking changes need explicit announcement.
//!
//! See `crates/sdk/README.md` for the full re-export list and maturity notes.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

// Re-export core crates
// Re-export commonly used external crates
pub use anyhow;
pub use async_trait::async_trait;
pub use nebula_action;
pub use nebula_core;
pub use nebula_credential;
pub use nebula_plugin;
pub use nebula_resource;
pub use nebula_schema;
pub use nebula_validator;
pub use nebula_workflow;
pub use serde;
pub use serde_json;
pub use thiserror;
// Re-export tokio when needed for async
#[cfg(feature = "testing")]
pub use tokio;

pub mod action;
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
///     "name": "test",
///     "count": 42
/// });
/// ```
pub use serde_json::json;

/// Helper macro for creating parameter collections.
///
/// # Examples
///
/// ```ignore
/// use nebula_sdk::params;
///
/// let params = params! {
///     "name" => "test",
///     "count" => 42
/// };
/// ```
#[macro_export]
macro_rules! params {
    ($($key:expr => $value:expr),* $(,)?) => {{
        use $crate::nebula_schema::value::FieldValues;
        use $crate::serde_json::json;

        let mut values = FieldValues::new();
        $(
            values.set_raw($key, json!($value));
        )*
        values
    }};
}

/// Macro for defining a workflow.
///
/// # Examples
///
/// ```ignore
/// use nebula_sdk::workflow;
///
/// let wf = workflow! {
///     name: "my_workflow",
///     nodes: [
///         start: StartNode => process,
///         process: ProcessAction => end,
///         end: EndNode
///     ]
/// };
/// ```
#[macro_export]
macro_rules! workflow {
    (
        name: $name:expr,
        nodes: [
            $($node_key:ident: $action:ty $(=> $next:ident)?),* $(,)?
        ]
    ) => {{
        use $crate::workflow::WorkflowBuilder;

        let mut builder = WorkflowBuilder::new($name);
        $(
            builder = builder.add_node(
                stringify!($node_key),
                stringify!($action)
            );
            $(
                builder = builder.connect(stringify!($node_key), stringify!($next));
            )?
        )*
        builder
    }};
}

/// Macro for defining a simple action.
///
/// # Examples
///
/// ```ignore
/// use nebula_sdk::simple_action;
///
/// simple_action! {
///     name: LogAction,
///     key: "debug.log",
///     input: LogInput,
///     output: LogOutput,
///     async fn execute(&self, input, ctx) {
///         println!("Log: {}", input.message);
///         Ok(LogOutput { success: true })
///     }
/// }
/// ```
#[macro_export]
macro_rules! simple_action {
    (
        name: $name:ident,
        key: $key:expr,
        input: $input:ty,
        output: $output:ty,
        async fn execute(&$self:tt, $input_param:ident, $ctx_param:ident) $body:block
    ) => {
        #[derive($crate::nebula_action::Action)]
        #[action(
            key = $key,
            name = stringify!($name),
            description = ""
        )]
        pub struct $name;

        #[::async_trait::async_trait]
        impl $crate::nebula_action::ProcessAction for $name {
            type Input = $input;
            type Output = $output;

            async fn execute(
                &$self,
                $input_param: Self::Input,
                $ctx_param: &$crate::nebula_action::ActionContext,
            ) -> ::std::result::Result<
                $crate::nebula_action::ActionResult<Self::Output>,
                $crate::nebula_action::ActionError
            > {
                $body
            }
        }
    };
}
