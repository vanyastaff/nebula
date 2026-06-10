//! Credential management request/response DTOs — **Plane B** (auth plane separation).
//!
//! These types form the HTTP API contract for credential lifecycle management.
//! Response types **never** include secret material (encrypted state, tokens, keys).
//! Request types carry user-provided configuration that will be validated against
//! the credential type's `ValidSchema` before persistence.

use std::collections::HashMap;

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
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateCredentialRequest {
    /// Credential type key (e.g. "oauth2", "api_key", "basic_auth").
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

/// Request body for updating an existing credential.
#[derive(Debug, Clone, Deserialize, ToSchema)]
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

/// Request body for initiating credential acquisition/resolution.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ResolveCredentialRequest {
    /// Credential type key to resolve.
    pub credential_key: String,
    /// Type-specific form field values matching the credential's input schema.
    pub data: serde_json::Value,
}

/// One form field of a `form_post` interaction.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FormPostField {
    /// Form field name.
    pub name: String,
    /// Form field value (never secret material — interaction payloads
    /// are UI instructions, not credential state).
    pub value: String,
}

/// Interaction type required to continue a pending credential
/// acquisition. Mirrors the credential contract's `InteractionRequest`.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcquisitionInteraction {
    /// User must be redirected to this URL (e.g. OAuth2 authorization_code).
    Redirect {
        /// URL to redirect the user to.
        url: String,
    },
    /// Client must auto-submit a POST form to the IdP (e.g. SAML POST binding).
    FormPost {
        /// IdP endpoint URL.
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
        /// Structured display payload (e.g. `UserCode` with the
        /// verification URI, or plain `Text`).
        data: serde_json::Value,
        /// Seconds until this information expires.
        #[serde(skip_serializing_if = "Option::is_none")]
        expires_in: Option<u64>,
    },
}

/// Result of a resolve or continue_resolve operation.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResolveCredentialResponse {
    /// Acquisition completed — credential is persisted.
    Complete {
        /// ID of the newly persisted credential.
        credential_id: String,
    },
    /// Acquisition requires further interaction.
    Pending {
        /// Opaque token to continue the acquisition flow.
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

/// Request body for continuing a pending credential acquisition.
#[derive(Debug, Clone, Deserialize, ToSchema)]
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

/// Alias — continue has the same response shape as initial resolve.
pub type ContinueResolveResponse = ResolveCredentialResponse;

// --- Lifecycle (test / refresh / revoke) ---

/// Response from testing a credential's connectivity.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TestCredentialResponse {
    /// Whether the connectivity test succeeded.
    pub success: bool,
    /// Human-readable result message.
    pub message: String,
    /// ISO 8601 timestamp of when the test was performed.
    pub tested_at: String,
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
    /// Unique type key (e.g. "oauth2", "api_key", "basic_auth").
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
