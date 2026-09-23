//! Versioned credential-management wire contracts for remote clients.
//!
//! These are transport data only. Tenant proofs, persistence selectors,
//! refresh claims, leases, and runtime authority are deliberately absent.

use std::{collections::BTreeMap, fmt, num::NonZeroU64};

use serde::{Deserialize, Serialize};

const REDACTED: &str = "[REDACTED]";

/// Version 1 of the public credential HTTP contract.
pub mod v1 {
    use super::*;

    /// Capability flags advertised for a credential type.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
    pub struct CredentialCapabilities {
        /// Whether acquisition can require user interaction.
        pub interactive: bool,
        /// Whether the credential supports refresh.
        pub refreshable: bool,
        /// Whether connectivity can be tested.
        pub testable: bool,
        /// Whether provider-side revocation is supported.
        pub revocable: bool,
    }

    /// Input for creating a credential.
    #[derive(Serialize)]
    pub struct CreateCredentialRequest {
        /// Registered credential type key.
        pub credential_key: String,
        /// Human-readable display name.
        pub name: String,
        /// Optional description.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub description: Option<String>,
        /// Type-specific, potentially secret input.
        pub data: serde_json::Value,
        /// Optional user-defined tags.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub tags: Option<BTreeMap<String, String>>,
    }

    impl fmt::Debug for CreateCredentialRequest {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CreateCredentialRequest")
                .field("credential_key", &self.credential_key)
                .field("name", &REDACTED)
                .field("description_present", &self.description.is_some())
                .field("data", &REDACTED)
                .field("tags_present", &self.tags.is_some())
                .finish()
        }
    }

    /// Input for updating credential metadata or secret material.
    #[derive(Serialize)]
    pub struct UpdateCredentialRequest {
        /// Replacement display name.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub name: Option<String>,
        /// Replacement description.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub description: Option<String>,
        /// Replacement type-specific, potentially secret input.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub data: Option<serde_json::Value>,
        /// Replacement tags.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tags: Option<BTreeMap<String, String>>,
        /// Expected version for optimistic concurrency.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub version: Option<u64>,
    }

    impl fmt::Debug for UpdateCredentialRequest {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("UpdateCredentialRequest")
                .field("name_present", &self.name.is_some())
                .field("description_present", &self.description.is_some())
                .field("data_present", &self.data.is_some())
                .field("tags_present", &self.tags.is_some())
                .field("version", &self.version)
                .finish()
        }
    }

    /// Query parameters for listing credentials.
    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
    pub struct ListCredentialsRequest {
        /// One-based page number. The server defaults this to one.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub page: Option<usize>,
        /// Requested page size. The server caps this at 100.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub page_size: Option<usize>,
        /// Optional credential type filter.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub credential_key: Option<String>,
        /// Optional authentication-pattern filter.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub auth_pattern: Option<String>,
    }

    /// Secret-free credential metadata returned by create and get.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    pub struct Credential {
        /// Credential identifier.
        pub id: String,
        /// Registered credential type key.
        pub credential_key: String,
        /// Human-readable display name.
        pub name: String,
        /// Optional description.
        pub description: Option<String>,
        /// Authentication-pattern classification.
        pub auth_pattern: String,
        /// Capabilities of the registered type.
        pub capabilities: CredentialCapabilities,
        /// ISO 8601 creation timestamp.
        pub created_at: String,
        /// ISO 8601 last-update timestamp.
        pub updated_at: String,
        /// ISO 8601 expiry timestamp, when present.
        pub expires_at: Option<String>,
        /// Monotonic version used by mutation contracts.
        pub version: u64,
        /// Durable availability state.
        pub lifecycle: CredentialLifecycleState,
        /// User-defined tags.
        #[serde(default)]
        pub tags: BTreeMap<String, String>,
    }

    /// Response returned after creating a credential.
    pub type CreateCredentialResponse = Credential;

    /// Response returned when getting one credential.
    pub type GetCredentialResponse = Credential;

    /// Secret-free credential projection returned by list.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    pub struct CredentialSummary {
        /// Credential identifier.
        pub id: String,
        /// Registered credential type key.
        pub credential_key: String,
        /// Human-readable display name.
        pub name: String,
        /// Authentication-pattern classification.
        pub auth_pattern: String,
        /// ISO 8601 expiry timestamp, when present.
        pub expires_at: Option<String>,
        /// Monotonic version.
        pub version: u64,
        /// Durable availability state.
        pub lifecycle: CredentialLifecycleState,
    }

    /// One page of credential summaries.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    pub struct ListCredentialsResponse {
        /// Credentials in this page.
        pub credentials: Vec<CredentialSummary>,
        /// Total matching credentials.
        pub total: usize,
        /// One-based current page.
        pub page: usize,
        /// Page size used by the server.
        pub page_size: usize,
    }

    /// Successful response for delete operations.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
    pub struct DeleteCredentialResponse {
        /// Always true on the success path.
        pub ok: bool,
    }

    /// Public durable availability of a credential.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "status", rename_all = "snake_case")]
    #[non_exhaustive]
    pub enum CredentialLifecycleState {
        /// No durable gate prevents use.
        Ready,
        /// Automatic refresh is deferred until the backend-authored instant.
        RefreshDeferred {
            /// RFC 3339 retry instant.
            retry_at: String,
        },
        /// Automatic refresh is blocked until material changes.
        RefreshBlocked,
        /// Interactive authorization must complete before use.
        ReauthRequired,
    }

    /// Input for starting universal credential acquisition.
    #[derive(Serialize)]
    pub struct ResolveCredentialRequest {
        /// Registered credential type key.
        pub credential_key: String,
        /// Type-specific, potentially secret input.
        pub data: serde_json::Value,
    }

    impl fmt::Debug for ResolveCredentialRequest {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("ResolveCredentialRequest")
                .field("credential_key", &self.credential_key)
                .field("data", &REDACTED)
                .finish()
        }
    }

    /// One field in a form-post interaction.
    #[derive(Deserialize)]
    pub struct FormPostField {
        /// Form field name.
        pub name: String,
        /// Sensitive form value.
        pub value: String,
    }

    impl fmt::Debug for FormPostField {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("FormPostField")
                .field("name", &REDACTED)
                .field("value", &REDACTED)
                .finish()
        }
    }

    /// Interaction required to continue acquisition.
    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    #[non_exhaustive]
    pub enum AcquisitionInteraction {
        /// Redirect the user to a short-lived provider URL.
        Redirect {
            /// Sensitive redirect URL.
            url: String,
        },
        /// Submit a form to a provider endpoint.
        FormPost {
            /// Sensitive provider URL.
            url: String,
            /// Sensitive form fields.
            fields: Vec<FormPostField>,
        },
        /// Display provider instructions to the user.
        DisplayInfo {
            /// Dialog title.
            title: String,
            /// Instructional message.
            message: String,
            /// Potentially sensitive structured instructions.
            data: serde_json::Value,
            /// Lifetime in seconds, when supplied.
            expires_in: Option<u64>,
        },
    }

    impl fmt::Debug for AcquisitionInteraction {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Redirect { .. } => {
                    f.debug_struct("Redirect").field("url", &REDACTED).finish()
                },
                Self::FormPost { fields, .. } => f
                    .debug_struct("FormPost")
                    .field("url", &REDACTED)
                    .field("field_count", &fields.len())
                    .finish(),
                Self::DisplayInfo { expires_in, .. } => f
                    .debug_struct("DisplayInfo")
                    .field("payload", &REDACTED)
                    .field("expires_in", expires_in)
                    .finish(),
            }
        }
    }

    /// Result of starting or continuing acquisition.
    #[derive(Deserialize)]
    #[serde(tag = "status", rename_all = "snake_case")]
    #[non_exhaustive]
    pub enum ResolveCredentialResponse {
        /// Acquisition completed and the credential was persisted.
        Complete {
            /// New credential identifier.
            credential_id: String,
        },
        /// Further user interaction is required.
        Pending {
            /// Sensitive, short-lived continuation bearer token.
            pending_token: String,
            /// Next interaction.
            interaction: AcquisitionInteraction,
        },
        /// Poll the continuation endpoint again after this delay.
        Retry {
            /// Minimum delay in seconds.
            retry_after_secs: u64,
        },
    }

    impl fmt::Debug for ResolveCredentialResponse {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Complete { credential_id } => f
                    .debug_struct("Complete")
                    .field("credential_id", credential_id)
                    .finish(),
                Self::Pending { interaction, .. } => f
                    .debug_struct("Pending")
                    .field("pending_token", &REDACTED)
                    .field("interaction", interaction)
                    .finish(),
                Self::Retry { retry_after_secs } => f
                    .debug_struct("Retry")
                    .field("retry_after_secs", retry_after_secs)
                    .finish(),
            }
        }
    }

    /// Input for continuing a pending acquisition.
    #[derive(Serialize)]
    pub struct ContinueResolveCredentialRequest {
        /// Credential type used to start acquisition.
        pub credential_key: String,
        /// Sensitive continuation bearer token.
        pub pending_token: String,
        /// Potentially sensitive typed continuation input.
        pub user_input: serde_json::Value,
    }

    /// Response returned when continuing acquisition.
    pub type ContinueResolveCredentialResponse = ResolveCredentialResponse;

    impl fmt::Debug for ContinueResolveCredentialRequest {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("ContinueResolveCredentialRequest")
                .field("credential_key", &self.credential_key)
                .field("pending_token", &REDACTED)
                .field("user_input", &REDACTED)
                .finish()
        }
    }

    /// Response from provider-side revocation.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    pub struct RevokeCredentialResponse {
        /// Whether revocation succeeded.
        pub revoked: bool,
        /// Human-readable result for display only.
        pub message: String,
    }

    /// One field-level RFC 9457 validation diagnostic.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    pub struct ValidationProblem {
        /// Stable machine-readable rejection code.
        pub code: String,
        /// Human-readable detail for display only.
        pub detail: String,
        /// RFC 6901 request pointer, when applicable.
        pub pointer: Option<String>,
        /// Logical domain path, when applicable.
        pub path: Option<String>,
        /// Secret-free expected contract.
        pub expected: Option<String>,
        /// Secret-free actual contract.
        pub actual: Option<String>,
        /// Stable remediation guidance.
        pub remediation: Option<String>,
    }

    /// RFC 9457 problem document returned by the public API.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    pub struct ProblemDetails {
        /// URI identifying the problem type.
        #[serde(rename = "type")]
        pub type_uri: String,
        /// Human-readable title.
        pub title: String,
        /// HTTP status code repeated in the document.
        pub status: u16,
        /// Human-readable occurrence detail.
        pub detail: Option<String>,
        /// URI identifying this occurrence.
        pub instance: Option<String>,
        /// Structured validation diagnostics.
        pub errors: Option<Vec<ValidationProblem>>,
        /// Problem-specific RFC 9457 extension members.
        #[serde(flatten)]
        pub extensions: BTreeMap<String, serde_json::Value>,
    }

    impl ProblemDetails {
        /// Classify a credential problem without parsing human-readable text.
        #[must_use]
        pub fn credential_kind(&self) -> CredentialProblemKind {
            CredentialProblemKind::from_type_uri(&self.type_uri)
        }
    }

    /// Stable client classification for credential problem types.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum CredentialProblemKind {
        /// Interactive reconnection is required.
        ReauthRequired,
        /// Refresh was proven not applied and must not be retried automatically.
        RefreshNotAppliedNever,
        /// Refresh was proven not applied and may be retried after `Retry-After`.
        RefreshNotAppliedAfter,
        /// Refresh completed but durable finalization requires reconciliation.
        RefreshReconciliationRequired,
        /// Revoke completed but durable finalization requires reconciliation.
        RevokeReconciliationRequired,
        /// A mutation acknowledgement was lost.
        OutcomeUnknown,
        /// A different or newer problem type.
        Other,
    }

    impl CredentialProblemKind {
        fn from_type_uri(type_uri: &str) -> Self {
            match type_uri {
                "https://nebula.dev/problems/credential-reauth-required" => Self::ReauthRequired,
                "https://nebula.dev/problems/credential-refresh-not-applied" => {
                    Self::RefreshNotAppliedNever
                },
                "https://nebula.dev/problems/credential-refresh-reconciliation-required" => {
                    Self::RefreshReconciliationRequired
                },
                "https://nebula.dev/problems/credential-revoke-reconciliation-required" => {
                    Self::RevokeReconciliationRequired
                },
                "https://nebula.dev/problems/outcome-unknown" => Self::OutcomeUnknown,
                _ => Self::Other,
            }
        }

        /// Stable API diagnostic code corresponding to this known problem.
        #[must_use]
        pub const fn code(self) -> Option<&'static str> {
            match self {
                Self::ReauthRequired => Some("API:CREDENTIAL_REAUTH_REQUIRED"),
                Self::RefreshNotAppliedNever => Some("API:CREDENTIAL_REFRESH_NOT_APPLIED_NEVER"),
                Self::RefreshNotAppliedAfter => Some("API:CREDENTIAL_REFRESH_NOT_APPLIED_AFTER"),
                Self::RefreshReconciliationRequired => {
                    Some("API:CREDENTIAL_REFRESH_RECONCILIATION_REQUIRED")
                },
                Self::RevokeReconciliationRequired => {
                    Some("API:CREDENTIAL_REVOKE_RECONCILIATION_REQUIRED")
                },
                Self::OutcomeUnknown => Some("API:OUTCOME_UNKNOWN"),
                Self::Other => None,
            }
        }
    }

    /// Parsed non-zero `Retry-After` delay associated with an HTTP response.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct RetryAfter(NonZeroU64);

    impl RetryAfter {
        /// Construct from the whole-second HTTP header value.
        #[must_use]
        pub const fn from_seconds(seconds: u64) -> Option<Self> {
            match NonZeroU64::new(seconds) {
                Some(seconds) => Some(Self(seconds)),
                None => None,
            }
        }

        /// Return the non-zero whole-second delay.
        #[must_use]
        pub const fn seconds(self) -> u64 {
            self.0.get()
        }
    }

    /// Typed failure response assembled by a future HTTP transport.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CredentialProblem {
        /// RFC 9457 response body.
        pub problem: ProblemDetails,
        /// Valid non-zero `Retry-After` header, when supplied.
        pub retry_after: Option<RetryAfter>,
    }

    impl CredentialProblem {
        /// Classify the problem using its type URI and typed response metadata.
        #[must_use]
        pub fn credential_kind(&self) -> CredentialProblemKind {
            let kind = self.problem.credential_kind();
            if kind == CredentialProblemKind::RefreshNotAppliedNever && self.retry_after.is_some() {
                CredentialProblemKind::RefreshNotAppliedAfter
            } else {
                kind
            }
        }
    }
}

// Preserve the pre-v1 public type identity until a declared breaking release.
// New remote-client code should use `v1::CredentialLifecycleState`.
pub use nebula_credential::CredentialLifecycleState;

#[cfg(test)]
mod tests {
    use super::v1::*;
    use serde_json::json;

    #[test]
    fn lifecycle_and_acquisition_tags_match_v1_wire_shape() {
        let lifecycle: CredentialLifecycleState = serde_json::from_value(json!({
            "status": "refresh_deferred",
            "retry_at": "2026-09-24T01:02:03Z"
        }))
        .expect("lifecycle decodes");
        assert!(matches!(
            lifecycle,
            CredentialLifecycleState::RefreshDeferred { .. }
        ));

        for interaction in [
            json!({"type": "redirect", "url": "https://provider.test/redirect"}),
            json!({
                "type": "form_post",
                "url": "https://provider.test/submit",
                "fields": [{"name": "assertion", "value": "secret"}]
            }),
            json!({
                "type": "display_info",
                "title": "Authorize",
                "message": "Enter the code",
                "data": {"code": "secret"},
                "expires_in": 60
            }),
        ] {
            serde_json::from_value::<AcquisitionInteraction>(interaction)
                .expect("interaction variant decodes");
        }

        for response in [
            json!({"status": "complete", "credential_id": "cred_01"}),
            json!({
                "status": "pending",
                "pending_token": "opaque",
                "interaction": {"type": "redirect", "url": "https://provider.test"}
            }),
            json!({"status": "retry", "retry_after_secs": 5}),
        ] {
            serde_json::from_value::<ResolveCredentialResponse>(response)
                .expect("acquisition response variant decodes");
        }
    }

    #[test]
    fn sensitive_debug_is_redacted() {
        let create = CreateCredentialRequest {
            credential_key: "oauth2".to_owned(),
            name: "name-secret".to_owned(),
            description: Some("description-secret".to_owned()),
            data: json!({"client_secret": "input-secret"}),
            tags: None,
        };
        let resolve = ResolveCredentialRequest {
            credential_key: "oauth2".to_owned(),
            data: json!({"client_secret": "resolve-secret"}),
        };
        let update = UpdateCredentialRequest {
            name: Some("updated-name-secret".to_owned()),
            description: Some("updated-description-secret".to_owned()),
            data: Some(json!({"client_secret": "updated-input-secret"})),
            tags: None,
            version: Some(7),
        };
        let continuation = ContinueResolveCredentialRequest {
            credential_key: "oauth2".to_owned(),
            pending_token: "pending-secret".to_owned(),
            user_input: json!({"Code": {"code": "code-secret"}}),
        };
        let pending: ResolveCredentialResponse = serde_json::from_value(json!({
            "status": "pending",
            "pending_token": "response-secret",
            "interaction": {
                "type": "redirect",
                "url": "https://provider.test/?state=url-secret"
            }
        }))
        .expect("pending response decodes");

        let debug = format!("{create:?} {update:?} {resolve:?} {continuation:?} {pending:?}");
        for secret in [
            "name-secret",
            "description-secret",
            "input-secret",
            "resolve-secret",
            "pending-secret",
            "code-secret",
            "response-secret",
            "url-secret",
            "updated-name-secret",
            "updated-description-secret",
            "updated-input-secret",
        ] {
            assert!(!debug.contains(secret), "Debug leaked {secret}: {debug}");
        }
    }

    #[test]
    fn requests_serialize_and_responses_deserialize() {
        let request = ContinueResolveCredentialRequest {
            credential_key: "oauth2".to_owned(),
            pending_token: "opaque".to_owned(),
            user_input: json!("Poll"),
        };
        assert_eq!(
            serde_json::to_value(request).expect("request serializes"),
            json!({
                "credential_key": "oauth2",
                "pending_token": "opaque",
                "user_input": "Poll"
            })
        );

        let update = UpdateCredentialRequest {
            name: None,
            description: Some("new description".to_owned()),
            data: None,
            tags: None,
            version: Some(7),
        };
        assert_eq!(
            serde_json::to_value(update).expect("update request serializes"),
            json!({"description": "new description", "version": 7})
        );

        let page: ListCredentialsResponse = serde_json::from_value(json!({
            "credentials": [{
                "id": "cred_01",
                "credential_key": "oauth2",
                "name": "Provider",
                "auth_pattern": "OAuth2",
                "version": 7,
                "lifecycle": {"status": "reauth_required"}
            }],
            "total": 1,
            "page": 1,
            "page_size": 20
        }))
        .expect("list response decodes");
        assert_eq!(page.credentials.len(), 1);

        let ack: DeleteCredentialResponse =
            serde_json::from_value(json!({"ok": true})).expect("delete ack decodes");
        assert!(ack.ok);
    }

    #[test]
    fn credential_projections_contain_no_internal_authority_fields() {
        let decoded: Credential = serde_json::from_value(json!({
            "id": "cred_01",
            "credential_key": "oauth2",
            "name": "Provider",
            "auth_pattern": "OAuth2",
            "capabilities": {
                "interactive": true,
                "refreshable": true,
                "testable": false,
                "revocable": true
            },
            "created_at": "2026-09-24T00:00:00Z",
            "updated_at": "2026-09-24T00:00:00Z",
            "version": 1,
            "lifecycle": {"status": "ready"},
            "tags": {}
        }))
        .expect("public credential decodes");
        let projection = format!("{decoded:?}");
        for field in [
            "claim",
            "lease",
            "fencing_token",
            "tenant_proof",
            "owner_selector",
            "generation",
        ] {
            assert!(
                !projection.contains(field),
                "public projection exposed {field}"
            );
        }
    }

    #[test]
    fn problems_are_classified_without_human_message_parsing() {
        let problem: ProblemDetails = serde_json::from_value(json!({
            "type": "https://nebula.dev/problems/credential-reauth-required",
            "title": "arbitrary localized title",
            "status": 409,
            "detail": "arbitrary localized detail"
        }))
        .expect("problem decodes");
        let kind = problem.credential_kind();
        assert_eq!(kind, CredentialProblemKind::ReauthRequired);
        assert_eq!(kind.code(), Some("API:CREDENTIAL_REAUTH_REQUIRED"));
        let lookalike: ProblemDetails = serde_json::from_value(json!({
            "type": "https://attacker.test/problems/credential-reauth-required",
            "title": "lookalike",
            "status": 409
        }))
        .expect("lookalike problem decodes");
        assert_eq!(lookalike.credential_kind(), CredentialProblemKind::Other);
        assert_eq!(
            RetryAfter::from_seconds(17).map(RetryAfter::seconds),
            Some(17)
        );
        assert_eq!(RetryAfter::from_seconds(0), None);

        let delayed_problem = CredentialProblem {
            problem: serde_json::from_value(json!({
                "type": "https://nebula.dev/problems/credential-refresh-not-applied",
                "title": "Refresh not applied",
                "status": 409
            }))
            .expect("refresh problem decodes"),
            retry_after: RetryAfter::from_seconds(17),
        };
        assert_eq!(
            delayed_problem.credential_kind().code(),
            Some("API:CREDENTIAL_REFRESH_NOT_APPLIED_AFTER")
        );
    }
}
