//! Credential management request/response DTOs — **Plane B** (auth plane separation).
//!
//! These types form the HTTP API contract for credential lifecycle management.
//! Management projections never include persisted credential material. Acquisition
//! responses intentionally carry short-lived pending tokens, redirect URLs, form
//! fields, and device/user instructions; treat that transit data as sensitive and
//! never log or casually persist it. Request types carry user-provided configuration
//! that will be validated against the credential type's `ValidSchema` before
//! persistence.

use std::{collections::HashMap, fmt};

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

// --- Capabilities ---

/// Capability flags for a credential type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CredentialCapabilities {
    /// Requires multi-step user interaction (e.g. OAuth redirect, device code).
    pub interactive: bool,
    /// Supports token refresh (e.g. OAuth2 refresh_token).
    pub refreshable: bool,
    /// Supports connection testing.
    pub testable: bool,
    /// Supports explicit revocation.
    pub revocable: bool,
}

// --- CRUD ---

/// Request body for creating a new credential.
#[derive(Deserialize, ToSchema)]
pub struct CreateCredentialRequest {
    /// Credential type key (e.g. "api_key", "basic_auth", "signing_key").
    pub credential_key: String,
    /// Human-readable display name.
    pub name: String,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Type-specific input data matching the credential's schema.
    pub data: serde_json::Value,
    /// Optional user-defined tags.
    #[serde(default)]
    pub tags: Option<HashMap<String, String>>,
}

impl fmt::Debug for CreateCredentialRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateCredentialRequest")
            .field("credential_key", &self.credential_key)
            .field("name", &REDACTED)
            .field("description_present", &self.description.is_some())
            .field("data", &REDACTED)
            .field("tags_present", &self.tags.is_some())
            .finish()
    }
}

/// Request body for updating an existing credential.
#[derive(Deserialize, ToSchema)]
pub struct UpdateCredentialRequest {
    /// Updated display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Updated description.
    #[serde(default)]
    pub description: Option<String>,
    /// Updated type-specific data.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    /// Updated tags (replaces all tags if provided).
    #[serde(default)]
    pub tags: Option<HashMap<String, String>>,
    /// Expected version for compare-and-swap optimistic locking.
    /// If provided, the update will fail with 409 Conflict if the
    /// stored version doesn't match.
    #[serde(default)]
    pub version: Option<u64>,
}

impl fmt::Debug for UpdateCredentialRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateCredentialRequest")
            .field("name_present", &self.name.is_some())
            .field("description_present", &self.description.is_some())
            .field("data_present", &self.data.is_some())
            .field("tags_present", &self.tags.is_some())
            .field("version", &self.version)
            .finish()
    }
}

/// Full credential metadata response — **never includes secrets**.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CredentialResponse {
    /// Unique credential identifier.
    pub id: String,
    /// Credential type key.
    pub credential_key: String,
    /// Human-readable display name.
    pub name: String,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Authentication pattern classification (e.g. "OAuth2", "SecretToken").
    pub auth_pattern: String,
    /// Capability flags for this credential type.
    pub capabilities: CredentialCapabilities,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// ISO 8601 expiration timestamp, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Monotonic version for CAS operations.
    pub version: u64,
    /// True when the credential cannot be used until re-authorized
    /// (e.g. an OAuth2 flow was started but not completed, or a refresh
    /// failed terminally).
    pub reauth_required: bool,
    /// User-defined tags.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// Lightweight credential summary for list responses.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CredentialSummary {
    /// Unique credential identifier.
    pub id: String,
    /// Credential type key.
    pub credential_key: String,
    /// Human-readable display name.
    pub name: String,
    /// Authentication pattern classification.
    pub auth_pattern: String,
    /// ISO 8601 expiration timestamp, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Monotonic version for CAS operations.
    pub version: u64,
    /// True when the credential cannot be used until re-authorized.
    pub reauth_required: bool,
}

/// Paginated list of credential summaries.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListCredentialsResponse {
    /// Credential summaries for the current page.
    pub credentials: Vec<CredentialSummary>,
    /// Total number of credentials matching the query.
    pub total: usize,
    /// Current page number (1-based).
    pub page: usize,
    /// Number of items per page.
    pub page_size: usize,
}

/// Query parameters for listing credentials.
#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListCredentialsQuery {
    /// Page number (1-based). Defaults to 1.
    #[serde(default = "default_page")]
    pub page: usize,
    /// Items per page (max 100). Defaults to 20.
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    /// Optional filter by credential type key.
    #[serde(default)]
    pub credential_key: Option<String>,
    /// Optional filter by authentication pattern.
    #[serde(default)]
    pub auth_pattern: Option<String>,
}

// --- Acquisition (resolve / continue) ---

const REDACTED: &str = "[REDACTED]";

/// Request body for initiating credential acquisition/resolution.
#[derive(Deserialize, ToSchema)]
pub struct ResolveCredentialRequest {
    /// Credential type key to resolve.
    pub credential_key: String,
    /// Type-specific form field values matching the credential's input schema.
    pub data: serde_json::Value,
}

impl fmt::Debug for ResolveCredentialRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolveCredentialRequest")
            .field("credential_key", &self.credential_key)
            .field("data", &REDACTED)
            .finish()
    }
}

/// One form field of a `form_post` interaction.
#[derive(Serialize, ToSchema)]
pub struct FormPostField {
    /// Form field name. Names may reveal protocol-specific state and are
    /// sensitive transit data.
    pub name: String,
    /// Sensitive form value sent to the provider. This may contain a SAML
    /// assertion, RelayState, authorization response, or other bearer material.
    pub value: String,
}

/// Interaction type required to continue a pending credential
/// acquisition. Mirrors the credential contract's `InteractionRequest`.
#[derive(Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcquisitionInteraction {
    /// User must be redirected to this URL (e.g. OAuth2 authorization_code).
    Redirect {
        /// Sensitive, short-lived redirect URL; query parameters may contain
        /// anti-CSRF state or other bearer-adjacent protocol data.
        url: String,
    },
    /// Client must auto-submit a POST form to the IdP (e.g. SAML POST binding).
    FormPost {
        /// IdP endpoint URL. Treat the complete interaction as sensitive
        /// transit data together with its form fields.
        url: String,
        /// Form fields to submit.
        fields: Vec<FormPostField>,
    },
    /// Information the user must act on (device code, instructions).
    DisplayInfo {
        /// Dialog title.
        title: String,
        /// Instructional message.
        message: String,
        /// Sensitive structured display payload (e.g. a device `UserCode`
        /// with its verification URI, or protocol instructions).
        data: serde_json::Value,
        /// Seconds until this information expires.
        #[serde(skip_serializing_if = "Option::is_none")]
        expires_in: Option<u64>,
    },
}

impl fmt::Debug for AcquisitionInteraction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Redirect { .. } => formatter
                .debug_struct("Redirect")
                .field("url", &REDACTED)
                .finish(),
            Self::FormPost { fields, .. } => formatter
                .debug_struct("FormPost")
                .field("url", &REDACTED)
                .field("field_count", &fields.len())
                .finish(),
            Self::DisplayInfo { expires_in, .. } => formatter
                .debug_struct("DisplayInfo")
                .field("payload", &REDACTED)
                .field("expires_in", expires_in)
                .finish(),
        }
    }
}

/// Result of a resolve or continue_resolve operation.
#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResolveCredentialResponse {
    /// Acquisition completed — credential is persisted.
    Complete {
        /// ID of the newly persisted credential.
        credential_id: String,
    },
    /// Acquisition requires further interaction.
    Pending {
        /// Sensitive opaque bearer token used to continue the acquisition
        /// flow. It is short-lived and must not be logged or casually persisted.
        pending_token: String,
        /// Interaction the client must perform next.
        interaction: AcquisitionInteraction,
    },
    /// The framework asked the client to poll the continuation again.
    Retry {
        /// Seconds to wait before re-calling `resolve/continue`.
        retry_after_secs: u64,
    },
}

impl fmt::Debug for ResolveCredentialResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Complete { credential_id } => formatter
                .debug_struct("Complete")
                .field("credential_id", credential_id)
                .finish(),
            Self::Pending { interaction, .. } => formatter
                .debug_struct("Pending")
                .field("pending_token", &REDACTED)
                .field("interaction", interaction)
                .finish(),
            Self::Retry { retry_after_secs } => formatter
                .debug_struct("Retry")
                .field("retry_after_secs", retry_after_secs)
                .finish(),
        }
    }
}

/// Request body for continuing a pending credential acquisition.
#[derive(Deserialize, ToSchema)]
pub struct ContinueResolveRequest {
    /// Credential type key the pending acquisition was started for.
    pub credential_key: String,
    /// Token from a previous `Pending` response.
    pub pending_token: String,
    /// Typed continuation payload — the serialized `UserInput` shape:
    /// `"Poll"`, `{"Code":{"code":".."}}`, `{"Callback":{"params":{..}}}`,
    /// or `{"FormData":{"params":{..}}}`.
    pub user_input: serde_json::Value,
}

impl fmt::Debug for ContinueResolveRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContinueResolveRequest")
            .field("credential_key", &self.credential_key)
            .field("pending_token", &REDACTED)
            .field("user_input", &REDACTED)
            .finish()
    }
}

/// Alias — continue has the same response shape as initial resolve.
pub type ContinueResolveResponse = ResolveCredentialResponse;

// --- Lifecycle (test / refresh / revoke) ---

/// Version 1 wire classification for a failed credential connectivity test.
///
/// The core credential adapter maps untrusted provider text to a payload-free
/// classification before it reaches this transport contract. This v1 wire set
/// is intentionally exhaustive and frozen: newer core classifications map to
/// `other` until a new transport version deliberately exposes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialTestFailureCodeV1 {
    /// The provider rejected the presented authentication material.
    AuthenticationRejected,
    /// Authentication succeeded but required permission is missing.
    PermissionDenied,
    /// The provider account is disabled, locked, suspended, or restricted.
    AccountRestricted,
    /// Credential or provider-specific setup is invalid.
    InvalidConfiguration,
    /// Another safely classified provider rejection.
    Other,
}

/// Response from testing a credential's connectivity.
///
/// The tagged shape makes contradictory states unrepresentable: success has
/// no failure code, while every failure carries exactly one frozen v1 code.
/// The two status variants are intentionally exhaustive; adding another v1
/// status would be a breaking wire change.
#[derive(Clone, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TestCredentialResponse {
    /// The provider accepted the credential.
    Success {
        /// Fixed, platform-owned human-readable message.
        message: String,
        /// ISO 8601 timestamp of when the test was performed.
        tested_at: String,
    },
    /// The provider rejected the credential.
    Failed {
        /// Stable v1 failure classification.
        code: CredentialTestFailureCodeV1,
        /// Fixed, platform-owned human-readable message. Provider text is
        /// never copied into this field.
        message: String,
        /// ISO 8601 timestamp of when the test was performed.
        tested_at: String,
    },
}

impl fmt::Debug for TestCredentialResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Success { tested_at, .. } => formatter
                .debug_struct("Success")
                .field("message", &"[PLATFORM MESSAGE]")
                .field("tested_at", tested_at)
                .finish(),
            Self::Failed {
                code, tested_at, ..
            } => formatter
                .debug_struct("Failed")
                .field("code", code)
                .field("message", &"[PLATFORM MESSAGE]")
                .field("tested_at", tested_at)
                .finish(),
        }
    }
}

/// Response from refreshing a credential's tokens.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RefreshCredentialResponse {
    /// Whether the refresh succeeded.
    pub refreshed: bool,
    /// Human-readable result message.
    pub message: String,
    /// New expiration timestamp if the refresh changed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_expires_at: Option<String>,
}

/// Response from revoking a credential.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RevokeCredentialResponse {
    /// Whether the revocation succeeded.
    pub revoked: bool,
    /// Human-readable result message.
    pub message: String,
}

// --- Type discovery ---

/// Metadata and schema for a registered credential type.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CredentialTypeInfo {
    /// Unique type key (e.g. "api_key", "basic_auth", "signing_key").
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Description of the credential type.
    pub description: String,
    /// Authentication pattern classification.
    pub auth_pattern: String,
    /// Capability flags.
    pub capabilities: CredentialCapabilities,
    /// JSON Schema describing the input fields for this credential type.
    pub schema: serde_json::Value,
    /// Optional icon identifier or URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Optional link to documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
}

/// Response listing all registered credential types.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListCredentialTypesResponse {
    /// Available credential types.
    pub types: Vec<CredentialTypeInfo>,
}

// --- Pagination helpers ---

fn default_page() -> usize {
    1
}

fn default_page_size() -> usize {
    20
}

impl ListCredentialsQuery {
    /// Compute the SQL/store offset from page number.
    pub fn offset(&self) -> usize {
        self.page.saturating_sub(1).saturating_mul(self.page_size)
    }

    /// Clamped page size (max 100).
    pub fn limit(&self) -> usize {
        self.page_size.min(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET_CANARY: &str = "credential-dto-secret-NEVER-DEBUG-c41f";

    #[test]
    fn create_request_debug_redacts_free_form_fields() {
        let request = CreateCredentialRequest {
            credential_key: "api_key".to_owned(),
            name: SECRET_CANARY.to_owned(),
            description: Some(SECRET_CANARY.to_owned()),
            data: serde_json::json!({ "api_key": SECRET_CANARY }),
            tags: Some(HashMap::from([(
                SECRET_CANARY.to_owned(),
                SECRET_CANARY.to_owned(),
            )])),
        };

        let debug = format!("{request:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "create request Debug must not expose free-form input: {debug}"
        );
    }

    #[test]
    fn update_request_debug_redacts_free_form_fields() {
        let request = UpdateCredentialRequest {
            name: Some(SECRET_CANARY.to_owned()),
            description: Some(SECRET_CANARY.to_owned()),
            data: Some(serde_json::json!({ "api_key": SECRET_CANARY })),
            tags: Some(HashMap::from([(
                SECRET_CANARY.to_owned(),
                SECRET_CANARY.to_owned(),
            )])),
            version: Some(42),
        };

        let debug = format!("{request:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "update request Debug must not expose free-form input: {debug}"
        );
        assert!(
            debug.contains("42"),
            "safe CAS version should remain visible"
        );
    }

    #[test]
    fn resolve_request_debug_redacts_input_data() {
        let request = ResolveCredentialRequest {
            credential_key: "probe".to_owned(),
            data: serde_json::json!({ "client_secret": SECRET_CANARY }),
        };

        let debug = format!("{request:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "resolve request Debug must not expose input data: {debug}"
        );
    }

    #[test]
    fn continue_request_debug_redacts_token_and_user_input() {
        let request = ContinueResolveRequest {
            credential_key: "probe".to_owned(),
            pending_token: SECRET_CANARY.to_owned(),
            user_input: serde_json::json!({ "code": SECRET_CANARY }),
        };

        let debug = format!("{request:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "continue request Debug must not expose token or user input: {debug}"
        );
    }

    #[test]
    fn pending_response_debug_redacts_token_and_interaction_payload() {
        let response = ResolveCredentialResponse::Pending {
            pending_token: SECRET_CANARY.to_owned(),
            interaction: AcquisitionInteraction::FormPost {
                url: format!("https://provider.example/submit?state={SECRET_CANARY}"),
                fields: vec![FormPostField {
                    name: SECRET_CANARY.to_owned(),
                    value: SECRET_CANARY.to_owned(),
                }],
            },
        };

        let debug = format!("{response:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "pending response Debug must not expose its token or interaction payload: {debug}"
        );
    }

    #[test]
    fn test_response_debug_redacts_message_payload() {
        let responses = [
            TestCredentialResponse::Success {
                message: SECRET_CANARY.to_owned(),
                tested_at: "2026-07-21T12:34:56Z".to_owned(),
            },
            TestCredentialResponse::Failed {
                code: CredentialTestFailureCodeV1::Other,
                message: SECRET_CANARY.to_owned(),
                tested_at: "2026-07-21T12:34:56Z".to_owned(),
            },
        ];

        for response in responses {
            let debug = format!("{response:?}");
            assert!(
                !debug.contains(SECRET_CANARY),
                "test response Debug must not expose its platform message: {debug}"
            );
        }
    }
}
