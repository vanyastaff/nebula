//! Backend-independent iteration-checkpoint decisions.
//!
//! The in-memory reference model, SQLite, and PostgreSQL must answer every
//! save identically — monotone upsert, exact recommit, conflict, regress —
//! so the decision is made once here, from the stored row's iteration and
//! digest, under each backend's execution fence. Row plumbing stays in each
//! adapter.

use nebula_storage_port::{CheckpointSaved, IterationCheckpoint, IterationCheckpointError};

use crate::execution_fence::FenceRefusal;

/// What a fenced save does to the stored row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveDecision {
    /// No row is stored: insert one.
    Insert,
    /// A row of a lower iteration is stored: replace it.
    Replace,
    /// The very same checkpoint is stored: change nothing.
    AlreadyRecorded,
}

impl SaveDecision {
    /// What the caller is told once the decision is applied.
    pub(crate) const fn saved(self) -> CheckpointSaved {
        match self {
            Self::Insert | Self::Replace => CheckpointSaved::Recorded,
            Self::AlreadyRecorded => CheckpointSaved::AlreadyRecorded,
        }
    }
}

/// Decides a save of `incoming` against the stored row's iteration and
/// digest, if a row is stored.
///
/// # Errors
///
/// [`Conflict`](IterationCheckpointError::Conflict) for the same iteration
/// with another digest; [`Regressed`](IterationCheckpointError::Regressed)
/// when the stored row names a later iteration.
pub(crate) fn decide_save(
    stored: Option<(u32, &[u8; 32])>,
    incoming: &IterationCheckpoint,
) -> Result<SaveDecision, IterationCheckpointError> {
    let Some((stored_iteration, stored_digest)) = stored else {
        return Ok(SaveDecision::Insert);
    };
    match stored_iteration.cmp(&incoming.iteration()) {
        std::cmp::Ordering::Less => Ok(SaveDecision::Replace),
        std::cmp::Ordering::Equal if stored_digest == incoming.state_digest() => {
            Ok(SaveDecision::AlreadyRecorded)
        },
        std::cmp::Ordering::Equal => Err(IterationCheckpointError::Conflict),
        std::cmp::Ordering::Greater => Err(IterationCheckpointError::Regressed {
            stored: stored_iteration,
        }),
    }
}

/// The checkpoint store's answer to an execution fence that refused a save.
impl From<FenceRefusal> for IterationCheckpointError {
    fn from(refusal: FenceRefusal) -> Self {
        match refusal {
            FenceRefusal::LeaseRejected => Self::ExecutionLeaseRejected,
            FenceRefusal::Unavailable => Self::Unavailable,
        }
    }
}

/// Stable label of a save's outcome, so every adapter reports the same
/// vocabulary on its spans.
pub(crate) const fn save_label(
    result: Result<CheckpointSaved, IterationCheckpointError>,
) -> &'static str {
    match result {
        Ok(CheckpointSaved::Recorded) => "recorded",
        Ok(CheckpointSaved::AlreadyRecorded) => "already_recorded",
        Ok(_) => "saved",
        Err(error) => error.label(),
    }
}

/// Stable label of a load's outcome.
pub(crate) const fn load_label(
    result: &Result<Option<IterationCheckpoint>, IterationCheckpointError>,
) -> &'static str {
    match result {
        Ok(Some(_)) => "found",
        Ok(None) => "absent",
        Err(error) => error.label(),
    }
}

/// Rebuilds a stored row from its SQL columns; any value outside its typed
/// range is [`InvalidRecord`](IterationCheckpointError::InvalidRecord).
#[cfg(any(feature = "sqlite", feature = "postgres"))]
#[expect(
    clippy::too_many_arguments,
    reason = "one argument per stored column, decoded in one place for both SQL backends"
)]
pub(crate) fn stored_checkpoint(
    iteration: i64,
    state: Vec<u8>,
    state_digest: Vec<u8>,
    resume_delay_ms: Option<i64>,
    attested_positions: i64,
    attempt_generation: i64,
    fencing_generation: i64,
    written_at_ms: i64,
) -> Result<IterationCheckpoint, IterationCheckpointError> {
    let invalid = |_range| IterationCheckpointError::InvalidRecord;
    let digest = <[u8; 32]>::try_from(state_digest)
        .map_err(|_width| IterationCheckpointError::InvalidRecord)?;
    let checkpoint = IterationCheckpoint::new(
        u32::try_from(iteration).map_err(invalid)?,
        state,
        digest,
        resume_delay_ms
            .map(u64::try_from)
            .transpose()
            .map_err(invalid)?,
        u32::try_from(attested_positions).map_err(invalid)?,
        u64::try_from(attempt_generation).map_err(invalid)?,
    )
    // A stored row that no longer fits the record's bounds is corrupt.
    .map_err(|_bounds| IterationCheckpointError::InvalidRecord)?;
    Ok(checkpoint.with_write_provenance(
        u64::try_from(fencing_generation).map_err(invalid)?,
        written_at_ms,
    ))
}

/// The SQL value of a `u64` the record already bounded to `i64`.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn durable_integer(value: u64) -> Result<i64, IterationCheckpointError> {
    i64::try_from(value).map_err(|_range| IterationCheckpointError::InvalidRecord)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint(iteration: u32, digest: u8) -> IterationCheckpoint {
        IterationCheckpoint::new(iteration, b"{}".to_vec(), [digest; 32], None, 0, 1).unwrap()
    }

    #[test]
    fn saves_are_monotone_and_exact() {
        let incoming = checkpoint(3, 1);
        assert_eq!(decide_save(None, &incoming), Ok(SaveDecision::Insert));
        assert_eq!(
            decide_save(Some((2, &[9; 32])), &incoming),
            Ok(SaveDecision::Replace)
        );
        assert_eq!(
            decide_save(Some((3, &[1; 32])), &incoming),
            Ok(SaveDecision::AlreadyRecorded)
        );
        assert_eq!(
            decide_save(Some((3, &[2; 32])), &incoming),
            Err(IterationCheckpointError::Conflict)
        );
        assert_eq!(
            decide_save(Some((4, &[1; 32])), &incoming),
            Err(IterationCheckpointError::Regressed { stored: 4 })
        );
    }

    #[test]
    fn a_fence_refusal_maps_onto_the_store_error() {
        assert_eq!(
            IterationCheckpointError::from(FenceRefusal::LeaseRejected),
            IterationCheckpointError::ExecutionLeaseRejected
        );
        assert_eq!(
            IterationCheckpointError::from(FenceRefusal::Unavailable),
            IterationCheckpointError::Unavailable
        );
    }
}
