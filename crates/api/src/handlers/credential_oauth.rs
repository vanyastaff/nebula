//! OAuth controller endpoints (ADR-0031 rollout slice).

use std::future::Future;

use axum::{
    Json, Router,
    extract::{Extension, Form, Path, Query, State},
    routing::get,
};
use nebula_credential::{
    Credential, CredentialState, CredentialStore, OAuth2Credential, OAuth2State, PendingState,
    PendingStateStore, PendingStoreError, PutMode, SecretString, StoreError, StoredCredential,
    generate_code_challenge, generate_pkce_verifier, generate_random_state,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::{
    errors::{ApiError, ApiResult},
    middleware::auth::AuthenticatedUser,
    services::oauth::{
        flow::{
            AuthorizationUriRequest, TokenExchangeRequest, build_authorization_uri, exchange_code,
        },
        state::{OAuthStateSigner, build_signed_state},
    },
    state::AppState,
};

/// OAuth controller routes mounted under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/credentials/{id}/oauth2/auth",
            get(get_oauth2_authorize_url),
        )
        .route(
            "/credentials/{id}/oauth2/callback",
            get(get_oauth2_callback).post(post_oauth2_callback),
        )
}

/// Response with provider authorization URL and generated state.
#[derive(Debug, Serialize)]
pub struct AuthorizationUriResponse {
    /// URL where the browser should be redirected.
    pub authorize_url: String,
    /// Signed opaque OAuth state.
    pub state: String,
}

/// GET `/credentials/{id}/oauth2/auth`
pub async fn get_oauth2_authorize_url(
    Path(credential_id): Path<String>,
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<AuthorizationUriRequest>,
) -> ApiResult<Json<AuthorizationUriResponse>> {
    let csrf = generate_random_state();
    let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
    let (signed_state, payload) = build_signed_state(&signer, &credential_id, csrf)
        .map_err(|e| ApiError::Internal(format!("state generation failed: {e}")))?;

    let code_verifier = generate_pkce_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);

    let pending = OAuthPendingExchange {
        token_url: query.token_url.clone(),
        client_id: query.client_id.clone(),
        client_secret: SecretString::new(query.client_secret.clone()),
        redirect_uri: query.redirect_uri.clone(),
        code_verifier: SecretString::new(code_verifier),
        scopes: query
            .scopes
            .as_deref()
            .map(|raw| raw.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        auth_style: query
            .auth_style
            .unwrap_or(nebula_credential::credentials::oauth2::AuthStyle::Header),
    };
    let pending_token = state
        .oauth_pending_store
        .put("oauth2", &user.user_id, &payload.csrf_token, pending)
        .await
        .map_err(|e| ApiError::Internal(format!("pending oauth state store failed: {e}")))?;
    state
        .oauth_state_tokens
        .write()
        .await
        .insert(signed_state.clone(), pending_token);

    let authorize_url =
        build_authorization_uri(&query, &signed_state, &code_challenge).map_err(|e| {
            ApiError::Validation {
                detail: format!("invalid authorization url: {e}"),
                errors: Vec::new(),
            }
        })?;

    Ok(Json(AuthorizationUriResponse {
        authorize_url: authorize_url.to_string(),
        state: signed_state,
    }))
}

/// GET callback params.
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    /// Authorization code.
    pub code: String,
    /// Signed opaque state.
    pub state: String,
}

/// POST callback body (for form_post response mode).
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackBody {
    /// Authorization code.
    pub code: String,
    /// Signed opaque state.
    pub state: String,
}

/// Callback response while persistence wiring is still in rollout.
#[derive(Debug, Serialize)]
pub struct OAuthCallbackResponse {
    /// Credential id from route.
    pub credential_id: String,
    /// True when callback state verified and code exchange succeeded.
    pub exchanged: bool,
}

/// GET `/credentials/{id}/oauth2/callback`
pub async fn get_oauth2_callback(
    Path(credential_id): Path<String>,
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<OAuthCallbackQuery>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    handle_callback(&credential_id, &state, &user, query.code, query.state).await
}

/// POST `/credentials/{id}/oauth2/callback`
///
/// Accepts `application/x-www-form-urlencoded` bodies (`form_post` response mode).
pub async fn post_oauth2_callback(
    Path(credential_id): Path<String>,
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(body): Form<OAuthCallbackBody>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    handle_callback(&credential_id, &state, &user, body.code, body.state).await
}

async fn handle_callback(
    credential_id: &str,
    state: &AppState,
    user: &AuthenticatedUser,
    code: String,
    signed_state: String,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    handle_callback_with_exchange(
        credential_id,
        state,
        user,
        code,
        signed_state,
        |req| async move { exchange_code(&req).await },
    )
    .await
}

/// When [`PendingStateStore::consume`] fails, decide whether to drop the in-memory
/// `signed_state` → pending-token entry.
///
/// If the pending row is already gone (`NotFound` / race loser), expired, or violated
/// single-use semantics, keeping the map entry only blocks retries with a misleading
/// "already consumed" on the **map** side. If validation failed (wrong user/session),
/// the row remains for the legitimate caller — do not remove.
fn should_drop_oauth_state_map_entry(err: &PendingStoreError) -> bool {
    matches!(
        err,
        PendingStoreError::NotFound
            | PendingStoreError::Expired
            | PendingStoreError::AlreadyConsumed
    )
}

async fn handle_callback_with_exchange<F, Fut>(
    credential_id: &str,
    state: &AppState,
    user: &AuthenticatedUser,
    code: String,
    signed_state: String,
    exchange_fn: F,
) -> ApiResult<Json<OAuthCallbackResponse>>
where
    F: Fn(TokenExchangeRequest) -> Fut,
    Fut: Future<Output = Result<serde_json::Value, String>>,
{
    let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
    let payload = signer
        .verify_for_credential(&signed_state, credential_id)
        .map_err(|e| ApiError::Unauthorized(format!("oauth state validation failed: {e}")))?;

    // Look up the opaque pending token without removing it yet: `consume` must succeed
    // before we drop the one-time map entry, otherwise a failed consume (wrong user,
    // transient store error) would strand the user with "already consumed" on retry.
    let pending_token = state
        .oauth_state_tokens
        .read()
        .await
        .get(&signed_state)
        .cloned()
        .ok_or_else(|| {
            ApiError::Unauthorized("oauth state not found or already consumed".to_owned())
        })?;

    let consume_result = state
        .oauth_pending_store
        .consume::<OAuthPendingExchange>(
            "oauth2",
            &pending_token,
            &user.user_id,
            &payload.csrf_token,
        )
        .await;

    let pending = match consume_result {
        Ok(p) => p,
        Err(e) => {
            if should_drop_oauth_state_map_entry(&e) {
                state.oauth_state_tokens.write().await.remove(&signed_state);
            }
            return Err(ApiError::Unauthorized(format!(
                "oauth pending consume failed: {e}"
            )));
        },
    };

    state.oauth_state_tokens.write().await.remove(&signed_state);

    let token_exchange = TokenExchangeRequest {
        token_url: pending.token_url.clone(),
        client_id: pending.client_id.clone(),
        client_secret: pending.client_secret.expose_secret().to_owned(),
        code,
        redirect_uri: pending.redirect_uri.clone(),
        code_verifier: pending.code_verifier.expose_secret().to_owned(),
        auth_style: pending.auth_style,
    };
    let token_body = exchange_fn(token_exchange)
        .await
        .map_err(ApiError::Internal)?;
    let oauth_state = build_oauth2_state(&token_body, &pending)?;
    persist_oauth_state(state, credential_id, oauth_state).await?;

    Ok(Json(OAuthCallbackResponse {
        credential_id: credential_id.to_owned(),
        exchanged: true,
    }))
}

/// Pending data required to exchange authorization code in callback.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuthPendingExchange {
    token_url: String,
    client_id: String,
    #[serde(with = "nebula_credential::serde_secret")]
    client_secret: SecretString,
    redirect_uri: String,
    #[serde(with = "nebula_credential::serde_secret")]
    code_verifier: SecretString,
    scopes: Vec<String>,
    auth_style: nebula_credential::credentials::oauth2::AuthStyle,
}

impl Zeroize for OAuthPendingExchange {
    fn zeroize(&mut self) {
        self.token_url.zeroize();
        self.client_id.zeroize();
        self.client_secret.zeroize();
        self.redirect_uri.zeroize();
        self.code_verifier.zeroize();
        self.scopes.zeroize();
    }
}

impl PendingState for OAuthPendingExchange {
    const KIND: &'static str = "api_oauth_pending";

    fn expires_in(&self) -> std::time::Duration {
        std::time::Duration::from_mins(10)
    }
}

fn build_oauth2_state(
    token_body: &serde_json::Value,
    pending: &OAuthPendingExchange,
) -> Result<OAuth2State, ApiError> {
    let access_token = token_body
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ApiError::Validation {
            detail: "token response missing required access_token".to_owned(),
            errors: Vec::new(),
        })?;

    let refresh_token = token_body
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .map(SecretString::new);
    let token_type = token_body
        .get("token_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Bearer")
        .to_owned();
    let expires_at = token_body
        .get("expires_in")
        .and_then(serde_json::Value::as_u64)
        .map(|secs| chrono::Utc::now() + chrono::Duration::seconds(secs as i64));
    let scopes = token_body
        .get("scope")
        .and_then(serde_json::Value::as_str)
        .map(|raw| raw.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_else(|| pending.scopes.clone());

    Ok(OAuth2State {
        access_token: SecretString::new(access_token),
        token_type,
        refresh_token,
        expires_at,
        scopes,
        client_id: SecretString::new(pending.client_id.clone()),
        client_secret: pending.client_secret.clone(),
        token_url: pending.token_url.clone(),
        auth_style: pending.auth_style,
    })
}

async fn persist_oauth_state(
    state: &AppState,
    credential_id: &str,
    oauth_state: OAuth2State,
) -> ApiResult<()> {
    let data = serde_json::to_vec(&oauth_state).map_err(|e| {
        ApiError::Internal(format!(
            "failed to serialize oauth state for persistence: {e}"
        ))
    })?;
    let now = chrono::Utc::now();
    let (created_at, metadata) = match state.oauth_credential_store.get(credential_id).await {
        Ok(existing) => (existing.created_at, existing.metadata),
        Err(StoreError::NotFound { .. }) => (now, serde_json::Map::new()),
        Err(e) => {
            return Err(ApiError::Internal(format!(
                "failed to read existing oauth credential: {e}"
            )));
        },
    };
    let stored = StoredCredential {
        id: credential_id.to_owned(),
        credential_key: OAuth2Credential::KEY.to_owned(),
        data,
        state_kind: OAuth2State::KIND.to_owned(),
        state_version: OAuth2State::VERSION,
        version: 0,
        created_at,
        updated_at: now,
        expires_at: oauth_state.expires_at(),
        metadata,
    };

    state
        .oauth_credential_store
        .put(stored, PutMode::Overwrite)
        .await
        .map_err(|e| {
            ApiError::Internal(format!("failed to persist oauth credential state: {e}"))
        })?;
    Ok(())
}

#[cfg(test)]
// `SecretString::expose_secret` is HRTB; `|s| s.to_owned()` is the correct pattern.
#[allow(clippy::redundant_closure_for_method_calls)]
mod tests {
    use std::sync::Arc;

    use nebula_credential::{CredentialStore, PendingStateStore};
    use nebula_storage::{
        InMemoryExecutionRepo, InMemoryWorkflowRepo,
        repos::{ControlQueueRepo, InMemoryControlQueueRepo},
    };

    use super::*;
    use crate::config::JwtSecret;

    fn test_app_state() -> AppState {
        let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
        let execution_repo = Arc::new(InMemoryExecutionRepo::new());
        let control_queue_repo: Arc<dyn ControlQueueRepo> =
            Arc::new(InMemoryControlQueueRepo::new());
        let jwt_secret =
            JwtSecret::new("test-jwt-secret-1234567890-abcdef").expect("valid test secret");
        AppState::new(
            workflow_repo,
            execution_repo,
            control_queue_repo,
            jwt_secret,
        )
    }

    fn test_pending_exchange() -> OAuthPendingExchange {
        OAuthPendingExchange {
            token_url: "https://provider.example.com/token".to_owned(),
            client_id: "client-id".to_owned(),
            client_secret: SecretString::new("client-secret"),
            redirect_uri: "https://app.example.com/callback".to_owned(),
            code_verifier: SecretString::new("pkce-verifier"),
            scopes: vec!["read".to_owned(), "write".to_owned()],
            auth_style: nebula_credential::credentials::oauth2::AuthStyle::Header,
        }
    }

    #[tokio::test]
    async fn callback_persists_oauth_state_in_credential_store() {
        let state = test_app_state();
        let credential_id = "cred-123";
        let user = AuthenticatedUser {
            user_id: "user-123".to_owned(),
        };

        let csrf = generate_random_state();
        let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
        let (signed_state, payload) =
            build_signed_state(&signer, credential_id, csrf).expect("signed state");
        let pending = test_pending_exchange();
        let pending_token = state
            .oauth_pending_store
            .put("oauth2", &user.user_id, &payload.csrf_token, pending)
            .await
            .expect("pending store put");
        state
            .oauth_state_tokens
            .write()
            .await
            .insert(signed_state.clone(), pending_token);

        let token_body = serde_json::json!({
            "access_token": "access-token-value",
            "refresh_token": "refresh-token-value",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "scope-a scope-b"
        });
        let callback = handle_callback_with_exchange(
            credential_id,
            &state,
            &user,
            "auth-code".to_owned(),
            signed_state,
            move |_req| {
                let token_body = token_body.clone();
                async move { Ok(token_body) }
            },
        )
        .await
        .expect("callback handled");
        assert!(callback.0.exchanged);

        let stored = state
            .oauth_credential_store
            .get(credential_id)
            .await
            .expect("stored oauth credential");
        assert_eq!(stored.state_kind, OAuth2State::KIND);
        assert_eq!(stored.credential_key, OAuth2Credential::KEY);

        let persisted_state: OAuth2State =
            serde_json::from_slice(&stored.data).expect("oauth state json");
        assert_eq!(
            persisted_state.access_token.expose_secret().to_owned(),
            "access-token-value"
        );
        assert_eq!(
            persisted_state
                .refresh_token
                .expect("refresh token")
                .expose_secret()
                .to_owned(),
            "refresh-token-value"
        );
        assert_eq!(persisted_state.scopes, vec!["scope-a", "scope-b"]);
    }

    #[tokio::test]
    async fn callback_overwrites_existing_oauth_state() {
        let state = test_app_state();
        let credential_id = "cred-456";
        let user = AuthenticatedUser {
            user_id: "user-456".to_owned(),
        };

        let old_oauth_state = OAuth2State {
            access_token: SecretString::new("old-access-token"),
            token_type: "Bearer".to_owned(),
            refresh_token: Some(SecretString::new("old-refresh-token")),
            expires_at: None,
            scopes: vec!["old-scope".to_owned()],
            client_id: SecretString::new("old-client-id"),
            client_secret: SecretString::new("old-client-secret"),
            token_url: "https://provider.example.com/token".to_owned(),
            auth_style: nebula_credential::credentials::oauth2::AuthStyle::Header,
        };
        persist_oauth_state(&state, credential_id, old_oauth_state)
            .await
            .expect("seed existing oauth state");
        let first_created_at = state
            .oauth_credential_store
            .get(credential_id)
            .await
            .expect("seeded credential")
            .created_at;

        let csrf = generate_random_state();
        let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
        let (signed_state, payload) =
            build_signed_state(&signer, credential_id, csrf).expect("signed state");
        let pending = test_pending_exchange();
        let pending_token = state
            .oauth_pending_store
            .put("oauth2", &user.user_id, &payload.csrf_token, pending)
            .await
            .expect("pending store put");
        state
            .oauth_state_tokens
            .write()
            .await
            .insert(signed_state.clone(), pending_token);

        let token_body = serde_json::json!({
            "access_token": "new-access-token",
            "refresh_token": "new-refresh-token",
            "token_type": "Bearer",
            "expires_in": 900,
            "scope": "new-scope-a new-scope-b"
        });
        let callback = handle_callback_with_exchange(
            credential_id,
            &state,
            &user,
            "auth-code".to_owned(),
            signed_state,
            move |_req| {
                let token_body = token_body.clone();
                async move { Ok(token_body) }
            },
        )
        .await
        .expect("callback handled");
        assert!(callback.0.exchanged);

        let stored = state
            .oauth_credential_store
            .get(credential_id)
            .await
            .expect("stored oauth credential");
        assert_eq!(stored.state_kind, OAuth2State::KIND);
        assert_eq!(stored.credential_key, OAuth2Credential::KEY);
        assert_eq!(stored.version, 2);
        assert_eq!(
            stored.created_at, first_created_at,
            "overwrite must preserve created_at"
        );

        let persisted_state: OAuth2State =
            serde_json::from_slice(&stored.data).expect("oauth state json");
        assert_eq!(
            persisted_state.access_token.expose_secret().to_owned(),
            "new-access-token"
        );
        assert_eq!(
            persisted_state
                .refresh_token
                .expect("refresh token")
                .expose_secret()
                .to_owned(),
            "new-refresh-token"
        );
        assert_eq!(persisted_state.scopes, vec!["new-scope-a", "new-scope-b"]);
    }
}
