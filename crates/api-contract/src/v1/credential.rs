//! Credential management request/response DTOs — **Plane B** (auth plane separation).
//!
//! These types form the HTTP API contract for credential lifecycle management.
//! Management projections never include persisted credential material. Acquisition
//! responses intentionally carry short-lived pending tokens, redirect URLs, form
//! fields, and user instructions; treat that transit data as sensitive and
//! never log or casually persist it. Request types carry user-provided configuration
//! that will be validated against the credential type's `ValidSchema` before
//! persistence.

use std::{collections::HashMap, fmt};

use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::{IntoParams, ToSchema};

#[cfg(feature = "openapi")]
fn write_only_object_schema() -> utoipa::openapi::schema::Object {
    utoipa::openapi::schema::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::Object)
        .write_only(Some(true))
        .description(Some(
            "Sensitive caller-supplied credential material; accepted only as input.",
        ))
        .build()
}

#[cfg(feature = "openapi")]
fn read_only_object_schema() -> utoipa::openapi::schema::Object {
    utoipa::openapi::schema::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::Object)
        .read_only(Some(true))
        .description(Some(
            "Server-generated, short-lived interaction authority; returned only as output.",
        ))
        .build()
}

// --- Capabilities ---

/// Capability flags for a credential type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CredentialCapabilities {
    /// Requires multi-step user interaction (for example, an OAuth redirect).
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
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateCredentialRequest {
    /// Credential type key (e.g. "api_key", "basic_auth", "signing_key").
    pub credential_key: String,
    /// Human-readable display name.
    pub name: String,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Type-specific input data matching the credential's schema.
    #[cfg_attr(feature = "openapi", schema(schema_with = write_only_object_schema))]
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
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateCredentialRequest {
    /// Updated display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Updated description.
    #[serde(default)]
    pub description: Option<String>,
    /// Updated type-specific data.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(schema_with = write_only_object_schema))]
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
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
    /// Durable credential availability state.
    pub lifecycle: CredentialLifecycleState,
    /// User-defined tags.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

impl fmt::Debug for CredentialResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialResponse")
            .field("id", &self.id)
            .field("credential_key", &self.credential_key)
            .field("name", &REDACTED)
            .field("description_present", &self.description.is_some())
            .field("auth_pattern", &self.auth_pattern)
            .field("capabilities", &self.capabilities)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("expires_at", &self.expires_at)
            .field("version", &self.version)
            .field("lifecycle", &self.lifecycle)
            .field("tag_count", &self.tags.len())
            .finish()
    }
}

/// Lightweight credential summary for list responses.
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
    /// Durable credential availability state.
    pub lifecycle: CredentialLifecycleState,
}

impl fmt::Debug for CredentialSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialSummary")
            .field("id", &self.id)
            .field("credential_key", &self.credential_key)
            .field("name", &REDACTED)
            .field("auth_pattern", &self.auth_pattern)
            .field("expires_at", &self.expires_at)
            .field("version", &self.version)
            .field("lifecycle", &self.lifecycle)
            .finish()
    }
}

/// Public credential availability state.
///
/// Active refresh ownership remains internal because claims and leases are
/// transient implementation details rather than durable client state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CredentialLifecycleState {
    /// No durable retry or authorization gate prevents use.
    Ready,
    /// Automatic refresh is deferred until the backend-authored instant.
    RefreshDeferred {
        /// RFC 3339 instant after which refresh may be attempted again.
        retry_at: String,
    },
    /// Automatic refresh is durably blocked until material changes.
    RefreshBlocked,
    /// Interactive authorization must complete before the credential is usable.
    ReauthRequired,
    /// A provider operation is in flight under server-owned authority.
    OperationInFlight {
        /// The operation currently crossing the provider boundary.
        operation: CredentialReconcileOperationV1,
    },
    /// An ambiguous provider outcome requires an operator decision.
    ReconciliationRequired {
        /// The affected operation. `None` identifies a legacy incident that
        /// predates durable operation typing and cannot be adjudicated.
        operation: Option<CredentialReconcileOperationV1>,
        /// The incident a reconciliation request must name. Absent only while
        /// a legacy unclassified operation is still in flight.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "uuid"))]
        incident: Option<uuid::Uuid>,
    },
}

/// Paginated list of credential summaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
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

/// New authorization properties for an existing credential.
/// Identity, type, and concurrency evidence are derived by the server.
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ReauthorizeCredentialRequest {
    /// Type-specific properties for the new authorization attempt.
    #[cfg_attr(feature = "openapi", schema(schema_with = write_only_object_schema))]
    pub data: serde_json::Value,
}

impl fmt::Debug for ReauthorizeCredentialRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReauthorizeCredentialRequest")
            .field("data", &REDACTED)
            .finish()
    }
}

/// Reauthorization uses the universal acquisition response and continuation.
pub type ReauthorizeCredentialResponse = ResolveCredentialResponse;

/// Request body for initiating credential acquisition/resolution.
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ResolveCredentialRequest {
    /// Credential type key to resolve.
    pub credential_key: String,
    /// Type-specific form field values matching the credential's input schema.
    #[cfg_attr(feature = "openapi", schema(schema_with = write_only_object_schema))]
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
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FormPostField {
    /// Form field name. Names may reveal protocol-specific state and are
    /// sensitive transit data.
    pub name: String,
    /// Sensitive form value sent to the provider. This may contain a SAML
    /// assertion, RelayState, authorization response, or other bearer material.
    #[cfg_attr(feature = "openapi", schema(read_only = true))]
    pub value: String,
}

/// Interaction type required to continue a pending credential
/// acquisition. Mirrors the credential contract's `InteractionRequest`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcquisitionInteraction {
    /// User must be redirected to this URL (e.g. OAuth2 authorization_code).
    Redirect {
        /// Sensitive, short-lived redirect URL; query parameters may contain
        /// anti-CSRF state or other bearer-adjacent protocol data.
        #[cfg_attr(feature = "openapi", schema(format = "uri", read_only = true))]
        url: String,
    },
    /// Client must auto-submit a POST form to the IdP (e.g. SAML POST binding).
    FormPost {
        /// IdP endpoint URL. Treat the complete interaction as sensitive
        /// transit data together with its form fields.
        #[cfg_attr(feature = "openapi", schema(format = "uri", read_only = true))]
        url: String,
        /// Form fields to submit.
        fields: Vec<FormPostField>,
    },
    /// Information the user must act on.
    DisplayInfo {
        /// Dialog title.
        title: String,
        /// Instructional message.
        message: String,
        /// Sensitive structured display payload containing protocol instructions.
        #[cfg_attr(feature = "openapi", schema(schema_with = read_only_object_schema))]
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
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
        #[cfg_attr(feature = "openapi", schema(format = "password", read_only = true))]
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
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ContinueResolveRequest {
    /// Credential type key the pending acquisition was started for.
    pub credential_key: String,
    /// Token from a previous `Pending` response.
    #[cfg_attr(feature = "openapi", schema(format = "password", write_only = true))]
    pub pending_token: String,
    /// Typed continuation payload — the serialized `UserInput` shape:
    /// `"Poll"`, `{"Code":{"code":".."}}`, `{"Callback":{"params":{..}}}`,
    /// or `{"FormData":{"params":{..}}}`.
    #[cfg_attr(feature = "openapi", schema(schema_with = write_only_object_schema))]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RevokeCredentialResponse {
    /// Whether the revocation succeeded.
    pub revoked: bool,
    /// Human-readable result message.
    pub message: String,
}

// --- Reconciliation ---

/// Version 1 wire vocabulary for the provider operation being reconciled.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CredentialReconcileOperationV1 {
    /// Refresh provider material and finalize the refreshed state locally.
    #[default]
    Refresh,
    /// Revoke provider material and finalize a local tombstone.
    Revoke,
}

/// Version 1 wire vocabulary for the provider outcome an operator established
/// for a typed credential operation whose claim expired in flight.
///
/// The API owns this vocabulary instead of serializing the port's
/// `CredentialOperationDecision`, which carries no serde derive. The wire spellings
/// are the same bytes on purpose: the durable `adjudication_decision` column
/// stores the port spelling, so a drift here would record a decision this route
/// accepted as a row storage can no longer decode. The unit test
/// `reconcile_decision_wire_spelling_matches_the_durable_port_spelling` pins
/// that equality. Refresh and revoke use disjoint outcomes; existing spellings
/// retain their meaning when a new operation is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CredentialReconcileDecisionV1 {
    /// The provider did apply the refresh; the refreshed material is durable
    /// and the credential may be refreshed again.
    ProviderApplied,
    /// The provider never applied the refresh; the previously stored material
    /// stands and the credential may be refreshed again.
    ProviderNotApplied,
    /// The provider revoked the credential.
    ProviderRevoked,
    /// The provider did not revoke the credential.
    ProviderNotRevoked,
}

/// Request body for reconciling a credential's retained provider-operation claim.
///
/// Reconciliation is an operator evidence command. Refresh reconciliation
/// resolves the retained poison; a confirmed revoke is finalized as an atomic
/// tombstone by the authoritative persistence adapter.
#[derive(Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReconcileCredentialRequest {
    /// Provider operation whose outcome is being reconciled.
    ///
    /// Omitted legacy requests retain their historical refresh meaning. New
    /// clients should always send this field explicitly.
    #[serde(default)]
    pub operation: CredentialReconcileOperationV1,
    /// The incident this decision was established for, as published in the
    /// credential's `reconciliation_required` lifecycle state.
    ///
    /// A retry after a lost acknowledgement names the same incident and is
    /// answered from its record; it can never resolve a newer incident on the
    /// same credential, which answers 409 `stale_incident` instead.
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "uuid"))]
    pub incident: uuid::Uuid,
    /// The provider outcome the operator has established.
    pub decision: CredentialReconcileDecisionV1,
    /// Operator note justifying the decision. Audited durable text, never
    /// credential material; the credential adapter caps its length.
    pub evidence: String,
}

impl fmt::Debug for ReconcileCredentialRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReconcileCredentialRequest")
            .field("operation", &self.operation)
            .field("incident", &self.incident)
            .field("decision", &self.decision)
            .field("evidence", &REDACTED)
            .finish()
    }
}

/// Response from reconciling a credential's retained provider-operation claim.
///
/// A 200 means the decision is on record, so there is deliberately no
/// `reconciled: bool` — a flag that is always `true` would only blur the one
/// distinction a client needs, which is `changed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReconcileCredentialResponse {
    /// Provider operation whose outcome is on record.
    pub operation: CredentialReconcileOperationV1,
    /// The incident whose outcome is on record.
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "uuid"))]
    pub incident: uuid::Uuid,
    /// The provider outcome now on record for the credential.
    pub decision: CredentialReconcileDecisionV1,
    /// Whether this call recorded the decision. `false` is the idempotent
    /// recommit of an already-recorded `(decision, evidence)` pair: a success,
    /// not a conflict.
    pub changed: bool,
    /// SHA-256 of the evidence whose resolution is on record, lowercase hex.
    ///
    /// The durable half of the reconciliation retry identity — the conflict
    /// identity the record is keyed on. A digest, not a secret, so it is
    /// visible in `Debug`; a client holding its original evidence can confirm
    /// what is on record and repeat the exact request for a `changed: false`
    /// no-op instead of a 409.
    pub evidence_digest: String,
    /// Human-readable result message.
    pub message: String,
}

// --- Type discovery ---

/// Metadata and schema for a registered credential type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
