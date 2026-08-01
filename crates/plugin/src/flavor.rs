//! Canonical registered plugin-set and immutable worker-flavor identities.
//!
//! The explicit structural encoding below is the v1 wire contract. Changing
//! field order, tags, normalization, or framing requires a new domain version.

use std::{cmp::Ordering, fmt, str::FromStr};

use nebula_core::{
    ActionKey, ArtifactSetDigest, CredentialKey, PluginKey, PluginSetId, ResourceKey,
    WorkerFlavorRevisionId,
};
use nebula_metadata::PluginDependency;
use semver::{BuildMetadata, Comparator, Op, Version, VersionReq};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};

use crate::ResolvedPlugin;

const PLUGIN_SET_DOMAIN: &[u8] = b"nebula.plugin-set.v1";
const WORKER_FLAVOR_DOMAIN: &[u8] = b"nebula.worker-flavor-revision.v1";
const WORKER_FLAVOR_RECORD_VERSION_V1: u16 = 1;
const WORKER_FLAVOR_CANONICAL_HASH_VERSION_V1: u16 = 1;

/// Version of the runtime contract expected by a worker flavor.
///
/// Build metadata is deliberately removed: it is artifact provenance, while
/// this type identifies the logical runtime contract. The trusted activation
/// boundary supplies artifact provenance separately through
/// [`ArtifactSetDigest`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeContractVersion(Version);

/// Failure to parse the worker runtime contract version.
#[derive(Debug, thiserror::Error)]
#[error("invalid runtime contract version")]
pub struct RuntimeContractVersionError {
    #[source]
    source: semver::Error,
}

impl RuntimeContractVersionError {
    /// Underlying semantic-version parser failure.
    #[must_use]
    pub const fn source_error(&self) -> &semver::Error {
        &self.source
    }
}

impl RuntimeContractVersion {
    /// Borrows the semantic version.
    #[must_use]
    pub const fn as_version(&self) -> &Version {
        &self.0
    }
}

impl FromStr for RuntimeContractVersion {
    type Err = RuntimeContractVersionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut version = value
            .parse::<Version>()
            .map_err(|source| RuntimeContractVersionError { source })?;
        version.build = BuildMetadata::EMPTY;
        Ok(Self(version))
    }
}

impl nebula_error::Classify for RuntimeContractVersionError {
    fn category(&self) -> nebula_error::ErrorCategory {
        nebula_error::ErrorCategory::Validation
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new("PLUGIN_FLAVOR:INVALID_RUNTIME_CONTRACT_VERSION")
    }
}

impl Serialize for RuntimeContractVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for RuntimeContractVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = std::borrow::Cow::<'de, str>::deserialize(deserializer)?;
        encoded.parse().map_err(de::Error::custom)
    }
}

impl fmt::Display for RuntimeContractVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Canonical logical contract entry for one registered plugin.
///
/// Every collection is sorted. Dependency requirements are normalized by a
/// structural comparator order, with exact duplicate comparators removed,
/// before they are stored or fingerprinted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginContractDescriptor {
    key: PluginKey,
    version: Version,
    action_keys: Vec<ActionKey>,
    resource_keys: Vec<ResourceKey>,
    credential_keys: Vec<CredentialKey>,
    dependencies: Vec<PluginDependency>,
}

impl PluginContractDescriptor {
    pub(crate) fn from_resolved(plugin: &ResolvedPlugin) -> Result<Self, PluginKey> {
        let mut version = plugin.version().clone();
        version.build = BuildMetadata::EMPTY;
        let mut action_keys = plugin
            .action_contracts()
            .map(|contract| contract.metadata().base.key.clone())
            .collect::<Vec<_>>();
        let mut resource_keys = plugin
            .resource_contracts()
            .map(|contract| contract.metadata().base.key.clone())
            .collect::<Vec<_>>();
        let mut credential_keys = plugin
            .credential_contracts()
            .map(|contract| contract.projected_key().clone())
            .collect::<Vec<_>>();
        action_keys.sort_unstable();
        resource_keys.sort_unstable();
        credential_keys.sort_unstable();

        let mut dependencies = plugin
            .manifest()
            .dependencies()
            .iter()
            .map(|dependency| {
                normalize_version_requirement(dependency.req())
                    .map(|requirement| PluginDependency::new(dependency.key().clone(), requirement))
                    .ok_or_else(|| dependency.key().clone())
            })
            .collect::<Result<Vec<_>, _>>()?;
        dependencies.sort_unstable_by(|left, right| {
            left.key()
                .cmp(right.key())
                .then_with(|| compare_version_requirements(left.req(), right.req()))
        });
        dependencies.dedup();

        Ok(Self {
            key: plugin.key().clone(),
            version,
            action_keys,
            resource_keys,
            credential_keys,
            dependencies,
        })
    }

    /// Plugin key in this logical contract entry.
    #[must_use]
    pub const fn key(&self) -> &PluginKey {
        &self.key
    }

    /// Canonical semver, including prerelease and excluding build metadata.
    #[must_use]
    pub const fn version(&self) -> &Version {
        &self.version
    }

    /// Sorted registered action keys.
    #[must_use]
    pub fn action_keys(&self) -> &[ActionKey] {
        &self.action_keys
    }

    /// Sorted registered resource keys.
    #[must_use]
    pub fn resource_keys(&self) -> &[ResourceKey] {
        &self.resource_keys
    }

    /// Sorted registered credential keys.
    #[must_use]
    pub fn credential_keys(&self) -> &[CredentialKey] {
        &self.credential_keys
    }

    /// Dependencies sorted by plugin key and normalized semver requirement.
    ///
    /// Exact duplicate declarations are removed.
    #[must_use]
    pub fn dependencies(&self) -> &[PluginDependency] {
        &self.dependencies
    }
}

/// Canonical identity and audit descriptor of a registered plugin set.
///
/// Component keys and declared dependency requirements are included, but
/// schemas and runtime behavior are not fingerprinted. Consequently a
/// [`PluginSetId`] identifies this registered logical surface; it is not a
/// capability-compatibility proof, an artifact authenticity proof, or
/// authorization to execute it. The ID alone does not prove that a complete
/// matching [`crate::FrozenPluginRegistry`] was loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSet {
    id: PluginSetId,
    plugins: Vec<PluginContractDescriptor>,
}

impl PluginSet {
    pub(crate) fn derive(
        mut plugins: Vec<PluginContractDescriptor>,
    ) -> Result<Self, (PluginKey, PluginKey)> {
        plugins.sort_unstable_by(|left, right| left.key.cmp(&right.key));
        let mut canonical = CanonicalSha256::new(PLUGIN_SET_DOMAIN);
        canonical.count(plugins.len());
        for plugin in &plugins {
            canonical.field(plugin.key.as_str().as_bytes());
            canonical.version(&plugin.version);
            canonical.key_category(b"actions", &plugin.action_keys);
            canonical.key_category(b"resources", &plugin.resource_keys);
            canonical.key_category(b"credentials", &plugin.credential_keys);
            canonical
                .dependency_category(&plugin.dependencies)
                .map_err(|dependency| (plugin.key.clone(), dependency))?;
        }
        Ok(Self {
            id: PluginSetId::from_bytes(canonical.finish()),
            plugins,
        })
    }

    /// Canonical registered-surface identity, not a capability proof.
    #[must_use]
    pub const fn id(&self) -> PluginSetId {
        self.id
    }

    /// Sorted logical plugin contract entries.
    #[must_use]
    pub fn plugins(&self) -> &[PluginContractDescriptor] {
        &self.plugins
    }
}

/// Immutable identity and provenance descriptor for one worker flavor revision.
///
/// The digest is reproducible only when the activation boundary supplies the
/// runtime contract version and artifact-set digest from trusted deployment
/// inputs. Derivation does not authenticate caller-provided bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerFlavorRevision {
    id: WorkerFlavorRevisionId,
    plugin_set_id: PluginSetId,
    runtime_contract_version: RuntimeContractVersion,
    artifact_set_digest: ArtifactSetDigest,
}

/// Version-one persisted projection of an immutable worker-flavor revision.
///
/// Deserialization creates only this untrusted record. Fields are private so
/// callers must use [`WorkerFlavorRevision::try_from_recorded_v1`] before the
/// value can participate in exact execution routing.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedWorkerFlavorRevisionV1 {
    record_version: u16,
    canonical_hash_version: u16,
    claimed_id: WorkerFlavorRevisionId,
    plugin_set_id: PluginSetId,
    runtime_contract_version: RuntimeContractVersion,
    artifact_set_digest: ArtifactSetDigest,
}

impl fmt::Debug for RecordedWorkerFlavorRevisionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordedWorkerFlavorRevisionV1")
            .field("record_version", &self.record_version)
            .field("canonical_hash_version", &self.canonical_hash_version)
            .field("claimed_id", &self.claimed_id)
            .field("plugin_set_id", &self.plugin_set_id)
            .finish_non_exhaustive()
    }
}

/// Integrity failures while loading a recorded worker-flavor revision.
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum WorkerFlavorIntegrityError {
    /// The envelope version is not the version-one format.
    #[classify(
        category = "validation",
        code = "PLUGIN_FLAVOR_INTEGRITY:UNSUPPORTED_RECORD_VERSION"
    )]
    #[error("unsupported worker-flavor record version {found}")]
    UnsupportedRecordVersion {
        /// Unsupported version read from the persisted envelope.
        found: u16,
    },

    /// The identity derivation version is not supported by this runtime.
    #[classify(
        category = "validation",
        code = "PLUGIN_FLAVOR_INTEGRITY:UNSUPPORTED_CANONICAL_HASH_VERSION"
    )]
    #[error("unsupported worker-flavor canonical hash version {found}")]
    UnsupportedCanonicalHashVersion {
        /// Unsupported canonical hash version read from the persisted envelope.
        found: u16,
    },

    /// The claimed revision identity differs from canonical record content.
    #[classify(
        category = "validation",
        code = "PLUGIN_FLAVOR_INTEGRITY:REVISION_ID_MISMATCH"
    )]
    #[error("worker-flavor revision identity does not match its canonical content")]
    RevisionIdMismatch {
        /// Identity claimed by the persisted envelope.
        claimed: WorkerFlavorRevisionId,
        /// Identity recomputed from the canonical flavor inputs.
        computed: WorkerFlavorRevisionId,
    },
}

impl WorkerFlavorRevision {
    pub(crate) fn derive(
        plugin_set_id: PluginSetId,
        runtime_contract_version: RuntimeContractVersion,
        artifact_set_digest: ArtifactSetDigest,
    ) -> Self {
        let mut canonical = CanonicalSha256::new(WORKER_FLAVOR_DOMAIN);
        canonical.field(plugin_set_id.as_bytes());
        canonical.version(runtime_contract_version.as_version());
        canonical.field(artifact_set_digest.as_bytes());
        Self {
            id: WorkerFlavorRevisionId::from_bytes(canonical.finish()),
            plugin_set_id,
            runtime_contract_version,
            artifact_set_digest,
        }
    }

    /// Validate a recorded v1 envelope and reconstruct the immutable revision.
    ///
    /// This check proves the existing flavor fingerprint only. It does not
    /// authenticate the artifact set, prove that a compatible registry is
    /// loaded, retain the revision, or authorize execution.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerFlavorIntegrityError`] when the record or canonical
    /// hash version is unsupported, or when the claimed identity differs from
    /// the identity derived from the recorded flavor inputs.
    pub fn try_from_recorded_v1(
        record: RecordedWorkerFlavorRevisionV1,
    ) -> Result<Self, WorkerFlavorIntegrityError> {
        if record.record_version != WORKER_FLAVOR_RECORD_VERSION_V1 {
            return Err(WorkerFlavorIntegrityError::UnsupportedRecordVersion {
                found: record.record_version,
            });
        }
        if record.canonical_hash_version != WORKER_FLAVOR_CANONICAL_HASH_VERSION_V1 {
            return Err(
                WorkerFlavorIntegrityError::UnsupportedCanonicalHashVersion {
                    found: record.canonical_hash_version,
                },
            );
        }

        let computed = Self::derive(
            record.plugin_set_id,
            record.runtime_contract_version,
            record.artifact_set_digest,
        );
        if record.claimed_id != computed.id {
            return Err(WorkerFlavorIntegrityError::RevisionIdMismatch {
                claimed: record.claimed_id,
                computed: computed.id,
            });
        }
        Ok(computed)
    }

    /// Derived worker-flavor revision identity.
    #[must_use]
    pub const fn id(&self) -> WorkerFlavorRevisionId {
        self.id
    }

    /// Logical registered plugin-set identity.
    #[must_use]
    pub const fn plugin_set_id(&self) -> PluginSetId {
        self.plugin_set_id
    }

    /// Runtime contract version pinned by this revision.
    #[must_use]
    pub const fn runtime_contract_version(&self) -> &RuntimeContractVersion {
        &self.runtime_contract_version
    }

    /// Trusted activation input describing artifact-set provenance.
    #[must_use]
    pub const fn artifact_set_digest(&self) -> ArtifactSetDigest {
        self.artifact_set_digest
    }
}

impl TryFrom<RecordedWorkerFlavorRevisionV1> for WorkerFlavorRevision {
    type Error = WorkerFlavorIntegrityError;

    fn try_from(record: RecordedWorkerFlavorRevisionV1) -> Result<Self, Self::Error> {
        Self::try_from_recorded_v1(record)
    }
}

impl From<&WorkerFlavorRevision> for RecordedWorkerFlavorRevisionV1 {
    fn from(revision: &WorkerFlavorRevision) -> Self {
        Self {
            record_version: WORKER_FLAVOR_RECORD_VERSION_V1,
            canonical_hash_version: WORKER_FLAVOR_CANONICAL_HASH_VERSION_V1,
            claimed_id: revision.id,
            plugin_set_id: revision.plugin_set_id,
            runtime_contract_version: revision.runtime_contract_version.clone(),
            artifact_set_digest: revision.artifact_set_digest,
        }
    }
}

fn normalize_version_requirement(requirement: &VersionReq) -> Option<VersionReq> {
    let mut comparators = requirement.comparators.clone();
    if comparators
        .iter()
        .any(|comparator| comparator_op_tag(comparator.op).is_none())
    {
        return None;
    }
    comparators.sort_unstable_by(compare_comparators);
    comparators.dedup();
    Some(VersionReq { comparators })
}

pub(crate) fn compare_version_requirements(left: &VersionReq, right: &VersionReq) -> Ordering {
    left.comparators
        .iter()
        .zip(&right.comparators)
        .map(|(left, right)| compare_comparators(left, right))
        .find(|ordering| !ordering.is_eq())
        .unwrap_or_else(|| left.comparators.len().cmp(&right.comparators.len()))
}

fn compare_comparators(left: &Comparator, right: &Comparator) -> Ordering {
    comparator_op_tag(left.op)
        .cmp(&comparator_op_tag(right.op))
        .then_with(|| left.major.cmp(&right.major))
        .then_with(|| left.minor.cmp(&right.minor))
        .then_with(|| left.patch.cmp(&right.patch))
        .then_with(|| left.pre.as_str().cmp(right.pre.as_str()))
}

fn comparator_op_tag(operator: Op) -> Option<u8> {
    // These assignments are persisted v1 wire tags. Never reorder or reuse
    // them; a new semver operator is rejected until a new protocol decision
    // assigns it a tag.
    match operator {
        Op::Exact => Some(0),
        Op::Greater => Some(1),
        Op::GreaterEq => Some(2),
        Op::Less => Some(3),
        Op::LessEq => Some(4),
        Op::Tilde => Some(5),
        Op::Caret => Some(6),
        Op::Wildcard => Some(7),
        _ => None,
    }
}

struct CanonicalSha256(Sha256);

impl CanonicalSha256 {
    fn new(domain: &[u8]) -> Self {
        let mut value = Self(Sha256::new());
        value.field(domain);
        value
    }

    fn count(&mut self, count: usize) {
        self.0.update((count as u64).to_be_bytes());
    }

    fn number(&mut self, value: u64) {
        self.0.update(value.to_be_bytes());
    }

    fn optional_number(&mut self, value: Option<u64>) {
        match value {
            Some(value) => {
                self.0.update([1]);
                self.number(value);
            },
            None => self.0.update([0]),
        }
    }

    fn field(&mut self, value: &[u8]) {
        self.count(value.len());
        self.0.update(value);
    }

    fn version(&mut self, version: &Version) {
        self.number(version.major);
        self.number(version.minor);
        self.number(version.patch);
        self.field(version.pre.as_str().as_bytes());
    }

    fn key_category<K>(&mut self, category: &[u8], keys: &[K])
    where
        K: AsRef<str>,
    {
        self.field(category);
        self.count(keys.len());
        for key in keys {
            self.field(key.as_ref().as_bytes());
        }
    }

    fn dependency_category(&mut self, dependencies: &[PluginDependency]) -> Result<(), PluginKey> {
        self.field(b"dependencies");
        self.count(dependencies.len());
        for dependency in dependencies {
            self.field(dependency.key().as_str().as_bytes());
            if !self.version_requirement(dependency.req()) {
                return Err(dependency.key().clone());
            }
        }
        Ok(())
    }

    fn version_requirement(&mut self, requirement: &VersionReq) -> bool {
        self.count(requirement.comparators.len());
        for comparator in &requirement.comparators {
            let Some(operator) = comparator_op_tag(comparator.op) else {
                return false;
            };
            self.0.update([operator]);
            self.number(comparator.major);
            self.optional_number(comparator.minor);
            self.optional_number(comparator.patch);
            self.field(comparator.pre.as_str().as_bytes());
        }
        true
    }

    fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

impl nebula_error::ActivationDiagnostics for WorkerFlavorIntegrityError {
    fn activation_diagnostics(&self) -> Vec<nebula_error::ActivationDiagnostic> {
        use nebula_error::ActivationDiagnostic;

        let (code, path, expected, actual, remediation) = match self {
            Self::UnsupportedRecordVersion { found } => (
                "PLUGIN_FLAVOR_INTEGRITY:UNSUPPORTED_RECORD_VERSION",
                "/worker_flavor/record_version",
                WORKER_FLAVOR_RECORD_VERSION_V1.to_string(),
                found.to_string(),
                "re-record the worker flavor with a runtime that writes this envelope version",
            ),
            Self::UnsupportedCanonicalHashVersion { found } => (
                "PLUGIN_FLAVOR_INTEGRITY:UNSUPPORTED_CANONICAL_HASH_VERSION",
                "/worker_flavor/canonical_hash_version",
                WORKER_FLAVOR_CANONICAL_HASH_VERSION_V1.to_string(),
                found.to_string(),
                "re-record the worker flavor with a runtime that derives this identity version",
            ),
            // Both identities are content digests, so reporting them leaks
            // nothing about the artifacts they address.
            Self::RevisionIdMismatch { claimed, computed } => (
                "PLUGIN_FLAVOR_INTEGRITY:REVISION_ID_MISMATCH",
                "/worker_flavor/id",
                computed.to_string(),
                claimed.to_string(),
                "discard the record: its claimed identity does not match its own content",
            ),
        };

        vec![
            ActivationDiagnostic::new(code, path, expected, actual, remediation).unwrap_or_else(
                || {
                    ActivationDiagnostic::new(
                        code,
                        path,
                        "<contract>",
                        "<unavailable>",
                        "discard and re-record the worker flavor",
                    )
                    .unwrap_or_else(|| {
                        unreachable!("the fallback diagnostic uses non-empty constants")
                    })
                },
            ),
        ]
    }
}

#[cfg(test)]
mod activation_diagnostic_tests {
    use nebula_error::ActivationDiagnostics;

    use super::*;

    #[test]
    fn every_flavor_integrity_rejection_reports_all_five_fields() {
        let rejections = [
            WorkerFlavorIntegrityError::UnsupportedRecordVersion { found: 99 },
            WorkerFlavorIntegrityError::UnsupportedCanonicalHashVersion { found: 99 },
            WorkerFlavorIntegrityError::RevisionIdMismatch {
                claimed: WorkerFlavorRevisionId::from_bytes([0x11; 32]),
                computed: WorkerFlavorRevisionId::from_bytes([0x22; 32]),
            },
        ];

        for rejection in &rejections {
            let diagnostics = rejection.activation_diagnostics();
            assert!(!diagnostics.is_empty());
            for reported in diagnostics {
                for field in [
                    reported.code(),
                    reported.path(),
                    reported.expected(),
                    reported.actual(),
                    reported.remediation(),
                ] {
                    assert!(!field.trim().is_empty(), "NS14 requires all five fields");
                }
            }
        }
    }

    /// The supported version is reported as `expected`, so a caller learns what
    /// to re-record against rather than only that the found one was wrong.
    #[test]
    fn an_unsupported_version_names_the_one_this_runtime_reads() {
        let reported = WorkerFlavorIntegrityError::UnsupportedRecordVersion { found: 99 }
            .activation_diagnostics();
        let only = reported.first().expect("one diagnostic is reported");
        assert_eq!(only.expected(), WORKER_FLAVOR_RECORD_VERSION_V1.to_string());
        assert_eq!(only.actual(), "99");
    }
}
