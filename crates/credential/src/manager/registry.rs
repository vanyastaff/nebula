//! Protocol registry for dynamic credential creation.
//!
//! Maps `type_id` strings to protocol handlers. Uses enum dispatch to avoid
//! type-erasure complexity with `CredentialType`'s associated types.

use std::collections::HashMap;

use nebula_parameter::values::ParameterValues;

use crate::core::{
    CredentialContext, CredentialError, CredentialState,
    result::{InitializeResult, InteractionRequest, PartialState, UserInput},
};

/// Internal result from protocol init; Complete carries state_json for storage.
/// create() converts to public CreateResult (with credential_id) after storing.
#[derive(Clone, Debug)]
pub(crate) enum InitResult {
    Complete {
        type_id: String,
        state_json: Vec<u8>,
    },
    Pending {
        type_id: String,
        partial_state: PartialState,
        next_step: InteractionRequest,
    },
    RequiresInteraction {
        type_id: String,
        partial_state: PartialState,
        interaction: InteractionRequest,
    },
}

use nebula_parameter::schema::Schema;

use crate::protocols::{
    ApiKeyProtocol, ApiKeyState, BasicAuthProtocol, BasicAuthState, OAuth2Config, OAuth2Protocol,
    OAuth2State,
};
use crate::traits::{FlowProtocol, StaticProtocol};

/// Registered protocol handler (enum dispatch).
///
/// Avoids `Box<dyn CredentialType>` because `CredentialType` has associated
/// types (`State`, `Input`) that prevent object-safe trait objects.
#[derive(Clone, Copy, Debug)]
pub enum RegisteredProtocol {
    /// API key (server + token)
    ApiKey,
    /// Basic auth (username + password)
    BasicAuth,
    /// OAuth2 (authorization code, client credentials, device flow)
    OAuth2,
}

impl RegisteredProtocol {
    /// Type identifier for this protocol (used in registry keys).
    #[must_use]
    pub fn type_id(&self) -> &'static str {
        match self {
            Self::ApiKey => ApiKeyState::KIND,
            Self::BasicAuth => BasicAuthState::KIND,
            Self::OAuth2 => OAuth2State::KIND,
        }
    }

    /// Build state for static protocols (sync, no IO).
    fn build_static<S: CredentialState>(
        values: &ParameterValues,
        build: impl FnOnce(&ParameterValues) -> Result<S, CredentialError>,
    ) -> Result<InitResult, CredentialError> {
        let state = build(values)?;
        Ok(InitResult::Complete {
            type_id: S::KIND.to_string(),
            state_json: serde_json::to_vec(&state).map_err(|e| CredentialError::Validation {
                source: crate::core::ValidationError::InvalidFormat(format!(
                    "state serialization failed: {}",
                    e
                )),
            })?,
        })
    }

    /// Execute protocol initialization and return type-erased result.
    pub(crate) async fn initialize(
        &self,
        values: &ParameterValues,
        ctx: &mut CredentialContext,
    ) -> Result<InitResult, CredentialError> {
        match self {
            Self::ApiKey => Self::build_static(values, ApiKeyProtocol::build_state),
            Self::BasicAuth => Self::build_static(values, BasicAuthProtocol::build_state),
            Self::OAuth2 => {
                let config = oauth2_config_from_values(values)?;
                let result = OAuth2Protocol::initialize(&config, values, ctx).await?;
                Ok(match result {
                    InitializeResult::Complete(state) => InitResult::Complete {
                        type_id: OAuth2State::KIND.to_string(),
                        state_json: serde_json::to_vec(&state).map_err(|e| {
                            CredentialError::Validation {
                                source: crate::core::ValidationError::InvalidFormat(format!(
                                    "state serialization failed: {}",
                                    e
                                )),
                            }
                        })?,
                    },
                    InitializeResult::Pending {
                        partial_state,
                        next_step,
                    } => InitResult::Pending {
                        type_id: OAuth2State::KIND.to_string(),
                        partial_state,
                        next_step,
                    },
                    InitializeResult::RequiresInteraction(req) => InitResult::RequiresInteraction {
                        type_id: OAuth2State::KIND.to_string(),
                        partial_state: PartialState {
                            data: serde_json::json!({"type_id": "oauth2"}),
                            step: "oauth2".to_string(),
                            created_at: crate::core::unix_now(),
                            ttl_seconds: None,
                            metadata: HashMap::new(),
                        },
                        interaction: req,
                    },
                })
            }
        }
    }
}

/// Build OAuth2Config from ParameterValues.
///
/// Expects optional keys: `auth_url`, `token_url`, `grant_type`, `scopes`.
/// Defaults: empty URLs, AuthorizationCode grant.
fn oauth2_config_from_values(values: &ParameterValues) -> Result<OAuth2Config, CredentialError> {
    use crate::protocols::GrantType;

    let auth_url = values
        .get_string("auth_url")
        .map(String::from)
        .unwrap_or_default();
    let token_url = values
        .get_string("token_url")
        .map(String::from)
        .unwrap_or_default();
    let grant_type = values
        .get_string("grant_type")
        .map(|s| match s.to_lowercase().as_str() {
            "client_credentials" => GrantType::ClientCredentials,
            "device_code" => GrantType::DeviceCode,
            _ => GrantType::AuthorizationCode,
        })
        .unwrap_or(GrantType::AuthorizationCode);
    let scopes: Vec<String> = values
        .get("scopes")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    Ok(OAuth2Config {
        auth_url,
        token_url,
        scopes,
        grant_type,
        auth_style: crate::protocols::AuthStyle::Header,
        pkce: false,
    })
}

/// Protocol registry: type_id -> handler.
#[derive(Clone, Default)]
pub struct ProtocolRegistry {
    protocols: HashMap<String, RegisteredProtocol>,
}

impl ProtocolRegistry {
    /// Create registry with built-in protocols (api_key, basic_auth, oauth2).
    #[must_use]
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        registry.register(RegisteredProtocol::ApiKey);
        registry.register(RegisteredProtocol::BasicAuth);
        registry.register(RegisteredProtocol::OAuth2);
        registry
    }

    /// Register a protocol by its type_id.
    pub fn register(&mut self, protocol: RegisteredProtocol) {
        self.protocols
            .insert(protocol.type_id().to_string(), protocol);
    }

    /// Look up protocol by type_id.
    #[must_use]
    pub fn get(&self, type_id: &str) -> Option<RegisteredProtocol> {
        self.protocols.get(type_id).copied()
    }

    /// List all registered type IDs.
    #[must_use]
    pub fn type_ids(&self) -> Vec<String> {
        self.protocols.keys().cloned().collect()
    }

    /// Build schema for a protocol (parameters, display name, capabilities).
    #[must_use]
    pub fn schema_for(&self, type_id: &str) -> Option<ProtocolSchema> {
        let protocol = self.get(type_id)?;
        Some(protocol.to_schema())
    }

    /// Continue interactive flow with user input.
    ///
    /// Dispatches to protocol-specific continue (OAuth2 only for now).
    pub(crate) async fn continue_flow(
        &self,
        type_id: &str,
        partial_state: &PartialState,
        user_input: &UserInput,
        ctx: &mut CredentialContext,
    ) -> Result<InitResult, CredentialError> {
        let protocol = self
            .get(type_id)
            .ok_or_else(|| CredentialError::Validation {
                source: crate::core::ValidationError::InvalidFormat(format!(
                    "unknown type_id for continue: {type_id}"
                )),
            })?;

        match protocol {
            RegisteredProtocol::OAuth2 => {
                let result = crate::protocols::oauth2::flow::continue_oauth2_flow(
                    partial_state,
                    user_input,
                    ctx,
                )
                .await?;
                Ok(match result {
                    InitializeResult::Complete(state) => InitResult::Complete {
                        type_id: OAuth2State::KIND.to_string(),
                        state_json: serde_json::to_vec(&state).map_err(|e| {
                            CredentialError::Validation {
                                source: crate::core::ValidationError::InvalidFormat(format!(
                                    "state serialization failed: {e}"
                                )),
                            }
                        })?,
                    },
                    InitializeResult::Pending {
                        partial_state: ps,
                        next_step,
                    } => InitResult::Pending {
                        type_id: OAuth2State::KIND.to_string(),
                        partial_state: ps,
                        next_step,
                    },
                    InitializeResult::RequiresInteraction(_) => {
                        return Err(CredentialError::Validation {
                            source: crate::core::ValidationError::InvalidFormat(
                                "OAuth2 continue should not return RequiresInteraction".into(),
                            ),
                        });
                    }
                })
            }
            RegisteredProtocol::ApiKey | RegisteredProtocol::BasicAuth => {
                Err(CredentialError::Validation {
                    source: crate::core::ValidationError::InvalidFormat(format!(
                        "{type_id} does not support interactive flow"
                    )),
                })
            }
        }
    }
}

/// Schema info for a registered protocol (used by list_types).
#[derive(Clone, Debug)]
pub struct ProtocolSchema {
    pub type_id: String,
    pub display_name: String,
    pub description: String,
    pub params: Schema,
    pub capabilities: Vec<String>,
}

impl RegisteredProtocol {
    fn to_schema(self) -> ProtocolSchema {
        let (type_id, display_name, description, params, capabilities) = match self {
            Self::ApiKey => (
                ApiKeyState::KIND,
                "API Key",
                "API key or personal access token authentication",
                ApiKeyProtocol::parameters(),
                vec!["static".to_string()],
            ),
            Self::BasicAuth => (
                BasicAuthState::KIND,
                "Basic Auth",
                "HTTP Basic authentication (username + password)",
                BasicAuthProtocol::parameters(),
                vec!["static".to_string()],
            ),
            Self::OAuth2 => (
                OAuth2State::KIND,
                "OAuth2",
                "OAuth2 authorization (authorization code, client credentials, device flow)",
                OAuth2Protocol::parameters(),
                vec!["refresh".to_string(), "interactive".to_string()],
            ),
        };
        ProtocolSchema {
            type_id: type_id.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            params,
            capabilities,
        }
    }
}
