//! Resolve result types for the v2 credential flow.
//!
//! These types model every possible outcome of credential resolution:
//! immediate completion, interactive pending states, and polling retries.
//! They also cover refresh, test, and interaction request/response types.

use std::{collections::HashMap, fmt, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{NoPendingState, PendingState};

// ── ResolveResult ──────────────────────────────────────────────────────

/// Outcome of [`Credential::resolve`](crate::Credential::resolve)
/// or [`Interactive::continue_resolve`](crate::Interactive::continue_resolve).
///
/// # Variants
///
/// - **Complete** -- credential ready immediately (API key, basic auth).
/// - **Pending** -- requires user interaction (OAuth2, SAML, device code).
/// - **Retry** -- framework should poll `continue_resolve()` after a delay (device code flow, RFC
///   8628).
pub enum ResolveResult<S, P: PendingState = NoPendingState> {
    /// Credential ready immediately (API key, basic auth, database).
    Complete(S),

    /// Requires user interaction (OAuth2 redirect, SAML, device code, 2FA).
    ///
    /// **Credential returns raw `PendingState`.** Framework handles:
    /// - Encrypting and storing the state in `PendingStateStore`
    /// - Generating a CSPRNG `PendingToken` bound to owner
    /// - Loading and consuming the state before `continue_resolve()`
    ///
    /// Credential author never calls `store_pending()` or
    /// `consume_pending()`.
    Pending {
        /// Raw typed pending state -- framework stores this securely.
        state: P,
        /// What to show/redirect the user.
        interaction: InteractionRequest,
    },

    /// Framework should call `continue_resolve()` again after delay.
    /// Used by device code flow (RFC 8628) polling pattern.
    Retry {
        /// How long to wait before the next poll.
        after: Duration,
    },
}

const REDACTED: &str = "[REDACTED]";

impl<S, P: PendingState> fmt::Debug for ResolveResult<S, P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Complete(_) => formatter.debug_tuple("Complete").field(&REDACTED).finish(),
            Self::Pending { .. } => formatter
                .debug_struct("Pending")
                .field("state", &REDACTED)
                .field("interaction", &REDACTED)
                .finish(),
            Self::Retry { after } => formatter
                .debug_struct("Retry")
                .field("after", after)
                .finish(),
        }
    }
}

/// Convenience alias for non-interactive credentials.
///
/// Avoids writing `ResolveResult<MyState, NoPendingState>` everywhere.
pub type StaticResolveResult<S> = ResolveResult<S, NoPendingState>;

// ── InteractionRequest ─────────────────────────────────────────────────

/// What the UI should show or do after `resolve()` returns `Pending`.
#[derive(Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum InteractionRequest {
    /// Redirect user's browser to this URL (OAuth2 authorization code).
    Redirect {
        /// Authorization URL to redirect to.
        url: String,
    },

    /// Auto-submit a POST form to IdP (SAML POST binding).
    FormPost {
        /// IdP endpoint URL.
        url: String,
        /// Form fields to submit.
        fields: Vec<(String, String)>,
    },

    /// Display information to user (device code, SMS code, TOTP).
    DisplayInfo {
        /// Dialog title.
        title: String,
        /// Instructional message.
        message: String,
        /// Structured display payload.
        data: DisplayData,
        /// Seconds until this information expires.
        expires_in: Option<u64>,
    },
}

impl fmt::Debug for InteractionRequest {
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

/// Structured data to display during an interactive flow.
#[derive(Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DisplayData {
    /// Device code flow: user types this code on another device.
    UserCode {
        /// The user code to enter.
        code: String,
        /// URL where the user enters the code.
        verification_uri: String,
        /// Pre-filled verification URL (optional).
        verification_uri_complete: Option<String>,
    },
    /// Generic text display (instructions, QR codes, etc.).
    Text(String),
}

impl fmt::Debug for DisplayData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserCode { .. } => formatter.debug_tuple("UserCode").field(&REDACTED).finish(),
            Self::Text(_) => formatter.debug_tuple("Text").field(&REDACTED).finish(),
        }
    }
}

// ── UserInput ──────────────────────────────────────────────────────────

/// What the user/callback provides back to `continue_resolve()`.
#[derive(Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum UserInput {
    /// OAuth2 callback: GET with query parameters (code, state).
    Callback {
        /// Query parameters from the callback URL.
        params: HashMap<String, String>,
    },

    /// SAML callback: POST with form data (SAMLResponse, RelayState).
    FormData {
        /// Form fields from the POST body.
        params: HashMap<String, String>,
    },

    /// Device code flow: "check if authorized yet" (framework polls).
    Poll,

    /// User entered a code (SMS, TOTP, 2FA).
    Code {
        /// The code the user entered.
        code: String,
    },
}

impl fmt::Debug for UserInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Callback { params } => formatter
                .debug_struct("Callback")
                .field("param_count", &params.len())
                .finish(),
            Self::FormData { params } => formatter
                .debug_struct("FormData")
                .field("param_count", &params.len())
                .finish(),
            Self::Poll => formatter.write_str("Poll"),
            Self::Code { .. } => formatter.debug_tuple("Code").field(&REDACTED).finish(),
        }
    }
}

// ── RefreshOutcome ─────────────────────────────────────────────────────

/// Represents **successful or expected** outcomes from
/// [`Refreshable::refresh`](crate::Refreshable::refresh).
///
/// Per Tech Spec §15.4 the type-level `Refreshable` membership already
/// encodes "this credential supports refresh," so the post-§15.4 enum
/// only models the success/expected-protocol-outcome dichotomy:
/// [`Refreshed`](Self::Refreshed) on a successful refresh and
/// [`ReauthRequired`](Self::ReauthRequired) when the refresh path fails
/// irrecoverably (refresh token revoked, scope changed). All other
/// failures go through `Err(CredentialError::...)`.
///
/// Per sub-spec §3.6 ([credential-refresh-coordination]) a third
/// successful outcome — [`CoalescedByOtherReplica`](Self::CoalescedByOtherReplica)
/// — surfaces when another replica refreshed the credential while we
/// were waiting on the cross-replica claim. Callers treat it as success
/// and re-read the credential state from the store.
///
/// [credential-refresh-coordination]: https://github.com/nebula-engine/nebula/blob/main/docs/INTEGRATION_MODEL.md (credential refresh)
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RefreshOutcome {
    /// Token was refreshed successfully.
    Refreshed,
    /// Refresh failed due to expected protocol behavior -- needs full
    /// re-authentication. Framework triggers re-resolve with user
    /// interaction.
    ///
    /// Per Tech Spec §15.4 the `NotSupported` variant from the
    /// pre-§15.4 const-bool era was removed: a credential is
    /// [`Refreshable`](crate::Refreshable) by trait membership, so
    /// runtime "not supported" is impossible (compile error at the
    /// dispatch site).
    ///
    /// Carries a typed [`ReauthReason`] so operators (and the credential
    /// engine) can distinguish provider-rejected refresh-token rotation
    /// from sentinel-threshold escalation per sub-spec §3.4.
    ReauthRequired(ReauthReason),
    /// Another replica refreshed the credential while this caller was
    /// waiting on the cross-replica claim. Caller should treat as
    /// success and re-read the credential state from the store. Per
    /// sub-spec §3.6.
    CoalescedByOtherReplica,
}

/// Why a credential transitioned to
/// [`RefreshOutcome::ReauthRequired`].
///
/// Surfaces per sub-spec §3.4 (sentinel threshold) and the existing
/// rotation-failure path (`refresh_token` invalidated by the IdP).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReauthReason {
    /// The IdP rejected the refresh — typically a rotated `refresh_token`
    /// that has been invalidated. The `detail` carries the provider's
    /// human-readable reason for diagnostics.
    ProviderRejected {
        /// Provider-supplied detail (e.g. error_description).
        detail: String,
    },
    /// Sentinel threshold exceeded — the credential keeps crashing
    /// mid-refresh. Per sub-spec §3.4 N=3 events within 1h escalate the
    /// credential to `ReauthRequired`.
    SentinelRepeated {
        /// Number of sentinel events observed within `window_secs`.
        event_count: u32,
        /// Length of the rolling window over which `event_count` was
        /// counted (seconds).
        window_secs: u64,
    },
    /// The credential lacks the refresh material required for a refresh
    /// (e.g., OAuth2 state has no `refresh_token`). Locally detected;
    /// the IdP was never contacted. Operators should re-auth (likely
    /// after fixing scope / `grant_type` configuration).
    ///
    /// Distinct from [`ReauthReason::ProviderRejected`] — that variant
    /// implies the IdP returned an error; this one means we never even
    /// reached the IdP because the local state was unusable for refresh.
    MissingRefreshMaterial {
        /// Human-readable diagnostic (e.g. which field is missing).
        detail: String,
    },
}

// ── TestResult ─────────────────────────────────────────────────────────

/// Stable, secret-free classification for a provider-side credential-test
/// rejection.
///
/// Provider response text is untrusted and may echo tokens, authorization
/// headers, account identifiers, or request bodies. Implementations must map
/// it to this payload-free, extensible vocabulary locally and discard the raw
/// text before returning from [`Testable::test`](crate::Testable::test).
///
/// This represents a definitive negative probe outcome
/// (`Ok(TestResult::Failed { .. })`). It deliberately does not reuse
/// [`ProviderErrorKind`](crate::ProviderErrorKind), which classifies failures
/// that prevented the operation from determining validity (including their
/// retryability) and therefore travel through `Err`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TestFailureCode {
    /// The provider rejected the presented authentication material.
    AuthenticationRejected,
    /// Authentication succeeded but the principal lacks required permission.
    PermissionDenied,
    /// The provider account is disabled, locked, suspended, or restricted.
    AccountRestricted,
    /// The credential or its provider-specific setup is invalid.
    InvalidConfiguration,
    /// A classified rejection that does not fit another stable category.
    Other,
}

/// Outcome of [`Testable::test`](crate::Testable::test).
///
/// Per Tech Spec §15.4 the type-level `Testable` membership already
/// encodes "this credential supports testing," so the post-§15.4
/// signature is `Result<TestResult, CredentialError>` without an
/// `Option` carve-out for "not testable" — non-testable credentials
/// simply do not implement [`Testable`](crate::Testable).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TestResult {
    /// Credential works -- authenticated successfully.
    Success,
    /// Credential was rejected with a stable, secret-free classification.
    Failed {
        /// Payload-free failure category. Raw provider text never crosses this
        /// seam.
        code: TestFailureCode,
    },
}

impl TestResult {
    /// Whether the provider accepted the credential.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    /// The stable rejection code, or `None` for [`Success`](Self::Success).
    #[must_use]
    pub const fn failure_code(&self) -> Option<TestFailureCode> {
        match self {
            Self::Success => None,
            Self::Failed { code } => Some(*code),
        }
    }
}

// ── RefreshPolicy ──────────────────────────────────────────────────────

/// Controls when and how the framework refreshes this credential.
///
/// Used as an associated const on the
/// [`Refreshable`](crate::Refreshable) sub-trait:
/// [`Refreshable::REFRESH_POLICY`](crate::Refreshable::REFRESH_POLICY).
/// Non-refreshable credentials do not declare a policy because they do
/// not implement [`Refreshable`](crate::Refreshable).
///
/// All fields are const-compatible (`Duration::from_secs` is `const fn`
/// since Rust 1.53).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefreshPolicy {
    /// Refresh this long before `expires_at()`. Default: 5 minutes.
    pub early_refresh: Duration,
    /// Minimum backoff between retry attempts on refresh failure.
    /// Default: 5 seconds.
    pub min_retry_backoff: Duration,
    /// Add random jitter `(0..jitter)` to `early_refresh` to prevent
    /// thundering herd. Default: 30 seconds.
    pub jitter: Duration,
}

impl RefreshPolicy {
    /// Sensible defaults for most credential types.
    pub const DEFAULT: Self = Self {
        early_refresh: Duration::from_mins(5),
        min_retry_backoff: Duration::from_secs(5),
        jitter: Duration::from_secs(30),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::{Zeroize, ZeroizeOnDrop};

    const SECRET_CANARY: &str = "credential-contract-secret-NEVER-DEBUG-7c2e";

    #[derive(Debug, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    struct SecretPending {
        secret: String,
    }

    impl PendingState for SecretPending {
        const KIND: &'static str = "secret_debug_probe";

        fn expires_in(&self) -> Duration {
            Duration::from_mins(1)
        }
    }

    #[test]
    fn resolve_result_debug_redacts_complete_and_pending_state() {
        let complete = ResolveResult::<String>::Complete(SECRET_CANARY.to_owned());
        let pending = ResolveResult::<String, SecretPending>::Pending {
            state: SecretPending {
                secret: SECRET_CANARY.to_owned(),
            },
            interaction: InteractionRequest::Redirect {
                url: format!("https://provider.example/?state={SECRET_CANARY}"),
            },
        };

        for debug in [format!("{complete:?}"), format!("{pending:?}")] {
            assert!(
                !debug.contains(SECRET_CANARY),
                "resolve result Debug must not expose state or interaction payload: {debug}"
            );
        }
    }

    #[test]
    fn interaction_request_debug_redacts_every_payload_shape() {
        let interactions = [
            InteractionRequest::Redirect {
                url: format!("https://provider.example/?state={SECRET_CANARY}"),
            },
            InteractionRequest::FormPost {
                url: format!("https://provider.example/?state={SECRET_CANARY}"),
                fields: vec![(SECRET_CANARY.to_owned(), SECRET_CANARY.to_owned())],
            },
            InteractionRequest::DisplayInfo {
                title: SECRET_CANARY.to_owned(),
                message: SECRET_CANARY.to_owned(),
                data: DisplayData::Text(SECRET_CANARY.to_owned()),
                expires_in: Some(60),
            },
        ];

        for interaction in interactions {
            let debug = format!("{interaction:?}");
            assert!(
                !debug.contains(SECRET_CANARY),
                "interaction Debug must not expose its payload: {debug}"
            );
        }
    }

    #[test]
    fn display_data_debug_redacts_device_code_and_text() {
        let values = [
            DisplayData::UserCode {
                code: SECRET_CANARY.to_owned(),
                verification_uri: format!("https://provider.example/{SECRET_CANARY}"),
                verification_uri_complete: Some(format!(
                    "https://provider.example/?code={SECRET_CANARY}"
                )),
            },
            DisplayData::Text(SECRET_CANARY.to_owned()),
        ];

        for value in values {
            let debug = format!("{value:?}");
            assert!(
                !debug.contains(SECRET_CANARY),
                "display data Debug must not expose user-facing codes or text: {debug}"
            );
        }
    }

    #[test]
    fn user_input_debug_redacts_callback_form_and_code_values() {
        let inputs = [
            UserInput::Callback {
                params: HashMap::from([(SECRET_CANARY.to_owned(), SECRET_CANARY.to_owned())]),
            },
            UserInput::FormData {
                params: HashMap::from([(SECRET_CANARY.to_owned(), SECRET_CANARY.to_owned())]),
            },
            UserInput::Code {
                code: SECRET_CANARY.to_owned(),
            },
        ];

        for input in inputs {
            let debug = format!("{input:?}");
            assert!(
                !debug.contains(SECRET_CANARY),
                "user input Debug must not expose callback or code values: {debug}"
            );
        }
    }

    #[test]
    fn test_result_accessors_and_debug_expose_only_payload_free_codes() {
        let success = TestResult::Success;
        assert!(success.is_success());
        assert_eq!(success.failure_code(), None);
        assert_eq!(format!("{success:?}"), "Success");

        for code in [
            TestFailureCode::AuthenticationRejected,
            TestFailureCode::PermissionDenied,
            TestFailureCode::AccountRestricted,
            TestFailureCode::InvalidConfiguration,
            TestFailureCode::Other,
        ] {
            let result = TestResult::Failed { code };
            assert!(!result.is_success());
            assert_eq!(result.failure_code(), Some(code));
            let debug = format!("{result:?}");
            assert!(debug.contains(&format!("{code:?}")));
            assert!(!debug.contains(SECRET_CANARY));
        }
    }
}
