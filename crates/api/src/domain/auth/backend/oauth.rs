//! Plane-A OAuth helpers — sign-in via Google or GitHub.
//!
//! This is **distinct** from Plane-B integration credential acquisition.
//! Plane A is "who is signing in to Nebula"; Plane B is "Nebula on behalf
//! of a user talking to Slack/HubSpot/etc." and uses the universal credential
//! resolve/continue protocol rather than raw OAuth routes.
//!
//! The state machine is small:
//!
//! 1. `start` — mint random `state` + PKCE `code_verifier`, store under TTL, return the
//!    `authorize_url`.
//! 2. `complete` — atomically remove the matching live `(state, provider)`
//!    entry and use its PKCE verifier for the token exchange. Presence means
//!    unconsumed; no replay tombstone is representable.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use sha2::{Digest, Sha256};

use super::error::AuthError;

/// Default TTL for the state store entry (10 minutes).
pub(super) const OAUTH_STATE_TTL: Duration = Duration::from_mins(10);

/// Supported Plane-A OAuth providers.
///
/// Serialize/Deserialize derived so `OAuthProvidersConfig` can use
/// `HashMap<OAuthProvider, OAuthProviderConfig>` keyed by enum value
/// (per ADR-0085 D-5). Each variant pins its exact serde and OpenAPI token;
/// this matters for `GitHub`, whose mechanical snake-case spelling would be
/// the incompatible `git_hub`. Drift against [`Self::as_str`] and `FromStr`
/// is covered by a unit test.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[non_exhaustive]
pub enum OAuthProvider {
    /// Sign in with Google.
    #[serde(rename = "google")]
    #[schema(rename = "google")]
    Google,
    /// Sign in with GitHub.
    #[serde(rename = "github")]
    #[schema(rename = "github")]
    GitHub,
}

impl std::str::FromStr for OAuthProvider {
    type Err = AuthError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "google" => Ok(Self::Google),
            "github" => Ok(Self::GitHub),
            _ => Err(AuthError::InvalidInput("unknown OAuth provider")),
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
        }
    }
}

/// Stored entry under one `state` key.
///
/// Presence in the state store means unconsumed. Successful completion
/// removes the entry atomically, so replay has no representable tombstone.
#[derive(Clone)]
pub(super) struct OAuthStateEntry {
    /// Identity provider for this OAuth flow.
    pub(super) provider: OAuthProvider,
    /// Random PKCE verifier (43-128 chars per RFC 7636 §4.1).
    pub(super) code_verifier: String,
    /// Unix-seconds expiry; entries past this are evicted lazily.
    pub(super) expires_at: u64,
    /// Handler-derived redirect_uri persisted at `start_oauth` time
    /// so `complete_oauth` can re-verify it against the user's
    /// callback per ADR-0085 REQ-oauth-003 Scenario 3.10
    /// (`public_url_changed_mid_flow` defense).
    pub(super) redirect_uri: String,
}

impl std::fmt::Debug for OAuthStateEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthStateEntry")
            .field("provider", &self.provider)
            .field("code_verifier", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .field("redirect_uri", &"[redacted]")
            .finish()
    }
}

/// PKCE pair — `state` plus `code_verifier`/`code_challenge`.
#[derive(Clone)]
pub(super) struct PkcePair {
    /// Opaque random state string.
    pub(super) state: String,
    /// PKCE verifier (kept server-side).
    pub(super) code_verifier: String,
    /// PKCE S256 challenge (sent to provider).
    pub(super) code_challenge: String,
}

impl std::fmt::Debug for PkcePair {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PkcePair")
            .field("state", &"[redacted]")
            .field("code_verifier", &"[redacted]")
            .field("code_challenge", &"[redacted]")
            .finish()
    }
}

/// Mint a fresh PKCE pair plus a state token.
pub(super) fn mint_pkce() -> Result<PkcePair, AuthError> {
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
pub(super) fn expiry_unix(ttl: Duration) -> u64 {
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
        for s in &["google", "github"] {
            let p: OAuthProvider = s.parse().unwrap();
            assert_eq!(p.as_str(), *s);
            assert_eq!(
                serde_json::to_string(&p).expect("provider serializes"),
                format!("\"{s}\"")
            );
            assert_eq!(
                serde_json::from_str::<OAuthProvider>(&format!("\"{s}\""))
                    .expect("provider deserializes"),
                p
            );
        }
        assert!("microsoft".parse::<OAuthProvider>().is_err());
        assert!("yahoo".parse::<OAuthProvider>().is_err());
    }

    #[test]
    fn unknown_provider_error_does_not_echo_input() {
        const CANARY: &str = "unknown-provider-CANARY-7d2e";

        let error = CANARY
            .parse::<OAuthProvider>()
            .expect_err("unknown provider must fail closed");
        let display = error.to_string();
        let debug = format!("{error:?}");

        assert!(!display.contains(CANARY));
        assert!(!debug.contains(CANARY));
        assert!(std::error::Error::source(&error).is_none());

        let api_error: crate::ApiError = error.into();
        assert!(!api_error.to_string().contains(CANARY));
        assert!(!format!("{api_error:?}").contains(CANARY));
        let (_, problem) = api_error.to_problem_details();
        let wire = serde_json::to_string(&problem).expect("problem details serialize");
        assert!(!wire.contains(CANARY));
    }

    #[test]
    fn oauth_state_and_pkce_debug_redact_secrets() {
        let pkce = PkcePair {
            state: "STATE_CANARY-6b5c".to_owned(),
            code_verifier: "VERIFIER_CANARY-a983".to_owned(),
            code_challenge: "CHALLENGE_CANARY-816d".to_owned(),
        };
        let pkce_debug = format!("{pkce:?}");
        assert!(!pkce_debug.contains("STATE_CANARY-6b5c"));
        assert!(!pkce_debug.contains("VERIFIER_CANARY-a983"));
        assert!(!pkce_debug.contains("CHALLENGE_CANARY-816d"));

        let entry = OAuthStateEntry {
            provider: OAuthProvider::Google,
            code_verifier: "ENTRY_VERIFIER_CANARY-c61f".to_owned(),
            expires_at: 42,
            redirect_uri: "https://example.test/callback?canary=REDIRECT_CANARY-04da".to_owned(),
        };
        let entry_debug = format!("{entry:?}");
        assert!(!entry_debug.contains("ENTRY_VERIFIER_CANARY-c61f"));
        assert!(!entry_debug.contains("REDIRECT_CANARY-04da"));
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
