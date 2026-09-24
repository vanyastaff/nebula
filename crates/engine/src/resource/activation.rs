//! Activation of stored resource rows into the live resource [`Manager`].
//!
//! A stored row (`ResourceStore`) is inert data: a kind, a config blob and
//! per-slot credential selectors. An execution's binding manifest names the
//! exact rows its nodes use; before a durable turn runs, each named row is
//! materialized into a registry row the node's acquire can reach.
//!
//! Activation is **lazy and per row**. Nothing is read or connected at boot,
//! idle tenants cost nothing, and one broken row only affects executions that
//! bind it. Each row activates at most once per stored version: concurrent
//! turns that bind the same row wait on one activation instead of racing
//! several registrations, and a version bump re-registers in place.
//!
//! The stored row id is part of the registry row identity
//! ([`RegisterRequest::row_id`]), so two rows of one kind in one workspace
//! never replace each other.
//!
//! The row's operator settings (`topology`, `resilience_override`) are passed
//! to the registration, which re-validates them. With the `rotation` feature
//! a row bound to credentials is recorded in the credential-rotation reverse
//! index as it registers and removed as it retires, so a refresh or revoke
//! of those credentials reaches the live resource.

use std::{sync::Arc, time::Duration};

use dashmap::DashMap;
use nebula_core::{CredentialId, ResourceId, ResourceKey, ScopeLevel, SlotKind, WorkspaceId};
use nebula_credential::{
    Capabilities, CredentialSlotResolveError, CredentialSlotResolver, TenantScope,
};
use nebula_expression::ExpressionEngine;
use nebula_resource::{
    CredentialSlotInstall, Manager, RegisterRequest, RegistrarError, ResourceActivatorRegistry,
    ResourceConfigInput, SlotBinding, SlotIdentity,
};
use nebula_storage_port::{Scope, StorageError, dto::ResourceRow, store::ResourceStore};
use tokio_util::sync::CancellationToken;

/// Default upper bound on one row activation (storage read, credential
/// resolution and registration together).
pub const DEFAULT_ACTIVATION_TIMEOUT: Duration = Duration::from_secs(30);

/// A stored row materialized into the manager: the registry row an acquire
/// must address to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivatedResource {
    /// Resource key the row was registered under.
    pub resource_key: ResourceKey,
    /// Scope the row was registered at.
    pub scope: ScopeLevel,
    /// Registry row identity, including the stored row id.
    pub slot_identity: SlotIdentity,
}

/// Why a stored row could not be activated.
///
/// Messages carry ids, kinds and slot names only; never config values or
/// credential material.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoredResourceActivationError {
    /// The execution's workspace id is not a typed workspace id.
    #[error("execution workspace id is not a typed workspace id")]
    InvalidTenantScope,
    /// The row does not exist in this workspace, or was deleted.
    #[error("resource {resource_id} does not exist in this workspace")]
    NotFound {
        /// The missing row.
        resource_id: ResourceId,
    },
    /// The row's kind differs from the kind the binding was planned against.
    #[error("resource {resource_id} is of kind `{stored}`, the binding expects `{expected}`")]
    KindMismatch {
        /// The offending row.
        resource_id: ResourceId,
        /// Kind stored on the row.
        stored: String,
        /// Kind the execution's binding contract names.
        expected: ResourceKey,
    },
    /// The row's kind is not in this process's resource allowlist.
    #[error("resource kind `{kind}` is not in the resource allowlist")]
    UnknownKind {
        /// The unknown kind.
        kind: String,
    },
    /// The row binds a slot the resource does not declare.
    #[error("resource binds undeclared credential slot `{slot}`")]
    UndeclaredSlot {
        /// The undeclared slot.
        slot: String,
    },
    /// A required credential slot has no binding on the row.
    #[error("required credential slot `{slot}` is not bound")]
    MissingRequiredSlot {
        /// The unbound slot.
        slot: String,
    },
    /// A slot's selector is not a credential id.
    #[error("credential selector for slot `{slot}` is not a credential id")]
    InvalidCredentialSelector {
        /// The slot with the malformed selector.
        slot: String,
    },
    /// The row binds credentials but this engine has no credential resolver.
    #[error("no credential resolver is configured to resolve slot `{slot}`")]
    NoCredentialResolver {
        /// The first slot that needed resolution.
        slot: String,
    },
    /// The bound credential could not be projected for this tenant.
    #[error("credential for slot `{slot}` could not be resolved: {source}")]
    Credential {
        /// The slot being resolved.
        slot: String,
        /// Why resolution failed.
        #[source]
        source: CredentialSlotResolveError,
    },
    /// Reading the row failed.
    #[error("resource storage read failed")]
    Storage(#[source] StorageError),
    /// The typed registration rejected the row (config, topology, identity).
    #[error("resource registration failed")]
    Register(#[source] RegistrarError),
    /// Activation did not finish within the configured bound.
    #[error("resource activation exceeded {0:?}")]
    TimedOut(Duration),
    /// The turn was cancelled while activating.
    #[error("resource activation was cancelled")]
    Cancelled,
}

/// Engine-owned collaborators an activation registers through.
pub struct ActivationContext<'a> {
    /// Closed `kind → factory` allowlist.
    pub registrars: &'a ResourceActivatorRegistry,
    /// Live manager rows are registered into.
    pub manager: &'a Manager,
    /// Tenant-scoped credential projection, when credentials are configured.
    pub credentials: Option<&'a dyn CredentialSlotResolver>,
    /// Expression engine for explicitly authored config expressions.
    pub expr_engine: &'a ExpressionEngine,
    /// Credential-rotation reverse index. A row bound to credentials is
    /// recorded here as it registers and removed as it retires, so a refresh
    /// or revoke of those credentials reaches the live resource.
    #[cfg(feature = "rotation")]
    pub fanout: Option<&'a nebula_resource::ResourceFanoutIndex>,
}

/// A stored row currently registered by a [`StoredResourceActivator`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveResourceRow {
    /// Tenant the row belongs to.
    pub scope: Scope,
    /// The stored row.
    pub resource_id: ResourceId,
    /// Stored version it was activated at.
    pub version: u64,
    /// Registry row serving it.
    pub activated: ActivatedResource,
}

/// What a [`StoredResourceActivator`] knows about one tracked row.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowState {
    /// Registered and idle.
    Active(ActiveResourceRow),
    /// An activation holds the row right now: its state is unknown until
    /// that finishes, and a registration it already had may still serve.
    Busy {
        /// Tenant the row belongs to.
        scope: Scope,
        /// The stored row.
        resource_id: ResourceId,
    },
}

#[derive(Debug, Clone)]
struct ActiveRow {
    version: u64,
    activated: ActivatedResource,
}

type RowSlot = Arc<tokio::sync::Mutex<Option<ActiveRow>>>;

/// Lazily activates stored resource rows, once per stored version.
pub struct StoredResourceActivator {
    store: Arc<dyn ResourceStore>,
    timeout: Duration,
    rows: DashMap<(Scope, ResourceId), RowSlot>,
    limit_key_secret: Arc<[u8; 32]>,
}

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

/// Default key for account quota keys: fixed, so every worker and every
/// restart derives the same key for one account and a shared limit store
/// holds one quota for it.
const DEFAULT_LIMIT_KEY_SECRET: [u8; 32] = *b"nebula.resource.limit-key.v1\0\0\0\0";

/// The quota key rows bound to the same credentials share: a provider limits
/// the account behind a credential, not one resource row.
///
/// Derived with a key every worker shares (see
/// [`StoredResourceActivator::with_limit_key_secret`]), so all workers name
/// one account's quota alike; the tenant owner is part of the input so two
/// tenants never share a quota. `None` for a row bound to no credential,
/// which is then limited on its own.
fn account_limit_key(
    secret: &[u8; 32],
    scope: &Scope,
    bindings: &[SlotBinding],
) -> Option<nebula_resource::rate_limit::LimitKey> {
    use hmac::{KeyInit as _, Mac as _};

    let mut credentials: Vec<String> = bindings
        .iter()
        .filter_map(|binding| binding.credential_id.map(|id| id.to_string()))
        .collect();
    if credentials.is_empty() {
        return None;
    }
    credentials.sort_unstable();
    credentials.dedup();
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(scope.credential_owner_id().as_bytes());
    for credential in &credentials {
        mac.update(&[0]);
        mac.update(credential.as_bytes());
    }
    let digest = mac.finalize().into_bytes();
    let hex = digest
        .iter()
        .take(16)
        .fold(String::with_capacity(32), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    nebula_resource::rate_limit::LimitKey::new(format!("acct:{hex}")).ok()
}

impl std::fmt::Debug for StoredResourceActivator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredResourceActivator")
            .field("timeout", &self.timeout)
            .field("tracked_rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}

impl StoredResourceActivator {
    /// Activates rows read from `store`.
    #[must_use]
    pub fn new(store: Arc<dyn ResourceStore>) -> Self {
        Self {
            store,
            timeout: DEFAULT_ACTIVATION_TIMEOUT,
            rows: DashMap::new(),
            limit_key_secret: Arc::new(DEFAULT_LIMIT_KEY_SECRET),
        }
    }

    /// Sets the secret account quota keys are derived with.
    ///
    /// Every worker sharing a limit store must use the same secret, or each
    /// derives different keys and gets its own quota. The default is a fixed
    /// domain key, identical in every process and across restarts: the
    /// inputs (tenant owner, credential ids) are identifiers the resource
    /// rows already store, so a secret only hides which rows share an
    /// account from someone reading the limit table. A deployment that wants
    /// that sets one secret for all its workers here.
    #[must_use]
    pub fn with_limit_key_secret(mut self, secret: [u8; 32]) -> Self {
        self.limit_key_secret = Arc::new(secret);
        self
    }

    /// Bounds one row activation. A slow backend or credential source then
    /// fails that row instead of stalling the turn indefinitely.
    #[must_use]
    pub fn with_activation_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Rows this activator currently holds registered, with the stored
    /// version each was activated at.
    ///
    /// A row whose activation is in progress is skipped rather than waited
    /// for: this is a status snapshot, and the next one picks it up.
    #[must_use]
    pub fn active_rows(&self) -> Vec<ActiveResourceRow> {
        self.row_states()
            .into_iter()
            .filter_map(|state| match state {
                RowState::Active(row) => Some(row),
                RowState::Busy { .. } => None,
            })
            .collect()
    }

    /// Every tracked row: active, or busy (an activation holds it right now,
    /// so its previous registration, if any, may still be serving).
    #[must_use]
    pub fn row_states(&self) -> Vec<RowState> {
        self.rows
            .iter()
            .filter_map(|entry| {
                let (scope, resource_id) = entry.key();
                let Ok(guard) = entry.value().try_lock() else {
                    return Some(RowState::Busy {
                        scope: scope.clone(),
                        resource_id: *resource_id,
                    });
                };
                let active = guard.as_ref()?;
                Some(RowState::Active(ActiveResourceRow {
                    scope: scope.clone(),
                    resource_id: *resource_id,
                    version: active.version,
                    activated: active.activated.clone(),
                }))
            })
            .collect()
    }

    /// Ensures row `resource_id` of `scope` is registered at its current
    /// stored version and returns the registry row to acquire.
    ///
    /// Concurrent calls for the same row wait on one activation. A row that
    /// was deleted since its last activation is retired from the manager.
    ///
    /// # Errors
    ///
    /// Any [`StoredResourceActivationError`]; the manager is left unchanged
    /// for a row that fails before registration.
    pub async fn activate(
        &self,
        context: &ActivationContext<'_>,
        scope: &Scope,
        resource_id: ResourceId,
        expected_key: &ResourceKey,
        cancel: &CancellationToken,
    ) -> Result<ActivatedResource, StoredResourceActivationError> {
        let slot = Arc::clone(
            self.rows
                .entry((scope.clone(), resource_id))
                .or_default()
                .value(),
        );
        let work = async {
            let mut active = slot.lock().await;
            self.refresh(
                context,
                scope,
                resource_id,
                expected_key,
                &mut active,
                cancel,
            )
            .await
        };
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(StoredResourceActivationError::Cancelled),
            outcome = tokio::time::timeout(self.timeout, work) => outcome
                .unwrap_or(Err(StoredResourceActivationError::TimedOut(self.timeout))),
        }
    }

    /// Retires every active row whose stored definition was deleted.
    ///
    /// Rows an activation holds right now are skipped (that activation sees
    /// the deletion itself); a storage read that fails leaves the row as it
    /// is until the next sweep.
    pub async fn retire_deleted(&self, context: &ActivationContext<'_>) {
        let keys: Vec<(Scope, ResourceId)> =
            self.rows.iter().map(|entry| entry.key().clone()).collect();
        for (scope, resource_id) in keys {
            let Some(slot) = self
                .rows
                .get(&(scope.clone(), resource_id))
                .map(|entry| Arc::clone(entry.value()))
            else {
                continue;
            };
            let Ok(mut active) = slot.try_lock() else {
                continue;
            };
            if active.is_none() {
                continue;
            }
            let row = match self.store.get(&scope, &resource_id.to_string()).await {
                Ok(row) => row,
                Err(error) => {
                    tracing::debug!(
                        target: "nebula_engine::resource_activation",
                        %resource_id,
                        %error,
                        "stored resource could not be re-read; kept until the next sweep"
                    );
                    continue;
                },
            };
            if row.is_some_and(|row| row.deleted_at.is_none()) {
                continue;
            }
            if let Some(stale) = active.take() {
                retire(context, &stale.activated);
                tracing::debug!(
                    target: "nebula_engine::resource_activation",
                    %resource_id,
                    "deleted stored resource retired"
                );
            }
        }
    }

    async fn refresh(
        &self,
        context: &ActivationContext<'_>,
        scope: &Scope,
        resource_id: ResourceId,
        expected_key: &ResourceKey,
        active: &mut Option<ActiveRow>,
        cancel: &CancellationToken,
    ) -> Result<ActivatedResource, StoredResourceActivationError> {
        let row = self
            .store
            .get(scope, &resource_id.to_string())
            .await
            .map_err(StoredResourceActivationError::Storage)?;
        let Some(row) = row.filter(|row| row.deleted_at.is_none()) else {
            if let Some(stale) = active.take() {
                retire(context, &stale.activated);
            }
            return Err(StoredResourceActivationError::NotFound { resource_id });
        };
        if row.kind != expected_key.as_str() {
            return Err(StoredResourceActivationError::KindMismatch {
                resource_id,
                stored: row.kind,
                expected: expected_key.clone(),
            });
        }
        if let Some(current) = active
            .as_ref()
            .filter(|current| current.version == row.version)
        {
            return Ok(current.activated.clone());
        }

        let activated = register_row(context, scope, &row, &self.limit_key_secret, cancel).await?;
        let previous = active.replace(ActiveRow {
            version: row.version,
            activated: activated.clone(),
        });
        // Same identity re-registers in place; a changed binding set leaves
        // the old registry row behind unless it is retired here.
        if let Some(previous) = previous.filter(|previous| previous.activated != activated) {
            retire(context, &previous.activated);
        }
        tracing::debug!(
            target: "nebula_engine::resource_activation",
            %resource_id,
            kind = %row.kind,
            version = row.version,
            "stored resource activated"
        );
        Ok(activated)
    }
}

async fn register_row(
    context: &ActivationContext<'_>,
    scope: &Scope,
    row: &ResourceRow,
    limit_key_secret: &[u8; 32],
    cancel: &CancellationToken,
) -> Result<ActivatedResource, StoredResourceActivationError> {
    let workspace = WorkspaceId::parse(&scope.workspace_id)
        .map_err(|_| StoredResourceActivationError::InvalidTenantScope)?;
    let factory = context.registrars.factory(&row.kind).ok_or_else(|| {
        StoredResourceActivationError::UnknownKind {
            kind: row.kind.clone(),
        }
    })?;
    let declared: Vec<_> = factory
        .dependencies()
        .slot_fields()
        .iter()
        .filter_map(|field| match &field.kind {
            SlotKind::Credential { key, .. } => Some((field.slot_key, key.clone(), field.required)),
            SlotKind::Resource { .. } => None,
        })
        .collect();
    if let Some(slot) = row
        .credential_bindings
        .keys()
        .find(|slot| !declared.iter().any(|(name, ..)| name == slot))
    {
        return Err(StoredResourceActivationError::UndeclaredSlot { slot: slot.clone() });
    }

    let tenant = TenantScope::from_scope(scope);
    let mut slot_bindings = Vec::new();
    let mut slot_installs = Vec::new();
    for (slot, credential_key, required) in declared {
        let Some(selector) = row.credential_bindings.get(slot) else {
            if required {
                return Err(StoredResourceActivationError::MissingRequiredSlot {
                    slot: slot.to_owned(),
                });
            }
            continue;
        };
        let credential_id = CredentialId::parse(selector).map_err(|_| {
            StoredResourceActivationError::InvalidCredentialSelector {
                slot: slot.to_owned(),
            }
        })?;
        let resolver = context.credentials.ok_or_else(|| {
            StoredResourceActivationError::NoCredentialResolver {
                slot: slot.to_owned(),
            }
        })?;
        let guard = resolver
            .resolve_slot(
                &tenant,
                credential_id,
                credential_key.clone(),
                Capabilities::empty(),
                cancel.clone(),
            )
            .await
            .map_err(|source| StoredResourceActivationError::Credential {
                slot: slot.to_owned(),
                source,
            })?;
        slot_bindings.push(SlotBinding {
            slot_name: slot.to_owned(),
            credential_key,
            credential_id: Some(credential_id),
        });
        slot_installs.push(CredentialSlotInstall {
            slot_name: slot.to_owned(),
            guard,
        });
    }

    let scope_level = ScopeLevel::Workspace(workspace);
    let limit_key = account_limit_key(limit_key_secret, scope, &slot_bindings);
    let request = RegisterRequest {
        config: ResourceConfigInput::data(row.config.clone()),
        expr_engine: context.expr_engine,
        slot_bindings,
        slot_installs,
        scope: scope_level.clone(),
        recovery_gate: None,
        // Validated against the kind when stored; re-validated by the
        // registration itself, so a row stored before its kind tightened
        // fails activation closed.
        topology: row.topology.clone(),
        resilience_override: row.resilience_override.clone(),
        row_id: Some(row.id.clone()),
        limit_key,
    };
    // With rotation, the row's credentials are bound into the reverse index
    // before the row becomes discoverable, so a refresh or revoke reaches it.
    #[cfg(feature = "rotation")]
    let outcome = context
        .registrars
        .register_and_bind(&row.kind, context.manager, request, context.fanout)
        .await;
    #[cfg(not(feature = "rotation"))]
    let outcome = context
        .registrars
        .register(&row.kind, context.manager, request)
        .await;
    let outcome = outcome.map_err(StoredResourceActivationError::Register)?;
    Ok(ActivatedResource {
        resource_key: outcome.resource_key,
        scope: scope_level,
        slot_identity: outcome.slot_identity,
    })
}

fn retire(context: &ActivationContext<'_>, activated: &ActivatedResource) {
    #[cfg(feature = "rotation")]
    if let Some(fanout) = context.fanout {
        fanout.unbind_resource_identity(
            &activated.resource_key,
            &activated.scope,
            &activated.slot_identity,
        );
    }
    let manager = context.manager;
    if let Err(error) = manager.remove_for(
        &activated.resource_key,
        &activated.scope,
        &activated.slot_identity,
    ) {
        tracing::warn!(
            target: "nebula_engine::resource_activation",
            resource_key = %activated.resource_key,
            error = %error,
            "stale stored-resource registry row could not be retired"
        );
    }
}

#[cfg(test)]
#[path = "activation_tests.rs"]
pub(crate) mod tests;
