//! How an action's `#[slot]` fields get their values for one run.
//!
//! The declaration half lives on the type: `#[derive(Action)]` writes
//! [`Component::dependencies`](crate::Component::dependencies) from the
//! fields. This module is the other half: [`FromSlot`] fills one field from
//! the node's binding, and the derive calls it for every `#[slot]` field
//! before the action runs.
//!
//! A slot value type implements [`ResolveSlot`] once; `T` and `Option<T>`
//! then both work as field types. An unbound resource slot takes the
//! instance registered under the resource's own key; an unbound credential
//! slot resolves by the slot's key. An optional slot that is unbound and
//! finds nothing stays `None`; a slot bound to an instance that does not
//! resolve is an error either way.

use std::future::Future;

use nebula_component::SlotValue;
use nebula_credential::Credential;
use nebula_resource::{CredentialSlot, call::ResourceHandle, resource::Provider};
use nebula_workflow::NodeDefinition;

use crate::{ActionContext, ActionContextExt, ActionError};

/// A field type an action's `#[slot]` field can hold.
///
/// Implemented for every [`ResolveSlot`] value and for `Option` of one.
#[diagnostic::on_unimplemented(
    message = "an action cannot hold `{Self}` in a slot",
    label = "not an action slot value",
    note = "an action slot holds `ResourceHandle<R>` or `CredentialSlot<C>`, optionally in `Option<...>`"
)]
pub trait FromSlot: SlotValue + Sized {
    /// The value of slot `key` for the run of `node`.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Fatal`] when a required slot finds no
    /// instance, or when a bound instance does not resolve.
    fn from_slot<'a>(
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
        key: &'static str,
    ) -> impl Future<Output = Result<Self, ActionError>> + Send + 'a;
}

/// A single slot value an action can resolve.
///
/// [`FromSlot`] builds both the required and the optional field from it.
#[diagnostic::on_unimplemented(
    message = "an action cannot resolve `{Self}` for a slot",
    label = "not an action slot value",
    note = "an action slot holds `ResourceHandle<R>` or `CredentialSlot<C>`, optionally in `Option<...>`"
)]
pub trait ResolveSlot: SlotValue + Sized {
    /// The value of slot `key`, or `None` when the slot is unbound and no
    /// instance is registered under the target's key.
    ///
    /// # Errors
    ///
    /// Returns an error when the slot is bound and its instance does not
    /// resolve, or when the instance found is of another type.
    fn try_resolve<'a>(
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
        key: &'static str,
    ) -> impl Future<Output = Result<Option<Self>, ActionError>> + Send + 'a;
}

impl<V: ResolveSlot> FromSlot for V {
    async fn from_slot<'a>(
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
        key: &'static str,
    ) -> Result<Self, ActionError> {
        V::try_resolve(node, ctx, key).await?.ok_or_else(|| {
            let target = nebula_component::SlotTarget::of::<V::Target>();
            ActionError::fatal(format!(
                "slot `{key}` is not bound and no {kind} `{target_key}` is registered",
                kind = target.kind_name(),
                target_key = target.key_str(),
            ))
        })
    }
}

impl<V: ResolveSlot> FromSlot for Option<V> {
    fn from_slot<'a>(
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
        key: &'static str,
    ) -> impl Future<Output = Result<Self, ActionError>> + Send + 'a {
        V::try_resolve(node, ctx, key)
    }
}

impl<R: Provider> ResolveSlot for ResourceHandle<R> {
    fn try_resolve<'a>(
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
        key: &'static str,
    ) -> impl Future<Output = Result<Option<Self>, ActionError>> + Send + 'a {
        let resolved = match node.resource_binding(key) {
            Some(id) => {
                <dyn ActionContext as ActionContextExt>::resource_handle_by_id::<R>(ctx, id)
                    .map(Some)
            },
            None => <dyn ActionContext as ActionContextExt>::try_resource_handle_by_id::<R>(
                ctx,
                nebula_resource::key_of::<R>().as_str(),
            ),
        };
        std::future::ready(resolved)
    }
}

impl<C> ResolveSlot for CredentialSlot<C>
where
    C: Credential,
    C::Scheme: zeroize::Zeroize,
{
    async fn try_resolve<'a>(
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
        key: &'static str,
    ) -> Result<Option<Self>, ActionError> {
        // Unbound, a credential slot resolves by its own key, which is the
        // default selector the plan compiler records for it.
        let bound = node.credential_binding(key);
        let id = bound.unwrap_or(key);
        match <dyn ActionContext as ActionContextExt>::resolve_credential_by_id::<C>(ctx, id).await
        {
            Ok(guard) => Ok(Some(CredentialSlot::bound(guard))),
            Err(error) if bound.is_some() => Err(ActionError::fatal(format!(
                "credential slot `{key}` is bound to `{id}`, which does not resolve: {error}"
            ))),
            Err(_) => Ok(None),
        }
    }
}
