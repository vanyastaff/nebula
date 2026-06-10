//! OAuth controller endpoints (API-owned OAuth flow rollout slice).

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
use utoipa::{IntoParams, ToSchema};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{
    error::{ApiError, ApiResult},
    middleware::auth::AuthenticatedUser,
    state::AppState,
    transport::oauth::{
        flow::{
            AuthorizationUriRequest, TokenExchangeRequest, build_authorization_uri,
            validate_oauth_outbound_url,
        },
        state::{OAuthStateSigner, build_signed_state},
    },
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
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorizationUriResponse {
    /// URL where the browser should be redirected.
    pub authorize_url: String,
    /// Signed opaque OAuth state.
    pub state: String,
}

/// GET `/credentials/{id}/oauth2/auth`
pub async fn get_oauth2_authorize_url(
    Path(_credential_id): Path<String>,
    State(_state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Query(_query): Query<AuthorizationUriRequest>,
) -> ApiResult<Json<AuthorizationUriResponse>> {
    Err(ApiError::Gone(
        "OAuth credential flow must use workspace-scoped routes".to_owned(),
    ))
}

/// Build an OAuth2 authorization URL and persist tenant-bound pending state.
///
/// The credential placeholder is created/read through the owner-scoped
/// facade store and the pending exchange records the expected credential
/// version for callback-time CAS persistence. `owner_id` is mandatory —
/// every OAuth credential acquisition is tenant-bound (the system-level
/// unscoped routes are permanently disabled).
pub async fn get_oauth2_authorize_url_for_owner(
    credential_id: &str,
    state: &AppState,
    user: &AuthenticatedUser,
    query: AuthorizationUriRequest,
    owner_id: String,
) -> ApiResult<Json<AuthorizationUriResponse>> {
    validate_oauth_outbound_url(&query.token_url).map_err(ApiError::validation_message)?;
    prepare_oauth_credential(state, &owner_id, credential_id).await?;

    let csrf = generate_random_state();
    let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
    let (signed_state, payload) = build_signed_state(&signer, credential_id, csrf)
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
            .unwrap_or(nebula_credential::AuthStyle::Header),
        owner_id,
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
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OAuthCallbackQuery {
    /// Authorization code.
    pub code: String,
    /// Signed opaque state.
    pub state: String,
}

/// POST callback body (for form_post response mode).
#[derive(Debug, Deserialize, ToSchema)]
pub struct OAuthCallbackBody {
    /// Authorization code.
    pub code: String,
    /// Signed opaque state.
    pub state: String,
}

/// Callback response while persistence wiring is still in rollout.
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthCallbackResponse {
    /// Credential id from route.
    pub credential_id: String,
    /// True when callback state verified and code exchange succeeded.
    pub exchanged: bool,
}

/// GET `/credentials/{id}/oauth2/callback`
pub async fn get_oauth2_callback(
    Path(_credential_id): Path<String>,
    State(_state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Query(_query): Query<OAuthCallbackQuery>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    Err(ApiError::Gone(
        "OAuth credential flow must use workspace-scoped routes".to_owned(),
    ))
}

/// POST `/credentials/{id}/oauth2/callback`
///
/// Accepts `application/x-www-form-urlencoded` bodies (`form_post` response mode).
pub async fn post_oauth2_callback(
    Path(_credential_id): Path<String>,
    State(_state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Form(_body): Form<OAuthCallbackBody>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    Err(ApiError::Gone(
        "OAuth credential flow must use workspace-scoped routes".to_owned(),
    ))
}

/// Consume tenant-bound OAuth pending state, exchange the code, and persist tokens.
///
/// The pending state owner must match `owner_id`; callbacks sent to the wrong
/// workspace fail before token exchange and before credential persistence.
pub async fn handle_callback_for_owner<F, Fut>(
    credential_id: &str,
    state: &AppState,
    user: &AuthenticatedUser,
    code: String,
    signed_state: String,
    owner_id: String,
    exchange_fn: F,
) -> ApiResult<Json<OAuthCallbackResponse>>
where
    F: Fn(TokenExchangeRequest) -> Fut,
    Fut: Future<Output = Result<serde_json::Value, String>>,
{
    handle_callback_with_exchange(
        credential_id,
        state,
        user,
        code,
        signed_state,
        owner_id,
        exchange_fn,
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
    owner_id: String,
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

    let pending_for_owner_check = state
        .oauth_pending_store
        .get_bound::<OAuthPendingExchange>(
            "oauth2",
            &pending_token,
            &user.user_id,
            &payload.csrf_token,
        )
        .await;
    let pending_for_owner_check = match pending_for_owner_check {
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
    if pending_for_owner_check.owner_id != owner_id {
        return Err(ApiError::Unauthorized(
            "oauth pending state tenant mismatch".to_owned(),
        ));
    }

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
    persist_oauth_state(state, &owner_id, credential_id, oauth_state).await?;

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
    auth_style: nebula_credential::AuthStyle,
    owner_id: String,
}

impl Zeroize for OAuthPendingExchange {
    fn zeroize(&mut self) {
        self.token_url.zeroize();
        self.client_id.zeroize();
        self.client_secret.zeroize();
        self.redirect_uri.zeroize();
        self.code_verifier.zeroize();
        self.scopes.zeroize();
        self.owner_id.zeroize();
    }
}

// Per Tech Spec §15.4 — `PendingState: ZeroizeOnDrop`. Hand-rolled
// because the manual `Zeroize` body above would conflict with a derived
// `Drop`; this delegates Drop to the existing zeroize logic so the
// deterministic-drop guarantee covers the API-side OAuth2 pending
// exchange (carries the PKCE verifier, client secret, and redirect URI).
impl Drop for OAuthPendingExchange {
    fn drop(&mut self) {
        self.zeroize();
    }
}
impl ZeroizeOnDrop for OAuthPendingExchange {}

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
    owner_id: &str,
    credential_id: &str,
    oauth_state: OAuth2State,
) -> ApiResult<()> {
    let data = serde_json::to_vec(&oauth_state).map_err(|e| {
        ApiError::Internal(format!(
            "failed to serialize oauth state for persistence: {e}"
        ))
    })?;
    let now = chrono::Utc::now();
    let store = crate::transport::credential::scoped_store(state, owner_id)?;
    // CAS on the version read HERE, not on a version pinned at authorize
    // time: a legitimate concurrent write (a rename while the user sat on
    // the IdP consent screen) must not burn the single-use authorization
    // code and the consumed pending state. Double-exchange is already
    // prevented one layer up by the pending state''s single-use consume.
    let existing = store
        .get(credential_id)
        .await
        .map_err(|e| map_oauth_store_err(e, credential_id))?;
    let stored = StoredCredential {
        id: credential_id.to_owned(),
        name: existing.name,
        credential_key: OAuth2Credential::KEY.to_owned(),
        data,
        state_kind: OAuth2State::KIND.to_owned(),
        state_version: OAuth2State::VERSION,
        version: 0,
        created_at: existing.created_at,
        updated_at: now,
        expires_at: oauth_state.expires_at(),
        // The exchange just minted fresh tokens — the placeholder's
        // `reauth_required` flag is cleared by this write.
        reauth_required: false,
        metadata: existing.metadata,
    };

    store
        .put(
            stored,
            PutMode::CompareAndSwap {
                expected_version: existing.version,
            },
        )
        .await
        .map_err(|e| map_oauth_store_err(e, credential_id))?;
    Ok(())
}

async fn prepare_oauth_credential(
    state: &AppState,
    owner_id: &str,
    credential_id: &str,
) -> ApiResult<()> {
    let store = crate::transport::credential::scoped_store(state, owner_id)?;
    match store.get(credential_id).await {
        Ok(existing) => {
            if existing.credential_key != OAuth2Credential::KEY
                || existing.state_kind != OAuth2State::KIND
            {
                return Err(ApiError::validation_message(
                    "credential is not an OAuth2 credential",
                ));
            }
            Ok(())
        },
        Err(StoreError::NotFound { .. }) => {
            let now = chrono::Utc::now();
            let stored = StoredCredential {
                id: credential_id.to_owned(),
                name: None,
                credential_key: OAuth2Credential::KEY.to_owned(),
                data: Vec::new(),
                state_kind: OAuth2State::KIND.to_owned(),
                state_version: OAuth2State::VERSION,
                version: 0,
                created_at: now,
                updated_at: now,
                expires_at: None,
                reauth_required: true,
                metadata: serde_json::Map::new(),
            };
            store
                .put(stored, PutMode::CreateOnly)
                .await
                .map(|_| ())
                .map_err(|e| map_oauth_store_err(e, credential_id))
        },
        Err(e) => Err(map_oauth_store_err(e, credential_id)),
    }
}

fn map_oauth_store_err(err: StoreError, credential_id: &str) -> ApiError {
    match err {
        StoreError::NotFound { .. } | StoreError::AlreadyExists { .. } => {
            ApiError::NotFound(format!("credential {credential_id} not found"))
        },
        StoreError::VersionConflict { .. } => ApiError::Conflict(
            "OAuth credential changed while authorization was in progress".to_owned(),
        ),
        StoreError::AuditFailure(reason) => {
            ApiError::ServiceUnavailable(format!("credential audit sink unavailable: {reason}"))
        },
        StoreError::Backend(e) => {
            ApiError::Internal(format!("credential store backend error: {e}"))
        },
        _ => ApiError::Internal(format!("credential store error for {credential_id}")),
    }
}

#[cfg(test)]
// `SecretString::expose_secret` is HRTB; `|s| s.to_owned()` is the correct pattern.
#[allow(clippy::redundant_closure_for_method_calls)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::config::JwtSecret;
    use nebula_credential::{CredentialStore, PendingStateStore};
    use nebula_storage::credential::EnvKeyProvider;
    use nebula_storage::inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
        InMemoryNodeResultStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
    };

    fn test_app_state() -> AppState {
        let jwt_secret =
            JwtSecret::new("test-jwt-secret-1234567890-abcdef").expect("valid test secret");

        // Raw storage-port wiring (mirrors `server::default_state`):
        // undecorated in-memory adapters with a shared execution core so
        // the control queue / journal observe a `commit`'s rows. The
        // per-request tenant scope is applied by the `AppState` accessors.
        let exec_store = InMemoryExecutionStore::new();
        let control_queue = InMemoryControlQueue::new(&exec_store);
        let journal = InMemoryJournalReader::new(&exec_store);
        let workflow_versions = InMemoryWorkflowVersionStore::new();
        let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);

        let key = Arc::new(
            EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key fixture"),
        );
        let svc = crate::ports::credential_service_factory::with_key_provider(key)
            .expect("service composes");

        AppState::new(
            Arc::new(workflow_store),
            Arc::new(workflow_versions),
            Arc::new(exec_store),
            Arc::new(InMemoryNodeResultStore::new()),
            Arc::new(journal),
            Arc::new(control_queue),
            jwt_secret,
        )
        .with_credential_service(svc)
    }

    /// 32 `0x42` bytes, base64 — a valid AES-256 key fixture (mirrors the
    /// factory dev key). Not a secret: a fixed test constant.
    const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

    /// Canonical owner key for the test tenant.
    const OWNER_A: &str = "org-a:ws-a";

    fn test_pending_exchange(owner_id: &str) -> OAuthPendingExchange {
        OAuthPendingExchange {
            token_url: "https://provider.example.com/token".to_owned(),
            client_id: "client-id".to_owned(),
            client_secret: SecretString::new("client-secret"),
            redirect_uri: "https://app.example.com/callback".to_owned(),
            code_verifier: SecretString::new("pkce-verifier"),
            scopes: vec!["read".to_owned(), "write".to_owned()],
            auth_style: nebula_credential::AuthStyle::Header,
            owner_id: owner_id.to_owned(),
        }
    }

    /// Read a stored credential row back through the same owner-scoped
    /// facade store handle the OAuth path writes through.
    async fn read_stored(
        state: &AppState,
        owner_id: &str,
        credential_id: &str,
    ) -> StoredCredential {
        crate::transport::credential::scoped_store(state, owner_id)
            .expect("credential service is wired in test_app_state")
            .get(credential_id)
            .await
            .expect("stored oauth credential")
    }

    /// One full authorize→callback round against `credential_id` under
    /// `OWNER_A`, exchanging for the given token body.
    async fn run_callback_round(
        state: &AppState,
        user: &AuthenticatedUser,
        credential_id: &str,
        token_body: serde_json::Value,
    ) {
        prepare_oauth_credential(state, OWNER_A, credential_id)
            .await
            .expect("placeholder prepared");

        let csrf = generate_random_state();
        let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
        let (signed_state, payload) =
            build_signed_state(&signer, credential_id, csrf).expect("signed state");
        let pending = test_pending_exchange(OWNER_A);
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

        let callback = handle_callback_with_exchange(
            credential_id,
            state,
            user,
            "auth-code".to_owned(),
            signed_state,
            OWNER_A.to_owned(),
            move |_req| {
                let token_body = token_body.clone();
                async move { Ok(token_body) }
            },
        )
        .await
        .expect("callback handled");
        assert!(callback.0.exchanged);
    }

    #[tokio::test]
    async fn callback_persists_oauth_state_in_credential_store() {
        let state = test_app_state();
        let credential_id = "cred-123";
        let user = AuthenticatedUser {
            user_id: "user-123".to_owned(),
        };

        let token_body = serde_json::json!({
            "access_token": "access-token-value",
            "refresh_token": "refresh-token-value",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "scope-a scope-b"
        });
        run_callback_round(&state, &user, credential_id, token_body).await;

        let stored = read_stored(&state, OWNER_A, credential_id).await;
        assert_eq!(stored.state_kind, OAuth2State::KIND);
        assert_eq!(stored.credential_key, OAuth2Credential::KEY);
        assert!(
            !stored.reauth_required,
            "the exchange must clear the placeholder reauth flag"
        );

        let persisted_state: OAuth2State =
            serde_json::from_slice(&stored.data).expect("oauth state json");
        assert_eq!(
            persisted_state.access_token.expose_secret().to_owned(),
            "access-token-value"
        );
        assert_eq!(
            persisted_state
                .refresh_token
                .clone()
                .expect("refresh token")
                .expose_secret()
                .to_owned(),
            "refresh-token-value"
        );
        assert_eq!(persisted_state.scopes, vec!["scope-a", "scope-b"]);
    }

    #[tokio::test]
    async fn second_callback_round_replaces_tokens_and_preserves_created_at() {
        let state = test_app_state();
        let credential_id = "cred-456";
        let user = AuthenticatedUser {
            user_id: "user-456".to_owned(),
        };

        run_callback_round(
            &state,
            &user,
            credential_id,
            serde_json::json!({
                "access_token": "old-access-token",
                "token_type": "Bearer",
            }),
        )
        .await;
        let first = read_stored(&state, OWNER_A, credential_id).await;

        run_callback_round(
            &state,
            &user,
            credential_id,
            serde_json::json!({
                "access_token": "new-access-token",
                "refresh_token": "new-refresh-token",
                "token_type": "Bearer",
                "expires_in": 900,
                "scope": "new-scope-a new-scope-b"
            }),
        )
        .await;

        let stored = read_stored(&state, OWNER_A, credential_id).await;
        assert_eq!(stored.state_kind, OAuth2State::KIND);
        assert_eq!(stored.credential_key, OAuth2Credential::KEY);
        assert!(
            stored.version > first.version,
            "the second exchange must CAS over the first ({} -> {})",
            first.version,
            stored.version
        );
        assert_eq!(
            stored.created_at, first.created_at,
            "re-authorization must preserve created_at"
        );

        let persisted_state: OAuth2State =
            serde_json::from_slice(&stored.data).expect("oauth state json");
        assert_eq!(
            persisted_state.access_token.expose_secret().to_owned(),
            "new-access-token"
        );
        assert_eq!(persisted_state.scopes, vec!["new-scope-a", "new-scope-b"]);
    }

    /// The OAuth-acquired row is visible to the generic CRUD plane (one
    /// store): `facade.get`/`list` see it under the same owner, and a
    /// foreign owner sees nothing.
    #[tokio::test]
    async fn oauth_row_is_visible_to_the_crud_plane() {
        let state = test_app_state();
        let credential_id = "cred-crud-visible";
        let user = AuthenticatedUser {
            user_id: "user-vis".to_owned(),
        };

        run_callback_round(
            &state,
            &user,
            credential_id,
            serde_json::json!({
                "access_token": "vis-access-token",
                "token_type": "Bearer",
            }),
        )
        .await;

        // OWNER_A is an opaque owner string for the scoped store; assert
        // via the scoped store that a *different* owner cannot read the
        // row (flat NotFound) while the owning scope still can.
        let foreign = crate::transport::credential::scoped_store(&state, "org-b:ws-b")
            .expect("service wired");
        let err = foreign
            .get(credential_id)
            .await
            .expect_err("foreign owner must not read the oauth row");
        assert!(matches!(err, StoreError::NotFound { .. }));
        // And the owning scope still reads it.
        let stored = read_stored(&state, OWNER_A, credential_id).await;
        assert_eq!(stored.credential_key, OAuth2Credential::KEY);
    }

    /// A legitimate concurrent write while the user sits on the IdP
    /// consent screen (e.g. a teammate renames the credential, bumping
    /// the row version) must NOT burn the single-use authorization code:
    /// the callback CASes on the version it reads at exchange time, not
    /// on a version pinned at authorize time.
    #[tokio::test]
    async fn concurrent_row_write_between_authorize_and_callback_does_not_break_exchange() {
        let state = test_app_state();
        let credential_id = "cred-rename-race";
        let user = AuthenticatedUser {
            user_id: "user-race".to_owned(),
        };

        prepare_oauth_credential(&state, OWNER_A, credential_id)
            .await
            .expect("placeholder prepared");

        let csrf = generate_random_state();
        let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
        let (signed_state, payload) =
            build_signed_state(&signer, credential_id, csrf).expect("signed state");
        let pending = test_pending_exchange(OWNER_A);
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

        // Concurrent write lands while the consent screen is open: bump
        // the row version via an owner-scoped CAS write.
        let store =
            crate::transport::credential::scoped_store(&state, OWNER_A).expect("service wired");
        let existing = store
            .get(credential_id)
            .await
            .expect("placeholder readable");
        let bumped = StoredCredential {
            updated_at: chrono::Utc::now(),
            ..existing.clone()
        };
        store
            .put(
                bumped,
                PutMode::CompareAndSwap {
                    expected_version: existing.version,
                },
            )
            .await
            .expect("concurrent write succeeds");

        // The callback still completes: the exchange CASes on the version
        // it reads now, not the authorize-time one.
        let token_body = serde_json::json!({
            "access_token": "race-access-token",
            "token_type": "Bearer",
        });
        let callback = handle_callback_with_exchange(
            credential_id,
            &state,
            &user,
            "auth-code".to_owned(),
            signed_state,
            OWNER_A.to_owned(),
            move |_req| {
                let token_body = token_body.clone();
                async move { Ok(token_body) }
            },
        )
        .await
        .expect("callback survives the concurrent rename");
        assert!(callback.0.exchanged);

        let stored = read_stored(&state, OWNER_A, credential_id).await;
        assert!(!stored.reauth_required, "exchange clears the reauth flag");
        let persisted_state: OAuth2State =
            serde_json::from_slice(&stored.data).expect("oauth state json");
        assert_eq!(
            persisted_state.access_token.expose_secret().to_owned(),
            "race-access-token"
        );
    }
    #[tokio::test]
    async fn tenant_mismatch_does_not_consume_pending_state() {
        let state = test_app_state();
        let credential_id = "cred-tenant-mismatch";
        let user = AuthenticatedUser {
            user_id: "user-tenant-mismatch".to_owned(),
        };

        prepare_oauth_credential(&state, OWNER_A, credential_id)
            .await
            .expect("placeholder prepared");

        let csrf = generate_random_state();
        let signer = OAuthStateSigner::new(state.jwt_secret.as_bytes());
        let (signed_state, payload) =
            build_signed_state(&signer, credential_id, csrf).expect("signed state");
        let pending = test_pending_exchange(OWNER_A);
        let pending_token = state
            .oauth_pending_store
            .put("oauth2", &user.user_id, &payload.csrf_token, pending)
            .await
            .expect("pending store put");
        let pending_token_for_assert = pending_token.clone();
        state
            .oauth_state_tokens
            .write()
            .await
            .insert(signed_state.clone(), pending_token);

        let err = handle_callback_with_exchange(
            credential_id,
            &state,
            &user,
            "auth-code".to_owned(),
            signed_state.clone(),
            "org-b:ws-b".to_owned(),
            |_req| async { Err::<serde_json::Value, String>("exchange should not run".to_owned()) },
        )
        .await
        .expect_err("tenant mismatch must fail");
        assert!(
            matches!(err, ApiError::Unauthorized(ref message) if message == "oauth pending state tenant mismatch"),
            "expected tenant mismatch, got: {err:?}"
        );

        let still_pending = state
            .oauth_pending_store
            .get_bound::<OAuthPendingExchange>(
                "oauth2",
                &pending_token_for_assert,
                &user.user_id,
                &payload.csrf_token,
            )
            .await
            .expect("tenant mismatch must leave pending state retryable");
        assert_eq!(still_pending.owner_id, OWNER_A);
        assert!(
            state
                .oauth_state_tokens
                .read()
                .await
                .contains_key(&signed_state),
            "tenant mismatch must leave signed state mapping retryable"
        );
    }
}
