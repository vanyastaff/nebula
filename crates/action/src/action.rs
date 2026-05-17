//! Base [`Action`] trait — identity, metadata, type-level Input/Output, slot deps.
//!
//! See module docs in `lib.rs` for trait family overview. Variant A trait shape
//! lands per ADR-0043 §6 (Phase 3 / Session 1).

use nebula_core::Dependencies;
use nebula_schema::HasSchema;
use serde::{Serialize, de::DeserializeOwned};

use crate::metadata::ActionMetadata;

/// Base trait for all action types.
///
/// Identity (`metadata`), type-level input/output (`Self::Input`,
/// `Self::Output`), and slot-binding declarations (`dependencies`) — all
/// static per concrete type. The schema is reached via the
/// `Input`/`Output: HasSchema` bound (`nebula_schema::schema_of::<A::Input>()`);
/// there is no per-trait schema method (ADR-0052 P3). Sub-traits
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
///     fn dependencies() -> &'static Dependencies {
///         static D: OnceLock<Dependencies> = OnceLock::new();
///         D.get_or_init(Dependencies::new)
///     }
/// }
/// ```
///
/// The input/output schema is obtained from the associated type, e.g.
/// `nebula_schema::schema_of::<<Echo as Action>::Input>()`.
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

    /// Slot-binding declarations (`#[resource]` / `#[credential]` fields, Phase 3 / S3+).
    fn dependencies() -> &'static Dependencies;
}
