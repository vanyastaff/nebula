//! Base [`Action`] trait — identity, metadata, type-level Input/Output, slot deps.
//!
//! See module docs in `lib.rs` for trait family overview. Variant A trait shape
//! lands per ADR-0043 §6 (Phase 3 / Session 1).

use nebula_core::Dependencies;
use nebula_schema::{HasSchema, ValidSchema};
use serde::{Serialize, de::DeserializeOwned};

use crate::metadata::ActionMetadata;

/// Base trait for all action types.
///
/// Identity (`metadata`), type-level input/output (`Self::Input`, `Self::Output`),
/// validation schemas (`input_schema` / `output_schema`), and slot-binding
/// declarations (`dependencies`) — all static per concrete type. Sub-traits
/// ([`StatelessAction`](crate::StatelessAction) etc.) define the execution
/// surface and consume `Self::Input` / `Self::Output`.
///
/// # Object safety
///
/// **Not object-safe.** `dyn Action` does not compile. Engine dispatch uses
/// per-execution factories (`ActionFactory` — Phase 3 / Session 4)
/// returning `Box<dyn ErasedAction>` for JSON-erased dispatch.
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::OnceLock;
/// use nebula_action::{Action, ActionMetadata};
/// use nebula_core::{Dependencies, action_key};
/// use nebula_schema::ValidSchema;
///
/// struct Echo;
///
/// impl Action for Echo {
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///
///     fn metadata() -> &'static ActionMetadata {
///         static M: OnceLock<ActionMetadata> = OnceLock::new();
///         M.get_or_init(|| ActionMetadata::new(action_key!("echo"), "Echo", "Echoes input"))
///     }
///     fn input_schema() -> &'static ValidSchema {
///         static S: OnceLock<ValidSchema> = OnceLock::new();
///         S.get_or_init(<serde_json::Value as nebula_schema::HasSchema>::schema)
///     }
///     fn output_schema() -> &'static ValidSchema {
///         static S: OnceLock<ValidSchema> = OnceLock::new();
///         S.get_or_init(<serde_json::Value as nebula_schema::HasSchema>::schema)
///     }
///     fn dependencies() -> &'static Dependencies {
///         static D: OnceLock<Dependencies> = OnceLock::new();
///         D.get_or_init(Dependencies::new)
///     }
/// }
/// ```
///
/// `#[derive(Action)]` (Phase 3 / Session 3) emits this boilerplate automatically.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as an Action",
    label = "this type does not implement the Action trait",
    note = "derive it: #[derive(Action)]"
)]
pub trait Action: Sized + Send + Sync + 'static {
    /// User-facing form data; deserialized from `node.input_json` per execution.
    type Input: HasSchema + DeserializeOwned + Send + Sync;

    /// What this action produces; serialized to JSON for downstream nodes.
    type Output: HasSchema + Serialize + Send + Sync;

    /// Static metadata describing this action type (key, version, ports, etc.).
    fn metadata() -> &'static ActionMetadata;

    /// Schema describing valid `Self::Input` values.
    fn input_schema() -> &'static ValidSchema;

    /// Schema describing valid `Self::Output` values.
    fn output_schema() -> &'static ValidSchema;

    /// Slot-binding declarations (`#[resource]` / `#[credential]` fields, Phase 3 / S3+).
    fn dependencies() -> &'static Dependencies;
}
