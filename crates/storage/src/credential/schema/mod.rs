//! Credential schema-admission policy.
//!
//! Backend probes translate database state into the backend-neutral observations
//! classified here. Keeping the policy pure makes every rejection path testable
//! without opening a database or leaking driver diagnostics.

use std::{
    collections::{BTreeSet, HashSet},
    fmt,
};

use nebula_core::CredentialId;
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};

const DUPLICATE_KEY_MARKER: &str = "__nebula_duplicate_json_key__";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackendKind {
    #[cfg(any(test, feature = "sqlite"))]
    Sqlite,
    #[cfg(any(test, feature = "postgres"))]
    Postgres,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MigrationSpec {
    pub(crate) version: i64,
    pub(crate) description: String,
    pub(crate) checksum: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct BackendMigrationPolicy {
    pub(crate) supported_floor: i64,
    pub(crate) current_version: i64,
    pub(crate) canonical: Vec<MigrationSpec>,
    pub(crate) reserved_other_backend_versions: BTreeSet<i64>,
}

#[derive(Clone)]
pub(crate) struct MigrationLedgerRow {
    pub(crate) version: i64,
    pub(crate) description: String,
    pub(crate) checksum: Vec<u8>,
    pub(crate) success: bool,
}

#[derive(Clone)]
pub(crate) enum MigrationLedger {
    Absent,
    Present(Vec<MigrationLedgerRow>),
}

#[derive(Clone)]
pub(crate) struct LegacyCredentialRecord {
    pub(crate) id: String,
    pub(crate) owner_id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) state_version: i64,
    pub(crate) data_len: usize,
    pub(crate) version: i64,
    pub(crate) material_epoch: Option<i64>,
    pub(crate) metadata: String,
    pub(crate) record_state: Option<String>,
    pub(crate) tombstoned_at_present: bool,
    pub(crate) expires_at_present: bool,
    pub(crate) reauth_required: bool,
    pub(crate) refresh_retry_mode: Option<String>,
    pub(crate) refresh_retry_not_before_present: bool,
    pub(crate) refresh_retry_phase: Option<String>,
    pub(crate) refresh_retry_kind: Option<String>,
    pub(crate) refresh_retry_diagnostic_code: Option<String>,
}

pub(crate) struct SchemaObservation {
    pub(crate) migration_ledger: MigrationLedger,
    pub(crate) has_user_relations: bool,
    pub(crate) has_credentials_relation: bool,
    pub(crate) credentials: Vec<LegacyCredentialRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SchemaAdmission {
    Fresh,
    CanonicalPrefix { latest: i64 },
}

/// Closed, secret-free reason why a reachable credential schema is unsupported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionReason {
    /// The SQLx ledger relation exists but contains no rows.
    EmptyLedger,
    /// User relations exist without a SQLx migration ledger.
    UnledgeredDatabase,
    /// The SQLx ledger does not have the canonical SQLx 0.9 shape.
    InvalidMigrationLedger,
    /// A supported migration prefix does not have a credentials relation.
    MissingCredentialsRelation,
    /// The credentials relation does not have the canonical backend shape.
    InvalidCredentialsRelation,
    /// The refresh sentinel-event relation or its incident-identity index
    /// does not have the canonical backend shape.
    InvalidSentinelEventsRelation,
    /// The canonical prefix predates the destructive credential support floor.
    BelowSupportedFloor {
        /// Latest successfully recorded migration.
        latest: i64,
    },
    /// A ledger row is not marked successful.
    FailedMigration {
        /// Affected migration number.
        migration: i64,
    },
    /// A migration number occurs more than once.
    DuplicateMigration {
        /// Duplicated migration number.
        migration: i64,
    },
    /// Ledger order or prefix membership differs from the embedded catalog.
    NonCanonicalOrder {
        /// Expected migration number at this position.
        expected: i64,
        /// Observed migration number at this position.
        actual: i64,
    },
    /// SQLite contains a migration number reserved for PostgreSQL.
    ReservedForOtherBackend {
        /// Reserved migration number.
        migration: i64,
    },
    /// The ledger contains a migration outside the embedded catalog.
    UnknownMigration {
        /// Unknown migration number.
        migration: i64,
    },
    /// A ledger description differs from SQLx's embedded description.
    DescriptionMismatch {
        /// Drifted migration number.
        migration: i64,
    },
    /// A ledger checksum differs from SQLx's SHA-384 checksum.
    ChecksumMismatch {
        /// Drifted migration number.
        migration: i64,
    },
    /// At least one credential has no owner.
    OwnerlessCredential,
    /// At least one credential id is not a typed Nebula credential ULID.
    InvalidCredentialId,
    /// At least one state version is outside the `u32` range.
    InvalidStateVersion,
    /// At least one persistence version is below one.
    InvalidCredentialVersion,
    /// A live row has consumed the version reserved for terminal transition.
    LiveVersionExhausted,
    /// A current credential row has an absent or non-positive material epoch.
    InvalidMaterialEpoch,
    /// A current row has an unknown or contradictory structural state.
    InvalidRecordState,
    /// A current tombstone still carries live-only fields or noncanonical metadata.
    InvalidTombstoneShape,
    /// A structural refresh-retry gate has an unknown or contradictory tuple.
    InvalidRefreshRetryGate,
    /// Credential metadata is not valid JSON.
    MalformedMetadata,
    /// Credential metadata is valid JSON but not an object.
    MetadataNotObject,
    /// Credential metadata contains an object key repeated at any depth.
    DuplicateMetadataKey,
    /// Known credential display fields have incompatible JSON shapes.
    InvalidDisplay,
    /// Physical and metadata-projected display names differ.
    DisplayNameMismatch,
    /// A physical name exists without a metadata display-name projection.
    OrphanPhysicalName,
    /// Two live credentials in one owner project the same non-null name.
    DuplicateProjectedName,
}

impl fmt::Display for AdmissionReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyLedger => formatter.write_str("empty migration ledger"),
            Self::UnledgeredDatabase => formatter.write_str("unledgered non-empty database"),
            Self::InvalidMigrationLedger => {
                formatter.write_str("migration ledger structure is invalid")
            },
            Self::MissingCredentialsRelation => {
                formatter.write_str("credentials relation is absent")
            },
            Self::InvalidCredentialsRelation => {
                formatter.write_str("credentials relation structure is invalid")
            },
            Self::InvalidSentinelEventsRelation => {
                formatter.write_str("credential sentinel-events relation structure is invalid")
            },
            Self::BelowSupportedFloor { latest } => {
                write!(
                    formatter,
                    "migration head {latest:04} is below the support floor"
                )
            },
            Self::FailedMigration { migration } => {
                write!(formatter, "migration {migration:04} is not successful")
            },
            Self::DuplicateMigration { migration } => {
                write!(formatter, "migration {migration:04} is duplicated")
            },
            Self::NonCanonicalOrder { expected, actual } => write!(
                formatter,
                "migration order is non-canonical at {actual:04}; expected {expected:04}"
            ),
            Self::ReservedForOtherBackend { migration } => {
                write!(
                    formatter,
                    "migration {migration:04} is reserved for another backend"
                )
            },
            Self::UnknownMigration { migration } => {
                write!(formatter, "migration {migration:04} is unknown")
            },
            Self::DescriptionMismatch { migration } => {
                write!(formatter, "migration {migration:04} description differs")
            },
            Self::ChecksumMismatch { migration } => {
                write!(formatter, "migration {migration:04} checksum differs")
            },
            Self::OwnerlessCredential => formatter.write_str("credential owner is absent"),
            Self::InvalidCredentialId => formatter.write_str("credential id is invalid"),
            Self::InvalidStateVersion => formatter.write_str("credential state version is invalid"),
            Self::InvalidCredentialVersion => {
                formatter.write_str("credential persistence version is invalid")
            },
            Self::LiveVersionExhausted => {
                formatter.write_str("live credential exhausted terminal version headroom")
            },
            Self::InvalidMaterialEpoch => {
                formatter.write_str("credential material epoch is invalid")
            },
            Self::InvalidRecordState => formatter.write_str("credential record state is invalid"),
            Self::InvalidTombstoneShape => {
                formatter.write_str("credential tombstone shape is invalid")
            },
            Self::InvalidRefreshRetryGate => {
                formatter.write_str("credential refresh retry gate is invalid")
            },
            Self::MalformedMetadata => formatter.write_str("credential metadata is malformed"),
            Self::MetadataNotObject => formatter.write_str("credential metadata is not an object"),
            Self::DuplicateMetadataKey => {
                formatter.write_str("credential metadata contains a duplicate key")
            },
            Self::InvalidDisplay => formatter.write_str("credential display metadata is invalid"),
            Self::DisplayNameMismatch => {
                formatter.write_str("credential display-name projection differs")
            },
            Self::OrphanPhysicalName => {
                formatter.write_str("credential physical name has no display projection")
            },
            Self::DuplicateProjectedName => {
                formatter.write_str("credential display-name projection is duplicated")
            },
        }
    }
}

/// A reachable credential database whose schema cannot be safely migrated.
#[derive(Clone, PartialEq, Eq)]
pub struct UnsupportedSchemaVersion {
    reason: AdmissionReason,
}

impl UnsupportedSchemaVersion {
    fn new(reason: AdmissionReason) -> Self {
        Self { reason }
    }

    /// Return the closed, secret-free rejection reason.
    pub fn reason(&self) -> &AdmissionReason {
        &self.reason
    }
}

impl fmt::Display for UnsupportedSchemaVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unsupported credential schema: {}", self.reason)
    }
}

impl fmt::Debug for UnsupportedSchemaVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnsupportedSchemaVersion")
            .field("reason", &self.reason)
            .finish()
    }
}

impl std::error::Error for UnsupportedSchemaVersion {}

/// Failure to construct a credential store that is safe to serve.
#[derive(Clone, PartialEq, Eq)]
pub enum CredentialStoreStartupError {
    /// The database is reachable but its schema or legacy credential rows do
    /// not match a supported canonical state.
    UnsupportedSchemaVersion(UnsupportedSchemaVersion),
    /// The database, migration lock, or migration transaction was unavailable.
    Unavailable,
}

impl fmt::Display for CredentialStoreStartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion(error) => error.fmt(formatter),
            Self::Unavailable => formatter.write_str("credential store unavailable"),
        }
    }
}

impl fmt::Debug for CredentialStoreStartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion(error) => formatter
                .debug_tuple("UnsupportedSchemaVersion")
                .field(error.reason())
                .finish(),
            Self::Unavailable => formatter.write_str("Unavailable"),
        }
    }
}

impl std::error::Error for CredentialStoreStartupError {}

impl From<UnsupportedSchemaVersion> for CredentialStoreStartupError {
    fn from(error: UnsupportedSchemaVersion) -> Self {
        Self::UnsupportedSchemaVersion(error)
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn observation_fetch_error(
    error: sqlx::Error,
    invalid_relation: AdmissionReason,
) -> CredentialStoreStartupError {
    match error {
        // The query reached the backend and returned a row that cannot be
        // decoded through the exact canonical projection. This is evidence
        // about the reachable schema or its physical values.
        sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnIndexOutOfBounds { .. }
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::RowNotFound
        | sqlx::Error::TypeNotFound { .. } => {
            UnsupportedSchemaVersion::new(invalid_relation).into()
        },
        // Connectivity, protocol, pool, TLS, and operational database errors
        // do not establish a durable schema verdict.
        _ => CredentialStoreStartupError::Unavailable,
    }
}

#[cfg(test)]
impl From<CredentialStoreStartupError> for nebula_storage_port::CredentialPersistenceError {
    fn from(_: CredentialStoreStartupError) -> Self {
        Self::Unavailable
    }
}

pub(crate) fn classify_schema(
    policy: &BackendMigrationPolicy,
    observation: &SchemaObservation,
) -> Result<SchemaAdmission, UnsupportedSchemaVersion> {
    let rows = match &observation.migration_ledger {
        MigrationLedger::Absent => {
            if observation.has_user_relations || !observation.credentials.is_empty() {
                return rejected(AdmissionReason::UnledgeredDatabase);
            }
            return Ok(SchemaAdmission::Fresh);
        },
        MigrationLedger::Present(rows) if rows.is_empty() => {
            return rejected(AdmissionReason::EmptyLedger);
        },
        MigrationLedger::Present(rows) => rows,
    };

    validate_ledger(policy, rows)?;
    let latest = rows
        .last()
        .map(|row| row.version)
        .ok_or_else(|| UnsupportedSchemaVersion::new(AdmissionReason::EmptyLedger))?;
    if latest < policy.supported_floor {
        return rejected(AdmissionReason::BelowSupportedFloor { latest });
    }
    if !observation.has_credentials_relation {
        return rejected(AdmissionReason::MissingCredentialsRelation);
    }

    validate_credentials(&observation.credentials, latest)?;
    Ok(SchemaAdmission::CanonicalPrefix { latest })
}

fn validate_ledger(
    policy: &BackendMigrationPolicy,
    rows: &[MigrationLedgerRow],
) -> Result<(), UnsupportedSchemaVersion> {
    let mut observed_versions = HashSet::with_capacity(rows.len());
    for row in rows {
        if !observed_versions.insert(row.version) {
            return rejected(AdmissionReason::DuplicateMigration {
                migration: row.version,
            });
        }
    }

    for row in rows {
        if !row.success {
            return rejected(AdmissionReason::FailedMigration {
                migration: row.version,
            });
        }
        if policy
            .reserved_other_backend_versions
            .contains(&row.version)
        {
            return rejected(AdmissionReason::ReservedForOtherBackend {
                migration: row.version,
            });
        }
        if row.version > policy.current_version {
            return rejected(AdmissionReason::UnknownMigration {
                migration: row.version,
            });
        }
        let Some(canonical) = policy
            .canonical
            .iter()
            .find(|migration| migration.version == row.version)
        else {
            return rejected(AdmissionReason::UnknownMigration {
                migration: row.version,
            });
        };
        if row.description != canonical.description {
            return rejected(AdmissionReason::DescriptionMismatch {
                migration: row.version,
            });
        }
        if row.checksum != canonical.checksum {
            return rejected(AdmissionReason::ChecksumMismatch {
                migration: row.version,
            });
        }
    }

    for (index, row) in rows.iter().enumerate() {
        let Some(expected) = policy.canonical.get(index) else {
            return rejected(AdmissionReason::UnknownMigration {
                migration: row.version,
            });
        };
        if row.version != expected.version {
            return rejected(AdmissionReason::NonCanonicalOrder {
                expected: expected.version,
                actual: row.version,
            });
        }
    }

    Ok(())
}

fn validate_credentials(
    records: &[LegacyCredentialRecord],
    latest: i64,
) -> Result<(), UnsupportedSchemaVersion> {
    let mut projected_names = HashSet::new();

    for record in records {
        if record.owner_id.is_none() {
            return rejected(AdmissionReason::OwnerlessCredential);
        }
        if record.id.parse::<CredentialId>().is_err() {
            return rejected(AdmissionReason::InvalidCredentialId);
        }
        if !(0..=i64::from(u32::MAX)).contains(&record.state_version) {
            return rejected(AdmissionReason::InvalidStateVersion);
        }
        if record.version < 1 {
            return rejected(AdmissionReason::InvalidCredentialVersion);
        }
        if latest >= 40 {
            if record.material_epoch.is_none_or(|epoch| epoch < 1) {
                return rejected(AdmissionReason::InvalidMaterialEpoch);
            }
        } else if record.material_epoch.is_some() {
            return rejected(AdmissionReason::InvalidMaterialEpoch);
        }

        let metadata = parse_unique_json(&record.metadata)?;
        let Value::Object(metadata) = metadata else {
            return rejected(AdmissionReason::MetadataNotObject);
        };
        if latest >= 39 {
            validate_current_record(record, &metadata, &mut projected_names, latest >= 40)?;
        } else {
            validate_legacy_record(record, &metadata, &mut projected_names)?;
        }
    }

    Ok(())
}

fn validate_legacy_record(
    record: &LegacyCredentialRecord,
    metadata: &Map<String, Value>,
    projected_names: &mut HashSet<(String, String)>,
) -> Result<(), UnsupportedSchemaVersion> {
    if record.record_state.is_some() {
        return rejected(AdmissionReason::InvalidRecordState);
    }
    if metadata.contains_key("revoked_at") {
        return Ok(());
    }
    if record.version == i64::MAX {
        return rejected(AdmissionReason::LiveVersionExhausted);
    }
    validate_live_projection(record, metadata, projected_names)
}

fn validate_current_record(
    record: &LegacyCredentialRecord,
    metadata: &Map<String, Value>,
    projected_names: &mut HashSet<(String, String)>,
    has_refresh_retry_gate: bool,
) -> Result<(), UnsupportedSchemaVersion> {
    if has_refresh_retry_gate {
        validate_refresh_retry_gate(record)?;
    } else if refresh_retry_tuple_is_present(record) {
        return rejected(AdmissionReason::InvalidRefreshRetryGate);
    }
    match record.record_state.as_deref() {
        Some("live") => {
            if record.tombstoned_at_present {
                return rejected(AdmissionReason::InvalidRecordState);
            }
            if record.version == i64::MAX {
                return rejected(AdmissionReason::LiveVersionExhausted);
            }
            validate_live_projection(record, metadata, projected_names)
        },
        Some("tombstoned") => {
            if !record.tombstoned_at_present
                || record.data_len != 0
                || record.name.is_some()
                || record.expires_at_present
                || record.reauth_required
                || record.metadata != "{}"
                || !metadata.is_empty()
                || refresh_retry_tuple_is_present(record)
            {
                return rejected(AdmissionReason::InvalidTombstoneShape);
            }
            Ok(())
        },
        _ => rejected(AdmissionReason::InvalidRecordState),
    }
}

fn refresh_retry_tuple_is_present(record: &LegacyCredentialRecord) -> bool {
    record.refresh_retry_mode.is_some()
        || record.refresh_retry_not_before_present
        || record.refresh_retry_phase.is_some()
        || record.refresh_retry_kind.is_some()
        || record.refresh_retry_diagnostic_code.is_some()
}

fn validate_refresh_retry_gate(
    record: &LegacyCredentialRecord,
) -> Result<(), UnsupportedSchemaVersion> {
    let valid_phase = matches!(
        record.refresh_retry_phase.as_deref(),
        Some("before_dispatch" | "provider_confirmed_not_applied")
    );
    let valid_kind = matches!(
        record.refresh_retry_kind.as_deref(),
        Some("transient_network" | "provider_unavailable" | "protocol_error")
    );
    let valid_diagnostic = record
        .refresh_retry_diagnostic_code
        .as_deref()
        .is_none_or(|code| {
            !code.is_empty()
                && code.len() <= 64
                && code.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
                })
        });

    let valid = match record.refresh_retry_mode.as_deref() {
        None => !refresh_retry_tuple_is_present(record),
        Some("never") => {
            !record.refresh_retry_not_before_present
                && valid_phase
                && valid_kind
                && valid_diagnostic
        },
        Some("not_before") => {
            record.refresh_retry_not_before_present && valid_phase && valid_kind && valid_diagnostic
        },
        Some(_) => false,
    };
    if !valid {
        return rejected(AdmissionReason::InvalidRefreshRetryGate);
    }
    Ok(())
}

fn validate_live_projection(
    record: &LegacyCredentialRecord,
    metadata: &Map<String, Value>,
    projected_names: &mut HashSet<(String, String)>,
) -> Result<(), UnsupportedSchemaVersion> {
    let projected_name = validate_display(metadata)?;
    match (&record.name, projected_name.as_deref()) {
        (Some(physical), Some(projected)) if physical != projected => {
            return rejected(AdmissionReason::DisplayNameMismatch);
        },
        (Some(_), None) => return rejected(AdmissionReason::OrphanPhysicalName),
        _ => {},
    }

    if let Some(projected) = projected_name {
        let owner = record
            .owner_id
            .as_ref()
            .ok_or_else(|| UnsupportedSchemaVersion::new(AdmissionReason::OwnerlessCredential))?
            .clone();
        if !projected_names.insert((owner, projected)) {
            return rejected(AdmissionReason::DuplicateProjectedName);
        }
    }

    Ok(())
}

fn validate_display(
    metadata: &Map<String, Value>,
) -> Result<Option<String>, UnsupportedSchemaVersion> {
    let Some(display) = metadata.get("display") else {
        return Ok(None);
    };
    let Value::Object(display) = display else {
        return rejected(AdmissionReason::InvalidDisplay);
    };

    for (key, value) in display {
        match key.as_str() {
            "display_name" | "description" if value.is_null() || value.is_string() => {},
            "tags" => {
                let Value::Object(tags) = value else {
                    return rejected(AdmissionReason::InvalidDisplay);
                };
                if tags.values().any(|tag| !tag.is_string()) {
                    return rejected(AdmissionReason::InvalidDisplay);
                }
            },
            "display_name" | "description" => {
                return rejected(AdmissionReason::InvalidDisplay);
            },
            _ => {},
        }
    }

    Ok(display
        .get("display_name")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

enum UniqueJsonFailure {
    Malformed,
    DuplicateKey,
}

struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueValueVisitor)
    }
}

struct UniqueValueVisitor;

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = UniqueValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueValue)
            .ok_or_else(|| E::custom("invalid JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        UniqueValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or_default());
        while let Some(UniqueValue(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(UniqueValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = HashSet::with_capacity(object.size_hint().unwrap_or_default());
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(serde::de::Error::custom(DUPLICATE_KEY_MARKER));
            }
            let UniqueValue(value) = object.next_value()?;
            values.insert(key, value);
        }
        Ok(UniqueValue(Value::Object(values)))
    }
}

fn parse_unique_json(input: &str) -> Result<Value, UnsupportedSchemaVersion> {
    let mut deserializer = serde_json::Deserializer::from_str(input);
    let parsed = UniqueValue::deserialize(&mut deserializer).map_err(|error| {
        if error.to_string().contains(DUPLICATE_KEY_MARKER) {
            UniqueJsonFailure::DuplicateKey
        } else {
            UniqueJsonFailure::Malformed
        }
    });
    let parsed = match parsed {
        Ok(parsed) => parsed,
        Err(UniqueJsonFailure::DuplicateKey) => {
            return rejected(AdmissionReason::DuplicateMetadataKey);
        },
        Err(UniqueJsonFailure::Malformed) => {
            return rejected(AdmissionReason::MalformedMetadata);
        },
    };
    if deserializer.end().is_err() {
        return rejected(AdmissionReason::MalformedMetadata);
    }
    Ok(parsed.0)
}

fn rejected<T>(reason: AdmissionReason) -> Result<T, UnsupportedSchemaVersion> {
    Err(UnsupportedSchemaVersion::new(reason))
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
const SUPPORTED_FLOOR: i64 = 30;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
const CURRENT_VERSION: i64 = 40;

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn policy_from_migrator(
    backend: BackendKind,
    migrator: &sqlx::migrate::Migrator,
) -> BackendMigrationPolicy {
    let canonical = migrator
        .iter()
        .map(|migration| MigrationSpec {
            version: migration.version,
            description: migration.description.to_string(),
            checksum: migration.checksum.to_vec(),
        })
        .collect();
    let reserved_other_backend_versions = match backend {
        #[cfg(any(test, feature = "sqlite"))]
        BackendKind::Sqlite => BTreeSet::from([29, 36, 37, 38]),
        #[cfg(any(test, feature = "postgres"))]
        BackendKind::Postgres => BTreeSet::new(),
    };

    BackendMigrationPolicy {
        supported_floor: SUPPORTED_FLOOR,
        current_version: CURRENT_VERSION,
        canonical,
        reserved_other_backend_versions,
    }
}

#[cfg(feature = "sqlite")]
pub(crate) static SQLITE_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

#[cfg(feature = "sqlite")]
pub(crate) fn sqlite_policy() -> BackendMigrationPolicy {
    policy_from_migrator(BackendKind::Sqlite, &SQLITE_MIGRATOR)
}

#[cfg(feature = "postgres")]
pub(crate) static POSTGRES_MIGRATOR: sqlx::migrate::Migrator =
    sqlx::migrate!("./migrations/postgres");

#[cfg(feature = "postgres")]
pub(crate) fn postgres_policy() -> BackendMigrationPolicy {
    policy_from_migrator(BackendKind::Postgres, &POSTGRES_MIGRATOR)
}

#[cfg(feature = "postgres")]
pub(crate) fn unlocked_postgres_migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate::Migrator {
        migrations: POSTGRES_MIGRATOR.migrations.clone(),
        ignore_missing: POSTGRES_MIGRATOR.ignore_missing,
        locking: false,
        no_tx: POSTGRES_MIGRATOR.no_tx,
        table_name: POSTGRES_MIGRATOR.table_name.clone(),
        create_schemas: POSTGRES_MIGRATOR.create_schemas.clone(),
    }
}

#[cfg(feature = "sqlite")]
pub(crate) mod sqlite;

#[cfg(feature = "postgres")]
pub(crate) mod postgres;

#[cfg(test)]
mod tests;
