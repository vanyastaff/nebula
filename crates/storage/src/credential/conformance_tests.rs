//! One backend-neutral semantic oracle for the credential lifecycle.
//!
//! The reference adapter, SQLite, and PostgreSQL all execute the same body.
//! SQL-specific migration, restart, race, and transport-fault evidence lives in
//! the dedicated backend tests; this file proves behavioural parity.

use std::{error::Error, time::Duration};

use nebula_core::CredentialId;
use nebula_storage_port::{
    CredentialAdmissionEpoch, CredentialAlreadyExistsKey, CredentialCreate, CredentialMaterial,
    CredentialMaterialEpoch, CredentialMaterialTransition, CredentialOperationStatus,
    CredentialOwner, CredentialPersistenceError, CredentialRecordState, CredentialRefreshHorizon,
    CredentialRefreshPageSize, CredentialReplacement, CredentialReplacementFence,
    CredentialSelector, CredentialTombstone, CredentialVersion, MaterialUpdate,
    RefreshRetryAdmission, RefreshRetryBlock, RefreshRetryDelay, RefreshRetryDiagnosticCode,
    RefreshRetryEvidence, RefreshRetryKind, RefreshRetryPhase, RefreshRetryProjection,
    RefreshRetryTransition, SecretBytes, StoredCredential,
};
use serde_json::{Map, Value};

#[cfg(feature = "postgres")]
use super::PgCredentialPersistence;
#[cfg(feature = "sqlite")]
use super::SqliteCredentialPersistence;
use super::{CredentialPersistenceConformance, ReferenceCredentialPersistence};

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn owner(value: &str) -> CredentialOwner {
    CredentialOwner::from_canonical(value)
}

fn selector(owner: &CredentialOwner, credential_id: CredentialId) -> CredentialSelector {
    CredentialSelector::new(owner.clone(), credential_id)
}

fn metadata(name: Option<&str>, marker: &str) -> Map<String, Value> {
    let mut metadata = Map::from_iter([("opaque".to_owned(), Value::String(marker.to_owned()))]);
    if let Some(name) = name {
        metadata.insert(
            "display".to_owned(),
            serde_json::json!({"display_name": name}),
        );
    }
    metadata
}

fn create(name: Option<&str>, secret: &[u8], marker: &str) -> CredentialCreate {
    create_expiring(name, secret, marker, None, false)
}

fn create_expiring(
    name: Option<&str>,
    secret: &[u8],
    marker: &str,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    reauth_required: bool,
) -> CredentialCreate {
    CredentialCreate::new(
        "provider.api-token".to_owned(),
        SecretBytes::new(secret.to_vec()),
        "active".to_owned(),
        7,
        name.map(str::to_owned),
        expires_at,
        reauth_required,
        metadata(name, marker),
    )
}

/// New material for an `Advance`: state version 8 and no expiry, so every
/// assertion can tell it from the version-7 material a create installs.
fn material(secret: &[u8]) -> MaterialUpdate {
    MaterialUpdate::Replace(CredentialMaterial::new(
        SecretBytes::new(secret.to_vec()),
        "active".to_owned(),
        8,
        None,
    ))
}

fn replace_material(secret: &[u8]) -> CredentialMaterialTransition {
    CredentialMaterialTransition::advance(material(secret))
}

fn replacement(
    expected: CredentialVersion,
    name: Option<&str>,
    marker: &str,
    material_transition: CredentialMaterialTransition,
) -> CredentialReplacement {
    CredentialReplacement::new(
        expected,
        name.map(str::to_owned),
        true,
        metadata(name, marker),
        material_transition,
    )
}

async fn run_semantic_oracle<B>(store: &B) -> TestResult<()>
where
    B: CredentialPersistenceConformance + ?Sized,
{
    let owner_a = owner("oracle-owner-a");
    let owner_b = owner("oracle-owner-b");
    let schedule_owner = owner("oracle-schedule-owner");

    let common_expiry = chrono::Utc::now() + chrono::Duration::minutes(5);
    let mut due_ids = [CredentialId::new(), CredentialId::new()];
    due_ids.sort_unstable();
    for (index, credential_id) in due_ids.into_iter().enumerate() {
        store
            .create(
                &selector(&schedule_owner, credential_id),
                create_expiring(
                    None,
                    format!("due-{index}").as_bytes(),
                    "due",
                    Some(common_expiry),
                    false,
                ),
            )
            .await?;
    }
    let blocked_id = CredentialId::new();
    let blocked_key = selector(&schedule_owner, blocked_id);
    let blocked = store
        .create(
            &blocked_key,
            create_expiring(None, b"blocked", "blocked", Some(common_expiry), false),
        )
        .await?;
    let schedule_evidence = RefreshRetryEvidence::new(
        RefreshRetryPhase::BeforeDispatch,
        RefreshRetryKind::TransientNetwork,
        None,
    );
    store
        .replace(
            &blocked_key,
            CredentialReplacement::new(
                blocked.version(),
                None,
                false,
                metadata(None, "blocked-v2"),
                CredentialMaterialTransition::preserve(RefreshRetryTransition::SetNever {
                    evidence: schedule_evidence.clone(),
                }),
            ),
        )
        .await?;
    let deferred_id = CredentialId::new();
    let deferred_key = selector(&schedule_owner, deferred_id);
    let deferred = store
        .create(
            &deferred_key,
            create_expiring(None, b"deferred", "deferred", Some(common_expiry), false),
        )
        .await?;
    store
        .replace(
            &deferred_key,
            CredentialReplacement::new(
                deferred.version(),
                None,
                false,
                metadata(None, "deferred-v2"),
                CredentialMaterialTransition::preserve(RefreshRetryTransition::SetAfter {
                    delay: RefreshRetryDelay::new(Duration::from_mins(1))?,
                    evidence: schedule_evidence,
                }),
            ),
        )
        .await?;
    let reauth_id = CredentialId::new();
    store
        .create(
            &selector(&schedule_owner, reauth_id),
            create_expiring(None, b"reauth", "reauth", Some(common_expiry), true),
        )
        .await?;
    let tombstoned_id = CredentialId::new();
    let tombstoned_key = selector(&schedule_owner, tombstoned_id);
    let tombstoned = store
        .create(
            &tombstoned_key,
            create_expiring(
                None,
                b"tombstoned",
                "tombstoned",
                Some(common_expiry),
                false,
            ),
        )
        .await?;
    store
        .tombstone(
            &tombstoned_key,
            CredentialTombstone::new(tombstoned.version()),
        )
        .await?;
    let far_id = CredentialId::new();
    store
        .create(
            &selector(&schedule_owner, far_id),
            create_expiring(
                None,
                b"far",
                "far",
                Some(common_expiry + chrono::Duration::hours(2)),
                false,
            ),
        )
        .await?;

    let horizon = CredentialRefreshHorizon::new(Duration::from_mins(10))?;
    let one = CredentialRefreshPageSize::new(1)?;
    let first_page = store.scan_due(None, horizon, one).await?;
    assert_eq!(first_page.len(), 1, "schedule pages must honor their bound");
    assert_eq!(first_page[0].selector().credential_id(), due_ids[0]);
    assert!(first_page[0].observed_at() <= common_expiry);
    let second_page = store
        .scan_due(Some(&first_page[0].cursor()), horizon, one)
        .await?;
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page[0].selector().credential_id(), due_ids[1]);
    assert!(
        store
            .scan_due(Some(&second_page[0].cursor()), horizon, one)
            .await?
            .is_empty(),
        "blocked, deferred, reauth, and outside-horizon rows must not leak into the schedule"
    );

    let credential_id = CredentialId::new();
    let key = selector(&owner_a, credential_id);

    let gate_id = CredentialId::new();
    let gate_key = selector(&owner_a, gate_id);
    let gate_created = store
        .create(&gate_key, create(None, b"gate-v1", "gate-create"))
        .await?;
    let created_snapshot = store.refresh_retry_snapshot(&gate_key).await?;
    assert_eq!(created_snapshot.version(), gate_created.version());
    assert_eq!(
        created_snapshot.material_epoch(),
        CredentialMaterialEpoch::MIN
    );
    assert!(!created_snapshot.reauth_required());
    assert_eq!(created_snapshot.admission(), &RefreshRetryAdmission::Open);
    let gate_live = store.get(&gate_key).await?;
    assert!(
        gate_live
            .as_live()
            .expect("created gate fixture is live")
            .refresh_retry_gate()
            .is_none(),
        "create must start with a clear structural retry gate"
    );

    let evidence = RefreshRetryEvidence::new(
        RefreshRetryPhase::BeforeDispatch,
        RefreshRetryKind::TransientNetwork,
        Some(RefreshRetryDiagnosticCode::parse("oauth.network")?),
    );
    let set_never = replacement(
        gate_created.version(),
        None,
        "gate-never",
        CredentialMaterialTransition::preserve(RefreshRetryTransition::SetNever {
            evidence: evidence.clone(),
        }),
    );
    let gate_never = store.replace(&gate_key, set_never).await?;
    let never_snapshot = store.refresh_retry_snapshot(&gate_key).await?;
    assert_eq!(never_snapshot.version(), gate_never.version());
    assert_eq!(
        never_snapshot.material_epoch(),
        created_snapshot.material_epoch(),
        "retry-gate finalization must preserve refresh authority"
    );
    assert!(never_snapshot.reauth_required());
    assert_eq!(
        never_snapshot.admission(),
        &RefreshRetryAdmission::Blocked(RefreshRetryBlock::Never {
            evidence: evidence.clone(),
        })
    );
    assert!(matches!(
        store
            .get(&gate_key)
            .await?
            .as_live()
            .and_then(|live| live.refresh_retry_gate()),
        Some(nebula_storage_port::RefreshRetryGate::Never { evidence: stored })
            if stored == &evidence
    ));
    assert_eq!(
        store.get_head(&gate_key).await?.refresh_retry(),
        Some(RefreshRetryProjection::Never),
        "management heads must omit adjudication evidence"
    );
    let never_live = store.get(&gate_key).await?;
    let never_live = never_live.as_live().expect("gated fixture remains live");
    assert_eq!(
        never_live.data().as_ref(),
        b"gate-v1",
        "Preserve carries no bytes: the stored material stays byte-identical"
    );
    assert_eq!(never_live.state_version(), 7);
    assert_eq!(never_live.expires_at(), None);

    let preserve = replacement(
        gate_never.version(),
        None,
        "gate-preserve",
        CredentialMaterialTransition::preserve(RefreshRetryTransition::Preserve),
    );
    let gate_preserved = store.replace(&gate_key, preserve).await?;
    assert!(matches!(
        store.refresh_retry_snapshot(&gate_key).await?.admission(),
        RefreshRetryAdmission::Blocked(RefreshRetryBlock::Never { .. })
    ));

    let clear = replacement(
        gate_preserved.version(),
        None,
        "gate-clear",
        CredentialMaterialTransition::preserve(RefreshRetryTransition::Clear),
    );
    let gate_cleared = store.replace(&gate_key, clear).await?;
    assert_eq!(
        store.refresh_retry_snapshot(&gate_key).await?.admission(),
        &RefreshRetryAdmission::Open
    );

    let delay = RefreshRetryDelay::new(Duration::from_millis(1_001))?;
    assert_eq!(delay.as_secs(), 2, "sub-second input must round upward");
    let set_after = replacement(
        gate_cleared.version(),
        None,
        "gate-after",
        CredentialMaterialTransition::preserve(RefreshRetryTransition::SetAfter {
            delay,
            evidence: evidence.clone(),
        }),
    );
    let gate_after = store.replace(&gate_key, set_after).await?;
    let persisted_not_before = store
        .get(&gate_key)
        .await?
        .as_live()
        .and_then(|live| live.refresh_retry_gate())
        .and_then(nebula_storage_port::RefreshRetryGate::not_before)
        .expect("timed gate stores an absolute backend-authored instant");
    assert_eq!(
        store.get_head(&gate_key).await?.refresh_retry(),
        Some(RefreshRetryProjection::NotBefore {
            not_before: persisted_not_before,
        })
    );
    assert!(store.list_heads(&owner_a, None).await?.iter().any(|head| {
        head.credential_id() == gate_id
            && head.refresh_retry()
                == Some(RefreshRetryProjection::NotBefore {
                    not_before: persisted_not_before,
                })
    }));
    match store
        .refresh_retry_snapshot(&gate_key)
        .await?
        .into_admission()
    {
        RefreshRetryAdmission::Blocked(RefreshRetryBlock::After {
            remaining,
            evidence: actual_evidence,
        }) => {
            assert!((1..=2).contains(&remaining.as_secs()));
            assert_eq!(actual_evidence, evidence);
        },
        other => panic!("fresh timed gate must block, got {other:?}"),
    }
    let gated_epoch = store
        .refresh_retry_snapshot(&gate_key)
        .await?
        .material_epoch();
    let byte_identical_reconnect = replacement(
        gate_after.version(),
        None,
        "gate-after",
        CredentialMaterialTransition::advance(MaterialUpdate::Unchanged),
    );
    let reconnected = store.replace(&gate_key, byte_identical_reconnect).await?;
    let reconnected_snapshot = store.refresh_retry_snapshot(&gate_key).await?;
    assert_eq!(reconnected_snapshot.material_epoch(), gated_epoch.next()?);
    let reconnected_live = store.get(&gate_key).await?;
    let reconnected_live = reconnected_live
        .as_live()
        .expect("reconnected fixture remains live");
    assert_eq!(
        reconnected_live.data().as_ref(),
        b"gate-v1",
        "Advance {{ Unchanged }} advances authority over the stored bytes"
    );
    assert_eq!(reconnected_live.state_version(), 7);
    assert_eq!(
        reconnected_snapshot.admission(),
        &RefreshRetryAdmission::Open,
        "new authority must never inherit the previous epoch's retry verdict"
    );
    assert_eq!(
        store
            .refresh_retry_snapshot(&selector(&owner_b, gate_id))
            .await
            .expect_err("wrong-owner gate reads are existence-hidden"),
        CredentialPersistenceError::NotFound
    );
    store
        .tombstone(&gate_key, CredentialTombstone::new(reconnected.version()))
        .await?;
    assert_eq!(
        store
            .refresh_retry_snapshot(&gate_key)
            .await
            .expect_err("tombstones are inert for refresh admission"),
        CredentialPersistenceError::NotFound
    );

    assert!(store.list(&owner_a, None).await?.is_empty());
    assert!(store.list_heads(&owner_a, None).await?.is_empty());
    assert!(!store.exists(&key).await?);

    let created = store
        .create(&key, create(Some("Production"), b"\x00\xff-v1", "create"))
        .await?;
    assert_eq!(created.credential_id(), credential_id);
    assert_eq!(created.version(), CredentialVersion::MIN);
    assert_eq!(created.state(), CredentialRecordState::Live);
    assert_eq!(created.tombstoned_at(), None);

    let live = match store.get(&key).await? {
        StoredCredential::Live(live) => live,
        StoredCredential::Tombstoned(_) => panic!("a create must produce a live record"),
    };
    assert_eq!(live.credential_id(), credential_id);
    assert_eq!(live.name(), Some("Production"));
    assert_eq!(live.credential_key(), "provider.api-token");
    assert_eq!(live.data().as_ref(), b"\x00\xff-v1");
    assert_eq!(live.version(), created.version());
    assert_eq!(live.material_epoch(), CredentialMaterialEpoch::MIN);
    assert_eq!(live.created_at(), created.created_at());
    assert_eq!(live.updated_at(), created.updated_at());
    assert!(store.exists(&key).await?);

    let head = store.get_head(&key).await?;
    assert_eq!(head.credential_id(), credential_id);
    assert_eq!(head.version(), CredentialVersion::MIN);
    assert_eq!(head.material_epoch(), live.material_epoch());
    assert_eq!(head.name(), Some("Production"));
    assert_eq!(store.list(&owner_a, Some("active")).await?, [credential_id]);
    assert_eq!(store.list_heads(&owner_a, None).await?.len(), 1);
    assert!(store.list(&owner_b, None).await?.is_empty());

    assert_eq!(
        store
            .create(&key, create(Some("Other"), b"same-id", "same-id"))
            .await
            .expect_err("same-owner id must remain reserved"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        }
    );
    assert_eq!(
        store
            .create(
                &selector(&owner_b, credential_id),
                create(Some("Foreign"), b"foreign", "foreign"),
            )
            .await
            .expect_err("foreign-owner global id collision is existence-hidden"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .create(
                &selector(&owner_a, CredentialId::new()),
                create(Some("Production"), b"same-name", "same-name"),
            )
            .await
            .expect_err("a live owner-local name is unique"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Name,
        }
    );

    let replaced = store
        .replace(
            &key,
            replacement(
                CredentialVersion::MIN,
                Some("Renamed"),
                "replace",
                replace_material(b"\x00\xff-v2"),
            ),
        )
        .await?;
    let version_two = CredentialVersion::MIN.next_live()?;
    assert_eq!(replaced.version(), version_two);
    assert_eq!(replaced.created_at(), created.created_at());
    let live = match store.get(&key).await? {
        StoredCredential::Live(live) => live,
        StoredCredential::Tombstoned(_) => panic!("replace must preserve live state"),
    };
    assert_eq!(live.name(), Some("Renamed"));
    assert_eq!(live.data().as_ref(), b"\x00\xff-v2");
    assert_eq!(live.state_version(), 8);
    assert_eq!(live.material_epoch(), CredentialMaterialEpoch::MIN.next()?);
    assert!(live.reauth_required());
    let current_epoch = CredentialMaterialEpoch::MIN.next()?;
    assert_eq!(
        store
            .replace(
                &key,
                replacement(
                    version_two,
                    Some("Epoch stale"),
                    "epoch-stale",
                    replace_material(b"epoch-stale"),
                )
                .with_fence(CredentialReplacementFence::new(
                    CredentialMaterialEpoch::MIN,
                    "provider.api-token".to_owned(),
                )),
            )
            .await
            .expect_err("stale material epoch must fail closed"),
        CredentialPersistenceError::VersionConflict {
            expected: version_two,
            actual: version_two,
        }
    );
    assert_eq!(
        store
            .replace(
                &key,
                replacement(
                    version_two,
                    Some("Key substituted"),
                    "key-substituted",
                    replace_material(b"key-substituted"),
                )
                .with_fence(CredentialReplacementFence::new(
                    current_epoch,
                    "provider.substituted".to_owned(),
                )),
            )
            .await
            .expect_err("credential-key substitution must fail closed"),
        CredentialPersistenceError::VersionConflict {
            expected: version_two,
            actual: version_two,
        }
    );
    assert_eq!(
        store
            .replace(
                &selector(&owner_b, credential_id),
                replacement(
                    version_two,
                    Some("Owner substituted"),
                    "owner-substituted",
                    replace_material(b"owner-substituted"),
                )
                .with_fence(CredentialReplacementFence::new(
                    current_epoch,
                    "provider.api-token".to_owned(),
                )),
            )
            .await
            .expect_err("owner substitution must preserve existence hiding"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .replace(
                &key,
                replacement(
                    CredentialVersion::MIN,
                    Some("Stale"),
                    "stale",
                    replace_material(b"stale"),
                ),
            )
            .await
            .expect_err("stale replace must report both typed versions"),
        CredentialPersistenceError::VersionConflict {
            expected: CredentialVersion::MIN,
            actual: version_two,
        }
    );
    assert_eq!(
        store
            .replace(
                &selector(&owner_b, credential_id),
                replacement(
                    CredentialVersion::MAX,
                    Some("Foreign"),
                    "foreign",
                    replace_material(b"foreign"),
                ),
            )
            .await
            .expect_err("foreign owner precedes version and exhaustion"),
        CredentialPersistenceError::NotFound
    );

    let occupied_id = CredentialId::new();
    let occupied_key = selector(&owner_a, occupied_id);
    store
        .create(
            &occupied_key,
            create(Some("Occupied"), b"occupied", "occupied"),
        )
        .await?;
    assert_eq!(
        store
            .tombstone(
                &selector(&owner_b, credential_id),
                CredentialTombstone::new(CredentialVersion::MAX),
            )
            .await
            .expect_err("foreign owner precedes tombstone version and exhaustion"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .tombstone(&key, CredentialTombstone::new(CredentialVersion::MIN))
            .await
            .expect_err("stale tombstone must report both typed versions"),
        CredentialPersistenceError::VersionConflict {
            expected: CredentialVersion::MIN,
            actual: version_two,
        }
    );
    assert_eq!(
        store
            .replace(
                &key,
                replacement(
                    version_two,
                    Some("Occupied"),
                    "name-collision",
                    replace_material(b"name-collision"),
                ),
            )
            .await
            .expect_err("replace cannot claim another live owner-local name"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Name,
        }
    );
    let unchanged_head = store.get_head(&key).await?;
    assert_eq!(unchanged_head.version(), version_two);
    assert_eq!(unchanged_head.name(), Some("Renamed"));

    let tombstoned = store
        .tombstone(&key, CredentialTombstone::new(version_two))
        .await?;
    assert_eq!(
        tombstoned.version(),
        version_two
            .next_tombstone()
            .expect("version two has headroom")
    );
    assert_eq!(tombstoned.state(), CredentialRecordState::Tombstoned);
    assert!(tombstoned.tombstoned_at().is_some());
    match store.get(&key).await? {
        StoredCredential::Tombstoned(record) => {
            assert_eq!(record.credential_id(), credential_id);
            assert_eq!(record.version(), tombstoned.version());
            assert_eq!(record.credential_key(), "provider.api-token");
            assert_eq!(record.state_kind(), "active");
        },
        StoredCredential::Live(_) => panic!("tombstone must be structural"),
    }
    assert_eq!(
        store
            .get_head(&key)
            .await
            .expect_err("management projection is live-only"),
        CredentialPersistenceError::NotFound
    );
    assert!(!store.exists(&key).await?);
    assert!(!store.list(&owner_a, None).await?.contains(&credential_id));
    assert_eq!(
        store
            .replace(
                &key,
                replacement(
                    CredentialVersion::MAX,
                    Some("Resurrect"),
                    "resurrect",
                    replace_material(b"resurrect"),
                ),
            )
            .await
            .expect_err("terminal state precedes stale version and exhaustion"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .tombstone(&key, CredentialTombstone::new(CredentialVersion::MAX))
            .await
            .expect_err("terminal tombstone is not repeatable as a mutation"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .create(&key, create(Some("Resurrect"), b"resurrect", "resurrect"),)
            .await
            .expect_err("a tombstone reserves the id forever"),
        CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        }
    );

    let reused_id = CredentialId::new();
    let reused_key = selector(&owner_a, reused_id);
    let reused = store
        .create(
            &reused_key,
            create(Some("Renamed"), b"reused-name", "reused-name"),
        )
        .await?;
    assert_eq!(reused.credential_id(), reused_id);
    assert!(store.exists(&reused_key).await?);

    let missing = selector(&owner_a, CredentialId::new());
    assert_eq!(
        store
            .replace(
                &missing,
                replacement(
                    CredentialVersion::MAX,
                    None,
                    "missing",
                    replace_material(b"missing"),
                ),
            )
            .await
            .expect_err("missing row precedes version exhaustion"),
        CredentialPersistenceError::NotFound
    );
    assert_eq!(
        store
            .tombstone(&missing, CredentialTombstone::new(CredentialVersion::MAX))
            .await
            .expect_err("missing row precedes tombstone exhaustion"),
        CredentialPersistenceError::NotFound
    );

    let malformed = CredentialCreate::new(
        "provider.api-token".to_owned(),
        SecretBytes::new(Vec::new()),
        "active".to_owned(),
        0,
        Some("Physical".to_owned()),
        None,
        false,
        metadata(Some("Different"), "mismatch"),
    );
    assert_eq!(
        store
            .create(&selector(&owner_a, CredentialId::new()), malformed)
            .await
            .expect_err("physical/display name mismatch must fail before storage"),
        CredentialPersistenceError::CorruptRecord
    );

    let unnamed_id = CredentialId::new();
    let unnamed_key = selector(&owner_b, unnamed_id);
    store
        .create(&unnamed_key, create(None, b"", "unnamed-empty"))
        .await?;
    let unnamed = match store.get(&unnamed_key).await? {
        StoredCredential::Live(live) => live,
        StoredCredential::Tombstoned(_) => panic!("unnamed empty credential must be live"),
    };
    assert_eq!(unnamed.name(), None);
    assert!(unnamed.data().is_empty());

    let dual_id = CredentialId::new();
    let dual_key = selector(&owner_b, dual_id);
    let (dual_a, dual_b) = tokio::join!(
        store.create(
            &dual_key,
            create(Some("Dual Collision"), b"dual-a", "dual-a")
        ),
        store.create(
            &dual_key,
            create(Some("Dual Collision"), b"dual-b", "dual-b")
        )
    );
    let dual_outcomes: [_; 2] = (dual_a, dual_b).into();
    assert_eq!(
        dual_outcomes
            .iter()
            .filter(|outcome| outcome.is_ok())
            .count(),
        1
    );
    assert_eq!(
        dual_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    Err(CredentialPersistenceError::AlreadyExists {
                        key: CredentialAlreadyExistsKey::Id,
                    })
                )
            })
            .count(),
        1,
        "simultaneous id+name collision must classify as id"
    );

    let headroom_id = CredentialId::new();
    let headroom_key = selector(&owner_b, headroom_id);
    store
        .create(&headroom_key, create(None, b"last-live", "headroom"))
        .await?;
    store
        .force_live_version_for_conformance(&headroom_key, CredentialVersion::MAX_LIVE)
        .await?;
    for fence in [
        CredentialReplacementFence::new(
            CredentialMaterialEpoch::MIN.next()?,
            "provider.api-token".to_owned(),
        ),
        CredentialReplacementFence::new(
            CredentialMaterialEpoch::MIN,
            "provider.substituted".to_owned(),
        ),
    ] {
        assert_eq!(
            store
                .replace(
                    &headroom_key,
                    replacement(
                        CredentialVersion::MAX_LIVE,
                        None,
                        "fenced-overflow",
                        replace_material(b"fenced-overflow"),
                    )
                    .with_fence(fence),
                )
                .await
                .expect_err("strict fence must be checked before version exhaustion"),
            CredentialPersistenceError::VersionConflict {
                expected: CredentialVersion::MAX_LIVE,
                actual: CredentialVersion::MAX_LIVE,
            }
        );
    }
    assert_eq!(
        store
            .replace(
                &headroom_key,
                replacement(
                    CredentialVersion::MAX_LIVE,
                    None,
                    "overflow",
                    replace_material(b"overflow"),
                ),
            )
            .await
            .expect_err("replace must preserve terminal version headroom"),
        CredentialPersistenceError::VersionExhausted
    );
    let terminal = store
        .tombstone(
            &headroom_key,
            CredentialTombstone::new(CredentialVersion::MAX_LIVE),
        )
        .await?;
    assert_eq!(terminal.version(), CredentialVersion::MAX);

    let epoch_headroom_id = CredentialId::new();
    let epoch_headroom_key = selector(&owner_b, epoch_headroom_id);
    let epoch_created = store
        .create(
            &epoch_headroom_key,
            create(None, b"epoch-last", "epoch-headroom"),
        )
        .await?;
    let epoch_gated = store
        .replace(
            &epoch_headroom_key,
            replacement(
                epoch_created.version(),
                None,
                "epoch-headroom",
                CredentialMaterialTransition::preserve(RefreshRetryTransition::SetNever {
                    evidence: evidence.clone(),
                }),
            ),
        )
        .await?;
    store
        .force_live_material_epoch_for_conformance(
            &epoch_headroom_key,
            CredentialMaterialEpoch::MAX,
        )
        .await?;
    let before_overflow = store.get(&epoch_headroom_key).await?;
    let before_overflow = before_overflow
        .as_live()
        .expect("epoch headroom fixture remains live");
    let before_version = before_overflow.version();
    let before_epoch = before_overflow.material_epoch();
    let before_gate = before_overflow.refresh_retry_gate().cloned();
    let before_data = before_overflow.data().clone();
    assert_eq!(before_version, epoch_gated.version());
    assert_eq!(before_epoch, CredentialMaterialEpoch::MAX);

    assert_eq!(
        store
            .replace(
                &epoch_headroom_key,
                replacement(
                    before_version,
                    None,
                    "epoch-overflow",
                    replace_material(b"must-not-commit"),
                ),
            )
            .await
            .expect_err("material authority cannot advance beyond its positive i64 range"),
        CredentialPersistenceError::MaterialEpochExhausted
    );
    let after_overflow = store.get(&epoch_headroom_key).await?;
    let after_overflow = after_overflow
        .as_live()
        .expect("failed epoch advance must leave the fixture live");
    assert_eq!(
        after_overflow.version(),
        before_version,
        "failed epoch advance must not consume the row CAS version"
    );
    assert_eq!(after_overflow.material_epoch(), before_epoch);
    assert_eq!(after_overflow.refresh_retry_gate(), before_gate.as_ref());
    assert_eq!(
        after_overflow.data(),
        &before_data,
        "failed epoch advance must not mutate material"
    );

    let corrupt_id = CredentialId::new();
    let corrupt_key = selector(&owner_b, corrupt_id);
    store
        .create(&corrupt_key, create(None, b"opaque", "corrupt"))
        .await?;
    store
        .corrupt_live_projection_for_conformance(&corrupt_key)
        .await?;
    assert_eq!(
        store
            .get(&corrupt_key)
            .await
            .expect_err("persisted malformed projection must fail closed"),
        CredentialPersistenceError::CorruptRecord
    );

    run_admission_epoch_oracle(store, &owner_b).await
}

/// The use revision observed through the authoritative operation status.
async fn admission_epoch_of<B>(
    store: &B,
    key: &CredentialSelector,
) -> TestResult<(
    CredentialAdmissionEpoch,
    CredentialVersion,
    CredentialMaterialEpoch,
)>
where
    B: CredentialPersistenceConformance + ?Sized,
{
    match store.operation_status(key).await? {
        CredentialOperationStatus::Open {
            version,
            material_epoch,
            admission_epoch,
            ..
        } => Ok((admission_epoch, version, material_epoch)),
        other => Err(format!("an unclaimed credential must be open, got {other:?}").into()),
    }
}

/// Replacement-side admission-epoch bumps: every `Advance` and every change of
/// `reauth_required` advances the use revision in the same write; display and
/// retry-gate writes leave it. Claim-side close edges live in the SQL
/// refresh-claim runners, because only those backends share one transaction
/// between claims and credentials.
async fn run_admission_epoch_oracle<B>(store: &B, owner: &CredentialOwner) -> TestResult<()>
where
    B: CredentialPersistenceConformance + ?Sized,
{
    let key = selector(owner, CredentialId::new());
    let created = store
        .create(&key, create(None, b"admission-v1", "admission"))
        .await?;
    let (epoch, version, material) = admission_epoch_of(store, &key).await?;
    assert_eq!(epoch, CredentialAdmissionEpoch::MIN, "create starts at MIN");
    assert_eq!(version, created.version());
    assert_eq!(
        store.get_operational_head(&key).await?.status(),
        store.operation_status(&key).await?,
        "every status read reports the same epoch"
    );
    assert_eq!(
        store.get_with_operation_status(&key).await?.1,
        Some(store.operation_status(&key).await?)
    );

    let replace = |expected, reauth_required, transition| {
        CredentialReplacement::new(
            expected,
            None,
            reauth_required,
            metadata(None, "admission"),
            transition,
        )
    };

    // A display edit preserves every admitted use.
    let display = store
        .replace(
            &key,
            replace(
                version,
                false,
                CredentialMaterialTransition::preserve(RefreshRetryTransition::Preserve),
            ),
        )
        .await?;
    assert_eq!(display.version(), version.next_live()?);
    assert_eq!(admission_epoch_of(store, &key).await?.0, epoch);

    // So does a retry-gate write.
    let gated = store
        .replace(
            &key,
            replace(
                display.version(),
                false,
                CredentialMaterialTransition::preserve(RefreshRetryTransition::SetNever {
                    evidence: RefreshRetryEvidence::new(
                        RefreshRetryPhase::BeforeDispatch,
                        RefreshRetryKind::TransientNetwork,
                        None,
                    ),
                }),
            ),
        )
        .await?;
    let (after_gate, gate_version, gate_material) = admission_epoch_of(store, &key).await?;
    assert_eq!(after_gate, epoch, "a retry-gate write closes no use");
    assert_eq!(gate_version, gated.version());
    assert_eq!(gate_material, material);

    // The durable reauthentication decision advances authority over the
    // stored bytes and closes every use.
    let reauth = store
        .replace(
            &key,
            replace(
                gated.version(),
                true,
                CredentialMaterialTransition::advance(MaterialUpdate::Unchanged),
            ),
        )
        .await?;
    let (after_reauth, reauth_version, reauth_material) = admission_epoch_of(store, &key).await?;
    assert_eq!(after_reauth, epoch.next()?);
    assert_eq!(reauth_version, reauth.version());
    assert_eq!(
        reauth_version,
        gated.version().next_live()?,
        "the admission bump rides the one replacement version"
    );
    assert_eq!(reauth_material, material.next()?);

    // New material advances it again.
    let replaced = store
        .replace(
            &key,
            replace(reauth.version(), false, replace_material(b"admission-v2")),
        )
        .await?;
    let (after_material, _, _) = admission_epoch_of(store, &key).await?;
    assert_eq!(after_material, after_reauth.next()?);

    // A preserve that flips `reauth_required` closes use too.
    let flipped = store
        .replace(
            &key,
            replace(
                replaced.version(),
                true,
                CredentialMaterialTransition::preserve(RefreshRetryTransition::Preserve),
            ),
        )
        .await?;
    let (after_flip, flip_version, flip_material) = admission_epoch_of(store, &key).await?;
    assert_eq!(after_flip, after_material.next()?);
    assert_eq!(flip_version, flipped.version());
    assert_eq!(
        flip_material,
        material.next()?.next()?,
        "a preserve never moves the material epoch"
    );

    // Overflow fails closed and commits nothing.
    store
        .force_live_admission_epoch_for_conformance(&key, CredentialAdmissionEpoch::MAX)
        .await?;
    let before = store.get(&key).await?;
    let before = before.as_live().expect("fixture remains live").clone();
    assert_eq!(
        store
            .replace(
                &key,
                replace(
                    flipped.version(),
                    false,
                    replace_material(b"must-not-commit"),
                ),
            )
            .await
            .expect_err("the admission epoch cannot advance past i64::MAX"),
        CredentialPersistenceError::AdmissionEpochExhausted
    );
    let after = store.get(&key).await?;
    let after = after.as_live().expect("fixture remains live");
    assert_eq!(after.version(), before.version());
    assert_eq!(after.material_epoch(), before.material_epoch());
    assert_eq!(after.data(), before.data());
    assert_eq!(after.reauth_required(), before.reauth_required());
    assert_eq!(
        admission_epoch_of(store, &key).await?.0,
        CredentialAdmissionEpoch::MAX
    );

    // A write that closes no use still succeeds at the terminal epoch.
    let display_at_max = store
        .replace(
            &key,
            replace(
                flipped.version(),
                true,
                CredentialMaterialTransition::preserve(RefreshRetryTransition::Preserve),
            ),
        )
        .await?;
    assert_eq!(display_at_max.version(), flipped.version().next_live()?);
    assert_eq!(
        admission_epoch_of(store, &key).await?.0,
        CredentialAdmissionEpoch::MAX
    );

    Ok(())
}

#[tokio::test]
async fn reference_semantic_oracle() -> TestResult<()> {
    let store = ReferenceCredentialPersistence::new();
    run_semantic_oracle(&store).await
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_semantic_oracle() -> TestResult<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("credential-oracle.sqlite3");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let store = SqliteCredentialPersistence::connect(&url).await?;
    run_semantic_oracle(&store).await
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_memory_semantic_oracle() -> TestResult<()> {
    let store = SqliteCredentialPersistence::connect_memory().await?;
    run_semantic_oracle(&store).await
}

#[cfg(feature = "postgres")]
mod postgres {
    use std::{
        str::FromStr,
        time::{SystemTime, UNIX_EPOCH},
    };

    use sqlx::{
        PgPool,
        postgres::{PgConnectOptions, PgPoolOptions},
    };

    use super::*;

    struct IsolatedSchema {
        admin: PgPool,
        options: PgConnectOptions,
        schema: String,
    }

    impl IsolatedSchema {
        async fn connect() -> Option<Self> {
            let url = match std::env::var("DATABASE_URL") {
                Ok(url) => url,
                Err(std::env::VarError::NotPresent) => {
                    assert!(
                        std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                        "NEBULA_REQUIRE_POSTGRES=1 but DATABASE_URL is absent"
                    );
                    return None;
                },
                Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
            };
            let admin = PgPoolOptions::new()
                .max_connections(2)
                .connect(&url)
                .await
                .expect("connect to DATABASE_URL");
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            let schema = format!("nebula_credential_oracle_{}_{nanos}", std::process::id());
            sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
                .execute(&admin)
                .await
                .expect("create isolated semantic-oracle schema");
            let options = PgConnectOptions::from_str(&url)
                .expect("parse DATABASE_URL")
                .options([("search_path", schema.as_str())]);
            Some(Self {
                admin,
                options,
                schema,
            })
        }

        async fn cleanup(self) {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP SCHEMA {} CASCADE",
                self.schema
            )))
            .execute(&self.admin)
            .await
            .expect("drop isolated semantic-oracle schema");
            self.admin.close().await;
        }
    }

    #[tokio::test]
    async fn postgres_semantic_oracle() -> TestResult<()> {
        let Some(database) = IsolatedSchema::connect().await else {
            return Ok(());
        };
        let store = PgCredentialPersistence::connect_with(database.options.clone()).await?;
        run_semantic_oracle(&store).await?;
        drop(store);
        database.cleanup().await;
        Ok(())
    }
}
