//! Base [`Action`] trait — identity, metadata, type-level Input/Output, slot deps.
//!
//! See module docs in `lib.rs` for trait family overview.

use nebula_core::{ActionKey, Dependencies};
use nebula_schema::HasSchema;
use serde::{Serialize, de::DeserializeOwned};

use crate::metadata::ActionMetadata;

/// Base trait for all action types.
///
/// Identity ([`KEY`](Self::KEY)), catalog metadata (`metadata`), type-level
/// input/output (`Self::Input`, `Self::Output`), and slot-binding declarations
/// (`dependencies`) — all static per concrete type. The schema is reached via the
/// `Input`/`Output: HasSchema` bound (`nebula_schema::schema_of::<A::Input>()`);
/// there is no per-trait schema method. Sub-traits
/// ([`StatelessAction`](crate::StatelessAction) etc.) define the execution
/// surface and consume `Self::Input` / `Self::Output`.
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
/// use nebula_action::{Action, ActionMetadata};
/// use nebula_core::Dependencies;
///
/// struct Echo;
///
/// impl Action for Echo {
///     const KEY: &'static str = "echo";
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///
///     fn metadata() -> ActionMetadata {
///         ActionMetadata::new(nebula_action::metadata_name!("Echo"), "Echoes input")
///     }
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
/// let _input_schema = nebula_schema::schema_of::<<Echo as Action>::Input>().expect("valid schema");
/// ```
///
/// The input/output schema is obtained from the associated type, e.g.
/// `nebula_schema::schema_of::<<Echo as Action>::Input>()`.
///
/// `#[derive(Action)]` emits this boilerplate from `#[action(key = "...", ...)]`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as an Action",
    label = "this type does not implement the Action trait",
    note = "derive it: #[derive(Action)]"
)]
pub trait Action: Sized + Send + Sync + 'static {
    /// The action's key, its identity in the catalog and in workflow nodes.
    ///
    /// Declared once, on the type. The metadata returned by
    /// [`metadata`](Self::metadata) cannot hold a key: a factory stamps this one
    /// at admission, so the two can never disagree. Read it as a typed
    /// [`ActionKey`] with [`key_of`]; an invalid literal fails the build there.
    const KEY: &'static str;

    /// User-facing form data; deserialized from `node.input_json` per execution.
    type Input: HasSchema + DeserializeOwned + Send + Sync;

    /// What this action produces; serialized to JSON for downstream nodes.
    type Output: HasSchema + Serialize + Send + Sync;

    /// Author-owned catalog metadata for this action type, without a key.
    ///
    /// Returned by value, symmetric with
    /// [`Credential::metadata`](nebula_credential) and
    /// `Resource::metadata`. Authors build it fresh — no `static`/`OnceLock`
    /// boilerplate. The engine caches the result once per registered factory
    /// (see `ActionFactory`), so the cold per-call cost is paid only at
    /// registration, not per dispatch. The receiving factory derives schemas
    /// from the associated types, stamps the structural kind, and performs the
    /// only admission pass.
    fn metadata() -> ActionMetadata;

    /// Slot-binding declarations (`#[resource]` / `#[credential]` fields, Phase 3 / S3+).
    fn dependencies() -> &'static Dependencies;
}

/// The typed key of action type `A`, derived from [`Action::KEY`].
///
/// The literal is checked in a `const` block, so an invalid `KEY` fails the
/// build wherever this is instantiated (every factory does), not at run time.
#[must_use]
pub fn key_of<A: Action>() -> ActionKey {
    const {
        assert!(
            ActionKey::is_valid_key_const(A::KEY),
            "`Action::KEY` is not a valid action key"
        );
    }
    ActionKey::from_static_unchecked(A::KEY)
}
