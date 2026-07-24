//! Structural credential refresh-retry state.
//!
//! Retry state is part of the credential aggregate. It is deliberately
//! separate from user-visible metadata and from the short-lived refresh claim
//! used only for cross-replica coordination.

use std::{fmt, time::Duration};

/// Sole dispatch-proof phase for a replay-safe refresh refusal.
///
/// [`RefreshRetryKind`] is orthogonal diagnostic context. Persistence and
/// retry consumers must never infer whether dispatch occurred from the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefreshRetryPhase {
    /// The integration proved that provider dispatch did not begin.
    BeforeDispatch,
    /// A complete provider response proved that the operation was not applied.
    ProviderConfirmedNotApplied,
}

/// Closed, low-cardinality diagnostic class of replay-safe refresh failure.
///
/// This value intentionally carries no dispatch semantics; the same kind may
/// accompany either proof phase. [`RefreshRetryPhase`] is the only authority
/// for whether provider dispatch began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefreshRetryKind {
    /// A transient network or transport condition.
    TransientNetwork,
    /// The provider was temporarily unavailable.
    ProviderUnavailable,
    /// A request, response, or framework contract was rejected at the protocol
    /// layer.
    ProtocolError,
}

/// Validated low-cardinality diagnostic code for refresh-retry evidence.
///
/// The code is limited to 64 ASCII alphanumeric/`_`/`-`/`.`/`:` bytes and is
/// redacted from `Debug` and `Display`. Provider descriptions, tenant data,
/// secrets, and other free-form input must never be stored in this value.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RefreshRetryDiagnosticCode(String);

impl RefreshRetryDiagnosticCode {
    /// Maximum encoded diagnostic-code length.
    pub const MAX_LEN: usize = 64;

    /// Parse a fixed, integration-authored diagnostic code.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, non-ASCII, or unsupported values.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, RefreshRetryDiagnosticCodeError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(RefreshRetryDiagnosticCodeError::Empty);
        }
        if value.len() > Self::MAX_LEN {
            return Err(RefreshRetryDiagnosticCodeError::TooLong);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
        {
            return Err(RefreshRetryDiagnosticCodeError::InvalidCharacter);
        }
        Ok(Self(value.to_owned()))
    }

    /// Explicitly expose the validated code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RefreshRetryDiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RefreshRetryDiagnosticCode([REDACTED])")
    }
}

impl fmt::Display for RefreshRetryDiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

/// Invalid refresh-retry diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RefreshRetryDiagnosticCodeError {
    /// Codes must not be empty.
    #[error("credential refresh retry diagnostic code must not be empty")]
    Empty,
    /// Codes are bounded for storage and observability safety.
    #[error("credential refresh retry diagnostic code is too long")]
    TooLong,
    /// Codes use a deliberately small ASCII alphabet.
    #[error("credential refresh retry diagnostic code contains an invalid character")]
    InvalidCharacter,
}

/// Closed proof that a failed refresh remains safe to retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshRetryEvidence {
    phase: RefreshRetryPhase,
    kind: RefreshRetryKind,
    diagnostic_code: Option<RefreshRetryDiagnosticCode>,
}

impl RefreshRetryEvidence {
    /// Construct validated replay-safety evidence.
    #[must_use]
    pub fn new(
        phase: RefreshRetryPhase,
        kind: RefreshRetryKind,
        diagnostic_code: Option<RefreshRetryDiagnosticCode>,
    ) -> Self {
        Self {
            phase,
            kind,
            diagnostic_code,
        }
    }

    /// Return the proof-bearing failure phase.
    #[must_use]
    pub const fn phase(&self) -> RefreshRetryPhase {
        self.phase
    }

    /// Return the closed failure kind.
    #[must_use]
    pub const fn kind(&self) -> RefreshRetryKind {
        self.kind
    }

    /// Borrow the optional low-cardinality diagnostic code.
    #[must_use]
    pub fn diagnostic_code(&self) -> Option<&RefreshRetryDiagnosticCode> {
        self.diagnostic_code.as_ref()
    }
}

/// Validated, whole-second delay for a replay-safe refresh retry.
///
/// Sub-second input is rounded up so a persisted gate can never reopen
/// earlier than requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RefreshRetryDelay(u32);

impl RefreshRetryDelay {
    /// Maximum supported retry delay: 365 days.
    pub const MAX_SECS: u64 = 31_536_000;

    /// Normalize a duration upward to a non-zero whole-second delay.
    ///
    /// # Errors
    ///
    /// Rejects zero and any value whose ceiling exceeds
    /// [`MAX_SECS`](Self::MAX_SECS).
    pub fn new(duration: Duration) -> Result<Self, RefreshRetryDelayError> {
        if duration.is_zero() {
            return Err(RefreshRetryDelayError::Zero);
        }
        let rounded_seconds = duration
            .as_secs()
            .checked_add(u64::from(duration.subsec_nanos() != 0))
            .ok_or(RefreshRetryDelayError::TooLong)?;
        if rounded_seconds > Self::MAX_SECS {
            return Err(RefreshRetryDelayError::TooLong);
        }
        let seconds =
            u32::try_from(rounded_seconds).map_err(|_| RefreshRetryDelayError::TooLong)?;
        Ok(Self(seconds))
    }

    /// Construct from an already whole-second database value.
    ///
    /// # Errors
    ///
    /// Applies the same non-zero upper bound as [`Self::new`].
    pub fn from_seconds(seconds: u64) -> Result<Self, RefreshRetryDelayError> {
        Self::new(Duration::from_secs(seconds))
    }

    /// Return the normalized whole-second value.
    #[must_use]
    pub const fn as_secs(self) -> u64 {
        self.0 as u64
    }

    /// Return the normalized duration.
    #[must_use]
    pub const fn get(self) -> Duration {
        Duration::from_secs(self.as_secs())
    }
}

impl TryFrom<Duration> for RefreshRetryDelay {
    type Error = RefreshRetryDelayError;

    fn try_from(duration: Duration) -> Result<Self, Self::Error> {
        Self::new(duration)
    }
}

/// Invalid refresh-retry delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RefreshRetryDelayError {
    /// Immediate retries could create an unbounded hot loop.
    #[error("credential refresh retry delay must be non-zero")]
    Zero,
    /// The delay exceeds the cross-backend and transport-safe bound.
    #[error("credential refresh retry delay exceeds the supported bound")]
    TooLong,
}

/// Durable structural retry gate on a live credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshRetryGate {
    /// Automatic retry is blocked for the current credential-material epoch.
    ///
    /// An explicit authority-changing replacement, reauthentication, or
    /// reconnect transition can clear this gate; it is not globally eternal.
    Never {
        /// Proof that the failure is safe to classify exactly.
        evidence: RefreshRetryEvidence,
    },
    /// Retry remains blocked until the backend-authored absolute instant.
    NotBefore {
        /// Absolute instant computed from the persistence backend's clock.
        not_before: chrono::DateTime<chrono::Utc>,
        /// Proof that the failure is safe to retry after the delay.
        evidence: RefreshRetryEvidence,
    },
}

impl RefreshRetryGate {
    /// Borrow the proof attached to this gate.
    #[must_use]
    pub const fn evidence(&self) -> &RefreshRetryEvidence {
        match self {
            Self::Never { evidence } | Self::NotBefore { evidence, .. } => evidence,
        }
    }

    /// Return the backend-authored reopening instant, when this is a timed gate.
    #[must_use]
    pub const fn not_before(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        match self {
            Self::Never { .. } => None,
            Self::NotBefore { not_before, .. } => Some(*not_before),
        }
    }
}

/// Explicit gate transition carried by a credential replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshRetryTransition {
    /// Leave the current structural retry gate unchanged.
    Preserve,
    /// Remove any current gate.
    Clear,
    /// Block automatic retry for the current credential-material epoch.
    ///
    /// A later explicit authority-changing transition can clear this gate.
    SetNever {
        /// Proof that provider application is exactly known.
        evidence: RefreshRetryEvidence,
    },
    /// Install a timed gate using the persistence backend's clock.
    SetAfter {
        /// Normalized, bounded whole-second delay.
        delay: RefreshRetryDelay,
        /// Proof that provider application is exactly known.
        evidence: RefreshRetryEvidence,
    },
}

/// Backend-clock evaluation of a credential refresh-retry gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshRetryAdmission {
    /// No active gate prevents a refresh attempt.
    Open,
    /// A durable gate currently prevents provider dispatch.
    Blocked(RefreshRetryBlock),
}

/// One atomic, secret-free view used to recheck refresh outcomes.
///
/// The row version, material epoch, reauthentication flag, and retry admission
/// are observed at one backend linearization point. Consumers must compare the
/// epoch—not serialized bytes or display-only row versions—to decide whether
/// refresh authority changed, and must not reconstruct this view from separate
/// persistence reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshRetrySnapshot {
    version: crate::CredentialVersion,
    material_epoch: crate::CredentialMaterialEpoch,
    reauth_required: bool,
    admission: RefreshRetryAdmission,
}

impl RefreshRetrySnapshot {
    /// Construct one backend-authored atomic refresh snapshot.
    #[must_use]
    pub fn new(
        version: crate::CredentialVersion,
        material_epoch: crate::CredentialMaterialEpoch,
        reauth_required: bool,
        admission: RefreshRetryAdmission,
    ) -> Self {
        Self {
            version,
            material_epoch,
            reauth_required,
            admission,
        }
    }

    /// Return the credential version observed with the admission decision.
    #[must_use]
    pub const fn version(&self) -> crate::CredentialVersion {
        self.version
    }

    /// Return the material/refresh-authority epoch from the same snapshot.
    #[must_use]
    pub const fn material_epoch(&self) -> crate::CredentialMaterialEpoch {
        self.material_epoch
    }

    /// Return whether reauthentication was required at the same snapshot.
    #[must_use]
    pub const fn reauth_required(&self) -> bool {
        self.reauth_required
    }

    /// Borrow the retry admission evaluated at the same snapshot.
    #[must_use]
    pub const fn admission(&self) -> &RefreshRetryAdmission {
        &self.admission
    }

    /// Consume the snapshot and return its retry-admission decision.
    #[must_use]
    pub fn into_admission(self) -> RefreshRetryAdmission {
        self.admission
    }
}

/// Why refresh admission is currently blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshRetryBlock {
    /// Automatic retry is blocked until an explicit authority-changing transition.
    Never {
        /// Proof attached to the durable gate.
        evidence: RefreshRetryEvidence,
    },
    /// Retry is prohibited for the remaining backend-clock duration.
    After {
        /// Remaining delay, rounded up to whole seconds.
        remaining: RefreshRetryDelay,
        /// Proof attached to the durable gate.
        evidence: RefreshRetryEvidence,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        RefreshRetryDelay, RefreshRetryDelayError, RefreshRetryDiagnosticCode,
        RefreshRetryDiagnosticCodeError,
    };
    use std::time::Duration;

    #[test]
    fn retry_delay_rounds_up_and_enforces_the_shared_bound() {
        assert_eq!(
            RefreshRetryDelay::new(Duration::from_nanos(1))
                .expect("one nanosecond rounds to one second")
                .as_secs(),
            1
        );
        assert_eq!(
            RefreshRetryDelay::new(Duration::from_millis(1_001))
                .expect("fractional seconds remain conservative")
                .as_secs(),
            2
        );
        assert_eq!(
            RefreshRetryDelay::from_seconds(RefreshRetryDelay::MAX_SECS)
                .expect("the inclusive upper bound is valid")
                .as_secs(),
            RefreshRetryDelay::MAX_SECS
        );
        assert_eq!(
            RefreshRetryDelay::new(Duration::ZERO),
            Err(RefreshRetryDelayError::Zero)
        );
        assert_eq!(
            RefreshRetryDelay::new(
                Duration::from_secs(RefreshRetryDelay::MAX_SECS) + Duration::from_nanos(1)
            ),
            Err(RefreshRetryDelayError::TooLong)
        );
    }

    #[test]
    fn diagnostic_code_is_bounded_and_redacted() {
        let code =
            RefreshRetryDiagnosticCode::parse("oauth.invalid-client").expect("fixed code is valid");
        assert_eq!(code.as_str(), "oauth.invalid-client");
        assert_eq!(
            format!("{code:?} {code}"),
            "RefreshRetryDiagnosticCode([REDACTED]) [REDACTED]"
        );
        assert_eq!(
            RefreshRetryDiagnosticCode::parse(""),
            Err(RefreshRetryDiagnosticCodeError::Empty)
        );
        assert_eq!(
            RefreshRetryDiagnosticCode::parse("contains space"),
            Err(RefreshRetryDiagnosticCodeError::InvalidCharacter)
        );
        assert_eq!(
            RefreshRetryDiagnosticCode::parse("x".repeat(65)),
            Err(RefreshRetryDiagnosticCodeError::TooLong)
        );
    }
}
