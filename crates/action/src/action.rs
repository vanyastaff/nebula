//! Base [`Action`] trait: the action's interface on top of its [`Component`].
//!
//! See module docs in `lib.rs` for trait family overview.

use core::num::NonZeroU32;

/// The component contract [`Action`] is built on, and the catalog metadata it
/// returns, re-exported next to it.
pub use nebula_component::{Component, kind};
use nebula_core::Dependencies;
pub use nebula_metadata::Metadata;
use nebula_schema::HasSchema;
use serde::Serialize;

use crate::{
    effect::{ActionEffectContract, JournalProtocol},
    metadata::IsolationLevel,
    port::{self, InputPort, OutputPort},
};

/// Base trait for all action types.
///
/// An action is a [`Component`] of kind [`kind::Action`]: the `Component`
/// impl carries its key ([`Component::KEY`]), its form data
/// ([`Component::Config`], decoded from the node's input per execution) and
/// its catalog [`Metadata`]. `Action` adds what
/// only an action has: its output type, its ports, its execution policy and
/// its slot dependencies. All of it is static per concrete type, so one type
/// is one catalog entry. Sub-traits ([`StatelessAction`](crate::StatelessAction)
/// etc.) define the execution surface and consume `Self::Config` /
/// `Self::Output`.
///
/// # Object safety
///
/// **Not object-safe.** `dyn Action` does not compile. Engine dispatch uses
/// per-execution factories (`ActionFactory`) returning an
/// [`ActionHandle`](crate::ActionHandle) for JSON-erased dispatch.
///
/// # Example
///
/// ```rust
/// use std::sync::OnceLock;
/// use nebula_action::{Action, Component, Metadata, kind};
/// use nebula_core::Dependencies;
///
/// struct Echo;
///
/// impl Component for Echo {
///     type Kind = kind::Action;
///     type Config = serde_json::Value;
///
///     const KEY: &'static str = "echo";
///
///     fn metadata() -> Metadata {
///         Metadata::new(nebula_action::metadata_name!("Echo"), "Echoes input")
///     }
/// }
///
/// impl Action for Echo {
///     type Output = serde_json::Value;
///
///     fn dependencies() -> &'static Dependencies {
///         static D: OnceLock<Dependencies> = OnceLock::new();
///         D.get_or_init(Dependencies::new)
///     }
/// }
///
/// // Identity is the type's constant; metadata carries no key...
/// assert_eq!(nebula_action::key_of::<Echo>().as_str(), "echo");
/// let _metadata = Echo::metadata();
/// // ...and the schema is reached through the associated type's `HasSchema`
/// // bound, not a method on the trait itself.
/// let _input_schema =
///     nebula_schema::schema_of::<<Echo as Component>::Config>().expect("valid schema");
/// ```
///
/// `#[derive(Action)]` emits both impls from `#[action(key = "...", ...)]`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as an Action",
    label = "this type does not implement the Action trait",
    note = "derive it: #[derive(Action)]"
)]
pub trait Action: Component<Kind = kind::Action> {
    /// What this action produces; serialized to JSON for downstream nodes.
    type Output: HasSchema + Serialize + Send + Sync;

    /// How isolated this action's execution is. Defaults to
    /// [`IsolationLevel::None`].
    const ISOLATION_LEVEL: IsolationLevel = IsolationLevel::None;

    /// The action's external-effect authority. Defaults to
    /// [`ActionEffectContract::Journaled`].
    ///
    /// Only [`Journaled`](ActionEffectContract::Journaled) and
    /// [`ReadOnly`](ActionEffectContract::ReadOnly) can be written here: a
    /// remote contract holds a boxed descriptor, which no constant can
    /// build. A remote action's contract is its
    /// [`RemoteEffectAction::descriptor`](crate::RemoteEffectAction::descriptor),
    /// and such an action keeps this default.
    const EFFECT_CONTRACT: ActionEffectContract =
        ActionEffectContract::Journaled(JournalProtocol::V1);

    /// Per-action concurrency hint. Defaults to no limit.
    const MAX_CONCURRENT: Option<NonZeroU32> = None;

    /// The action's input ports. Defaults to one main input.
    fn inputs() -> Vec<InputPort> {
        port::default_input_ports()
    }

    /// The action's output ports. Defaults to one main output.
    fn outputs() -> Vec<OutputPort> {
        port::default_output_ports()
    }

    /// Slot-binding declarations (`#[resource]` / `#[credential]` fields).
    fn dependencies() -> &'static Dependencies;
}
