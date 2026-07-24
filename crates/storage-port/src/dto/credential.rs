//! Port-local credential persistence values.
//!
//! These types carry opaque encrypted state and owner-qualified selectors
//! across the credential-controller → persistence boundary. They intentionally
//! depend on no credential-domain crate.

use std::fmt;
use std::ops::Deref;

use nebula_core::CredentialId;
use serde_json::{Map, Value};
use zeroize::Zeroizing;

use crate::Scope;
use crate::dto::RefreshRetryGate;
use crate::dto::RefreshRetryTransition;
use crate::store::CredentialPersistenceError;

/// Canonical credential-owner partition.
///
/// This value is data, not authority. Possessing one grants no persistence
/// access. Trusted technical services, adapters, and composition roots may
/// retain a [`CredentialPersistence`](crate::CredentialPersistence) handle,
/// while supported API handlers and SDK consumers do not.
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
    /// This constructor exists for trusted durable system records. The value
    /// never confers actor authority by itself.
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

/// Owner-bound selector for one globally unique credential.
///
/// Persistence adapters include both values in every row predicate.
/// Wrong-owner and missing rows therefore share the same not-found result.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CredentialSelector {
    owner: CredentialOwner,
    credential_id: CredentialId,
}

impl CredentialSelector {
    /// Bind a server-generated credential id to one owner partition.
    #[must_use]
    pub fn new(owner: CredentialOwner, credential_id: CredentialId) -> Self {
        Self {
            owner,
            credential_id,
        }
    }

    /// Borrow the owner partition.
    #[must_use]
    pub fn owner(&self) -> &CredentialOwner {
        &self.owner
    }

    /// Return the typed credential id.
    #[must_use]
    pub fn credential_id(&self) -> CredentialId {
        self.credential_id
    }
}

impl fmt::Debug for CredentialSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialSelector([redacted])")
    }
}

/// Validated persisted credential version.
///
/// The database representation is a signed 64-bit integer, but zero and
/// negative values are invalid. The terminal value is reserved for a
/// tombstone so every live row retains one final transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CredentialVersion(i64);

impl CredentialVersion {
    /// First valid persisted version.
    pub const MIN: Self = Self(1);

    /// Last version a live record may consume.
    pub const MAX_LIVE: Self = Self(i64::MAX - 1);

    /// Last representable version, reserved for terminal state.
    pub const MAX: Self = Self(i64::MAX);

    /// Return the database representation.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Whether this version is admissible on a live record.
    #[must_use]
    pub const fn is_live(self) -> bool {
        self.0 <= Self::MAX_LIVE.0
    }

    /// Advance a live record while preserving terminal tombstone headroom.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialPersistenceError::VersionExhausted`] when the next
    /// value would consume the terminal version.
    pub const fn next_live(self) -> Result<Self, CredentialPersistenceError> {
        if self.0 >= Self::MAX_LIVE.0 {
            return Err(CredentialPersistenceError::VersionExhausted);
        }
        Ok(Self(self.0 + 1))
    }

    /// Advance a live record into a tombstone.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialPersistenceError::VersionExhausted`] at the
    /// terminal value. Callers classify terminal records as not found before
    /// attempting this advance.
    pub const fn next_tombstone(self) -> Result<Self, CredentialPersistenceError> {
        if self.0 == Self::MAX.0 {
            return Err(CredentialPersistenceError::VersionExhausted);
        }
        Ok(Self(self.0 + 1))
    }
}

impl fmt::Display for CredentialVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Persisted credential version lies outside `1..=i64::MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("credential version is outside the supported range")]
pub struct CredentialVersionError;

impl TryFrom<i64> for CredentialVersion {
    type Error = CredentialVersionError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value < Self::MIN.0 {
            return Err(CredentialVersionError);
        }
        Ok(Self(value))
    }
}

impl TryFrom<u64> for CredentialVersion {
    type Error = CredentialVersionError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        let value = i64::try_from(value).map_err(|_| CredentialVersionError)?;
        Self::try_from(value)
    }
}

/// Monotonic epoch for credential material and refresh authority.
///
/// Unlike [`CredentialVersion`], which advances for every aggregate mutation,
/// this value advances only when new credential material or an equivalent
/// reconnect/reauthentication transition establishes new refresh authority.
/// Display-only mutations and retry-gate finalization preserve it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CredentialMaterialEpoch(i64);

impl CredentialMaterialEpoch {
    /// First backend-authored material epoch.
    pub const MIN: Self = Self(1);

    /// Last representable material epoch.
    pub const MAX: Self = Self(i64::MAX);

    /// Return the database representation.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Advance after an explicit material-authority transition.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialPersistenceError::MaterialEpochExhausted`] at the
    /// terminal representable epoch.
    pub const fn next(self) -> Result<Self, CredentialPersistenceError> {
        if self.0 == Self::MAX.0 {
            return Err(CredentialPersistenceError::MaterialEpochExhausted);
        }
        Ok(Self(self.0 + 1))
    }
}

impl fmt::Display for CredentialMaterialEpoch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Persisted credential material epoch lies outside `1..=i64::MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("credential material epoch is outside the supported range")]
pub struct CredentialMaterialEpochError;

impl TryFrom<i64> for CredentialMaterialEpoch {
    type Error = CredentialMaterialEpochError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value < Self::MIN.0 {
            return Err(CredentialMaterialEpochError);
        }
        Ok(Self(value))
    }
}

impl TryFrom<u64> for CredentialMaterialEpoch {
    type Error = CredentialMaterialEpochError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        let value = i64::try_from(value).map_err(|_| CredentialMaterialEpochError)?;
        Self::try_from(value)
    }
}

/// Explicit refresh-authority intent carried by a replacement.
///
/// The shape makes an advanced material epoch with a retained or newly
/// installed old-epoch retry gate unrepresentable: [`Self::Advance`] always
/// clears the gate, while [`Self::Preserve`] carries the only admissible gate
/// transition for the current epoch. Callers choose only this intent; adapters
/// own the actual epoch value, initialize it at [`CredentialMaterialEpoch::MIN`],
/// and fail closed with
/// [`CredentialPersistenceError::MaterialEpochExhausted`] rather than wrapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialMaterialTransition {
    /// Preserve refresh authority while applying a current-epoch retry-gate transition.
    Preserve {
        /// Structural retry-gate transition for the unchanged material epoch.
        refresh_retry: RefreshRetryTransition,
    },
    /// Advance refresh authority and unconditionally clear the prior epoch's gate.
    Advance,
}

impl CredentialMaterialTransition {
    /// Preserve the current epoch with an explicit gate transition.
    #[must_use]
    pub const fn preserve(refresh_retry: RefreshRetryTransition) -> Self {
        Self::Preserve { refresh_retry }
    }

    /// Advance the material epoch and clear any prior gate.
    #[must_use]
    pub const fn advance() -> Self {
        Self::Advance
    }

    /// Whether the replacement establishes new refresh authority.
    #[must_use]
    pub const fn advances_epoch(&self) -> bool {
        matches!(self, Self::Advance)
    }

    /// Borrow the current-epoch gate transition, if authority is preserved.
    #[must_use]
    pub const fn refresh_retry_transition(&self) -> Option<&RefreshRetryTransition> {
        match self {
            Self::Preserve { refresh_retry } => Some(refresh_retry),
            Self::Advance => None,
        }
    }
}

/// Zeroizing opaque credential-state bytes.
///
/// Both plaintext above an encryption decorator and ciphertext below it use
/// this wrapper. Raw bytes are borrowable but cannot be extracted into a
/// non-zeroizing `Vec` through the public API.
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
        formatter.write_str("SecretBytes([redacted])")
    }
}

/// Complete live value for a new credential.
///
/// Identity, owner, version, and timestamps are deliberately absent. The
/// trusted controller supplies identity through [`CredentialSelector`], while
/// the backend assigns version `1` and commit timestamps.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialCreate {
    credential_key: String,
    data: SecretBytes,
    state_kind: String,
    state_version: u32,
    name: Option<String>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    reauth_required: bool,
    metadata: Map<String, Value>,
}

impl CredentialCreate {
    /// Construct a complete live create value.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor mirrors the closed persistence record and keeps every field explicit"
    )]
    pub fn new(
        credential_key: String,
        data: SecretBytes,
        state_kind: String,
        state_version: u32,
        name: Option<String>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
        reauth_required: bool,
        metadata: Map<String, Value>,
    ) -> Self {
        Self {
            credential_key,
            data,
            state_kind,
            state_version,
            name,
            expires_at,
            reauth_required,
            metadata,
        }
    }

    /// Borrow the immutable registered credential key.
    #[must_use]
    pub fn credential_key(&self) -> &str {
        &self.credential_key
    }

    /// Borrow the opaque live state bytes.
    #[must_use]
    pub fn data(&self) -> &SecretBytes {
        &self.data
    }

    /// Borrow the state type identifier.
    #[must_use]
    pub fn state_kind(&self) -> &str {
        &self.state_kind
    }

    /// Return the state schema version.
    #[must_use]
    pub fn state_version(&self) -> u32 {
        self.state_version
    }

    /// Borrow the optional owner-local display-name projection.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Return the optional material expiry.
    #[must_use]
    pub fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.expires_at
    }

    /// Whether interactive re-authentication is required.
    #[must_use]
    pub fn reauth_required(&self) -> bool {
        self.reauth_required
    }

    /// Borrow the opaque live metadata.
    #[must_use]
    pub fn metadata(&self) -> &Map<String, Value> {
        &self.metadata
    }
}

impl fmt::Debug for CredentialCreate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialCreate([redacted])")
    }
}

/// Version-fenced replacement of mutable live credential state.
///
/// Identity, owner, registered credential key, and creation time are absent
/// and therefore cannot be changed through replacement.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialReplacement {
    expected_version: CredentialVersion,
    data: SecretBytes,
    state_kind: String,
    state_version: u32,
    name: Option<String>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    reauth_required: bool,
    metadata: Map<String, Value>,
    material_transition: CredentialMaterialTransition,
}

impl CredentialReplacement {
    /// Construct a complete version-fenced replacement value.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor mirrors the closed mutable record and keeps every field explicit"
    )]
    pub fn new(
        expected_version: CredentialVersion,
        data: SecretBytes,
        state_kind: String,
        state_version: u32,
        name: Option<String>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
        reauth_required: bool,
        metadata: Map<String, Value>,
        material_transition: CredentialMaterialTransition,
    ) -> Self {
        Self {
            expected_version,
            data,
            state_kind,
            state_version,
            name,
            expires_at,
            reauth_required,
            metadata,
            material_transition,
        }
    }

    /// Return the version this replacement observed.
    #[must_use]
    pub fn expected_version(&self) -> CredentialVersion {
        self.expected_version
    }

    /// Borrow the replacement state bytes.
    #[must_use]
    pub fn data(&self) -> &SecretBytes {
        &self.data
    }

    /// Borrow the replacement state type identifier.
    #[must_use]
    pub fn state_kind(&self) -> &str {
        &self.state_kind
    }

    /// Return the replacement state schema version.
    #[must_use]
    pub fn state_version(&self) -> u32 {
        self.state_version
    }

    /// Borrow the replacement display-name projection.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Return the replacement material expiry.
    #[must_use]
    pub fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.expires_at
    }

    /// Whether the replacement requires interactive re-authentication.
    #[must_use]
    pub fn reauth_required(&self) -> bool {
        self.reauth_required
    }

    /// Borrow the replacement metadata.
    #[must_use]
    pub fn metadata(&self) -> &Map<String, Value> {
        &self.metadata
    }

    /// Borrow the explicit material-authority transition.
    #[must_use]
    pub const fn material_transition(&self) -> &CredentialMaterialTransition {
        &self.material_transition
    }
}

impl fmt::Debug for CredentialReplacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialReplacement([redacted])")
    }
}

/// Version-fenced transition from live state to a terminal tombstone.
///
/// The backend supplies the tombstone timestamp and clears every live-only
/// value atomically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialTombstone {
    expected_version: CredentialVersion,
}

impl CredentialTombstone {
    /// Construct a tombstone command for an observed live version.
    #[must_use]
    pub const fn new(expected_version: CredentialVersion) -> Self {
        Self { expected_version }
    }

    /// Return the version this command observed.
    #[must_use]
    pub const fn expected_version(self) -> CredentialVersion {
        self.expected_version
    }
}

/// Structural lifecycle state returned by persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CredentialRecordState {
    /// The credential carries a live value.
    Live,
    /// The credential id is permanently reserved without a live value.
    Tombstoned,
}

/// Physical credential record.
///
/// The tombstone variant has a distinct payload, making secret bytes, name,
/// expiry, re-authentication, and metadata unrepresentable in terminal state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCredential {
    /// Live record carrying opaque credential state.
    Live(StoredLiveCredential),
    /// Terminal record that permanently reserves the id.
    Tombstoned(StoredTombstonedCredential),
}

impl StoredCredential {
    /// Return the structural lifecycle state.
    #[must_use]
    pub const fn state(&self) -> CredentialRecordState {
        match self {
            Self::Live(_) => CredentialRecordState::Live,
            Self::Tombstoned(_) => CredentialRecordState::Tombstoned,
        }
    }

    /// Return the typed credential id.
    #[must_use]
    pub fn credential_id(&self) -> CredentialId {
        match self {
            Self::Live(record) => record.credential_id(),
            Self::Tombstoned(record) => record.credential_id(),
        }
    }

    /// Return the persisted version.
    #[must_use]
    pub fn version(&self) -> CredentialVersion {
        match self {
            Self::Live(record) => record.version(),
            Self::Tombstoned(record) => record.version(),
        }
    }

    /// Borrow the live record, if this record is live.
    #[must_use]
    pub const fn as_live(&self) -> Option<&StoredLiveCredential> {
        match self {
            Self::Live(record) => Some(record),
            Self::Tombstoned(_) => None,
        }
    }

    /// Borrow the tombstone record, if this record is terminal.
    #[must_use]
    pub const fn as_tombstoned(&self) -> Option<&StoredTombstonedCredential> {
        match self {
            Self::Live(_) => None,
            Self::Tombstoned(record) => Some(record),
        }
    }
}

impl From<StoredLiveCredential> for StoredCredential {
    fn from(record: StoredLiveCredential) -> Self {
        Self::Live(record)
    }
}

impl From<StoredTombstonedCredential> for StoredCredential {
    fn from(record: StoredTombstonedCredential) -> Self {
        Self::Tombstoned(record)
    }
}

/// Live physical credential record.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredLiveCredential {
    credential_id: CredentialId,
    name: Option<String>,
    credential_key: String,
    data: SecretBytes,
    state_kind: String,
    state_version: u32,
    version: CredentialVersion,
    material_epoch: CredentialMaterialEpoch,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    reauth_required: bool,
    metadata: Map<String, Value>,
    refresh_retry_gate: Option<RefreshRetryGate>,
}

impl StoredLiveCredential {
    /// Construct a live record from an adapter row.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialPersistenceError::CorruptRecord`] if the record
    /// consumes the version reserved for a terminal tombstone.
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor is the explicit validation boundary for a physical database row"
    )]
    pub fn new(
        credential_id: CredentialId,
        name: Option<String>,
        credential_key: String,
        data: SecretBytes,
        state_kind: String,
        state_version: u32,
        version: CredentialVersion,
        material_epoch: CredentialMaterialEpoch,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
        reauth_required: bool,
        metadata: Map<String, Value>,
        refresh_retry_gate: Option<RefreshRetryGate>,
    ) -> Result<Self, CredentialPersistenceError> {
        if !version.is_live() {
            return Err(CredentialPersistenceError::CorruptRecord);
        }
        Ok(Self {
            credential_id,
            name,
            credential_key,
            data,
            state_kind,
            state_version,
            version,
            material_epoch,
            created_at,
            updated_at,
            expires_at,
            reauth_required,
            metadata,
            refresh_retry_gate,
        })
    }

    /// Return the typed credential id.
    #[must_use]
    pub fn credential_id(&self) -> CredentialId {
        self.credential_id
    }

    /// Borrow the optional owner-local display-name projection.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Borrow the immutable registered credential key.
    #[must_use]
    pub fn credential_key(&self) -> &str {
        &self.credential_key
    }

    /// Borrow the opaque state bytes.
    #[must_use]
    pub fn data(&self) -> &SecretBytes {
        &self.data
    }

    /// Borrow the state type identifier.
    #[must_use]
    pub fn state_kind(&self) -> &str {
        &self.state_kind
    }

    /// Return the state schema version.
    #[must_use]
    pub fn state_version(&self) -> u32 {
        self.state_version
    }

    /// Return the persisted version.
    #[must_use]
    pub fn version(&self) -> CredentialVersion {
        self.version
    }

    /// Return the monotonic material/refresh-authority epoch.
    #[must_use]
    pub const fn material_epoch(&self) -> CredentialMaterialEpoch {
        self.material_epoch
    }

    /// Return the creation instant.
    #[must_use]
    pub fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.created_at
    }

    /// Return the last mutation instant.
    #[must_use]
    pub fn updated_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.updated_at
    }

    /// Return the optional material expiry.
    #[must_use]
    pub fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.expires_at
    }

    /// Whether interactive re-authentication is required.
    #[must_use]
    pub fn reauth_required(&self) -> bool {
        self.reauth_required
    }

    /// Borrow the opaque metadata.
    #[must_use]
    pub fn metadata(&self) -> &Map<String, Value> {
        &self.metadata
    }

    /// Borrow the structural refresh-retry gate, if one is installed.
    ///
    /// This state is never projected into user metadata.
    #[must_use]
    pub const fn refresh_retry_gate(&self) -> Option<&RefreshRetryGate> {
        self.refresh_retry_gate.as_ref()
    }
}

impl fmt::Debug for StoredLiveCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StoredLiveCredential([redacted])")
    }
}

/// Terminal physical credential record.
///
/// This type deliberately has no data, name, expiry, re-authentication, or
/// metadata field.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredTombstonedCredential {
    credential_id: CredentialId,
    credential_key: String,
    state_kind: String,
    state_version: u32,
    version: CredentialVersion,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    tombstoned_at: chrono::DateTime<chrono::Utc>,
}

impl StoredTombstonedCredential {
    /// Construct a terminal record from an adapter row.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor mirrors the complete secret-free terminal database row"
    )]
    pub fn new(
        credential_id: CredentialId,
        credential_key: String,
        state_kind: String,
        state_version: u32,
        version: CredentialVersion,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
        tombstoned_at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            credential_id,
            credential_key,
            state_kind,
            state_version,
            version,
            created_at,
            updated_at,
            tombstoned_at,
        }
    }

    /// Return the typed credential id.
    #[must_use]
    pub fn credential_id(&self) -> CredentialId {
        self.credential_id
    }

    /// Borrow the immutable registered credential key.
    #[must_use]
    pub fn credential_key(&self) -> &str {
        &self.credential_key
    }

    /// Borrow the retained state type identifier.
    #[must_use]
    pub fn state_kind(&self) -> &str {
        &self.state_kind
    }

    /// Return the retained state schema version.
    #[must_use]
    pub fn state_version(&self) -> u32 {
        self.state_version
    }

    /// Return the terminal persisted version.
    #[must_use]
    pub fn version(&self) -> CredentialVersion {
        self.version
    }

    /// Return the original creation instant.
    #[must_use]
    pub fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.created_at
    }

    /// Return the last mutation instant.
    #[must_use]
    pub fn updated_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.updated_at
    }

    /// Return the terminal transition instant.
    #[must_use]
    pub fn tombstoned_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.tombstoned_at
    }
}

impl fmt::Debug for StoredTombstonedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StoredTombstonedCredential([redacted])")
    }
}

/// Secret-free persisted live projection for management reads.
///
/// Backends build this value without selecting `data`; consequently
/// `get_head` and `list_heads` cannot decrypt credential material as an
/// implementation detail.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredCredentialHead {
    credential_id: CredentialId,
    name: Option<String>,
    credential_key: String,
    state_kind: String,
    state_version: u32,
    version: CredentialVersion,
    material_epoch: CredentialMaterialEpoch,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    reauth_required: bool,
    metadata: Map<String, Value>,
}

impl StoredCredentialHead {
    /// Construct a live secret-free projection from an adapter row.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialPersistenceError::CorruptRecord`] if the projection
    /// consumes the version reserved for a terminal tombstone.
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor is the explicit validation boundary for a secret-free database projection"
    )]
    pub fn new(
        credential_id: CredentialId,
        name: Option<String>,
        credential_key: String,
        state_kind: String,
        state_version: u32,
        version: CredentialVersion,
        material_epoch: CredentialMaterialEpoch,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
        reauth_required: bool,
        metadata: Map<String, Value>,
    ) -> Result<Self, CredentialPersistenceError> {
        if !version.is_live() {
            return Err(CredentialPersistenceError::CorruptRecord);
        }
        Ok(Self {
            credential_id,
            name,
            credential_key,
            state_kind,
            state_version,
            version,
            material_epoch,
            created_at,
            updated_at,
            expires_at,
            reauth_required,
            metadata,
        })
    }

    /// Return the typed credential id.
    #[must_use]
    pub fn credential_id(&self) -> CredentialId {
        self.credential_id
    }

    /// Borrow the optional owner-local display-name projection.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Borrow the immutable registered credential key.
    #[must_use]
    pub fn credential_key(&self) -> &str {
        &self.credential_key
    }

    /// Borrow the state type identifier.
    #[must_use]
    pub fn state_kind(&self) -> &str {
        &self.state_kind
    }

    /// Return the state schema version.
    #[must_use]
    pub fn state_version(&self) -> u32 {
        self.state_version
    }

    /// Return the persisted version.
    #[must_use]
    pub fn version(&self) -> CredentialVersion {
        self.version
    }

    /// Return the monotonic material/refresh-authority epoch.
    #[must_use]
    pub const fn material_epoch(&self) -> CredentialMaterialEpoch {
        self.material_epoch
    }

    /// Return the creation instant.
    #[must_use]
    pub fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.created_at
    }

    /// Return the last mutation instant.
    #[must_use]
    pub fn updated_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.updated_at
    }

    /// Return the optional material expiry.
    #[must_use]
    pub fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.expires_at
    }

    /// Whether interactive re-authentication is required.
    #[must_use]
    pub fn reauth_required(&self) -> bool {
        self.reauth_required
    }

    /// Borrow the opaque non-secret metadata.
    #[must_use]
    pub fn metadata(&self) -> &Map<String, Value> {
        &self.metadata
    }
}

impl From<&StoredLiveCredential> for StoredCredentialHead {
    fn from(stored: &StoredLiveCredential) -> Self {
        Self {
            credential_id: stored.credential_id,
            name: stored.name.clone(),
            credential_key: stored.credential_key.clone(),
            state_kind: stored.state_kind.clone(),
            state_version: stored.state_version,
            version: stored.version,
            material_epoch: stored.material_epoch,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
            expires_at: stored.expires_at,
            reauth_required: stored.reauth_required,
            metadata: stored.metadata.clone(),
        }
    }
}

impl fmt::Debug for StoredCredentialHead {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredCredentialHead")
            .field("credential_id", &"[redacted]")
            .field("name_present", &self.name.is_some())
            .field("credential_key", &"[redacted]")
            .field("state_kind", &"[redacted]")
            .field("state_version", &self.state_version)
            .field("version", &self.version)
            .field("material_epoch", &self.material_epoch)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("expires_at", &self.expires_at)
            .field("reauth_required", &self.reauth_required)
            .field("metadata_key_count", &self.metadata.len())
            .finish()
    }
}

/// Secret-free projection of one confirmed credential mutation.
///
/// Adapters construct this from the modifying statement's `RETURNING`
/// projection and release it only after commit acknowledgement.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CredentialCommit {
    credential_id: CredentialId,
    version: CredentialVersion,
    state: CredentialRecordState,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    tombstoned_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl CredentialCommit {
    /// Construct a confirmed live commit.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialPersistenceError::CorruptRecord`] if a backend
    /// reports the terminal version for a live mutation.
    pub fn live(
        credential_id: CredentialId,
        version: CredentialVersion,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Self, CredentialPersistenceError> {
        if !version.is_live() {
            return Err(CredentialPersistenceError::CorruptRecord);
        }
        Ok(Self {
            credential_id,
            version,
            state: CredentialRecordState::Live,
            created_at,
            updated_at,
            tombstoned_at: None,
        })
    }

    /// Construct a confirmed terminal commit.
    #[must_use]
    pub fn tombstoned(
        credential_id: CredentialId,
        version: CredentialVersion,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
        tombstoned_at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            credential_id,
            version,
            state: CredentialRecordState::Tombstoned,
            created_at,
            updated_at,
            tombstoned_at: Some(tombstoned_at),
        }
    }

    /// Return the typed credential id.
    #[must_use]
    pub fn credential_id(self) -> CredentialId {
        self.credential_id
    }

    /// Return the committed version.
    #[must_use]
    pub const fn version(self) -> CredentialVersion {
        self.version
    }

    /// Return the committed lifecycle state.
    #[must_use]
    pub const fn state(self) -> CredentialRecordState {
        self.state
    }

    /// Return the original creation instant.
    #[must_use]
    pub const fn created_at(self) -> chrono::DateTime<chrono::Utc> {
        self.created_at
    }

    /// Return the committed mutation instant.
    #[must_use]
    pub const fn updated_at(self) -> chrono::DateTime<chrono::Utc> {
        self.updated_at
    }

    /// Return the terminal transition instant, when terminal.
    #[must_use]
    pub const fn tombstoned_at(self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.tombstoned_at
    }
}

impl fmt::Debug for CredentialCommit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialCommit")
            .field("credential_id", &"[redacted]")
            .field("version", &self.version)
            .field("state", &self.state)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("tombstoned_at", &self.tombstoned_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant(seconds: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(seconds, 0).expect("test timestamp is representable")
    }

    fn live(version: CredentialVersion, secret: Vec<u8>) -> StoredLiveCredential {
        StoredLiveCredential::new(
            CredentialId::new(),
            Some("production".to_owned()),
            "github_oauth".to_owned(),
            SecretBytes::new(secret),
            "oauth2_state".to_owned(),
            1,
            version,
            CredentialMaterialEpoch::MIN,
            instant(1_700_000_000),
            instant(1_700_000_001),
            None,
            false,
            Map::new(),
            None,
        )
        .expect("test live version is admissible")
    }

    #[test]
    fn empty_owner_is_not_an_admin_sentinel() {
        let empty = CredentialOwner::from_canonical("");
        let other = CredentialOwner::from_canonical("tenant");
        assert_ne!(empty, other);
        assert_eq!(empty.as_str(), "");
    }

    #[test]
    fn terminal_version_is_valid_but_not_live() {
        let terminal = CredentialVersion::try_from(i64::MAX).expect("terminal version is valid");
        assert!(!terminal.is_live());
        assert_eq!(
            StoredLiveCredential::new(
                CredentialId::new(),
                None,
                "key".to_owned(),
                SecretBytes::default(),
                "state".to_owned(),
                1,
                terminal,
                CredentialMaterialEpoch::MIN,
                instant(1_700_000_000),
                instant(1_700_000_001),
                None,
                false,
                Map::new(),
                None,
            ),
            Err(CredentialPersistenceError::CorruptRecord)
        );
    }

    #[test]
    fn live_debug_shape_is_independent_of_secret_content_and_length() {
        let version = CredentialVersion::MIN;
        let empty = format!("{:?}", live(version, Vec::new()));
        let short = format!("{:?}", live(version, vec![0x41]));
        let long = format!("{:?}", live(version, vec![0x5a; 4_096]));
        assert_eq!(empty, short);
        assert_eq!(short, long);
    }

    #[test]
    fn structural_tombstone_carries_no_live_value() {
        let at = instant(1_700_000_002);
        let tombstone = StoredTombstonedCredential::new(
            CredentialId::new(),
            "github_oauth".to_owned(),
            "oauth2_state".to_owned(),
            1,
            CredentialVersion::MIN,
            instant(1_700_000_000),
            at,
            at,
        );
        let stored = StoredCredential::from(tombstone);

        assert_eq!(stored.state(), CredentialRecordState::Tombstoned);
        assert!(stored.as_live().is_none());
        assert!(stored.as_tombstoned().is_some());
    }
}
