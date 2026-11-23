//! Test fixtures for observing and saturating a [`ReleaseQueue`].
//!
//! A worker runs many jobs at once, so the queue gives tests no ordering to
//! lean on:
//!
//! - One parked job does not keep the next one in the channel. A test that
//!   needs a job to stay *buffered* (an abort before it starts, a waiter that
//!   gives up on a job nobody picked up) fills every execution slot of the
//!   lane first — [`ReleaseQueue::park_entry_lane`] and
//!   [`ReleaseQueue::park_coordinator_lane`].
//! - A no-op job submitted last is not a checkpoint: it can settle before the
//!   cleanup submitted ahead of it. A test that needs "all accepted cleanup
//!   has run" awaits [`ReleaseQueue::settled`].

use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::{JOBS_PER_WORKER, JobClass, ReleaseQueue};

/// Every primary execution slot of one lane, held by parked jobs.
///
/// The jobs stay parked until [`release`](Self::release), or until the queue
/// aborts them. Dropping this value leaves them parked.
#[must_use = "the lane is saturated only while the parked jobs are neither released nor aborted"]
pub(crate) struct ParkedLane {
    release: CancellationToken,
}

impl ParkedLane {
    /// Lets every parked job finish, freeing the lane.
    pub(crate) fn release(self) {
        self.release.cancel();
    }
}

impl ReleaseQueue {
    /// Resolves once no accepted job is buffered or running.
    ///
    /// Jobs parked by this module count: release them first.
    pub(crate) async fn settled(&self) {
        self.admission
            .wait_for(|state| state.outstanding == 0)
            .await;
    }

    /// Parks a job on every primary execution slot of the entry lane and
    /// returns once all of them run. The next entry job stays buffered.
    pub(crate) async fn park_entry_lane(&self) -> ParkedLane {
        self.park_lane(JobClass::Entry).await
    }

    /// Parks a job on every primary execution slot of the coordinator lane
    /// and returns once all of them run. The next coordinator job stays
    /// buffered.
    pub(crate) async fn park_coordinator_lane(&self) -> ParkedLane {
        self.park_lane(JobClass::Coordinator).await
    }

    async fn park_lane(&self, class: JobClass) -> ParkedLane {
        let jobs = self.lane(class).primary_workers * JOBS_PER_WORKER;
        let release = CancellationToken::new();
        let started = Arc::new(Semaphore::new(0));
        for _ in 0..jobs {
            let release = release.clone();
            let started = Arc::clone(&started);
            self.submit_job(
                move || {
                    Box::pin(async move {
                        started.add_permits(1);
                        release.cancelled().await;
                        Ok(())
                    })
                },
                class,
            )
            .expect("an open queue accepts the parked jobs")
            .detach();
        }
        let permits = u32::try_from(jobs).expect("slot count fits in u32");
        started
            .acquire_many(permits)
            .await
            .expect("the start semaphore is never closed")
            .forget();
        ParkedLane { release }
    }
}
