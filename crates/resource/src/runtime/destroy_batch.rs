//! Queue-owned, constant-message teardown of an owned collection of entries.

use std::{collections::VecDeque, sync::Arc};

use crate::{
    Error, Provider, TeardownReason,
    release_queue::{ReleaseQueue, TaskLoss},
};

use super::managed::{EntryOf, ManagedResource};
use super::retained_store::TrackedRetained;

enum BatchEntry<E> {
    Fresh(E),
    Retained(TrackedRetained<E>),
}

/// Untouched entries remain accounted for even when the coordinator is never
/// polled or is aborted between members. Each running member gets its own budget.
pub(crate) struct DestroyBatch<R: Provider> {
    managed: Arc<ManagedResource<R>>,
    entries: VecDeque<BatchEntry<EntryOf<R>>>,
    losses: TaskLoss,
    reason: TeardownReason,
}

impl<R: Provider> DestroyBatch<R> {
    pub(crate) fn new(
        managed: Arc<ManagedResource<R>>,
        entries: Vec<EntryOf<R>>,
        reason: TeardownReason,
    ) -> Self {
        let losses = managed.release_queue.entry_losses(entries.len());
        Self {
            managed,
            entries: entries.into_iter().map(BatchEntry::Fresh).collect(),
            losses,
            reason,
        }
    }

    pub(crate) fn extend(&mut self, entries: Vec<EntryOf<R>>) {
        self.losses.add(entries.len());
        self.entries
            .extend(entries.into_iter().map(BatchEntry::Fresh));
    }

    pub(crate) fn extend_retained(&mut self, entries: Vec<TrackedRetained<EntryOf<R>>>) {
        self.entries
            .extend(entries.into_iter().map(BatchEntry::Retained));
    }

    pub(crate) fn push(&mut self, entry: EntryOf<R>) {
        self.losses.add(1);
        self.entries.push_back(BatchEntry::Fresh(entry));
    }

    pub(crate) fn append(&mut self, mut other: Self) {
        self.entries.append(&mut other.entries);
        self.losses.absorb(other.losses);
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) async fn run(mut self) -> Result<(), Error> {
        let mut outcome = Ok(());
        let mut failed_entries = 0usize;
        while let Some(entry) = self.entries.pop_front() {
            let (entry, loss) = match entry {
                BatchEntry::Fresh(entry) => (entry, self.losses.transfer_one()),
                BatchEntry::Retained(tracked) => tracked.into_parts(),
            };
            let result =
                ReleaseQueue::run_entry(self.managed.destroy_entry(entry, self.reason), loss).await;
            if result.is_err() {
                failed_entries += 1;
            }
            if outcome.is_ok() && result.is_err() {
                outcome = result;
            }
            tokio::task::consume_budget().await;
        }
        if failed_entries != 0 {
            tracing::warn!(resource.key = %R::key(), failed_entries, "resource teardown batch completed with failures");
        }
        outcome
    }
}
