use super::*;
use std::sync::Arc;

#[derive(Default)]
struct FakeGuard(u32);
impl zeroize::Zeroize for FakeGuard {
    fn zeroize(&mut self) {
        self.0 = 0;
    }
}

#[test]
fn slot_cell_swaps_without_clone_and_reads_latest() {
    let cell: SlotCell<FakeGuard> = SlotCell::empty();
    assert!(cell.load().is_none());
    cell.store(Arc::new(FakeGuard(1)));
    assert_eq!(cell.load().expect("v1").0, 1);
    cell.store(Arc::new(FakeGuard(2)));
    assert_eq!(cell.load().expect("v2").0, 2);
}

#[test]
fn take_and_is_some() {
    let cell: SlotCell<FakeGuard> = SlotCell::empty();

    // Empty cell: is_some is false, take returns None.
    assert!(!cell.is_some());
    assert!(cell.take().is_none());

    // After store: is_some is true, take returns the value.
    cell.store(Arc::new(FakeGuard(1)));
    assert!(cell.is_some());
    let taken = cell.take();
    assert_eq!(taken.expect("should be Some").0, 1);

    // After take: cell is empty again.
    assert!(cell.load().is_none());
    assert!(!cell.is_some());

    // Second take on now-empty cell returns None.
    assert!(cell.take().is_none());
}

#[test]
fn generation_starts_at_zero_and_is_strictly_monotonic() {
    let cell: SlotCell<FakeGuard> = SlotCell::empty();
    // Never bound.
    assert_eq!(cell.generation(), 0, "unbound slot epoch is 0");
    assert!(cell.load_versioned().is_none());

    // First store -> generation 1, coupled to the value.
    cell.store(Arc::new(FakeGuard(10)));
    let (g1, v1) = cell.load_versioned().expect("bound");
    assert_eq!(g1, 1, "first store is generation 1");
    assert_eq!(v1.0, 10);
    assert_eq!(cell.generation(), 1);

    // Second store -> strictly greater generation, new value.
    cell.store(Arc::new(FakeGuard(20)));
    let (g2, v2) = cell.load_versioned().expect("bound");
    assert!(g2 > g1, "store must strictly advance the generation");
    assert_eq!(v2.0, 20);
}

#[test]
fn take_advances_generation_and_is_observable_when_empty() {
    let cell: SlotCell<FakeGuard> = SlotCell::empty();
    cell.store(Arc::new(FakeGuard(1)));
    let g_after_store = cell.generation();
    assert_eq!(g_after_store, 1);

    // A clear is a credential-state transition: the generation must
    // advance so an instance built against the pre-clear guard is
    // detectably stale, and it stays observable while empty.
    let _ = cell.take();
    assert!(cell.load().is_none(), "slot is cleared");
    let g_after_take = cell.generation();
    assert!(
        g_after_take > g_after_store,
        "take must strictly advance the generation (a clear is a transition)"
    );

    // Storing again after a clear keeps advancing.
    cell.store(Arc::new(FakeGuard(2)));
    assert!(cell.generation() > g_after_take);
}

#[test]
fn authoritative_install_rejects_out_of_order_refresh() {
    let cell = SlotCell::empty();
    assert_eq!(
        cell.install_at_material_epoch(7, Arc::new(FakeGuard(7))),
        Ok(SlotUpdate::Installed)
    );

    assert_eq!(
        cell.install_at_material_epoch(6, Arc::new(FakeGuard(6))),
        Ok(SlotUpdate::Stale {
            current_material_epoch: 7,
        })
    );
    let (epoch, guard) = cell
        .load_material_versioned()
        .expect("epoch 7 remains live");
    assert_eq!(epoch, 7);
    assert_eq!(guard.0, 7);
}

#[test]
fn revoke_wins_over_same_epoch_and_stale_refresh() {
    let cell = SlotCell::empty();
    assert_eq!(
        cell.install_at_material_epoch(4, Arc::new(FakeGuard(4))),
        Ok(SlotUpdate::Installed)
    );
    assert_eq!(cell.revoke(), SlotUpdate::Revoked);
    assert_eq!(cell.material_epoch(), None);
    assert!(cell.load().is_none());

    assert_eq!(
        cell.install_at_material_epoch(4, Arc::new(FakeGuard(40))),
        Ok(SlotUpdate::Revoked)
    );
    assert!(
        cell.load().is_none(),
        "same-epoch refresh must not resurrect"
    );
}

#[test]
fn terminal_revoke_is_idempotent() {
    let cell = SlotCell::empty();
    assert_eq!(
        cell.install_at_material_epoch(9, Arc::new(FakeGuard(9))),
        Ok(SlotUpdate::Installed)
    );

    assert_eq!(cell.revoke(), SlotUpdate::Revoked);
    assert_eq!(cell.revoke(), SlotUpdate::AlreadyRevoked);
    assert!(cell.load().is_none());
}

#[test]
fn reserved_revoke_authority_cannot_be_installed_as_material() {
    let cell = SlotCell::empty();
    assert_eq!(
        cell.install_at_material_epoch(REVOKED_AUTHORITY, Arc::new(FakeGuard(1))),
        Err(SlotInstallError::InvalidMaterialEpoch)
    );
    assert!(cell.load().is_none());
}

#[test]
fn take_on_never_bound_still_advances_generation() {
    let cell: SlotCell<FakeGuard> = SlotCell::empty();
    assert_eq!(cell.generation(), 0);
    // Even a no-op clear advances the generation: a "clear" signal is
    // meaningful to a rotation observer regardless of prior state.
    assert!(cell.take().is_none());
    assert!(
        cell.generation() > 0,
        "take advances generation even when the slot was already empty"
    );
}

/// Concurrency characterization (informs the single-writer-per-slot
/// question; not a fix). Many tasks race `store`/`take` on one cell.
/// `load_versioned` must never observe a torn `(generation, value)`
/// pair — the generation must be exactly the one published with that
/// value (each store stamps the value with its own generation), never a
/// generation from a different transition.
///
/// The coupling is what makes a torn read *detectable*: every entry is
/// published via `store_stamped` so its value is exactly its own
/// generation (`value == generation`). A single immutable `SlotEntry`
/// observed through one `ArcSwapOption` load must therefore always
/// satisfy `u64::from(value) == generation`. A torn read — the value of
/// one transition paired with the generation of another — would break
/// that equality and fail the assertion below.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_store_take_load_versioned_is_never_torn() {
    let cell: Arc<SlotCell<FakeGuard>> = Arc::new(SlotCell::empty());

    let writers = 8u32;
    let iters = 200u32;
    // Total transitions = stores + takes. The largest generation any
    // store stamps is bounded by this, so it fits in the `u32` payload
    // of `FakeGuard` and the `value == generation` round-trip is exact.
    let total_transitions = u64::from(writers) * u64::from(iters) * 2;
    assert!(
        u32::try_from(total_transitions).is_ok(),
        "test sizing must keep generations within FakeGuard's u32 payload"
    );

    let mut handles = Vec::new();
    for _ in 0..writers {
        let cell = Arc::clone(&cell);
        handles.push(tokio::spawn(async move {
            for _ in 0..iters {
                // Stamp the value with the *exact* generation this entry
                // is published at, inside the production single-store
                // publish. `g as u32` is lossless: `g <=
                // total_transitions <= u32::MAX` (asserted above).
                cell.store_stamped(|g| Arc::new(FakeGuard(g as u32)));
                if let Some((observed_gen, val)) = cell.load_versioned() {
                    assert!(
                        observed_gen >= 1,
                        "a published entry always has generation >= 1"
                    );
                    // The load-bearing torn-read check: value and
                    // generation came from one immutable entry, so the
                    // value must be the generation that entry stamped.
                    // A torn `(generation, value)` pair (value from a
                    // different transition than `observed_gen`) breaks
                    // this equality.
                    assert_eq!(
                        u64::from(val.0),
                        observed_gen,
                        "torn read: value {} was not stamped with its \
                         published generation {observed_gen}",
                        val.0
                    );
                }
                let _ = cell.take();
            }
        }));
    }
    for h in handles {
        h.await.expect("writer task must not panic");
    }

    // After all transitions the generation is strictly positive and
    // monotone: every store and every take bumped it exactly once, so
    // it is at least the total number of transitions performed.
    let total_transitions = u64::from(writers) * u64::from(iters) * 2;
    assert!(
        cell.generation() >= total_transitions,
        "generation must have advanced at least once per transition \
         (got {}, expected >= {total_transitions})",
        cell.generation()
    );
}

/// Reader/writer race: a dedicated reader continuously calls
/// `load_versioned` while a writer stores monotically-increasing
/// generations. The observed generation must be monotone non-decreasing
/// from this single reader's vantage (no torn read can surface a
/// generation older than one already observed paired with a newer
/// value).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_reader_observes_monotone_generations_under_concurrent_store() {
    let cell: Arc<SlotCell<FakeGuard>> = Arc::new(SlotCell::empty());

    let writer = {
        let cell = Arc::clone(&cell);
        tokio::spawn(async move {
            for i in 1..=1_000u32 {
                cell.store(Arc::new(FakeGuard(i)));
            }
        })
    };

    let reader = {
        let cell = Arc::clone(&cell);
        tokio::spawn(async move {
            let mut last = 0u64;
            for _ in 0..5_000 {
                if let Some((observed_gen, _v)) = cell.load_versioned() {
                    assert!(
                        observed_gen >= last,
                        "load_versioned regressed from {last} to \
                         {observed_gen} (torn read / lost publish \
                         ordering)"
                    );
                    last = observed_gen;
                }
            }
        })
    };

    writer.await.expect("writer task must not panic");
    reader.await.expect("reader task must not panic");
}

#[test]
fn projected_slot_rejects_another_credential_even_at_a_higher_epoch() {
    use nebula_credential::{CredentialGuardMetadata, CredentialId, TenantScope};
    let cell = SlotCell::<FakeGuard>::empty();
    let cid = CredentialId::new();
    let owner = TenantScope::new("org", "workspace");
    let metadata = CredentialGuardMetadata::new(cid, "oauth".parse().expect("key"), 1, 1)
        .with_scope(owner.clone());
    assert_eq!(
        cell.install_projected(metadata.clone(), Arc::new(FakeGuard(1)))
            .expect("install"),
        SlotUpdate::Installed
    );
    for incoming in [
        CredentialGuardMetadata::new(CredentialId::new(), "oauth".parse().expect("key"), 99, 99)
            .with_scope(owner),
        CredentialGuardMetadata::new(cid, "oauth".parse().expect("key"), 99, 99)
            .with_scope(TenantScope::new("other", "workspace")),
    ] {
        assert!(matches!(
            cell.install_projected(incoming, Arc::new(FakeGuard(99))),
            Err(SlotInstallError::CredentialIdentityMismatch)
        ));
    }
    assert_eq!(cell.projection_metadata(), Some(metadata));
    assert_eq!(cell.load().expect("original guard").0, 1);
    assert_eq!(cell.generation(), 1);
}

#[test]
fn unqualified_writes_clear_projection_identity_only_when_applied() {
    use nebula_credential::{CredentialGuardMetadata, CredentialId, TenantScope};
    for use_store in [false, true] {
        let cell = SlotCell::<FakeGuard>::empty();
        let metadata =
            CredentialGuardMetadata::new(CredentialId::new(), "oauth".parse().expect("key"), 2, 2)
                .with_scope(TenantScope::new("org", "workspace"));
        assert_eq!(
            cell.install_projected(metadata.clone(), Arc::new(FakeGuard(2)))
                .expect("projected"),
            SlotUpdate::Installed
        );
        assert!(matches!(
            cell.install_at_material_epoch(1, Arc::new(FakeGuard(1)))
                .expect("stale"),
            SlotUpdate::Stale { .. }
        ));
        assert_eq!(cell.projection_metadata(), Some(metadata));
        if use_store {
            cell.store(Arc::new(FakeGuard(3)));
        } else {
            assert_eq!(
                cell.install_at_material_epoch(3, Arc::new(FakeGuard(3)))
                    .expect("unqualified"),
                SlotUpdate::Installed
            );
        }
        assert_eq!(cell.projection_metadata(), None);
        assert_eq!(cell.load().expect("live").0, 3);
    }
}
