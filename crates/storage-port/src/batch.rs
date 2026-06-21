//! Atomic state-transition unit-of-work.
//!
//! [`TransitionBatch`] is the *only* way to apply a state transition. Its
//! fields are private and it has no public constructor other than
//! [`TransitionBatch::builder`], so a caller structurally cannot transition
//! without declaring the scope, the expected CAS version, and the lease
//! [`FencingToken`]. `commit` writes `new_state` + `outbox` + `journal` +
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
use crate::dto::resume_token::ResumeTokenRow;
use crate::dto::{ControlMsg, JournalEntry};
use crate::error::StorageError;
use crate::ids::FencingToken;
use crate::scope::Scope;

/// The atomic transition payload consumed by `ExecutionStore::commit`.
#[derive(Debug, Clone)]
pub struct TransitionBatch {
    scope: Scope,
    execution_id: String,
    expected_version: u64,
    fencing: FencingToken,
    new_state: serde_json::Value,
    outbox: Vec<ControlMsg>,
    journal: Vec<JournalEntry>,
    /// Resume-token rows to INSERT in the same transaction as the state
    /// snapshot.  Empty on non-signal-park commits.  The INSERT uses
    /// `ON CONFLICT(execution_id, node_key) DO NOTHING` so a crash
    /// re-drive that re-parks the same node does NOT mint a second token.
    resume_tokens: Vec<ResumeTokenRow>,
}

impl TransitionBatch {
    /// Start building a batch. Required: `scope`, `execution_id`,
    /// `expected_version`, `fencing`, `new_state`. `outbox`/`journal`
    /// default empty.
    #[must_use]
    pub fn builder() -> TransitionBatchBuilder {
        TransitionBatchBuilder::default()
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
}

/// Typed builder for [`TransitionBatch`]. A missing required field makes
/// [`TransitionBatchBuilder::build`] fail closed with
/// [`StorageError::Configuration`] rather than panicking.
#[derive(Debug, Default)]
pub struct TransitionBatchBuilder {
    scope: Option<Scope>,
    execution_id: Option<String>,
    expected_version: Option<u64>,
    fencing: Option<FencingToken>,
    new_state: Option<serde_json::Value>,
    outbox: Vec<ControlMsg>,
    journal: Vec<JournalEntry>,
    resume_tokens: Vec<ResumeTokenRow>,
}

impl TransitionBatchBuilder {
    /// Set the tenant scope (required).
    #[must_use]
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Set the target execution id (required).
    #[must_use]
    pub fn execution_id(mut self, id: impl Into<String>) -> Self {
        self.execution_id = Some(id.into());
        self
    }

    /// Set the expected CAS version (required).
    #[must_use]
    pub fn expected_version(mut self, v: u64) -> Self {
        self.expected_version = Some(v);
        self
    }

    /// Set the lease fencing token (required).
    #[must_use]
    pub fn fencing(mut self, token: FencingToken) -> Self {
        self.fencing = Some(token);
        self
    }

    /// Set the opaque new execution state (required).
    #[must_use]
    pub fn new_state(mut self, state: serde_json::Value) -> Self {
        self.new_state = Some(state);
        self
    }

    /// Set the outbox rows to append atomically (optional, default empty).
    #[must_use]
    pub fn outbox(mut self, outbox: Vec<ControlMsg>) -> Self {
        self.outbox = outbox;
        self
    }

    /// Set the journal rows to append atomically (optional, default empty).
    #[must_use]
    pub fn journal(mut self, journal: Vec<JournalEntry>) -> Self {
        self.journal = journal;
        self
    }

    /// Set the resume-token rows to INSERT atomically (W-S3c).
    ///
    /// Optional — default is empty.  The engine sets this to a
    /// single-element vec on signal-park commits.  Backends must use
    /// `ON CONFLICT(execution_id, node_key) DO NOTHING` so a crash
    /// re-drive does not produce a duplicate live token.
    #[must_use]
    pub fn resume_tokens(mut self, tokens: Vec<ResumeTokenRow>) -> Self {
        self.resume_tokens = tokens;
        self
    }

    /// Finalize the batch. Fails closed if any required field is missing.
    pub fn build(self) -> Result<TransitionBatch, StorageError> {
        let scope = self
            .scope
            .ok_or_else(|| StorageError::Configuration("TransitionBatch.scope missing".into()))?;
        let execution_id = self.execution_id.ok_or_else(|| {
            StorageError::Configuration("TransitionBatch.execution_id missing".into())
        })?;
        let expected_version = self.expected_version.ok_or_else(|| {
            StorageError::Configuration("TransitionBatch.expected_version missing".into())
        })?;
        let fencing = self
            .fencing
            .ok_or_else(|| StorageError::Configuration("TransitionBatch.fencing missing".into()))?;
        let new_state = self.new_state.ok_or_else(|| {
            StorageError::Configuration("TransitionBatch.new_state missing".into())
        })?;
        Ok(TransitionBatch {
            scope,
            execution_id,
            expected_version,
            fencing,
            new_state,
            outbox: self.outbox,
            journal: self.journal,
            resume_tokens: self.resume_tokens,
        })
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
