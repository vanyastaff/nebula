//! Type-safe reference to an action type.
//!
//! Used by plugins to declare which action types they provide, in the same
//! style as [`CredentialRef`](nebula_credential::CredentialRef) and
//! [`ResourceRef`](nebula_resource::ResourceRef). Plugins use these refs in
//! their component declarations alongside credential and resource refs.

use std::any::TypeId;
use std::fmt;

/// Type-safe reference to an action type.
///
/// Wraps a `TypeId` to identify an action type. Used by plugins to declare
/// provided actions in plugin component declarations, in the same style as credential and resource refs.
///
/// # Example
///
/// ```rust
/// use nebula_action::ActionRef;
///
/// struct HttpRequestAction;
///
/// let action_ref = ActionRef::of::<HttpRequestAction>();
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionRef(TypeId);

impl ActionRef {
    /// Create an action reference from a type.
    pub const fn of<T: 'static>() -> Self {
        Self(TypeId::of::<T>())
    }

    /// Returns the underlying type ID.
    pub const fn type_id(self) -> TypeId {
        self.0
    }
}

impl fmt::Debug for ActionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ActionRef").field(&self.0).finish()
    }
}

impl fmt::Display for ActionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ActionRef({:?})", self.0)
    }
}

impl<T: 'static> From<std::marker::PhantomData<T>> for ActionRef {
    fn from(_: std::marker::PhantomData<T>) -> Self {
        Self::of::<T>()
    }
}
