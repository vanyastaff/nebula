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
//! and a live fan-out driver, a row bound to credentials is recorded in the
//! credential-rotation reverse index as it registers (the manager drops it
//! as the row retires), so a refresh or revoke of those credentials reaches
//! the live resource; activation returns once the fan-out has reread them
//! and the row serves.
//!
//! Every activation of an already registered row re-checks its credentials.
//! A credential that denies use at its current material (reauthentication
//! required, an operation blocking use) **suspends** the kept registration
//! (`Manager::suspend_credential_row`) and fails the turn; the next
//! activation that finds the same material usable reopens it without
//! registering again. Only a credential that can no longer be resolved at
//! all retires the registration.

use std::{sync::Arc, time::Duration};

use dashmap::DashMap;
use nebula_core::{CredentialId, ResourceId, ResourceKey, ScopeLevel, SlotKind, WorkspaceId};
use nebula_credential::{
    Capabilities, CredentialAvailability, CredentialAvailabilityObserver, CredentialBlock,
    CredentialObserveError, CredentialSlotResolveError, CredentialSlotResolver, TenantScope,
};
use nebula_expression::ExpressionEngine;
use nebula_resource::{
    CredentialSlotInstall, CredentialUnavailableReason, Manager, RegisterRequest, RegistrarError,
    ResourceActivatorRegistry, ResourceConfigInput, SlotBinding, SlotIdentity,
};
use nebula_storage_port::{Scope, StorageError, dto::ResourceRow, store::ResourceStore};
use tokio_util::sync::CancellationToken;

/// Default upper bound on one row activation (storage read, credential
/// resolution and registration together).
pub const DEFAULT_ACTIVATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Rows one [`StoredResourceActivator::retire_deleted`] sweep re-reads: the
/// sweep runs on a status tick, so its storage cost stays bounded however
/// many rows are tracked; successive sweeps rotate through all of them.
pub const RETIRE_SWEEP_BATCH: usize = 32;

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
    /// The resource's resilience policy names an account credential slot it
    /// does not declare (see `ResiliencePolicy::account_credential`).
    #[error("resilience policy names undeclared account credential slot `{slot}`")]
    UndeclaredAccountSlot {
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
    /// Credential-rotation reverse index, present only while a fan-out
    /// driver reconciles this manager's rotation-bound rows.
    ///
    /// With it, a row bound to credentials registers rotation-bound: its
    /// credentials are bound into the index before the row is discoverable,
    /// and the row serves once the fan-out has reread them. Without it
    /// nothing would ever reconcile such a row, so the row registers opted
    /// out of rotation and activation's own credential re-check keeps it
    /// current.
    #[cfg(feature = "rotation")]
    pub fanout: Option<&'a Arc<nebula_resource::ResourceFanoutIndex>>,
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
    /// The last activation read stored version `version` and failed to
    /// register it (credentials, slots, config); a registration of an older
    /// version, if any, may still serve.
    Failed {
        /// Tenant the row belongs to.
        scope: Scope,
        /// The stored row.
        resource_id: ResourceId,
        /// Stored version that failed to activate.
        version: u64,
    },
}

#[derive(Debug, Clone)]
struct ActiveRow {
    version: u64,
    activated: ActivatedResource,
    /// The credentials this registration was built with: checked against
    /// the credential store on every activation, and bound into the
    /// rotation index (released when a re-registration keeps the identity).
    bindings: Vec<BoundCredential>,
}

/// One credential a registration resolved, and the material it got.
#[derive(Debug, Clone)]
struct BoundCredential {
    credential_id: CredentialId,
    slot: String,
    credential_key: nebula_core::CredentialKey,
    /// `(material_epoch, revision)` of the guard installed.
    material: (u64, u64),
}

/// What an activator tracks for one stored row.
#[derive(Debug, Default)]
struct TrackedRow {
    /// The registration serving the row, if any.
    active: Option<ActiveRow>,
    /// Stored version whose activation last failed; cleared once a version
    /// registers.
    failed_version: Option<u64>,
    /// Stored version an activation in progress read, so an activation cut
    /// short by its timeout can still record that version as failed.
    reading: Option<u64>,
}

impl TrackedRow {
    const fn is_empty(&self) -> bool {
        self.active.is_none() && self.failed_version.is_none()
    }
}

type RowSlot = Arc<tokio::sync::Mutex<TrackedRow>>;

/// Lazily activates stored resource rows, once per stored version.
pub struct StoredResourceActivator {
    store: Arc<dyn ResourceStore>,
    timeout: Duration,
    rows: DashMap<(Scope, ResourceId), RowSlot>,
    limit_key_secret: Arc<[u8; 32]>,
    /// Where the next [`retire_deleted`](Self::retire_deleted) sweep starts.
    sweep_cursor: std::sync::atomic::AtomicUsize,
    /// Registry rows this activator stopped tracking but the manager has not
    /// removed yet (its retirement queue pushed back); every sweep retries.
    pending_retirements: std::sync::Mutex<Vec<PendingRetirement>>,
}

/// A registration the manager has not removed yet, with the stored row it
/// served: only that row can register the same identity again.
#[derive(Debug)]
struct PendingRetirement {
    row: (Scope, ResourceId),
    stale: ActiveRow,
}

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

/// Default key for account quota keys: fixed, so every worker and every
/// restart derives the same key for one account and a shared limit store
/// holds one quota for it.
const DEFAULT_LIMIT_KEY_SECRET: [u8; 32] = *b"nebula.resource.limit-key.v1\0\0\0\0";

/// The quota key rows bound to the same credentials share: a provider limits
/// the account behind a credential, not one resource row.
///
/// Only the slots in `account_slots` count when the resource declares them
/// ([`ResiliencePolicy::account_credential`](nebula_resource::rate_limit::ResiliencePolicy::account_credential)),
/// so an auxiliary credential does not split one account's quota; every
/// bound credential counts otherwise.
///
/// Derived with a key every worker shares (see
/// [`StoredResourceActivator::with_limit_key_secret`]), so all workers name
/// one account's quota alike; the tenant owner is part of the input so two
/// tenants never share a quota. `None` for a row bound to no credential (see
/// [`row_limit_key`]).
fn account_limit_key(
    secret: &[u8; 32],
    scope: &Scope,
    bindings: &[BoundCredential],
    account_slots: &[&str],
) -> Option<nebula_resource::rate_limit::LimitKey> {
    use hmac::{KeyInit as _, Mac as _};

    let mut credentials: Vec<String> = bindings
        .iter()
        .filter(|bound| account_slots.is_empty() || account_slots.contains(&bound.slot.as_str()))
        .map(|bound| bound.credential_id.to_string())
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

/// The quota key of a row bound to no credential: the stored row itself,
/// named alike by every worker, so a cluster-wide limit on it is shared
/// across workers rather than applied once per process.
fn row_limit_key(
    secret: &[u8; 32],
    scope: &Scope,
    row_id: &str,
) -> Option<nebula_resource::rate_limit::LimitKey> {
    use hmac::{KeyInit as _, Mac as _};

    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(scope.credential_owner_id().as_bytes());
    mac.update(&[1]);
    mac.update(row_id.as_bytes());
    let digest = mac.finalize().into_bytes();
    let hex = digest
        .iter()
        .take(16)
        .fold(String::with_capacity(32), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    nebula_resource::rate_limit::LimitKey::new(format!("row:{hex}")).ok()
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
            sweep_cursor: std::sync::atomic::AtomicUsize::new(0),
            pending_retirements: std::sync::Mutex::new(Vec::new()),
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
                RowState::Busy { .. } | RowState::Failed { .. } => None,
            })
            .collect()
    }

    /// Every tracked row: active, busy (an activation holds it right now,
    /// so its previous registration, if any, may still be serving), or
    /// failed at its latest stored version.
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
                if let Some(version) = guard.failed_version {
                    return Some(RowState::Failed {
                        scope: scope.clone(),
                        resource_id: *resource_id,
                        version,
                    });
                }
                let active = guard.active.as_ref()?;
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
            let mut tracked = slot.lock().await;
            tracked.reading = None;
            let outcome = self
                .refresh(
                    context,
                    scope,
                    resource_id,
                    expected_key,
                    &mut tracked,
                    cancel,
                )
                .await;
            tracked.reading = None;
            outcome
        };
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(StoredResourceActivationError::Cancelled),
            outcome = tokio::time::timeout(self.timeout, work) => match outcome {
                Ok(outcome) => outcome,
                Err(_elapsed) => {
                    // The work was dropped midway; the version it read, if
                    // it got that far, failed to activate in time. Recorded
                    // without waiting: if another activation holds the row
                    // (this one timed out waiting for it), that one records
                    // its own outcome.
                    if let Ok(mut tracked) = slot.try_lock()
                        && let Some(version) = tracked.reading.take()
                    {
                        tracked.failed_version = Some(version);
                    }
                    Err(StoredResourceActivationError::TimedOut(self.timeout))
                },
            },
        }
    }

    /// Retires active rows whose stored definition was deleted.
    ///
    /// Re-reads at most [`RETIRE_SWEEP_BATCH`] rows per call, continuing
    /// where the previous sweep stopped, so a deletion is noticed within a
    /// bounded number of sweeps. Rows an activation holds right now are
    /// skipped (that activation sees the deletion itself); a storage read
    /// that fails leaves the row as it is until a later sweep. A deleted
    /// row's recorded failure is forgotten too, and tracking entries left
    /// empty (a retired row, or an activation that failed before reading
    /// its row) are dropped, so they do not accumulate.
    pub async fn retire_deleted(&self, context: &ActivationContext<'_>) {
        use std::sync::atomic::Ordering;

        self.retry_retirements(context);
        let keys: Vec<(Scope, ResourceId)> =
            self.rows.iter().map(|entry| entry.key().clone()).collect();
        if keys.is_empty() {
            return;
        }
        let start = self
            .sweep_cursor
            .fetch_add(RETIRE_SWEEP_BATCH, Ordering::Relaxed)
            % keys.len();
        let batch = keys
            .iter()
            .cycle()
            .skip(start)
            .take(RETIRE_SWEEP_BATCH.min(keys.len()));
        for key in batch {
            self.retire_if_deleted(context, key).await;
            self.reap_if_empty(key);
        }
    }

    async fn retire_if_deleted(&self, context: &ActivationContext<'_>, key: &(Scope, ResourceId)) {
        let Some(slot) = self.rows.get(key).map(|entry| Arc::clone(entry.value())) else {
            return;
        };
        let Ok(mut tracked) = slot.try_lock() else {
            return;
        };
        if tracked.is_empty() {
            return;
        }
        let (scope, resource_id) = key;
        let row = match self.store.get(scope, &resource_id.to_string()).await {
            Ok(row) => row,
            Err(error) => {
                tracing::debug!(
                    target: "nebula_engine::resource_activation",
                    %resource_id,
                    %error,
                    "stored resource could not be re-read; kept until a later sweep"
                );
                return;
            },
        };
        if row.is_some_and(|row| row.deleted_at.is_none()) {
            return;
        }
        tracked.failed_version = None;
        if let Some(stale) = tracked.active.take() {
            self.retire(context, key, stale);
            tracing::debug!(
                target: "nebula_engine::resource_activation",
                %resource_id,
                "deleted stored resource retired"
            );
        }
    }

    /// Drops `key`'s tracking entry if it holds no registration and nothing
    /// else holds it. An activation clones the slot under the same map
    /// lock this check runs under, so a concurrent activation keeps it.
    fn reap_if_empty(&self, key: &(Scope, ResourceId)) {
        self.rows.remove_if(key, |_, slot| {
            Arc::strong_count(slot) == 1 && slot.try_lock().is_ok_and(|tracked| tracked.is_empty())
        });
    }

    /// Retires `stale`, the registration row `row` no longer serves, keeping
    /// it for a later sweep when the manager cannot take it yet.
    fn retire(&self, context: &ActivationContext<'_>, row: &(Scope, ResourceId), stale: ActiveRow) {
        let mut pending = self
            .pending_retirements
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if retire(context, &stale.activated) {
            // Retiring an identity removed its registry row: an earlier
            // pushed-back registration of it has nothing left to retire.
            pending.retain(|entry| entry.stale.activated != stale.activated);
        } else {
            pending.push(PendingRetirement {
                row: row.clone(),
                stale,
            });
        }
    }

    /// Retries the retirements the manager pushed back.
    ///
    /// Registry identities include the stored row id, so only the row a
    /// pending entry came from can register that identity again. An entry
    /// whose row serves the identity again is dropped rather than removed
    /// (removing it would remove the live row); the registration that
    /// replaced it released its rotation bindings. An entry whose row an
    /// activation holds right now waits for a sweep that can see it; every
    /// other entry is retried.
    ///
    /// The row's lock is held from the check through the retirement, so an
    /// activation cannot register the same identity in between and have the
    /// retry remove that live registration.
    fn retry_retirements(&self, context: &ActivationContext<'_>) {
        let pending = std::mem::take(
            &mut *self
                .pending_retirements
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        if pending.is_empty() {
            return;
        }
        let mut still_pending: Vec<PendingRetirement> = Vec::new();
        let mut retired: Vec<ActivatedResource> = Vec::new();
        for entry in pending {
            if retired.contains(&entry.stale.activated) {
                continue;
            }
            // A row not tracked any more gets an (empty) entry to lock, which
            // a concurrent activation then waits behind.
            let row = entry.row.clone();
            let slot = Arc::clone(self.rows.entry(row.clone()).or_default().value());
            let Ok(tracked) = slot.try_lock() else {
                still_pending.push(entry);
                continue;
            };
            let serving = tracked
                .active
                .as_ref()
                .is_some_and(|active| active.activated == entry.stale.activated);
            if !serving {
                if retire(context, &entry.stale.activated) {
                    retired.push(entry.stale.activated.clone());
                } else {
                    still_pending.push(entry);
                }
            }
            drop(tracked);
            drop(slot);
            // The empty entry this retry may have created is not kept.
            self.reap_if_empty(&row);
        }
        still_pending.retain(|entry| !retired.contains(&entry.stale.activated));
        self.pending_retirements
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(still_pending);
    }

    async fn refresh(
        &self,
        context: &ActivationContext<'_>,
        scope: &Scope,
        resource_id: ResourceId,
        expected_key: &ResourceKey,
        tracked: &mut TrackedRow,
        cancel: &CancellationToken,
    ) -> Result<ActivatedResource, StoredResourceActivationError> {
        let row = self
            .store
            .get(scope, &resource_id.to_string())
            .await
            .map_err(StoredResourceActivationError::Storage)?;
        let Some(row) = row.filter(|row| row.deleted_at.is_none()) else {
            tracked.failed_version = None;
            if let Some(stale) = tracked.active.take() {
                self.retire(context, &(scope.clone(), resource_id), stale);
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
        tracked.reading = Some(row.version);
        if let Some((activated, bindings)) = tracked
            .active
            .as_ref()
            .filter(|current| current.version == row.version)
            .map(|current| (current.activated.clone(), current.bindings.clone()))
        {
            // Captured before the credentials are read: a suspension landing
            // after the check supersedes the reopen below.
            let ticket = context
                .manager
                .credential_gate_ticket(
                    &activated.resource_key,
                    &activated.scope,
                    &activated.slot_identity,
                )
                .ok();
            let serving = match credentials_current(context, scope, &bindings, cancel).await {
                CredentialCheck::Current => {
                    reopen_suspended(context, &activated, &bindings, ticket);
                    serves(context, &activated).await
                },
                // Refreshed or rotated since: registered again below, with
                // the credentials as they are now.
                CredentialCheck::Changed => false,
                // Denied at the material the row holds: the row is kept for
                // reuse but suspended, and this turn cannot use it.
                CredentialCheck::Blocked(blocked) => {
                    tracked.failed_version = Some(row.version);
                    return Err(suspend_blocked(context, &activated, blocked));
                },
                CredentialCheck::Terminal(error) => {
                    tracked.failed_version = Some(row.version);
                    if let Some(stale) = tracked.active.take() {
                        self.retire(context, &(scope.clone(), resource_id), stale);
                    }
                    return Err(error);
                },
            };
            // Serving at this version: a failure recorded for it earlier (a
            // check that timed out) no longer holds. A row that stopped
            // serving is registered again below.
            if serving {
                tracked.failed_version = None;
                return Ok(activated);
            }
        }

        // A failure is recorded against the version read, so status reports
        // it as failed rather than never activated.
        let (activated, bindings) = match register_row(
            context,
            scope,
            &row,
            &self.limit_key_secret,
            cancel,
            RetirementSink {
                row: (scope.clone(), resource_id),
                pending: &self.pending_retirements,
            },
        )
        .await
        {
            Ok(registered) => registered,
            Err(error) => {
                tracked.failed_version = Some(row.version);
                if let StoredResourceActivationError::Credential { slot, source } = &error {
                    match classify(source) {
                        // A credential that can no longer be resolved
                        // (revoked, deleted, refused) must not keep serving
                        // through the previous registration, whichever
                        // slot's change led here.
                        CredentialClass::Terminal => {
                            if let Some(stale) = tracked.active.take() {
                                self.retire(context, &(scope.clone(), resource_id), stale);
                            }
                        },
                        // Denied at its current material: the previous
                        // registration is kept for reuse, suspended.
                        CredentialClass::Blocked(reason) => {
                            if let Some(active) = tracked.active.as_ref() {
                                suspend_slot(context, &active.activated, slot, reason, None);
                            }
                        },
                        CredentialClass::Transient => {},
                    }
                }
                return Err(error);
            },
        };
        tracked.failed_version = None;
        let previous = tracked.active.replace(ActiveRow {
            version: row.version,
            activated: activated.clone(),
            bindings,
        });
        // Same identity re-registers in place, and the manager released the
        // replaced registration's rotation bindings as it replaced it. A
        // changed identity leaves the old registry row behind unless it is
        // retired here.
        if let Some(previous) = previous.filter(|previous| previous.activated != activated) {
            self.retire(context, &(scope.clone(), resource_id), previous);
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
    sink: RetirementSink<'_>,
) -> Result<(ActivatedResource, Vec<BoundCredential>), StoredResourceActivationError> {
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
    // A misspelt account slot would otherwise select no credential and
    // quietly limit the row on its own instead of per account.
    let policy = factory.resilience_policy();
    if let Some(slot) = policy
        .account_slots()
        .iter()
        .find(|slot| !declared.iter().any(|(name, ..)| name == *slot))
    {
        return Err(StoredResourceActivationError::UndeclaredAccountSlot {
            slot: (*slot).to_owned(),
        });
    }

    let tenant = TenantScope::from_scope(scope);
    // Rotation-bound only while a fan-out reconciles the row: nothing else
    // would ever reread a rotation-bound row's credentials and let it serve.
    #[cfg(feature = "rotation")]
    let rotation_bound = context.fanout.is_some();
    #[cfg(not(feature = "rotation"))]
    let rotation_bound = false;
    let mut slot_bindings = Vec::new();
    let mut slot_installs = Vec::new();
    let mut bindings = Vec::new();
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
        let metadata = guard.metadata();
        bindings.push(BoundCredential {
            credential_id,
            slot: slot.to_owned(),
            credential_key: credential_key.clone(),
            material: (metadata.material_epoch(), metadata.revision()),
        });
        slot_bindings.push(SlotBinding {
            slot_name: slot.to_owned(),
            credential_key,
            // Both absent opts the binding out of rotation.
            credential_id: rotation_bound.then_some(credential_id),
            credential_scope: rotation_bound.then(|| tenant.clone()),
        });
        slot_installs.push(CredentialSlotInstall {
            slot_name: slot.to_owned(),
            guard,
        });
    }

    let scope_level = ScopeLevel::Workspace(workspace);
    let limit_key = account_limit_key(limit_key_secret, scope, &bindings, policy.account_slots())
        .or_else(|| row_limit_key(limit_key_secret, scope, &row.id));
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
    // Rotation-bound, the row's credentials are bound into the reverse index
    // before the row becomes discoverable, so a refresh or revoke reaches it.
    #[cfg(feature = "rotation")]
    let outcome = match context.fanout {
        Some(index) => {
            context
                .registrars
                .register_and_bind(&row.kind, context.manager, request, Some(index))
                .await
        },
        None => {
            context
                .registrars
                .register(&row.kind, context.manager, request)
                .await
        },
    };
    #[cfg(not(feature = "rotation"))]
    let outcome = context
        .registrars
        .register(&row.kind, context.manager, request)
        .await;
    let outcome = outcome.map_err(StoredResourceActivationError::Register)?;
    let activated = ActivatedResource {
        resource_key: outcome.resource_key,
        scope: scope_level,
        slot_identity: outcome.slot_identity,
    };
    // A rotation-bound row serves once the fan-out has reread its
    // credentials; the turn this activation is for must not see it earlier.
    // Bounded by the activation's own timeout and cancellation. Until the
    // row serves, no activation records it: one abandoned here (timed out,
    // cancelled) or failing retires the registration it made rather than
    // leave a row nothing tracks.
    if rotation_bound && !bindings.is_empty() {
        let unclaimed = Unclaimed {
            context,
            activated: &activated,
            version: row.version,
            bindings: &bindings,
            sink,
            claimed: false,
        };
        context
            .manager
            .until_accepting(
                &activated.resource_key,
                &activated.scope,
                &activated.slot_identity,
                None,
            )
            .await
            .map_err(|source| {
                StoredResourceActivationError::Register(RegistrarError::Register {
                    kind: row.kind.clone(),
                    source,
                })
            })?;
        unclaimed.claim();
    }
    Ok((activated, bindings))
}

/// Where a registration that could not be retired yet is kept for the next
/// sweep, with the stored row it was made for.
struct RetirementSink<'p> {
    row: (Scope, ResourceId),
    pending: &'p std::sync::Mutex<Vec<PendingRetirement>>,
}

/// A registration no activation has recorded yet; retired when dropped
/// before it is [`claim`](Self::claim)ed.
///
/// When the manager pushes the retirement back, the registration is handed
/// to the activator's pending retirements instead of being forgotten: this
/// guard is the only record of it, and every sweep retries those.
struct Unclaimed<'c, 'a> {
    context: &'c ActivationContext<'a>,
    activated: &'c ActivatedResource,
    version: u64,
    bindings: &'c [BoundCredential],
    sink: RetirementSink<'c>,
    claimed: bool,
}

impl Unclaimed<'_, '_> {
    /// The caller records the registration from here on.
    fn claim(mut self) {
        self.claimed = true;
    }
}

impl Drop for Unclaimed<'_, '_> {
    fn drop(&mut self) {
        if !self.claimed && !retire(self.context, self.activated) {
            self.sink
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(PendingRetirement {
                    row: self.sink.row.clone(),
                    stale: ActiveRow {
                        version: self.version,
                        activated: self.activated.clone(),
                        bindings: self.bindings.to_vec(),
                    },
                });
        }
    }
}

/// Whether the registered `activated` row serves acquires now.
///
/// A rotation-bound row the live fan-out is rereading (after a material
/// replacement, say) is waited for, bounded by the activation. Without a
/// live fan-out nothing would ever let such a row serve again, so it does
/// not, and the caller registers the row again, opted out of rotation; a row
/// draining or failed is registered again too.
async fn serves(context: &ActivationContext<'_>, activated: &ActivatedResource) -> bool {
    let Some(row) = context.manager.get_row(
        &activated.resource_key,
        &activated.scope,
        &activated.slot_identity,
    ) else {
        return false;
    };
    if row.phase().is_accepting() {
        return true;
    }
    #[cfg(feature = "rotation")]
    if context.fanout.is_some() && row.phase() == nebula_resource::ResourcePhase::Initializing {
        return context
            .manager
            .until_accepting(
                &activated.resource_key,
                &activated.scope,
                &activated.slot_identity,
                None,
            )
            .await
            .is_ok();
    }
    false
}

/// Whether the credentials a registration was built with are still the
/// current ones.
///
/// Each is resolved again, as a node's action resolves its credentials on
/// every turn; the credential store's own `(material_epoch, revision)`
/// says whether it was refreshed or rotated since. This holds without the
/// rotation fan-out and across processes: a refresh, rotation or revoke
/// made anywhere reaches this row on its next activation, not only when
/// its definition changes.
///
/// Whether a credential is usable is checked **before** its material is
/// compared: a credential that needs reauthentication, or that a revoke or
/// an unreconciled operation blocks, is [`Blocked`](CredentialCheck::Blocked)
/// whatever its material. A blocked row is suspended, not retired: its
/// physical owners are kept and it serves again once the credential is
/// usable at the same material (Design CONTRACT, same-material block). A
/// credential that can no longer be resolved at all (revoked, deleted,
/// refused) is [`Terminal`](CredentialCheck::Terminal): the row must stop
/// serving it. A transient failure (the store or source unavailable, a
/// refresh in flight, cancellation) keeps the registration as it is — an
/// outage decides nothing about safety here — and is checked again next
/// time. One blocked slot wins over a changed one, and a terminal one over
/// both.
///
/// With a resolver that exposes an availability observer, each binding is
/// checked from the credential's operational head only, without decrypting;
/// only a changed credential is projected, by the registration that follows.
/// An absent head is re-checked through projection, which tells a tombstone
/// apart.
///
/// The check runs the same with a rotation fan-out attached: fan-out events
/// can be lost and nothing else installs a replacement then, so a changed
/// credential re-registers the row here too. When the fan-out did deliver,
/// that re-registration is redundant but harmless; serving a rotated-out or
/// revoked credential is not.
async fn credentials_current(
    context: &ActivationContext<'_>,
    scope: &Scope,
    bindings: &[BoundCredential],
    cancel: &CancellationToken,
) -> CredentialCheck {
    let Some(resolver) = context.credentials.filter(|_| !bindings.is_empty()) else {
        return CredentialCheck::Current;
    };
    let tenant = TenantScope::from_scope(scope);
    let mut changed = false;
    let mut blocked = Vec::new();
    for bound in bindings {
        let observed = match resolver.as_availability_observer() {
            Some(observer) => observe_binding(observer, &tenant, bound, cancel.clone()).await,
            None => None,
        };
        let status = if let Some(status) = observed {
            status
        } else {
            let resolved = resolver
                .resolve_slot(
                    &tenant,
                    bound.credential_id,
                    bound.credential_key.clone(),
                    Capabilities::empty(),
                    cancel.clone(),
                )
                .await;
            match resolved {
                Ok(guard) => {
                    let metadata = guard.metadata();
                    BindingStatus::Usable {
                        material: (metadata.material_epoch(), metadata.revision()),
                    }
                },
                Err(source) => BindingStatus::Failed {
                    source,
                    observed_material_epoch: None,
                },
            }
        };
        match status {
            BindingStatus::Usable { material } => changed |= material != bound.material,
            BindingStatus::Failed {
                source,
                observed_material_epoch,
            } => match classify(&source) {
                CredentialClass::Transient => {
                    tracing::debug!(
                        target: "nebula_engine::resource_activation",
                        slot = %bound.slot,
                        "credential could not be re-checked now; the registration is kept as it is"
                    );
                },
                CredentialClass::Blocked(reason) => blocked.push(BlockedSlot {
                    slot: bound.slot.clone(),
                    reason,
                    observed_material_epoch,
                    source,
                }),
                CredentialClass::Terminal => {
                    return CredentialCheck::Terminal(StoredResourceActivationError::Credential {
                        slot: bound.slot.clone(),
                        source,
                    });
                },
            },
        }
    }
    if !blocked.is_empty() {
        CredentialCheck::Blocked(blocked)
    } else if changed {
        CredentialCheck::Changed
    } else {
        CredentialCheck::Current
    }
}

/// The outcome of re-checking a registration's credentials.
enum CredentialCheck {
    /// Every credential is usable at the material the row holds.
    Current,
    /// A credential was refreshed or rotated: register the row again.
    Changed,
    /// These credentials deny use at their current material.
    Blocked(Vec<BlockedSlot>),
    /// A credential can no longer be resolved: retire the row.
    Terminal(StoredResourceActivationError),
}

/// A bound credential that denies use.
struct BlockedSlot {
    slot: String,
    reason: CredentialUnavailableReason,
    observed_material_epoch: Option<u64>,
    source: CredentialSlotResolveError,
}

/// One binding's state, from an observation or a projection.
enum BindingStatus {
    Usable {
        material: (u64, u64),
    },
    Failed {
        source: CredentialSlotResolveError,
        observed_material_epoch: Option<u64>,
    },
}

/// Head-only observation of one binding; `None` when only a projection can
/// tell (no live head: absent, or tombstoned).
async fn observe_binding(
    observer: &dyn CredentialAvailabilityObserver,
    tenant: &TenantScope,
    bound: &BoundCredential,
    cancel: CancellationToken,
) -> Option<BindingStatus> {
    let observation = observer
        .observe_availability(
            tenant,
            bound.credential_id,
            bound.credential_key.clone(),
            cancel,
        )
        .await;
    let observation = match observation {
        Ok(observation) => observation,
        Err(CredentialObserveError::Absent) => return None,
        Err(error) => {
            let source = match error {
                CredentialObserveError::WrongCredentialKey => {
                    CredentialSlotResolveError::WrongCredentialKey
                },
                CredentialObserveError::Unavailable => CredentialSlotResolveError::Unavailable,
                CredentialObserveError::SourceUnavailable => {
                    CredentialSlotResolveError::SourceUnavailable
                },
                CredentialObserveError::Cancelled => CredentialSlotResolveError::Cancelled,
                _ => CredentialSlotResolveError::InvalidState,
            };
            return Some(BindingStatus::Failed {
                source,
                observed_material_epoch: None,
            });
        },
    };
    let source = match observation.availability() {
        CredentialAvailability::Available => {
            return Some(BindingStatus::Usable {
                material: (observation.material_epoch(), observation.revision()),
            });
        },
        CredentialAvailability::RefreshInFlight => CredentialSlotResolveError::RefreshInFlight {
            retry_after: Duration::from_secs(1),
        },
        CredentialAvailability::Blocked(CredentialBlock::ReauthRequired) => {
            CredentialSlotResolveError::ReauthRequired
        },
        CredentialAvailability::Blocked(
            CredentialBlock::OperationInFlight { operation }
            | CredentialBlock::ReconciliationRequired { operation },
        ) => CredentialSlotResolveError::OperationBlocked { operation },
        // A block this engine does not know yet: the operation is unknown.
        CredentialAvailability::Blocked(_) => CredentialSlotResolveError::OperationBlocked {
            operation: nebula_credential::CredentialOperationKind::LegacyUnclassified,
        },
        // An availability this engine does not know yet: re-check later.
        _ => CredentialSlotResolveError::Unavailable,
    };
    Some(BindingStatus::Failed {
        source,
        observed_material_epoch: Some(observation.material_epoch()),
    })
}

/// How a credential resolution failure bears on the row that uses it.
enum CredentialClass {
    /// Only for now (the store or source unavailable, a refresh still in
    /// flight, the call cancelled): the row is kept as it is. A refresh in
    /// flight must not retire the registration that serves the credential.
    Transient,
    /// The credential denies use at its current material: the row is kept
    /// but suspended.
    Blocked(CredentialUnavailableReason),
    /// The credential can no longer be resolved at all: the row is retired.
    Terminal,
}

/// The one classification both the re-check and the registration use.
const fn classify(error: &CredentialSlotResolveError) -> CredentialClass {
    match error {
        CredentialSlotResolveError::Unavailable
        | CredentialSlotResolveError::SourceUnavailable
        | CredentialSlotResolveError::RefreshInFlight { .. }
        | CredentialSlotResolveError::Cancelled => CredentialClass::Transient,
        CredentialSlotResolveError::ReauthRequired => {
            CredentialClass::Blocked(CredentialUnavailableReason::ReauthRequired)
        },
        CredentialSlotResolveError::OperationBlocked { .. } => {
            CredentialClass::Blocked(CredentialUnavailableReason::OperationBlocked)
        },
        _ => CredentialClass::Terminal,
    }
}

/// Suspends every blocked slot of the kept registration `activated` and
/// returns the activation error the turn reports.
fn suspend_blocked(
    context: &ActivationContext<'_>,
    activated: &ActivatedResource,
    blocked: Vec<BlockedSlot>,
) -> StoredResourceActivationError {
    let mut first = None;
    for entry in blocked {
        suspend_slot(
            context,
            activated,
            &entry.slot,
            entry.reason,
            entry.observed_material_epoch,
        );
        first.get_or_insert(StoredResourceActivationError::Credential {
            slot: entry.slot,
            source: entry.source,
        });
    }
    first.unwrap_or(StoredResourceActivationError::Cancelled)
}

/// Records that `slot` of the kept registration `activated` denies use.
fn suspend_slot(
    context: &ActivationContext<'_>,
    activated: &ActivatedResource,
    slot: &str,
    reason: CredentialUnavailableReason,
    observed_material_epoch: Option<u64>,
) {
    let outcome = context.manager.suspend_credential_row(
        &activated.resource_key,
        &activated.scope,
        &activated.slot_identity,
        slot,
        reason,
        observed_material_epoch,
    );
    match outcome {
        Ok(outcome) => tracing::warn!(
            target: "nebula_engine::resource_activation",
            resource_key = %activated.resource_key,
            slot,
            %reason,
            ?outcome,
            "bound credential denies use; the registration is suspended, not retired"
        ),
        Err(error) => tracing::debug!(
            target: "nebula_engine::resource_activation",
            resource_key = %activated.resource_key,
            slot,
            error = %error,
            "bound credential denies use; no registration to suspend"
        ),
    }
}

/// Reopens the kept registration `activated` when a check found every
/// credential usable at the material it holds. A suspension recorded after
/// `ticket` was captured wins; the row then refuses acquires until a later
/// check reopens it.
fn reopen_suspended(
    context: &ActivationContext<'_>,
    activated: &ActivatedResource,
    bindings: &[BoundCredential],
    ticket: Option<nebula_resource::CredentialGateTicket>,
) {
    let Some(ticket) = ticket else {
        return;
    };
    let suspended = context
        .manager
        .get_row(
            &activated.resource_key,
            &activated.scope,
            &activated.slot_identity,
        )
        .and_then(|row| row.credential_suspension())
        .is_some();
    if !suspended {
        return;
    }
    for bound in bindings {
        let outcome = context.manager.reopen_credential_row(
            &activated.resource_key,
            &activated.scope,
            &activated.slot_identity,
            &bound.slot,
            ticket,
        );
        tracing::debug!(
            target: "nebula_engine::resource_activation",
            resource_key = %activated.resource_key,
            slot = %bound.slot,
            ?outcome,
            "bound credential usable again; reopening the kept registration"
        );
    }
}

/// Removes `activated` from the manager, which drops its rotation bindings
/// from every attached index as it removes it; `false` when the manager
/// pushed back (its retirement queue is full) and the row still serves,
/// rotation bindings included.
fn retire(context: &ActivationContext<'_>, activated: &ActivatedResource) -> bool {
    let removed = context.manager.remove_for(
        &activated.resource_key,
        &activated.scope,
        &activated.slot_identity,
    );
    match removed {
        Ok(()) => {},
        Err(error) if matches!(error.kind(), nebula_resource::ErrorKind::Backpressure) => {
            tracing::warn!(
                target: "nebula_engine::resource_activation",
                resource_key = %activated.resource_key,
                error = %error,
                "stale stored-resource registry row not retired yet; retried on the next sweep"
            );
            return false;
        },
        // Already gone (not found), or the manager is shutting down and
        // retires every row itself.
        Err(error) => tracing::debug!(
            target: "nebula_engine::resource_activation",
            resource_key = %activated.resource_key,
            error = %error,
            "stale stored-resource registry row needs no retirement"
        ),
    }
    true
}

#[cfg(test)]
#[path = "activation_tests.rs"]
pub(crate) mod tests;
