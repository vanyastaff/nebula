//! `KeyProvider` — the seam between [`EncryptionLayer`](super::layer::EncryptionLayer)
//! and the source of the AES-256 key material.
//!
//! `EncryptionLayer` no longer takes `Arc<EncryptionKey>` directly; instead it
//! accepts `Arc<dyn KeyProvider>`. Composition roots choose the provider — env
//! var, file, or (in future) a KMS / Vault / cloud-secret-manager impl — at
//! wiring time. See `crates/storage/README.md` for the key-provider seam.
//!
//! ## Invariants
//!
//! - [`KeyProvider::current`] returns the key id and key handle in one atomic
//!   [`KeySnapshot`]. A dynamic KMS/Vault provider must never expose a key id
//!   from one generation with bytes from another.
//! - The snapshot key id must change whenever the key bytes change.
//!   `EnvKeyProvider` / `FileKeyProvider` derive it from a SHA-256 fingerprint.
//! - Snapshots expose only `Arc<EncryptionKey>` — a stable handle over the
//!   zeroize-on-drop key newtype. Providers do not expose raw key bytes.
//! - `Debug` / `Display` on providers and on `ProviderError` must not reveal key material
//!   (secret handling).
//! - Intermediate plaintext (env-var strings, file bytes) is wrapped in `Zeroizing<_>` so scope
//!   exit scrubs it.

use std::{fmt::Write as _, path::PathBuf, sync::Arc};

use base64::Engine;
use nebula_crypto::EncryptionKey;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// Short, non-secret fingerprint of 32-byte key material.
///
/// Returns the first 8 bytes of SHA-256 over the key, hex-encoded (16 chars).
/// Used as the rotating segment of [`KeySnapshot::key_id`] so an in-place
/// key rotation (same env var, same file path, new bytes) produces a
/// **different** envelope `key_id`. Stored records then flow through the
/// legacy-key path instead of silently mis-decrypting under the new key.
///
/// 64 bits of output is ample for rotation correlation inside a single
/// deployment while keeping envelope overhead small. The SHA-256 input is
/// a cryptographic key (not user input) so second-preimage resistance is
/// not a concern for this use.
fn key_fingerprint(bytes: &[u8; 32]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(16);
    for byte in &digest[..8] {
        // Two lowercase hex chars per byte — `write!` to a String never fails.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// One atomically observed encryption-key generation.
pub struct KeySnapshot {
    key_id: Arc<str>,
    key: Arc<EncryptionKey>,
}

impl KeySnapshot {
    /// Construct a validated snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::KeyMaterialRejected`] when `key_id` is empty,
    /// contains control characters, or exceeds the envelope bound.
    pub fn new(
        key_id: impl Into<Arc<str>>,
        key: Arc<EncryptionKey>,
    ) -> Result<Self, ProviderError> {
        let key_id = key_id.into();
        if key_id.is_empty() || key_id.len() > 255 || key_id.chars().any(char::is_control) {
            return Err(ProviderError::KeyMaterialRejected {
                reason: "key identifier is invalid".to_owned(),
            });
        }
        Ok(Self { key_id, key })
    }

    /// Stable non-secret identifier stored in new envelopes.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// Borrow the zeroize-on-drop key handle.
    #[must_use]
    pub fn key(&self) -> &EncryptionKey {
        &self.key
    }

    /// Split the snapshot into its non-secret id and key handle.
    #[must_use]
    pub fn into_parts(self) -> (Arc<str>, Arc<EncryptionKey>) {
        (self.key_id, self.key)
    }
}

impl std::fmt::Debug for KeySnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KeySnapshot")
            .field("key_id", &self.key_id)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

/// Source of the current encryption key for [`EncryptionLayer`](super::layer::EncryptionLayer).
///
/// Implementations must preserve every secret-handling rule:
/// zeroized intermediate plaintext, redacted `Debug`, typed errors without
/// embedded secret material.
pub trait KeyProvider: Send + Sync + 'static {
    /// Atomically snapshot the current key identifier and key handle.
    ///
    /// Called on every encrypt/decrypt cycle, so implementations should cache
    /// internally. Dynamic providers must synchronize rotation so both fields
    /// come from the same generation.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when the backing source is unreachable or the
    /// material fails validation. The layer wraps this as
    /// `CredentialPersistenceError::Unavailable` so callers see the closed
    /// persistence failure taxonomy without a dynamic driver message.
    fn current(&self) -> Result<KeySnapshot, ProviderError>;
}

/// Typed errors returned by [`KeyProvider`] implementations.
///
/// `#[non_exhaustive]` so future KMS / Vault backends can add variants without
/// breaking downstream consumers. No variant carries raw key bytes
/// (secret handling).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderError {
    /// The configured source for the key is not present (env var unset, file
    /// path missing, …). `name` identifies what was checked — never the secret.
    #[error("key material source not configured: {name}")]
    NotConfigured {
        /// Human-readable identifier of the source that was missing
        /// (`NEBULA_CRED_MASTER_KEY`, a file path, …). Never the secret.
        name: String,
    },

    /// The configured source produced a value that failed validation (wrong
    /// length after decode, dev placeholder, …). `reason` describes the
    /// failure shape — never the secret.
    #[error("key material rejected: {reason}")]
    KeyMaterialRejected {
        /// Structured reason for rejection. Never the secret.
        reason: String,
    },

    /// The source literally matches the well-known development placeholder.
    /// Refused even in dev; there is no ephemeral-key fallback for encryption
    /// (unlike `JwtSecret::generate_ephemeral`) because an ephemeral encryption
    /// key makes stored credentials unreadable across restarts.
    #[error("key material matches the well-known development placeholder — refusing to start")]
    DevPlaceholder,

    /// The source's bytes failed to decode (e.g. base64).
    #[error("key material decode failed: {reason}")]
    Decode {
        /// Decoder-reported reason. Never the secret.
        reason: String,
    },

    /// Filesystem permissions on a file-backed source are too permissive.
    /// Flagged by [`FileKeyProvider`] on Unix; skipped on Windows where POSIX
    /// mode bits do not apply.
    #[error("key material file has insecure permissions: {path}")]
    InsecurePermissions {
        /// The path whose mode is unsafe. Not the secret.
        path: PathBuf,
    },

    /// I/O error reading a file-backed source. Carries the offending path so
    /// operators can diagnose missing mounts, permission denied, etc. without
    /// having to correlate with their own log lines.
    #[error("key material file I/O failed for {path}")]
    FileIo {
        /// The file path that failed. Not the secret.
        path: PathBuf,
        /// The underlying filesystem error.
        #[source]
        source: std::io::Error,
    },

    /// I/O error reaching a non-file backing source. Retained for future
    /// providers whose source is not path-shaped; file-backed providers use
    /// [`Self::FileIo`] so the path surfaces.
    #[error("key material source I/O failed")]
    Io(#[source] std::io::Error),
}

// ============================================================================
// EnvKeyProvider
// ============================================================================

/// Reads a 32-byte AES-256 key from an environment variable (base64).
///
/// The canonical local / single-tenant default. Fail-closed on missing,
/// short, wrong-length, or dev-placeholder values — mirroring
/// [`JwtSecret::new`](../../../../crates/api/src/config.rs) so operators see
/// one mental model across auth and encryption-at-rest pre-conditions.
///
/// # Examples
///
/// Production wiring reads the env var via [`EnvKeyProvider::from_env`]; the
/// runnable example below uses [`EnvKeyProvider::from_base64`] (the same
/// validators, minus the env lookup) so it exercises the real parse + version
/// fingerprint without touching the process environment:
///
/// ```rust
/// use base64::Engine;
/// use nebula_storage::credential::{EnvKeyProvider, KeyProvider};
///
/// let key_b64 = base64::engine::general_purpose::STANDARD.encode([0x42u8; 32]);
/// let provider = EnvKeyProvider::from_base64(&key_b64).expect("valid 32-byte key");
///
/// // The id and key are observed as one atomic generation.
/// let snapshot = provider.current()?;
/// assert!(snapshot.key_id().starts_with("env:"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct EnvKeyProvider {
    key: Arc<EncryptionKey>,
    version: Arc<str>,
}

impl EnvKeyProvider {
    /// Default env var name. Operators may wrap the constructor if they need
    /// a different name for a specific deployment shape.
    pub const ENV_VAR: &'static str = "NEBULA_CRED_MASTER_KEY";

    /// Required length after base64 decode (32 bytes = AES-256).
    pub const MIN_BYTES: usize = 32;

    /// Well-known development placeholder. Refused even inside the process;
    /// the whole point is that a leaked-back placeholder value does not
    /// silently become the production key.
    pub const DEV_PLACEHOLDER: &'static str = "dev-encryption-key-change-in-production";

    /// Load the key from [`Self::ENV_VAR`].
    ///
    /// # Errors
    ///
    /// - [`ProviderError::NotConfigured`] if the env var is unset.
    /// - [`ProviderError::DevPlaceholder`] if the value matches [`Self::DEV_PLACEHOLDER`].
    /// - [`ProviderError::Decode`] on base64-decode failure.
    /// - [`ProviderError::KeyMaterialRejected`] on wrong decoded length.
    pub fn from_env() -> Result<Self, ProviderError> {
        let raw = nebula_env::var(Self::ENV_VAR).map_err(|_| ProviderError::NotConfigured {
            name: Self::ENV_VAR.to_string(),
        })?;
        let raw = Zeroizing::new(raw);
        Self::from_base64(&raw)
    }

    /// Load the key from a base64 string (used by [`Self::from_env`] and by
    /// direct composition-root wiring that sources the key from something
    /// other than `std::env`, e.g. a systemd credential file whose contents
    /// are base64).
    ///
    /// # Errors
    ///
    /// See [`Self::from_env`].
    pub fn from_base64(raw: &str) -> Result<Self, ProviderError> {
        if raw == Self::DEV_PLACEHOLDER {
            return Err(ProviderError::DevPlaceholder);
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(raw)
            .map_err(|e| ProviderError::Decode {
                reason: e.to_string(),
            })?;
        let decoded = Zeroizing::new(decoded);
        if decoded.len() != Self::MIN_BYTES {
            return Err(ProviderError::KeyMaterialRejected {
                reason: format!(
                    "expected exactly {} bytes after base64 decode, got {}",
                    Self::MIN_BYTES,
                    decoded.len()
                ),
            });
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&decoded);
        let fingerprint = key_fingerprint(&key_bytes);
        Ok(Self {
            key: Arc::new(EncryptionKey::from_bytes(key_bytes)),
            version: Arc::from(format!("env:{fingerprint}")),
        })
    }
}

impl KeyProvider for EnvKeyProvider {
    fn current(&self) -> Result<KeySnapshot, ProviderError> {
        KeySnapshot::new(Arc::clone(&self.version), Arc::clone(&self.key))
    }
}

impl std::fmt::Debug for EnvKeyProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvKeyProvider")
            .field("version", &&*self.version)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// FileKeyProvider
// ============================================================================

/// Reads 32 raw key bytes from a filesystem path.
///
/// Useful for Kubernetes secrets mounted into the container filesystem
/// (`/run/secrets/`), systemd credential files
/// (`$CREDENTIALS_DIRECTORY/`), and operators who want the key on-disk with
/// a discrete rotation story rather than in the process environment.
///
/// # Permissions
///
/// On Unix, the key file must NOT be world-readable (`mode & 0o004 == 0`);
/// a world-readable key file is refused with
/// [`ProviderError::InsecurePermissions`]. On Windows the check is skipped —
/// POSIX mode bits do not apply. Operators on Windows are expected to restrict
/// the file via Windows ACLs out of band.
///
/// # Examples
///
/// Production points `from_path` at a mounted secret
/// (`/run/secrets/nebula_cred_key`); the runnable example writes a 32-byte key
/// to a private temp file (created `0o600` on Unix, satisfying the
/// world-readable check) and loads it through the real API:
///
/// ```rust
/// use std::io::Write as _;
///
/// use nebula_storage::credential::{FileKeyProvider, KeyProvider};
///
/// let mut key_file = tempfile::NamedTempFile::new()?;
/// key_file.write_all(&[0x42u8; 32])?;
///
/// let provider = FileKeyProvider::from_path(key_file.path())?;
/// let snapshot = provider.current()?;
/// assert!(snapshot.key_id().starts_with("file:"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct FileKeyProvider {
    key: Arc<EncryptionKey>,
    version: Arc<str>,
}

impl FileKeyProvider {
    /// Required raw file length (32 bytes = AES-256).
    pub const MIN_BYTES: usize = 32;

    /// Load the key from `path`.
    ///
    /// The file is opened once; all subsequent checks run against that
    /// handle. This avoids a TOCTOU gap between `stat(path)` and
    /// `read(path)` where a symlink could be swapped in-between, and
    /// ensures non-regular files (FIFOs, device nodes) are refused
    /// before any blocking or unbounded read would occur — `std::fs::read`
    /// without a regular-file check can block on a named pipe or read
    /// arbitrary data from a character device.
    ///
    /// # Errors
    ///
    /// - [`ProviderError::InsecurePermissions`] if the file is world-readable on Unix.
    /// - [`ProviderError::FileIo`] on filesystem errors (missing file, permission denied, …) —
    ///   carries the offending path for diagnostics.
    /// - [`ProviderError::KeyMaterialRejected`] if the target is not a regular file, or its length
    ///   differs from [`Self::MIN_BYTES`].
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Self, ProviderError> {
        use std::io::Read as _;

        let path = path.as_ref();

        // Open once — subsequent checks run against this handle rather than
        // the path, closing the stat/read TOCTOU gap.
        let mut file = std::fs::File::open(path).map_err(|source| ProviderError::FileIo {
            path: path.to_path_buf(),
            source,
        })?;
        let metadata = file.metadata().map_err(|source| ProviderError::FileIo {
            path: path.to_path_buf(),
            source,
        })?;

        // Reject anything that is not a regular file FIRST: FIFOs would
        // block on read_exact, character devices (`/dev/urandom` etc.)
        // would yield unbounded / meaningless data, and directories
        // simply don't hold a 32-byte key. Has to precede the Unix
        // permissions gate below — POSIX default mode on a directory
        // (0o755) would otherwise trip the world-readable check and
        // mask the real "not a file" problem with a misleading
        // `InsecurePermissions` error.
        if !metadata.is_file() {
            return Err(ProviderError::KeyMaterialRejected {
                reason: format!(
                    "expected a regular file at {}, got a non-regular filesystem entry",
                    path.display()
                ),
            });
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // World-readable bit set on a regular key file => refuse.
            if metadata.mode() & 0o004 != 0 {
                return Err(ProviderError::InsecurePermissions {
                    path: path.to_path_buf(),
                });
            }
        }

        if metadata.len() != Self::MIN_BYTES as u64 {
            return Err(ProviderError::KeyMaterialRejected {
                reason: format!(
                    "expected exactly {} bytes in key file, got {}",
                    Self::MIN_BYTES,
                    metadata.len()
                ),
            });
        }

        // Fixed-size buffer + read_exact: no chance of reading more than 32
        // bytes, zeroized on scope exit if any subsequent step fails.
        let mut key_bytes = Zeroizing::new([0u8; 32]);
        file.read_exact(key_bytes.as_mut_slice())
            .map_err(|source| ProviderError::FileIo {
                path: path.to_path_buf(),
                source,
            })?;

        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("key");
        let fingerprint = key_fingerprint(&key_bytes);
        Ok(Self {
            // `EncryptionKey::from_bytes` copies the array into its own
            // zeroize-on-drop newtype; `key_bytes` (the Zeroizing wrapper)
            // drops at end of scope and scrubs the source buffer.
            key: Arc::new(EncryptionKey::from_bytes(*key_bytes)),
            version: Arc::from(format!("file:{filename}:{fingerprint}")),
        })
    }
}

impl KeyProvider for FileKeyProvider {
    fn current(&self) -> Result<KeySnapshot, ProviderError> {
        KeySnapshot::new(Arc::clone(&self.version), Arc::clone(&self.key))
    }
}

impl std::fmt::Debug for FileKeyProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileKeyProvider")
            .field("version", &&*self.version)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// StaticKeyProvider (test-only)
// ============================================================================

/// Test-only provider that wraps an in-memory [`EncryptionKey`].
///
/// Gated behind `#[cfg(test)]`. Production release builds never see this
/// type; every non-test composition path uses [`EnvKeyProvider`] or
/// [`FileKeyProvider`].
#[cfg(test)]
pub(crate) struct StaticKeyProvider {
    key: Arc<EncryptionKey>,
    version: Arc<str>,
}

#[cfg(test)]
impl StaticKeyProvider {
    /// Wrap `key` with the default test version `"static:test"`.
    pub(crate) fn new(key: Arc<EncryptionKey>) -> Self {
        Self::with_version(key, "static:test")
    }

    /// Wrap `key` with a caller-supplied version string. The version is
    /// stored as the envelope `key_id`, so tests that exercise rotation
    /// across versions pass distinct strings here.
    pub(crate) fn with_version(key: Arc<EncryptionKey>, version: impl Into<Arc<str>>) -> Self {
        Self {
            key,
            version: version.into(),
        }
    }
}

#[cfg(test)]
impl KeyProvider for StaticKeyProvider {
    fn current(&self) -> Result<KeySnapshot, ProviderError> {
        KeySnapshot::new(Arc::clone(&self.version), Arc::clone(&self.key))
    }
}

#[cfg(test)]
impl std::fmt::Debug for StaticKeyProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticKeyProvider")
            .field("version", &&*self.version)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "key_provider_tests.rs"]
mod tests;
