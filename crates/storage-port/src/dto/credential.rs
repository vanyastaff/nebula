//! Port-local credential persistence values.
//!
//! These types carry opaque encrypted state and tenant selectors across the
//! credential-controller → persistence boundary. They intentionally depend on
//! no credential-domain crate.

use std::fmt;
use std::ops::Deref;

use serde_json::Value;
use zeroize::Zeroizing;

use crate::Scope;

/// Canonical credential-owner partition.
///
/// This value is data, not authority. Possessing one grants no persistence
/// access. Trusted technical services, decorators, adapters, and composition
/// roots may retain a [`CredentialPersistence`](crate::CredentialPersistence)
/// handle, while supported API handlers and SDK consumers do not. Unlike the
/// retired optional owner resolver, an empty value is still an ordinary isolated
/// partition and never means administrator access.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CredentialOwner(String);

impl CredentialOwner {
    /// Derive the canonical credential owner from a resolved tenant scope.
    #[must_use]
    pub fn from_scope(scope: &Scope) -> Self {
        Self(scope.credential_owner_id())
    }

    /// Wrap an already-canonical owner partition.
    ///
    /// This constructor exists for durable system records and compatibility
    /// reads. The value never confers authority by itself.
    #[must_use]
    pub fn from_canonical(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the canonical persistence key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CredentialOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialOwner([redacted])")
    }
}

/// Owner-bound selector for one credential row.
///
/// Persistence adapters must include both fields in every read, delete, and
/// compare-and-swap predicate. Wrong-owner and missing rows therefore share
/// the same not-found result.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CredentialSelector {
    owner: CredentialOwner,
    credential_id: String,
}

impl CredentialSelector {
    /// Bind a credential id to one owner partition.
    #[must_use]
    pub fn new(owner: CredentialOwner, credential_id: impl Into<String>) -> Self {
        Self {
            owner,
            credential_id: credential_id.into(),
        }
    }

    /// Borrow the owner partition.
    #[must_use]
    pub fn owner(&self) -> &CredentialOwner {
        &self.owner
    }

    /// Borrow the credential id.
    #[must_use]
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }
}

impl fmt::Debug for CredentialSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialSelector")
            .field("credential_id", &self.credential_id)
            .field("owner", &"[redacted]")
            .finish()
    }
}

/// Conflict policy for a credential write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialWriteMode {
    /// Insert only when the id is unused in this owner partition.
    CreateOnly,
    /// Replace the current row without a version predicate.
    Overwrite,
    /// Replace only the version the caller observed.
    CompareAndSwap {
        /// Expected persisted version.
        expected_version: u64,
    },
}

/// Zeroizing opaque credential-state bytes.
///
/// Both plaintext above the encryption decorator and ciphertext below it use
/// this wrapper, so every owned intermediate buffer is cleared on drop. Raw
/// bytes are borrowable but cannot be extracted into a non-zeroizing `Vec`
/// through the public API.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    /// Wrap owned bytes so they are zeroized on drop.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }
}

impl From<Vec<u8>> for SecretBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl From<Zeroizing<Vec<u8>>> for SecretBytes {
    fn from(bytes: Zeroizing<Vec<u8>>) -> Self {
        Self(bytes)
    }
}

impl Deref for SecretBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0.as_slice()
    }
}

impl AsRef<[u8]> for SecretBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl<const N: usize> PartialEq<[u8; N]> for SecretBytes {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.as_ref() == other
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for SecretBytes {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self.as_ref() == *other
    }
}

impl PartialEq<Vec<u8>> for SecretBytes {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.as_ref() == other.as_slice()
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SecretBytes([redacted; {} bytes])", self.len())
    }
}

/// Opaque credential row carried by the storage port.
///
/// `data` is ciphertext at a backend boundary and plaintext only inside the
/// credential service/resolver side of the encryption decorator. The controller
/// and HTTP layer never receive this row. The type has no serde implementation,
/// and its manual `Debug` omits both bytes and metadata values.
#[derive(Clone)]
pub struct StoredCredential {
    /// Credential identifier.
    pub id: String,
    /// Optional owner-local display name.
    pub name: Option<String>,
    /// Registered credential type key.
    pub credential_key: String,
    /// Opaque state bytes.
    pub data: SecretBytes,
    /// State type identifier.
    pub state_kind: String,
    /// State schema version.
    pub state_version: u32,
    /// Monotonic persistence version.
    pub version: u64,
    /// Creation instant.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last mutation instant.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Optional material expiry.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether interactive re-authentication is required.
    pub reauth_required: bool,
    /// Opaque metadata. Values are never rendered by `Debug`.
    pub metadata: serde_json::Map<String, Value>,
}

impl fmt::Debug for StoredCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredCredential")
            .field("id", &self.id)
            .field("name_present", &self.name.is_some())
            .field("credential_key", &self.credential_key)
            .field(
                "data",
                &format_args!("[redacted; {} bytes]", self.data.len()),
            )
            .field("state_kind", &self.state_kind)
            .field("state_version", &self.state_version)
            .field("version", &self.version)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("expires_at", &self.expires_at)
            .field("reauth_required", &self.reauth_required)
            .field("metadata_key_count", &self.metadata.len())
            .finish()
    }
}

impl StoredCredential {
    /// Whether the row carries a terminal revocation tombstone.
    #[must_use]
    pub fn is_tombstoned(&self) -> bool {
        self.metadata.contains_key(REVOKED_AT_METADATA_KEY)
    }

    /// Parse the revocation epoch when it is present and well formed.
    #[must_use]
    pub fn revoked_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata
            .get(REVOKED_AT_METADATA_KEY)
            .and_then(Value::as_str)
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|instant| instant.with_timezone(&chrono::Utc))
    }

    /// Return the last provider-validation instant, when stamped.
    #[must_use]
    pub fn last_validated_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata
            .get(LAST_VALIDATED_AT_METADATA_KEY)
            .and_then(Value::as_str)
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|instant| instant.with_timezone(&chrono::Utc))
    }

    /// Return the validation instant, falling back to creation for legacy rows.
    #[must_use]
    pub fn last_validated_or_created(&self) -> chrono::DateTime<chrono::Utc> {
        self.last_validated_at().unwrap_or(self.created_at)
    }

    /// Stamp a successful provider validation.
    pub fn stamp_validated(&mut self, at: chrono::DateTime<chrono::Utc>) {
        self.metadata.insert(
            LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
            Value::String(at.to_rfc3339()),
        );
    }
}

/// Secret-free persisted credential projection for management reads.
///
/// Backends build this value without selecting `data`; encryption decorators
/// delegate it unchanged. Consequently `get_head` and `list_heads` cannot
/// decrypt or retain credential material as an implementation detail.
#[derive(Clone)]
pub struct StoredCredentialHead {
    /// Credential identifier.
    pub id: String,
    /// Optional owner-local display name.
    pub name: Option<String>,
    /// Registered credential type key.
    pub credential_key: String,
    /// State type identifier, used only for non-secret filtering.
    pub state_kind: String,
    /// State schema version.
    pub state_version: u32,
    /// Monotonic persistence version.
    pub version: u64,
    /// Creation instant.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last mutation instant.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Optional material expiry.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether interactive re-authentication is required.
    pub reauth_required: bool,
    /// Opaque non-secret metadata; values are omitted from `Debug`.
    pub metadata: serde_json::Map<String, Value>,
}

impl fmt::Debug for StoredCredentialHead {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredCredentialHead")
            .field("id", &self.id)
            .field("name_present", &self.name.is_some())
            .field("credential_key", &self.credential_key)
            .field("state_kind", &self.state_kind)
            .field("state_version", &self.state_version)
            .field("version", &self.version)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("expires_at", &self.expires_at)
            .field("reauth_required", &self.reauth_required)
            .field("metadata_key_count", &self.metadata.len())
            .finish()
    }
}

impl From<&StoredCredential> for StoredCredentialHead {
    fn from(stored: &StoredCredential) -> Self {
        Self {
            id: stored.id.clone(),
            name: stored.name.clone(),
            credential_key: stored.credential_key.clone(),
            state_kind: stored.state_kind.clone(),
            state_version: stored.state_version,
            version: stored.version,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
            expires_at: stored.expires_at,
            reauth_required: stored.reauth_required,
            metadata: stored.metadata.clone(),
        }
    }
}

impl StoredCredentialHead {
    /// Whether the projection carries a terminal revocation tombstone.
    #[must_use]
    pub fn is_tombstoned(&self) -> bool {
        self.metadata.contains_key(REVOKED_AT_METADATA_KEY)
    }

    /// Return the last provider-validation instant, when stamped.
    #[must_use]
    pub fn last_validated_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata
            .get(LAST_VALIDATED_AT_METADATA_KEY)
            .and_then(Value::as_str)
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|instant| instant.with_timezone(&chrono::Utc))
    }
}

/// Reserved metadata key for the canonical owner partition.
///
/// New adapters derive their SQL owner column from [`CredentialSelector`]
/// and overwrite this defense-in-depth stamp on write; callers never choose it.
pub const OWNER_ID_METADATA_KEY: &str = "owner_id";

/// Reserved metadata key for the terminal revocation epoch.
pub const REVOKED_AT_METADATA_KEY: &str = "revoked_at";

/// Reserved metadata key for the last provider-validation instant.
pub const LAST_VALIDATED_AT_METADATA_KEY: &str = "last_validated_at";

#[cfg(test)]
mod tests {
    use super::*;

    fn row_with(metadata: serde_json::Map<String, Value>) -> StoredCredential {
        StoredCredential {
            id: "cred_x".to_owned(),
            name: None,
            credential_key: "github_oauth".to_owned(),
            data: vec![1, 2, 3].into(),
            state_kind: "oauth2_state".to_owned(),
            state_version: 1,
            version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            reauth_required: false,
            metadata,
        }
    }

    #[test]
    fn debug_redacts_state_and_metadata_values() {
        let secret = "STORAGE-PORT-SECRET-CANARY-9b3f";
        let mut metadata = serde_json::Map::new();
        metadata.insert("secret".to_owned(), Value::String(secret.to_owned()));
        let mut row = row_with(metadata);
        row.name = Some(secret.to_owned());
        row.data = secret.as_bytes().to_vec().into();

        let rendered = format!("{row:?}");
        assert!(!rendered.contains(secret));
        assert!(rendered.contains("[redacted;"));
        assert!(rendered.contains("metadata_key_count"));
    }

    #[test]
    fn live_row_is_not_tombstoned() {
        let row = row_with(serde_json::Map::new());
        assert!(!row.is_tombstoned());
        assert!(row.revoked_at().is_none());
    }

    #[test]
    fn well_formed_epoch_parses() {
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            REVOKED_AT_METADATA_KEY.to_owned(),
            Value::String("2026-06-13T10:00:00Z".to_owned()),
        );
        let row = row_with(metadata);
        assert!(row.is_tombstoned());
        assert!(row.revoked_at().is_some());
    }

    #[test]
    fn malformed_epoch_still_reads_as_tombstoned() {
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            REVOKED_AT_METADATA_KEY.to_owned(),
            Value::String("not-a-timestamp".to_owned()),
        );
        let row = row_with(metadata);
        assert!(row.is_tombstoned());
        assert!(row.revoked_at().is_none());
    }

    #[test]
    fn last_validated_falls_back_to_created_when_absent() {
        let row = row_with(serde_json::Map::new());
        assert!(row.last_validated_at().is_none());
        assert_eq!(row.last_validated_or_created(), row.created_at);
    }

    #[test]
    fn display_edit_does_not_postpone_the_validation_time() {
        let validated = chrono::Utc::now() - chrono::Duration::days(30);
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
            Value::String(validated.to_rfc3339()),
        );
        let mut row = row_with(metadata);
        row.updated_at = chrono::Utc::now();

        let resolved = row.last_validated_or_created();
        assert!(resolved < row.updated_at);
        assert!((resolved - validated).num_seconds().abs() <= 1);
    }

    #[test]
    fn empty_owner_is_not_an_admin_sentinel() {
        let empty = CredentialOwner::from_canonical("");
        let other = CredentialOwner::from_canonical("tenant");
        assert_ne!(empty, other);
        assert_eq!(empty.as_str(), "");
    }
}
