//! Error types for credential operations
//!
//! This module defines a layered error hierarchy:
//! - [`CredentialError`]: Top-level error wrapping Crypto/Validation
//! - [`CryptoError`]: Encryption, decryption, key derivation
//! - [`ValidationError`]: Invalid credential IDs, malformed data
//! - [`StoreError`](crate::StoreError): Storage-layer errors (not found, conflict)
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
//! [`StoreError`](crate::StoreError) is used directly by the storage layer:
//!
//! ```
//! use nebula_credential::StoreError;
//!
//! let err = StoreError::NotFound {
//!     id: "missing_cred".to_string(),
//! };
//! assert!(err.to_string().contains("missing_cred"));
//! ```

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

/// Identifies a credential auth scheme by kind. Used in
/// [`SchemeMismatch`] without leaking scheme-internal types into the
/// error surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SchemeKind {
    /// Simple opaque bearer / API token.
    SecretToken,
    /// Username + password credential.
    IdentityPassword,
    /// OAuth 2 access/refresh token pair.
    OAuth2Token,
    /// Asymmetric key pair.
    KeyPair,
    /// TLS or mTLS certificate.
    Certificate,
    /// Request-signing key (HMAC / SigV4).
    SigningKey,
    /// Database or service connection URI.
    ConnectionUri,
    /// Cloud-provider instance metadata credential.
    InstanceBinding,
    /// Pre-shared symmetric key.
    SharedKey,
}

impl std::fmt::Display for SchemeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

/// Identity of an auth scheme on either side of a [`SchemeMismatch`].
///
/// First-party schemes are referred to by their typed [`SchemeKind`]
/// variant; plugin / third-party schemes are carried by their pattern
/// name string (the snapshot layer only knows them by name, not by
/// enum variant — see [`SchemeMismatch::by_name`]).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SchemeIdentity {
    /// A first-party scheme (compile-time-known variant).
    Typed(SchemeKind),
    /// A scheme identified by its pattern name (typically a plugin scheme
    /// whose [`SchemeKind`] is not in the first-party enum).
    Named(CompactString),
}

impl std::fmt::Display for SchemeIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Typed(k) => std::fmt::Display::fmt(k, f),
            Self::Named(n) => f.write_str(n.as_str()),
        }
    }
}

/// Scheme mismatch between what a consumer expects and what is present.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SchemeMismatch {
    expected: SchemeIdentity,
    actual: SchemeIdentity,
}

impl SchemeMismatch {
    /// Construct from typed [`SchemeKind`] sides (first-party schemes).
    pub fn new(expected: SchemeKind, actual: SchemeKind) -> Self {
        Self {
            expected: SchemeIdentity::Typed(expected),
            actual: SchemeIdentity::Typed(actual),
        }
    }

    /// Construct from pattern-name strings (used by the snapshot layer
    /// which only knows plugin schemes by name).
    pub fn by_name(expected: impl Into<CompactString>, actual: impl Into<CompactString>) -> Self {
        Self {
            expected: SchemeIdentity::Named(expected.into()),
            actual: SchemeIdentity::Named(actual.into()),
        }
    }

    /// The scheme the consumer expected.
    pub fn expected(&self) -> &SchemeIdentity {
        &self.expected
    }

    /// The scheme that was actually present.
    pub fn actual(&self) -> &SchemeIdentity {
        &self.actual
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

/// What kind of refresh failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshErrorKind {
    /// Refresh token itself has expired -- needs re-authentication.
    TokenExpired,
    /// Credential was explicitly revoked at the provider.
    TokenRevoked,
    /// Transient network error -- retry may succeed.
    TransientNetwork,
    /// Provider is temporarily unavailable.
    ProviderUnavailable,
    /// Protocol-level error (invalid grant, bad response format).
    ProtocolError,
}

/// Retry guidance from credential to framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetryAdvice {
    /// Never retry -- permanent failure.
    Never,
    /// Retry immediately.
    Immediate,
    /// Retry after the given duration.
    After(std::time::Duration),
}

/// Context struct for [`CredentialError::RefreshFailed`].
///
/// Each field is accessible only via the provided accessor methods — the
/// struct is `#[non_exhaustive]` so future fields do not break callers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RefreshFailedContext {
    kind: RefreshErrorKind,
    retry: RetryAdvice,
    cause: SecretFreeMessage,
    provider_code: Option<CompactString>,
}

impl RefreshFailedContext {
    /// Construct with kind, retry advice, and a secret-free cause message.
    pub fn new(kind: RefreshErrorKind, retry: RetryAdvice, cause: SecretFreeMessage) -> Self {
        Self {
            kind,
            retry,
            cause,
            provider_code: None,
        }
    }

    /// Attach an optional provider-specific error code string.
    pub fn with_code(mut self, code: impl Into<CompactString>) -> Self {
        self.provider_code = Some(code.into());
        self
    }

    /// The kind of refresh failure.
    pub fn kind(&self) -> RefreshErrorKind {
        self.kind
    }

    /// Retry guidance for the framework.
    pub fn retry(&self) -> RetryAdvice {
        self.retry
    }

    /// The secret-free cause message.
    pub fn cause(&self) -> &SecretFreeMessage {
        &self.cause
    }

    /// An optional provider-specific error code.
    pub fn provider_code(&self) -> Option<&str> {
        self.provider_code.as_deref()
    }
}

impl std::fmt::Display for RefreshFailedContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.cause)
    }
}

// ── Revoke failure ───────────────────────────────────────────────────────────

/// Discriminated kind for a credential revocation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RevokeErrorKind {
    /// The provider explicitly rejected the revocation.
    ProviderRejected,
    /// Network-level failure during revocation.
    Network,
    /// The token was already revoked at the provider.
    AlreadyRevoked,
    /// Revocation is not supported for this credential type.
    Unsupported,
    /// Catch-all for other revocation errors.
    Other,
}

/// Context struct for [`CredentialError::RevokeFailed`].
///
/// Each field is accessible only via the provided accessor methods — the
/// struct is `#[non_exhaustive]` so future fields do not break callers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RevokeFailedContext {
    kind: RevokeErrorKind,
    cause: SecretFreeMessage,
}

impl RevokeFailedContext {
    /// Construct with kind and a secret-free cause message.
    pub fn new(kind: RevokeErrorKind, cause: SecretFreeMessage) -> Self {
        Self { kind, cause }
    }

    /// The kind of revocation failure.
    pub fn kind(&self) -> RevokeErrorKind {
        self.kind
    }

    /// The secret-free cause message.
    pub fn cause(&self) -> &SecretFreeMessage {
        &self.cause
    }
}

impl std::fmt::Display for RevokeFailedContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.cause)
    }
}

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
/// - `RefreshFailed(Box<RefreshFailedContext>)` — boxed context; use
///   [`RefreshFailedContext::new`] + accessors.
/// - `RevokeFailed(Box<RevokeFailedContext>)` — boxed context.
/// - `SchemeMismatch(SchemeMismatch)` — inline (two `SchemeKind` bytes + tag ≤ 8 B).
/// - `NotInteractive` — unit variant.
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

    /// Refresh failed with structured error info.
    #[error("refresh failed: {0}")]
    RefreshFailed(Box<RefreshFailedContext>),

    /// Credential revocation failed.
    #[error("revoke failed: {0}")]
    RevokeFailed(Box<RevokeFailedContext>),

    /// Operation requires an interactive credential, but this credential
    /// is non-interactive.
    #[error("credential does not support interactive flows")]
    NotInteractive,

    /// Scheme type mismatch between credential and resource. Boxed
    /// because the inner `SchemeMismatch` now carries `SchemeIdentity`
    /// variants that may hold a [`CompactString`] (plugin scheme name)
    /// — keeping it inline would push the enum past the 32-byte cap.
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
            Self::Provider(_) => nebula_error::ErrorCategory::External,
            Self::RefreshFailed(_) | Self::RevokeFailed(_) => nebula_error::ErrorCategory::External,
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
            Self::Provider(_) => nebula_error::ErrorCode::new("CREDENTIAL:PROVIDER"),
            Self::RefreshFailed(_) => nebula_error::ErrorCode::new("CREDENTIAL:REFRESH_FAILED"),
            Self::RevokeFailed(_) => nebula_error::ErrorCode::new("CREDENTIAL:REVOKE_FAILED"),
            Self::SchemeMismatch(_) => nebula_error::ErrorCode::new("CREDENTIAL:SCHEME_MISMATCH"),
            Self::InvalidInput(_) => nebula_error::ErrorCode::new("CREDENTIAL:INVALID_INPUT"),
            Self::Resolution(_) => nebula_error::ErrorCode::new("CREDENTIAL:RESOLUTION_FAILED"),
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::RefreshFailed(ctx) => matches!(
                ctx.kind(),
                RefreshErrorKind::TransientNetwork | RefreshErrorKind::ProviderUnavailable
            ),
            Self::Provider(ctx) => matches!(
                ctx.kind(),
                ProviderErrorKind::Network
                    | ProviderErrorKind::RateLimit
                    | ProviderErrorKind::ServerError
            ),
            _ => false,
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
//   RefreshFailed(Box<RefreshFailedContext>) — 8B pointer
//   RevokeFailed(Box<RevokeFailedContext>)   — 8B pointer
//   NotInteractive                   — 0B payload
//   SchemeMismatch(Box<SchemeMismatch>) — 8B pointer
//      (was 2B payload pre-#732 fixup; boxed once `SchemeIdentity::Named`
//      carries a `CompactString` so the inline form pushed past 32B).
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
        let err = CredentialError::RefreshFailed(Box::new(RefreshFailedContext::new(
            RefreshErrorKind::TokenExpired,
            RetryAdvice::Never,
            SecretFreeMessage::new("refresh token expired"),
        )));
        assert!(matches!(
            &err,
            CredentialError::RefreshFailed(ctx) if ctx.kind() == RefreshErrorKind::TokenExpired
        ));
        assert!(err.to_string().contains("refresh failed"));
    }

    #[test]
    fn scheme_mismatch_error() {
        let err = CredentialError::SchemeMismatch(Box::new(SchemeMismatch::new(
            SchemeKind::SecretToken,
            SchemeKind::ConnectionUri,
        )));
        assert!(err.to_string().contains("SecretToken"));
        assert!(err.to_string().contains("ConnectionUri"));
    }

    #[test]
    fn refresh_transient_is_retryable() {
        use nebula_error::Classify;

        let err = CredentialError::RefreshFailed(Box::new(RefreshFailedContext::new(
            RefreshErrorKind::TransientNetwork,
            RetryAdvice::Immediate,
            SecretFreeMessage::new("connection reset"),
        )));
        assert!(err.is_retryable());

        let err = CredentialError::RefreshFailed(Box::new(RefreshFailedContext::new(
            RefreshErrorKind::TokenExpired,
            RetryAdvice::Never,
            SecretFreeMessage::new("expired"),
        )));
        assert!(!err.is_retryable());
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
