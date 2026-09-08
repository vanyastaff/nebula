use nebula_storage_port::{
    FencingToken, Scope, StorageError,
    store::{RecoverableTurn, RecoverableTurnPage, RecoveryTurnAcceptance, RecoveryTurnHandoff},
};

use super::{execution::SharedState, plan_flavor_catalog::execution_matches_live_flavor};

pub(super) struct AcceptedTurn {
    pub scope: Scope,
    pub generation: u64,
    pub source: &'static str,
    pub queue_id: [u8; 16],
}
impl std::fmt::Debug for AcceptedTurn {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AcceptedTurn")
            .field("scope", &self.scope)
            .field("generation", &self.generation)
            .field("source", &self.source)
            .field("queue_id", &self.queue_id)
            .finish()
    }
}

#[tracing::instrument(name = "turn_recovery.list", skip_all, fields(backend = "in_memory", limit, candidates = tracing::field::Empty))]
pub(super) fn list(
    inner: &SharedState,
    clock: &dyn nebula_core::accessor::Clock,
    flavor: nebula_core::WorkerFlavorRevisionId,
    after: Option<&str>,
    limit: u32,
) -> Result<RecoverableTurnPage, StorageError> {
    if !(1..=256).contains(&limit) {
        return Err(StorageError::Configuration(
            "recovery page limit must be in 1..=256".into(),
        ));
    }
    let state = inner.lock();
    let now = clock.now();
    let limit = usize::try_from(limit)
        .map_err(|_| StorageError::Internal("recovery limit is invalid".into()))?;
    let lower = after.map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
    let ids: Vec<_> = state
        .accepted_turns
        .range::<str, _>((lower, std::ops::Bound::Unbounded))
        .filter_map(|(id, marker)| {
            let row = state.rows.get(id)?;
            let execution = id.parse().ok()?;
            (row.scope == marker.scope
                && after.is_none_or(|after| id.as_str() > after)
                && execution_matches_live_flavor(&state.revision_catalog, execution, flavor))
            .then_some(id)
        })
        .take(limit)
        .collect();
    let next_cursor = (ids.len() == limit)
        .then(|| ids.last().map(|id| (*id).clone()))
        .flatten();
    let turns = ids
        .into_iter()
        .filter_map(|id| {
            let row = state.rows.get(id)?;
            let marker = state.accepted_turns.get(id)?;
            row.lease_expires_at
                .is_none_or(|expiry| expiry < now)
                .then(|| RecoverableTurn::new(marker.scope.clone(), id.clone(), marker.generation))
        })
        .collect::<Vec<_>>();
    tracing::Span::current().record("candidates", turns.len());
    Ok(RecoverableTurnPage::new(turns, next_cursor))
}

#[tracing::instrument(name = "turn_recovery.accept", skip_all, fields(backend = "in_memory", execution_id = handoff.execution_id(), outcome = tracing::field::Empty))]
pub(super) fn accept(
    inner: &SharedState,
    clock: &dyn nebula_core::accessor::Clock,
    handoff: &RecoveryTurnHandoff<'_>,
) -> Result<RecoveryTurnAcceptance, StorageError> {
    let result = (|| {
        i64::try_from(handoff.expected_execution_version())
            .map_err(|_| StorageError::Internal("recovery version is invalid".into()))?;
        let mut state = inner.lock();
        let Some(marker) = state
            .accepted_turns
            .get(handoff.execution_id())
            .filter(|marker| {
                &marker.scope == handoff.scope()
                    && marker.generation == handoff.expected_accepted_fencing_generation()
            })
        else {
            return Ok(RecoveryTurnAcceptance::CandidateSuperseded);
        };
        let Some(row) = state
            .rows
            .get(handoff.execution_id())
            .filter(|row| row.scope == marker.scope)
        else {
            return Ok(RecoveryTurnAcceptance::CandidateSuperseded);
        };
        let Ok(execution) = handoff.execution_id().parse() else {
            return Ok(RecoveryTurnAcceptance::CandidateSuperseded);
        };
        if !execution_matches_live_flavor(
            &state.revision_catalog,
            execution,
            handoff.worker_flavor_revision_id(),
        ) {
            return Ok(RecoveryTurnAcceptance::CandidateSuperseded);
        }
        if row.version != handoff.expected_execution_version() {
            return Ok(RecoveryTurnAcceptance::VersionConflict {
                actual: row.version,
            });
        }
        let now = clock.now();
        if row.lease_expires_at.is_some_and(|expiry| expiry >= now) {
            return Ok(RecoveryTurnAcceptance::TurnHeldByAnotherOwner);
        }
        let generation = row
            .fencing_generation
            .checked_add(1)
            .filter(|generation| {
                marker.generation > 0
                    && row.fencing_generation >= marker.generation
                    && i64::try_from(*generation).is_ok()
            })
            .ok_or_else(|| StorageError::Internal("recovery fence is exhausted".into()))?;
        let ttl = handoff.lease_ttl().clamp(
            std::time::Duration::from_secs(1),
            std::time::Duration::from_hours(24),
        );
        let duration = chrono::Duration::from_std(ttl)
            .map_err(|_| StorageError::Internal("recovery lease is invalid".into()))?;
        let expires = now
            .checked_add_signed(duration)
            .ok_or_else(|| StorageError::Internal("recovery deadline is invalid".into()))?;
        let super::execution::State {
            rows,
            accepted_turns,
            ..
        } = &mut *state;
        let row = rows
            .get_mut(handoff.execution_id())
            .ok_or_else(|| StorageError::Internal("recovery execution disappeared".into()))?;
        let marker = accepted_turns
            .get_mut(handoff.execution_id())
            .ok_or_else(|| StorageError::Internal("recovery marker disappeared".into()))?;
        row.fencing_generation = generation;
        row.lease_holder = Some(handoff.holder().to_owned());
        row.lease_expires_at = Some(expires);
        marker.generation = generation;
        Ok(RecoveryTurnAcceptance::Accepted {
            fence: FencingToken::from_generation(generation),
        })
    })();
    tracing::Span::current().record(
        "outcome",
        match &result {
            Ok(RecoveryTurnAcceptance::Accepted { .. }) => "accepted",
            Ok(RecoveryTurnAcceptance::CandidateSuperseded) => "candidate_superseded",
            Ok(RecoveryTurnAcceptance::TurnHeldByAnotherOwner) => "turn_held",
            Ok(RecoveryTurnAcceptance::VersionConflict { .. }) => "version_conflict",
            Ok(_) => "unsupported_outcome",
            Err(_) => "error",
        },
    );
    result
}
