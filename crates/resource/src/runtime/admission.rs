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
//! - **Readmission** ([`AdmissionCell::reopen`] on an admitting row) — the
//!   credential's use revision (admission epoch) advanced at the same
//!   material past everything the slot admitted: use was closed and reopened
//!   in between without this row observing the denial. A fresh generation is
//!   published so new work is admitted under the new revision (Design
//!   CONTRACT "old use revision does not admit"); like a benign publication
//!   it leaves the predecessor open, because only an observed block closes
//!   admitted work. Refusing work admitted during an unobserved interval is
//!   the strict per-acquire availability read, a separate contract.
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

use crate::{
    error::CredentialUnavailableReason, manager::CredentialObservedAt, state::CredentialSuspension,
};

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
    /// The row was not suspended, but the credential's use revision advanced
    /// past everything the slot admitted at the same material (a denial
    /// interval nobody observed): generation `seq` is current. The
    /// predecessor stays open.
    Readmitted {
        /// Sequence number of the fresh generation.
        seq: u64,
    },
    /// The slot cleared (or was not suspended) but another slot still is.
    StillSuspended,
    /// The row was not suspended, and the observation adds nothing.
    NotSuspended,
    /// The observation is older than what the slot already admitted or was
    /// denied at, or is at another material than the one installed.
    StaleObservation,
    /// A suspension happened after the ticket was captured.
    Superseded,
    /// The row is retired.
    Retired,
}

/// A credential use revision: the material epoch and the admission epoch it
/// was admitted or denied at, ordered lexicographically (a material advance
/// dominates any admission epoch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct UseMark {
    material_epoch: u64,
    admission_epoch: u64,
}

impl UseMark {
    pub(crate) const fn new(material_epoch: u64, admission_epoch: u64) -> Self {
        Self {
            material_epoch,
            admission_epoch,
        }
    }

    /// The mark an observation carries, when it carries a use revision.
    pub(crate) const fn observed(observed: CredentialObservedAt) -> Option<Self> {
        match observed.admission_epoch() {
            Some(admission_epoch) => Some(Self::new(observed.material_epoch(), admission_epoch)),
            None => None,
        }
    }

    /// The mark of an installed projection.
    #[expect(
        dead_code,
        reason = "guard-justified: the manager reads installed marks in the next change"
    )]
    pub(crate) fn installed(metadata: &nebula_credential::CredentialGuardMetadata) -> Self {
        Self::new(metadata.material_epoch(), metadata.admission_epoch())
    }

    pub(crate) const fn material_epoch(self) -> u64 {
        self.material_epoch
    }
}

/// The use revision a suspended slot was denied at: a reopen must present a
/// newer one.
///
/// **Witnessed** floors come from a denial read together with its use
/// revision (an `Open` status awaiting reauthentication); a reopen then needs
/// a strictly higher revision, because every backend advances the admission
/// epoch when such a denial clears. **Unwitnessed** floors come from a
/// denial without a revision (an operation in flight or awaiting
/// reconciliation, a resolver without an observer); the backend bumped the
/// revision when that denial began, so the floor is only what the slot had
/// admitted, and a reopen at an equal revision is accepted.
///
/// Ordered by `(mark, witnessed)`, so the larger of two floors is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) struct SuspensionFloor {
    mark: Option<UseMark>,
    witnessed: bool,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "guard-justified: the manager computes floors in the next change"
    )
)]
impl SuspensionFloor {
    /// A denial observed at `mark` together with its use revision.
    pub(crate) const fn witnessed(mark: UseMark) -> Self {
        Self {
            mark: Some(mark),
            witnessed: true,
        }
    }

    /// A denial observed without a use revision; `admitted` is what the slot
    /// had admitted, if known.
    pub(crate) const fn unwitnessed(admitted: Option<UseMark>) -> Self {
        Self {
            mark: admitted,
            witnessed: false,
        }
    }

    /// Whether a use at `observed` clears this floor.
    fn cleared_by(self, observed: Option<UseMark>) -> bool {
        match (self.mark, observed) {
            // Nothing to compare: the pre-revision (ticket-only) rule.
            (None, _) | (_, None) => true,
            (Some(floor), Some(observed)) if self.witnessed => observed > floor,
            (Some(floor), Some(observed)) => {
                if observed == floor {
                    tracing::debug!(
                        material_epoch = observed.material_epoch,
                        admission_epoch = observed.admission_epoch,
                        "credential reopened at the use revision it had admitted: the denial \
                         carried no revision of its own"
                    );
                }
                observed >= floor
            },
        }
    }
}

/// Why one slot denies use, and the use revision a reopen must beat.
#[derive(Debug, Clone, Copy)]
struct SlotDenial {
    reason: CredentialUnavailableReason,
    floor: SuspensionFloor,
}

/// Which bound slots currently deny use, the use revision each slot last
/// admitted beyond its installed projection, and the ticket counter a reopen
/// must present. Only suspensions advance `epoch`: a late deny always lands,
/// while an admit observed before a later deny is refused.
#[derive(Debug, Default)]
struct CredentialGate {
    by_slot: BTreeMap<Box<str>, SlotDenial>,
    admitted: BTreeMap<Box<str>, UseMark>,
    epoch: u64,
}

impl CredentialGate {
    /// The use revision `slot` admits at: the higher of the installed
    /// projection's and the last one a reopen or readmit recorded.
    fn admitted(&self, slot: &str, installed: Option<UseMark>) -> Option<UseMark> {
        installed.max(self.admitted.get(slot).copied())
    }

    fn record_admitted(&mut self, slot: &str, observed: Option<UseMark>) {
        let Some(observed) = observed else {
            return;
        };
        match self.admitted.get_mut(slot) {
            Some(admitted) => *admitted = (*admitted).max(observed),
            None => {
                self.admitted.insert(slot.into(), observed);
            },
        }
    }
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
    /// reopen ticket captured before it is refused. A repeated suspension of
    /// the same slot records the new reason and keeps the higher `floor`.
    pub(crate) fn suspend(
        &self,
        slot: &str,
        reason: CredentialUnavailableReason,
        floor: SuspensionFloor,
    ) -> SuspendTransition {
        if self.terminal.is_cancelled() {
            return SuspendTransition::Retired;
        }
        let mut gate = self.gate();
        let first = gate.by_slot.is_empty();
        let floor = gate
            .by_slot
            .get(slot)
            .map_or(floor, |denial| denial.floor.max(floor));
        gate.by_slot
            .insert(slot.into(), SlotDenial { reason, floor });
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

    /// Records that `slot`'s credential is usable at `observed`, given the
    /// projection `installed` in the slot (if known).
    ///
    /// - An observation at another material than the installed one is
    ///   [`StaleObservation`](ReopenTransition::StaleObservation): newer
    ///   material reaches the row through its install.
    /// - **Suspended row**: `ticket` must still equal the gate epoch
    ///   ([`Superseded`](ReopenTransition::Superseded) otherwise). A denied
    ///   slot clears only when `observed` clears its [`SuspensionFloor`];
    ///   clearing the last one publishes a fresh generation in the span the
    ///   suspension opened, and the generations it closed stay closed.
    /// - **Admitting row**: a use revision above everything the slot admitted
    ///   means a denial interval came and went unobserved. A fresh generation
    ///   is published ([`Readmitted`](ReopenTransition::Readmitted)) so that
    ///   new work is admitted under the new revision; the predecessor is not
    ///   closed — closing is reserved for an observed block — and nothing is
    ///   rebuilt.
    ///
    /// Every accepted observation raises the slot's admitted revision.
    pub(crate) fn reopen(
        &self,
        slot: &str,
        ticket: u64,
        observed: CredentialObservedAt,
        installed: Option<UseMark>,
    ) -> ReopenTransition {
        if self.terminal.is_cancelled() {
            return ReopenTransition::Retired;
        }
        if installed
            .is_some_and(|installed| installed.material_epoch() != observed.material_epoch())
        {
            return ReopenTransition::StaleObservation;
        }
        let mark = UseMark::observed(observed);
        let mut gate = self.gate();
        if gate.by_slot.is_empty() {
            let admitted = gate.admitted(slot, installed);
            return match (mark, admitted) {
                (Some(mark), Some(admitted)) if mark > admitted => {
                    gate.record_admitted(slot, Some(mark));
                    match self.publish_in_span() {
                        Some(generation) => ReopenTransition::Readmitted {
                            seq: generation.seq(),
                        },
                        None => ReopenTransition::Retired,
                    }
                },
                (Some(mark), Some(admitted)) if mark < admitted => {
                    ReopenTransition::StaleObservation
                },
                _ => {
                    gate.record_admitted(slot, mark);
                    ReopenTransition::NotSuspended
                },
            };
        }
        if gate.epoch != ticket {
            return ReopenTransition::Superseded;
        }
        let Some(denial) = gate.by_slot.get(slot).copied() else {
            gate.record_admitted(slot, mark);
            return ReopenTransition::StillSuspended;
        };
        if !denial.floor.cleared_by(mark) {
            return ReopenTransition::StaleObservation;
        }
        gate.by_slot.remove(slot);
        gate.record_admitted(slot, mark);
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

    /// The use revision `slot` currently admits at: the higher of
    /// `installed` and the last revision a reopen or readmit recorded.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "guard-justified: the manager reads admitted marks in the next change"
        )
    )]
    pub(crate) fn admitted(&self, slot: &str, installed: Option<UseMark>) -> Option<UseMark> {
        self.gate().admitted(slot, installed)
    }

    /// The ticket a later [`reopen`](Self::reopen) must present: capture it
    /// before observing the credential, so a suspension landing after the
    /// observation supersedes the reopen.
    pub(crate) fn gate_epoch(&self) -> u64 {
        self.gate().epoch
    }

    /// Whether a bound credential currently suspends the row.
    pub(crate) fn is_suspended(&self) -> bool {
        self.suspended.load(Ordering::Acquire)
    }

    /// The suspended slots and their reasons, or `None` when admitting.
    pub(crate) fn suspension(&self) -> Option<CredentialSuspension> {
        let gate = self.gate();
        (!gate.by_slot.is_empty()).then(|| {
            CredentialSuspension::new(
                gate.by_slot
                    .iter()
                    .map(|(slot, denial)| (slot.to_string(), denial.reason))
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
