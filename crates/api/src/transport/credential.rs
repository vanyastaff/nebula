//! Credential service layer — business logic for credential operations.
//!
//! Each function takes an `AppState` reference plus domain-specific parameters
//! and returns `ApiResult`.
//!
//! ## honest capability honesty split (Phase 4)
//!
//! The CRUD subset (`create` / `get` / `update` / `delete` / `list`)
//! persists through `scoped_store` (crate-private): the raw
//! `InMemoryStore` (`AppState::oauth_credential_store`) wrapped by
//! `CredentialScopeLayer`, which stamps/checks the tenant owner on every
//! call. The OAuth2 callback path writes through `scoped_store` the same
//! way (no type dispatch).
//!
//! The lifecycle / acquisition / type-discovery functions stay **honest
//! 503** (`ApiError::ServiceUnavailable`): `test` / `refresh` / `revoke`
//! / `resolve` / `continue_resolve` need a `CredentialRegistry` to
//! dispatch a concrete `Credential` and an engine-owned resolver/refresh
//! orchestrator (`nebula-engine::credential`, engine refresh orchestration — see
//! `docs/MATURITY.md` "engine-owned `credential` runtime surface").
//! Neither is wired into `AppState`, so faking success would be a
//! honest capability false capability (the worst class for a credential surface).
//!
//! Note: the generic `resolve_credential` / `continue_resolve`
//! functions are honest-503 at the function boundary, and their routes
//! **reach the handler**: `crate::middleware::tenancy::resolve_path_ids`
//! special-cases the literal `resolve` sub-route so it is not parsed as
//! a `{cred}` `CredentialId` (an earlier route-shadow returned a flat
//! 404 *before* the handler — fixed). A request to
//! `…/credentials/resolve[/continue]` therefore surfaces this honest
//! **503** (no false success — the caller cannot obtain a fake
//! credential); the genuine `…/credentials/{cred}` position stays
//! strictly ULID-validated. The honest-503 is pinned by the unit test
//! below and the `credential_e2e` route-reachability regression guard.
//!
//! ## credential secrecy secret handling
//!
//! - The credential `data` blob arrives **write-only**: it is wrapped in
//!   [`nebula_credential::SecretString`] for its in-process lifetime and
//!   persisted via the `serde_secret` (write-only; encrypted at rest
//!   **only when an `EncryptionLayer` is composed** — not wired here)
//!   path into the opaque [`StoredCredential::data`] byte buffer.
//! - The wire response types ([`CredentialResponse`] /
//!   [`CredentialSummary`]) are **metadata-only** — they have no `data`
//!   field, so `get` / `list` cannot structurally echo the secret.
//! - Errors carry the credential id only; no secret reaches an
//!   `ApiError` / `ProblemDetails`. Tracing spans log `cred.id` /
//!   `cred.key` only.
//!
//! Durability is process-local (in-memory store) — encryption at rest
//! arrives with the typed credential facade in a follow-up; see the
//! credential durability note in `crates/api/README.md`.
//!
//! ## Workspace isolation
//!
//! Workspace handlers derive an owner id from the resolved tenant scope.
//! `scoped_store`'s `CredentialScopeLayer` stamps `metadata["owner_id"]`
//! on write and checks it on every read, update, and delete, so credential
//! IDs remain non-dereferenceable across workspaces.

use nebula_credential::{
    CredentialStore, PutMode, ScopeResolver, SecretString, StoreError, StoredCredential,
};
use nebula_storage::credential::InMemoryStore;
use nebula_storage_port::Scope;
use nebula_tenancy::CredentialScopeLayer;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{
    domain::credential::dto::{
        ContinueResolveRequest, ContinueResolveResponse, CreateCredentialRequest,
        CredentialCapabilities, CredentialResponse, CredentialSummary, CredentialTypeInfo,
        ListCredentialTypesResponse, ListCredentialsQuery, ListCredentialsResponse,
        RefreshCredentialResponse, ResolveCredentialRequest, ResolveCredentialResponse,
        RevokeCredentialResponse, TestCredentialResponse, UpdateCredentialRequest,
    },
    error::{ApiError, ApiResult},
    state::AppState,
};

// ── Persisted record envelope ────────────────────────────────────────────────

/// Non-secret display metadata persisted alongside the credential.
///
/// Stored in [`StoredCredential::metadata`] (the plain JSON map) — never
/// secret. The secret `data` blob lives separately in
/// [`StoredCredential::data`] wrapped by [`PersistedSecretData`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CredentialMeta {
    credential_key: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
}

#[derive(Debug)]
struct RequestCredentialOwner(String);

impl ScopeResolver for RequestCredentialOwner {
    fn current_owner(&self) -> Option<&str> {
        Some(&self.0)
    }
}

pub(crate) fn owner_id_from_scope(scope: &Scope) -> String {
    // The single canonical derivation (ADR-0088 D7) — shared with the
    // credential-runtime plane so both key the same tenant identically. Was a
    // local `format!("{}:{}", …)` that drifted from the runtime's `/` form.
    scope.credential_owner_id()
}

/// Legacy alias — `oauth.rs` still uses this directly until OAuth is
/// migrated in a follow-up. Removed when Task 18 deletes `CredentialScopeLayer`.
pub(crate) fn scoped_store(
    state: &AppState,
    owner_id: &str,
) -> CredentialScopeLayer<InMemoryStore> {
    CredentialScopeLayer::new(
        state.oauth_credential_store.as_ref().clone(),
        Arc::new(RequestCredentialOwner(owner_id.to_owned())),
    )
}

/// Envelope for the type-specific secret input blob.
///
/// Wraps the serialized `data` JSON in [`SecretString`] so a stray
/// `Debug` / default `Serialize` redacts. The on-disk form uses the
/// `serde_secret` helper (write-only; encrypted at rest **only when an
/// `EncryptionLayer` is composed** — not wired here, so the in-memory
/// store keeps the raw bytes in plaintext-at-rest; see the operator
/// warning in `crates/api/README.md`). Production deployments wrap the
/// store with `nebula_storage`'s `EncryptionLayer` (storage credential layers).
#[derive(Serialize, Deserialize)]
struct PersistedSecretData {
    #[serde(with = "nebula_credential::serde_secret")]
    blob: SecretString,
}

impl std::fmt::Debug for PersistedSecretData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Defence-in-depth: even though `blob` is a `SecretString`
        // (already redacted), spell the redaction out so a future field
        // addition does not silently start leaking.
        f.debug_struct("PersistedSecretData")
            .field("blob", &"[REDACTED]")
            .finish()
    }
}

const STATE_KIND: &str = "api_managed_credential";
const STATE_VERSION: u32 = 1;

/// Serialize `data` into the opaque secret byte buffer.
///
/// The plaintext JSON string is held only inside this function and
/// dropped (zeroized via [`SecretString`]) before returning the bytes.
fn encode_secret_data(data: &serde_json::Value) -> ApiResult<Vec<u8>> {
    let plaintext = serde_json::to_string(data).map_err(|e| {
        // No secret in the message — only the serde shape failure.
        ApiError::Internal(format!("failed to encode credential data: {e}"))
    })?;
    let envelope = PersistedSecretData {
        blob: SecretString::new(plaintext),
    };
    serde_json::to_vec(&envelope)
        .map_err(|e| ApiError::Internal(format!("failed to encode credential envelope: {e}")))
}

/// Map a secret-safe [`CredentialFieldError`] list to the api-wide
/// validation status (400 — `ApiError::Validation`, consistent with every
/// other request-validation failure).
///
/// `CredentialFieldError` carries only an RFC-6901 path, a validator code,
/// and a static message — never the submitted value (credential redaction redaction;
/// credential-schema validation). The mapping introduces no value either.
fn credential_validation_error(
    errs: Vec<crate::ports::credential_schema::CredentialFieldError>,
) -> ApiError {
    let errors = errs
        .into_iter()
        .map(|e| crate::error::ValidationFieldError {
            code: e.code,
            detail: e.message,
            pointer: e.path,
        })
        .collect();
    ApiError::Validation {
        detail: "credential data failed schema validation".to_owned(),
        errors,
    }
}

/// credential-schema validation: validate credential `data` against the credential
/// type's resolved schema **before persist**. Authority sits with the
/// validator (invoked behind the [`CredentialSchemaPort`]). When no port
/// is configured the request is rejected with 503 — credential `data` is
/// **never** persisted unvalidated (closes the honest capability/§10 fail-open the
/// handler docstring previously mis-claimed was closed).
///
/// [`CredentialSchemaPort`]: crate::ports::credential_schema::CredentialSchemaPort
fn validate_credential_data(
    state: &AppState,
    credential_key: &str,
    data: &serde_json::Value,
) -> ApiResult<()> {
    match state.credential_schema.as_ref() {
        Some(port) => port
            .validate_data(credential_key, data)
            .map_err(credential_validation_error),
        None => Err(ApiError::ServiceUnavailable(
            "credential data validation unavailable: no credential-schema port configured"
                .to_owned(),
        )),
    }
}

/// Classify the auth pattern + capability flags for a built-in
/// credential key.
///
/// These are **type-level** facts about the built-in credential
/// taxonomy (`nebula_credential::credentials`), not a runtime claim
/// that the type works end-to-end. Unknown keys fall back to the
/// honest "custom / no declared capabilities" classification rather
/// than asserting a capability the engine cannot honor.
fn classify(credential_key: &str) -> (&'static str, CredentialCapabilities) {
    match credential_key {
        "oauth2" => (
            "OAuth2",
            CredentialCapabilities {
                interactive: true,
                refreshable: true,
                testable: false,
                revocable: true,
            },
        ),
        "api_key" => (
            "SecretToken",
            CredentialCapabilities {
                interactive: false,
                refreshable: false,
                testable: false,
                revocable: false,
            },
        ),
        "basic_auth" => (
            "IdentityPassword",
            CredentialCapabilities {
                interactive: false,
                refreshable: false,
                testable: false,
                revocable: false,
            },
        ),
        _ => (
            "Custom",
            CredentialCapabilities {
                interactive: false,
                refreshable: false,
                testable: false,
                revocable: false,
            },
        ),
    }
}

/// Project a stored credential into the metadata-only wire response.
///
/// Secret-safe by construction: [`CredentialResponse`] has no `data`
/// field, and this never touches [`StoredCredential::data`].
fn to_response(stored: &StoredCredential) -> ApiResult<CredentialResponse> {
    let meta: CredentialMeta =
        serde_json::from_value(serde_json::Value::Object(stored.metadata.clone()))
            .map_err(|e| ApiError::Internal(format!("corrupt credential metadata: {e}")))?;
    let (auth_pattern, capabilities) = classify(&meta.credential_key);
    Ok(CredentialResponse {
        id: stored.id.clone(),
        credential_key: meta.credential_key,
        name: meta.name,
        description: meta.description,
        auth_pattern: auth_pattern.to_owned(),
        capabilities,
        created_at: stored.created_at.to_rfc3339(),
        updated_at: stored.updated_at.to_rfc3339(),
        expires_at: stored.expires_at.map(|t| t.to_rfc3339()),
        version: stored.version,
        tags: meta.tags,
    })
}

/// Project a stored credential into the lightweight list summary.
fn to_summary(stored: &StoredCredential) -> ApiResult<CredentialSummary> {
    let meta: CredentialMeta =
        serde_json::from_value(serde_json::Value::Object(stored.metadata.clone()))
            .map_err(|e| ApiError::Internal(format!("corrupt credential metadata: {e}")))?;
    let (auth_pattern, _) = classify(&meta.credential_key);
    Ok(CredentialSummary {
        id: stored.id.clone(),
        credential_key: meta.credential_key,
        name: meta.name,
        auth_pattern: auth_pattern.to_owned(),
        expires_at: stored.expires_at.map(|t| t.to_rfc3339()),
        version: stored.version,
    })
}

/// Map a store-layer error onto a typed [`ApiError`].
///
/// Cross-workspace / unknown ids collapse to `404` with **no existence
/// disclosure** (mirrors the Phase-2 owner-scoped pattern). The error
/// string never contains secret material or credential identifiers.
fn map_store_err(err: StoreError, cred: &str) -> ApiError {
    match err {
        StoreError::NotFound { .. } => ApiError::NotFound("credential not found".to_owned()),
        StoreError::AlreadyExists { .. } => {
            ApiError::Conflict(format!("credential {cred} already exists"))
        },
        StoreError::VersionConflict {
            expected, actual, ..
        } => ApiError::VersionMismatch(format!(
            "credential {cred}: expected version {expected}, found {actual}"
        )),
        StoreError::AuditFailure(reason) => {
            ApiError::ServiceUnavailable(format!("credential audit sink unavailable: {reason}"))
        },
        StoreError::Backend(e) => {
            ApiError::Internal(format!("credential store backend error: {e}"))
        },
        // `StoreError` is `#[non_exhaustive]`; an unforeseen future
        // variant is an internal store fault — never echo a secret, and
        // do not disclose existence beyond the (client-supplied) id.
        _ => ApiError::Internal(format!("credential store error for {cred}")),
    }
}

/// Fetch a credential, treating a cross-workspace / unknown id as a
/// flat 404 (no existence disclosure, problem+json error seam / Phase-2 pattern).
async fn load(state: &AppState, owner_id: &str, cred: &str) -> ApiResult<StoredCredential> {
    scoped_store(state, owner_id)
        .get(cred)
        .await
        .map_err(|e| map_store_err(e, cred))
}

// ── CRUD ────────────────────────────────────────────────────────────────────

/// Create a new credential in the given workspace.
///
/// Persists the type-specific `data` as a write-only secret blob and
/// returns metadata only (never the secret).
#[tracing::instrument(skip_all, fields(cred.key = %req.credential_key))]
pub async fn create_credential(
    state: &AppState,
    owner_id: &str,
    req: CreateCredentialRequest,
) -> ApiResult<CredentialResponse> {
    // credential-schema validation: validate `data` against the type's schema BEFORE
    // any persist/encode. No port ⇒ 503 (never persist unvalidated).
    validate_credential_data(state, &req.credential_key, &req.data)?;

    let id = nebula_core::CredentialId::new().to_string();
    let secret_bytes = encode_secret_data(&req.data)?;

    let meta = CredentialMeta {
        credential_key: req.credential_key.clone(),
        name: req.name.clone(),
        description: req.description.clone(),
        tags: req.tags.clone().unwrap_or_default(),
    };
    let metadata = match serde_json::to_value(&meta) {
        Ok(serde_json::Value::Object(m)) => m,
        Ok(_) | Err(_) => {
            return Err(ApiError::Internal(
                "failed to encode credential metadata".to_owned(),
            ));
        },
    };

    let now = chrono::Utc::now();
    let stored = StoredCredential {
        id: id.clone(),
        credential_key: req.credential_key,
        data: secret_bytes,
        state_kind: STATE_KIND.to_owned(),
        state_version: STATE_VERSION,
        version: 0,
        created_at: now,
        updated_at: now,
        expires_at: None,
        reauth_required: false,
        metadata,
    };

    let persisted = scoped_store(state, owner_id)
        .put(stored, PutMode::CreateOnly)
        .await
        .map_err(|e| map_store_err(e, &id))?;

    tracing::info!(cred.id = %persisted.id, "credential created");
    to_response(&persisted)
}

/// Retrieve a single credential by ID within a workspace.
///
/// Returns metadata only — the secret `data` blob is never read here
/// and the response type has no field for it.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn get_credential(
    state: &AppState,
    owner_id: &str,
    cred: &str,
) -> ApiResult<CredentialResponse> {
    let stored = load(state, owner_id, cred).await?;
    to_response(&stored)
}

/// Update an existing credential in the workspace.
///
/// Partial update: only provided fields change. A provided `version`
/// engages compare-and-swap (409 on mismatch). A provided `data`
/// re-encodes the write-only secret blob.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn update_credential(
    state: &AppState,
    owner_id: &str,
    cred: &str,
    req: UpdateCredentialRequest,
) -> ApiResult<CredentialResponse> {
    let existing = load(state, owner_id, cred).await?;

    let mut meta: CredentialMeta =
        serde_json::from_value(serde_json::Value::Object(existing.metadata.clone()))
            .map_err(|e| ApiError::Internal(format!("corrupt credential metadata: {e}")))?;
    if let Some(name) = req.name {
        meta.name = name;
    }
    if req.description.is_some() {
        meta.description = req.description;
    }
    if let Some(tags) = req.tags {
        meta.tags = tags;
    }
    let metadata = match serde_json::to_value(&meta) {
        Ok(serde_json::Value::Object(m)) => m,
        Ok(_) | Err(_) => {
            return Err(ApiError::Internal(
                "failed to encode credential metadata".to_owned(),
            ));
        },
    };

    // Re-encode the secret only when the caller supplied new data;
    // otherwise carry the existing opaque blob through untouched.
    // credential-schema validation: when new `data` is supplied, validate it against
    // the (unchanged) credential type's schema before re-encode/persist.
    let data = match req.data.as_ref() {
        Some(value) => {
            validate_credential_data(state, &existing.credential_key, value)?;
            encode_secret_data(value)?
        },
        None => existing.data.clone(),
    };

    let mode = match req.version {
        Some(expected_version) => PutMode::CompareAndSwap { expected_version },
        None => PutMode::Overwrite,
    };

    let updated = StoredCredential {
        id: existing.id.clone(),
        credential_key: existing.credential_key.clone(),
        data,
        state_kind: existing.state_kind.clone(),
        state_version: existing.state_version,
        version: existing.version,
        created_at: existing.created_at,
        updated_at: chrono::Utc::now(),
        expires_at: existing.expires_at,
        reauth_required: existing.reauth_required,
        metadata,
    };

    let persisted = scoped_store(state, owner_id)
        .put(updated, mode)
        .await
        .map_err(|e| map_store_err(e, cred))?;

    tracing::info!(cred.id = %persisted.id, "credential updated");
    to_response(&persisted)
}

/// Delete a credential from the workspace.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn delete_credential(state: &AppState, owner_id: &str, cred: &str) -> ApiResult<()> {
    scoped_store(state, owner_id)
        .delete(cred)
        .await
        .map_err(|e| map_store_err(e, cred))?;
    tracing::info!(cred.id = %cred, "credential deleted");
    Ok(())
}

/// List credentials in the workspace with optional filters.
///
/// Returns paginated metadata summaries (no secret material).
#[tracing::instrument(skip_all)]
pub async fn list_credentials(
    state: &AppState,
    owner_id: &str,
    query: ListCredentialsQuery,
) -> ApiResult<ListCredentialsResponse> {
    // Push the `state_kind` filter into the store: only rows this layer
    // manages (`STATE_KIND`) come back, so the OAuth-callback rows (a
    // different `state_kind` + non-`CredentialMeta` metadata shape) are
    // excluded at the source rather than fetched-then-discarded — no
    // wasted `get` + projection, and no metadata-shape 500 risk.
    let store = scoped_store(state, owner_id);
    let ids = store
        .list(Some(STATE_KIND))
        .await
        .map_err(|e| map_store_err(e, "<list>"))?;

    let mut summaries: Vec<CredentialSummary> = Vec::new();
    for id in ids {
        // A row may vanish between `list` and `get` (concurrent delete);
        // skip it rather than failing the whole page.
        let Ok(stored) = store.get(&id).await else {
            continue;
        };
        let summary = to_summary(&stored)?;
        if let Some(ref key) = query.credential_key
            && &summary.credential_key != key
        {
            continue;
        }
        if let Some(ref pattern) = query.auth_pattern
            && &summary.auth_pattern != pattern
        {
            continue;
        }
        summaries.push(summary);
    }

    summaries.sort_by(|a, b| a.id.cmp(&b.id));
    let total = summaries.len();
    let offset = query.offset();
    let limit = query.limit();
    let page: Vec<CredentialSummary> = summaries.into_iter().skip(offset).take(limit).collect();

    Ok(ListCredentialsResponse {
        credentials: page,
        total,
        page: query.page,
        page_size: query.page_size,
    })
}

// ── Lifecycle (honest 503 — engine-owned, no registry wired) ─────────────────

/// Test credential connectivity against the external system.
///
/// **Honest 503.** A real test requires dispatching the registered
/// `Credential`'s `Testable::test` (an outbound provider call). No
/// `CredentialRegistry` is wired into `AppState`, and test dispatch is
/// engine-owned (`nebula-engine::credential`, engine credential orchestration — see
/// `docs/MATURITY.md`). A "test" that does not contact the provider
/// would be a honest capability false capability.
pub async fn test_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _cred: &str,
) -> ApiResult<TestCredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential test is engine-owned (Testable::test dispatch via \
         nebula-engine::credential) and not wired into this API build — \
         no CredentialRegistry in AppState"
            .into(),
    ))
}

/// Force a token refresh for the credential.
///
/// **Honest 503.** Refresh orchestration is engine-owned (engine credential orchestration /
/// refresh coordinator; the L2 refresh coordinator lives in
/// `nebula-engine::credential::refresh`). The API does not own a
/// refresh path end-to-end, so this stays honest rather than faking a
/// successful refresh.
pub async fn refresh_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _cred: &str,
) -> ApiResult<RefreshCredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential refresh is engine-owned (RefreshCoordinator in \
         nebula-engine::credential, engine refresh orchestration) and not exposed \
         through this API build"
            .into(),
    ))
}

/// Explicitly revoke the credential at the provider.
///
/// **Honest 503.** Revocation requires dispatching the registered
/// `Credential`'s `Revocable::revoke` (a provider call). No registry is
/// wired and dispatch is engine-owned.
pub async fn revoke_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _cred: &str,
) -> ApiResult<RevokeCredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential revoke is engine-owned (Revocable::revoke dispatch \
         via nebula-engine::credential) and not wired into this API \
         build — no CredentialRegistry in AppState"
            .into(),
    ))
}

// ── Acquisition (honest 503 — engine-owned resolver, no registry wired) ──────

/// Start credential acquisition / resolution.
///
/// **Honest 503.** Generic resolve dispatches a registered
/// `Credential::resolve()` by key — there is no `CredentialRegistry` in
/// `AppState`, and credential runtime resolution is engine-owned
/// (`nebula-engine::credential`, MATURITY P8). The interactive OAuth2
/// pending-exchange path that *is* wired is reached through the
/// dedicated `/credentials/{id}/oauth2/auth` controller, not this
/// generic endpoint.
pub async fn resolve_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _req: ResolveCredentialRequest,
) -> ApiResult<ResolveCredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "generic credential resolve is engine-owned (Credential::resolve \
         dispatch via nebula-engine::credential, P8) and needs a \
         CredentialRegistry that is not wired into this API build; the \
         interactive OAuth2 path is served by /credentials/{id}/oauth2/auth"
            .into(),
    ))
}

/// Continue a multi-step credential acquisition.
///
/// **Honest 503.** Symmetric to [`resolve_credential`] — needs a
/// registry + `Interactive::continue_resolve` dispatch (engine-owned).
/// The wired pending-exchange path is the OAuth2 callback controller,
/// not this generic endpoint.
pub async fn continue_resolve(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _req: ContinueResolveRequest,
) -> ApiResult<ContinueResolveResponse> {
    Err(ApiError::ServiceUnavailable(
        "generic credential continue_resolve is engine-owned \
         (Interactive::continue_resolve dispatch via \
         nebula-engine::credential) and not wired into this API build; \
         the interactive OAuth2 path is served by \
         /credentials/{id}/oauth2/callback"
            .into(),
    ))
}

// ── Type discovery (honest 503 — no CredentialRegistry wired) ────────────────

/// List all registered credential types with their schemas and capabilities.
///
/// **Honest 503.** Enumerating registered types + their schemas
/// requires a `CredentialRegistry`; none is wired into `AppState`.
/// Returning a hand-rolled catalog would misrepresent what is actually
/// registered (honest capability).
/// Map a port [`CredentialTypeDescriptor`] to the wire DTO, applying the
/// api-owned public projection to the schema (credential-schema port + #6 — the
/// raw `json_schema()` export's `x-nebula-root-rules` / predicate operands
/// are stripped before the unauthenticated wire).
fn credential_type_info_from_descriptor(
    d: crate::ports::credential_schema::CredentialTypeDescriptor,
) -> CredentialTypeInfo {
    CredentialTypeInfo {
        key: d.key,
        name: d.name,
        description: d.description,
        auth_pattern: d.auth_pattern,
        capabilities: CredentialCapabilities {
            interactive: d.capabilities.interactive,
            refreshable: d.capabilities.refreshable,
            testable: d.capabilities.testable,
            revocable: d.capabilities.revocable,
        },
        schema: crate::domain::credential::schema_projection::project_public_schema(d.schema_json),
        icon: d.icon,
        documentation_url: d.documentation_url,
    }
}

const NO_CRED_SCHEMA_PORT: &str =
    "credential type discovery unavailable: no credential-schema port configured";

/// credential-schema port: list registered credential types with their
/// public-projected input schema. No port ⇒ honest 503 (honest capability).
pub async fn list_credential_types(state: &AppState) -> ApiResult<ListCredentialTypesResponse> {
    let port = state
        .credential_schema
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable(NO_CRED_SCHEMA_PORT.to_owned()))?;
    let types = port
        .list_types()
        .into_iter()
        .map(credential_type_info_from_descriptor)
        .collect();
    Ok(ListCredentialTypesResponse { types })
}

/// credential-schema port: one credential type by key. No port ⇒ honest 503;
/// unknown key ⇒ 404 (credential *types* are public catalog info, so
/// non-existence disclosure is non-sensitive — unlike credential
/// *instances*, which are flat-404 per IDOR rules).
pub async fn get_credential_type(state: &AppState, key: &str) -> ApiResult<CredentialTypeInfo> {
    let port = state
        .credential_schema
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable(NO_CRED_SCHEMA_PORT.to_owned()))?;
    port.get_type(key)
        .map(credential_type_info_from_descriptor)
        .ok_or_else(|| ApiError::NotFound(format!("unknown credential type: {key}")))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use nebula_storage::inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
        InMemoryNodeResultStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
    };

    /// Permissive port so the CRUD/secret-projection unit tests still
    /// exercise persistence after credential-schema validation closed the
    /// unvalidated-persist fail-open (the no-port → 503 behavior is
    /// covered by `tests/seam_credential_write_path_validation.rs`).
    struct PermissivePort;
    impl crate::ports::credential_schema::CredentialSchemaPort for PermissivePort {
        fn validate_data(
            &self,
            _k: &str,
            _d: &serde_json::Value,
        ) -> Result<(), Vec<crate::ports::credential_schema::CredentialFieldError>> {
            Ok(())
        }
        fn list_types(&self) -> Vec<crate::ports::credential_schema::CredentialTypeDescriptor> {
            Vec::new()
        }
        fn get_type(
            &self,
            _k: &str,
        ) -> Option<crate::ports::credential_schema::CredentialTypeDescriptor> {
            None
        }
    }

    fn test_state() -> AppState {
        // Fresh raw (undecorated) in-memory port adapters — the production
        // composition shape post-decorator-removal. These tests only
        // exercise the credential write path, never storage rows; the
        // per-request tenant scope is applied by the `AppState` accessors.
        let exec_store = InMemoryExecutionStore::new();
        let control_queue = InMemoryControlQueue::new(&exec_store);
        let journal = InMemoryJournalReader::new(&exec_store);
        let jwt = crate::config::ApiConfig::for_test().jwt_secret;
        let workflow_versions = InMemoryWorkflowVersionStore::new();
        let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);
        AppState::new(
            Arc::new(workflow_store),
            Arc::new(workflow_versions),
            Arc::new(exec_store),
            Arc::new(InMemoryNodeResultStore::new()),
            Arc::new(journal),
            Arc::new(control_queue),
            jwt,
        )
        .with_credential_schema(Arc::new(PermissivePort))
    }

    /// The blob round-trips through `serde_secret`, and the redaction
    /// envelope's `Debug` never spells the plaintext (credential secrecy).
    #[test]
    fn secret_data_envelope_redacts_and_round_trips() {
        let secret = "sk-unit-NEVER-LEAK-9f9f";
        let data = serde_json::json!({ "api_key": secret });
        let bytes = encode_secret_data(&data).expect("encode");

        // The persisted bytes DO carry the secret (write-only; at-rest
        // encryption requires an `EncryptionLayer`, not wired here —
        // see the operator warning in README.md). What must never leak
        // is Debug / default Serialize of the envelope.
        let env: PersistedSecretData = serde_json::from_slice(&bytes).expect("decode");
        assert!(
            !format!("{env:?}").contains(secret),
            "PersistedSecretData Debug must not spell the secret"
        );
        // Default `Serialize` of the inner SecretString redacts.
        let redacted = serde_json::to_string(&env.blob).expect("serialize");
        assert!(
            !redacted.contains(secret),
            "default SecretString Serialize must redact"
        );
        assert_eq!(redacted, "\"[REDACTED]\"");
    }

    /// honest capability: the engine-owned / registry-absent functions return a
    /// typed honest 503 — never a faked success — even at the function
    /// boundary (independent of route shadowing).
    #[tokio::test]
    async fn engine_owned_fns_are_honest_503() {
        let s = test_state();
        assert!(matches!(
            test_credential(&s, "o", "w", "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            refresh_credential(&s, "o", "w", "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            revoke_credential(&s, "o", "w", "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            resolve_credential(
                &s,
                "o",
                "w",
                ResolveCredentialRequest {
                    credential_key: "api_key".into(),
                    data: serde_json::json!({}),
                }
            )
            .await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            continue_resolve(
                &s,
                "o",
                "w",
                ContinueResolveRequest {
                    pending_token: "t".into(),
                    user_input: serde_json::json!({}),
                }
            )
            .await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        // credential-schema port: `list_credential_types`/`get_credential_type`
        // are no longer engine-owned-503 — they are port-backed (a
        // permissive port is wired in `test_state()`). Their no-port → 503
        // behavior is covered by
        // `tests/seam_credential_catalog_schema.rs::catalog_503_when_port_unconfigured`.
    }

    /// CRUD over the wired in-memory store: create → get → list →
    /// delete, asserting the response projection never carries `data`
    /// and the secret never appears in any returned struct.
    #[tokio::test]
    async fn crud_round_trips_without_secret_in_projection() {
        let s = test_state();
        let owner_id = "o:w";
        let secret = "sk-unit-crud-NEVER-LEAK-7a7a";
        let created = create_credential(
            &s,
            owner_id,
            CreateCredentialRequest {
                credential_key: "api_key".into(),
                name: "Unit Key".into(),
                description: Some("d".into()),
                data: serde_json::json!({ "api_key": secret }),
                tags: None,
            },
        )
        .await
        .expect("create");
        assert!(created.id.starts_with("cred_"));
        assert_eq!(created.version, 1);
        let dbg = format!("{created:?}");
        assert!(
            !dbg.contains(secret),
            "CredentialResponse Debug must not carry the secret: {dbg}"
        );

        let got = get_credential(&s, owner_id, &created.id)
            .await
            .expect("get");
        assert_eq!(got.id, created.id);
        assert!(!format!("{got:?}").contains(secret));

        let listed = list_credentials(
            &s,
            owner_id,
            ListCredentialsQuery {
                page: 1,
                page_size: 20,
                credential_key: None,
                auth_pattern: None,
            },
        )
        .await
        .expect("list");
        assert_eq!(listed.total, 1);
        assert!(!format!("{listed:?}").contains(secret));

        delete_credential(&s, owner_id, &created.id)
            .await
            .expect("delete");
        assert!(matches!(
            get_credential(&s, owner_id, &created.id).await,
            Err(ApiError::NotFound(_))
        ));
    }
}
