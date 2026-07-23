//! Multi-replica credential-refresh chaos regression.
//!
//! Three independent [`RefreshCoordinator`] instances share one concrete
//! in-memory L2 adapter, matching the ownership topology used by replicas
//! backed by one database. A deterministic expiry driver repeatedly advances
//! credential epochs while concurrent workers race to refresh them. The
//! provider probe records every `(credential, epoch)` dispatch, making a
//! duplicate provider POST an exact test failure rather than a probabilistic
//! metric.
//!
//! The test is always ignored. Run the short local plane with:
//!
//! ```text
//! cargo nextest run -p nebula-storage --test refresh_coordinator_chaos \
//!   --run-ignored only
//! ```
//!
//! Nightly CI additionally enables `chaos-full`, widening the same harness to
//! 10 minutes, 100 credentials, and 24 workers.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use nebula_core::CredentialId;
use nebula_credential::runtime::{
    RefreshCoordConfig, RefreshCoordinator, RefreshDisposition, RefreshError,
};
use nebula_storage::credential::{InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId};
use parking_lot::Mutex;

#[derive(Clone, Copy, Debug)]
struct ChaosParams {
    duration: Duration,
    credentials: usize,
    replicas: usize,
    workers_per_replica: usize,
    expiry_period: Duration,
}

impl ChaosParams {
    #[cfg(not(feature = "chaos-full"))]
    fn selected() -> Self {
        Self {
            duration: Duration::from_secs(5),
            credentials: 5,
            replicas: 3,
            workers_per_replica: 3,
            expiry_period: Duration::from_millis(20),
        }
    }

    #[cfg(feature = "chaos-full")]
    fn selected() -> Self {
        Self {
            duration: Duration::from_mins(10),
            credentials: 100,
            replicas: 3,
            workers_per_replica: 8,
            expiry_period: Duration::from_millis(20),
        }
    }
}

#[derive(Debug)]
struct CredentialProbe {
    desired_epoch: AtomicU64,
    refreshed_epoch: AtomicU64,
    active_provider_calls: AtomicU64,
    total_provider_calls: AtomicU64,
}

impl CredentialProbe {
    fn new() -> Self {
        Self {
            // Every credential starts expired, which guarantees the harness
            // exercises real provider work before the expiry driver advances.
            desired_epoch: AtomicU64::new(1),
            refreshed_epoch: AtomicU64::new(0),
            active_provider_calls: AtomicU64::new(0),
            total_provider_calls: AtomicU64::new(0),
        }
    }

    fn needs_refresh(&self) -> bool {
        self.refreshed_epoch.load(Ordering::Acquire) < self.desired_epoch.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
struct CredentialHarness {
    id: CredentialId,
    probe: Arc<CredentialProbe>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit chaos workload; nightly CI runs it with --run-ignored only"]
async fn three_replicas_never_double_dispatch_one_refresh_epoch() {
    let params = ChaosParams::selected();
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let credentials: Arc<Vec<CredentialHarness>> = Arc::new(
        (0..params.credentials)
            .map(|_| CredentialHarness {
                id: CredentialId::new(),
                probe: Arc::new(CredentialProbe::new()),
            })
            .collect(),
    );
    let provider_calls_by_epoch = Arc::new(Mutex::new(HashMap::<(CredentialId, u64), u64>::new()));
    let coalesced_calls = Arc::new(AtomicU64::new(0));
    let confirmed_calls = Arc::new(AtomicU64::new(0));
    let work_cursor = Arc::new(AtomicUsize::new(0));
    let deadline = Instant::now() + params.duration;

    let coordinators: Vec<Arc<RefreshCoordinator>> = (0..params.replicas)
        .map(|replica| {
            Arc::new(
                RefreshCoordinator::new_with(
                    Arc::clone(&repo),
                    ReplicaId::new(format!("chaos-replica-{replica}")),
                    chaos_config(),
                )
                .expect("chaos configuration must satisfy coordinator invariants"),
            )
        })
        .collect();

    let expiry_credentials = Arc::clone(&credentials);
    let expiry_driver = tokio::spawn(async move {
        let mut index = 0usize;
        while Instant::now() < deadline {
            expiry_credentials[index]
                .probe
                .desired_epoch
                .fetch_add(1, Ordering::AcqRel);
            index = (index + 1) % expiry_credentials.len();
            tokio::time::sleep(params.expiry_period).await;
        }
    });

    let mut workers = Vec::with_capacity(params.replicas * params.workers_per_replica);
    for coordinator in coordinators {
        for _ in 0..params.workers_per_replica {
            let coordinator = Arc::clone(&coordinator);
            let credentials = Arc::clone(&credentials);
            let provider_calls_by_epoch = Arc::clone(&provider_calls_by_epoch);
            let coalesced_calls = Arc::clone(&coalesced_calls);
            let confirmed_calls = Arc::clone(&confirmed_calls);
            let work_cursor = Arc::clone(&work_cursor);

            workers.push(tokio::spawn(async move {
                while Instant::now() < deadline {
                    let index = work_cursor.fetch_add(1, Ordering::Relaxed) % credentials.len();
                    let credential = &credentials[index];
                    if !credential.probe.needs_refresh() {
                        tokio::task::yield_now().await;
                        continue;
                    }

                    let predicate_probe = Arc::clone(&credential.probe);
                    let provider_probe = Arc::clone(&credential.probe);
                    let calls = Arc::clone(&provider_calls_by_epoch);
                    let credential_id = credential.id;
                    let outcome = coordinator
                        .refresh_coalesced(
                            &credential.id,
                            move |_| {
                                let probe = Arc::clone(&predicate_probe);
                                async move { Ok(probe.needs_refresh()) }
                            },
                            move || async move {
                                let epoch = provider_probe.desired_epoch.load(Ordering::Acquire);
                                let active = provider_probe
                                    .active_provider_calls
                                    .fetch_add(1, Ordering::AcqRel)
                                    + 1;
                                assert_eq!(
                                    active, 1,
                                    "credential {credential_id} had concurrent provider calls"
                                );
                                provider_probe
                                    .total_provider_calls
                                    .fetch_add(1, Ordering::Relaxed);
                                {
                                    let mut calls = calls.lock();
                                    *calls.entry((credential_id, epoch)).or_insert(0) += 1;
                                }

                                tokio::time::sleep(Duration::from_millis(5)).await;
                                provider_probe
                                    .refreshed_epoch
                                    .fetch_max(epoch, Ordering::AcqRel);
                                provider_probe
                                    .active_provider_calls
                                    .fetch_sub(1, Ordering::AcqRel);
                                RefreshDisposition::state_advanced(())
                            },
                        )
                        .await;

                    match outcome {
                        Ok(()) => {
                            confirmed_calls.fetch_add(1, Ordering::Relaxed);
                        },
                        Err(RefreshError::CoalescedByOtherReplica) => {
                            coalesced_calls.fetch_add(1, Ordering::Relaxed);
                        },
                        Err(error) => {
                            panic!(
                                "unexpected chaos refresh failure for {}: {error}",
                                credential.id
                            );
                        },
                    }
                }
            }));
        }
    }

    expiry_driver
        .await
        .expect("expiry driver must finish without panicking");
    for worker in workers {
        worker
            .await
            .expect("refresh chaos worker must finish without panicking");
    }

    let calls = provider_calls_by_epoch.lock();
    assert!(!calls.is_empty(), "chaos run must dispatch provider work");
    for ((credential_id, epoch), count) in &*calls {
        assert_eq!(
            *count, 1,
            "credential {credential_id} epoch {epoch} dispatched the provider {count} times"
        );
    }
    drop(calls);

    let total_provider_calls: u64 = credentials
        .iter()
        .map(|credential| {
            assert_eq!(
                credential
                    .probe
                    .active_provider_calls
                    .load(Ordering::Acquire),
                0,
                "provider work leaked past test completion for {}",
                credential.id
            );
            credential
                .probe
                .total_provider_calls
                .load(Ordering::Relaxed)
        })
        .sum();
    assert_eq!(
        total_provider_calls,
        confirmed_calls.load(Ordering::Relaxed),
        "every provider dispatch must yield exactly one confirmed owner result"
    );
    assert!(
        coalesced_calls.load(Ordering::Relaxed) > 0,
        "workload did not exercise L1/L2 coalescing"
    );
}

fn chaos_config() -> RefreshCoordConfig {
    RefreshCoordConfig {
        claim_ttl: Duration::from_secs(3),
        heartbeat_interval: Duration::from_secs(1),
        refresh_timeout: Duration::from_millis(800),
        reclaim_sweep_interval: Duration::from_secs(1),
        sentinel_threshold: 3,
        sentinel_window: Duration::from_hours(1),
    }
}
