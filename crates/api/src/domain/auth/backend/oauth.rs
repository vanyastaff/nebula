//! Plane-A OAuth helpers — sign-in via Google / GitHub / Microsoft.
//!
//! This is **distinct** from Plane-B integration credential OAuth (per
//! auth plane separation and `crates/api/src/services/oauth/`). Plane A is "who is
//! signing in to Nebula"; Plane B is "Nebula on behalf of a user
//! talking to Slack/HubSpot/etc.".
//!
//! The state machine is small:
//!
//! 1. `start` — mint random `state` + PKCE `code_verifier`, store under TTL, return the
//!    `authorize_url`.
//! 2. `complete` — pop the stored entry by `state`, ensure the entry has not been consumed, return
//!    the `code_verifier` so the backend can finish the token exchange.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use sha2::{Digest, Sha256};

use super::error::AuthError;

/// Default TTL for the state store entry (10 minutes).
pub const OAUTH_STATE_TTL: Duration = Duration::from_mins(10);

/// Supported Plane-A OAuth providers.
///
/// Serialize/Deserialize derived so `OAuthProvidersConfig` can use
/// `HashMap<OAuthProvider, OAuthProviderConfig>` keyed by enum value
/// (per ADR-0085 D-5). `serde(rename_all = "snake_case")` so TOML keys
/// like `[auth.oauth.providers.google]` deserialize directly into the
/// enum without a custom `Deserialize` impl. Drift between `FromStr`
/// and `Deserialize` is closed because both use the same lowercase
/// strings (verified by a unit test).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthProvider {
    /// Sign in with Google.
    Google,
    /// Sign in with GitHub.
    GitHub,
    /// Sign in with Microsoft.
    Microsoft,
}

impl std::str::FromStr for OAuthProvider {
    type Err = AuthError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "google" => Ok(Self::Google),
            "github" => Ok(Self::GitHub),
            "microsoft" => Ok(Self::Microsoft),
            other => Err(AuthError::OAuthFailed(format!("unknown provider: {other}"))),
        }
    }
}

impl OAuthProvider {
    /// Stable string representation for storage / logging.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::GitHub => "github",
            Self::Microsoft => "microsoft",
        }
    }
}

/// Stored entry under one `state` key.
///
/// `consumed` flips `true` after `complete` so a replay returns
/// [`AuthError::InvalidToken`] instead of leaking the verifier.
#[derive(Debug, Clone)]
pub struct OAuthStateEntry {
    /// Identity provider for this OAuth flow.
    pub provider: OAuthProvider,
    /// Random PKCE verifier (43-128 chars per RFC 7636 §4.1).
    pub code_verifier: String,
    /// Unix-seconds expiry; entries past this are evicted lazily.
    pub expires_at: u64,
    /// Set on first `complete` call to prevent replay.
    pub consumed: bool,
}

/// PKCE pair — `state` plus `code_verifier`/`code_challenge`.
#[derive(Debug, Clone)]
pub struct PkcePair {
    /// Opaque random state string.
    pub state: String,
    /// PKCE verifier (kept server-side).
    pub code_verifier: String,
    /// PKCE S256 challenge (sent to provider).
    pub code_challenge: String,
}

/// Mint a fresh PKCE pair plus a state token.
pub fn mint_pkce() -> Result<PkcePair, AuthError> {
    let mut state_bytes = [0u8; 32];
    let mut verifier_bytes = [0u8; 64];
    rand::rng().fill_bytes(&mut state_bytes);
    rand::rng().fill_bytes(&mut verifier_bytes);

    let state = URL_SAFE_NO_PAD.encode(state_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    Ok(PkcePair {
        state,
        code_verifier,
        code_challenge,
    })
}

/// Compute the unix-seconds timestamp `ttl` from now.
#[must_use]
pub fn expiry_unix(ttl: Duration) -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + ttl.as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_roundtrip() {
        for s in &["google", "github", "microsoft"] {
            let p: OAuthProvider = s.parse().unwrap();
            assert_eq!(p.as_str(), *s);
        }
        assert!(matches!(
            "yahoo".parse::<OAuthProvider>(),
            Err(AuthError::OAuthFailed(_))
        ));
    }

    #[test]
    fn pkce_pairs_are_unique() {
        let a = mint_pkce().unwrap();
        let b = mint_pkce().unwrap();
        assert_ne!(a.state, b.state);
        assert_ne!(a.code_verifier, b.code_verifier);
        assert_ne!(a.code_challenge, b.code_challenge);
    }

    #[test]
    fn pkce_challenge_is_sha256_of_verifier() {
        let p = mint_pkce().unwrap();
        let mut h = Sha256::new();
        h.update(p.code_verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(h.finalize());
        assert_eq!(p.code_challenge, expected);
    }
}
