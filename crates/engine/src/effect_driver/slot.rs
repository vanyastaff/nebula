//! The ledger mechanics of one durable effect slot.
//!
//! [`LedgerSlot`] is the shared core of every execution-owned effect: the
//! remote-effect [`Driver`] and the node effect journal
//! ([`NodeEffectJournal`]) both prepare, advance and record a slot through
//! it. It owns no policy of its own — which
//! command to issue and what a phase means is the caller's — only the
//! protocol checks that make a durable answer trustworthy:
//!
//! - **prepare** recovers an unacknowledged preparation by the slot's natural
//!   address and never treats an unconfirmed binding as prepared;
//! - **advance** accepts a returned record only when its revision, counters,
//!   phase and call identity follow from the command just issued;
//! - **commit evidence** recommits exactly the same frozen evidence after a
//!   lost acknowledgement, and never anything else.

use super::*;

/// Write authority over the operation ledger, borrowed from the
/// execution-owning turn: every mutation is fenced by `fencing`.
#[derive(Clone, Copy)]
pub(super) struct LedgerAccess<'a> {
    pub(super) ledger: &'a dyn OperationLedger,
    pub(super) scope: &'a Scope,
    pub(super) fencing: FencingToken,
    pub(super) clock: &'a dyn Clock,
}

/// One freshly granted provider or query call, as the ledger issued it.
pub(super) enum GrantedCall {
    Invocation {
        call: OperationCallId,
        authorized_at_ms: i64,
        request_started: Instant,
    },
    Reconciliation {
        call: OperationCallId,
        authorized_at_ms: i64,
        request_started: Instant,
    },
}

/// What a granted call is for: it bounds the call by a different window.
pub(super) enum CallPurpose {
    Invocation,
    Reconciliation,
}

/// One prepared effect slot and the last durable record the ledger
/// acknowledged for it.
pub(super) struct LedgerSlot {
    contract: PreparedEffectContract,
    fingerprint: RequestFingerprint,
    record: OperationRecord,
}

impl LedgerSlot {
    /// Durably prepares `binding` (or replays its existing preparation) and
    /// reads back the exact record.
    ///
    /// A lost prepare acknowledgement is recovered by the natural address
    /// only: the slot is prepared when the ledger holds it, and an absent or
    /// unreadable one leaves the acknowledgement unknown.
    pub(super) async fn prepare(
        access: LedgerAccess<'_>,
        binding: &EffectSlotBinding<'_>,
    ) -> Result<Self, EffectExecutionError> {
        let record = match access.ledger.prepare(binding, access.fencing).await {
            Ok(outcome) => {
                let record = access
                    .ledger
                    .read_exact(access.scope, outcome.operation().slot_id())
                    .await?;
                if record.operation() != outcome.operation() {
                    return Err(EffectExecutionError::InvalidEvidence);
                }
                record
            },
            Err(OperationLedgerError::AcknowledgementUnknown) => access
                .ledger
                .read_occurrence(&binding.occurrence_key())
                .await
                .map_err(|_| OperationLedgerError::AcknowledgementUnknown)?
                .ok_or(OperationLedgerError::AcknowledgementUnknown)?,
            Err(error) => return Err(error.into()),
        };
        let slot = Self {
            contract: binding.contract.clone(),
            fingerprint: binding.fingerprint,
            record,
        };
        slot.validate_record(&slot.record)?;
        Ok(slot)
    }

    /// The storage-minted slot identity.
    pub(super) fn slot_id(&self) -> nebula_storage_port::dto::EffectSlotId {
        self.record.operation().slot_id()
    }

    /// The provider idempotency key recorded when the slot was first
    /// prepared: the durable value, never a recomputation.
    pub(super) fn provider_key(&self) -> Option<nebula_storage_port::dto::ProviderIdempotencyKey> {
        self.record.operation().provider_key()
    }

    /// The operation identity the provider receives.
    pub(super) fn operation_id(&self) -> OperationId {
        self.record.operation().operation_id()
    }

    /// The acknowledged operation record.
    pub(super) fn record(&self) -> &OperationRecord {
        &self.record
    }

    /// The acknowledged protocol projection.
    pub(super) fn protocol(
        &self,
    ) -> Result<&nebula_storage_port::dto::OperationProtocolRecord, EffectExecutionError> {
        self.record
            .protocol()
            .ok_or(EffectExecutionError::InvalidEvidence)
    }

    fn validate_record(&self, record: &OperationRecord) -> Result<(), EffectExecutionError> {
        let protocol = record
            .protocol()
            .ok_or(EffectExecutionError::InvalidEvidence)?;
        if record.operation() != self.record.operation()
            || record.fingerprint() != self.fingerprint
            || protocol.contract() != &self.contract
            || record.operation().destination() != self.contract.policy().capability()
        {
            return Err(EffectExecutionError::InvalidEvidence);
        }
        protocol.validate()?;
        if protocol.crossed_invocations() > self.contract.policy().max_invocations()
            || protocol.queries() > self.contract.policy().max_queries()
            || (protocol.phase() == EffectPhase::Resolved) != protocol.evidence().is_some()
        {
            return Err(EffectExecutionError::InvalidEvidence);
        }
        if let Some(evidence) = protocol.evidence() {
            evidence
                .validate()
                .map_err(|_| EffectExecutionError::InvalidEvidence)?;
        }
        Ok(())
    }

    fn accept(&mut self, record: OperationRecord) -> Result<(), EffectExecutionError> {
        self.validate_record(&record)?;
        self.record = record;
        Ok(())
    }

    /// Issues `command` under the turn's fence and accepts the resulting
    /// record only when it is the transition `command` asked for.
    pub(super) async fn advance(
        &mut self,
        access: LedgerAccess<'_>,
        command: &OperationCommand,
    ) -> Result<Option<GrantedCall>, EffectExecutionError> {
        let previous = self.protocol()?;
        let revision = previous.revision();
        let invocations = previous.invocations();
        let queries = previous.queries();
        let request_started = access.clock.monotonic();
        let advance = access
            .ledger
            .advance(
                access.scope,
                self.record.operation().slot_id(),
                access.fencing,
                command,
            )
            .await?;
        match advance {
            OperationAdvance::Granted {
                call,
                authorized_at_ms,
                record,
            } => {
                let protocol = record
                    .protocol()
                    .ok_or(EffectExecutionError::InvalidEvidence)?;
                if Some(protocol.revision()) != revision.checked_add(1)
                    || Some(protocol.invocations()) != invocations.checked_add(1)
                    || protocol.queries() != queries
                    || protocol.phase() != EffectPhase::InvocationOutstanding
                    || protocol.invocation() != Some(call)
                    || protocol.query().is_some()
                {
                    return Err(EffectExecutionError::InvalidEvidence);
                }
                self.accept(record)?;
                Ok(Some(GrantedCall::Invocation {
                    call,
                    authorized_at_ms,
                    request_started,
                }))
            },
            OperationAdvance::ReconciliationGranted {
                call,
                authorized_at_ms,
                record,
            } => {
                let protocol = record
                    .protocol()
                    .ok_or(EffectExecutionError::InvalidEvidence)?;
                if Some(protocol.revision()) != revision.checked_add(1)
                    || Some(protocol.queries()) != queries.checked_add(1)
                    || protocol.invocations() != invocations
                    || protocol.query() != Some(call)
                    || protocol.phase() != EffectPhase::OutcomeUnknown
                {
                    return Err(EffectExecutionError::InvalidEvidence);
                }
                self.accept(record)?;
                Ok(Some(GrantedCall::Reconciliation {
                    call,
                    authorized_at_ms,
                    request_started,
                }))
            },
            OperationAdvance::Recorded(record) => {
                let recorded_revision = record
                    .protocol()
                    .ok_or(EffectExecutionError::InvalidEvidence)?
                    .revision();
                if !recorded_revision_is_valid(command, revision, recorded_revision) {
                    return Err(EffectExecutionError::InvalidEvidence);
                }
                self.accept(record)?;
                Ok(None)
            },
            _ => Err(EffectExecutionError::InvalidEvidence),
        }
    }

    /// The backend-clock deadline of the call for `purpose` that the current
    /// record just authorized at `authorized_at_ms`.
    ///
    /// The ledger lets a slot whose every earlier call was proven not to cross
    /// be granted at any age, so the first call that may reach the provider
    /// takes its window from its own authorization; every later call, and
    /// every reconciliation query, stays bounded by the window measured from
    /// preparation, as the ledger's re-grant rule is.
    pub(super) fn deadline(
        &self,
        purpose: CallPurpose,
        authorized_at_ms: i64,
    ) -> Result<i64, EffectExecutionError> {
        let protocol = self.protocol()?;
        let policy = self.contract.policy();
        let (window, anchor_ms) = match purpose {
            CallPurpose::Reconciliation => (policy.recovery_window_ms(), protocol.prepared_at_ms()),
            CallPurpose::Invocation => (
                policy.stable_window_ms().map_or_else(
                    || policy.recovery_window_ms(),
                    |stable| stable.min(policy.recovery_window_ms()),
                ),
                if protocol.crossed_invocations() == 1 {
                    authorized_at_ms
                } else {
                    protocol.prepared_at_ms()
                },
            ),
        };
        anchor_ms
            .checked_add(i64::try_from(window).map_err(|_| EffectExecutionError::InvalidContract)?)
            .ok_or(EffectExecutionError::InvalidEvidence)
    }

    /// The deadline of a call granted at `authorized_at_ms` and the local
    /// budget left for it once the grant's round trip is paid.
    pub(super) fn call_timing(
        &self,
        clock: &dyn Clock,
        purpose: CallPurpose,
        authorized_at_ms: i64,
        request_started: Instant,
    ) -> Result<(i64, Duration), EffectExecutionError> {
        let deadline = self.deadline(purpose, authorized_at_ms)?;
        let remaining = remaining_call_budget(
            deadline,
            authorized_at_ms,
            request_started,
            clock.monotonic(),
        )
        .ok_or(EffectExecutionError::InvalidEvidence)?;
        Ok((deadline, remaining))
    }

    /// Records that no further invocation may be granted.
    pub(super) async fn mark_unknown(
        &mut self,
        access: LedgerAccess<'_>,
    ) -> Result<(), EffectExecutionError> {
        let revision = self.protocol()?.revision();
        self.advance(
            access,
            &OperationCommand::MarkUnknown {
                expected_revision: revision,
            },
        )
        .await?;
        Ok(())
    }

    /// Commits `evidence` as the slot's outcome.
    ///
    /// A lost or failed acknowledgement is recovered by reading the slot
    /// back: the exact same evidence already recorded is success, other
    /// evidence is invalid, and absent evidence permits one exact recommit
    /// of the same immutable command. No path invokes a provider.
    pub(super) async fn commit_evidence(
        &mut self,
        access: LedgerAccess<'_>,
        evidence: &FrozenOutcomeEvidence,
    ) -> Result<(), EffectExecutionError> {
        let command = OperationCommand::RecordOutcome(evidence.clone());
        let mut last_error = OperationLedgerError::Unavailable;
        let mut acknowledgement_unknown = false;
        for _ in 0..2 {
            match self.advance(access, &command).await {
                Ok(None) => {
                    if self.protocol()?.evidence() != Some(evidence) {
                        return Err(EffectExecutionError::InvalidEvidence);
                    }
                    return Ok(());
                },
                Ok(Some(_)) => return Err(EffectExecutionError::InvalidEvidence),
                Err(EffectExecutionError::Ledger(
                    error @ (OperationLedgerError::Unavailable
                    | OperationLedgerError::AcknowledgementUnknown),
                )) => {
                    acknowledgement_unknown |=
                        error == OperationLedgerError::AcknowledgementUnknown;
                    last_error = error;
                    let record = access
                        .ledger
                        .read_exact(access.scope, self.record.operation().slot_id())
                        .await
                        .map_err(|error| {
                            if acknowledgement_unknown {
                                OperationLedgerError::AcknowledgementUnknown
                            } else {
                                error
                            }
                        })?;
                    self.accept(record)?;
                    if let Some(recorded) = self.protocol()?.evidence() {
                        if recorded != evidence {
                            return Err(EffectExecutionError::InvalidEvidence);
                        }
                        return Ok(());
                    }
                    // Only this exact immutable command can be retried. No path
                    // from outcome acknowledgement recovery invokes a provider.
                },
                Err(error) => return Err(error),
            }
        }
        Err(if acknowledgement_unknown {
            OperationLedgerError::AcknowledgementUnknown
        } else {
            last_error
        }
        .into())
    }
}
