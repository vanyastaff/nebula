//! Bridge between Credential trait and CredentialFactory
//!
//! This module provides automatic conversion from type-safe Credential implementations
//! to the object-safe CredentialFactory trait using type erasure.

use crate::core::{AccessToken, CredentialContext, CredentialError, CredentialMetadata};
use crate::registry::CredentialFactory;
use crate::traits::Credential;
use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};

/// Adapter that wraps a Credential and implements CredentialFactory
///
/// This allows you to implement the type-safe Credential trait and automatically
/// get a CredentialFactory implementation through type erasure.
///
/// # Example
///
/// ```ignore
/// struct MyCredential;
///
/// #[async_trait]
/// impl Credential for MyCredential {
///     type Input = MyInput;
///     type State = MyState;
///     // ... implement methods
/// }
///
/// // Convert to factory
/// let factory = CredentialAdapter::new(MyCredential);
/// registry.register(Arc::new(factory));
/// ```
pub struct CredentialAdapter<C: Credential> {
    credential: C,
}

impl<C: Credential> CredentialAdapter<C> {
    /// Create a new adapter for the given credential
    pub fn new(credential: C) -> Self {
        Self { credential }
    }
}

#[async_trait]
impl<C> CredentialFactory for CredentialAdapter<C>
where
    C: Credential,
    C::Input: Serialize + DeserializeOwned + Send + Sync + 'static,
    C::State: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn type_name(&self) -> &'static str {
        C::TYPE_NAME
    }

    async fn create_and_init(
        &self,
        input_json: serde_json::Value,
        cx: &mut CredentialContext,
    ) -> Result<(Box<dyn erased_serde::Serialize>, Option<AccessToken>), CredentialError> {
        // Deserialize input
        let input: C::Input = serde_json::from_value(input_json)
            .map_err(|e| CredentialError::DeserializationFailed(e.to_string()))?;

        // Call credential's initialize
        let (state, token) = self.credential.initialize(&input, cx).await?;

        // Type erase the state
        Ok((Box::new(state), token))
    }

    async fn refresh(
        &self,
        state_json: serde_json::Value,
        cx: &mut CredentialContext,
    ) -> Result<(Box<dyn erased_serde::Serialize>, AccessToken), CredentialError> {
        // Deserialize state
        let mut state: C::State = serde_json::from_value(state_json)
            .map_err(|e| CredentialError::DeserializationFailed(e.to_string()))?;

        // Call credential's refresh
        let token = self.credential.refresh(&mut state, cx).await?;

        // Type erase the updated state
        Ok((Box::new(state), token))
    }

    fn metadata(&self) -> CredentialMetadata {
        self.credential.metadata()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SecureString;
    use serde::{Deserialize, Serialize};
    use std::time::SystemTime;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestInput {
        value: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestState {
        value: SecureString,
        created_at: SystemTime,
    }

    impl crate::core::CredentialState for TestState {
        const KIND: &'static str = "test_adapter";
        const VERSION: u16 = 1;
    }

    struct TestCredential;

    #[async_trait]
    impl Credential for TestCredential {
        type Input = TestInput;
        type State = TestState;

        fn metadata(&self) -> CredentialMetadata {
            CredentialMetadata {
                id: "test_adapter",
                name: "Test Adapter Credential",
                description: "Testing the adapter pattern",
                supports_refresh: true,
                requires_interaction: false,
            }
        }

        async fn initialize(
            &self,
            input: &Self::Input,
            _ctx: &mut CredentialContext,
        ) -> Result<(Self::State, Option<AccessToken>), CredentialError> {
            let state = TestState {
                value: SecureString::new(input.value.clone()),
                created_at: SystemTime::now(),
            };

            let token = AccessToken::bearer(format!("token_{}", input.value))
                .with_expiration(SystemTime::now() + std::time::Duration::from_secs(3600));

            Ok((state, Some(token)))
        }

        async fn refresh(
            &self,
            state: &mut Self::State,
            _ctx: &mut CredentialContext,
        ) -> Result<AccessToken, CredentialError> {
            let token = AccessToken::bearer(format!("refreshed_{}", state.value.expose()))
                .with_expiration(SystemTime::now() + std::time::Duration::from_secs(3600));

            Ok(token)
        }
    }

    #[tokio::test]
    async fn test_adapter_create_and_init() {
        let adapter = CredentialAdapter::new(TestCredential);
        let mut cx = CredentialContext::new();

        let input = serde_json::json!({
            "value": "test123"
        });

        let result = adapter.create_and_init(input, &mut cx).await;
        assert!(result.is_ok());

        let (state_box, token) = result.unwrap();
        assert!(token.is_some());

        // Verify state can be serialized back
        let state_json = serde_json::to_value(&state_box).unwrap();
        assert!(state_json.is_object());
    }

    #[tokio::test]
    async fn test_adapter_refresh() {
        let adapter = CredentialAdapter::new(TestCredential);
        let mut cx = CredentialContext::new();

        // Create initial state
        let state = TestState {
            value: SecureString::new("test456"),
            created_at: SystemTime::now(),
        };

        let state_json = serde_json::to_value(&state).unwrap();

        let result = adapter.refresh(state_json, &mut cx).await;
        assert!(result.is_ok());

        let (_new_state, token) = result.unwrap();
        assert!(!token.is_expired());
    }

    #[test]
    fn test_adapter_metadata() {
        let adapter = CredentialAdapter::new(TestCredential);
        let metadata = adapter.metadata();

        assert_eq!(metadata.id, "test_adapter");
        assert_eq!(metadata.name, "Test Adapter Credential");
        assert!(metadata.supports_refresh);
    }

    #[test]
    fn test_adapter_type_name() {
        let adapter = CredentialAdapter::new(TestCredential);
        assert_eq!(adapter.type_name(), "test_adapter");
    }
}
