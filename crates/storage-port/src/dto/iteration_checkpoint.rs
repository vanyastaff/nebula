//! Fenced, version-bound iteration checkpoints of journaled stateful actions.
//!
//! A journaled stateful action runs its iterations inside one node attempt,
//! each bracketed by the node effect journal's barrier. Once an iteration's
//! barrier passed, the engine may record an [`IterationCheckpoint`]: the next
//! iteration to run and the state to run it with. A later attempt of the same
//! node resumes there instead of replaying every earlier iteration.
//!
//! The record is addressed by an [`IterationCheckpointKey`] that binds the
//! action key **and** version: a redeployed action never reads a checkpoint
//! an older version wrote. It is written only under the execution's live
//! lease ([`crate::FencingToken`]), monotonically: a lower iteration never
//! replaces a higher one, and an equal one must be an exact recommit.
//!
//! **Redaction.** The state is the action's own data. Adapters never log it,
//! [`Debug`](std::fmt::Debug) prints its length and digest only, and every
//! [`IterationCheckpointError`] is payload-free. Credentials and secrets never
//! belong in a stateful action's state: it is persisted as given, unencrypted.

use crate::scope::Scope;

/// Most bytes an iteration checkpoint's state may hold: 1 MiB.
pub const MAX_ITERATION_CHECKPOINT_STATE_BYTES: usize = 1_048_576;

/// Highest iteration a checkpoint may name as the next one to run: the
/// stateful runtime runs at most this many iterations.
pub const MAX_CHECKPOINT_ITERATION: u32 = 10_000;

/// Most bytes of the execution, node and action-key parts of an
/// [`IterationCheckpointKey`] (the version is bounded by action metadata
/// admission instead).
pub const MAX_ITERATION_CHECKPOINT_KEY_PART_BYTES: usize = 512;

/// Why an iteration checkpoint could not be loaded or saved.
///
/// Payload-free: no state bytes, driver message or SQL ever crosses this
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IterationCheckpointError {
    /// The execution is absent, outside this scope, or lacks this live
    /// lease. Nothing was written.
    #[error("execution lease does not authorize this iteration checkpoint")]
    ExecutionLeaseRejected,
    /// A checkpoint of a later iteration is already stored: a checkpoint
    /// never moves backwards. Nothing was written.
    #[error("a later iteration checkpoint ({stored}) is already stored")]
    Regressed {
        /// The iteration the stored checkpoint names.
        stored: u32,
    },
    /// A checkpoint of the same iteration with other state is already
    /// stored. Nothing was written.
    #[error("an iteration checkpoint of the same iteration with other state is stored")]
    Conflict,
    /// The state exceeds [`MAX_ITERATION_CHECKPOINT_STATE_BYTES`]. Nothing
    /// was written.
    #[error("iteration checkpoint state exceeds its size bound")]
    TooLarge,
    /// The key or record violates its bounds, or a stored row cannot be
    /// interpreted.
    #[error("iteration checkpoint record is invalid")]
    InvalidRecord,
    /// The backend failed before it could decide; nothing was written.
    #[error("iteration checkpoint store is unavailable")]
    Unavailable,
    /// The commit's acknowledgement was lost: the write may or may not have
    /// landed.
    #[error("iteration checkpoint commit acknowledgement is unknown")]
    AcknowledgementUnknown,
}

impl IterationCheckpointError {
    /// Whether the failure says nothing about the stored checkpoint — the
    /// backend could not be reached, the acknowledgement was lost, or the
    /// caller's lease no longer authorizes it — so a later attempt, under a
    /// live lease, may succeed.
    #[must_use]
    pub const fn is_deferred(self) -> bool {
        matches!(
            self,
            Self::Unavailable | Self::AcknowledgementUnknown | Self::ExecutionLeaseRejected
        )
    }

    /// Stable bounded label for spans and counters.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ExecutionLeaseRejected => "lease_rejected",
            Self::Regressed { .. } => "regressed",
            Self::Conflict => "conflict",
            Self::TooLarge => "too_large",
            Self::InvalidRecord => "invalid_record",
            Self::Unavailable => "unavailable",
            Self::AcknowledgementUnknown => "acknowledgement_unknown",
        }
    }
}

/// How a fenced save of an iteration checkpoint ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CheckpointSaved {
    /// The checkpoint was inserted, or replaced one of an earlier iteration.
    Recorded,
    /// The very same checkpoint (same iteration, same state digest) was
    /// already stored: an exact recommit, nothing changed.
    AlreadyRecorded,
}

/// The address of one iteration checkpoint: tenant, execution, node, and the
/// action key and version that wrote it.
///
/// Binding the version means a redeployed action reads no checkpoint an
/// older version wrote: it starts at iteration 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IterationCheckpointKey<'a> {
    scope: &'a Scope,
    execution_id: &'a str,
    node_key: &'a str,
    action_key: &'a str,
    action_version: &'a str,
}

impl<'a> IterationCheckpointKey<'a> {
    /// Build an address. These values grant no execution authority.
    ///
    /// # Errors
    ///
    /// [`IterationCheckpointError::InvalidRecord`] when the execution, node or
    /// action key is empty or longer than
    /// [`MAX_ITERATION_CHECKPOINT_KEY_PART_BYTES`], or when `action_version`
    /// is not a canonical semantic version (`MAJOR.MINOR.PATCH`, no leading
    /// zeros, optional pre-release and build of `[0-9A-Za-z-.]`). The version
    /// takes no length cap of its own: action metadata admission already
    /// bounds it, and any version it admits must address a checkpoint. The
    /// tenant scope is taken as the execution was admitted under it — `Scope`
    /// and `port_executions` bound neither part — so every admitted execution
    /// can be checkpointed, and a scope-enforcing decorator's
    /// [`rescoped`](Self::rescoped) address is held to the same rule.
    pub fn new(
        scope: &'a Scope,
        execution_id: &'a str,
        node_key: &'a str,
        action_key: &'a str,
        action_version: &'a str,
    ) -> Result<Self, IterationCheckpointError> {
        let parts = [execution_id, node_key, action_key];
        if parts
            .iter()
            .any(|part| part.is_empty() || part.len() > MAX_ITERATION_CHECKPOINT_KEY_PART_BYTES)
            || !is_canonical_semver(action_version)
        {
            return Err(IterationCheckpointError::InvalidRecord);
        }
        Ok(Self {
            scope,
            execution_id,
            node_key,
            action_key,
            action_version,
        })
    }

    /// The same address under another tenant scope — what a scope-enforcing
    /// decorator substitutes. The other parts were validated already.
    #[must_use]
    pub const fn rescoped<'b>(&self, scope: &'b Scope) -> IterationCheckpointKey<'b>
    where
        'a: 'b,
    {
        IterationCheckpointKey {
            scope,
            execution_id: self.execution_id,
            node_key: self.node_key,
            action_key: self.action_key,
            action_version: self.action_version,
        }
    }

    /// Tenant scope.
    #[must_use]
    pub const fn scope(&self) -> &'a Scope {
        self.scope
    }

    /// Owning execution identity.
    #[must_use]
    pub const fn execution_id(&self) -> &'a str {
        self.execution_id
    }

    /// The node whose stateful action wrote the checkpoint.
    #[must_use]
    pub const fn node_key(&self) -> &'a str {
        self.node_key
    }

    /// The action key that wrote the checkpoint.
    #[must_use]
    pub const fn action_key(&self) -> &'a str {
        self.action_key
    }

    /// The action version that wrote the checkpoint (canonical semver).
    #[must_use]
    pub const fn action_version(&self) -> &'a str {
        self.action_version
    }
}

/// Whether `version` is a canonical semantic version: `MAJOR.MINOR.PATCH`
/// in decimal without leading zeros, then an optional `-pre-release` and
/// `+build` of non-empty dot-separated `[0-9A-Za-z-]` identifiers.
fn is_canonical_semver(version: &str) -> bool {
    let (version, build) = match version.split_once('+') {
        Some((version, build)) => (version, Some(build)),
        None => (version, None),
    };
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let numeric = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|byte| byte.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
            && part.parse::<u64>().is_ok()
    };
    let identifiers = |text: &str| {
        text.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    };
    let mut components = core.split('.');
    let three = (0..3).all(|_| components.next().is_some_and(numeric));
    three
        && components.next().is_none()
        && pre.is_none_or(identifiers)
        && build.is_none_or(identifiers)
}

/// One iteration checkpoint of a journaled stateful action.
///
/// Built by the engine with [`new`](Self::new); an adapter attaches the
/// write's provenance ([`with_write_provenance`](Self::with_write_provenance))
/// when it stores or reads one back.
#[derive(Clone, PartialEq, Eq)]
pub struct IterationCheckpoint {
    iteration: u32,
    state: Vec<u8>,
    state_digest: [u8; 32],
    resume_delay_ms: Option<u64>,
    attested_positions: u32,
    attempt_generation: u64,
    fencing_generation: u64,
    written_at_ms: i64,
}

impl IterationCheckpoint {
    /// A checkpoint naming `iteration` as the next one to run with `state`
    /// (canonical JSON bytes) and its SHA-256 `state_digest`, the delay the
    /// action asked for before it, how many iterated ledger positions below
    /// `iteration` it attests, and the attempt that wrote it (provenance
    /// only: it grants nothing).
    ///
    /// The port does not hash: the engine computes the digest and verifies
    /// it on load; an adapter compares it to recognize an exact recommit.
    ///
    /// # Errors
    ///
    /// - [`TooLarge`](IterationCheckpointError::TooLarge) when `state`
    ///   exceeds [`MAX_ITERATION_CHECKPOINT_STATE_BYTES`];
    /// - [`InvalidRecord`](IterationCheckpointError::InvalidRecord) when
    ///   `iteration` is below 1 or above [`MAX_CHECKPOINT_ITERATION`], or the
    ///   delay or generation exceeds the portable durable range (`i64`).
    pub fn new(
        iteration: u32,
        state: Vec<u8>,
        state_digest: [u8; 32],
        resume_delay_ms: Option<u64>,
        attested_positions: u32,
        attempt_generation: u64,
    ) -> Result<Self, IterationCheckpointError> {
        if state.len() > MAX_ITERATION_CHECKPOINT_STATE_BYTES {
            return Err(IterationCheckpointError::TooLarge);
        }
        let portable = |value: u64| i64::try_from(value).is_ok();
        if !(1..=MAX_CHECKPOINT_ITERATION).contains(&iteration)
            || !resume_delay_ms.is_none_or(portable)
            || !portable(attempt_generation)
        {
            return Err(IterationCheckpointError::InvalidRecord);
        }
        Ok(Self {
            iteration,
            state,
            state_digest,
            resume_delay_ms,
            attested_positions,
            attempt_generation,
            fencing_generation: 0,
            written_at_ms: 0,
        })
    }

    /// This checkpoint as stored: written under `fencing_generation` at
    /// `written_at_ms` (the backend's clock). Adapters call this; the
    /// engine's own values are ignored on save.
    #[must_use]
    pub const fn with_write_provenance(
        mut self,
        fencing_generation: u64,
        written_at_ms: i64,
    ) -> Self {
        self.fencing_generation = fencing_generation;
        self.written_at_ms = written_at_ms;
        self
    }

    /// The next iteration to run (`1..=10_000`).
    #[must_use]
    pub const fn iteration(&self) -> u32 {
        self.iteration
    }

    /// The state to run it with: canonical JSON bytes.
    #[must_use]
    pub fn state(&self) -> &[u8] {
        &self.state
    }

    /// SHA-256 of [`state`](Self::state).
    #[must_use]
    pub const fn state_digest(&self) -> &[u8; 32] {
        &self.state_digest
    }

    /// The delay the action asked for before [`iteration`](Self::iteration),
    /// in milliseconds.
    #[must_use]
    pub const fn resume_delay_ms(&self) -> Option<u64> {
        self.resume_delay_ms
    }

    /// How many distinct iterated ledger positions below
    /// [`iteration`](Self::iteration) the node's ledger held when this was
    /// written: a resume that finds another count halts.
    #[must_use]
    pub const fn attested_positions(&self) -> u32 {
        self.attested_positions
    }

    /// The node attempt that wrote it (provenance only).
    #[must_use]
    pub const fn attempt_generation(&self) -> u64 {
        self.attempt_generation
    }

    /// The execution fencing generation it was written under (adapter
    /// written; `0` before it is stored).
    #[must_use]
    pub const fn fencing_generation(&self) -> u64 {
        self.fencing_generation
    }

    /// When it was written, in milliseconds since the Unix epoch by the
    /// backend's clock (adapter written; `0` before it is stored).
    #[must_use]
    pub const fn written_at_ms(&self) -> i64 {
        self.written_at_ms
    }
}

impl std::fmt::Debug for IterationCheckpoint {
    /// Never prints the state: its length and digest only.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Write as _;
        let digest = self
            .state_digest
            .iter()
            .fold(String::with_capacity(64), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            });
        formatter
            .debug_struct("IterationCheckpoint")
            .field("iteration", &self.iteration)
            .field("state_bytes", &self.state.len())
            .field("state_digest", &digest)
            .field("resume_delay_ms", &self.resume_delay_ms)
            .field("attested_positions", &self.attested_positions)
            .field("attempt_generation", &self.attempt_generation)
            .field("fencing_generation", &self.fencing_generation)
            .field("written_at_ms", &self.written_at_ms)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_semver_is_strict() {
        for valid in [
            "0.0.0",
            "1.2.3",
            "10.20.30",
            "1.0.0-alpha.1",
            "1.0.0+build.7",
            "1.0.0-rc-1+b",
        ] {
            assert!(is_canonical_semver(valid), "{valid}");
        }
        for invalid in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.3-",
            "1.2.3+",
            "1.2.3-a..b",
            "v1.2.3",
            "1.2.3 ",
            "1.2.3-é",
        ] {
            assert!(!is_canonical_semver(invalid), "{invalid}");
        }
    }

    /// Any scope an execution was admitted under addresses a checkpoint —
    /// neither `Scope` nor `port_executions` bounds it — while the
    /// checkpoint's own parts stay bounded.
    #[test]
    fn a_key_takes_any_admitted_scope_and_bounds_its_own_parts() {
        let long = "w".repeat(MAX_ITERATION_CHECKPOINT_KEY_PART_BYTES + 1);
        for scope in [Scope::new("", ""), Scope::new(long.as_str(), "org")] {
            assert!(
                IterationCheckpointKey::new(&scope, "exec", "node", "a.b", "1.0.0").is_ok(),
                "{scope:?}"
            );
        }
        let scope = Scope::new("ws", "org");
        // Any version metadata admission accepts addresses a checkpoint,
        // however long its build suffix.
        let version = format!(
            "1.0.0+{}",
            "b".repeat(MAX_ITERATION_CHECKPOINT_KEY_PART_BYTES + 1)
        );
        assert!(IterationCheckpointKey::new(&scope, "exec", "node", "a.b", &version).is_ok());
        for (execution, node, action) in [
            ("", "node", "a.b"),
            ("exec", "", "a.b"),
            ("exec", "node", ""),
        ] {
            assert_eq!(
                IterationCheckpointKey::new(&scope, execution, node, action, "1.0.0"),
                Err(IterationCheckpointError::InvalidRecord)
            );
        }
    }
}
