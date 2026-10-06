//! Atomic state-transition unit-of-work.
//!
//! [`TransitionBatch`] is the *only* way to apply a state transition. Its
//! fields are private and its one constructor, [`TransitionBatch::new`],
//! takes every required part — scope, execution id, expected CAS version,
//! lease [`FencingToken`], and the snapshot with its listing — so a batch
//! missing one does not compile. `commit` writes `new_state` + `outbox` + `journal` +
//! `resume_tokens` in one transaction (or one mutex-guarded mutation for
//! InMemory), gated by the version CAS *and* the fencing token. This makes
//! a split between durable state and outbox/journal/resume-tokens impossible
//! by construction: there is exactly one call site and one transaction for
//! the four.
//!
//! The `resume_tokens` field carries at most one [`ResumeTokenRow`] per
//! signal-park: the engine mints the token and pushes the row here so
//! backends insert it atomically with the `Waiting` state snapshot.  See
//! ADR-0099 W-S3c.
use chrono::{DateTime, Utc};

use crate::dto::resume_token::ResumeTokenRow;
use crate::dto::{ControlMsg, ExecutionListing, JournalEntry};
use crate::ids::FencingToken;
use crate::scope::Scope;

/// How a terminal execution transition mutates its execution-owned
/// plan/flavor reference.
///
/// The terminal commit is the execution-owner transaction that releases a
/// live reference or moves it into a rollback window. Backends apply the
/// reference change atomically with the aggregate state, outbox, journal, and
/// resume-token writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionReferenceTransition {
    /// Release the live reference immediately (plain terminal transition).
    ReleaseLive,
    /// Keep the reference in a rollback window until `retain_until`.
    RetainRollback {
        /// Opaque rollback-window identity.
        window_id: [u8; 16],
        /// Absolute expiry of the rollback window.
        retain_until: DateTime<Utc>,
    },
}

/// The atomic transition payload consumed by `ExecutionStore::commit`.
#[derive(Debug, Clone)]
pub struct TransitionBatch {
    scope: Scope,
    execution_id: String,
    expected_version: u64,
    fencing: FencingToken,
    new_state: serde_json::Value,
    /// Queryable projection of `new_state`, written in the same statement.
    listing: ExecutionListing,
    outbox: Vec<ControlMsg>,
    journal: Vec<JournalEntry>,
    /// Resume-token rows to INSERT in the same transaction as the state
    /// snapshot.  Empty on non-signal-park commits.  The INSERT uses
    /// `ON CONFLICT(execution_id, node_key) DO NOTHING` so a crash
    /// re-drive that re-parks the same node does NOT mint a second token.
    resume_tokens: Vec<ResumeTokenRow>,
    /// Optional execution-owned revision-reference mutation to apply in the
    /// same commit. `None` on non-terminal transitions.
    reference_transition: Option<ExecutionReferenceTransition>,
}

impl TransitionBatch {
    /// A transition of `execution_id` in `scope` from `expected_version`,
    /// under the lease `fencing`, to `state` with its `listing` projection.
    ///
    /// The snapshot and its projection are one argument pair so neither can be
    /// committed without the other. Outbox, journal, resume tokens and the
    /// reference transition start empty; add them with the `with_*` methods.
    #[must_use]
    pub fn new(
        scope: Scope,
        execution_id: impl Into<String>,
        expected_version: u64,
        fencing: FencingToken,
        state: serde_json::Value,
        listing: ExecutionListing,
    ) -> Self {
        Self {
            scope,
            execution_id: execution_id.into(),
            expected_version,
            fencing,
            new_state: state,
            listing,
            outbox: Vec::new(),
            journal: Vec::new(),
            resume_tokens: Vec::new(),
            reference_transition: None,
        }
    }

    /// Control-queue rows to append in the same transaction.
    #[must_use]
    pub fn with_outbox(mut self, outbox: Vec<ControlMsg>) -> Self {
        self.outbox = outbox;
        self
    }

    /// Journal rows to append in the same transaction.
    #[must_use]
    pub fn with_journal(mut self, journal: Vec<JournalEntry>) -> Self {
        self.journal = journal;
        self
    }

    /// Resume-token rows to INSERT in the same transaction (W-S3c).
    ///
    /// The engine sets one row on a signal-park commit. Backends insert with
    /// `ON CONFLICT(execution_id, node_key) DO NOTHING`, so a crash re-drive
    /// does not produce a duplicate live token.
    #[must_use]
    pub fn with_resume_tokens(mut self, tokens: Vec<ResumeTokenRow>) -> Self {
        self.resume_tokens = tokens;
        self
    }

    /// An execution-owned revision-reference mutation applied atomically with
    /// the transition (terminal commits only).
    #[must_use]
    pub const fn with_reference_transition(
        mut self,
        transition: ExecutionReferenceTransition,
    ) -> Self {
        self.reference_transition = Some(transition);
        self
    }

    /// Tenant scope this transition applies within.
    #[must_use]
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Target execution id (opaque string form).
    #[must_use]
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }

    /// CAS version the caller expects the row to be at.
    #[must_use]
    pub fn expected_version(&self) -> u64 {
        self.expected_version
    }

    /// Lease fencing token; a superseded token is rejected even on a
    /// version match.
    #[must_use]
    pub fn fencing(&self) -> FencingToken {
        self.fencing
    }

    /// Opaque new execution state to persist.
    #[must_use]
    pub fn new_state(&self) -> &serde_json::Value {
        &self.new_state
    }

    /// Listing projection of [`Self::new_state`]; backends write it in the
    /// same statement as the state.
    #[must_use]
    pub const fn listing(&self) -> ExecutionListing {
        self.listing
    }

    /// The same batch retargeted at `scope`: the batch itself, every outbox
    /// row, and every resume-token row.
    ///
    /// This is how a tenancy decorator binds a batch to its tenant. Every
    /// other field is carried over unchanged, so adding a field to the batch
    /// can never be silently dropped by a hand-written rebuild.
    #[must_use]
    pub fn rebound_to(&self, scope: &Scope) -> Self {
        let mut batch = self.clone();
        batch.scope = scope.clone();
        for message in &mut batch.outbox {
            message.scope = scope.clone();
        }
        for token in &mut batch.resume_tokens {
            token.scope = scope.clone();
        }
        batch
    }

    /// The same batch with `entries` appended to its journal rows.
    ///
    /// Used by backends that add an owner-written observation to a verified
    /// batch inside their own lock.
    #[must_use]
    pub fn with_appended_journal(&self, entries: impl IntoIterator<Item = JournalEntry>) -> Self {
        let mut batch = self.clone();
        batch.journal.extend(entries);
        batch
    }

    /// Control-queue rows to append in the same transaction.
    #[must_use]
    pub fn outbox(&self) -> &[ControlMsg] {
        &self.outbox
    }

    /// Journal rows to append in the same transaction.
    #[must_use]
    pub fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }

    /// Resume-token rows to INSERT in the same transaction (W-S3c).
    ///
    /// Empty on all non-signal-park commits.  Each backend must INSERT
    /// these rows using `ON CONFLICT(execution_id, node_key) DO NOTHING`
    /// so a crash re-drive that re-parks the same node does not mint a
    /// duplicate live token.
    #[must_use]
    pub fn resume_tokens(&self) -> &[ResumeTokenRow] {
        &self.resume_tokens
    }

    /// Optional execution-owned revision-reference mutation for this commit.
    ///
    /// `None` for non-terminal transitions.
    #[must_use]
    pub const fn reference_transition(&self) -> Option<ExecutionReferenceTransition> {
        self.reference_transition
    }
}

/// Result of `ExecutionStore::commit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    /// CAS + fencing succeeded; the row is now at `new_version`.
    Applied {
        /// Version the row was bumped to.
        new_version: u64,
    },
    /// CAS failed — the row's actual version differs from the expected one.
    VersionConflict {
        /// Version actually persisted at commit time.
        actual: u64,
    },
    /// The caller's fencing token was superseded by a newer lease
    /// generation; the transition was rejected even if the version matched.
    FencedOut,
}
