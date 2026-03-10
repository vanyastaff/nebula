//! ResourceComponents and HasResourceComponents — typed dependency declaration for resources.
//!
//! Используется менеджером ресурсов для:
//! 1. Построения графа зависимостей (`DependencyGraph`) по ресурсам.
//! 2. Связывания пулов с нужными credential’ами.
//! 3. Инъекции саб‑ресурсов в `Context` перед вызовом `Resource::create()`.

use std::marker::PhantomData;

use nebula_credential::core::reference::ErasedCredentialRef;
use nebula_credential::{CredentialResource, CredentialType, RotationStrategy};
use serde::de::DeserializeOwned;

use crate::pool::CredentialHandler;
use crate::reference::ErasedResourceRef;
use crate::resource::Resource;

/// Декларация зависимостей ресурса: опциональный credential и список саб‑ресурсов.
///
/// Заполняется в `HasResourceComponents::components()` и читается менеджером.
#[derive(Debug, Clone, Default)]
pub struct ResourceComponents {
    credential: Option<ErasedCredentialRef>,
    resources: Vec<ErasedResourceRef>,
}

impl ResourceComponents {
    /// Создать пустой набор компонентов без зависимостей.
    pub fn new() -> Self {
        Self::default()
    }

    /// Объявить credential, требуемый ресурсом.
    ///
    /// # Паника
    ///
    /// Падает, если `id` — невалидный UUID. В `components()` обычно используются
    /// строковые литералы с UUID, поэтому это ошибка конфигурации.
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

    /// Объявить зависимость от другого ресурса по ключу.
    ///
    /// # Паника
    ///
    /// Падает, если `key` — невалидный `ResourceKey`.
    pub fn resource<R: Resource>(mut self, key: &str) -> Self {
        let key = nebula_core::ResourceKey::new(key).expect(
            "invalid resource key in HasResourceComponents::components() (expected literal key)",
        );
        self.resources.push(ErasedResourceRef { key });
        self
    }

    /// Внутренний доступ к credential (для менеджера).
    pub(crate) fn credential_ref(&self) -> Option<&ErasedCredentialRef> {
        self.credential.as_ref()
    }

    /// Внутренний доступ к списку ресурсных зависимостей (для менеджера).
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

/// Реализуется на `Resource`‑фабрике для декларации credential’ов и саб‑ресурсов.
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
