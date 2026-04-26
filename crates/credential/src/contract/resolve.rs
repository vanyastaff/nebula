//! Resolve result types for the v2 credential flow.
//!
//! These types model every possible outcome of credential resolution:
//! immediate completion, interactive pending states, and polling retries.
//! They also cover refresh, test, and interaction request/response types.

use std::{collections::HashMap, time::Duration};

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
#[derive(Debug)]
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

/// Convenience alias for non-interactive credentials.
///
/// Avoids writing `ResolveResult<MyState, NoPendingState>` everywhere.
pub type StaticResolveResult<S> = ResolveResult<S, NoPendingState>;

// ── InteractionRequest ─────────────────────────────────────────────────

/// What the UI should show or do after `resolve()` returns `Pending`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Structured data to display during an interactive flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

// ── UserInput ──────────────────────────────────────────────────────────

/// What the user/callback provides back to `continue_resolve()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    ReauthRequired,
}

// ── TestResult ─────────────────────────────────────────────────────────

/// Outcome of
/// [`Testable::test`](crate::Testable::test).
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
    /// Credential failed -- with reason.
    Failed {
        /// Human-readable failure reason.
        reason: String,
    },
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
