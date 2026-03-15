//! Typed credential handler bridging erased pool state to [`CredentialResource`] instances.

use std::marker::PhantomData;

use nebula_credential::{CredentialResource, CredentialType, RotationStrategy};
use serde::de::{Deserialize, DeserializeOwned};

use crate::pool::CredentialHandler;

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
        let typed = <I::Credential as CredentialType>::State::deserialize(state)
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

        let mut instance = MockClient { token: String::new() };
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
        assert!(matches!(handler.rotation_strategy(), RotationStrategy::DrainAndRecreate));
    }
}
