//! Resolve result types for the v2 credential flow.
//!
//! These types model every possible outcome of credential resolution:
//! immediate completion, interactive pending states, and polling retries.
//! They also cover refresh, test, and interaction request/response types.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::pending::{NoPendingState, PendingState};

// ── ResolveResult ──────────────────────────────────────────────────────

/// Outcome of [`Credential::resolve`](crate::credential::Credential::resolve)
/// or [`Credential::continue_resolve`](crate::credential::Credential::continue_resolve).
///
/// # Variants
///
/// - **Complete** -- credential ready immediately (API key, basic auth).
/// - **Pending** -- requires user interaction (OAuth2, SAML, device code).
/// - **Retry** -- framework should poll `continue_resolve()` after a delay
///   (device code flow, RFC 8628).
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
/// [`Credential::refresh`](crate::credential::Credential::refresh).
///
/// All **failures** go through
/// `Err(CredentialError::...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshOutcome {
    /// Token was refreshed successfully.
    Refreshed,
    /// Credential doesn't support refresh (permanent tokens, API keys).
    NotSupported,
    /// Refresh failed due to expected protocol behavior -- needs full
    /// re-authentication. Framework triggers re-resolve with user
    /// interaction.
    ReauthRequired,
}

// ── TestResult ─────────────────────────────────────────────────────────

/// Outcome of
/// [`Credential::test`](crate::credential::Credential::test).
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
    /// Credential type doesn't support testing.
    Untestable,
}

// ── RefreshPolicy ──────────────────────────────────────────────────────

/// Controls when and how the framework refreshes this credential.
///
/// Used as an associated const on the
/// [`Credential`](crate::credential::Credential) trait:
/// `const REFRESH_POLICY: RefreshPolicy`.
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
        early_refresh: Duration::from_secs(300),
        min_retry_backoff: Duration::from_secs(5),
        jitter: Duration::from_secs(30),
    };
}
