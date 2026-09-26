//! Admission generations: the per-row closing notice a lease observes.
//!
//! A registered row admits leases under an **admission generation** — a
//! sequence number paired with a closing [`CancellationToken`]. An acquire
//! captures the row's current generation under `Manager.admission`, and the
//! resulting guard keeps it for its whole lease, so a lease can observe that
//! the row stopped admitting work in its generation without polling.
//!
//! Three kinds of change reach a generation:
//!
//! - **Benign publication** ([`AdmissionCell::publish`]) — a config reload or
//!   a credential refresh. A new generation becomes current; the predecessor
//!   stays **open**, because leases admitted under it remain valid (canon
//!   §13.2: a refresh or reload never interrupts in-flight work).
//! - **Credential suspension** ([`AdmissionCell::suspend`]) — a bound
//!   credential denies use at its current material (reauthentication
//!   required, an operation blocking use). Suspension closes every generation
//!   published since the previous suspension — its **admission span** — not
//!   only the current one: every unit admitted on the old logical binding is
//!   told to stop cooperatively, and that binding stays retired (Design
//!   CONTRACT "same-material block"; review ADM-N1). The row then admits
//!   nothing until [`AdmissionCell::reopen`] clears the last suspended slot
//!   and publishes a fresh generation in a fresh span. Retained physical
//!   owners stay; only the admission (logical binding) generation changes.
//! - **Retirement** ([`AdmissionCell::retire`]) — credential taint/revoke,
//!   row removal, shutdown, manager drop. It cancels the row's terminal token
//!   and so every generation ever published, in every span.
//!
//! Token tree: manager cancellation → row terminal → admission span →
//! generation. Cancelling a node closes everything below it.
//!
//! Closing is a cooperative notice. It does not stop a lease, revoke a
//! borrow, or roll anything back remotely; the guard is still released
//! normally.
//!
//! All mutations (`publish`, `suspend`, `reopen`, `retire`) run under
//! `Manager.admission`, except the manager-drop retirement which holds
//! `&mut Manager`. Reads are lock-free apart from the credential gate's own
//! short mutex.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use arc_swap::{ArcSwap, ArcSwapOption};
use tokio_util::sync::CancellationToken;

use crate::{error::CredentialUnavailableReason, state::CredentialSuspension};

/// Why an admission span closed, when something other than retirement closed
/// it. Read by the hand-out refusal and by `Limited` waits so the caller
/// learns the credential reason rather than a bare cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseCause {
    /// A bound credential denied use at its current material.
    Credential(CredentialUnavailableReason),
}

/// The generations published between two suspensions share one span: a
/// suspension records its cause, then cancels the span's token (closing all
/// of them).
#[derive(Debug)]
struct AdmissionSpan {
    /// Child of the row's terminal token.
    token: CancellationToken,
    /// Set once, before `token` is cancelled by a suspension.
    cause: OnceLock<CloseCause>,
}

impl AdmissionSpan {
    fn new(terminal: &CancellationToken) -> Self {
        Self {
            token: terminal.child_token(),
            cause: OnceLock::new(),
        }
    }
}

/// One admission generation of a row: its sequence number and the token that
/// fires when the row stops admitting work in this generation.
///
/// Immutable once published: a lease that captured a generation always
/// observes that generation's token, whatever the row publishes later.
#[derive(Debug)]
pub(crate) struct AdmissionGeneration {
    seq: u64,
    /// Child of `span.token`.
    closing: CancellationToken,
    span: Arc<AdmissionSpan>,
}

impl AdmissionGeneration {
    fn in_span(seq: u64, span: &Arc<AdmissionSpan>) -> Self {
        Self {
            seq,
            closing: span.token.child_token(),
            span: Arc::clone(span),
        }
    }

    /// A generation that is closed from birth, for a lease built while the
    /// row admits nothing.
    fn closed_sentinel() -> Self {
        let span = Arc::new(AdmissionSpan {
            token: CancellationToken::new(),
            cause: OnceLock::new(),
        });
        span.token.cancel();
        Self::in_span(0, &span)
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
    pub(crate) fn token(&self) -> &CancellationToken {
        &self.closing
    }

    /// Why this generation's span was closed by a suspension, if it was.
    /// `None` for an open generation and for one closed only by retirement.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by the suspension hand-out refusal (A3.3)")
    )]
    pub(crate) fn close_cause(&self) -> Option<CloseCause> {
        self.span.cause.get().copied()
    }
}

/// Result of [`AdmissionCell::suspend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuspendTransition {
    /// The row was admitting; its span closed.
    Suspended {
        /// Highest sequence number the closed span could contain.
        closed_through: u64,
    },
    /// The row was already suspended; the slot's reason was (re)recorded.
    Updated,
    /// The row is retired; nothing changed.
    Retired,
}

/// Result of [`AdmissionCell::reopen`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReopenTransition {
    /// The last suspended slot cleared; generation `seq` is current.
    Reopened {
        /// Sequence number of the fresh generation.
        seq: u64,
    },
    /// The slot cleared (or was not suspended) but another slot still is.
    StillSuspended,
    /// The row was not suspended.
    NotSuspended,
    /// A suspension happened after the ticket was captured.
    Superseded,
    /// The row is retired.
    Retired,
}

/// Which bound slots currently deny use, and the ticket counter a reopen must
/// present. Only suspensions advance `epoch`: a late deny always lands, while
/// an admit observed before a later deny is refused.
#[derive(Debug, Default)]
struct CredentialGate {
    by_slot: BTreeMap<Box<str>, CredentialUnavailableReason>,
    epoch: u64,
}

/// A row's admission state: the current generation, the span it belongs to,
/// the credential gate, and the terminal token everything descends from.
#[derive(Debug)]
pub(crate) struct AdmissionCell {
    /// Child of the manager's cancellation token; cancelled once, on
    /// retirement.
    terminal: CancellationToken,
    /// The span new generations are published in.
    span: ArcSwap<AdmissionSpan>,
    /// `None` once retired or while suspended.
    current: ArcSwapOption<AdmissionGeneration>,
    /// Next sequence number to publish; mutated under `Manager.admission`.
    next_seq: AtomicU64,
    /// Lock-free mirror of "the gate has at least one suspended slot".
    suspended: AtomicBool,
    gate: std::sync::Mutex<CredentialGate>,
}

impl AdmissionCell {
    /// A cell whose first generation (sequence 1) is already current.
    pub(crate) fn new(terminal: CancellationToken) -> Self {
        let span = Arc::new(AdmissionSpan::new(&terminal));
        let first = Arc::new(AdmissionGeneration::in_span(1, &span));
        Self {
            terminal,
            span: ArcSwap::new(span),
            current: ArcSwapOption::from(Some(first)),
            next_seq: AtomicU64::new(2),
            suspended: AtomicBool::new(false),
            gate: std::sync::Mutex::new(CredentialGate::default()),
        }
    }

    fn gate(&self) -> std::sync::MutexGuard<'_, CredentialGate> {
        self.gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The generation new work is admitted under, or `None` when the row is
    /// retired, suspended, or its current generation is closed.
    pub(crate) fn current(&self) -> Option<Arc<AdmissionGeneration>> {
        if self.terminal.is_cancelled() {
            return None;
        }
        self.current
            .load_full()
            .filter(|generation| !generation.is_closed())
    }

    /// Publishes a generation in the current span and makes it current.
    fn publish_in_span(&self) -> Option<Arc<AdmissionGeneration>> {
        let span = self.span.load_full();
        let generation = Arc::new(AdmissionGeneration::in_span(
            self.next_seq.fetch_add(1, Ordering::AcqRel),
            &span,
        ));
        self.current.store(Some(Arc::clone(&generation)));
        // A retirement racing this store already cancelled the terminal
        // token, so the new generation was born closed; report it unpublished.
        (!generation.is_closed()).then_some(generation)
    }

    /// Publishes a new current generation for a benign change (reload,
    /// credential refresh). The predecessor is **not** closed: leases
    /// admitted under it stay valid. Returns `None`, publishing nothing,
    /// once the row is retired or while it is suspended — a suspended row
    /// admits again only through [`reopen`](Self::reopen).
    pub(crate) fn publish(&self) -> Option<Arc<AdmissionGeneration>> {
        if self.terminal.is_cancelled() {
            return None;
        }
        let gate = self.gate();
        if !gate.by_slot.is_empty() {
            return None;
        }
        let published = self.publish_in_span();
        drop(gate);
        published
    }

    /// Records that `slot` denies use for `reason`.
    ///
    /// The first suspended slot records the cause and closes the current
    /// span — every generation published since the previous suspension,
    /// including older benign ones still held by leases — and leaves the row
    /// with no current generation. Every call advances the gate epoch, so a
    /// reopen ticket captured before it is refused.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "wired by the row suspension API (A3.3)")
    )]
    pub(crate) fn suspend(
        &self,
        slot: &str,
        reason: CredentialUnavailableReason,
    ) -> SuspendTransition {
        if self.terminal.is_cancelled() {
            return SuspendTransition::Retired;
        }
        let mut gate = self.gate();
        let first = gate.by_slot.is_empty();
        gate.by_slot.insert(slot.into(), reason);
        gate.epoch += 1;
        if !first {
            return SuspendTransition::Updated;
        }
        self.suspended.store(true, Ordering::Release);
        let closing = self.span.load_full();
        // The cause is set before the cancel so a waiter woken by the token
        // always reads it.
        let _ = closing.cause.set(CloseCause::Credential(reason));
        closing.token.cancel();
        self.span
            .store(Arc::new(AdmissionSpan::new(&self.terminal)));
        self.current.store(None);
        SuspendTransition::Suspended {
            closed_through: self.next_seq.load(Ordering::Acquire).saturating_sub(1),
        }
    }

    /// Clears `slot`'s suspension if `ticket` still equals the gate epoch.
    /// Clearing the last suspended slot publishes a fresh generation in the
    /// span opened by the suspension; the generations it closed stay closed.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "wired by the row suspension API (A3.3)")
    )]
    pub(crate) fn reopen(&self, slot: &str, ticket: u64) -> ReopenTransition {
        if self.terminal.is_cancelled() {
            return ReopenTransition::Retired;
        }
        let mut gate = self.gate();
        if gate.by_slot.is_empty() {
            return ReopenTransition::NotSuspended;
        }
        if gate.epoch != ticket {
            return ReopenTransition::Superseded;
        }
        gate.by_slot.remove(slot);
        if !gate.by_slot.is_empty() {
            return ReopenTransition::StillSuspended;
        }
        self.suspended.store(false, Ordering::Release);
        match self.publish_in_span() {
            Some(generation) => ReopenTransition::Reopened {
                seq: generation.seq(),
            },
            None => ReopenTransition::Retired,
        }
    }

    /// The ticket a later [`reopen`](Self::reopen) must present: capture it
    /// before observing the credential, so a suspension landing after the
    /// observation supersedes the reopen.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "wired by the row suspension API (A3.3)")
    )]
    pub(crate) fn gate_epoch(&self) -> u64 {
        self.gate().epoch
    }

    /// Whether a bound credential currently suspends the row.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "wired by the row suspension API (A3.3)")
    )]
    pub(crate) fn is_suspended(&self) -> bool {
        self.suspended.load(Ordering::Acquire)
    }

    /// The suspended slots and their reasons, or `None` when admitting.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "wired by the row status snapshot (A3.3)")
    )]
    pub(crate) fn suspension(&self) -> Option<CredentialSuspension> {
        let gate = self.gate();
        (!gate.by_slot.is_empty()).then(|| {
            CredentialSuspension::new(
                gate.by_slot
                    .iter()
                    .map(|(slot, reason)| (slot.to_string(), *reason))
                    .collect(),
            )
        })
    }

    /// Closes every generation of this row — the current one and older ones
    /// still held by leases, in every span — and admits nothing afterwards.
    /// Idempotent.
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
