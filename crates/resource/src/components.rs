//! Typed dependency declaration for resources.
//!
//! [`ResourceComponents`] is filled in by [`HasResourceComponents::components`] and consumed
//! by the manager to:
//!
//! 1. Build the [`DependencyGraph`] for ordered startup and shutdown.
//! 2. Attach a [`CredentialHandler`] to the pool so credential rotation events are
//!    automatically forwarded to in-pool instances.
//! 3. Inject sub-resource handles into [`Context`] before [`Resource::create`] is called.
//!
//! [`DependencyGraph`]: crate::manager::DependencyGraph
//! [`CredentialHandler`]: crate::pool::CredentialHandler
//! [`Context`]: crate::context::Context
//! [`Resource::create`]: crate::resource::Resource::create

use std::marker::PhantomData;

use nebula_credential::core::reference::ErasedCredentialRef;
use nebula_credential::{CredentialResource, CredentialType, RotationStrategy};
use serde::de::DeserializeOwned;

use crate::pool::CredentialHandler;
use crate::reference::ErasedResourceRef;
use crate::resource::Resource;

/// Dependency declaration for a resource type: an optional credential and a list of
/// sub-resource references.
///
/// Built via the fluent API in [`HasResourceComponents::components`] and consumed by the
/// manager at registration time. After registration this value is not stored — the
/// manager extracts what it needs (credential handler, dependency edges) and discards it.
#[derive(Debug, Clone, Default)]
pub struct ResourceComponents {
    credential: Option<ErasedCredentialRef>,
    resources: Vec<ErasedResourceRef>,
}

impl ResourceComponents {
    /// Create an empty component set with no dependencies.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a credential required by this resource.
    ///
    /// `id` must be a valid UUID string matching the credential registered in
    /// [`nebula_credential`]. The credential's type `C` determines which
    /// [`CredentialHandler`] is attached to the pool and what rotation semantics apply.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not a valid UUID. This is intentional — `components()` is
    /// called once at startup from static configuration; an invalid ID is a
    /// programming error, not a runtime condition.
    ///
    /// [`CredentialHandler`]: crate::pool::CredentialHandler
    pub fn credential<C>(mut self, id: &str) -> Self
    where
        C: nebula_credential::CredentialType,
    {
        let id = nebula_core::CredentialId::parse(id).expect(
            "invalid credential id in HasResourceComponents::components() (expected UUID string)",
        );
        self.credential = Some(ErasedCredentialRef {
            id,
            key: C::credential_key(),
        });
        self
    }

    /// Declare a dependency on another registered resource.
    ///
    /// The manager uses this to add an edge in the [`DependencyGraph`], ensuring the
    /// dependency is started before this resource and shut down after it.
    ///
    /// # Panics
    ///
    /// Panics if `key` is not a valid [`ResourceKey`]. Like [`credential`], this is
    /// called at startup from static configuration, so an invalid key is a
    /// programming error.
    ///
    /// [`DependencyGraph`]: crate::manager::DependencyGraph
    /// [`ResourceKey`]: nebula_core::ResourceKey
    /// [`credential`]: Self::credential
    pub fn resource<R: Resource>(mut self, key: &str) -> Self {
        let key = nebula_core::ResourceKey::new(key).expect(
            "invalid resource key in HasResourceComponents::components() (expected literal key)",
        );
        self.resources.push(ErasedResourceRef { key });
        self
    }

    /// Returns the declared credential reference, if any.
    pub(crate) fn credential_ref(&self) -> Option<&ErasedCredentialRef> {
        self.credential.as_ref()
    }

    /// Returns the declared sub-resource dependencies.
    pub(crate) fn resource_refs(&self) -> &[ErasedResourceRef] {
        &self.resources
    }

    #[cfg(test)]
    pub(crate) fn with_credential_for_test(mut self, erased: ErasedCredentialRef) -> Self {
        self.credential = Some(erased);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_resource_for_test(mut self, erased: ErasedResourceRef) -> Self {
        self.resources.push(erased);
        self
    }
}

/// Implemented on a [`Resource`] factory to declare its credential and sub-resource
/// dependencies.
///
/// The manager calls `components()` once at registration time. Implement this trait
/// when your resource requires a credential (for rotation support) or depends on
/// another resource being available first (for dependency-ordered startup).
///
/// # Example
///
/// ```rust,ignore
/// impl HasResourceComponents for MyDbResource {
///     fn components() -> ResourceComponents {
///         ResourceComponents::new()
///             .credential::<DatabaseCredential>("550e8400-e29b-41d4-a716-446655440000")
///             .resource::<CacheResource>("cache")
///     }
/// }
/// ```
pub trait HasResourceComponents: Resource {
    /// Declares credential and sub-resource dependencies for this resource.
    ///
    /// The manager uses this to build the dependency graph, attach a credential
    /// handler to the pool, and (when wired) inject sub-resource handles into
    /// the context before `Resource::create()`.
    fn components() -> ResourceComponents
    where
        Self: Sized;
}

// ---------------------------------------------------------------------------
// TypedCredentialHandler — bridge from erased pool to CredentialResource
// ---------------------------------------------------------------------------

/// Concrete credential handler for any instance implementing [`CredentialResource`].
///
/// Stored in the pool at registration time. Deserializes JSON state
/// (from `CredentialRotationEvent::new_state`) and calls `authorize()`.
#[derive(Debug)]
pub struct TypedCredentialHandler<I>(PhantomData<fn() -> I>);

impl<I> TypedCredentialHandler<I> {
    /// Create a new typed credential handler.
    #[must_use]
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<I> Default for TypedCredentialHandler<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I> Clone for TypedCredentialHandler<I> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<I> CredentialHandler<I> for TypedCredentialHandler<I>
where
    I: CredentialResource,
    <I::Credential as CredentialType>::State: DeserializeOwned,
{
    fn authorize(&self, instance: &mut I, state: &serde_json::Value) -> crate::error::Result<()> {
        let typed =
            serde_json::from_value::<<I::Credential as CredentialType>::State>(state.clone())
                .map_err(|e| crate::error::Error::configuration(e.to_string()))?;
        instance.authorize(&typed);
        Ok(())
    }

    fn rotation_strategy(&self) -> RotationStrategy {
        I::rotation_strategy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    #[test]
    fn components_with_no_deps() {
        let c = ResourceComponents::new();
        assert!(c.credential_ref().is_none());
        assert!(c.resource_refs().is_empty());
    }

    #[test]
    fn components_with_credential() {
        let id = nebula_core::CredentialId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let key = nebula_core::CredentialKey::new("dummy").unwrap();
        let erased = ErasedCredentialRef { id, key };
        let c = ResourceComponents::new().with_credential_for_test(erased);
        let r = c.credential_ref().unwrap();
        assert_eq!(r.id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(r.key.as_str(), "dummy");
    }

    #[test]
    fn components_with_resource() {
        let key = nebula_core::ResourceKey::new("http-global").unwrap();
        let erased = ErasedResourceRef { key };
        let c = ResourceComponents::new().with_resource_for_test(erased);
        assert_eq!(c.resource_refs().len(), 1);
        assert_eq!(c.resource_refs()[0].key.as_str(), "http-global");
    }

    #[test]
    fn typed_handler_deserializes_and_calls_authorize() {
        use nebula_credential::protocols::HeaderAuthState;

        struct TestCred;

        #[async_trait]
        impl nebula_credential::CredentialType for TestCred {
            type Input = ();
            type State = HeaderAuthState;

            fn description() -> nebula_credential::CredentialDescription
            where
                Self: Sized,
            {
                nebula_credential::CredentialDescription::builder()
                    .key("test_header")
                    .name("Test")
                    .description("Test")
                    .properties(nebula_parameter::schema::Schema::new())
                    .build()
                    .unwrap()
            }

            async fn initialize(
                &self,
                _: &Self::Input,
                _: &mut nebula_credential::CredentialContext,
            ) -> Result<
                nebula_credential::InitializeResult<Self::State>,
                nebula_credential::CredentialError,
            > {
                unreachable!()
            }
        }

        struct MockClient {
            token: String,
        }
        impl CredentialResource for MockClient {
            type Credential = TestCred;

            fn authorize(&mut self, state: &HeaderAuthState) {
                self.token = state.header_value.clone();
            }
        }

        let mut instance = MockClient {
            token: String::new(),
        };
        let handler = TypedCredentialHandler::<MockClient>::new();
        handler
            .authorize(
                &mut instance,
                &serde_json::json!({
                    "header_name": "Authorization",
                    "header_value": "Bearer tok123"
                }),
            )
            .unwrap();
        assert_eq!(instance.token, "Bearer tok123");
    }

    #[test]
    fn typed_handler_returns_correct_rotation_strategy() {
        use nebula_credential::protocols::DatabaseState;

        struct TestDbCred;

        #[async_trait]
        impl nebula_credential::CredentialType for TestDbCred {
            type Input = ();
            type State = DatabaseState;

            fn description() -> nebula_credential::CredentialDescription
            where
                Self: Sized,
            {
                nebula_credential::CredentialDescription::builder()
                    .key("test_db")
                    .name("Test DB")
                    .description("Test")
                    .properties(nebula_parameter::schema::Schema::new())
                    .build()
                    .unwrap()
            }

            async fn initialize(
                &self,
                _: &Self::Input,
                _: &mut nebula_credential::CredentialContext,
            ) -> Result<
                nebula_credential::InitializeResult<Self::State>,
                nebula_credential::CredentialError,
            > {
                unreachable!()
            }
        }

        struct MyDb;
        impl CredentialResource for MyDb {
            type Credential = TestDbCred;

            fn authorize(&mut self, _: &DatabaseState) {}

            fn rotation_strategy() -> RotationStrategy
            where
                Self: Sized,
            {
                RotationStrategy::DrainAndRecreate
            }
        }

        let handler = TypedCredentialHandler::<MyDb>::new();
        assert!(matches!(
            handler.rotation_strategy(),
            RotationStrategy::DrainAndRecreate
        ));
    }
}
