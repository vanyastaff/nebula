//! Durable identity and reconciliation values for shared runtime resources.

use std::fmt;
use std::hash::{Hash, Hasher};

use sha2::{Digest, Sha256};

use crate::Scope;

const MAX_RESOURCE_KIND_BYTES: usize = 128;
const MAX_CONFIGURATION_IDENTITY_BYTES: usize = 65_536;
const MAX_SLOT_IDENTITY_BYTES: usize = 65_536;
const MAX_PAGE_SIZE: u16 = 1_000;

/// Validation failure for a bounded shared-resource value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SharedResourceValueError {
    /// A required value was empty.
    #[error("{field} must not be empty")]
    Empty {
        /// Stable field name.
        field: &'static str,
    },
    /// A value exceeded its byte limit.
    #[error("{field} exceeds the {max_bytes}-byte limit")]
    TooLong {
        /// Stable field name.
        field: &'static str,
        /// Maximum accepted byte length.
        max_bytes: usize,
    },
    /// A page or batch size was outside `1..=1000`.
    #[error("page or batch size must be between 1 and 1000")]
    InvalidPageSize,
}

/// UTF-8 resource implementation kind.
#[must_use]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResourceKind(String);

impl ResourceKind {
    /// Construct a resource kind from `1..=128` UTF-8 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`SharedResourceValueError`] when the value is empty or too long.
    pub fn new(kind: impl Into<String>) -> Result<Self, SharedResourceValueError> {
        let kind = kind.into();
        validate_required_bytes("resource kind", kind.as_bytes(), MAX_RESOURCE_KIND_BYTES)?;
        Ok(Self(kind))
    }

    /// Borrow the resource kind.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ResourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceKind")
            .field("value", &self.0)
            .finish()
    }
}

/// Compatibility version participating in exact shared-resource identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceCompatibilityVersion(u32);

impl ResourceCompatibilityVersion {
    /// Construct a compatibility version.
    pub const fn new(version: u32) -> Self {
        Self(version)
    }

    /// Return the stored version.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Non-empty canonical resource configuration identity bytes.
#[must_use]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResourceConfigurationIdentity(Box<[u8]>);

impl ResourceConfigurationIdentity {
    /// Construct canonical configuration identity bytes.
    ///
    /// # Errors
    ///
    /// Returns [`SharedResourceValueError`] unless the length is `1..=65536`.
    pub fn try_from_vec(bytes: Vec<u8>) -> Result<Self, SharedResourceValueError> {
        validate_required_bytes(
            "resource configuration identity",
            &bytes,
            MAX_CONFIGURATION_IDENTITY_BYTES,
        )?;
        Ok(Self(bytes.into_boxed_slice()))
    }

    /// Borrow the exact canonical bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for ResourceConfigurationIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        redacted_bytes_debug(formatter, "ResourceConfigurationIdentity", self.0.len())
    }
}

/// Canonical credential-slot identity bytes.
#[must_use]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResourceSlotIdentity(Box<[u8]>);

impl ResourceSlotIdentity {
    /// Construct slot identity bytes with a maximum length of 65536 bytes.
    ///
    /// Empty bytes represent a resource with no credential slot.
    ///
    /// # Errors
    ///
    /// Returns [`SharedResourceValueError`] when the value is too long.
    pub fn try_from_vec(bytes: Vec<u8>) -> Result<Self, SharedResourceValueError> {
        if bytes.len() > MAX_SLOT_IDENTITY_BYTES {
            return Err(SharedResourceValueError::TooLong {
                field: "resource slot identity",
                max_bytes: MAX_SLOT_IDENTITY_BYTES,
            });
        }
        Ok(Self(bytes.into_boxed_slice()))
    }

    /// Borrow the exact canonical bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for ResourceSlotIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        redacted_bytes_debug(formatter, "ResourceSlotIdentity", self.0.len())
    }
}

/// Exact identity of one shareable runtime resource within a [`Scope`].
#[must_use]
#[derive(Clone)]
pub struct SharedResourceIdentity {
    kind: ResourceKind,
    compatibility_version: ResourceCompatibilityVersion,
    configuration: ResourceConfigurationIdentity,
    slot: ResourceSlotIdentity,
    digest: [u8; 32],
}

impl SharedResourceIdentity {
    /// Construct an exact identity and compute its SHA-256 lookup accelerator.
    pub fn new(
        kind: ResourceKind,
        compatibility_version: ResourceCompatibilityVersion,
        configuration: ResourceConfigurationIdentity,
        slot: ResourceSlotIdentity,
    ) -> Self {
        let digest = identity_digest(&kind, compatibility_version, &configuration, &slot);
        Self {
            kind,
            compatibility_version,
            configuration,
            slot,
            digest,
        }
    }

    /// Return the resource kind.
    pub const fn kind(&self) -> &ResourceKind {
        &self.kind
    }

    /// Return the compatibility version.
    pub const fn compatibility_version(&self) -> ResourceCompatibilityVersion {
        self.compatibility_version
    }

    /// Borrow exact canonical configuration identity bytes.
    pub fn configuration_bytes(&self) -> &[u8] {
        self.configuration.as_bytes()
    }

    /// Borrow exact canonical slot identity bytes.
    pub fn slot_bytes(&self) -> &[u8] {
        self.slot.as_bytes()
    }

    /// Return the SHA-256 lookup accelerator.
    ///
    /// Callers must still compare every exact identity field after a digest match.
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

impl PartialEq for SharedResourceIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest
            && self.kind == other.kind
            && self.compatibility_version == other.compatibility_version
            && self.configuration == other.configuration
            && self.slot == other.slot
    }
}

impl Eq for SharedResourceIdentity {}

impl Hash for SharedResourceIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.digest.hash(state);
    }
}

impl fmt::Debug for SharedResourceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedResourceIdentity")
            .field("kind", &self.kind)
            .field("compatibility_version", &self.compatibility_version)
            .field("configuration_len", &self.configuration.0.len())
            .field("slot_len", &self.slot.0.len())
            .finish()
    }
}

/// Opaque store-minted identifier for a shared runtime resource.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SharedResourceId([u8; 16]);

impl SharedResourceId {
    /// Rehydrate an identifier at a persistence boundary.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Return the exact persisted bytes.
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Debug for SharedResourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SharedResourceId([opaque])")
    }
}

/// Exclusive backend sequence cursor for reconciliation keyset pagination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReconciliationCursor(u64);

impl ReconciliationCursor {
    /// Construct a cursor from a backend-authored sequence.
    pub const fn from_sequence(sequence: u64) -> Self {
        Self(sequence)
    }

    /// Return the exclusive backend sequence.
    pub const fn sequence(self) -> u64 {
        self.0
    }
}

/// Validated reconciliation page or delivery batch size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourcePageSize(u16);

impl ResourcePageSize {
    /// Construct a size in `1..=1000`.
    ///
    /// # Errors
    ///
    /// Returns [`SharedResourceValueError::InvalidPageSize`] outside that range.
    pub const fn new(size: u16) -> Result<Self, SharedResourceValueError> {
        if size == 0 || size > MAX_PAGE_SIZE {
            return Err(SharedResourceValueError::InvalidPageSize);
        }
        Ok(Self(size))
    }

    /// Return the validated size.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Request to resolve an exact identity, creating it when absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveSharedResourceRequest {
    scope: Scope,
    identity: SharedResourceIdentity,
}

impl ResolveSharedResourceRequest {
    /// Construct a resolve request.
    pub const fn new(scope: Scope, identity: SharedResourceIdentity) -> Self {
        Self { scope, identity }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the exact resource identity.
    pub const fn identity(&self) -> &SharedResourceIdentity {
        &self.identity
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// Durable shared-resource record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedResourceRecord {
    id: SharedResourceId,
    identity: SharedResourceIdentity,
    reconciliation_sequence: u64,
}

impl SharedResourceRecord {
    /// Rehydrate a durable shared-resource record.
    pub const fn new(
        id: SharedResourceId,
        identity: SharedResourceIdentity,
        reconciliation_sequence: u64,
    ) -> Self {
        Self {
            id,
            identity,
            reconciliation_sequence,
        }
    }

    /// Return the store-minted identifier.
    pub const fn id(&self) -> SharedResourceId {
        self.id
    }

    /// Return the exact identity.
    pub const fn identity(&self) -> &SharedResourceIdentity {
        &self.identity
    }

    /// Return the backend-authored reconciliation sequence.
    pub const fn reconciliation_sequence(&self) -> u64 {
        self.reconciliation_sequence
    }
}

/// Result of resolving one exact shared-resource identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveSharedResourceOutcome {
    /// A new durable row was inserted.
    Created(SharedResourceRecord),
    /// The exact durable identity already existed.
    Existing(SharedResourceRecord),
}

/// One exclusive-keyset reconciliation page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedResourcePage {
    resources: Vec<SharedResourceRecord>,
    next_cursor: Option<ReconciliationCursor>,
}

impl SharedResourcePage {
    /// Construct a reconciliation page from backend-ordered rows.
    pub fn new(
        resources: Vec<SharedResourceRecord>,
        next_cursor: Option<ReconciliationCursor>,
    ) -> Self {
        Self {
            resources,
            next_cursor,
        }
    }

    /// Borrow rows ordered by increasing backend sequence.
    pub fn resources(&self) -> &[SharedResourceRecord] {
        &self.resources
    }

    /// Return the cursor to use exclusively for the next page.
    pub const fn next_cursor(&self) -> Option<ReconciliationCursor> {
        self.next_cursor
    }
}

fn validate_required_bytes(
    field: &'static str,
    bytes: &[u8],
    max_bytes: usize,
) -> Result<(), SharedResourceValueError> {
    if bytes.is_empty() {
        return Err(SharedResourceValueError::Empty { field });
    }
    if bytes.len() > max_bytes {
        return Err(SharedResourceValueError::TooLong { field, max_bytes });
    }
    Ok(())
}

fn redacted_bytes_debug(
    formatter: &mut fmt::Formatter<'_>,
    type_name: &'static str,
    len: usize,
) -> fmt::Result {
    formatter
        .debug_struct(type_name)
        .field("len", &len)
        .finish()
}

fn identity_digest(
    kind: &ResourceKind,
    compatibility_version: ResourceCompatibilityVersion,
    configuration: &ResourceConfigurationIdentity,
    slot: &ResourceSlotIdentity,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    update_length_prefixed(&mut hasher, kind.as_str().as_bytes());
    hasher.update(compatibility_version.get().to_be_bytes());
    update_length_prefixed(&mut hasher, configuration.as_bytes());
    update_length_prefixed(&mut hasher, slot.as_bytes());
    hasher.finalize().into()
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(framed_length(bytes));
    hasher.update(bytes);
}

pub(super) fn framed_length(bytes: &[u8]) -> [u8; 8] {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes()
}
