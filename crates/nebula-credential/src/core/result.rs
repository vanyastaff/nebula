//! Credential flow primitives for interactive and non-interactive authentication

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{context::CredentialContext, error::CredentialError, state::CredentialState};

/// Partial state for multi-step interactive flows
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartialState {
    /// Step-specific data (can include secure references)
    pub data: serde_json::Value,
    /// Current step marker
    pub step: String,
    /// Unix timestamp when created
    #[serde(default = "crate::core::unix_now")]
    pub created_at: u64,
    /// Time-to-live in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
    /// Optional metadata
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

/// Result of credential initialization
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", bound = "S: CredentialState")]
pub enum InitializeResult<S: CredentialState> {
    /// Credential ready without interaction
    Complete(S),

    /// Save partial and request next step (multi-step scenario)
    Pending {
        partial_state: PartialState,
        next_step: InteractionRequest,
    },

    /// Requires user action (single-step scenario)
    RequiresInteraction(InteractionRequest),
}

/// Universal interaction request types (protocol-agnostic)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InteractionRequest {
    /// Redirect user to URL (OAuth2, SAML, OpenID Connect, etc.)
    Redirect {
        url: String,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        validation_params: HashMap<String, String>,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        metadata: HashMap<String, String>,
    },

    /// Request code input (2FA, email OTP, SMS, TOTP)
    CodeInput {
        #[serde(skip_serializing_if = "Option::is_none")]
        delivery_method: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<CodeFormat>,
        #[serde(skip_serializing_if = "Option::is_none")]
        expires_in: Option<u64>,
    },

    /// Display information to user (QR code, user code for device flow)
    DisplayInfo {
        display_data: DisplayData,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        expires_in: Option<u64>,
    },

    /// Wait for user confirmation (device flow, push notification)
    AwaitConfirmation {
        confirmation_type: String,
        message: String,
        timeout: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        poll_interval: Option<u64>,
    },

    /// Cryptographic challenge (WebAuthn, client certificate, FIDO2)
    Challenge {
        challenge_data: String,
        challenge_type: String,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        params: HashMap<String, serde_json::Value>,
    },

    /// CAPTCHA verification
    Captcha {
        captcha_data: String,
        captcha_type: CaptchaType,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        params: HashMap<String, String>,
    },

    /// Custom interaction (bridge to UI)
    Custom {
        interaction_type: String,
        data: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
    },
}

/// Display data types
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DisplayData {
    /// QR code for scanning
    QrCode {
        data: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        image_url: Option<String>,
    },
    /// User code for manual entry (OAuth2 device flow, etc.)
    UserCode {
        code: String,
        verification_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        complete_url: Option<String>,
    },
    /// Plain text
    Text { text: String },
}

/// Code format constraints
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeFormat {
    Numeric,
    Alphanumeric,
    Any,
}

/// CAPTCHA types
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptchaType {
    ReCaptcha,
    HCaptcha,
    Image,
    Audio,
}

/// Universal user input types (protocol-agnostic)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserInput {
    /// Callback from redirect (OAuth2, SAML, etc.)
    Callback {
        params: HashMap<String, String>,
    },

    /// Code entered by user
    Code { code: String },

    /// CAPTCHA solution
    CaptchaSolution {
        solution: String,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        extra: HashMap<String, String>,
    },

    /// Poll for status (device flow, async operations)
    Poll,

    /// Challenge response (WebAuthn, cryptographic)
    ChallengeResponse {
        response: serde_json::Value,
    },

    /// Confirmation token
    ConfirmationToken { token: String },

    /// Custom input
    Custom {
        input_type: String,
        data: serde_json::Value,
    },
}

/// Universal trait for any credential flow (OAuth2, SAML, JWT, Kerberos, mTLS, etc.)
#[async_trait]
pub trait CredentialFlow: Send + Sync + 'static {
    /// Input type for flow initialization
    type Input: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static;

    /// State type for persistence
    type State: CredentialState;

    /// Flow identifier (matches KIND by default)
    fn flow_name(&self) -> &'static str;

    /// Whether this flow requires user interaction
    fn requires_interaction(&self) -> bool {
        false
    }

    /// Execute the credential flow
    async fn execute(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>;

    /// Refresh the credential (optional)
    async fn refresh(
        &self,
        _state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        Err(CredentialError::refresh_not_supported(
            self.flow_name().to_string(),
        ))
    }

    /// Revoke the credential (optional)
    async fn revoke(
        &self,
        _state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        Ok(())
    }
}
