//! Join-next coalescing: every answer comes from a read issued after the
//! caller arrived, one read per lane at a time, re-election on abandonment.

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_credential::{
    CredentialAvailability, CredentialAvailabilityObservation, CredentialAvailabilityObserver,
    CredentialBlock, CredentialId, CredentialKey, CredentialObserveError, TenantScope,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::{CredentialAdmissionMetrics, CredentialReads, Published, ReadFailure};

/// A scripted, counting availability observer. Each call reads its answer
/// when it starts (so a read issued before a block committed answers with
/// the state before it), then, while gated, waits for a permit.
pub(crate) struct ScriptedObserver {
    sticky: Mutex<Published>,
    queued: Mutex<VecDeque<Published>>,
    per_credential: Mutex<HashMap<CredentialId, Published>>,
    calls: AtomicUsize,
    gated: AtomicBool,
    gate: Semaphore,
}

impl ScriptedObserver {
    /// Answers `answer` to every call, ungated.
    pub(crate) fn answering(answer: Published) -> Arc<Self> {
        Arc::new(Self {
            sticky: Mutex::new(answer),
            queued: Mutex::new(VecDeque::new()),
            per_credential: Mutex::new(HashMap::new()),
            calls: AtomicUsize::new(0),
            gated: AtomicBool::new(false),
            gate: Semaphore::new(0),
        })
    }

    /// Answers `answer` to every call, each waiting for a [`release`](Self::release).
    pub(crate) fn gated(answer: Published) -> Arc<Self> {
        let observer = Self::answering(answer);
        observer.gated.store(true, Ordering::SeqCst);
        observer
    }

    /// Answers `answer` from now on.
    pub(crate) fn answer(&self, answer: Published) {
        *self.sticky.lock().expect("script lock") = answer;
    }

    /// Lets `calls` gated calls complete.
    pub(crate) fn release(&self, calls: usize) {
        self.gate.add_permits(calls);
    }

    /// Answers `answers` to the next calls, in order, before the others.
    pub(crate) fn then(&self, answers: impl IntoIterator<Item = Published>) {
        self.queued.lock().expect("script lock").extend(answers);
    }

    /// Answers `answer` for `credential_id` only.
    pub(crate) fn answer_for(&self, credential_id: CredentialId, answer: Published) {
        self.per_credential
            .lock()
            .expect("script lock")
            .insert(credential_id, answer);
    }

    /// Stops gating: every waiting and later call completes.
    pub(crate) fn open(&self) {
        self.gated.store(false, Ordering::SeqCst);
        self.gate.add_permits(1024);
    }

    /// Observer calls so far.
    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Yields until `n` calls have started.
    pub(crate) async fn until_calls(&self, n: usize) {
        while self.calls() < n {
            tokio::task::yield_now().await;
        }
    }
}

impl CredentialAvailabilityObserver for ScriptedObserver {
    fn observe_availability<'a>(
        &'a self,
        _scope: &'a TenantScope,
        credential_id: CredentialId,
        _expected_key: CredentialKey,
        _cancel: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<CredentialAvailabilityObservation, CredentialObserveError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let answer = {
                let queued = self.queued.lock().expect("script lock").pop_front();
                let per_credential = self
                    .per_credential
                    .lock()
                    .expect("script lock")
                    .get(&credential_id)
                    .copied();
                queued
                    .or(per_credential)
                    .unwrap_or_else(|| *self.sticky.lock().expect("script lock"))
            };
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.gated.load(Ordering::SeqCst) {
                self.gate.acquire().await.expect("gate open").forget();
            }
            answer
        })
    }
}

/// An observation at material `material` and use revision `admission`.
pub(crate) fn seen(
    material: u64,
    admission: u64,
    availability: CredentialAvailability,
) -> Published {
    Ok(
        CredentialAvailabilityObservation::new(material, 1, availability)
            .with_admission_epoch(admission),
    )
}

fn available() -> Published {
    seen(1, 1, CredentialAvailability::Available)
}

fn blocked() -> Published {
    seen(
        1,
        2,
        CredentialAvailability::Blocked(CredentialBlock::ReauthRequired),
    )
}

fn scope(workspace: &str) -> TenantScope {
    TenantScope::new("org", workspace)
}

fn key() -> CredentialKey {
    CredentialKey::new("bearer_token").expect("static key is valid")
}

fn reads(
    observer: &Arc<ScriptedObserver>,
) -> (Arc<CredentialReads>, nebula_metrics::MetricsRegistry) {
    let registry = nebula_metrics::MetricsRegistry::new();
    let metrics = CredentialAdmissionMetrics::new(&registry).expect("metrics register");
    (
        Arc::new(CredentialReads::new(
            Arc::clone(observer) as Arc<dyn CredentialAvailabilityObserver>,
            CancellationToken::new(),
            Some(metrics),
        )),
        registry,
    )
}

fn later() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_mins(1)
}

fn spawn_read(
    reads: &Arc<CredentialReads>,
    scope: TenantScope,
    id: CredentialId,
) -> tokio::task::JoinHandle<super::ReadResult> {
    let reads = Arc::clone(reads);
    tokio::spawn(async move { reads.read_after_arrival(&scope, id, &key(), later()).await })
}

async fn until_users(reads: &CredentialReads, id: CredentialId, n: usize) {
    while reads.users(id) < n {
        tokio::task::yield_now().await;
    }
}

/// C1: a caller arriving after a block committed never takes the answer of
/// the read that started before it.
#[tokio::test]
async fn a_caller_never_joins_a_read_in_flight_when_it_arrived() {
    let observer = ScriptedObserver::gated(available());
    let (reads, _registry) = reads(&observer);
    let id = CredentialId::new();

    let first = spawn_read(&reads, scope("ws"), id);
    observer.until_calls(1).await;
    // The block commits while read #1 (which saw the credential usable) is
    // still in flight; B arrives after it.
    observer.answer(blocked());
    let second = spawn_read(&reads, scope("ws"), id);
    until_users(&reads, id, 2).await;
    observer.release(2);

    assert_eq!(first.await.expect("joined").ok(), available().ok());
    assert_eq!(
        second.await.expect("joined").ok(),
        blocked().ok(),
        "B is answered by read #2, issued after it arrived"
    );
    assert_eq!(observer.calls(), 2);
    assert_eq!(reads.lane_count(), 0, "an idle lane is dropped");
}

#[tokio::test]
async fn a_burst_during_one_read_costs_exactly_one_more_read() {
    let observer = ScriptedObserver::gated(available());
    let (reads, _registry) = reads(&observer);
    let id = CredentialId::new();

    let leader = spawn_read(&reads, scope("ws"), id);
    observer.until_calls(1).await;
    let burst: Vec<_> = (0..8)
        .map(|_| spawn_read(&reads, scope("ws"), id))
        .collect();
    until_users(&reads, id, 9).await;
    observer.release(16);

    assert_eq!(leader.await.expect("joined").ok(), available().ok());
    for caller in burst {
        assert_eq!(caller.await.expect("joined").ok(), available().ok());
    }
    assert_eq!(observer.calls(), 2, "read #1 plus one read for the burst");
    let metrics = reads.metrics().expect("metrics");
    assert_eq!(metrics.reads(), [2, 0, 0, 0, 0, 0, 0]);
    assert_eq!(metrics.joined(), 7, "one of the eight led read #2");
    assert_eq!(reads.lane_count(), 0);
}

#[tokio::test]
async fn a_dropped_leader_hands_the_lane_to_a_waiter() {
    let observer = ScriptedObserver::gated(available());
    let (reads, _registry) = reads(&observer);
    let id = CredentialId::new();

    let leader = spawn_read(&reads, scope("ws"), id);
    observer.until_calls(1).await;
    observer.answer(blocked());
    let waiter = spawn_read(&reads, scope("ws"), id);
    until_users(&reads, id, 2).await;
    // The leader's acquire is cancelled mid-read: its read is dropped and
    // publishes nothing.
    leader.abort();
    assert!(leader.await.expect_err("aborted").is_cancelled());
    observer.until_calls(2).await;
    observer.release(1);

    assert_eq!(
        waiter.await.expect("joined").ok(),
        blocked().ok(),
        "the waiter re-elected itself and read after it arrived"
    );
    assert_eq!(observer.calls(), 2);
    assert_eq!(reads.lane_count(), 0);
}

#[tokio::test]
async fn owners_of_one_credential_id_read_on_separate_lanes() {
    let observer = ScriptedObserver::gated(available());
    let (reads, _registry) = reads(&observer);
    let id = CredentialId::new();

    let tenant_a = spawn_read(&reads, scope("a"), id);
    let tenant_b = spawn_read(&reads, scope("b"), id);
    // Both reads are in flight at once: neither lane waits on the other.
    observer.until_calls(2).await;
    assert_eq!(reads.lane_count(), 2);
    observer.release(2);

    assert_eq!(tenant_a.await.expect("joined").ok(), available().ok());
    assert_eq!(tenant_b.await.expect("joined").ok(), available().ok());
    assert_eq!(reads.lane_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_read_that_does_not_answer_times_out_at_the_deadline() {
    let observer = ScriptedObserver::gated(available());
    let (reads, _registry) = reads(&observer);
    let id = CredentialId::new();

    let started = tokio::time::Instant::now();
    let deadline = started + super::CREDENTIAL_READ_TIMEOUT;
    let result = reads
        .read_after_arrival(&scope("ws"), id, &key(), deadline)
        .await;

    assert_eq!(result, Err(ReadFailure::TimedOut));
    assert_eq!(started.elapsed(), super::CREDENTIAL_READ_TIMEOUT);
    assert_eq!(observer.calls(), 1);
    assert_eq!(reads.metrics().expect("metrics").reads()[5], 1);
    assert_eq!(reads.lane_count(), 0);
}

#[tokio::test]
async fn shutdown_ends_the_read_and_its_waiters_as_cancelled() {
    let observer = ScriptedObserver::gated(available());
    let cancel = CancellationToken::new();
    let registry = nebula_metrics::MetricsRegistry::new();
    let reads = Arc::new(CredentialReads::new(
        Arc::clone(&observer) as Arc<dyn CredentialAvailabilityObserver>,
        cancel.clone(),
        Some(CredentialAdmissionMetrics::new(&registry).expect("metrics register")),
    ));
    let id = CredentialId::new();

    let leader = spawn_read(&reads, scope("ws"), id);
    observer.until_calls(1).await;
    let waiter = spawn_read(&reads, scope("ws"), id);
    until_users(&reads, id, 2).await;
    cancel.cancel();

    assert_eq!(leader.await.expect("joined"), Err(ReadFailure::Cancelled));
    assert_eq!(waiter.await.expect("joined"), Err(ReadFailure::Cancelled));
    assert_eq!(observer.calls(), 1);
    assert_eq!(reads.lane_count(), 0);
    assert_eq!(
        reads.metrics().expect("metrics").reads(),
        [0, 0, 0, 0, 0, 0, 1],
        "the cancelled read is counted once, as cancelled, not as unavailable"
    );
}

#[tokio::test]
async fn an_observer_that_reports_cancellation_is_not_counted_unavailable() {
    let observer = ScriptedObserver::answering(Err(CredentialObserveError::Cancelled));
    let (reads, _registry) = reads(&observer);
    let result = reads
        .read_after_arrival(&scope("ws"), CredentialId::new(), &key(), later())
        .await;
    assert_eq!(result, Err(ReadFailure::Cancelled));
    assert_eq!(
        reads.metrics().expect("metrics").reads(),
        [0, 0, 0, 0, 0, 0, 1]
    );
}

#[tokio::test]
async fn a_failed_read_answers_with_its_error() {
    let observer = ScriptedObserver::answering(Err(CredentialObserveError::Unavailable));
    let (reads, _registry) = reads(&observer);
    let result = reads
        .read_after_arrival(&scope("ws"), CredentialId::new(), &key(), later())
        .await;
    assert_eq!(
        result,
        Err(ReadFailure::Observe(CredentialObserveError::Unavailable))
    );
    assert_eq!(reads.metrics().expect("metrics").reads()[4], 1);
}
