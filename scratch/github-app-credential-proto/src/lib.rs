//! # GitHub App credential prototype
//!
//! Scratch crate validating `nebula-credential`'s Credential trait against
//! a realistic complex authentication flow (GitHub App JWT + installation token).
//!
//! **Purpose:** exercise Spec H0 (refresh coordination) assumptions with a real
//! multi-step refresh workflow. NOT production code.
//!
//! ## Flow
//!
//! ```text
//! Setup:   GitHubAppConfig { app_id, installation_id, private_key_pem, api_base_url }
//!   │
//!   ▼ resolve()
//! Initial state: token=None, expires_at=None (lazy — first refresh() populates)
//!   │
//!   ▼ refresh()
//!   1. Generate JWT (RS256, iss=app_id, iat=now, exp=now+10min)
//!   2. POST {api_base_url}/app/installations/{id}/access_tokens
//!      Headers: Authorization: Bearer <JWT>, Accept: application/vnd.github+json
//!   3. Parse response: { "token": "ghs_...", "expires_at": "2024-01-01T12:00:00Z" }
//!   4. Update state.installation_token, state.token_expires_at
//!   │
//!   ▼ project()
//! Consumer gets: OAuth2Token { access_token: installation_token, token_type: "Bearer" }
//! ```
//!
//! ## What this scratch tests
//!
//! 1. Does current Credential trait actually support multi-step refresh?
//! 2. Does SecretBytes work for RSA PEM loading + passing to jsonwebtoken?
//! 3. Concurrent refresh race — без coordinator каждая "replica" hits mock
//!    independently. With simple L1 Mutex — one hit regardless of N callers.

use std::sync::Arc;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use nebula_credential::{
    AuthPattern, CredentialContext, CredentialMetadata, CredentialState, NoPendingState,
    OAuth2Token, SecretString, credential_key,
    error::CredentialError,
    resolve::{RefreshOutcome, ResolveResult},
};
use nebula_schema::FieldValues;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// GitHub App credential state — what gets persisted.
///
/// Holds both persistent material (app_id, installation_id, private_key) and
/// refreshable material (installation_token + expiry).
#[derive(Clone, Serialize, Deserialize)]
pub struct GitHubAppState {
    // ── Persistent (set at create, never changes) ────────────────────────────
    pub app_id: String,
    pub installation_id: String,
    /// RSA private key PEM bytes. Must be RS256-compatible.
    /// Stored as String for serde simplicity; treated as secret via redaction.
    #[serde(with = "nebula_credential::serde_secret")]
    pub private_key_pem: SecretString,
    /// API base URL. `https://api.github.com` в production; overridable для mock.
    pub api_base_url: String,

    // ── Refreshable (populated by refresh()) ─────────────────────────────────
    #[serde(default, with = "nebula_credential::serde_secret::option")]
    pub installation_token: Option<SecretString>,
    pub token_expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for GitHubAppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubAppState")
            .field("app_id", &self.app_id)
            .field("installation_id", &self.installation_id)
            .field("private_key_pem", &"[REDACTED]")
            .field("api_base_url", &self.api_base_url)
            .field("installation_token", &self.installation_token.as_ref().map(|_| "[REDACTED]"))
            .field("token_expires_at", &self.token_expires_at)
            .finish()
    }
}

impl Zeroize for GitHubAppState {
    fn zeroize(&mut self) {
        // String fields — wipe best-effort. SecretString handles its own zeroize via Drop.
        self.app_id.zeroize();
        self.installation_id.zeroize();
        self.api_base_url.zeroize();
        // installation_token / private_key_pem: SecretString already ZeroizeOnDrop.
    }
}

impl CredentialState for GitHubAppState {
    const KIND: &'static str = "github_app";
    const VERSION: u32 = 1;

    fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.token_expires_at
    }
}

/// GitHub App credential type.
pub struct GitHubAppCredential;

impl nebula_credential::Credential for GitHubAppCredential {
    type Input = FieldValues; // scratch — не defining typed input schema
    type Scheme = OAuth2Token;
    type State = GitHubAppState;
    type Pending = NoPendingState;

    const KEY: &'static str = "github_app";
    const REFRESHABLE: bool = true;
    const TESTABLE: bool = false;

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(credential_key!("github_app"))
            .name("GitHub App")
            .description("GitHub App JWT + installation token authentication")
            .pattern(AuthPattern::OAuth2)
            .build()
            .expect("static metadata should always build")
    }

    fn project(state: &Self::State) -> Self::Scheme {
        // Если refresh ещё не вызывался — token пустой. Consumer получает empty Bearer.
        // Real flow: callers check expires_at, triggering refresh перед project.
        match &state.installation_token {
            Some(tok) => OAuth2Token::new(tok.clone())
                .with_expires_at(state.token_expires_at.unwrap_or_else(Utc::now)),
            None => OAuth2Token::new(SecretString::new("".to_string())),
        }
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, CredentialError> {
        let app_id = values
            .get_string_by_str("app_id")
            .ok_or_else(|| CredentialError::InvalidInput("missing app_id".into()))?
            .to_string();
        let installation_id = values
            .get_string_by_str("installation_id")
            .ok_or_else(|| CredentialError::InvalidInput("missing installation_id".into()))?
            .to_string();
        let private_key_pem = values
            .get_string_by_str("private_key_pem")
            .ok_or_else(|| CredentialError::InvalidInput("missing private_key_pem".into()))?
            .to_string();
        let api_base_url = values
            .get_string_by_str("api_base_url")
            .unwrap_or("https://api.github.com")
            .to_string();

        Ok(ResolveResult::Complete(GitHubAppState {
            app_id,
            installation_id,
            private_key_pem: SecretString::new(private_key_pem),
            api_base_url,
            installation_token: None,
            token_expires_at: None,
        }))
    }

    async fn refresh(
        state: &mut Self::State,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        refresh_github_app_token(state)
            .await
            .map_err(|e| CredentialError::Provider(e.to_string()))?;
        Ok(RefreshOutcome::Refreshed)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// JWT + token exchange
// ─────────────────────────────────────────────────────────────────────────────

/// Errors specific to GitHub App refresh flow.
#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error("JWT signing failed: {0}")]
    JwtSign(#[from] jsonwebtoken::errors::Error),
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("GitHub API returned status {status}: {body}")]
    BadStatus { status: u16, body: String },
    #[error("response parse failed: {0}")]
    Parse(String),
    #[error("invalid private key PEM")]
    InvalidKey,
}

/// Refresh GitHub App installation token.
///
/// Separate function (not trait method) so tests + coordinator demo can call
/// directly без going through full Credential trait dispatch.
pub async fn refresh_github_app_token(state: &mut GitHubAppState) -> Result<(), RefreshError> {
    // ── Step 1: sign JWT ────────────────────────────────────────────────────
    let jwt = sign_app_jwt(&state.app_id, &state.private_key_pem)?;

    // ── Step 2: POST exchange ───────────────────────────────────────────────
    let url = format!(
        "{}/app/installations/{}/access_tokens",
        state.api_base_url.trim_end_matches('/'),
        state.installation_id
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let resp = client
        .post(&url)
        .bearer_auth(&jwt)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "nebula-github-app-credential-proto/0.0.0")
        .send()
        .await?;

    let status = resp.status();
    let body = resp.text().await?;

    if !status.is_success() {
        return Err(RefreshError::BadStatus {
            status: status.as_u16(),
            body,
        });
    }

    // ── Step 3: parse response ──────────────────────────────────────────────
    let parsed: TokenExchangeResponse =
        serde_json::from_str(&body).map_err(|e| RefreshError::Parse(e.to_string()))?;

    state.installation_token = Some(SecretString::new(parsed.token));
    state.token_expires_at = Some(parsed.expires_at);

    Ok(())
}

/// Sign a GitHub App JWT per
/// <https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-json-web-token-jwt-for-a-github-app>.
fn sign_app_jwt(app_id: &str, private_key_pem: &SecretString) -> Result<String, RefreshError> {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

    #[derive(Serialize)]
    struct Claims {
        iat: i64,
        exp: i64,
        iss: String,
    }

    let now = Utc::now();
    let iat = (now - ChronoDuration::seconds(60)).timestamp(); // clock skew tolerance
    let exp = (now + ChronoDuration::minutes(10)).timestamp();

    let claims = Claims {
        iat,
        exp,
        iss: app_id.to_string(),
    };

    // EncodingKey needs raw bytes; expose_secret briefly.
    let pem_bytes = private_key_pem.expose_secret().as_bytes();
    let key = EncodingKey::from_rsa_pem(pem_bytes).map_err(|_| RefreshError::InvalidKey)?;

    let mut header = Header::new(Algorithm::RS256);
    header.kid = None;

    encode(&header, &claims, &key).map_err(RefreshError::from)
}

#[derive(Deserialize)]
struct TokenExchangeResponse {
    token: String,
    expires_at: DateTime<Utc>,
}

// ─────────────────────────────────────────────────────────────────────────────
// L1 Refresh Coordinator (simplified Spec H0 L1 tier)
// ─────────────────────────────────────────────────────────────────────────────

/// In-process refresh coalescer — simplified L1 tier из Spec H0.
///
/// Maps credential_id → Arc<Mutex<()>>. Concurrent refresh for same credential
/// await same mutex; first winner refreshes, others reuse the fresh state.
///
/// **NOT** a full coordinator (no L2 durable claim, no heartbeat, no sentinel).
/// Demonstrates that in-process coalescing alone solves the single-replica case.
#[derive(Clone, Default)]
pub struct L1Coalescer {
    per_key: Arc<tokio::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl L1Coalescer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the per-key mutex и run `f`. If another task holds it, await then
    /// re-check если fresh (caller's responsibility to pass `is_fresh` closure).
    pub async fn coalesce<F, Fut>(
        &self,
        key: &str,
        is_fresh: impl FnOnce() -> bool,
        f: F,
    ) -> Result<(), RefreshError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), RefreshError>>,
    {
        let mutex = {
            let mut map = self.per_key.lock().await;
            map.entry(key.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };

        let _guard = mutex.lock().await;

        // After acquiring, re-check: maybe another task refreshed while we waited.
        if is_fresh() {
            return Ok(());
        }

        f().await
    }
}
