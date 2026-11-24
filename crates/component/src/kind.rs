//! Component kinds.
//!
//! A kind is a type, not a value: [`Component::Kind`]
//! names it, so code generic over components can choose behavior per kind at
//! compile time. The set is closed; [`ComponentKind`] is sealed.

use domain_key::KeyDomain;
use nebula_core::{ActionDomain, CredentialDomain, ResourceDomain};

mod sealed {
    pub trait Sealed {}
}

/// One of the three kinds of integration component.
///
/// Implemented only by [`Action`], [`Resource`] and [`Credential`].
pub trait ComponentKind: sealed::Sealed + Send + Sync + 'static {
    /// The key domain of this kind, which makes an action key and a resource
    /// key different types.
    type Domain: KeyDomain;

    /// Lowercase kind name for diagnostics and traces: `"action"`,
    /// `"resource"` or `"credential"`.
    const NAME: &'static str;
}

/// A step a workflow node runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Action;

/// A long-lived client or connection that actions and other resources use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Resource;

/// Stored authentication material and the way to obtain it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Credential;

impl sealed::Sealed for Action {}
impl sealed::Sealed for Resource {}
impl sealed::Sealed for Credential {}

impl ComponentKind for Action {
    type Domain = ActionDomain;

    const NAME: &'static str = "action";
}

impl ComponentKind for Resource {
    type Domain = ResourceDomain;

    const NAME: &'static str = "resource";
}

impl ComponentKind for Credential {
    type Domain = CredentialDomain;

    const NAME: &'static str = "credential";
}
