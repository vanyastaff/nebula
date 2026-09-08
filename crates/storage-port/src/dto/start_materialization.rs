//! Opaque contract records and complete execution-owner start requests.

use super::{ControlMsg, NewExecution};
use crate::Scope;
use crate::store::{StartContractIdentity, StartFingerprint, StartMaterializationError};
use std::fmt;

/// Maximum recorded bundle size accepted by the persistence seam.
pub const MAX_CONTRACT_BUNDLE_BYTES: usize = 1024 * 1024;

/// Supported persisted execution-contract envelope format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContractBundleFormat {
    /// Version-one JSON execution contract envelope.
    V1Json,
}

/// Bounded opaque bundle plus explicit exact relational identities.
/// Construction is neither semantic integrity validation nor tenant authority.
#[derive(Clone, PartialEq, Eq)]
pub struct ContractBundleRecord {
    identity: StartContractIdentity,
    bytes: Vec<u8>,
}
impl ContractBundleRecord {
    /// Record bounded v1 bytes without interpreting the domain envelope.
    ///
    /// # Errors
    /// Rejects empty or oversized records.
    pub fn v1_json(
        identity: StartContractIdentity,
        bytes: Vec<u8>,
    ) -> Result<Self, StartMaterializationError> {
        if bytes.is_empty() || bytes.len() > MAX_CONTRACT_BUNDLE_BYTES {
            return Err(StartMaterializationError::InvalidEnvelope);
        }
        Ok(Self { identity, bytes })
    }
    /// Exact bundle/plan/flavor identities.
    #[must_use]
    pub const fn identity(&self) -> StartContractIdentity {
        self.identity
    }
    /// Explicit supported wire format.
    #[must_use]
    pub const fn format(&self) -> ContractBundleFormat {
        ContractBundleFormat::V1Json
    }
    /// Sensitive recorded payload, available only by explicit access.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl fmt::Debug for ContractBundleRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContractBundleRecord")
            .field("identity", &self.identity)
            .field("byte_count", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// Optional caller-key identity for one accepted start command.
#[derive(Clone, Copy)]
pub struct StartKey<'a> {
    key: &'a str,
    fingerprint: StartFingerprint,
}
impl<'a> StartKey<'a> {
    /// Pair a caller key with its versioned canonical intent fingerprint.
    #[must_use]
    pub const fn new(key: &'a str, fingerprint: StartFingerprint) -> Self {
        Self { key, fingerprint }
    }
    /// Caller-supplied key; it may contain sensitive user data.
    #[must_use]
    pub const fn key(self) -> &'a str {
        self.key
    }
    /// Versioned request fingerprint.
    #[must_use]
    pub const fn fingerprint(self) -> StartFingerprint {
        self.fingerprint
    }
}
impl fmt::Debug for StartKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StartKey").finish_non_exhaustive()
    }
}

/// Source-natural trigger identity, independent of caller idempotency keys.
#[derive(Clone, Copy)]
pub struct TriggerStartKey<'a> {
    trigger: &'a str,
    event: &'a str,
}
impl<'a> TriggerStartKey<'a> {
    /// Identify a delivery using its original trigger and event identities.
    #[must_use]
    pub const fn new(trigger_id: &'a str, event_id: &'a str) -> Self {
        Self {
            trigger: trigger_id,
            event: event_id,
        }
    }
    /// Trigger identity within the selected tenant.
    #[must_use]
    pub const fn trigger_id(self) -> &'a str {
        self.trigger
    }
    /// Original event identity; its payload does not alter deduplication.
    #[must_use]
    pub const fn event_id(self) -> &'a str {
        self.event
    }
}
impl fmt::Debug for TriggerStartKey<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TriggerStartKey")
            .finish_non_exhaustive()
    }
}

enum StartOrigin<'a> {
    Unkeyed,
    Caller(StartKey<'a>),
    Trigger(TriggerStartKey<'a>),
}

/// One complete keyed or unkeyed start to commit atomically.
pub struct MaterializedStart<'a> {
    scope: &'a Scope,
    origin: StartOrigin<'a>,
    execution_id: &'a str,
    execution: NewExecution<'a>,
    command: &'a ControlMsg,
    bundle: &'a ContractBundleRecord,
}
impl<'a> MaterializedStart<'a> {
    /// Assemble a start; storage validates its relational envelope before writes.
    #[must_use]
    pub const fn new(
        scope: &'a Scope,
        idempotency: Option<StartKey<'a>>,
        execution_id: &'a str,
        execution: NewExecution<'a>,
        command: &'a ControlMsg,
        bundle: &'a ContractBundleRecord,
    ) -> Self {
        Self {
            scope,
            origin: match idempotency {
                Some(key) => StartOrigin::Caller(key),
                None => StartOrigin::Unkeyed,
            },
            execution_id,
            execution,
            command,
            bundle,
        }
    }
    /// Assemble a trigger delivery in the existing scoped trigger-dedup namespace.
    #[must_use]
    pub const fn for_trigger(
        scope: &'a Scope,
        key: TriggerStartKey<'a>,
        execution_id: &'a str,
        execution: NewExecution<'a>,
        command: &'a ControlMsg,
        bundle: &'a ContractBundleRecord,
    ) -> Self {
        Self {
            scope,
            origin: StartOrigin::Trigger(key),
            execution_id,
            execution,
            command,
            bundle,
        }
    }
    /// Trigger origin, mutually exclusive with caller idempotency.
    #[must_use]
    pub const fn trigger(&self) -> Option<TriggerStartKey<'a>> {
        match self.origin {
            StartOrigin::Trigger(key) => Some(key),
            _ => None,
        }
    }
    /// Authenticated host's tenant selection; plain data, not proof.
    #[must_use]
    pub const fn scope(&self) -> &Scope {
        self.scope
    }
    /// Caller idempotency identity, absent for an unkeyed command.
    #[must_use]
    pub const fn idempotency(&self) -> Option<StartKey<'a>> {
        match self.origin {
            StartOrigin::Caller(key) => Some(key),
            _ => None,
        }
    }
    /// Proposed execution identity, retained across uncertain retries.
    #[must_use]
    pub const fn execution_id(&self) -> &str {
        self.execution_id
    }
    /// Initial aggregate parameters.
    #[must_use]
    pub const fn execution(&self) -> NewExecution<'a> {
        self.execution
    }
    /// Exact durable Start message.
    #[must_use]
    pub const fn command(&self) -> &ControlMsg {
        self.command
    }
    /// Immutable recorded contract.
    #[must_use]
    pub const fn bundle(&self) -> &ContractBundleRecord {
        self.bundle
    }
}
impl fmt::Debug for MaterializedStart<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaterializedStart")
            .field("execution_id", &self.execution_id)
            .field("bundle", &self.bundle)
            .finish_non_exhaustive()
    }
}

/// Original keyed acceptance identity, read only under its tenant scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartReservation {
    /// Original fingerprint, including its canonicalization version.
    fingerprint: StartFingerprint,
    /// Original durable execution identity; read its scoped current state for a receipt.
    execution_id: String,
}

impl StartReservation {
    /// Reconstruct the original keyed acceptance identity from persisted data.
    #[must_use]
    pub fn new(fingerprint: StartFingerprint, execution_id: String) -> Self {
        Self {
            fingerprint,
            execution_id,
        }
    }

    /// Original fingerprint, including its canonicalization version.
    #[must_use]
    pub const fn fingerprint(&self) -> StartFingerprint {
        self.fingerprint
    }

    /// Original durable execution identity.
    #[must_use]
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }
}

/// Immutable bundle retained for an execution, including after terminal reference release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredContractBundle {
    /// Owning tenant selection.
    scope: Scope,
    /// Owning execution identity.
    execution_id: String,
    /// Original recorded contract, with payload-redacted Debug.
    record: ContractBundleRecord,
}

impl StoredContractBundle {
    /// Reconstruct an immutable execution contract from persisted data.
    #[must_use]
    pub fn new(scope: Scope, execution_id: String, record: ContractBundleRecord) -> Self {
        Self {
            scope,
            execution_id,
            record,
        }
    }

    /// Owning tenant selection.
    #[must_use]
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Owning execution identity.
    #[must_use]
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }

    /// Original recorded contract.
    #[must_use]
    pub const fn record(&self) -> &ContractBundleRecord {
        &self.record
    }

    /// Consume the stored value and return its original recorded contract.
    #[must_use]
    pub fn into_record(self) -> ContractBundleRecord {
        self.record
    }
}
