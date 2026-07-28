//! Immutable execution-contract bundle vocabulary and structural integrity checks.
//!
//! A bundle is recorded data, not authority. Successful reconstruction proves only that its
//! wire form is canonical and its semantic fields match its claimed content fingerprint. It
//! does not prove referenced objects exist, belong to the tenant, remain retained, are mutually
//! compatible, or have been admitted for execution.

use nebula_core::{
    CredentialId, ExecutablePlanRevisionId, ExecutionContractBundleFingerprint,
    ExecutionContractBundleId, OrgId, PluginSetId, WorkspaceId,
};
use serde::{Deserialize, Deserializer, Serialize, de};
use sha2::{Digest, Sha256};

use crate::ExecutionRevisions;

const SUPPORTED_SCHEMA_VERSION: u16 = 1;
const SUPPORTED_DURABLE_ENVELOPE_VERSION: u16 = 1;
const SUPPORTED_FINGERPRINT_VERSION: u16 = 1;
const FINGERPRINT_DOMAIN_V1: &[u8] = b"nebula.execution-contract-bundle.fingerprint.v1";

const TAG_DOMAIN: u8 = 0;
const TAG_SCHEMA_VERSION: u8 = 1;
const TAG_DURABLE_ENVELOPE_VERSION: u8 = 2;
const TAG_FINGERPRINT_VERSION: u8 = 3;
const TAG_ORG_ID: u8 = 4;
const TAG_WORKSPACE_ID: u8 = 5;
const TAG_PROFILE: u8 = 6;
const TAG_EXECUTABLE_PLAN_REVISION_ID: u8 = 7;
const TAG_PLUGIN_SET_ID: u8 = 8;
const TAG_WORKFLOW_VERSION_ID: u8 = 9;
const TAG_WORKER_FLAVOR_REVISION_ID: u8 = 10;
const TAG_CREDENTIAL_COUNT: u8 = 11;
const TAG_CREDENTIAL_ID: u8 = 12;

/// Execution model selected by an immutable execution-contract bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ExecutionProfile {
    /// Execute a precompiled workflow graph.
    Graph,
}

impl ExecutionProfile {
    const fn wire_name(self) -> &'static str {
        match self {
            Self::Graph => "graph",
        }
    }
}

/// Final immutable semantic contract recorded for one execution.
///
/// Tenant identifiers are identity fields, not authority. Runtime admission must independently
/// load and verify every exact referenced revision, validate current tenant authorization, and
/// reject missing or incompatible references without falling back to a latest version.
///
/// The exact loaded plan must carry its slot-to-selected-credential mappings (whose values are
/// [`CredentialId`]s) and credential contract revisions. Admission must compare the unique
/// selected credential identities exactly with [`Self::authorized_credential_ids`]. If a plan
/// contains only abstract credential requirements, this v1 bundle shape is insufficient to
/// establish the credential closure.
///
/// # Example
///
/// ```
/// use nebula_core::{
///     CredentialId, ExecutablePlanRevisionId, ExecutionContractBundleId, OrgId, PluginSetId,
///     WorkerFlavorRevisionId, WorkflowVersionId, WorkspaceId,
/// };
/// use nebula_execution::{ExecutionContractBundle, ExecutionRevisions};
///
/// let revisions = ExecutionRevisions::new(
///     WorkflowVersionId::new(),
///     WorkerFlavorRevisionId::from_bytes([0x11; 32]),
/// );
/// let bundle = ExecutionContractBundle::new_graph_v1(
///     ExecutionContractBundleId::new(),
///     OrgId::new(),
///     WorkspaceId::new(),
///     ExecutablePlanRevisionId::from_bytes([0x22; 32]),
///     PluginSetId::from_bytes([0x33; 32]),
///     revisions,
///     [CredentialId::new()],
/// );
///
/// assert_eq!(bundle.schema_version(), 1);
/// assert_eq!(bundle.authorized_credential_ids().len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionContractBundle {
    bundle_id: ExecutionContractBundleId,
    org_id: OrgId,
    workspace_id: WorkspaceId,
    profile: ExecutionProfile,
    executable_plan_revision_id: ExecutablePlanRevisionId,
    plugin_set_id: PluginSetId,
    revisions: ExecutionRevisions,
    authorized_credential_ids: Box<[CredentialId]>,
    schema_version: u16,
    durable_envelope_version: u16,
    fingerprint_version: u16,
    fingerprint: ExecutionContractBundleFingerprint,
}

impl ExecutionContractBundle {
    /// Constructs the supported Graph-v1 bundle and computes its final fingerprint.
    ///
    /// Credential identities are sorted and deduplicated before they are retained or
    /// fingerprinted. The constructor stamps every supported protocol version internally; a
    /// caller cannot claim a version or fingerprint.
    #[must_use]
    pub fn new_graph_v1(
        bundle_id: ExecutionContractBundleId,
        org_id: OrgId,
        workspace_id: WorkspaceId,
        executable_plan_revision_id: ExecutablePlanRevisionId,
        plugin_set_id: PluginSetId,
        revisions: ExecutionRevisions,
        authorized_credential_ids: impl IntoIterator<Item = CredentialId>,
    ) -> Self {
        let mut authorized_credential_ids =
            authorized_credential_ids.into_iter().collect::<Vec<_>>();
        authorized_credential_ids.sort_unstable();
        authorized_credential_ids.dedup();
        let authorized_credential_ids = authorized_credential_ids.into_boxed_slice();
        let profile = ExecutionProfile::Graph;
        let fingerprint = fingerprint_v1(FingerprintFields {
            org_id,
            workspace_id,
            profile,
            executable_plan_revision_id,
            plugin_set_id,
            revisions,
            authorized_credential_ids: &authorized_credential_ids,
        });

        Self {
            bundle_id,
            org_id,
            workspace_id,
            profile,
            executable_plan_revision_id,
            plugin_set_id,
            revisions,
            authorized_credential_ids,
            schema_version: SUPPORTED_SCHEMA_VERSION,
            durable_envelope_version: SUPPORTED_DURABLE_ENVELOPE_VERSION,
            fingerprint_version: SUPPORTED_FINGERPRINT_VERSION,
            fingerprint,
        }
    }

    /// Reconstructs a bundle after validating the complete recorded-v1 wire contract.
    ///
    /// This performs structural integrity checks only. Success is not authorization, admission,
    /// existence, retention, or compatibility proof.
    ///
    /// # Errors
    ///
    /// Returns an integrity error for unsupported protocol versions or profile, noncanonical
    /// credential ordering, or a content fingerprint mismatch.
    pub fn try_from_recorded_v1(
        recorded: RecordedExecutionContractBundleV1,
    ) -> Result<Self, ExecutionContractBundleIntegrityError> {
        if recorded.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(
                ExecutionContractBundleIntegrityError::UnsupportedSchemaVersion {
                    actual: recorded.schema_version,
                },
            );
        }
        if recorded.durable_envelope_version != SUPPORTED_DURABLE_ENVELOPE_VERSION {
            return Err(
                ExecutionContractBundleIntegrityError::UnsupportedEnvelopeVersion {
                    actual: recorded.durable_envelope_version,
                },
            );
        }
        if recorded.fingerprint_version != SUPPORTED_FINGERPRINT_VERSION {
            return Err(
                ExecutionContractBundleIntegrityError::UnsupportedFingerprintVersion {
                    actual: recorded.fingerprint_version,
                },
            );
        }
        let profile = match recorded.profile.as_str() {
            "graph" => ExecutionProfile::Graph,
            _ => return Err(ExecutionContractBundleIntegrityError::UnsupportedProfile),
        };
        if !recorded
            .authorized_credential_ids
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        {
            return Err(ExecutionContractBundleIntegrityError::NonCanonicalCredentialIds);
        }

        let computed = fingerprint_v1(FingerprintFields {
            org_id: recorded.org_id,
            workspace_id: recorded.workspace_id,
            profile,
            executable_plan_revision_id: recorded.executable_plan_revision_id,
            plugin_set_id: recorded.plugin_set_id,
            revisions: recorded.revisions,
            authorized_credential_ids: &recorded.authorized_credential_ids,
        });
        if recorded.fingerprint != computed {
            return Err(ExecutionContractBundleIntegrityError::FingerprintMismatch {
                claimed: recorded.fingerprint,
                computed,
            });
        }

        Ok(Self {
            bundle_id: recorded.bundle_id,
            org_id: recorded.org_id,
            workspace_id: recorded.workspace_id,
            profile,
            executable_plan_revision_id: recorded.executable_plan_revision_id,
            plugin_set_id: recorded.plugin_set_id,
            revisions: recorded.revisions,
            authorized_credential_ids: recorded.authorized_credential_ids,
            schema_version: recorded.schema_version,
            durable_envelope_version: recorded.durable_envelope_version,
            fingerprint_version: recorded.fingerprint_version,
            fingerprint: recorded.fingerprint,
        })
    }

    /// Returns the random record identity, which is excluded from the semantic fingerprint.
    #[must_use]
    pub const fn bundle_id(&self) -> ExecutionContractBundleId {
        self.bundle_id
    }

    /// Returns the organization identity recorded in the bundle.
    #[must_use]
    pub const fn org_id(&self) -> OrgId {
        self.org_id
    }

    /// Returns the workspace identity recorded in the bundle.
    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    /// Returns the selected execution profile.
    #[must_use]
    pub const fn profile(&self) -> ExecutionProfile {
        self.profile
    }

    /// Returns the exact immutable executable-plan revision.
    #[must_use]
    pub const fn executable_plan_revision_id(&self) -> ExecutablePlanRevisionId {
        self.executable_plan_revision_id
    }

    /// Returns the independent plugin-set pin.
    ///
    /// This identity is not proof of schemas, runtime behavior, artifact authenticity, or the
    /// complete frozen registry. Admission must load and validate the referenced plugin set.
    #[must_use]
    pub const fn plugin_set_id(&self) -> PluginSetId {
        self.plugin_set_id
    }

    /// Returns the workflow and worker-flavor revision pins.
    #[must_use]
    pub const fn revisions(&self) -> ExecutionRevisions {
        self.revisions
    }

    /// Returns the sorted, deduplicated credential identities selected by the exact plan.
    #[must_use]
    pub fn authorized_credential_ids(&self) -> &[CredentialId] {
        &self.authorized_credential_ids
    }

    /// Returns the bundle schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Returns the durable-envelope version.
    #[must_use]
    pub const fn durable_envelope_version(&self) -> u16 {
        self.durable_envelope_version
    }

    /// Returns the fingerprint protocol version.
    #[must_use]
    pub const fn fingerprint_version(&self) -> u16 {
        self.fingerprint_version
    }

    /// Returns the structural content fingerprint.
    ///
    /// The fingerprint is not an authentication code, signature, tenant proof, or admission
    /// capability.
    #[must_use]
    pub const fn fingerprint(&self) -> ExecutionContractBundleFingerprint {
        self.fingerprint
    }
}

impl<'de> Deserialize<'de> for ExecutionContractBundle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let recorded = RecordedExecutionContractBundleV1::deserialize(deserializer)?;
        Self::try_from_recorded_v1(recorded).map_err(de::Error::custom)
    }
}

/// Untrusted recorded/wire input for execution-contract bundle schema v1.
///
/// Fields remain private so this type cannot be mistaken for the validated immutable bundle.
/// Deserialize it from durable wire data, then pass it to
/// [`ExecutionContractBundle::try_from_recorded_v1`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedExecutionContractBundleV1 {
    bundle_id: ExecutionContractBundleId,
    org_id: OrgId,
    workspace_id: WorkspaceId,
    profile: String,
    executable_plan_revision_id: ExecutablePlanRevisionId,
    plugin_set_id: PluginSetId,
    revisions: ExecutionRevisions,
    authorized_credential_ids: Box<[CredentialId]>,
    schema_version: u16,
    durable_envelope_version: u16,
    fingerprint_version: u16,
    fingerprint: ExecutionContractBundleFingerprint,
}

/// Structural integrity failures while reconstructing a recorded execution contract.
///
/// These errors never report authorization, admission, compatibility, existence, or retention
/// outcomes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum ExecutionContractBundleIntegrityError {
    /// The recorded schema version is not supported.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE:UNSUPPORTED_SCHEMA_VERSION"
    )]
    #[error("unsupported execution contract bundle schema version {actual}")]
    UnsupportedSchemaVersion {
        /// Unsupported recorded value.
        actual: u16,
    },

    /// The recorded durable-envelope version is not supported.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE:UNSUPPORTED_ENVELOPE_VERSION"
    )]
    #[error("unsupported execution contract bundle durable envelope version {actual}")]
    UnsupportedEnvelopeVersion {
        /// Unsupported recorded value.
        actual: u16,
    },

    /// The recorded fingerprint protocol version is not supported.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE:UNSUPPORTED_FINGERPRINT_VERSION"
    )]
    #[error("unsupported execution contract bundle fingerprint version {actual}")]
    UnsupportedFingerprintVersion {
        /// Unsupported recorded value.
        actual: u16,
    },

    /// The recorded execution profile is not supported.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE:UNSUPPORTED_PROFILE"
    )]
    #[error("execution contract bundle uses an unsupported execution profile")]
    UnsupportedProfile,

    /// Credential identities are not strictly sorted and unique.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE:NON_CANONICAL_CREDENTIAL_IDS"
    )]
    #[error("execution contract bundle credential identities are not canonical")]
    NonCanonicalCredentialIds,

    /// The claimed fingerprint does not match the semantic content.
    #[classify(
        category = "validation",
        code = "EXECUTION_CONTRACT_BUNDLE:FINGERPRINT_MISMATCH"
    )]
    #[error("execution contract bundle fingerprint does not match its semantic content")]
    FingerprintMismatch {
        /// Fingerprint recorded in the untrusted envelope.
        claimed: ExecutionContractBundleFingerprint,
        /// Fingerprint recomputed from the envelope content.
        computed: ExecutionContractBundleFingerprint,
    },
}

#[derive(Clone, Copy)]
struct FingerprintFields<'a> {
    org_id: OrgId,
    workspace_id: WorkspaceId,
    profile: ExecutionProfile,
    executable_plan_revision_id: ExecutablePlanRevisionId,
    plugin_set_id: PluginSetId,
    revisions: ExecutionRevisions,
    authorized_credential_ids: &'a [CredentialId],
}

fn fingerprint_v1(fields: FingerprintFields<'_>) -> ExecutionContractBundleFingerprint {
    let mut fingerprint = CanonicalFingerprintV1::new();
    fingerprint.field(TAG_SCHEMA_VERSION, &SUPPORTED_SCHEMA_VERSION.to_be_bytes());
    fingerprint.field(
        TAG_DURABLE_ENVELOPE_VERSION,
        &SUPPORTED_DURABLE_ENVELOPE_VERSION.to_be_bytes(),
    );
    fingerprint.field(
        TAG_FINGERPRINT_VERSION,
        &SUPPORTED_FINGERPRINT_VERSION.to_be_bytes(),
    );
    fingerprint.field(TAG_ORG_ID, &fields.org_id.as_bytes());
    fingerprint.field(TAG_WORKSPACE_ID, &fields.workspace_id.as_bytes());
    fingerprint.field(TAG_PROFILE, fields.profile.wire_name().as_bytes());
    fingerprint.field(
        TAG_EXECUTABLE_PLAN_REVISION_ID,
        fields.executable_plan_revision_id.as_bytes(),
    );
    fingerprint.field(TAG_PLUGIN_SET_ID, fields.plugin_set_id.as_bytes());
    fingerprint.field(
        TAG_WORKFLOW_VERSION_ID,
        &fields.revisions.workflow().as_bytes(),
    );
    fingerprint.field(
        TAG_WORKER_FLAVOR_REVISION_ID,
        fields.revisions.worker_flavor().as_bytes(),
    );
    fingerprint.field(
        TAG_CREDENTIAL_COUNT,
        &(fields.authorized_credential_ids.len() as u64).to_be_bytes(),
    );
    for credential_id in fields.authorized_credential_ids {
        fingerprint.field(TAG_CREDENTIAL_ID, &credential_id.as_bytes());
    }
    fingerprint.finish()
}

struct CanonicalFingerprintV1(Sha256);

impl CanonicalFingerprintV1 {
    fn new() -> Self {
        let mut fingerprint = Self(Sha256::new());
        fingerprint.field(TAG_DOMAIN, FINGERPRINT_DOMAIN_V1);
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
