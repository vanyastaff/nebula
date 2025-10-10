//! Adapter from `CredentialFlow` to Credential trait

use async_trait::async_trait;

use super::{
    CredentialContext, CredentialError, CredentialKey, CredentialMetadata,
    result::{CredentialFlow, InitializeResult, PartialState, UserInput},
};
use crate::traits::Credential;

/// Generic wrapper that adapts any `CredentialFlow` to the Credential trait
///
/// This adapter makes it easy to use flow-based credentials without
/// manually implementing the Credential trait.
///
/// # Example
/// ```ignore
/// let flow = OAuth2ClientCredentialsFlow;
/// let credential = FlowCredential::new(flow);
/// registry.register(credential);
/// ```
pub struct FlowCredential<F: CredentialFlow> {
    flow: F,
    metadata_override: Option<CredentialMetadata>,
}

impl<F: CredentialFlow> FlowCredential<F> {
    /// Create a new flow credential adapter
    pub fn new(flow: F) -> Self {
        Self {
            flow,
            metadata_override: None,
        }
    }

    /// Create from a flow (alias for new)
    pub fn from_flow(flow: F) -> Self {
        Self::new(flow)
    }

    /// Override the default metadata
    pub fn with_metadata(mut self, metadata: CredentialMetadata) -> Self {
        self.metadata_override = Some(metadata);
        self
    }

    /// Get access to the underlying flow
    pub fn flow(&self) -> &F {
        &self.flow
    }
}

#[async_trait]
impl<F: CredentialFlow> Credential for FlowCredential<F> {
    type Input = F::Input;
    type State = F::State;

    fn metadata(&self) -> CredentialMetadata {
        if let Some(m) = &self.metadata_override {
            return m.clone();
        }

        // Generate metadata from flow
        CredentialMetadata {
            key: CredentialKey::new(self.flow.flow_name())
                .unwrap_or_else(|_| panic!("Invalid flow name: {}", self.flow.flow_name())),
            name: format!("{} Flow", self.flow.flow_name()),
            description: format!("Authentication via {}", self.flow.flow_name()),
            supports_refresh: true, // Most flows support refresh
            requires_interaction: self.flow.requires_interaction(),
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        self.flow.execute(input, ctx).await
    }

    async fn refresh(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        self.flow.refresh(state, ctx).await
    }

    async fn revoke(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        self.flow.revoke(state, ctx).await
    }
}

/// Auto-implement `InteractiveCredential` for flows that need it
///
/// Note: This is a blanket implementation. For flows that support interaction,
/// you should manually implement `InteractiveCredential` to handle `continue_initialization`.
impl<F: CredentialFlow> FlowCredential<F> {
    /// Helper to continue an interactive flow
    ///
    /// Override this in your flow implementation to handle user input
    pub async fn continue_flow(
        &self,
        _partial_state: PartialState,
        _user_input: UserInput,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<F::State>, CredentialError> {
        Err(CredentialError::internal(
            "Interactive flow continuation not implemented",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CredentialContext, state::CredentialState};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Serialize, Deserialize)]
    struct TestState {
        value: String,
    }

    impl CredentialState for TestState {
        const VERSION: u16 = 1;
        const KIND: &'static str = "test";
    }

    #[derive(Serialize, Deserialize)]
    struct TestInput {
        key: String,
    }

    struct TestFlow;

    #[async_trait]
    impl CredentialFlow for TestFlow {
        type Input = TestInput;
        type State = TestState;

        fn flow_name(&self) -> &'static str {
            "test_flow"
        }

        async fn execute(
            &self,
            input: &Self::Input,
            _ctx: &mut CredentialContext,
        ) -> Result<InitializeResult<Self::State>, CredentialError> {
            Ok(InitializeResult::Complete(TestState {
                value: input.key.clone(),
            }))
        }
    }

    #[tokio::test]
    async fn test_flow_credential_adapter() {
        let flow = TestFlow;
        let credential = FlowCredential::new(flow);

        let metadata = credential.metadata();
        assert_eq!(metadata.key.as_str(), "test_flow");
        assert_eq!(metadata.name, "test_flow Flow");
    }

    #[tokio::test]
    async fn test_flow_credential_with_override() {
        let flow = TestFlow;
        let credential = FlowCredential::new(flow).with_metadata(CredentialMetadata {
            key: CredentialKey::new("custom").unwrap(),
            name: "Custom Name".to_string(),
            description: "Custom Description".to_string(),
            supports_refresh: true,
            requires_interaction: false,
        });

        let metadata = credential.metadata();
        assert_eq!(metadata.key.as_str(), "custom");
        assert_eq!(metadata.name, "Custom Name");
    }
}
