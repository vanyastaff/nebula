//! Error types for credential operations
//!
//! This module defines a layered error hierarchy:
//! - [`CredentialError`]: Top-level error wrapping Crypto/Validation
//! - [`CryptoError`]: Encryption, decryption, key derivation
//! - [`ValidationError`]: Invalid credential IDs, malformed data
//!
//! Persistence failures belong to the technical `nebula-storage-port`
//! contract. They are deliberately not re-exported through the
//! `nebula-credential` product surface.
//!
//! # Error Conversion Examples
//!
//! [`CryptoError`] and [`ValidationError`] convert to [`CredentialError`] via
//! `From` implementations:
//!
//! ```
//! use nebula_credential::error::{CredentialError, ValidationError};
//!
//! // Validation errors convert automatically
//! let val_err = ValidationError::InvalidCredentialId {
//!     id: "bad id".to_string(),
//!     reason: "contains spaces".to_string(),
//! };
//! let cred_err: CredentialError = val_err.into();
//! assert!(cred_err.to_string().contains("bad id"));
//! ```
//!

use compact_str::CompactString;
use thiserror::Error;

// ── Secret-free message wrapper ─────────────────────────────────────────────

/// A message that has been hand-validated as not containing raw secret
/// material. Constructor pattern-checks for known secret-like substrings
/// in debug builds.
#[derive(Debug, Clone)]
pub struct SecretFreeMessage(CompactString);

impl SecretFreeMessage {
    /// Construct from a value the caller asserts is secret-free. In
    /// debug builds, `debug_assert!` fires on substrings that look like
    /// tokens / base64 blobs / long hex.
    pub fn new(s: impl Into<CompactString>) -> Self {
        let v = s.into();
        // Avoid interpolating `v` into the assertion message — if the
        // candidate IS a secret, this would echo it into panic output /
        // test logs. Length is the only safe metadatum to surface.
        debug_assert!(
            !looks_like_secret(&v),
            "SecretFreeMessage given likely secret content (len={})",
            v.len()
        );
        Self(v)
    }

    /// The message as a str slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Display for SecretFreeMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Conservative heuristic for "this looks like a secret token / base64
/// blob / long hex string". Used in debug_assert. False positives are
/// acceptable — the intent is to catch accidental injection.
fn looks_like_secret(s: &str) -> bool {
    let len = s.len();
    if len >= 32
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '=' || c == '+' || c == '/'
        })
    {
        return true;
    }
    false
}

// ── Scheme classification ────────────────────────────────────────────────────

/// Scheme mismatch between what a consumer expects and what is actually
/// present, identified by auth-scheme **name**.
///
/// The materialization boundary
/// ([`CredentialSnapshot::into_project`](crate::snapshot::CredentialSnapshot::into_project))
/// knows schemes only by their pattern name, so both sides are carried as
/// names. The canonical scheme taxonomy is
/// [`nebula_core::auth::AuthPattern`](crate::AuthPattern); this type deliberately
/// does **not** duplicate it — the earlier `SchemeKind` enum mirrored
/// `AuthPattern` variant-for-variant yet was never populated on the live path.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SchemeMismatch {
    expected: CompactString,
    actual: CompactString,
}

impl SchemeMismatch {
    /// Construct from auth-scheme pattern names — the names the snapshot layer
    /// carries, for first-party and plugin schemes alike.
    pub fn by_name(expected: impl Into<CompactString>, actual: impl Into<CompactString>) -> Self {
        Self {
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// The scheme the consumer expected.
    #[must_use]
    pub fn expected(&self) -> &str {
        self.expected.as_str()
    }

    /// The scheme that was actually present.
    #[must_use]
    pub fn actual(&self) -> &str {
        self.actual.as_str()
    }
}

impl std::fmt::Display for SchemeMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "expected {}, got {}", self.expected, self.actual)
    }
}

// ── Provider error ───────────────────────────────────────────────────────────

/// Discriminated kind for a provider-level credential error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderErrorKind {
    /// Network-level failure (TCP, DNS).
    Network,
    /// Authentication failure at the provider.
    Auth,
    /// Provider rate-limiting.
    RateLimit,
    /// OAuth2 `invalid_grant` or equivalent.
    InvalidGrant,
    /// Internal server error at the provider.
    ServerError,
    /// Schema / response parsing error.
    Schema,
    /// Catch-all for provider-specific error codes.
    Other,
}

/// Context struct for [`CredentialError::Provider`].
///
/// Each field is accessible only via the provided accessor methods — the
/// struct is `#[non_exhaustive]` so future fields do not break callers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProviderErrorContext {
    kind: ProviderErrorKind,
    message: SecretFreeMessage,
    provider_code: Option<CompactString>,
}

impl ProviderErrorContext {
    /// Construct with kind and a secret-free message.
    pub fn new(kind: ProviderErrorKind, message: SecretFreeMessage) -> Self {
        Self {
            kind,
            message,
            provider_code: None,
        }
    }

    /// Attach an optional provider-specific error code string.
    pub fn with_code(mut self, code: impl Into<CompactString>) -> Self {
        self.provider_code = Some(code.into());
        self
    }

    /// The kind of provider error.
    pub fn kind(&self) -> ProviderErrorKind {
        self.kind
    }

    /// The secret-free human-readable message.
    pub fn message(&self) -> &SecretFreeMessage {
        &self.message
    }

    /// An optional provider-specific error code.
    pub fn provider_code(&self) -> Option<&str> {
        self.provider_code.as_deref()
    }
}

impl std::fmt::Display for ProviderErrorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

// ── Refresh failure ──────────────────────────────────────────────────────────

/// Diagnostic class of a replay-safe refresh failure.
///
/// This classification is orthogonal to [`RefreshNotAppliedPhase`]. It may
/// describe a failure observed either before dispatch or in a completed
/// provider response; consumers must use the phase, never the kind, as the
/// proof of whether provider dispatch occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshErrorKind {
    /// A transient network or transport condition.
    TransientNetwork,
    /// The provider was temporarily unavailable.
    ProviderUnavailable,
    /// A request, response, or framework contract was rejected at the protocol
    /// layer.
    ProtocolError,
}

/// Validated non-zero delay before a replay-safe refresh retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RetryDelay(std::time::Duration);

impl RetryDelay {
    /// Largest retry delay accepted by credential, persistence, and HTTP
    /// boundaries (365 days).
    pub const MAX_SECS: u64 = 31_536_000;

    /// Validate a non-zero retry delay.
    ///
    /// # Errors
    ///
    /// Returns [`RetryDelayError::Zero`] for `Duration::ZERO`; immediate
    /// retries are deliberately absent from the refresh contract.
    pub fn new(duration: std::time::Duration) -> std::result::Result<Self, RetryDelayError> {
        if duration.is_zero() {
            return Err(RetryDelayError::Zero);
        }
        let seconds = duration
            .as_secs()
            .saturating_add(u64::from(duration.subsec_nanos() != 0));
        if seconds > Self::MAX_SECS {
            return Err(RetryDelayError::TooLong);
        }
        Ok(Self(std::time::Duration::from_secs(seconds)))
    }

    /// Borrow the validated duration value.
    #[must_use]
    pub const fn get(self) -> std::time::Duration {
        self.0
    }
}

impl TryFrom<std::time::Duration> for RetryDelay {
    type Error = RetryDelayError;

    fn try_from(duration: std::time::Duration) -> std::result::Result<Self, Self::Error> {
        Self::new(duration)
    }
}

/// Invalid retry delay.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetryDelayError {
    /// Immediate retries would permit an unbounded hot loop.
    #[error("credential refresh retry delay must be non-zero")]
    Zero,
    /// Delays beyond the shared persistence/API bound are rejected.
    #[error("credential refresh retry delay exceeds 365 days")]
    TooLong,
}

/// Retry guidance from credential to framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetryAdvice {
    /// Do not automatically retry the current credential material epoch.
    ///
    /// When persisted as a durable retry gate, this advice is scoped to the
    /// current material epoch. An explicit authority transition may replace
    /// that material and clear the gate; this is not a permanent ban on the
    /// credential identity.
    Never,
    /// Retry after a validated non-zero duration.
    After(RetryDelay),
}

/// Validated low-cardinality diagnostic code for an exact refresh refusal.
///
/// Values are limited to 64 ASCII alphanumeric/`_`/`-`/`.`/`:` bytes. The
/// value is never rendered by `Debug` or `Display`; explicit access through
/// [`as_str`](Self::as_str) is required.
///
/// Codes must come from a fixed vocabulary owned by the integration. Provider
/// data may select a predeclared code only after mapping through a closed
/// integration enum. Never pass provider descriptions, extension codes,
/// secrets, tenant data, or other free-form input here. Shape validation is a
/// defense in depth; it is not sanitization.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RefreshDiagnosticCode(CompactString);

impl RefreshDiagnosticCode {
    /// Maximum encoded diagnostic-code length.
    pub const MAX_LEN: usize = 64;

    /// Parse and validate a fixed, integration-authored diagnostic code.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, non-ASCII, or free-form values.
    ///
    /// This validates shape only. Callers must first map any provider value
    /// onto their own closed, low-cardinality vocabulary.
    pub fn parse(value: impl AsRef<str>) -> std::result::Result<Self, RefreshDiagnosticCodeError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(RefreshDiagnosticCodeError::Empty);
        }
        if value.len() > Self::MAX_LEN {
            return Err(RefreshDiagnosticCodeError::TooLong);
        }
        let valid = value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'));
        if !valid {
            return Err(RefreshDiagnosticCodeError::InvalidCharacter);
        }
        Ok(Self(CompactString::new(value)))
    }

    /// Explicitly expose the validated diagnostic code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Debug for RefreshDiagnosticCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RefreshDiagnosticCode([REDACTED])")
    }
}

impl std::fmt::Display for RefreshDiagnosticCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

/// Invalid [`RefreshDiagnosticCode`].
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshDiagnosticCodeError {
    /// Codes are required when the optional field is present.
    #[error("credential refresh diagnostic code cannot be empty")]
    Empty,
    /// Codes must remain low-cardinality and bounded.
    #[error("credential refresh diagnostic code exceeds 64 bytes")]
    TooLong,
    /// Free-form or non-ASCII values are forbidden.
    #[error("credential refresh diagnostic code contains an invalid character")]
    InvalidCharacter,
}

/// Replay-safe failure detail supplied before a linear proof is consumed.
#[derive(Clone)]
pub struct RefreshFailureSpec {
    kind: RefreshErrorKind,
    retry: RetryAdvice,
    diagnostic_code: Option<RefreshDiagnosticCode>,
}

impl RefreshFailureSpec {
    /// Construct typed failure detail. This value is not proof by itself; only
    /// a [`crate::RefreshAttempt`] or [`crate::CompletedResponseProof`] can
    /// turn it into a refresh report.
    #[must_use]
    pub const fn new(kind: RefreshErrorKind, retry: RetryAdvice) -> Self {
        Self {
            kind,
            retry,
            diagnostic_code: None,
        }
    }

    /// Attach an already-validated low-cardinality diagnostic code.
    #[must_use = "builder methods must be chained"]
    pub fn with_diagnostic_code(mut self, code: RefreshDiagnosticCode) -> Self {
        self.diagnostic_code = Some(code);
        self
    }

    /// Failure category.
    #[must_use]
    pub const fn kind(&self) -> RefreshErrorKind {
        self.kind
    }

    /// Retry guidance.
    #[must_use]
    pub const fn retry(&self) -> RetryAdvice {
        self.retry
    }

    /// Optional validated code, exposed only by explicit accessor.
    #[must_use]
    pub fn diagnostic_code(&self) -> Option<&RefreshDiagnosticCode> {
        self.diagnostic_code.as_ref()
    }
}

impl std::fmt::Debug for RefreshFailureSpec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RefreshFailureSpec")
            .field("kind", &self.kind)
            .field("retry", &self.retry)
            .field("diagnostic_code_present", &self.diagnostic_code.is_some())
            .finish()
    }
}

/// Sole dispatch-proof phase for [`CredentialError::RefreshNotApplied`].
///
/// Failure kind is diagnostic only. Retry and dispatch decisions must derive
/// from this linear-evidence phase plus [`RetryAdvice`], never by inferring a
/// phase from [`RefreshErrorKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshNotAppliedPhase {
    /// Request construction failed before transport dispatch.
    BeforeDispatch,
    /// A complete provider response proved that the operation had no effect.
    ProviderConfirmedNotApplied,
}

/// Proof-bearing context for [`CredentialError::RefreshNotApplied`].
///
/// Each field is accessible only via the provided accessor methods — the
/// constructor is crate-private and is reachable only through the linear
/// refresh witness.
#[non_exhaustive]
pub struct RefreshNotAppliedContext {
    phase: RefreshNotAppliedPhase,
    kind: RefreshErrorKind,
    retry: RetryAdvice,
    diagnostic_code: Option<RefreshDiagnosticCode>,
}

impl RefreshNotAppliedContext {
    pub(crate) fn from_spec(phase: RefreshNotAppliedPhase, spec: RefreshFailureSpec) -> Self {
        Self {
            phase,
            kind: spec.kind,
            retry: spec.retry,
            diagnostic_code: spec.diagnostic_code,
        }
    }

    /// Proof phase that authorized this exact failure.
    #[must_use]
    pub const fn phase(&self) -> RefreshNotAppliedPhase {
        self.phase
    }

    /// The kind of refresh failure.
    #[must_use]
    pub const fn kind(&self) -> RefreshErrorKind {
        self.kind
    }

    /// Retry guidance for the framework.
    #[must_use]
    pub const fn retry(&self) -> RetryAdvice {
        self.retry
    }

    /// Optional validated diagnostic code.
    #[must_use]
    pub fn diagnostic_code(&self) -> Option<&RefreshDiagnosticCode> {
        self.diagnostic_code.as_ref()
    }
}

impl std::fmt::Debug for RefreshNotAppliedContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RefreshNotAppliedContext")
            .field("phase", &self.phase)
            .field("kind", &self.kind)
            .field("retry", &self.retry)
            .field("diagnostic_code_present", &self.diagnostic_code.is_some())
            .finish()
    }
}

impl std::fmt::Display for RefreshNotAppliedContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?}/{:?} (retry: {:?}; diagnostic code redacted)",
            self.phase, self.kind, self.retry
        )
    }
}

// ── Revoke failure ───────────────────────────────────────────────────────────

// ── Access errors ─────────────────────────────────────────────────────────────

/// Error type for credential access operations.
///
/// Returned by [`CredentialAccessor`](crate::CredentialAccessor) trait methods.
/// Each variant represents a distinct failure mode during credential access.
///
/// # Examples
///
/// ```
/// use nebula_credential::CredentialAccessError;
///
/// let err = CredentialAccessError::NotFound("api_key".to_owned());
/// assert!(err.to_string().contains("api_key"));
/// ```
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialAccessError {
    /// Credential not found.
    #[error("credential not found: {0}")]
    NotFound(String),

    /// Credential type mismatch (scheme projection failed).
    #[error("credential type mismatch: {0}")]
    TypeMismatch(String),

    /// Access to undeclared credential type (capability violation).
    #[error("credential access denied: {capability} for action `{action_id}`")]
    AccessDenied {
        /// The capability that was denied.
        capability: String,
        /// The action that requested the capability.
        action_id: String,
    },

    /// Accessor not configured.
    #[error("credential accessor not configured: {0}")]
    NotConfigured(String),
}

// ── Top-level error ──────────────────────────────────────────────────────────

/// Top-level credential error.
///
/// Each structural variant wraps a `Box<...Context>` so the enum stays
/// pointer-sized (≤ 32 bytes). The 32-byte hard cap is enforced by the
/// `const_assert!` at the bottom of this file (closes #588).
///
/// # Variant shapes
///
/// - `Crypto` / `Validation` — transparent wrappers around typed sub-errors.
/// - `Provider(Box<ProviderErrorContext>)` — boxed context; use
///   [`ProviderErrorContext::new`] + accessors.
/// - `RefreshNotApplied(Box<RefreshNotAppliedContext>)` — proof-bearing
///   replay-safe refresh failure.
/// - `SchemeMismatch(Box<SchemeMismatch>)` — boxed; carries two scheme-name strings.
/// - `NotInteractive` — unit variant.
/// - `OutcomeUnknown` — unit variant; a provider side effect or durable
///   mutation may have completed without exact acknowledgement, so callers
///   must reconcile instead of replaying it blindly.
/// - `RefreshFinalization` — unit variant; the refresh outcome is known, but
///   its required retry-gate or reauthentication transition is not durable.
/// - `PostProviderPersistence` — unit variant; a provider operation has already
///   completed, so replaying it is unsafe even when the following persistence
///   failure was definite.
/// - `InvalidInput(String)` — 24-byte string payload (ptr+len+cap); fits.
/// - `Crypto(Box<CryptoError>)` — boxed so the largest CryptoError variant
///   does not push the enum past 32 bytes.
/// - `Validation(Box<ValidationError>)` — boxed for the same reason
///   (`InvalidCredentialId` has two String fields).
/// - `Resolution(Box<nebula_core::CoreError>)` — boxed; CoreError size is
///   upstream-controlled.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CredentialError {
    /// Cryptographic error.
    #[error("{0}")]
    Crypto(#[source] Box<CryptoError>),

    /// Validation error.
    #[error("{0}")]
    Validation(#[source] Box<ValidationError>),

    /// Provider-specific error from a credential implementation.
    #[error("provider error: {0}")]
    Provider(Box<ProviderErrorContext>),

    /// Refresh was proven not to have changed provider state.
    #[error("credential refresh was not applied: {0}")]
    RefreshNotApplied(Box<RefreshNotAppliedContext>),

    /// Operation requires an interactive credential, but this credential
    /// is non-interactive.
    #[error("credential does not support interactive flows")]
    NotInteractive,

    /// A provider side effect or durable credential mutation may have
    /// completed without exact acknowledgement. This is deliberately
    /// non-retryable because replaying either operation can duplicate work or
    /// race the committed state.
    #[error("credential mutation outcome is unknown; reconcile before retrying")]
    OutcomeUnknown,

    /// The refresh outcome is known, but the framework definitely could not
    /// finalize the corresponding durable retry gate or reauthentication
    /// decision.
    ///
    /// The refresh claim remains retained so another replica cannot
    /// immediately replay the provider request. Callers must reconcile the
    /// credential aggregate or reconnect the integration.
    #[error("credential refresh decision could not be finalized; reconcile before retrying")]
    RefreshFinalization,

    /// A credential provider operation completed, but preparing or committing
    /// its following durable credential update definitely failed.
    ///
    /// This is deliberately distinct from a pre-provider storage outage. The
    /// provider may already have rotated or invalidated the old grant, so a
    /// generic retry loop must not issue the provider operation again.
    #[error(
        "provider operation succeeded but durable credential finalization failed; reconcile before retrying"
    )]
    PostProviderPersistence,

    /// Scheme type mismatch between credential and resource. Boxed because
    /// the inner `SchemeMismatch` carries two [`CompactString`] scheme names —
    /// keeping it inline would push the enum past the 32-byte cap.
    #[error("scheme mismatch: {0}")]
    SchemeMismatch(Box<SchemeMismatch>),

    /// Invalid input from user (parameter values).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Resolution failed — wraps a [`CoreError`](nebula_core::CoreError) from
    /// the [`CredentialAccessor`](nebula_core::accessor::CredentialAccessor).
    #[error("credential resolution failed: {0}")]
    Resolution(Box<nebula_core::CoreError>),
}

impl From<CryptoError> for CredentialError {
    fn from(e: CryptoError) -> Self {
        Self::Crypto(Box::new(e))
    }
}

impl From<ValidationError> for CredentialError {
    fn from(e: ValidationError) -> Self {
        Self::Validation(Box::new(e))
    }
}

impl From<nebula_core::CoreError> for CredentialError {
    fn from(e: nebula_core::CoreError) -> Self {
        Self::Resolution(Box::new(e))
    }
}

impl nebula_error::Classify for CredentialError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::Crypto(s) => nebula_error::Classify::category(s.as_ref()),
            Self::Validation(s) => nebula_error::Classify::category(s.as_ref()),
            Self::NotInteractive => nebula_error::ErrorCategory::Unsupported,
            Self::OutcomeUnknown => nebula_error::ErrorCategory::Internal,
            Self::RefreshFinalization => nebula_error::ErrorCategory::Internal,
            Self::PostProviderPersistence => nebula_error::ErrorCategory::Internal,
            Self::Provider(_) => nebula_error::ErrorCategory::External,
            Self::RefreshNotApplied(_) => nebula_error::ErrorCategory::External,
            Self::SchemeMismatch(_) => nebula_error::ErrorCategory::Validation,
            Self::InvalidInput(_) => nebula_error::ErrorCategory::Validation,
            Self::Resolution(s) => nebula_error::Classify::category(s.as_ref()),
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::Crypto(s) => nebula_error::Classify::code(s.as_ref()),
            Self::Validation(s) => nebula_error::Classify::code(s.as_ref()),
            Self::NotInteractive => nebula_error::ErrorCode::new("CREDENTIAL:NOT_INTERACTIVE"),
            Self::OutcomeUnknown => nebula_error::ErrorCode::new("CREDENTIAL:OUTCOME_UNKNOWN"),
            Self::RefreshFinalization => {
                nebula_error::ErrorCode::new("CREDENTIAL:REFRESH_FINALIZATION")
            },
            Self::PostProviderPersistence => {
                nebula_error::ErrorCode::new("CREDENTIAL:POST_PROVIDER_PERSISTENCE")
            },
            Self::Provider(_) => nebula_error::ErrorCode::new("CREDENTIAL:PROVIDER"),
            Self::RefreshNotApplied(_) => {
                nebula_error::ErrorCode::new("CREDENTIAL:REFRESH_NOT_APPLIED")
            },
            Self::SchemeMismatch(_) => nebula_error::ErrorCode::new("CREDENTIAL:SCHEME_MISMATCH"),
            Self::InvalidInput(_) => nebula_error::ErrorCode::new("CREDENTIAL:INVALID_INPUT"),
            Self::Resolution(_) => nebula_error::ErrorCode::new("CREDENTIAL:RESOLUTION_FAILED"),
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            // Positive matching is deliberate: adding a future advice variant
            // must fail closed until its replay semantics are reviewed.
            Self::RefreshNotApplied(ctx) => matches!(ctx.retry(), RetryAdvice::After(_)),
            Self::Provider(ctx) => matches!(
                ctx.kind(),
                ProviderErrorKind::Network
                    | ProviderErrorKind::RateLimit
                    | ProviderErrorKind::ServerError
            ),
            _ => false,
        }
    }

    fn retry_hint(&self) -> Option<nebula_error::RetryHint> {
        match self {
            Self::RefreshNotApplied(ctx) => match ctx.retry() {
                RetryAdvice::Never => None,
                RetryAdvice::After(delay) => Some(nebula_error::RetryHint::after(delay.get())),
            },
            _ => None,
        }
    }
}

// ── Cryptographic errors ─────────────────────────────────────────────────────

// `CryptoError` moved to `nebula-crypto` (ADR-0088). Re-exported here because it
// is part of `CredentialError` (the `Crypto` variant + `From<CryptoError>`), so
// `nebula_credential::CryptoError` remains a stable path.
pub use nebula_crypto::CryptoError;

// ── Validation errors ────────────────────────────────────────────────────────

/// Validation errors
///
/// Errors from input validation including invalid credential IDs
/// and malformed credential data.
#[derive(Debug, Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum ValidationError {
    /// Credential ID cannot be empty
    #[classify(category = "validation", code = "CREDENTIAL:EMPTY_ID")]
    #[error("Credential ID cannot be empty")]
    EmptyCredentialId,

    /// Invalid credential ID
    #[classify(category = "validation", code = "CREDENTIAL:INVALID_ID")]
    #[error("Invalid credential ID '{id}': {reason}")]
    InvalidCredentialId {
        /// The invalid ID
        id: String,
        /// Reason for invalidity
        reason: String,
    },

    /// Invalid credential format
    #[classify(category = "validation", code = "CREDENTIAL:INVALID_FORMAT")]
    #[error("Invalid credential format: {0}")]
    InvalidFormat(String),
}

// ── Resolution stage ─────────────────────────────────────────────────────────

/// Where in the resolution pipeline an error occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolutionStage {
    /// Loading state from store.
    LoadState,
    /// Decrypting stored data.
    Decrypt,
    /// Deserializing state bytes.
    DeserializeState,
    /// Projecting scheme from state.
    ProjectScheme,
    /// Coercing scheme to resource Auth type.
    CoerceToResourceAuth,
    /// Refreshing expired credentials.
    Refresh,
}

/// Result type alias for credential operations
pub type Result<T> = std::result::Result<T, CredentialError>;

// ── Size cap (closes #588) ───────────────────────────────────────────────────

// All variants carry at most one Box pointer (8 bytes) or a small inline
// payload. Tag discriminant is folded into niche/alignment padding.
//
// Breakdown (64-bit):
//   Crypto(Box<CryptoError>)         — 8B pointer
//   Validation(Box<ValidationError>) — 8B pointer
//   Provider(Box<ProviderErrorContext>) — 8B pointer
//   RefreshNotApplied(Box<RefreshNotAppliedContext>) — 8B pointer
//   NotInteractive                   — 0B payload
//   SchemeMismatch(Box<SchemeMismatch>) — 8B pointer
//      (boxed: `SchemeMismatch` carries two `CompactString` scheme names,
//      so the inline form would push the enum past 32B).
//   InvalidInput(String)             — 24B (ptr+len+cap)
//   Resolution(Box<CoreError>)       — 8B pointer
//
// Largest payload is `InvalidInput(String)` = 24B; with discriminant ≤ 32B.
// The assert is the enforcement — if it fires, box the fat variant.
// `size_of` is in the Rust 2024 prelude (RFC 3458, stable since 1.80) so
// qualifying it triggers `-W unused-qualifications`. Keep unqualified.
static_assertions::const_assert!(size_of::<CredentialError>() <= 32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crypto_error_decryption_failed() {
        let err = CryptoError::DecryptionFailed;
        assert_eq!(
            err.to_string(),
            "Decryption failed - invalid key or corrupted data"
        );
    }

    #[test]
    fn test_crypto_error_key_derivation() {
        let err = CryptoError::KeyDerivation("invalid salt".to_string());
        assert!(err.to_string().contains("Key derivation failed"));
        assert!(err.to_string().contains("invalid salt"));
    }

    #[test]
    fn test_validation_error_empty_id() {
        let err = ValidationError::EmptyCredentialId;
        assert_eq!(err.to_string(), "Credential ID cannot be empty");
    }

    #[test]
    fn test_validation_error_invalid_id() {
        let err = ValidationError::InvalidCredentialId {
            id: "../etc/passwd".to_string(),
            reason: "contains path traversal characters".to_string(),
        };
        assert!(err.to_string().contains("../etc/passwd"));
        assert!(err.to_string().contains("path traversal"));
    }

    #[test]
    fn test_credential_error_from_crypto() {
        let crypto_err = CryptoError::DecryptionFailed;
        let cred_err: CredentialError = crypto_err.into();
        assert!(matches!(cred_err, CredentialError::Crypto(_)));
        assert!(cred_err.to_string().contains("Decryption failed"));
    }

    #[test]
    fn test_credential_error_from_validation() {
        let val_err = ValidationError::EmptyCredentialId;
        let cred_err: CredentialError = val_err.into();
        assert!(matches!(cred_err, CredentialError::Validation(_)));
        assert!(cred_err.to_string().contains("empty"));
    }

    #[test]
    fn refresh_error_context() {
        let context = RefreshNotAppliedContext::from_spec(
            RefreshNotAppliedPhase::BeforeDispatch,
            RefreshFailureSpec::new(RefreshErrorKind::ProtocolError, RetryAdvice::Never),
        );
        let err = CredentialError::RefreshNotApplied(Box::new(context));
        assert!(matches!(
            &err,
            CredentialError::RefreshNotApplied(ctx)
                if ctx.kind() == RefreshErrorKind::ProtocolError
        ));
        assert!(err.to_string().contains("not applied"));
    }

    #[test]
    fn scheme_mismatch_error() {
        let err = CredentialError::SchemeMismatch(Box::new(SchemeMismatch::by_name(
            "SecretToken",
            "ConnectionUri",
        )));
        assert!(err.to_string().contains("SecretToken"));
        assert!(err.to_string().contains("ConnectionUri"));
    }

    #[test]
    fn refresh_retryability_and_hint_follow_typed_advice() {
        use nebula_error::Classify;

        let delay = std::time::Duration::from_secs(17);
        let retry_delay = RetryDelay::new(delay).expect("non-zero test delay");
        let after =
            CredentialError::RefreshNotApplied(Box::new(RefreshNotAppliedContext::from_spec(
                RefreshNotAppliedPhase::ProviderConfirmedNotApplied,
                RefreshFailureSpec::new(
                    RefreshErrorKind::ProtocolError,
                    RetryAdvice::After(retry_delay),
                ),
            )));
        assert!(after.is_retryable());
        assert_eq!(
            after.retry_hint(),
            Some(nebula_error::RetryHint::after(delay))
        );

        let never =
            CredentialError::RefreshNotApplied(Box::new(RefreshNotAppliedContext::from_spec(
                RefreshNotAppliedPhase::BeforeDispatch,
                RefreshFailureSpec::new(RefreshErrorKind::ProviderUnavailable, RetryAdvice::Never),
            )));
        assert!(!never.is_retryable());
        assert_eq!(never.retry_hint(), None);
    }

    #[test]
    fn refresh_retry_delay_is_ceil_second_bounded() {
        assert_eq!(
            RetryDelay::new(std::time::Duration::ZERO),
            Err(RetryDelayError::Zero)
        );
        assert_eq!(
            RetryDelay::new(std::time::Duration::from_nanos(1))
                .expect("a positive subsecond delay rounds up")
                .get(),
            std::time::Duration::from_secs(1)
        );
        assert_eq!(
            RetryDelay::new(std::time::Duration::from_secs(RetryDelay::MAX_SECS + 1)),
            Err(RetryDelayError::TooLong)
        );
    }

    #[test]
    fn refresh_failure_diagnostics_redact_integration_text_and_code() {
        const CANARY: &str = "integration-secret-canary";
        let code = RefreshDiagnosticCode::parse(CANARY).expect("valid diagnostic code");
        let context = RefreshNotAppliedContext::from_spec(
            RefreshNotAppliedPhase::ProviderConfirmedNotApplied,
            RefreshFailureSpec::new(RefreshErrorKind::ProtocolError, RetryAdvice::Never)
                .with_diagnostic_code(code),
        );

        assert_eq!(
            context.diagnostic_code().map(RefreshDiagnosticCode::as_str),
            Some(CANARY)
        );

        let error = CredentialError::RefreshNotApplied(Box::new(context));
        let display = format!("{error}");
        let debug = format!("{error:?}");
        assert!(!display.contains(CANARY));
        assert!(!debug.contains(CANARY));
        assert!(display.contains("redacted"));
        assert!(debug.contains("diagnostic_code_present"));
    }

    #[test]
    fn provider_network_is_retryable() {
        use nebula_error::Classify;

        let err = CredentialError::Provider(Box::new(ProviderErrorContext::new(
            ProviderErrorKind::Network,
            SecretFreeMessage::new("connection refused"),
        )));
        assert!(err.is_retryable());

        let err = CredentialError::Provider(Box::new(ProviderErrorContext::new(
            ProviderErrorKind::Auth,
            SecretFreeMessage::new("unauthorized"),
        )));
        assert!(!err.is_retryable());
    }

    // ── Access error tests ──────────────────────────────────────────────────

    #[test]
    fn access_not_found_display() {
        let err = CredentialAccessError::NotFound("api_key".to_owned());
        assert_eq!(err.to_string(), "credential not found: api_key");
    }

    #[test]
    fn access_type_mismatch_display() {
        let err = CredentialAccessError::TypeMismatch("expected SecretToken".to_owned());
        assert!(err.to_string().contains("SecretToken"));
    }

    #[test]
    fn access_denied_display() {
        let err = CredentialAccessError::AccessDenied {
            capability: "credential type `OAuth2Token`".to_owned(),
            action_id: "my_action".to_owned(),
        };
        assert!(err.to_string().contains("OAuth2Token"));
        assert!(err.to_string().contains("my_action"));
    }

    #[test]
    fn access_not_configured_display() {
        let err = CredentialAccessError::NotConfigured(
            "credential capability is not configured".to_owned(),
        );
        assert!(err.to_string().contains("not configured"));
    }

    #[test]
    fn access_error_is_clone() {
        let err = CredentialAccessError::NotFound("x".to_owned());
        let cloned = err.clone();
        assert_eq!(err.to_string(), cloned.to_string());
    }
}
