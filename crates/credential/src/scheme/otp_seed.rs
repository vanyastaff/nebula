//! TOTP/HOTP seed for one-time passcode generation.

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// Seed material for generating TOTP or HOTP one-time passcodes.
///
/// The seed is typically a Base32-encoded shared secret provisioned by
/// the authenticating service. Combined with `algorithm`, `digits`, and
/// `period`, it fully describes an OTP configuration.
///
/// # Examples
///
/// ```
/// use nebula_credential::scheme::OtpSeed;
/// use nebula_core::SecretString;
///
/// let seed = OtpSeed::new(SecretString::new("JBSWY3DPEHPK3PXP"), "SHA1", 6)
///     .with_period(30);
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct OtpSeed {
    #[serde(with = "nebula_core::serde_secret")]
    seed: SecretString,
    algorithm: String,
    digits: u8,
    period: Option<u32>,
}

impl OtpSeed {
    /// Creates a new OTP seed.
    ///
    /// - `seed`: Base32-encoded shared secret
    /// - `algorithm`: hash algorithm (e.g., `"SHA1"`, `"SHA256"`)
    /// - `digits`: number of OTP digits (typically 6 or 8)
    #[must_use]
    pub fn new(seed: SecretString, algorithm: impl Into<String>, digits: u8) -> Self {
        Self {
            seed,
            algorithm: algorithm.into(),
            digits,
            period: None,
        }
    }

    /// Sets the TOTP time step in seconds (e.g., 30 for standard TOTP).
    ///
    /// Leave unset for HOTP (counter-based) configurations.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_period(mut self, period: u32) -> Self {
        self.period = Some(period);
        self
    }

    /// Returns the OTP seed secret.
    pub fn seed(&self) -> &SecretString {
        &self.seed
    }

    /// Returns the hash algorithm identifier.
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }

    /// Returns the number of digits in generated OTPs.
    pub fn digits(&self) -> u8 {
        self.digits
    }

    /// Returns the TOTP time step in seconds, or `None` for HOTP.
    pub fn period(&self) -> Option<u32> {
        self.period
    }
}

impl AuthScheme for OtpSeed {
    fn pattern() -> AuthPattern {
        AuthPattern::OneTimePasscode
    }
}

impl std::fmt::Debug for OtpSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtpSeed")
            .field("seed", &"[REDACTED]")
            .field("algorithm", &self.algorithm)
            .field("digits", &self.digits)
            .field("period", &self.period)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_one_time_passcode() {
        assert_eq!(OtpSeed::pattern(), AuthPattern::OneTimePasscode);
    }

    #[test]
    fn debug_redacts_seed() {
        let seed = OtpSeed::new(SecretString::new("JBSWY3DPEHPK3PXP"), "SHA1", 6)
            .with_period(30);
        let debug = format!("{seed:?}");
        assert!(debug.contains("SHA1"));
        assert!(debug.contains("30"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("JBSWY3DPEHPK3PXP"));
    }
}
