//! The effect journal of one node attempt of a journaled action.
//!
//! An action whose admitted contract is
//! [`Journaled`](nebula_action::effect::ActionEffectContract::Journaled)
//! submits its effects as units on resource handles
//! ([`ResourceHandle`](nebula_resource::call::ResourceHandle)). For a
//! stateless action on a durable turn the engine builds one [`NodeEffectJournal`] per
//! node attempt and hands it to the node's handles
//! ([`Manager::handle_any_journaled`](nebula_resource::Manager::handle_any_journaled));
//! the resource runtime drives every `Idempotent` or `Write` unit through
//! it (see [`nebula_resource::call::journal`]). The journal records each
//! unit as one slot of the operation ledger, through the same [`LedgerSlot`]
//! core as the remote-effect driver, under the turn's execution lease:
//!
//! | Unit step | Ledger command |
//! |---|---|
//! | prepare | `prepare` (natural key `(scope, execution, node, occurrence)`) |
//! | grant | `GrantInvocation` |
//! | explain | `RecordDisposition` (`BeforeBoundary` / `Ambiguous`) |
//! | settle | `RecordOutcome` (exact recommit on a lost acknowledgement) |
//!
//! **Lazy writes, one read.** Building a journal costs nothing durable: no
//! ledger write happens until the first `Idempotent` or `Write` unit is
//! prepared, and a `Read` unit is never prepared. Concluding always reads
//! the node's occurrences once — even for a node that only read: a process
//! that died during an earlier dispatch of the node, before that attempt
//! was recorded, leaves the next attempt at the same generation, and only
//! the ledger knows the call it may have made.
//!
//! **Occurrences.** A unit's occurrence is the label the resource runtime
//! builds, `unit/v1/{resource}/{op|session}/{name}/v{version}/#{ordinal:06}`,
//! with ordinals per `(resource, kind, name)` restarting at zero in every
//! journal, in submit order. An engine retry of the node therefore reuses
//! the occurrences of its earlier attempts: a settled slot replays its
//! recorded outcome without a provider call, an opaque ambiguous one is
//! unknown, and a retryable failure — never recorded as a rejection — may
//! be granted again within the slot's budget. The `it{n}/` prefix is
//! reserved for the iterations of a stateful action, which a later stage
//! journals; stateful, control and agent actions keep read-only handles
//! until then.
//!
//! **Provider key.** Every slot records the idempotency key the provider
//! receives ([`provider_idempotency_key`]); a unit always presents the key
//! read back from the prepared record, never a recomputation.
//!
//! **Crash residue.** A call granted and never explained (the process
//! died, or the unit outlived its node) is an outstanding invocation. The
//! journal never writes from `Drop`: the next prepare of the slot records
//! it as an ambiguous crossing first — an opaque effect's outcome becomes
//! unknown, a stable-key effect may be granted again within its window.
//!
//! **Grant budget.** A grant carries what is left of the ledger's window
//! for the call ([`LedgerSlot::call_timing`], as for a remote effect): the
//! resource runtime stops the unit there, so no call starts after a stable
//! key's deduplication window. A grant with nothing left is withheld: the
//! outcome becomes unknown when an earlier call of the slot may have
//! crossed, and the call is recorded not crossed otherwise.
//!
//! **Destination.** A slot's contract identity binds the row (resource key,
//! credential slot identity and configuration fingerprint) and how its
//! success is recorded: a reload that points the row elsewhere, or a
//! changed `RECORD_OUTPUT`, makes a recorded occurrence a mismatch, and
//! nothing is sent.
//!
//! **Verdict.** The journal never lets a node finish on a result its ledger
//! contradicts: after the action returns,
//! [`conclude`](NodeEffectJournal::conclude) drains the in-flight units,
//! closes the journal, records every granted-but-unexplained call as
//! ambiguous (within the drain limit) and reports a verdict that overrides
//! the action's result. Any slot whose call may have crossed without a
//! recorded outcome fails the node as unknown.

use std::{
    collections::HashMap,
    fmt,
    num::NonZeroU32,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use base64::Engine as _;
use nebula_core::ResourceKey;
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_EFFECT_JOURNAL_PREPARES_TOTAL, NEBULA_EFFECT_JOURNAL_REFUSALS_TOTAL,
        NEBULA_EFFECT_JOURNAL_VERDICTS_TOTAL, effect_journal_prepare_phase, effect_journal_step,
        effect_journal_verdict,
    },
};
use nebula_resource::{
    SlotIdentity,
    call::{
        Effect, IdempotencyKey, OPERATION_DEADLINE_CAP,
        journal::{
            CallGrant, CallOutcome, Crossing, EffectJournal, ErrorKindCode, InFlight,
            JournalIntent, JournalRefusal, JournalSlot, RecordedOutcome, Recovery, SlotPhase,
            UnitKind,
        },
    },
};
use nebula_storage_port::dto::{
    EffectOccurrenceRecord, EffectSlotId, KnownOutcome, OperationState, ProviderIdempotencyKey,
};
use serde::{Deserialize, Serialize};

use super::*;

/// Domain of a journaled effect's contract identity digest.
const CONTRACT_DOMAIN: &[u8] = b"nebula.effect-journal.contract.v1";
/// Domain of a journaled effect's request fingerprint digest.
const REQUEST_DOMAIN: &[u8] = b"nebula.effect-journal.request.v1";
/// Domain of the provider idempotency key digest.
const IDEMPOTENCY_KEY_DOMAIN: &[u8] = b"nebula.idempotency-key.v1";

/// Longest window the ledger accepts: one year.
const MAX_LEDGER_WINDOW: Duration = Duration::from_hours(365 * 24);

/// Most provider calls one slot may be granted.
const MAX_SLOT_INVOCATIONS: u32 = 10_000;

/// Engine-private proof that a dispatch runs under a [`NodeEffectJournal`]:
/// only this module can mint it, so generic dispatch cannot run a journaled
/// action with write authority.
pub(crate) struct JournalAdmission(());

/// Everything a journal is built from: the authority of the node attempt
/// that runs the action.
pub(crate) struct JournalAuthority {
    pub ledger: Arc<dyn OperationLedger>,
    pub scope: Scope,
    pub fencing: FencingToken,
    pub execution_id: ExecutionId,
    pub node_key: NodeKey,
    pub action_key: String,
    pub action_version: semver::Version,
    pub attempt_generation: u64,
    pub clock: Arc<dyn Clock>,
    pub metrics: MetricsRegistry,
}

/// The per-node-attempt journal of a journaled action's effects: the
/// [`EffectJournal`] the engine hands to the action's resource handles.
///
/// Cheap to clone; every clone is the same journal.
#[derive(Clone)]
pub(crate) struct NodeEffectJournal {
    inner: Arc<JournalInner>,
}

struct JournalInner {
    authority: JournalAuthority,
    /// The execution id as the ledger addresses it.
    execution: String,
    state: Mutex<JournalState>,
    in_flight: AtomicUsize,
    drained: tokio::sync::Notify,
    closed: AtomicBool,
}

#[derive(Default)]
struct JournalState {
    /// The next ordinal per `(resource, kind, name)`.
    ordinals: HashMap<(ResourceKey, UnitKind, String), u32>,
    /// The slots this journal prepared. A slot is used by one unit at a
    /// time; its async lock serializes that unit's ledger steps with the
    /// journal's conclusion. The sync lock around the map is never held
    /// across an await.
    slots: HashMap<EffectSlotId, Arc<tokio::sync::Mutex<LedgerSlot>>>,
    /// The failure that decides the node's verdict (a deferring one
    /// replaces a non-deferring one).
    failure: Option<EffectExecutionError>,
}

/// A unit's slot binding as the journal derives it from its intent.
struct DerivedBinding {
    contract: PreparedEffectContract,
    fingerprint: RequestFingerprint,
    provider_key: ProviderIdempotencyKey,
}

/// The recorded outcome of a journaled effect, as its frozen evidence
/// payload. Its tags differ from a remote effect's evidence, so neither
/// decodes as the other.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum JournalEvidence {
    /// The effect applied; its output as JSON.
    #[serde(rename = "JournalOutput")]
    Output {
        operation_id: [u8; 16],
        output: Value,
    },
    /// The effect applied; its output is not kept.
    #[serde(rename = "JournalOutputUnavailable")]
    OutputUnavailable { operation_id: [u8; 16] },
    /// The provider definitively rejected the effect.
    #[serde(rename = "JournalRejected")]
    Rejected {
        operation_id: [u8; 16],
        code: String,
    },
}

impl NodeEffectJournal {
    /// The journal of one node attempt, writing under `authority`. Builds
    /// nothing durable.
    pub(crate) fn new(authority: JournalAuthority) -> Self {
        let execution = authority.execution_id.to_string();
        Self {
            inner: Arc::new(JournalInner {
                authority,
                execution,
                state: Mutex::new(JournalState::default()),
                in_flight: AtomicUsize::new(0),
                drained: tokio::sync::Notify::new(),
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// The admission witness for dispatching the journal's action.
    pub(crate) fn admission(&self) -> JournalAdmission {
        JournalAdmission(())
    }

    fn access(&self) -> LedgerAccess<'_> {
        let authority = &self.inner.authority;
        LedgerAccess {
            ledger: authority.ledger.as_ref(),
            scope: &authority.scope,
            fencing: authority.fencing,
            clock: authority.clock.as_ref(),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, JournalState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a failure of the node's verdict: the first one wins, except
    /// that a deferring failure (lease, acknowledgement, availability)
    /// replaces a non-deferring one.
    fn note_failure(&self, error: EffectExecutionError) {
        let mut state = self.state();
        let replace = match state.failure {
            None => true,
            Some(current) => !current.is_deferred() && error.is_deferred(),
        };
        if replace {
            state.failure = Some(error);
        }
    }

    /// Counts a refusal of `step` and returns it.
    fn refused(&self, step: &'static str, refusal: JournalRefusal) -> JournalRefusal {
        let metrics = &self.inner.authority.metrics;
        let labels = metrics
            .interner()
            .label_set(&[("step", step), ("refusal", refusal.as_str())]);
        if let Ok(counter) = metrics.counter_labeled(NEBULA_EFFECT_JOURNAL_REFUSALS_TOTAL, &labels)
        {
            counter.inc();
        }
        refusal
    }

    /// The refusal a unit sees for a durable `step` that failed with
    /// `error`, recording the failure in the node's verdict.
    fn refuse(&self, step: &'static str, error: EffectExecutionError) -> JournalRefusal {
        let (refusal, verdict) = classify_failure(error);
        tracing::warn!(
            execution_id = %self.inner.authority.execution_id,
            node_key = %self.inner.authority.node_key,
            step,
            refusal = refusal.as_str(),
            code = verdict.code(),
            "effect journal refused a unit step"
        );
        self.note_failure(verdict);
        self.refused(step, refusal)
    }

    /// The slot this journal prepared for `unit`, for `step`.
    fn entry(
        &self,
        step: &'static str,
        unit: &JournalSlot,
    ) -> Result<Arc<tokio::sync::Mutex<LedgerSlot>>, JournalRefusal> {
        if self.is_closed() {
            return Err(self.refused(step, JournalRefusal::Closed));
        }
        let id = EffectSlotId::from_storage_bytes(*unit.id());
        let entry = self.state().slots.get(&id).cloned();
        entry.ok_or_else(|| self.refuse(step, EffectExecutionError::InvalidEvidence))
    }

    /// Derives the slot binding of `intent`: its contract identity, request
    /// fingerprint and provider idempotency key.
    ///
    /// The contract identity binds the action, the destination (resource
    /// key, credential slot identity, configuration fingerprint), the unit
    /// (kind, operation, version, effect class, recorded output) and the
    /// slot policy: any of them changing under a recorded occurrence is an
    /// occurrence mismatch, and nothing is sent. The provider key binds
    /// only what a provider deduplicates on.
    fn derive(&self, intent: &JournalIntent<'_>) -> Result<DerivedBinding, EffectExecutionError> {
        let authority = &self.inner.authority;
        let policy = slot_policy(intent.effect, intent.recovery, intent.max_invocations)?;
        let mut identity = Sha256::new();
        frame(&mut identity, CONTRACT_DOMAIN)?;
        frame(&mut identity, authority.action_key.as_bytes())?;
        frame(
            &mut identity,
            authority.action_version.to_string().as_bytes(),
        )?;
        // The destination: the row, its credentials and its configuration.
        frame(&mut identity, intent.resource_key.as_str().as_bytes())?;
        frame(&mut identity, &slot_identity_bytes(intent.binding)?)?;
        identity.update(intent.config_fingerprint.to_be_bytes());
        frame(&mut identity, intent.kind.as_str().as_bytes())?;
        frame(&mut identity, intent.operation.as_bytes())?;
        identity.update(intent.version.to_be_bytes());
        identity.update([effect_class(intent.effect)?]);
        // How a success is recorded decides what a replay yields.
        identity.update([u8::from(intent.record_output)]);
        identity.update([capability_discriminant(policy.capability())?]);
        identity.update(policy.max_invocations().to_be_bytes());
        identity.update(policy.max_queries().to_be_bytes());
        identity.update(policy.recovery_window_ms().to_be_bytes());
        match policy.stable_window_ms() {
            Some(window) => {
                identity.update([1]);
                identity.update(window.to_be_bytes());
            },
            None => identity.update([0]),
        }
        let identity: [u8; 32] = identity.finalize().into();
        let contract = PreparedEffectContract::new(RequestFingerprint::new(1, identity), policy)
            .map_err(|_| EffectExecutionError::InvalidContract)?;
        let mut request = Sha256::new();
        frame(&mut request, REQUEST_DOMAIN)?;
        request.update(identity);
        frame(&mut request, intent.canonical_request)?;
        let fingerprint = RequestFingerprint::new(1, request.finalize().into());
        let provider_key = provider_idempotency_key(&ProviderKeyParts {
            org_id: &authority.scope.org_id,
            workspace_id: &authority.scope.workspace_id,
            resource_key: intent.resource_key.as_str(),
            operation: intent.operation,
            version: intent.version,
            developer: intent.key_part,
            execution_id: &self.inner.execution,
            node_key: authority.node_key.as_str(),
            occurrence: intent.occurrence,
        })?;
        Ok(DerivedBinding {
            contract,
            fingerprint,
            provider_key,
        })
    }

    /// Waits for every in-flight unit to finish, at most `limit`; `true`
    /// when none is left.
    async fn drain(&self, limit: Duration) -> bool {
        let wait = async {
            loop {
                let notified = self.inner.drained.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.inner.in_flight.load(Ordering::SeqCst) == 0 {
                    return;
                }
                notified.await;
            }
        };
        tokio::time::timeout(limit, wait).await.is_ok()
    }

    /// Closes the journal: every later unit step is refused
    /// [`Closed`](JournalRefusal::Closed), including steps of units it
    /// already prepared.
    pub(crate) fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
    }

    /// Ends the journal's node attempt: drains the in-flight units for at
    /// most `drain_limit`, closes the journal, records leaked calls within
    /// what is left of `drain_limit` and returns its verdict, which
    /// overrides the action's result.
    ///
    /// Reads the node's occurrences once, always: no reliable signal tells
    /// a first dispatch from one that follows a crash before the earlier
    /// attempt was recorded.
    ///
    /// # Errors
    ///
    /// - a deferring failure (lease lost, acknowledgement unknown, ledger
    ///   unavailable) — the turn must release its lease without finalizing;
    /// - [`JournalOutcomeUnknown`](EffectExecutionError::JournalOutcomeUnknown)
    ///   when any slot of the node has an unknown outcome — recorded
    ///   unknown, an unexplained call, an unresolved call that may have
    ///   crossed (an ambiguous stable-key call), or a slot a stuck unit kept
    ///   locked past the drain limit — even if the action swallowed the
    ///   unit's error;
    /// - any other failure the journal met (an occurrence mismatch, an
    ///   invalid contract or record).
    pub(crate) async fn conclude(&self, drain_limit: Duration) -> Result<(), EffectExecutionError> {
        let verdict = self.verdict(drain_limit).await;
        let label = verdict_label(verdict.as_ref().err());
        let metrics = &self.inner.authority.metrics;
        let labels = metrics.interner().single("code", label);
        if let Ok(counter) = metrics.counter_labeled(NEBULA_EFFECT_JOURNAL_VERDICTS_TOTAL, &labels)
        {
            counter.inc();
        }
        verdict
    }

    async fn verdict(&self, drain_limit: Duration) -> Result<(), EffectExecutionError> {
        let authority = &self.inner.authority;
        let drain_started = tokio::time::Instant::now();
        if !self.drain(drain_limit).await {
            tracing::warn!(
                execution_id = %authority.execution_id,
                node_key = %authority.node_key,
                in_flight = self.inner.in_flight.load(Ordering::SeqCst),
                "journaled units outlived their action; closing the effect journal"
            );
        }
        self.close();
        // The cleanup shares the drain's limit: a unit stuck in a ledger
        // call keeps its slot locked, and the node must still conclude.
        let cleanup_deadline = drain_started
            .checked_add(drain_limit)
            .unwrap_or_else(tokio::time::Instant::now);
        let uninspected = self.record_leaked_calls(cleanup_deadline).await;
        let failure = self.state().failure;
        if let Some(failure) = failure
            && failure.is_deferred()
        {
            return Err(failure);
        }
        // Always read, even when this attempt prepared nothing: a process
        // that died during an earlier dispatch of the node — before the
        // attempt was recorded, so this attempt's generation may still be
        // 1 — can have left a granted call that only the ledger knows of.
        let slots = authority
            .ledger
            .read_occurrences(
                &authority.scope,
                &self.inner.execution,
                authority.node_key.as_str(),
            )
            .await?;
        let mut unresolved: Vec<EffectSlotId> = slots
            .iter()
            .map(EffectOccurrenceRecord::record)
            .filter(|record| is_unresolved(record))
            .map(|record| record.operation().slot_id())
            .collect();
        // A slot whose unit still holds it could not be inspected: it may
        // be mid-call, so it is never counted as resolved.
        for slot_id in uninspected {
            if !unresolved.contains(&slot_id) {
                unresolved.push(slot_id);
            }
        }
        if let Some(first) = unresolved.first() {
            let listed: Vec<String> = unresolved.iter().map(ToString::to_string).collect();
            tracing::error!(
                execution_id = %authority.execution_id,
                node_key = %authority.node_key,
                slots = ?listed,
                "journaled effect outcome unknown; failing the node"
            );
            return Err(EffectExecutionError::JournalOutcomeUnknown {
                slot_id: *first,
                unresolved: u32::try_from(unresolved.len()).unwrap_or(u32::MAX),
            });
        }
        failure.map_or(Ok(()), Err)
    }

    /// Records every call a unit was granted and never explained as an
    /// ambiguous crossing: a unit that outlived its action (or whose outcome
    /// could not be recorded) may have reached the provider.
    ///
    /// Bounded by `deadline`: a slot whose unit still holds it past the
    /// deadline (a ledger request that never returns), or whose recording
    /// does not finish by then, is returned uninspected — the verdict counts
    /// it as unresolved.
    async fn record_leaked_calls(&self, deadline: tokio::time::Instant) -> Vec<EffectSlotId> {
        if self
            .state()
            .failure
            .is_some_and(EffectExecutionError::is_deferred)
        {
            return Vec::new();
        }
        let entries: Vec<_> = self
            .state()
            .slots
            .iter()
            .map(|(slot_id, entry)| (*slot_id, Arc::clone(entry)))
            .collect();
        let mut uninspected = Vec::new();
        for (slot_id, entry) in entries {
            // `timeout_at` polls the lock once even past the deadline: a free
            // slot is always inspected.
            let Ok(mut slot) = tokio::time::timeout_at(deadline, entry.lock()).await else {
                uninspected.push(slot_id);
                continue;
            };
            let outstanding = slot.protocol().ok().and_then(|protocol| {
                (protocol.phase() == EffectPhase::InvocationOutstanding)
                    .then(|| protocol.invocation())
                    .flatten()
            });
            let Some(call) = outstanding else {
                continue;
            };
            let recorded = tokio::time::timeout_at(
                deadline,
                slot.advance(
                    self.access(),
                    &OperationCommand::RecordDisposition {
                        invocation: call,
                        disposition: InvocationDisposition::Ambiguous,
                    },
                ),
            )
            .await;
            match recorded {
                Ok(Ok(_)) => {},
                Ok(Err(error)) => {
                    let _ = self.refuse(effect_journal_step::RECORD_LEAKED_CALL, error);
                    uninspected.push(slot_id);
                    return uninspected;
                },
                Err(_elapsed) => uninspected.push(slot_id),
            }
        }
        uninspected
    }

    /// Resolves the phase a unit sees for a prepared `slot`, recording a
    /// crash residue (an outstanding call) as an ambiguous crossing first.
    async fn resolve_phase(
        &self,
        slot: &mut LedgerSlot,
    ) -> Result<SlotPhase, EffectExecutionError> {
        if slot.protocol()?.phase() == EffectPhase::InvocationOutstanding {
            // The call may have reached the provider. An opaque effect's
            // outcome becomes unknown; a stable-key effect may be granted
            // again within its window.
            let call = slot
                .protocol()?
                .invocation()
                .ok_or(EffectExecutionError::InvalidEvidence)?;
            slot.advance(
                self.access(),
                &OperationCommand::RecordDisposition {
                    invocation: call,
                    disposition: InvocationDisposition::Ambiguous,
                },
            )
            .await?;
        }
        slot_phase(slot)
    }

    /// Withholds a granted `call` whose window ended before the grant
    /// reached the journal. The call is never handed out. When an earlier
    /// call of the slot may have crossed, a resend could reach a provider
    /// that no longer deduplicates it: the outcome becomes unknown.
    /// Otherwise nothing ever crossed: the call is recorded not crossed and
    /// refused as unavailable (the ledger answered too slowly).
    async fn expired_grant(&self, slot: &mut LedgerSlot, call: OperationCallId) -> JournalRefusal {
        const STEP: &str = effect_journal_step::GRANT;
        // The withheld call is counted among the crossed ones until it is
        // explained.
        let crossed_before = slot.protocol().map_or(1, |protocol| {
            protocol.crossed_invocations().saturating_sub(1)
        });
        tracing::warn!(
            execution_id = %self.inner.authority.execution_id,
            node_key = %self.inner.authority.node_key,
            crossed_before,
            "journaled effect granted with no window left; call withheld"
        );
        if crossed_before > 0 {
            return match slot.mark_unknown(self.access()).await {
                Ok(()) => self.refused(STEP, JournalRefusal::Unknown),
                Err(error) => self.refuse(STEP, error),
            };
        }
        let recorded = slot
            .advance(
                self.access(),
                &OperationCommand::RecordDisposition {
                    invocation: call,
                    disposition: InvocationDisposition::BeforeBoundary,
                },
            )
            .await;
        match recorded {
            Ok(_) => self.refused(STEP, JournalRefusal::Unavailable),
            Err(error) => self.refuse(STEP, error),
        }
    }

    fn count_prepared(&self, phase: &SlotPhase) {
        let label = match phase {
            SlotPhase::Runnable => effect_journal_prepare_phase::RUNNABLE,
            SlotPhase::Replay(_) => effect_journal_prepare_phase::REPLAY,
            _ => effect_journal_prepare_phase::UNKNOWN,
        };
        let metrics = &self.inner.authority.metrics;
        let labels = metrics.interner().single("phase", label);
        if let Ok(counter) = metrics.counter_labeled(NEBULA_EFFECT_JOURNAL_PREPARES_TOTAL, &labels)
        {
            counter.inc();
        }
    }
}

impl fmt::Debug for NodeEffectJournal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeEffectJournal")
            .field("execution_id", &self.inner.authority.execution_id)
            .field("node_key", &self.inner.authority.node_key)
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl EffectJournal for NodeEffectJournal {
    fn next_ordinal(&self, key: &ResourceKey, kind: UnitKind, name: &str) -> u32 {
        let mut state = self.state();
        let next = state
            .ordinals
            .entry((key.clone(), kind, name.to_owned()))
            .or_insert(0);
        let ordinal = *next;
        *next = next.saturating_add(1);
        ordinal
    }

    async fn prepare(&self, intent: &JournalIntent<'_>) -> Result<JournalSlot, JournalRefusal> {
        const STEP: &str = effect_journal_step::PREPARE;
        if self.is_closed() {
            return Err(self.refused(STEP, JournalRefusal::Closed));
        }
        let authority = &self.inner.authority;
        let derived = self
            .derive(intent)
            .map_err(|error| self.refuse(STEP, error))?;
        let binding = EffectSlotBinding {
            scope: &authority.scope,
            execution_id: &self.inner.execution,
            node_key: authority.node_key.as_str(),
            occurrence: intent.occurrence,
            attempt_generation: AttemptGeneration::new(authority.attempt_generation),
            fingerprint: derived.fingerprint,
            destination: derived.contract.policy().capability(),
            contract: &derived.contract,
            provider_key: Some(derived.provider_key),
        };
        let slot = LedgerSlot::prepare(self.access(), &binding)
            .await
            .map_err(|error| self.refuse(STEP, error))?;
        // The key the provider receives is the durable one, read back from
        // the ledger — never recomputed for an existing slot.
        let idempotency_key = slot
            .provider_key()
            .and_then(|key| IdempotencyKey::new(key.as_str()).ok())
            .ok_or_else(|| self.refuse(STEP, EffectExecutionError::InvalidEvidence))?;
        let slot_id = slot.slot_id();
        let entry = Arc::new(tokio::sync::Mutex::new(slot));
        {
            let mut state = self.state();
            if state.slots.contains_key(&slot_id) {
                drop(state);
                // One occurrence submitted twice in one node attempt: two
                // units would share one slot's calls.
                return Err(self.refuse(STEP, EffectExecutionError::OccurrenceMismatch));
            }
            state.slots.insert(slot_id, Arc::clone(&entry));
        }
        let mut slot = entry.lock().await;
        let phase = self
            .resolve_phase(&mut slot)
            .await
            .map_err(|error| self.refuse(STEP, error))?;
        let revision = slot
            .protocol()
            .map_err(|error| self.refuse(STEP, error))?
            .revision();
        self.count_prepared(&phase);
        tracing::debug!(
            execution_id = %authority.execution_id,
            node_key = %authority.node_key,
            slot_id = %slot_id,
            occurrence = intent.occurrence,
            phase = ?phase,
            "journaled effect prepared"
        );
        Ok(JournalSlot::new(
            *slot_id.as_bytes(),
            idempotency_key,
            revision,
            phase,
        ))
    }

    async fn grant(&self, unit: &JournalSlot) -> Result<CallGrant, JournalRefusal> {
        const STEP: &str = effect_journal_step::GRANT;
        let authority = &self.inner.authority;
        let entry = self.entry(STEP, unit)?;
        let mut slot = entry.lock().await;
        if self.is_closed() {
            return Err(self.refused(STEP, JournalRefusal::Closed));
        }
        let protocol = slot.protocol().map_err(|error| self.refuse(STEP, error))?;
        if protocol.phase() == EffectPhase::OutcomeUnknown {
            return Err(self.refused(STEP, JournalRefusal::Unknown));
        }
        let revision = protocol.revision();
        match slot
            .advance(
                self.access(),
                &OperationCommand::GrantInvocation {
                    expected_revision: revision,
                },
            )
            .await
        {
            Ok(Some(GrantedCall::Invocation {
                call,
                authorized_at_ms,
                request_started,
            })) => {
                if self.is_closed() {
                    // The node concluded while the grant was in flight: the
                    // call is never handed out, so it provably did not cross.
                    let _ = slot
                        .advance(
                            self.access(),
                            &OperationCommand::RecordDisposition {
                                invocation: call,
                                disposition: InvocationDisposition::BeforeBoundary,
                            },
                        )
                        .await;
                    return Err(self.refused(STEP, JournalRefusal::Closed));
                }
                // The ledger vouches for the call only until its window ends
                // (a stable key's deduplication, an opaque effect's recovery
                // window): the unit must finish the call within what is left.
                let budget = match slot.call_timing(
                    authority.clock.as_ref(),
                    CallPurpose::Invocation,
                    authorized_at_ms,
                    request_started,
                ) {
                    Ok((_, budget)) => budget,
                    Err(error) => return Err(self.refuse(STEP, error)),
                };
                if budget.is_zero() {
                    return Err(self.expired_grant(&mut slot, call).await);
                }
                Ok(CallGrant::from_bytes(*call.as_bytes()).with_budget(budget))
            },
            // The ledger refused a fresh call: the budget or the window ran
            // out and the outcome is now unknown.
            Ok(None)
                if slot
                    .protocol()
                    .is_ok_and(|protocol| protocol.phase() == EffectPhase::OutcomeUnknown) =>
            {
                Err(self.refused(STEP, JournalRefusal::Unknown))
            },
            Ok(_) => Err(self.refuse(STEP, EffectExecutionError::InvalidEvidence)),
            Err(error) => Err(self.refuse(STEP, error)),
        }
    }

    async fn explain(
        &self,
        unit: &JournalSlot,
        call: CallGrant,
        crossing: Crossing,
    ) -> Result<(), JournalRefusal> {
        const STEP: &str = effect_journal_step::EXPLAIN;
        let entry = self.entry(STEP, unit)?;
        let mut slot = entry.lock().await;
        if self.is_closed() {
            return Err(self.refused(STEP, JournalRefusal::Closed));
        }
        let disposition = match crossing {
            Crossing::NotCrossed => InvocationDisposition::BeforeBoundary,
            // An unrecognized crossing may have reached the provider.
            _ => InvocationDisposition::Ambiguous,
        };
        slot.advance(
            self.access(),
            &OperationCommand::RecordDisposition {
                invocation: OperationCallId::from_bytes(*call.as_bytes()),
                disposition,
            },
        )
        .await
        .map(|_| ())
        .map_err(|error| self.refuse(STEP, error))
    }

    async fn settle(
        &self,
        unit: &JournalSlot,
        call: CallGrant,
        outcome: CallOutcome<'_>,
    ) -> Result<(), JournalRefusal> {
        const STEP: &str = effect_journal_step::SETTLE;
        let entry = self.entry(STEP, unit)?;
        let mut slot = entry.lock().await;
        if self.is_closed() {
            return Err(self.refused(STEP, JournalRefusal::Closed));
        }
        let evidence = journal_evidence(
            slot.operation_id(),
            OperationCallId::from_bytes(*call.as_bytes()),
            outcome,
        )
        .map_err(|error| self.refuse(STEP, error))?;
        slot.commit_evidence(self.access(), &evidence)
            .await
            .map_err(|error| self.refuse(STEP, error))
    }

    fn track(&self) -> InFlight {
        self.inner.in_flight.fetch_add(1, Ordering::SeqCst);
        let inner = Arc::clone(&self.inner);
        InFlight::new(move || {
            if inner.in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
                inner.drained.notify_waiters();
            }
        })
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }
}

/// The refusal a unit sees for a durable step that failed with `error`,
/// and the failure it records in the node's verdict.
fn classify_failure(error: EffectExecutionError) -> (JournalRefusal, EffectExecutionError) {
    match error {
        EffectExecutionError::OccurrenceMismatch
        | EffectExecutionError::Ledger(OperationLedgerError::OperationMismatch { .. }) => (
            JournalRefusal::Mismatch,
            EffectExecutionError::OccurrenceMismatch,
        ),
        EffectExecutionError::Ledger(OperationLedgerError::InvalidOccurrence { .. })
        | EffectExecutionError::InvalidContract => (
            JournalRefusal::Mismatch,
            EffectExecutionError::InvalidContract,
        ),
        EffectExecutionError::Ledger(OperationLedgerError::AcknowledgementUnknown) => {
            (JournalRefusal::AcknowledgementUnknown, error)
        },
        EffectExecutionError::Ledger(OperationLedgerError::ExecutionLeaseRejected) => {
            (JournalRefusal::LeaseLost, error)
        },
        error => (JournalRefusal::Unavailable, error),
    }
}

/// The `code` label of a journal verdict.
fn verdict_label(failure: Option<&EffectExecutionError>) -> &'static str {
    match failure {
        None => effect_journal_verdict::OK,
        Some(error) if error.is_deferred() => effect_journal_verdict::DEFERRED,
        Some(EffectExecutionError::JournalOutcomeUnknown { .. }) => {
            effect_journal_verdict::OUTCOME_UNKNOWN
        },
        Some(EffectExecutionError::OccurrenceMismatch) => {
            effect_journal_verdict::OCCURRENCE_MISMATCH
        },
        Some(EffectExecutionError::InvalidContract) => effect_journal_verdict::INVALID_CONTRACT,
        Some(EffectExecutionError::InvalidEvidence) => effect_journal_verdict::INVALID_EVIDENCE,
        Some(_) => effect_journal_verdict::LEDGER,
    }
}

/// Whether a slot of the node leaves its effect's outcome unknown: a slot
/// whose outcome is not recorded although a call may have reached the
/// provider — recorded unknown, outstanding (a crashed or leaked call), or
/// any later phase after a call that may have crossed, such as an
/// ambiguous stable-key call the unit did not get to resend (or whose
/// error the action swallowed).
fn is_unresolved(record: &OperationRecord) -> bool {
    record.protocol().map_or_else(
        || record.state() == OperationState::OutcomeUnknown,
        |protocol| match protocol.phase() {
            EffectPhase::Resolved => false,
            EffectPhase::OutcomeUnknown | EffectPhase::InvocationOutstanding => true,
            _ => protocol.crossed_invocations() > 0,
        },
    )
}

/// The phase a unit sees for a prepared `slot` with no outstanding call.
fn slot_phase(slot: &LedgerSlot) -> Result<SlotPhase, EffectExecutionError> {
    let protocol = slot.protocol()?;
    match protocol.phase() {
        EffectPhase::Prepared | EffectPhase::BeforeBoundary | EffectPhase::Ambiguous => {
            Ok(SlotPhase::Runnable)
        },
        EffectPhase::OutcomeUnknown => Ok(SlotPhase::Unknown),
        EffectPhase::Resolved => {
            let evidence = protocol
                .evidence()
                .ok_or(EffectExecutionError::InvalidEvidence)?;
            replay(slot.operation_id(), evidence).map(SlotPhase::Replay)
        },
        _ => Err(EffectExecutionError::InvalidEvidence),
    }
}

/// The durable policy of a journaled effect's slot.
///
/// - `Idempotent` + stable key: at most `max_invocations` calls (clamped to
///   the ledger's `1..=10_000`), the author's key window (at most a year),
///   a recovery window covering it and at least [`OPERATION_DEADLINE_CAP`];
/// - `Write` + opaque: the recovery window is [`OPERATION_DEADLINE_CAP`].
///
/// No queries: journaled effects have no reconciliation query yet.
fn slot_policy(
    effect: Effect,
    recovery: Recovery,
    max_invocations: NonZeroU32,
) -> Result<PreparedEffectPolicy, EffectExecutionError> {
    let max_invocations = max_invocations.get().min(MAX_SLOT_INVOCATIONS);
    let builder = match (effect, recovery) {
        (Effect::Idempotent, Recovery::StableKey { window }) => {
            let window = window.min(MAX_LEDGER_WINDOW);
            PreparedEffectPolicy::builder(DestinationCapability::StableKey)
                .stable_key_window(window)
                .recovery_window(window.max(OPERATION_DEADLINE_CAP).min(MAX_LEDGER_WINDOW))
        },
        (Effect::Write, Recovery::Opaque) => {
            PreparedEffectPolicy::builder(DestinationCapability::Opaque)
                .recovery_window(OPERATION_DEADLINE_CAP)
        },
        _ => return Err(EffectExecutionError::InvalidContract),
    };
    builder
        .maximum_invocations(max_invocations)
        .maximum_queries(0)
        .build()
        .map_err(|_| EffectExecutionError::InvalidContract)
}

/// The byte of a unit's declared effect class in its contract identity.
fn effect_class(effect: Effect) -> Result<u8, EffectExecutionError> {
    match effect {
        Effect::Idempotent => Ok(1),
        Effect::Write => Ok(2),
        _ => Err(EffectExecutionError::InvalidContract),
    }
}

/// The structural bytes of a row's credential slot identity: `0` when
/// unbound; otherwise `1`, the pair count and every framed `(slot,
/// credential)` pair in canonical order. Credential ids, not material: a
/// rotation keeps the identity, re-pointing a slot changes it.
fn slot_identity_bytes(binding: &SlotIdentity) -> Result<Vec<u8>, EffectExecutionError> {
    match binding {
        SlotIdentity::Unbound => Ok(vec![0]),
        SlotIdentity::Structural(pairs) => {
            let count =
                u64::try_from(pairs.len()).map_err(|_| EffectExecutionError::InvalidContract)?;
            let mut bytes = vec![1];
            bytes.extend_from_slice(&count.to_be_bytes());
            for (slot, credential) in pairs.iter() {
                push_frame(&mut bytes, slot.as_bytes())?;
                push_frame(&mut bytes, credential.as_bytes())?;
            }
            Ok(bytes)
        },
        _ => Err(EffectExecutionError::InvalidContract),
    }
}

/// Appends `bytes` to `buffer`, framed by their big-endian `u64` length.
fn push_frame(buffer: &mut Vec<u8>, bytes: &[u8]) -> Result<(), EffectExecutionError> {
    let length = u64::try_from(bytes.len()).map_err(|_| EffectExecutionError::InvalidContract)?;
    buffer.extend_from_slice(&length.to_be_bytes());
    buffer.extend_from_slice(bytes);
    Ok(())
}

/// What the provider idempotency key of one effect is composed of.
pub(super) struct ProviderKeyParts<'a> {
    pub org_id: &'a str,
    pub workspace_id: &'a str,
    pub resource_key: &'a str,
    pub operation: &'a str,
    pub version: u32,
    /// The author's key part, when the operation declared one.
    pub developer: Option<&'a str>,
    pub execution_id: &'a str,
    pub node_key: &'a str,
    pub occurrence: &'a str,
}

/// The provider idempotency key of one journaled effect:
///
/// ```text
/// base64url_nopad(SHA-256(
///     frame("nebula.idempotency-key.v1")
///   ‖ frame(frame(org_id) ‖ frame(workspace_id))
///   ‖ frame(resource_key)
///   ‖ frame(operation)
///   ‖ u32_be(version)
///   ‖ frame(developer part)))
/// ```
///
/// where `frame(x) = u64_be(len(x)) ‖ x` and the developer part is the
/// author's key part, or — without one — `frame(execution_id) ‖
/// frame(node_key) ‖ frame(occurrence)`. A developer key therefore
/// deduplicates across executions and carries no execution id; neither form
/// contains an attempt number, so every retry and resume presents the same
/// key. 43 characters. Separate from the resource runtime's local key
/// (domain `nebula.idempotency.local.v1`) that an unjournaled row presents.
pub(super) fn provider_idempotency_key(
    parts: &ProviderKeyParts<'_>,
) -> Result<ProviderIdempotencyKey, EffectExecutionError> {
    let mut tenant = Vec::new();
    push_frame(&mut tenant, parts.org_id.as_bytes())?;
    push_frame(&mut tenant, parts.workspace_id.as_bytes())?;
    let mut digest = Sha256::new();
    frame(&mut digest, IDEMPOTENCY_KEY_DOMAIN)?;
    frame(&mut digest, &tenant)?;
    frame(&mut digest, parts.resource_key.as_bytes())?;
    frame(&mut digest, parts.operation.as_bytes())?;
    digest.update(parts.version.to_be_bytes());
    if let Some(developer) = parts.developer {
        frame(&mut digest, developer.as_bytes())?;
    } else {
        let mut run = Vec::new();
        push_frame(&mut run, parts.execution_id.as_bytes())?;
        push_frame(&mut run, parts.node_key.as_bytes())?;
        push_frame(&mut run, parts.occurrence.as_bytes())?;
        frame(&mut digest, &run)?;
    }
    let digest: [u8; 32] = digest.finalize().into();
    ProviderIdempotencyKey::new(&base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest))
        .map_err(|_| EffectExecutionError::InvalidContract)
}

/// Every closed rejection code a journaled effect may record.
const ERROR_KIND_CODES: [ErrorKindCode; 10] = [
    ErrorKindCode::Transient,
    ErrorKindCode::Permanent,
    ErrorKindCode::Exhausted,
    ErrorKindCode::Backpressure,
    ErrorKindCode::NotFound,
    ErrorKindCode::Cancelled,
    ErrorKindCode::Revoked,
    ErrorKindCode::Ambiguous,
    ErrorKindCode::CredentialUnavailable,
    ErrorKindCode::OutcomeUnknown,
];

/// The frozen evidence of a granted call's `outcome`.
fn journal_evidence(
    operation_id: OperationId,
    call: OperationCallId,
    outcome: CallOutcome<'_>,
) -> Result<FrozenOutcomeEvidence, EffectExecutionError> {
    let operation_id = *operation_id.as_bytes();
    let unavailable = || JournalEvidence::OutputUnavailable { operation_id };
    let (known, recorded) = match outcome {
        CallOutcome::Applied(bytes) => (
            KnownOutcome::Succeeded,
            serde_json::from_slice::<Value>(bytes).map_or_else(
                |_| unavailable(),
                |output| JournalEvidence::Output {
                    operation_id,
                    output,
                },
            ),
        ),
        CallOutcome::AppliedWithoutOutput => (KnownOutcome::Succeeded, unavailable()),
        CallOutcome::Rejected(code) => (
            KnownOutcome::Failed,
            JournalEvidence::Rejected {
                operation_id,
                code: code.as_str().to_owned(),
            },
        ),
        _ => return Err(EffectExecutionError::InvalidEvidence),
    };
    let source = OutcomeEvidenceSource::Invocation(call);
    let payload =
        serde_json::to_vec(&recorded).map_err(|_| EffectExecutionError::InvalidEvidence)?;
    // A known applied effect never becomes retryable because its output is
    // too large to keep: the bounded terminal fact remains durable.
    FrozenOutcomeEvidence::v1_json(source, known, payload).or_else(|_| {
        let payload = serde_json::to_vec(&unavailable())
            .map_err(|_| EffectExecutionError::InvalidEvidence)?;
        FrozenOutcomeEvidence::v1_json(source, known, payload).map_err(Into::into)
    })
}

/// The recorded outcome a resumed unit replays from `evidence`.
fn replay(
    operation_id: OperationId,
    evidence: &FrozenOutcomeEvidence,
) -> Result<RecordedOutcome, EffectExecutionError> {
    evidence
        .validate()
        .map_err(|_| EffectExecutionError::InvalidEvidence)?;
    let recorded = serde_json::from_slice::<JournalEvidence>(evidence.payload());
    let operation_id = *operation_id.as_bytes();
    match (evidence.outcome(), recorded) {
        (
            KnownOutcome::Succeeded,
            Ok(JournalEvidence::Output {
                operation_id: recorded,
                output,
            }),
        ) if recorded == operation_id => serde_json::to_vec(&output)
            .map(RecordedOutcome::Succeeded)
            .map_err(|_| EffectExecutionError::InvalidEvidence),
        (
            KnownOutcome::Succeeded,
            Ok(JournalEvidence::OutputUnavailable {
                operation_id: recorded,
            }),
        ) if recorded == operation_id => Ok(RecordedOutcome::OutputUnavailable),
        (
            KnownOutcome::Failed,
            Ok(JournalEvidence::Rejected {
                operation_id: recorded,
                code,
            }),
        ) if recorded == operation_id => ERROR_KIND_CODES
            .into_iter()
            .find(|known| known.as_str() == code)
            .map(RecordedOutcome::Failed)
            .ok_or(EffectExecutionError::InvalidEvidence),
        // An operator's adjudication records the outcome in its own words:
        // replay only what is known, never the output.
        (KnownOutcome::Succeeded, _)
            if matches!(evidence.source(), OutcomeEvidenceSource::Adjudication(_)) =>
        {
            Ok(RecordedOutcome::OutputUnavailable)
        },
        (KnownOutcome::Failed, _)
            if matches!(evidence.source(), OutcomeEvidenceSource::Adjudication(_)) =>
        {
            Ok(RecordedOutcome::Failed(ErrorKindCode::Permanent))
        },
        _ => Err(EffectExecutionError::InvalidEvidence),
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
