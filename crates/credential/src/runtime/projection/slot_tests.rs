use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use nebula_storage_port::{
    CredentialCommit, CredentialCreate, CredentialMaterialEpoch, CredentialOwner,
    CredentialReplacement, CredentialSelector, CredentialTombstone, CredentialVersion,
    RefreshRetrySnapshot, SecretBytes, StoredCredentialHead, StoredLiveCredential,
    StoredTombstonedCredential,
};

use super::*;
use crate::{
    BearerTokenCredential, Credential, CredentialService, CredentialState, SecretString, SharedKey,
    StateWireFingerprint, scheme::SecretToken,
};

const SECRET_CANARY: &str = "slot-resolver-secret-NEVER-DEBUG-994";

#[derive(Debug)]
struct SlotStore {
    owner: CredentialOwner,
    row: StoredCredential,
    row_after_head: Option<StoredCredential>,
    material_loads: AtomicUsize,
    /// An operation the head reports in flight, for this many head reads.
    in_flight: Option<(
        nebula_storage_port::store::CredentialOperationKind,
        AtomicUsize,
    )>,
}

#[async_trait]
impl crate::CredentialPersistence for SlotStore {
    async fn get_operational_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<nebula_storage_port::StoredCredentialOperationalHead, CredentialPersistenceError>
    {
        let head = self.get_head(selector).await?;
        if let Some((operation, reads)) = &self.in_flight
            && reads
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                    left.checked_sub(1)
                })
                .is_ok()
        {
            return Ok(nebula_storage_port::StoredCredentialOperationalHead::new(
                head,
                nebula_storage_port::store::CredentialOperationStatus::InFlight {
                    operation: *operation,
                },
            ));
        }
        let status = nebula_storage_port::store::CredentialOperationStatus::Open {
            version: head.version(),
            material_epoch: head.material_epoch(),
            reauth_required: head.reauth_required(),
        };
        Ok(nebula_storage_port::StoredCredentialOperationalHead::new(
            head, status,
        ))
    }

    async fn list_operational_heads(
        &self,
        _owner: &CredentialOwner,
        _state_kind: Option<&str>,
    ) -> Result<Vec<nebula_storage_port::StoredCredentialOperationalHead>, CredentialPersistenceError>
    {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn operation_status(
        &self,
        selector: &CredentialSelector,
    ) -> Result<nebula_storage_port::store::CredentialOperationStatus, CredentialPersistenceError>
    {
        if selector.owner() != &self.owner || selector.credential_id() != self.row.credential_id() {
            return Err(CredentialPersistenceError::NotFound);
        }
        let row = self.row_after_head.as_ref().unwrap_or(&self.row);
        let StoredCredential::Live(live) = row else {
            return Err(CredentialPersistenceError::NotFound);
        };
        Ok(
            nebula_storage_port::store::CredentialOperationStatus::Open {
                version: live.version(),
                material_epoch: live.material_epoch(),
                reauth_required: live.reauth_required(),
            },
        )
    }

    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        self.material_loads.fetch_add(1, Ordering::Relaxed);
        if selector.owner() == &self.owner && selector.credential_id() == self.row.credential_id() {
            Ok(self.row_after_head.as_ref().unwrap_or(&self.row).clone())
        } else {
            Err(CredentialPersistenceError::NotFound)
        }
    }

    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        if selector.owner() != &self.owner || selector.credential_id() != self.row.credential_id() {
            return Err(CredentialPersistenceError::NotFound);
        }
        match &self.row {
            StoredCredential::Live(stored) => Ok(StoredCredentialHead::from(stored)),
            StoredCredential::Tombstoned(_) => Err(CredentialPersistenceError::NotFound),
        }
    }

    async fn refresh_retry_snapshot(
        &self,
        _selector: &CredentialSelector,
    ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn create(
        &self,
        _selector: &CredentialSelector,
        _create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn replace(
        &self,
        _selector: &CredentialSelector,
        _replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn tombstone(
        &self,
        _selector: &CredentialSelector,
        _tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn tombstone_revoked_material(
        &self,
        _selector: &CredentialSelector,
        _expected_material_epoch: CredentialMaterialEpoch,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn list(
        &self,
        _owner: &CredentialOwner,
        _state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn list_heads(
        &self,
        _owner: &CredentialOwner,
        _state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }

    async fn exists(
        &self,
        _selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }
}

fn fixture() -> (
    SlotStore,
    crate::CredentialRegistry,
    crate::DispatchOps<ErasedPendingStore>,
    TenantScope,
    CredentialId,
    CredentialKey,
) {
    // Plain (pre-envelope) legacy JSON: the legacy-decode path keeps this
    // exact fixture as its slot-surface coverage.
    let token = SecretToken::new(SecretString::new(SECRET_CANARY));
    let data = crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&token))
        .expect("test token serializes");
    fixture_with_payload(
        SecretBytes::new(data),
        <SecretToken as CredentialState>::KIND,
        <SecretToken as CredentialState>::VERSION,
    )
}

fn fixture_with_payload(
    data: SecretBytes,
    kind: &str,
    version: u32,
) -> (
    SlotStore,
    crate::CredentialRegistry,
    crate::DispatchOps<ErasedPendingStore>,
    TenantScope,
    CredentialId,
    CredentialKey,
) {
    let scope = TenantScope::new("org-slot", "workspace-slot");
    let id = CredentialId::new();
    let now = Utc::now();
    let row = StoredLiveCredential::new(
        id,
        Some("default".to_owned()),
        BearerTokenCredential::KEY.to_owned(),
        data,
        kind.to_owned(),
        version,
        CredentialVersion::MIN,
        CredentialMaterialEpoch::MIN,
        now,
        now,
        None,
        false,
        serde_json::Map::new(),
        None,
    )
    .expect("fixture row is valid");
    let store = SlotStore {
        owner: scope.owner().clone(),
        row: row.into(),
        row_after_head: None,
        material_loads: AtomicUsize::new(0),
        in_flight: None,
    };
    let mut registry = crate::CredentialRegistry::new();
    registry
        .register(BearerTokenCredential, "slot-test")
        .expect("fixture registration is unique");
    let mut ops = crate::DispatchOps::new();
    crate::register_runtime_ops::<BearerTokenCredential, ErasedPendingStore>(&mut ops)
        .expect("fixture ops registration is unique");
    let key = CredentialKey::new(BearerTokenCredential::KEY).expect("fixture key is valid");
    (store, registry, ops, scope, id, key)
}

/// Envelope bytes for the slot surface with caller-supplied overrides for
/// the three checked facets (`None` = the honest value).
fn envelope_payload(
    fingerprint: Option<u64>,
    kind_tag: Option<&str>,
    interface_version: Option<u32>,
) -> Vec<u8> {
    let token = SecretToken::new(SecretString::new(SECRET_CANARY));
    crate::serde_secret::expose_for_serialization(|| {
        let body = serde_json::to_value(&token).expect("fixture state encodes");
        let envelope = serde_json::json!({
            "interface_version": interface_version
                .unwrap_or(<SecretToken as CredentialState>::VERSION),
            "schema_fingerprint": fingerprint
                .unwrap_or(<SecretToken as StateWireFingerprint>::SCHEMA_FINGERPRINT)
                .to_le_bytes(),
            "kind_tag": kind_tag.unwrap_or(<SecretToken as CredentialState>::KIND),
            "body": body,
        });
        serde_json::to_vec(&envelope).expect("fixture envelope encodes")
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the test helper keeps each security-relevant resolver input explicit"
)]
async fn resolve_fixture(
    store: &SlotStore,
    registry: &crate::CredentialRegistry,
    ops: &crate::DispatchOps<ErasedPendingStore>,
    scope: &TenantScope,
    id: CredentialId,
    key: CredentialKey,
    required: Capabilities,
    cancel: CancellationToken,
) -> Result<ErasedCredentialGuard, CredentialSlotResolveError> {
    resolve_slot_with(
        store,
        registry,
        ops,
        &StateSource::LocalEncrypted,
        SlotResolutionRequest {
            scope,
            credential_id: id,
            expected_key: key,
            required_capabilities: required,
            cancel,
        },
    )
    .await
}

#[tokio::test]
async fn resolves_opaque_guard_with_authoritative_ordering_metadata() {
    let (store, registry, ops, scope, id, key) = fixture();
    let erased = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key.clone(),
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect("matching owner and contract resolve");

    assert_eq!(erased.metadata().credential_id(), id);
    assert_eq!(erased.metadata().credential_key(), &key);
    assert_eq!(erased.metadata().material_epoch(), 1);
    assert_eq!(erased.metadata().revision(), 1);
    let typed = erased
        .into_typed::<SecretToken>()
        .expect("registered scheme type extracts");
    assert_eq!(typed.token().expose_secret(), SECRET_CANARY);
}

#[tokio::test]
async fn owner_qualified_tombstone_is_distinct_from_absence() {
    let (mut store, registry, ops, scope, id, key) = fixture();
    let now = Utc::now();
    store.row = StoredTombstonedCredential::new(
        id,
        BearerTokenCredential::KEY.to_owned(),
        SecretToken::KIND.to_owned(),
        SecretToken::VERSION,
        CredentialVersion::MIN,
        now,
        now,
        now,
    )
    .into();
    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("tombstone cannot project");
    assert_eq!(error, CredentialSlotResolveError::Revoked);
}

#[tokio::test]
async fn tombstone_with_wrong_contract_does_not_revoke_the_requested_slot() {
    let (mut store, registry, ops, scope, id, _) = fixture();
    let now = Utc::now();
    store.row = StoredTombstonedCredential::new(
        id,
        BearerTokenCredential::KEY.to_owned(),
        SecretToken::KIND.to_owned(),
        SecretToken::VERSION,
        CredentialVersion::MIN,
        now,
        now,
        now,
    )
    .into();
    let wrong = CredentialKey::new("shared_key").expect("test key is valid");

    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        wrong,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("wrong tombstone contract must not become a revoke signal");

    assert_eq!(error, CredentialSlotResolveError::WrongCredentialKey);
}

#[tokio::test]
async fn tombstone_committed_between_head_and_material_read_is_revoked() {
    let (mut store, registry, ops, scope, id, key) = fixture();
    let now = Utc::now();
    store.row_after_head = Some(
        StoredTombstonedCredential::new(
            id,
            BearerTokenCredential::KEY.to_owned(),
            SecretToken::KIND.to_owned(),
            SecretToken::VERSION,
            CredentialVersion::MIN,
            now,
            now,
            now,
        )
        .into(),
    );
    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("concurrent tombstone cannot project");
    assert_eq!(error, CredentialSlotResolveError::Revoked);
}

#[tokio::test]
async fn wrong_contract_tombstone_committed_after_head_is_not_revoked() {
    let (mut store, registry, ops, scope, id, key) = fixture();
    let now = Utc::now();
    store.row_after_head = Some(
        StoredTombstonedCredential::new(
            id,
            "shared_key".to_owned(),
            SecretToken::KIND.to_owned(),
            SecretToken::VERSION,
            CredentialVersion::MIN,
            now,
            now,
            now,
        )
        .into(),
    );

    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("a tombstone for another contract must not revoke the slot");

    assert_eq!(error, CredentialSlotResolveError::WrongCredentialKey);
}

#[tokio::test]
async fn rejects_wrong_key_before_projection() {
    let (store, registry, ops, scope, id, _) = fixture();
    let wrong = CredentialKey::new("shared_key").expect("test key is valid");
    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        wrong,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("wrong contract must fail");
    assert_eq!(error, CredentialSlotResolveError::WrongCredentialKey);
    assert_eq!(store.material_loads.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn rejects_missing_capability_before_projection() {
    let (store, registry, ops, scope, id, key) = fixture();
    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::REFRESHABLE,
        CancellationToken::new(),
    )
    .await
    .expect_err("unsupported capability must fail");
    assert_eq!(error, CredentialSlotResolveError::MissingCapabilities);
    assert_eq!(store.material_loads.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn missing_and_cross_tenant_are_indistinguishable() {
    let (store, registry, ops, scope, id, key) = fixture();
    let other_scope = TenantScope::new("other-org", "other-workspace");
    let cross_tenant = resolve_fixture(
        &store,
        &registry,
        &ops,
        &other_scope,
        id,
        key.clone(),
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("cross-tenant lookup must be hidden");
    let missing = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        CredentialId::new(),
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("missing lookup must fail");

    assert_eq!(cross_tenant, CredentialSlotResolveError::NotFound);
    assert_eq!(missing, cross_tenant);
    assert_eq!(cross_tenant.to_string(), missing.to_string());
}

#[tokio::test]
async fn cancellation_prevents_guard_publication() {
    let (store, registry, ops, scope, id, key) = fixture();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        cancel,
    )
    .await
    .expect_err("pre-cancelled resolution must fail");
    assert_eq!(error, CredentialSlotResolveError::Cancelled);
}

#[tokio::test]
async fn typed_extraction_rejects_a_different_scheme() {
    let (store, registry, ops, scope, id, key) = fixture();
    let erased = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect("fixture resolves");
    let error = erased
        .into_typed::<SharedKey>()
        .expect_err("different scheme must not downcast");
    assert_eq!(error, ErasedCredentialGuardTypeError);
}

#[tokio::test]
async fn debug_and_errors_do_not_expose_projected_secret() {
    let (store, registry, ops, scope, id, key) = fixture();
    let erased = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect("fixture resolves");
    let debug = format!("{erased:?}");
    let extraction_error = erased
        .into_typed::<SharedKey>()
        .expect_err("different scheme must not downcast")
        .to_string();

    assert!(!debug.contains(SECRET_CANARY));
    assert!(!extraction_error.contains(SECRET_CANARY));
    assert!(debug.contains("REDACTED"));
}

#[test]
fn resolver_trait_is_object_safe() {
    fn accepts_object(_: Option<&dyn CredentialSlotResolver>) {}
    fn assert_implemented<T: CredentialSlotResolver>() {}
    accepts_object(None);
    assert_implemented::<CredentialService>();
    assert_implemented::<crate::CredentialProjectionRuntime>();
}

// ── Envelope fail-closed decode (ADR-0107 Seam 2) ─────────────────

#[tokio::test]
async fn envelope_wrapped_row_resolves_on_the_slot_surface() {
    let data = crate::serde_secret::expose_for_serialization(|| {
        let token = SecretToken::new(SecretString::new(SECRET_CANARY));
        crate::state_envelope::encode_state_payload(&token)
    })
    .expect("fixture envelope encodes");
    let (store, registry, ops, scope, id, key) = fixture_with_payload(
        SecretBytes::from(data),
        <SecretToken as CredentialState>::KIND,
        <SecretToken as CredentialState>::VERSION,
    );

    let erased = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key.clone(),
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect("an envelope-wrapped row resolves exactly like a legacy row");
    let typed = erased
        .into_typed::<SecretToken>()
        .expect("registered scheme type extracts");
    assert_eq!(typed.token().expose_secret(), SECRET_CANARY);
}

#[tokio::test]
async fn envelope_schema_fingerprint_mismatch_refuses_as_stored_state_refused() {
    let honest = <SecretToken as StateWireFingerprint>::SCHEMA_FINGERPRINT;
    let (store, registry, ops, scope, id, key) = fixture_with_payload(
        SecretBytes::new(envelope_payload(
            Some(honest ^ 0x0000_0000_0000_0001),
            None,
            None,
        )),
        <SecretToken as CredentialState>::KIND,
        <SecretToken as CredentialState>::VERSION,
    );

    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("a stored fingerprint mismatch must refuse slot resolution");
    assert_eq!(
        error,
        CredentialSlotResolveError::StoredStateRefused(
            crate::StateEnvelopeError::SchemaFingerprintMismatch {
                expected: honest,
                stored: honest ^ 0x0000_0000_0000_0001,
            },
        ),
        "the slot surface must keep the refusal distinct from InvalidState"
    );
}

#[tokio::test]
async fn envelope_kind_tag_mismatch_refuses_as_stored_state_refused() {
    let (store, registry, ops, scope, id, key) = fixture_with_payload(
        SecretBytes::new(envelope_payload(None, Some("intruder_kind"), None)),
        <SecretToken as CredentialState>::KIND,
        <SecretToken as CredentialState>::VERSION,
    );

    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("a stored kind-tag mismatch must refuse slot resolution");
    assert_eq!(
        error,
        CredentialSlotResolveError::StoredStateRefused(crate::StateEnvelopeError::KindMismatch {
            expected: <SecretToken as CredentialState>::KIND,
        },),
        "the slot surface must keep the refusal distinct from InvalidState"
    );
}

#[tokio::test]
async fn envelope_unsupported_version_refuses_as_stored_state_refused() {
    let future = <SecretToken as CredentialState>::VERSION + 1;
    let (store, registry, ops, scope, id, key) = fixture_with_payload(
        SecretBytes::new(envelope_payload(None, None, Some(future))),
        <SecretToken as CredentialState>::KIND,
        future,
    );

    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("a stored shape newer than this build must refuse slot resolution");
    assert_eq!(
        error,
        CredentialSlotResolveError::StoredStateRefused(
            crate::StateEnvelopeError::UnknownSchemaVersion {
                stored_version: future,
                supported_version: <SecretToken as CredentialState>::VERSION,
            },
        ),
        "the slot surface must keep the refusal distinct from InvalidState"
    );
}

#[tokio::test]
async fn legacy_payload_with_future_row_version_refuses_as_stored_state_refused() {
    // A NON-envelope payload (plain legacy JSON) whose row axis exceeds
    // this build's VERSION must refuse on the legacy path's own version
    // check. The envelope-unsupported-version test above feeds an envelope
    // payload and hits the envelope's `interface_version` check; this pins
    // the sibling branch (state_envelope.rs legacy fallback) on the slot
    // surface.
    let future = <SecretToken as CredentialState>::VERSION + 1;
    let token = SecretToken::new(SecretString::new(SECRET_CANARY));
    let data = crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&token))
        .expect("test token serializes");
    let (store, registry, ops, scope, id, key) = fixture_with_payload(
        SecretBytes::new(data),
        <SecretToken as CredentialState>::KIND,
        future,
    );

    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("a legacy row claiming a future version must refuse slot resolution");
    assert_eq!(
        error,
        CredentialSlotResolveError::StoredStateRefused(
            crate::StateEnvelopeError::UnknownSchemaVersion {
                stored_version: future,
                supported_version: <SecretToken as CredentialState>::VERSION,
            },
        ),
        "the slot surface must keep the refusal distinct from InvalidState"
    );
}

#[tokio::test]
async fn projection_metadata_uses_durable_owner_without_interactive_binding() {
    let (store, registry, ops, scope, id, key) = fixture();
    let bound = scope.clone().with_authentication_binding(
        crate::CredentialAuthenticationBinding::parse("A".repeat(43)).expect("binding"),
    );
    let guard = resolve_fixture(
        &store,
        &registry,
        &ops,
        &bound,
        id,
        key.clone(),
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect("bound owner resolves");
    assert_eq!(guard.metadata().scope(), Some(&scope));
    let adapter = CredentialGuardMetadata::new(id, key, 1, 1).with_scope(bound);
    assert_eq!(adapter.scope(), Some(&scope));
}

/// A projection that meets a refresh crossing the provider boundary joins it
/// and is served once the refresh settles, instead of failing for its whole
/// provider call.
#[tokio::test(start_paused = true)]
async fn a_projection_joins_a_refresh_in_flight() {
    let (mut store, registry, ops, scope, id, key) = fixture();
    store.in_flight = Some((
        nebula_storage_port::store::CredentialOperationKind::Refresh,
        AtomicUsize::new(3),
    ));
    let started = tokio::time::Instant::now();
    let _guard = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect("the projection is served once the refresh settles");
    assert!(started.elapsed() < REFRESH_JOIN_WAIT);
    assert_eq!(
        store.material_loads.load(Ordering::Relaxed),
        1,
        "material is read only once the head admits it"
    );
}

/// A refresh still outstanding at the bound is reported as busy, with a retry
/// hint, never as a blocked credential.
#[tokio::test(start_paused = true)]
async fn a_refresh_outstanding_at_the_bound_is_busy_not_blocked() {
    let (mut store, registry, ops, scope, id, key) = fixture();
    store.in_flight = Some((
        nebula_storage_port::store::CredentialOperationKind::Refresh,
        AtomicUsize::new(usize::MAX),
    ));
    let started = tokio::time::Instant::now();
    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("a refresh that never settles");
    assert!(matches!(
        error,
        CredentialSlotResolveError::RefreshInFlight { .. }
    ));
    assert!(started.elapsed() <= REFRESH_JOIN_WAIT);
    assert_eq!(store.material_loads.load(Ordering::Relaxed), 0);
}

/// A revoke in flight is never joined: the projection is refused at once.
#[tokio::test(start_paused = true)]
async fn a_revoke_in_flight_is_refused_without_waiting() {
    let (mut store, registry, ops, scope, id, key) = fixture();
    store.in_flight = Some((
        nebula_storage_port::store::CredentialOperationKind::Revoke,
        AtomicUsize::new(usize::MAX),
    ));
    let started = tokio::time::Instant::now();
    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        CancellationToken::new(),
    )
    .await
    .expect_err("a revoke in flight blocks new material");
    assert_eq!(
        error,
        CredentialSlotResolveError::OperationBlocked {
            operation: nebula_storage_port::store::CredentialOperationKind::Revoke,
        }
    );
    assert_eq!(started.elapsed(), std::time::Duration::ZERO);
}

/// Cancelling a projection while it waits for a refresh ends the wait.
#[tokio::test(start_paused = true)]
async fn cancelling_a_joined_projection_ends_the_wait() {
    let (mut store, registry, ops, scope, id, key) = fixture();
    store.in_flight = Some((
        nebula_storage_port::store::CredentialOperationKind::Refresh,
        AtomicUsize::new(usize::MAX),
    ));
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        trigger.cancel();
    });
    let started = tokio::time::Instant::now();
    let error = resolve_fixture(
        &store,
        &registry,
        &ops,
        &scope,
        id,
        key,
        Capabilities::empty(),
        cancel,
    )
    .await
    .expect_err("cancelled while waiting");
    assert_eq!(error, CredentialSlotResolveError::Cancelled);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}
