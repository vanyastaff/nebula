//! Integration tests for CredentialRegistry

use nebula_credential::testing::TestCredentialFactory;
use nebula_credential::CredentialRegistry;
use std::sync::Arc;

#[test]
fn test_registry_initialization() {
    let registry = CredentialRegistry::new();
    assert!(registry.list_types().is_empty());
}

#[test]
fn test_factory_registration() {
    let registry = CredentialRegistry::new();
    let factory = Arc::new(TestCredentialFactory::new());

    registry.register(factory);

    assert!(registry.has_type("test_credential"));
    assert_eq!(registry.list_types(), vec!["test_credential"]);
}

#[test]
fn test_factory_lookup() {
    let registry = CredentialRegistry::new();
    let factory = Arc::new(TestCredentialFactory::new());

    registry.register(factory);

    let retrieved = registry.get("test_credential");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().type_name(), "test_credential");
}

#[test]
fn test_factory_not_found() {
    let registry = CredentialRegistry::new();

    let result = registry.get("nonexistent_type");
    assert!(result.is_none());
    assert!(!registry.has_type("nonexistent_type"));
}

#[test]
fn test_multiple_credential_types() {
    use nebula_credential::core::{AccessToken, CredentialContext, Result};
    use nebula_credential::registry::CredentialFactory;
    use nebula_credential::testing::TestCredentialInput;
    use async_trait::async_trait;

    // Create a second factory type
    struct AnotherCredentialFactory;

    #[async_trait]
    impl CredentialFactory for AnotherCredentialFactory {
        fn type_name(&self) -> &'static str {
            "another_credential"
        }

        async fn create_and_init(
            &self,
            input_json: serde_json::Value,
            _cx: &mut CredentialContext,
        ) -> Result<(Box<dyn erased_serde::Serialize>, Option<AccessToken>)> {
            let _input: TestCredentialInput = serde_json::from_value(input_json)
                .map_err(|e| nebula_credential::core::CredentialError::DeserializationFailed(e.to_string()))?;

            Ok((Box::new(()), None))
        }

        async fn refresh(
            &self,
            _state_json: serde_json::Value,
            _cx: &mut CredentialContext,
        ) -> Result<(Box<dyn erased_serde::Serialize>, AccessToken)> {
            Err(nebula_credential::core::CredentialError::RefreshNotSupported {
                credential_type: "another_credential".to_string(),
            })
        }
    }

    let registry = CredentialRegistry::new();
    registry.register(Arc::new(TestCredentialFactory::new()));
    registry.register(Arc::new(AnotherCredentialFactory));

    assert_eq!(registry.list_types().len(), 2);
    assert!(registry.has_type("test_credential"));
    assert!(registry.has_type("another_credential"));
}

#[test]
fn test_registry_replace_factory() {
    use nebula_credential::testing::TestCredential;

    let registry = CredentialRegistry::new();

    // Register first factory
    let factory1 = Arc::new(TestCredentialFactory::new());
    registry.register(factory1);

    // Register another factory with same type (should replace)
    let custom_credential = TestCredential {
        fail_on_refresh: true,
        refresh_delay: None,
    };
    let factory2 = Arc::new(TestCredentialFactory::with_credential(custom_credential));
    registry.register(factory2);

    // Should only have one type
    assert_eq!(registry.list_types().len(), 1);
    assert!(registry.has_type("test_credential"));
}

#[tokio::test]
async fn test_type_safe_credential_creation() {
    use nebula_credential::core::CredentialContext;
    use serde_json::json;

    let registry = CredentialRegistry::new();
    registry.register(Arc::new(TestCredentialFactory::new()));

    let factory = registry.get("test_credential").unwrap();
    let mut ctx = CredentialContext::new();

    let input = json!({
        "value": "test-value",
        "should_fail": false
    });

    let result = factory.create_and_init(input, &mut ctx).await;
    assert!(result.is_ok());

    let (state, token) = result.unwrap();
    assert!(token.is_some());

    // Verify state is serializable
    let state_json = serde_json::to_value(&state);
    assert!(state_json.is_ok());
}

#[test]
fn test_factory_type_name_verification() {
    let registry = CredentialRegistry::new();
    registry.register(Arc::new(TestCredentialFactory::new()));

    let factory = registry.get("test_credential").unwrap();

    assert_eq!(factory.type_name(), "test_credential");
}

#[tokio::test]
async fn test_factory_refresh_operation() {
    use nebula_credential::core::CredentialContext;
    use nebula_credential::testing::TestCredentialState;
    use std::time::SystemTime;

    let registry = CredentialRegistry::new();
    registry.register(Arc::new(TestCredentialFactory::new()));

    let factory = registry.get("test_credential").unwrap();
    let mut ctx = CredentialContext::new();

    // Create initial state
    let state = TestCredentialState {
        value: nebula_credential::core::SecureString::new("test-value"),
        refresh_count: 0,
        created_at: SystemTime::now(),
    };

    let state_json = serde_json::to_value(&state).unwrap();

    // Refresh the credential
    let result = factory.refresh(state_json, &mut ctx).await;
    assert!(result.is_ok());

    let (new_state, token) = result.unwrap();
    assert!(!token.is_expired());

    // Verify state was updated
    let new_state_json = serde_json::to_value(&new_state).unwrap();
    assert!(new_state_json.is_object());
}
