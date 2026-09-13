//! Version-two execution contract with exact, site-qualified binding records.

use core::{fmt, str::FromStr};

use nebula_core::{
    CredentialId, CredentialKey, ExecutablePlanRevisionId, ExecutionContractBundleFingerprint,
    ExecutionContractBundleId, NodeKey, OrgId, PluginSetId, ResourceId, ResourceKey, WorkspaceId,
};
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, de};
use sha2::{Digest, Sha256};

use crate::{ExecutionProfile, ExecutionRevisions};

const SCHEMA_VERSION_V2: u16 = 2;
const DURABLE_ENVELOPE_VERSION_V2: u16 = 2;
const FINGERPRINT_VERSION_V2: u16 = 2;
const FINGERPRINT_DOMAIN_V2: &[u8] = b"nebula.execution-contract-bundle.fingerprint.v2";

/// Canonical semantic version recorded for a resource or credential contract.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BindingContractVersion(String);

impl BindingContractVersion {
    /// Parses a canonical semantic version.
    ///
    /// # Errors
    ///
    /// Returns [`BindingManifestError::InvalidContractVersion`] when the value is not valid,
    /// canonical SemVer.
    pub fn parse(value: &str) -> Result<Self, BindingManifestError> {
        let parsed =
            Version::parse(value).map_err(|_| BindingManifestError::InvalidContractVersion)?;
        if parsed.to_string() != value {
            return Err(BindingManifestError::InvalidContractVersion);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the canonical SemVer text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BindingContractVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for BindingContractVersion {
    type Err = BindingManifestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for BindingContractVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

/// One credential capability required by a bound slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialCapability {
    /// Interactive authorization is required.
    Interactive,
    /// Refresh is required.
    Refreshable,
    /// Revocation is required.
    Revocable,
    /// Provider-side validation is required.
    Testable,
    /// Dynamic material generation is required.
    Dynamic,
}

/// Exact credential contract expected by one selected binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialBindingContractV2 {
    #[serde(
        serialize_with = "serialize_credential_key",
        deserialize_with = "deserialize_credential_key"
    )]
    key: CredentialKey,
    version: BindingContractVersion,
    required_capabilities: Box<[CredentialCapability]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedCredentialBindingContractV2 {
    #[serde(deserialize_with = "deserialize_credential_key")]
    key: CredentialKey,
    version: BindingContractVersion,
    required_capabilities: Box<[CredentialCapability]>,
}

impl<'de> Deserialize<'de> for CredentialBindingContractV2 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let recorded = RecordedCredentialBindingContractV2::deserialize(deserializer)?;
        if !recorded
            .required_capabilities
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        {
            return Err(de::Error::custom(
                "credential capability requirements are not canonical",
            ));
        }
        Ok(Self {
            key: recorded.key,
            version: recorded.version,
            required_capabilities: recorded.required_capabilities,
        })
    }
}

impl CredentialBindingContractV2 {
    /// Creates a contract and canonicalizes its capability set.
    #[must_use]
    pub fn new(
        key: CredentialKey,
        version: BindingContractVersion,
        required_capabilities: impl IntoIterator<Item = CredentialCapability>,
    ) -> Self {
        let mut required_capabilities = required_capabilities.into_iter().collect::<Vec<_>>();
        required_capabilities.sort_unstable();
        required_capabilities.dedup();
        Self {
            key,
            version,
            required_capabilities: required_capabilities.into_boxed_slice(),
        }
    }

    /// Returns the credential contract key.
    #[must_use]
    pub const fn key(&self) -> &CredentialKey {
        &self.key
    }

    /// Returns the exact contract version.
    #[must_use]
    pub const fn version(&self) -> &BindingContractVersion {
        &self.version
    }

    /// Returns the sorted, duplicate-free capability requirements.
    #[must_use]
    pub const fn required_capabilities(&self) -> &[CredentialCapability] {
        &self.required_capabilities
    }
}

/// Exact resource contract expected by one selected binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceBindingContractV2 {
    #[serde(
        serialize_with = "serialize_resource_key",
        deserialize_with = "deserialize_resource_key"
    )]
    key: ResourceKey,
    version: BindingContractVersion,
}

impl ResourceBindingContractV2 {
    /// Creates an exact resource contract record.
    #[must_use]
    pub const fn new(key: ResourceKey, version: BindingContractVersion) -> Self {
        Self { key, version }
    }

    /// Returns the resource contract key.
    #[must_use]
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Returns the exact contract version.
    #[must_use]
    pub const fn version(&self) -> &BindingContractVersion {
        &self.version
    }
}

/// Exact workflow site owning a binding slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExecutionBindingSiteV2 {
    /// Executable graph node.
    Node(NodeKey),
    /// Workflow trigger.
    Trigger(NodeKey),
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "key",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum RecordedExecutionBindingSiteV2 {
    Node(String),
    Trigger(String),
}

impl Serialize for ExecutionBindingSiteV2 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Node(key) => RecordedExecutionBindingSiteV2::Node(key.to_string()),
            Self::Trigger(key) => RecordedExecutionBindingSiteV2::Trigger(key.to_string()),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExecutionBindingSiteV2 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match RecordedExecutionBindingSiteV2::deserialize(deserializer)? {
            RecordedExecutionBindingSiteV2::Node(key) => key
                .parse()
                .map(Self::Node)
                .map_err(|_| de::Error::custom("invalid node binding site key")),
            RecordedExecutionBindingSiteV2::Trigger(key) => key
                .parse()
                .map(Self::Trigger)
                .map_err(|_| de::Error::custom("invalid trigger binding site key")),
        }
    }
}

/// Typed selected object and expected contract for a binding slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionBindingTargetV2 {
    /// Exact credential selection.
    Credential {
        /// Tenant-owned credential identifier selected during activation.
        credential_id: CredentialId,
        /// Contract and capabilities the worker must revalidate.
        contract: CredentialBindingContractV2,
    },
    /// Exact resource selection.
    Resource {
        /// Tenant-owned resource identifier selected during activation.
        resource_id: ResourceId,
        /// Contract the worker must revalidate.
        contract: ResourceBindingContractV2,
    },
}

/// One exact `(site, slot_key)` binding selected during authenticated activation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBindingEntryV2 {
    site: ExecutionBindingSiteV2,
    slot_key: String,
    target: ExecutionBindingTargetV2,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedExecutionBindingEntryV2 {
    site: ExecutionBindingSiteV2,
    slot_key: String,
    target: ExecutionBindingTargetV2,
}

impl<'de> Deserialize<'de> for ExecutionBindingEntryV2 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let recorded = RecordedExecutionBindingEntryV2::deserialize(deserializer)?;
        Self::new(recorded.site, recorded.slot_key, recorded.target).map_err(de::Error::custom)
    }
}

impl ExecutionBindingEntryV2 {
    /// Creates a site-qualified binding entry.
    ///
    /// # Errors
    ///
    /// Returns [`BindingManifestError::InvalidSlotKey`] for an empty or padded slot key.
    pub fn new(
        site: ExecutionBindingSiteV2,
        slot_key: impl Into<String>,
        target: ExecutionBindingTargetV2,
    ) -> Result<Self, BindingManifestError> {
        let slot_key = slot_key.into();
        if slot_key.is_empty() || slot_key.trim() != slot_key {
            return Err(BindingManifestError::InvalidSlotKey);
        }
        Ok(Self {
            site,
            slot_key,
            target,
        })
    }

    /// Returns the exact node or trigger site.
    #[must_use]
    pub const fn site(&self) -> &ExecutionBindingSiteV2 {
        &self.site
    }

    /// Returns the declared slot key.
    #[must_use]
    pub fn slot_key(&self) -> &str {
        &self.slot_key
    }

    /// Returns the typed selected target and expected contract.
    #[must_use]
    pub const fn target(&self) -> &ExecutionBindingTargetV2 {
        &self.target
    }
}

/// Canonically ordered, exact binding closure for an execution contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ExecutionBindingManifestV2(Box<[ExecutionBindingEntryV2]>);

impl ExecutionBindingManifestV2 {
    /// Builds a canonical manifest and rejects duplicate `(site, slot_key)` entries.
    ///
    /// # Errors
    ///
    /// Returns [`BindingManifestError::DuplicateSiteSlot`] when two entries claim one slot.
    pub fn new(
        entries: impl IntoIterator<Item = ExecutionBindingEntryV2>,
    ) -> Result<Self, BindingManifestError> {
        let mut entries = entries.into_iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| binding_key(left).cmp(&binding_key(right)));
        if entries
            .windows(2)
            .any(|pair| binding_key(&pair[0]) == binding_key(&pair[1]))
        {
            return Err(BindingManifestError::DuplicateSiteSlot);
        }
        Ok(Self(entries.into_boxed_slice()))
    }

    /// Returns the canonical site/slot entries.
    #[must_use]
    pub const fn entries(&self) -> &[ExecutionBindingEntryV2] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ExecutionBindingManifestV2 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Box::<[ExecutionBindingEntryV2]>::deserialize(deserializer)?;
        if entries
            .windows(2)
            .any(|pair| binding_key(&pair[0]) >= binding_key(&pair[1]))
        {
            return Err(de::Error::custom(
                "execution binding manifest is not canonical",
            ));
        }
        Ok(Self(entries))
    }
}

fn binding_key(entry: &ExecutionBindingEntryV2) -> (&ExecutionBindingSiteV2, &str) {
    (&entry.site, entry.slot_key.as_str())
}

/// Construction failures for exact binding manifests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BindingManifestError {
    /// A contract version was invalid or not in canonical SemVer form.
    #[error("binding contract version is not canonical semantic versioning")]
    InvalidContractVersion,
    /// A slot key was empty or had surrounding whitespace.
    #[error("binding slot key is not canonical")]
    InvalidSlotKey,
    /// More than one entry claimed the same site and slot.
    #[error("execution binding manifest contains a duplicate site and slot")]
    DuplicateSiteSlot,
}

/// Final version-two execution contract with an exact binding manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionContractBundleV2 {
    bundle_id: ExecutionContractBundleId,
    org_id: OrgId,
    workspace_id: WorkspaceId,
    profile: ExecutionProfile,
    executable_plan_revision_id: ExecutablePlanRevisionId,
    plugin_set_id: PluginSetId,
    revisions: ExecutionRevisions,
    binding_manifest: ExecutionBindingManifestV2,
    schema_version: u16,
    durable_envelope_version: u16,
    fingerprint_version: u16,
    fingerprint: ExecutionContractBundleFingerprint,
}

impl ExecutionContractBundleV2 {
    /// Creates a Graph-v2 contract and binds every semantic field into its fingerprint.
    #[must_use]
    pub fn new_graph_v2(
        bundle_id: ExecutionContractBundleId,
        org_id: OrgId,
        workspace_id: WorkspaceId,
        executable_plan_revision_id: ExecutablePlanRevisionId,
        plugin_set_id: PluginSetId,
        revisions: ExecutionRevisions,
        binding_manifest: ExecutionBindingManifestV2,
    ) -> Self {
        let fields = FingerprintFieldsV2 {
            org_id,
            workspace_id,
            profile: ExecutionProfile::Graph,
            executable_plan_revision_id,
            plugin_set_id,
            revisions,
            binding_manifest: &binding_manifest,
        };
        let fingerprint = fingerprint_v2(fields);
        Self {
            bundle_id,
            org_id,
            workspace_id,
            profile: ExecutionProfile::Graph,
            executable_plan_revision_id,
            plugin_set_id,
            revisions,
            binding_manifest,
            schema_version: SCHEMA_VERSION_V2,
            durable_envelope_version: DURABLE_ENVELOPE_VERSION_V2,
            fingerprint_version: FINGERPRINT_VERSION_V2,
            fingerprint,
        }
    }

    /// Reconstructs and structurally validates an untrusted V2 record.
    ///
    /// # Errors
    ///
    /// Returns a typed integrity error for unsupported protocol values or fingerprint mismatch.
    pub fn try_from_recorded_v2(
        recorded: RecordedExecutionContractBundleV2,
    ) -> Result<Self, ExecutionContractBundleIntegrityErrorV2> {
        if recorded.schema_version != SCHEMA_VERSION_V2 {
            return Err(
                ExecutionContractBundleIntegrityErrorV2::UnsupportedSchemaVersion {
                    actual: recorded.schema_version,
                },
            );
        }
        if recorded.durable_envelope_version != DURABLE_ENVELOPE_VERSION_V2 {
            return Err(
                ExecutionContractBundleIntegrityErrorV2::UnsupportedEnvelopeVersion {
                    actual: recorded.durable_envelope_version,
                },
            );
        }
        if recorded.fingerprint_version != FINGERPRINT_VERSION_V2 {
            return Err(
                ExecutionContractBundleIntegrityErrorV2::UnsupportedFingerprintVersion {
                    actual: recorded.fingerprint_version,
                },
            );
        }
        if recorded.profile != ExecutionProfile::Graph {
            return Err(ExecutionContractBundleIntegrityErrorV2::UnsupportedProfile);
        }
        let computed = fingerprint_v2(FingerprintFieldsV2 {
            org_id: recorded.org_id,
            workspace_id: recorded.workspace_id,
            profile: recorded.profile,
            executable_plan_revision_id: recorded.executable_plan_revision_id,
            plugin_set_id: recorded.plugin_set_id,
            revisions: recorded.revisions,
            binding_manifest: &recorded.binding_manifest,
        });
        if recorded.fingerprint != computed {
            return Err(
                ExecutionContractBundleIntegrityErrorV2::FingerprintMismatch {
                    claimed: recorded.fingerprint,
                    computed,
                },
            );
        }
        Ok(Self {
            bundle_id: recorded.bundle_id,
            org_id: recorded.org_id,
            workspace_id: recorded.workspace_id,
            profile: recorded.profile,
            executable_plan_revision_id: recorded.executable_plan_revision_id,
            plugin_set_id: recorded.plugin_set_id,
            revisions: recorded.revisions,
            binding_manifest: recorded.binding_manifest,
            schema_version: recorded.schema_version,
            durable_envelope_version: recorded.durable_envelope_version,
            fingerprint_version: recorded.fingerprint_version,
            fingerprint: recorded.fingerprint,
        })
    }

    /// Returns the random record identity, excluded from the semantic fingerprint.
    #[must_use]
    pub const fn bundle_id(&self) -> ExecutionContractBundleId {
        self.bundle_id
    }
    /// Returns the recorded organization identity.
    #[must_use]
    pub const fn org_id(&self) -> OrgId {
        self.org_id
    }
    /// Returns the recorded workspace identity.
    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
    /// Returns the selected execution profile.
    #[must_use]
    pub const fn profile(&self) -> ExecutionProfile {
        self.profile
    }
    /// Returns the exact executable-plan revision.
    #[must_use]
    pub const fn executable_plan_revision_id(&self) -> ExecutablePlanRevisionId {
        self.executable_plan_revision_id
    }
    /// Returns the independent plugin-set pin.
    #[must_use]
    pub const fn plugin_set_id(&self) -> PluginSetId {
        self.plugin_set_id
    }
    /// Returns workflow and worker-flavor revision pins.
    #[must_use]
    pub const fn revisions(&self) -> ExecutionRevisions {
        self.revisions
    }
    /// Returns the exact site/slot binding manifest.
    #[must_use]
    pub const fn binding_manifest(&self) -> &ExecutionBindingManifestV2 {
        &self.binding_manifest
    }
    /// Returns the bundle schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }
    /// Returns the durable envelope version.
    #[must_use]
    pub const fn durable_envelope_version(&self) -> u16 {
        self.durable_envelope_version
    }
    /// Returns the fingerprint protocol version.
    #[must_use]
    pub const fn fingerprint_version(&self) -> u16 {
        self.fingerprint_version
    }
    /// Returns the structural fingerprint; this is not tenant authority or a signature.
    #[must_use]
    pub const fn fingerprint(&self) -> ExecutionContractBundleFingerprint {
        self.fingerprint
    }
}

impl<'de> Deserialize<'de> for ExecutionContractBundleV2 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let recorded = RecordedExecutionContractBundleV2::deserialize(deserializer)?;
        Self::try_from_recorded_v2(recorded).map_err(de::Error::custom)
    }
}

/// Untrusted recorded V2 execution-contract envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedExecutionContractBundleV2 {
    bundle_id: ExecutionContractBundleId,
    org_id: OrgId,
    workspace_id: WorkspaceId,
    profile: ExecutionProfile,
    executable_plan_revision_id: ExecutablePlanRevisionId,
    plugin_set_id: PluginSetId,
    revisions: ExecutionRevisions,
    binding_manifest: ExecutionBindingManifestV2,
    schema_version: u16,
    durable_envelope_version: u16,
    fingerprint_version: u16,
    fingerprint: ExecutionContractBundleFingerprint,
}

impl From<&ExecutionContractBundleV2> for RecordedExecutionContractBundleV2 {
    fn from(bundle: &ExecutionContractBundleV2) -> Self {
        Self {
            bundle_id: bundle.bundle_id,
            org_id: bundle.org_id,
            workspace_id: bundle.workspace_id,
            profile: bundle.profile,
            executable_plan_revision_id: bundle.executable_plan_revision_id,
            plugin_set_id: bundle.plugin_set_id,
            revisions: bundle.revisions,
            binding_manifest: bundle.binding_manifest.clone(),
            schema_version: bundle.schema_version,
            durable_envelope_version: bundle.durable_envelope_version,
            fingerprint_version: bundle.fingerprint_version,
            fingerprint: bundle.fingerprint,
        }
    }
}

/// Structural integrity failures for a recorded V2 execution contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum ExecutionContractBundleIntegrityErrorV2 {
    /// Unsupported schema version.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE_V2:UNSUPPORTED_SCHEMA_VERSION"
    )]
    #[error("unsupported V2 execution contract bundle schema version {actual}")]
    UnsupportedSchemaVersion {
        /// Unsupported recorded value.
        actual: u16,
    },
    /// Unsupported envelope version.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE_V2:UNSUPPORTED_ENVELOPE_VERSION"
    )]
    #[error("unsupported V2 execution contract bundle envelope version {actual}")]
    UnsupportedEnvelopeVersion {
        /// Unsupported recorded value.
        actual: u16,
    },
    /// Unsupported fingerprint version.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE_V2:UNSUPPORTED_FINGERPRINT_VERSION"
    )]
    #[error("unsupported V2 execution contract bundle fingerprint version {actual}")]
    UnsupportedFingerprintVersion {
        /// Unsupported recorded value.
        actual: u16,
    },
    /// Unsupported execution profile.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE_V2:UNSUPPORTED_PROFILE"
    )]
    #[error("V2 execution contract bundle uses an unsupported profile")]
    UnsupportedProfile,
    /// Claimed fingerprint differs from the semantic content.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE_V2:FINGERPRINT_MISMATCH"
    )]
    #[error("V2 execution contract bundle fingerprint does not match its semantic content")]
    FingerprintMismatch {
        /// Fingerprint from the untrusted envelope.
        claimed: ExecutionContractBundleFingerprint,
        /// Recomputed fingerprint.
        computed: ExecutionContractBundleFingerprint,
    },
}

struct FingerprintFieldsV2<'a> {
    org_id: OrgId,
    workspace_id: WorkspaceId,
    profile: ExecutionProfile,
    executable_plan_revision_id: ExecutablePlanRevisionId,
    plugin_set_id: PluginSetId,
    revisions: ExecutionRevisions,
    binding_manifest: &'a ExecutionBindingManifestV2,
}

fn fingerprint_v2(fields: FingerprintFieldsV2<'_>) -> ExecutionContractBundleFingerprint {
    let mut fingerprint = CanonicalFingerprintV2::new();
    fingerprint.field(1, &SCHEMA_VERSION_V2.to_be_bytes());
    fingerprint.field(2, &DURABLE_ENVELOPE_VERSION_V2.to_be_bytes());
    fingerprint.field(3, &FINGERPRINT_VERSION_V2.to_be_bytes());
    fingerprint.field(4, &fields.org_id.as_bytes());
    fingerprint.field(5, &fields.workspace_id.as_bytes());
    fingerprint.field(6, fields.profile.wire_name().as_bytes());
    fingerprint.field(7, fields.executable_plan_revision_id.as_bytes());
    fingerprint.field(8, fields.plugin_set_id.as_bytes());
    fingerprint.field(9, &fields.revisions.workflow().as_bytes());
    fingerprint.field(10, fields.revisions.worker_flavor().as_bytes());
    fingerprint.field(
        11,
        &(fields.binding_manifest.entries().len() as u64).to_be_bytes(),
    );
    for entry in fields.binding_manifest.entries() {
        let (site_tag, site_key) = match entry.site() {
            ExecutionBindingSiteV2::Node(key) => (0_u8, key.as_str()),
            ExecutionBindingSiteV2::Trigger(key) => (1_u8, key.as_str()),
        };
        fingerprint.field(12, &[site_tag]);
        fingerprint.field(13, site_key.as_bytes());
        fingerprint.field(14, entry.slot_key().as_bytes());
        match entry.target() {
            ExecutionBindingTargetV2::Credential {
                credential_id,
                contract,
            } => {
                fingerprint.field(15, &[0]);
                fingerprint.field(16, &credential_id.as_bytes());
                fingerprint.field(17, contract.key().as_str().as_bytes());
                fingerprint.field(18, contract.version().as_str().as_bytes());
                fingerprint.field(
                    19,
                    &(contract.required_capabilities().len() as u64).to_be_bytes(),
                );
                for capability in contract.required_capabilities() {
                    fingerprint.field(20, &[capability.wire_tag()]);
                }
            },
            ExecutionBindingTargetV2::Resource {
                resource_id,
                contract,
            } => {
                fingerprint.field(15, &[1]);
                fingerprint.field(16, &resource_id.as_bytes());
                fingerprint.field(17, contract.key().as_str().as_bytes());
                fingerprint.field(18, contract.version().as_str().as_bytes());
            },
        }
    }
    fingerprint.finish()
}

struct CanonicalFingerprintV2(Sha256);

impl CanonicalFingerprintV2 {
    fn new() -> Self {
        let mut fingerprint = Self(Sha256::new());
        fingerprint.field(0, FINGERPRINT_DOMAIN_V2);
        fingerprint
    }

    fn field(&mut self, tag: u8, value: &[u8]) {
        self.0.update([tag]);
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    fn finish(self) -> ExecutionContractBundleFingerprint {
        ExecutionContractBundleFingerprint::from_bytes(self.0.finalize().into())
    }
}

impl CredentialCapability {
    const fn wire_tag(self) -> u8 {
        match self {
            Self::Interactive => 0,
            Self::Refreshable => 1,
            Self::Revocable => 2,
            Self::Testable => 3,
            Self::Dynamic => 4,
        }
    }
}

fn serialize_credential_key<S>(key: &CredentialKey, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(key.as_str())
}

fn deserialize_credential_key<'de, D>(deserializer: D) -> Result<CredentialKey, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer)?
        .parse()
        .map_err(|_| de::Error::custom("invalid credential contract key"))
}

fn serialize_resource_key<S>(key: &ResourceKey, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(key.as_str())
}

fn deserialize_resource_key<'de, D>(deserializer: D) -> Result<ResourceKey, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer)?
        .parse()
        .map_err(|_| de::Error::custom("invalid resource contract key"))
}
