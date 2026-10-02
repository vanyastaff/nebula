//! One shared acceptance oracle for the fenced iteration-checkpoint store.
//!
//! The in-memory reference model, SQLite, and PostgreSQL implement the same
//! `CheckpointStore`, so they must answer every save and load identically:
//! fenced by the execution's live lease, monotone, exact on recommit, bound to
//! tenant, execution, node, action key and action version. Each backend's
//! test file supplies a store plus the execution store that owns its leases
//! and runs the same cases.
//!
//! Cases are keyed by a per-process namespace and a per-case seed so they can
//! share one durable store across runs.

use std::time::Duration;

use nebula_storage_port::store::{CheckpointStore, ExecutionStore};
use nebula_storage_port::{
    CheckpointSaved, FencingToken, IterationCheckpoint, IterationCheckpointError,
    IterationCheckpointKey, MAX_ITERATION_CHECKPOINT_STATE_BYTES, Scope,
};

/// Per-process namespace folded into every execution identity.
static NAMESPACE: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| uuid::Uuid::new_v4().simple().to_string());

pub(crate) fn scope() -> Scope {
    Scope::new("ws-checkpoint", "org-checkpoint")
}

/// A second tenant, used to prove one tenant cannot reach another's rows.
pub(crate) fn other_scope() -> Scope {
    Scope::new("ws-checkpoint-other", "org-checkpoint-other")
}

pub(crate) fn execution_id(seed: u8) -> String {
    format!("exe-ckpt-{}-{seed:02x}", *NAMESPACE)
}

const NODE: &str = "loop";
const ACTION: &str = "billing.poll";
const VERSION: &str = "1.0.0";

fn key<'a>(scope: &'a Scope, execution: &'a str) -> IterationCheckpointKey<'a> {
    IterationCheckpointKey::new(scope, execution, NODE, ACTION, VERSION).expect("valid key")
}

fn checkpoint(iteration: u32, digest: u8) -> IterationCheckpoint {
    IterationCheckpoint::new(
        iteration,
        format!("{{\"n\":{iteration}}}").into_bytes(),
        [digest; 32],
        Some(250),
        iteration,
        1,
    )
    .expect("valid checkpoint")
}

async fn create_execution(executions: &dyn ExecutionStore, scope: &Scope, execution: &str) {
    executions
        .create(
            scope,
            execution,
            "workflow",
            serde_json::json!({"status":"Created"}),
        )
        .await
        .unwrap();
}

async fn lease(
    executions: &dyn ExecutionStore,
    scope: &Scope,
    execution: &str,
    ttl: Duration,
) -> FencingToken {
    executions
        .acquire_lease(scope, execution, "runner", ttl)
        .await
        .unwrap()
        .expect("lease is free")
}

async fn leased_execution(
    executions: &dyn ExecutionStore,
    scope: &Scope,
    execution: &str,
) -> FencingToken {
    create_execution(executions, scope, execution).await;
    lease(executions, scope, execution, Duration::from_secs(30)).await
}

/// Runs every case against `$store`, an expression evaluating to a future of
/// `Option<(impl CheckpointStore, impl ExecutionStore)>`; `None` fails each
/// case loudly (an unreachable deployment backend is never a pass).
#[macro_export]
macro_rules! iteration_checkpoint_conformance_suite {
    ($store:expr) => {
        $crate::iteration_checkpoint_case!(an_absent_checkpoint_loads_as_none, 0x01, $store);
        $crate::iteration_checkpoint_case!(a_save_requires_a_live_fence, 0x02, $store);
        $crate::iteration_checkpoint_case!(a_stale_generation_is_rejected, 0x03, $store);
        $crate::iteration_checkpoint_case!(an_expired_lease_is_rejected, 0x04, $store);
        $crate::iteration_checkpoint_case!(saves_are_a_monotone_upsert, 0x05, $store);
        $crate::iteration_checkpoint_case!(an_exact_recommit_changes_nothing, 0x06, $store);
        $crate::iteration_checkpoint_case!(
            other_state_at_the_same_iteration_conflicts,
            0x07,
            $store
        );
        $crate::iteration_checkpoint_case!(a_lower_iteration_never_replaces_a_higher, 0x08, $store);
        $crate::iteration_checkpoint_case!(the_row_is_bound_to_its_whole_key, 0x09, $store);
        $crate::iteration_checkpoint_case!(the_largest_state_round_trips, 0x0A, $store);
        $crate::iteration_checkpoint_case!(a_checkpoint_round_trips_byte_exact, 0x0B, $store);
    };
}

#[macro_export]
macro_rules! iteration_checkpoint_case {
    ($case:ident, $seed:expr, $store:expr) => {
        #[tokio::test]
        async fn $case() {
            let Some((store, executions)) = $store.await else {
                panic!(concat!(
                    stringify!($case),
                    ": backend unreachable — the case cannot run and must fail rather than \
                     pass unchecked; reach the backend (set DATABASE_URL for postgres) or run \
                     without this feature"
                ));
            };
            oracle::$case(&store, &executions, $seed).await;
        }
    };
}

pub(crate) async fn an_absent_checkpoint_loads_as_none(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    assert_eq!(
        store
            .load_iteration_checkpoint(&key(&scope, &execution))
            .await,
        Ok(None),
        "no execution, no row"
    );
    leased_execution(executions, &scope, &execution).await;
    assert_eq!(
        store
            .load_iteration_checkpoint(&key(&scope, &execution))
            .await,
        Ok(None),
        "an execution without a checkpoint has none"
    );
}

pub(crate) async fn a_save_requires_a_live_fence(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let invented = FencingToken::from_generation(u64::MAX >> 1);
    assert_eq!(
        store
            .save_iteration_checkpoint(&key(&scope, &execution), &checkpoint(1, 1), invented)
            .await,
        Err(IterationCheckpointError::ExecutionLeaseRejected),
        "a missing execution authorizes nothing"
    );
    let fencing = leased_execution(executions, &scope, &execution).await;
    assert_eq!(
        store
            .save_iteration_checkpoint(&key(&scope, &execution), &checkpoint(1, 1), invented)
            .await,
        Err(IterationCheckpointError::ExecutionLeaseRejected),
        "an invented generation is not the lease"
    );
    let foreign = other_scope();
    assert_eq!(
        store
            .save_iteration_checkpoint(&key(&foreign, &execution), &checkpoint(1, 1), fencing)
            .await,
        Err(IterationCheckpointError::ExecutionLeaseRejected),
        "another tenant's execution identity is not this lease"
    );
    executions
        .release_lease(&scope, &execution, fencing)
        .await
        .unwrap();
    assert_eq!(
        store
            .save_iteration_checkpoint(&key(&scope, &execution), &checkpoint(1, 1), fencing)
            .await,
        Err(IterationCheckpointError::ExecutionLeaseRejected),
        "a released lease authorizes nothing"
    );
    assert_eq!(
        store
            .load_iteration_checkpoint(&key(&scope, &execution))
            .await,
        Ok(None),
        "every refusal wrote nothing"
    );
}

pub(crate) async fn a_stale_generation_is_rejected(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let stale = leased_execution(executions, &scope, &execution).await;
    executions
        .release_lease(&scope, &execution, stale)
        .await
        .unwrap();
    let current = lease(executions, &scope, &execution, Duration::from_secs(30)).await;
    assert!(current > stale);
    assert_eq!(
        store
            .save_iteration_checkpoint(&key(&scope, &execution), &checkpoint(2, 1), stale)
            .await,
        Err(IterationCheckpointError::ExecutionLeaseRejected)
    );
    assert_eq!(
        store
            .save_iteration_checkpoint(&key(&scope, &execution), &checkpoint(2, 1), current)
            .await,
        Ok(CheckpointSaved::Recorded)
    );
    let stored = store
        .load_iteration_checkpoint(&key(&scope, &execution))
        .await
        .unwrap()
        .expect("recorded");
    assert_eq!(stored.fencing_generation(), current.generation());
}

pub(crate) async fn an_expired_lease_is_rejected(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    create_execution(executions, &scope, &execution).await;
    // Every backend clamps a lease to at least one second.
    let fencing = lease(executions, &scope, &execution, Duration::from_secs(1)).await;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(
        store
            .save_iteration_checkpoint(&key(&scope, &execution), &checkpoint(1, 1), fencing)
            .await,
        Err(IterationCheckpointError::ExecutionLeaseRejected),
        "the current generation of an expired lease authorizes nothing"
    );
    assert_eq!(
        store
            .load_iteration_checkpoint(&key(&scope, &execution))
            .await,
        Ok(None)
    );
}

pub(crate) async fn saves_are_a_monotone_upsert(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let fencing = leased_execution(executions, &scope, &execution).await;
    let key = key(&scope, &execution);
    assert_eq!(
        store
            .save_iteration_checkpoint(&key, &checkpoint(1, 1), fencing)
            .await,
        Ok(CheckpointSaved::Recorded)
    );
    assert_eq!(
        store
            .save_iteration_checkpoint(&key, &checkpoint(3, 3), fencing)
            .await,
        Ok(CheckpointSaved::Recorded),
        "a later iteration replaces an earlier one"
    );
    let stored = store
        .load_iteration_checkpoint(&key)
        .await
        .unwrap()
        .expect("recorded");
    assert_eq!(stored.iteration(), 3);
    assert_eq!(stored.state_digest(), &[3; 32]);
    assert_eq!(stored.state(), b"{\"n\":3}");
    assert_eq!(stored.fencing_generation(), fencing.generation());
    assert!(
        stored.written_at_ms() > 0,
        "the adapter stamps its own clock"
    );
}

pub(crate) async fn an_exact_recommit_changes_nothing(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let first = leased_execution(executions, &scope, &execution).await;
    let key = key(&scope, &execution);
    assert_eq!(
        store
            .save_iteration_checkpoint(&key, &checkpoint(2, 2), first)
            .await,
        Ok(CheckpointSaved::Recorded)
    );
    let original = store.load_iteration_checkpoint(&key).await.unwrap();
    // The acknowledgement was lost and a later owner recommits it.
    executions
        .release_lease(&scope, &execution, first)
        .await
        .unwrap();
    let second = lease(executions, &scope, &execution, Duration::from_secs(30)).await;
    assert_eq!(
        store
            .save_iteration_checkpoint(&key, &checkpoint(2, 2), second)
            .await,
        Ok(CheckpointSaved::AlreadyRecorded)
    );
    assert_eq!(
        store.load_iteration_checkpoint(&key).await.unwrap(),
        original,
        "an exact recommit keeps the stored row, provenance included"
    );
}

pub(crate) async fn other_state_at_the_same_iteration_conflicts(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let fencing = leased_execution(executions, &scope, &execution).await;
    let key = key(&scope, &execution);
    store
        .save_iteration_checkpoint(&key, &checkpoint(4, 4), fencing)
        .await
        .unwrap();
    assert_eq!(
        store
            .save_iteration_checkpoint(&key, &checkpoint(4, 5), fencing)
            .await,
        Err(IterationCheckpointError::Conflict)
    );
    let stored = store
        .load_iteration_checkpoint(&key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.state_digest(),
        &[4; 32],
        "the conflict wrote nothing"
    );
}

pub(crate) async fn a_lower_iteration_never_replaces_a_higher(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let fencing = leased_execution(executions, &scope, &execution).await;
    let key = key(&scope, &execution);
    store
        .save_iteration_checkpoint(&key, &checkpoint(5, 5), fencing)
        .await
        .unwrap();
    assert_eq!(
        store
            .save_iteration_checkpoint(&key, &checkpoint(3, 3), fencing)
            .await,
        Err(IterationCheckpointError::Regressed { stored: 5 })
    );
    let stored = store
        .load_iteration_checkpoint(&key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.iteration(), 5, "the regression wrote nothing");
}

pub(crate) async fn the_row_is_bound_to_its_whole_key(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let fencing = leased_execution(executions, &scope, &execution).await;
    store
        .save_iteration_checkpoint(&key(&scope, &execution), &checkpoint(2, 2), fencing)
        .await
        .unwrap();
    let foreign = other_scope();
    let sibling = format!("{execution}-sibling");
    let invisible = [
        IterationCheckpointKey::new(&scope, &execution, NODE, ACTION, "2.0.0").unwrap(),
        IterationCheckpointKey::new(&scope, &execution, NODE, "billing.other", VERSION).unwrap(),
        IterationCheckpointKey::new(&scope, &execution, "other-node", ACTION, VERSION).unwrap(),
        IterationCheckpointKey::new(&scope, &sibling, NODE, ACTION, VERSION).unwrap(),
        IterationCheckpointKey::new(&foreign, &execution, NODE, ACTION, VERSION).unwrap(),
    ];
    for other in &invisible {
        assert_eq!(
            store.load_iteration_checkpoint(other).await,
            Ok(None),
            "{other:?} must not see the row"
        );
    }
    // Another version of the same node is its own row: it starts afresh and
    // never touches the first.
    let redeployed =
        IterationCheckpointKey::new(&scope, &execution, NODE, ACTION, "2.0.0").unwrap();
    assert_eq!(
        store
            .save_iteration_checkpoint(&redeployed, &checkpoint(1, 9), fencing)
            .await,
        Ok(CheckpointSaved::Recorded)
    );
    let original = store
        .load_iteration_checkpoint(&key(&scope, &execution))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.iteration(), 2);
    assert_eq!(original.state_digest(), &[2; 32]);
}

pub(crate) async fn the_largest_state_round_trips(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let fencing = leased_execution(executions, &scope, &execution).await;
    let state = vec![b'x'; MAX_ITERATION_CHECKPOINT_STATE_BYTES];
    let largest = IterationCheckpoint::new(7, state.clone(), [7; 32], None, 0, 1).unwrap();
    assert_eq!(
        store
            .save_iteration_checkpoint(&key(&scope, &execution), &largest, fencing)
            .await,
        Ok(CheckpointSaved::Recorded)
    );
    let stored = store
        .load_iteration_checkpoint(&key(&scope, &execution))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state().len(), MAX_ITERATION_CHECKPOINT_STATE_BYTES);
    // Compared byte by byte without printing a mebibyte on failure.
    assert!(
        stored.state().iter().all(|byte| *byte == b'x'),
        "the largest state must round-trip unchanged"
    );
}

pub(crate) async fn a_checkpoint_round_trips_byte_exact(
    store: &dyn CheckpointStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = execution_id(seed);
    let fencing = leased_execution(executions, &scope, &execution).await;
    let key = key(&scope, &execution);
    // Every byte value, NUL and non-UTF-8 included: the store keeps bytes.
    let state: Vec<u8> = (0..=255_u8).rev().chain(0..=255_u8).collect();
    let digest: [u8; 32] = std::array::from_fn(|index| u8::try_from(index).unwrap() ^ 0xA5);
    let saved = IterationCheckpoint::new(
        10_000,
        state.clone(),
        digest,
        Some(u64::try_from(i64::MAX).unwrap()),
        u32::MAX >> 1,
        u64::try_from(i64::MAX).unwrap(),
    )
    .unwrap();
    store
        .save_iteration_checkpoint(&key, &saved, fencing)
        .await
        .unwrap();
    let stored = store
        .load_iteration_checkpoint(&key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.iteration(), 10_000);
    assert_eq!(stored.state(), state.as_slice());
    assert_eq!(stored.state_digest(), &digest);
    assert_eq!(stored.resume_delay_ms(), saved.resume_delay_ms());
    assert_eq!(stored.attested_positions(), u32::MAX >> 1);
    assert_eq!(stored.attempt_generation(), saved.attempt_generation());

    // No delay is kept as no delay.
    let execution = format!("{execution}-nodelay");
    let fencing = leased_execution(executions, &scope, &execution).await;
    let undelayed = IterationCheckpoint::new(1, b"[]".to_vec(), [1; 32], None, 0, 0).unwrap();
    store
        .save_iteration_checkpoint(&self::key(&scope, &execution), &undelayed, fencing)
        .await
        .unwrap();
    let stored = store
        .load_iteration_checkpoint(&self::key(&scope, &execution))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resume_delay_ms(), None);
    assert_eq!(stored.attempt_generation(), 0);
}
