//! The [`Component`] trait, its typed key and its catalog admission.

use domain_key::Key;
use nebula_metadata::{BaseDefinition, Metadata, MetadataBuildError};
use nebula_schema::{HasSchema, schema_of};
use serde::de::DeserializeOwned;

use crate::kind::ComponentKind;

/// What every action, resource and credential declares about itself.
///
/// A component is a Rust type. It states its identity once ([`KEY`](Self::KEY)),
/// what a user fills in to configure it ([`Config`](Self::Config)), and what the
/// catalog shows for it ([`metadata`](Self::metadata)). The kind trait built on
/// top (`Action`, `Resource`, `Credential`) adds only what that kind does.
///
/// ```
/// use nebula_component::{Component, key_of, kind};
/// use nebula_metadata::{Metadata, metadata_name};
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
///         Metadata::new(metadata_name!("Echo"), "Returns its input")
///     }
/// }
///
/// let key: nebula_core::ActionKey = key_of::<Echo>();
/// assert_eq!(key.as_str(), "echo");
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a component",
    label = "this type does not implement `Component`",
    note = "implement `Component` for it, or derive the kind you need: `#[derive(Action)]`, `#[derive(Resource)]`, `#[derive(Credential)]`"
)]
pub trait Component: Sized + Send + Sync + 'static {
    /// Which kind of component this is: [`kind::Action`](crate::kind::Action),
    /// [`kind::Resource`](crate::kind::Resource) or
    /// [`kind::Credential`](crate::kind::Credential).
    type Kind: ComponentKind;

    /// What a user fills in for one instance of this component: an action's
    /// node parameters, a resource's settings, a credential's setup form.
    ///
    /// Its schema is the component's schema, read with
    /// [`schema_of`]. Use `()` when there is nothing
    /// to configure.
    type Config: HasSchema + DeserializeOwned + Send + Sync + 'static;

    /// The component's key, its identity in the catalog and in every stored
    /// reference to it.
    ///
    /// This is the only place the key is written. Read it as a typed key with
    /// [`key_of`]; a literal that is not a valid key fails the build there.
    const KEY: &'static str;

    /// What the catalog shows for this component: name, description, version,
    /// icon, links, tags, maturity.
    ///
    /// The metadata holds neither a key nor a schema. Admission stamps
    /// [`KEY`](Self::KEY) and derives the schema from [`Config`](Self::Config),
    /// so an admitted definition cannot describe a different component.
    fn metadata() -> Metadata;
}

/// The typed key of component `C`: an `ActionKey`, a `ResourceKey` or a
/// `CredentialKey`, chosen by [`Component::Kind`].
pub type KeyOf<C> = Key<<<C as Component>::Kind as ComponentKind>::Domain>;

/// The typed key of component `C`, built from [`Component::KEY`].
///
/// The literal is checked in a `const` block, so an invalid `KEY` fails the
/// build of whatever names `key_of::<C>()`. Registration always does, which
/// leaves nothing to validate at run time.
#[must_use]
pub fn key_of<C: Component>() -> KeyOf<C> {
    const {
        assert!(
            KeyOf::<C>::is_valid_key_const(C::KEY),
            "`Component::KEY` is not a valid component key"
        );
    }
    // Checked above: a valid key literal is already in canonical form.
    KeyOf::<C>::from_static_unchecked(C::KEY)
}

/// Admits `C`'s catalog definition: its author's [`metadata`](Component::metadata)
/// under [`Component::KEY`] and the schema of [`Component::Config`].
///
/// This is the one admission path for the fields every kind shares. A kind
/// that records more than these (an action's ports, a credential's auth
/// pattern) wraps the result in its own definition type.
///
/// # Errors
///
/// Returns [`MetadataBuildError`] when `C::Config` has no valid schema or the
/// authored metadata breaks a catalog rule.
#[tracing::instrument(
    name = "component.admit",
    skip_all,
    fields(component.kind = <C::Kind as ComponentKind>::NAME, component.key = C::KEY),
    err
)]
pub fn admit<C: Component>() -> Result<BaseDefinition<KeyOf<C>>, MetadataBuildError> {
    let schema = schema_of::<C::Config>()?;
    C::metadata().bind(key_of::<C>(), schema)
}
