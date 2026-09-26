//! Admission generations: the per-row closing notice a lease observes.
//!
//! A registered row admits leases under an **admission generation** — a
//! sequence number paired with a closing [`CancellationToken`]. An acquire
//! captures the row's current generation under `Manager.admission`, and the
//! resulting guard keeps it for its whole lease, so a lease can observe that
//! the row stopped admitting work in its generation without polling.
//!
//! Two kinds of change reach a generation:
//!
//! - **Benign publication** ([`AdmissionCell::publish`]) — a config reload or
//!   a credential refresh. A new generation becomes current; the predecessor
//!   stays **open**, because leases admitted under it remain valid (canon
//!   §13.2: a refresh or reload never interrupts in-flight work).
//! - **Closing** — [`AdmissionCell::retire`] (credential taint/revoke, row
//!   removal, shutdown, manager drop) cancels the row's terminal token and so
//!   every generation ever published, including older benign ones still held
//!   by leases. [`AdmissionCell::close_current`] closes only the current
//!   generation (the credential-suspension primitive).
//!
//! Every generation token is a child of the row's terminal token, which is
//! itself a child of the manager's cancellation token: cancelling the manager
//! closes every row's generations at once.
//!
//! Closing is a cooperative notice. It does not stop a lease, revoke a
//! borrow, or roll anything back remotely; the guard is still released
//! normally.
//!
//! All mutations (`publish`, `close_current`, `retire`) run under
//! `Manager.admission`, except the manager-drop retirement which holds
//! `&mut Manager`. Reads are lock-free.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use arc_swap::ArcSwapOption;
use tokio_util::sync::CancellationToken;

/// One admission generation of a row: its sequence number and the token that
/// fires when the row stops admitting work in this generation.
///
/// Immutable once published: a lease that captured a generation always
/// observes that generation's token, whatever the row publishes later.
#[derive(Debug)]
pub(crate) struct AdmissionGeneration {
    seq: u64,
    closing: CancellationToken,
}

impl AdmissionGeneration {
    /// A generation that is closed from birth, for a lease built while the
    /// row admits nothing.
    #[cfg_attr(not(test), expect(dead_code, reason = "guards adopt generations next"))]
    fn closed_sentinel() -> Self {
        let closing = CancellationToken::new();
        closing.cancel();
        Self { seq: 0, closing }
    }

    /// The generation's sequence number; strictly increasing per row.
    /// Crate-private: a lease observes closing, not the sequence.
    pub(crate) fn seq(&self) -> u64 {
        self.seq
    }

    /// Whether the row stopped admitting work in this generation.
    pub(crate) fn is_closed(&self) -> bool {
        self.closing.is_cancelled()
    }

    /// The generation's closing token.
    #[cfg_attr(not(test), expect(dead_code, reason = "guards adopt generations next"))]
    pub(crate) fn token(&self) -> &CancellationToken {
        &self.closing
    }
}

/// A row's admission state: the current generation and the terminal token
/// every generation descends from.
#[derive(Debug)]
pub(crate) struct AdmissionCell {
    /// Child of the manager's cancellation token; cancelled once, on
    /// retirement.
    terminal: CancellationToken,
    /// `None` once retired (or, with credential suspension, suspended).
    current: ArcSwapOption<AdmissionGeneration>,
    /// Next sequence number to publish; mutated under `Manager.admission`.
    next_seq: AtomicU64,
}

impl AdmissionCell {
    /// A cell whose first generation (sequence 1) is already current.
    pub(crate) fn new(terminal: CancellationToken) -> Self {
        let first = Arc::new(AdmissionGeneration {
            seq: 1,
            closing: terminal.child_token(),
        });
        Self {
            terminal,
            current: ArcSwapOption::from(Some(first)),
            next_seq: AtomicU64::new(2),
        }
    }

    /// The generation new work is admitted under, or `None` when the row is
    /// retired, suspended, or its current generation is closed.
    #[cfg_attr(not(test), expect(dead_code, reason = "guards adopt generations next"))]
    pub(crate) fn current(&self) -> Option<Arc<AdmissionGeneration>> {
        if self.terminal.is_cancelled() {
            return None;
        }
        self.current
            .load_full()
            .filter(|generation| !generation.is_closed())
    }

    /// Publishes a new current generation for a benign change (reload,
    /// credential refresh). The predecessor is **not** closed: leases
    /// admitted under it stay valid. Returns `None`, publishing nothing,
    /// once the row is retired.
    pub(crate) fn publish(&self) -> Option<Arc<AdmissionGeneration>> {
        if self.terminal.is_cancelled() {
            return None;
        }
        let generation = Arc::new(AdmissionGeneration {
            seq: self.next_seq.fetch_add(1, Ordering::AcqRel),
            closing: self.terminal.child_token(),
        });
        self.current.store(Some(Arc::clone(&generation)));
        // A retirement racing this store already cancelled the terminal
        // token, so the new generation was born closed; report it unpublished.
        (!generation.is_closed()).then_some(generation)
    }

    /// Closes the current generation and leaves the row with none, returning
    /// the closed sequence. `None` when there was no current generation.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by credential suspension (A3)")
    )]
    pub(crate) fn close_current(&self) -> Option<u64> {
        let closed = self.current.swap(None)?;
        closed.closing.cancel();
        Some(closed.seq)
    }

    /// Closes every generation of this row — the current one and older ones
    /// still held by leases — and admits nothing afterwards. Idempotent.
    pub(crate) fn retire(&self) {
        self.terminal.cancel();
        self.current.store(None);
    }

    /// Whether [`retire`](Self::retire) ran or the manager was cancelled.
    #[cfg(test)]
    pub(crate) fn is_retired(&self) -> bool {
        self.terminal.is_cancelled()
    }

    /// The current generation, or a generation closed from birth when the
    /// row admits nothing.
    #[cfg_attr(not(test), expect(dead_code, reason = "guards adopt generations next"))]
    pub(crate) fn snapshot(&self) -> Arc<AdmissionGeneration> {
        self.current()
            .unwrap_or_else(|| Arc::new(AdmissionGeneration::closed_sentinel()))
    }
}

impl Default for AdmissionCell {
    /// A cell under its own, unowned terminal token (tests and rows built
    /// outside a manager).
    fn default() -> Self {
        Self::new(CancellationToken::new())
    }
}

#[cfg(test)]
#[path = "admission_tests.rs"]
mod tests;
