//! In-process reference implementation of the credential lifecycle contract.
//!
//! This adapter is a deterministic semantic oracle for tests and conformance.
//! It is not a deployment backend and is compiled only for this crate's tests.

use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use nebula_core::CredentialId;
use nebula_storage_port::{
    CredentialAlreadyExistsKey, CredentialCommit, CredentialCreate, CredentialMaterialEpoch,
    CredentialMaterialTransition, CredentialOperationStatus, CredentialOwner,
    CredentialPersistence, CredentialPersistenceError, CredentialRefreshCursor,
    CredentialRefreshHorizon, CredentialRefreshPageSize, CredentialRefreshSchedule,
    CredentialRefreshScheduleError, CredentialReplacement, CredentialSelector, CredentialTombstone,
    CredentialVersion, DueCredentialRefresh, RefreshRetryAdmission, RefreshRetryProjection,
    RefreshRetrySnapshot, StoredCredential, StoredCredentialHead, StoredCredentialOperationalHead,
    StoredLiveCredential, StoredTombstonedCredential,
};
use parking_lot::Mutex;
use serde_json::{Map, Value};

#[derive(Clone)]
struct OwnedRecord {
    owner: CredentialOwner,
    record: StoredCredential,
}

/// Test/reference credential persistence with the same lifecycle semantics as
/// the SQL adapters.
#[derive(Clone, Default)]
pub(crate) struct ReferenceCredentialPersistence {
    records: Arc<Mutex<HashMap<CredentialId, OwnedRecord>>>,
}

impl ReferenceCredentialPersistence {
    /// Construct an empty reference store.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Single clock authority for this in-process backend.
    ///
    /// Both `SetAfter` and admission sample this seam; callers never supply a
    /// timestamp and therefore cannot shorten a durable gate.
    fn backend_now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    fn validate_projection(
        name: Option<&str>,
        metadata: &Map<String, Value>,
    ) -> Result<(), CredentialPersistenceError> {
        let projected = match metadata.get("display") {
            None => None,
            Some(Value::Object(display)) => {
                if !matches!(
                    display.get("description"),
                    None | Some(Value::Null | Value::String(_))
                ) || !matches!(display.get("tags"), None | Some(Value::Object(_)))
                    || display
                        .get("tags")
                        .and_then(Value::as_object)
                        .is_some_and(|tags| tags.values().any(|value| !value.is_string()))
                {
                    return Err(CredentialPersistenceError::CorruptRecord);
                }
                match display.get("display_name") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(value)) => Some(value.as_str()),
                    Some(_) => return Err(CredentialPersistenceError::CorruptRecord),
                }
            },
            Some(_) => return Err(CredentialPersistenceError::CorruptRecord),
        };
        if projected != name {
            return Err(CredentialPersistenceError::CorruptRecord);
        }
        Ok(())
    }

    fn name_is_taken(
        records: &HashMap<CredentialId, OwnedRecord>,
        owner: &CredentialOwner,
        name: Option<&str>,
        except: Option<CredentialId>,
    ) -> bool {
        let Some(name) = name else {
            return false;
        };
        records.iter().any(|(id, owned)| {
            except != Some(*id)
                && &owned.owner == owner
                && owned
                    .record
                    .as_live()
                    .is_some_and(|live| live.name() == Some(name))
        })
    }
}

#[async_trait]
impl CredentialRefreshSchedule for ReferenceCredentialPersistence {
    async fn scan_due(
        &self,
        after: Option<&CredentialRefreshCursor>,
        horizon: CredentialRefreshHorizon,
        limit: CredentialRefreshPageSize,
    ) -> Result<Vec<DueCredentialRefresh>, CredentialRefreshScheduleError> {
        let now = self.backend_now();
        let horizon = chrono::Duration::from_std(horizon.get())
            .map_err(|_| CredentialRefreshScheduleError::CorruptRecord)?;
        let cutoff = now
            .checked_add_signed(horizon)
            .ok_or(CredentialRefreshScheduleError::CorruptRecord)?;
        let records = self.records.lock();
        let mut candidates = records
            .values()
            .filter_map(|owned| {
                let live = owned.record.as_live()?;
                let expires_at = live.expires_at()?;
                if live.reauth_required() || expires_at > cutoff {
                    return None;
                }
                let admission =
                    super::retry_gate::evaluate_gate(live.refresh_retry_gate(), now).ok()?;
                if !matches!(admission, RefreshRetryAdmission::Open) {
                    return None;
                }
                let key = (expires_at, live.credential_id());
                if after.is_some_and(|cursor| key <= (cursor.expires_at(), cursor.credential_id()))
                {
                    return None;
                }
                Some((
                    key,
                    DueCredentialRefresh::new(
                        CredentialSelector::new(owned.owner.clone(), live.credential_id()),
                        live.credential_key().to_owned(),
                        expires_at,
                        now,
                    ),
                ))
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|(key, _)| *key);
        candidates.truncate(usize::from(limit.get()));
        Ok(candidates
            .into_iter()
            .map(|(_, candidate)| candidate)
            .collect())
    }
}

fn project_head(
    live: &StoredLiveCredential,
    backend_now: chrono::DateTime<chrono::Utc>,
) -> Result<StoredCredentialHead, CredentialPersistenceError> {
    let refresh_retry = match live.refresh_retry_gate() {
        None => None,
        Some(nebula_storage_port::RefreshRetryGate::Never { .. }) => {
            Some(RefreshRetryProjection::Never)
        },
        Some(nebula_storage_port::RefreshRetryGate::NotBefore { not_before, .. })
            if *not_before <= backend_now =>
        {
            None
        },
        Some(nebula_storage_port::RefreshRetryGate::NotBefore { not_before, .. }) => {
            Some(RefreshRetryProjection::NotBefore {
                not_before: *not_before,
            })
        },
    };
    StoredCredentialHead::new_with_refresh_retry(
        live.credential_id(),
        live.name().map(str::to_owned),
        live.credential_key().to_owned(),
        live.state_kind().to_owned(),
        live.state_version(),
        live.version(),
        live.material_epoch(),
        live.created_at(),
        live.updated_at(),
        live.expires_at(),
        live.reauth_required(),
        refresh_retry,
        live.metadata().clone(),
    )
}

impl fmt::Debug for ReferenceCredentialPersistence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReferenceCredentialPersistence")
    }
}

#[cfg(test)]
#[async_trait]
impl super::CredentialPersistenceConformance for ReferenceCredentialPersistence {
    async fn force_live_version_for_conformance(
        &self,
        selector: &CredentialSelector,
        version: CredentialVersion,
    ) -> Result<(), CredentialPersistenceError> {
        let mut records = self.records.lock();
        let owned = records
            .get_mut(&selector.credential_id())
            .ok_or(CredentialPersistenceError::NotFound)?;
        if &owned.owner != selector.owner() {
            return Err(CredentialPersistenceError::NotFound);
        }
        let StoredCredential::Live(current) = &owned.record else {
            return Err(CredentialPersistenceError::NotFound);
        };
        let refresh_retry_gate = current.refresh_retry_gate().cloned();
        owned.record = StoredLiveCredential::new(
            current.credential_id(),
            current.name().map(str::to_owned),
            current.credential_key().to_owned(),
            current.data().clone(),
            current.state_kind().to_owned(),
            current.state_version(),
            version,
            current.material_epoch(),
            current.created_at(),
            current.updated_at(),
            current.expires_at(),
            current.reauth_required(),
            current.metadata().clone(),
            refresh_retry_gate,
        )?
        .into();
        Ok(())
    }

    async fn force_live_material_epoch_for_conformance(
        &self,
        selector: &CredentialSelector,
        material_epoch: CredentialMaterialEpoch,
    ) -> Result<(), CredentialPersistenceError> {
        let mut records = self.records.lock();
        let owned = records
            .get_mut(&selector.credential_id())
            .ok_or(CredentialPersistenceError::NotFound)?;
        if &owned.owner != selector.owner() {
            return Err(CredentialPersistenceError::NotFound);
        }
        let StoredCredential::Live(current) = &owned.record else {
            return Err(CredentialPersistenceError::NotFound);
        };
        owned.record = StoredLiveCredential::new(
            current.credential_id(),
            current.name().map(str::to_owned),
            current.credential_key().to_owned(),
            current.data().clone(),
            current.state_kind().to_owned(),
            current.state_version(),
            current.version(),
            material_epoch,
            current.created_at(),
            current.updated_at(),
            current.expires_at(),
            current.reauth_required(),
            current.metadata().clone(),
            current.refresh_retry_gate().cloned(),
        )?
        .into();
        Ok(())
    }

    async fn corrupt_live_projection_for_conformance(
        &self,
        selector: &CredentialSelector,
    ) -> Result<(), CredentialPersistenceError> {
        let mut records = self.records.lock();
        let owned = records
            .get_mut(&selector.credential_id())
            .ok_or(CredentialPersistenceError::NotFound)?;
        if &owned.owner != selector.owner() {
            return Err(CredentialPersistenceError::NotFound);
        }
        let StoredCredential::Live(current) = &owned.record else {
            return Err(CredentialPersistenceError::NotFound);
        };
        let metadata = Map::from_iter([(
            "display".to_owned(),
            Value::String("not-an-object".to_owned()),
        )]);
        let refresh_retry_gate = current.refresh_retry_gate().cloned();
        owned.record = StoredLiveCredential::new(
            current.credential_id(),
            None,
            current.credential_key().to_owned(),
            current.data().clone(),
            current.state_kind().to_owned(),
            current.state_version(),
            current.version(),
            current.material_epoch(),
            current.created_at(),
            current.updated_at(),
            current.expires_at(),
            current.reauth_required(),
            metadata,
            refresh_retry_gate,
        )?
        .into();
        Ok(())
    }
}

#[async_trait]
impl CredentialPersistence for ReferenceCredentialPersistence {
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let records = self.records.lock();
        let Some(owned) = records.get(&selector.credential_id()) else {
            return Err(CredentialPersistenceError::NotFound);
        };
        if &owned.owner != selector.owner() {
            return Err(CredentialPersistenceError::NotFound);
        }
        if let StoredCredential::Live(live) = &owned.record {
            Self::validate_projection(live.name(), live.metadata())?;
        }
        Ok(owned.record.clone())
    }

    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        let record = self.get(selector).await?;
        match record {
            StoredCredential::Live(live) => project_head(&live, self.backend_now()),
            StoredCredential::Tombstoned(_) => Err(CredentialPersistenceError::NotFound),
        }
    }

    async fn operation_status(
        &self,
        selector: &CredentialSelector,
    ) -> Result<CredentialOperationStatus, CredentialPersistenceError> {
        let records = self.records.lock();
        let owned = records
            .get(&selector.credential_id())
            .ok_or(CredentialPersistenceError::NotFound)?;
        if &owned.owner != selector.owner() {
            return Err(CredentialPersistenceError::NotFound);
        }
        let StoredCredential::Live(live) = &owned.record else {
            return Err(CredentialPersistenceError::NotFound);
        };
        Ok(CredentialOperationStatus::Open {
            version: live.version(),
            material_epoch: live.material_epoch(),
            reauth_required: live.reauth_required(),
        })
    }

    async fn get_operational_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialOperationalHead, CredentialPersistenceError> {
        let records = self.records.lock();
        let owned = records
            .get(&selector.credential_id())
            .ok_or(CredentialPersistenceError::NotFound)?;
        if &owned.owner != selector.owner() {
            return Err(CredentialPersistenceError::NotFound);
        }
        let StoredCredential::Live(live) = &owned.record else {
            return Err(CredentialPersistenceError::NotFound);
        };
        let head = project_head(live, self.backend_now())?;
        let status = CredentialOperationStatus::Open {
            version: live.version(),
            material_epoch: live.material_epoch(),
            reauth_required: live.reauth_required(),
        };
        Ok(StoredCredentialOperationalHead::new(head, status))
    }

    async fn list_operational_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialOperationalHead>, CredentialPersistenceError> {
        let records = self.records.lock();
        records
            .values()
            .filter(|owned| &owned.owner == owner)
            .filter_map(|owned| {
                let StoredCredential::Live(live) = &owned.record else {
                    return None;
                };
                if state_kind.is_some_and(|kind| live.state_kind() != kind) {
                    return None;
                }
                Some((|| {
                    let head = project_head(live, self.backend_now())?;
                    let status = CredentialOperationStatus::Open {
                        version: live.version(),
                        material_epoch: live.material_epoch(),
                        reauth_required: live.reauth_required(),
                    };
                    Ok(StoredCredentialOperationalHead::new(head, status))
                })())
            })
            .collect()
    }

    async fn refresh_retry_snapshot(
        &self,
        selector: &CredentialSelector,
    ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
        let records = self.records.lock();
        let owned = records
            .get(&selector.credential_id())
            .ok_or(CredentialPersistenceError::NotFound)?;
        if &owned.owner != selector.owner() {
            return Err(CredentialPersistenceError::NotFound);
        }
        let StoredCredential::Live(live) = &owned.record else {
            return Err(CredentialPersistenceError::NotFound);
        };
        let admission =
            super::retry_gate::evaluate_gate(live.refresh_retry_gate(), self.backend_now())?;
        Ok(RefreshRetrySnapshot::new(
            live.version(),
            live.material_epoch(),
            live.reauth_required(),
            admission,
        ))
    }

    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Self::validate_projection(create.name(), create.metadata())?;
        let mut records = self.records.lock();
        if let Some(existing) = records.get(&selector.credential_id()) {
            if &existing.owner != selector.owner() {
                return Err(CredentialPersistenceError::NotFound);
            }
            return Err(CredentialPersistenceError::AlreadyExists {
                key: CredentialAlreadyExistsKey::Id,
            });
        }
        if Self::name_is_taken(&records, selector.owner(), create.name(), None) {
            return Err(CredentialPersistenceError::AlreadyExists {
                key: CredentialAlreadyExistsKey::Name,
            });
        }

        let now = self.backend_now();
        let version = CredentialVersion::MIN;
        let live = StoredLiveCredential::new(
            selector.credential_id(),
            create.name().map(str::to_owned),
            create.credential_key().to_owned(),
            create.data().clone(),
            create.state_kind().to_owned(),
            create.state_version(),
            version,
            CredentialMaterialEpoch::MIN,
            now,
            now,
            create.expires_at(),
            create.reauth_required(),
            create.metadata().clone(),
            None,
        )?;
        records.insert(
            selector.credential_id(),
            OwnedRecord {
                owner: selector.owner().clone(),
                record: live.into(),
            },
        );
        CredentialCommit::live(selector.credential_id(), version, now, now)
    }

    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Self::validate_projection(replacement.name(), replacement.metadata())?;
        let mut records = self.records.lock();
        let Some(existing) = records.get(&selector.credential_id()) else {
            return Err(CredentialPersistenceError::NotFound);
        };
        if &existing.owner != selector.owner() {
            return Err(CredentialPersistenceError::NotFound);
        }
        let StoredCredential::Live(current) = &existing.record else {
            return Err(CredentialPersistenceError::NotFound);
        };
        if current.version() != replacement.expected_version() {
            return Err(CredentialPersistenceError::VersionConflict {
                expected: replacement.expected_version(),
                actual: current.version(),
            });
        }
        if let Some(fence) = replacement.fence() {
            if current.material_epoch() != fence.expected_material_epoch() {
                return Err(CredentialPersistenceError::VersionConflict {
                    expected: replacement.expected_version(),
                    actual: current.version(),
                });
            }
            if current.credential_key() != fence.expected_credential_key() {
                return Err(CredentialPersistenceError::VersionConflict {
                    expected: replacement.expected_version(),
                    actual: current.version(),
                });
            }
        }
        let next_version = current.version().next_live()?;
        if Self::name_is_taken(
            &records,
            selector.owner(),
            replacement.name(),
            Some(selector.credential_id()),
        ) {
            return Err(CredentialPersistenceError::AlreadyExists {
                key: CredentialAlreadyExistsKey::Name,
            });
        }

        let credential_key = current.credential_key().to_owned();
        let created_at = current.created_at();
        let now = self.backend_now();
        let (material_epoch, refresh_retry_gate) = match replacement.material_transition() {
            CredentialMaterialTransition::Preserve { refresh_retry } => (
                current.material_epoch(),
                super::retry_gate::apply_transition(
                    current.refresh_retry_gate(),
                    refresh_retry,
                    now,
                )?,
            ),
            CredentialMaterialTransition::Advance { .. } => {
                (current.material_epoch().next()?, None)
            },
        };
        // Material columns change only for `Advance { Replace }`.
        let (data, state_kind, state_version, expires_at) =
            match replacement.material_transition().material() {
                Some(material) => (
                    material.data().clone(),
                    material.state_kind().to_owned(),
                    material.state_version(),
                    material.expires_at(),
                ),
                None => (
                    current.data().clone(),
                    current.state_kind().to_owned(),
                    current.state_version(),
                    current.expires_at(),
                ),
            };
        let live = StoredLiveCredential::new(
            selector.credential_id(),
            replacement.name().map(str::to_owned),
            credential_key,
            data,
            state_kind,
            state_version,
            next_version,
            material_epoch,
            created_at,
            now,
            expires_at,
            replacement.reauth_required(),
            replacement.metadata().clone(),
            refresh_retry_gate,
        )?;
        records.insert(
            selector.credential_id(),
            OwnedRecord {
                owner: selector.owner().clone(),
                record: live.into(),
            },
        );
        CredentialCommit::live(selector.credential_id(), next_version, created_at, now)
    }

    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let mut records = self.records.lock();
        let Some(existing) = records.get(&selector.credential_id()) else {
            return Err(CredentialPersistenceError::NotFound);
        };
        if &existing.owner != selector.owner() {
            return Err(CredentialPersistenceError::NotFound);
        }
        let StoredCredential::Live(current) = &existing.record else {
            return Err(CredentialPersistenceError::NotFound);
        };
        if current.version() != tombstone.expected_version() {
            return Err(CredentialPersistenceError::VersionConflict {
                expected: tombstone.expected_version(),
                actual: current.version(),
            });
        }
        let next_version = current.version().next_tombstone()?;
        let credential_key = current.credential_key().to_owned();
        let state_kind = current.state_kind().to_owned();
        let state_version = current.state_version();
        let created_at = current.created_at();
        let now = self.backend_now();
        let terminal = StoredTombstonedCredential::new(
            selector.credential_id(),
            credential_key,
            state_kind,
            state_version,
            next_version,
            created_at,
            now,
            now,
        );
        records.insert(
            selector.credential_id(),
            OwnedRecord {
                owner: selector.owner().clone(),
                record: terminal.into(),
            },
        );
        Ok(CredentialCommit::tombstoned(
            selector.credential_id(),
            next_version,
            created_at,
            now,
            now,
        ))
    }

    async fn tombstone_revoked_material(
        &self,
        _selector: &CredentialSelector,
        _expected_material_epoch: CredentialMaterialEpoch,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        // This reference backend has no atomically shared typed operation
        // ledger. Claiming revoke authority here would make the semantic
        // oracle stronger than its persistence boundary.
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
        let records = self.records.lock();
        let mut ids = records
            .values()
            .filter(|owned| &owned.owner == owner)
            .filter_map(|owned| owned.record.as_live())
            .filter(|live| state_kind.is_none_or(|kind| live.state_kind() == kind))
            .map(StoredLiveCredential::credential_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        Ok(ids)
    }

    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        let records = self.records.lock();
        let mut heads = records
            .values()
            .filter(|owned| &owned.owner == owner)
            .filter_map(|owned| owned.record.as_live())
            .filter(|live| state_kind.is_none_or(|kind| live.state_kind() == kind))
            .map(|live| project_head(live, self.backend_now()))
            .collect::<Result<Vec<_>, _>>()?;
        heads.sort_unstable_by_key(StoredCredentialHead::credential_id);
        Ok(heads)
    }

    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        let records = self.records.lock();
        Ok(records.get(&selector.credential_id()).is_some_and(|owned| {
            &owned.owner == selector.owner() && matches!(owned.record, StoredCredential::Live(_))
        }))
    }
}

#[cfg(test)]
mod atomic_snapshot_source_tests {
    #[test]
    fn refresh_retry_snapshot_holds_one_aggregate_lock() {
        let source = include_str!("reference.rs");
        let body = source
            .split_once("async fn refresh_retry_snapshot(")
            .expect("snapshot method must exist")
            .1
            .split_once("\n    async fn create(")
            .expect("the following port method must delimit the snapshot body")
            .0;

        assert_eq!(body.matches("self.records.lock()").count(), 1);
        assert!(body.contains("live.version()"));
        assert!(body.contains("live.reauth_required()"));
        assert!(body.contains("live.refresh_retry_gate()"));
        assert!(!body.contains("self.get(selector).await"));
    }
}
