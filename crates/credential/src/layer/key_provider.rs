//! `KeyProvider` — the seam between [`EncryptionLayer`](super::EncryptionLayer)
//! and the source of the AES-256 key material.
//!
//! `EncryptionLayer` no longer takes `Arc<EncryptionKey>` directly; instead it
//! accepts `Arc<dyn KeyProvider>`. Composition roots choose the provider — env
//! var, file, or (in future) a KMS / Vault / cloud-secret-manager impl — at
//! wiring time. See [ADR-0022](../../../../docs/adr/0022-keyprovider-trait.md)
//! for the decision and ADR-0020 §3 for why the seam is a pre-condition to any
//! `apps/server` composition work.
//!
//! ## Invariants
//!
//! - `version()` must be non-empty. The encryption-layer envelope writer refuses empty `key_id`
//!   values (see `crypto::encrypt_with_key_id`).
//! - **`version()` must change whenever the key bytes change.** `EncryptionLayer` routes a record
//!   whose envelope `key_id` differs from `version()` through the legacy-key path, so a
//!   stable-version/rotating-key provider would silently mis-decrypt under the new key.
//!   `EnvKeyProvider` / `FileKeyProvider` derive `version()` from a SHA-256 fingerprint of the key
//!   material so an in-place rotation (same env var / same file path, new bytes) produces a fresh
//!   identifier automatically. See ADR-0022 §3 + §6 Rotation procedure.
//! - `current_key()` returns `Arc<EncryptionKey>` — a stable handle over the zeroize-on-drop key
//!   newtype. Providers do not expose raw key bytes.
//! - `Debug` / `Display` on providers and on `ProviderError` must not reveal key material (see
//!   [`STYLE.md §6`](../../../../docs/STYLE.md#6-secret-handling)).
//! - Intermediate plaintext (env-var strings, file bytes) is wrapped in `Zeroizing<_>` so scope
//!   exit scrubs it.

use std::{fmt::Write as _, path::PathBuf, sync::Arc};

use base64::Engine;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::crypto::EncryptionKey;

/// Short, non-secret fingerprint of 32-byte key material.
///
/// Returns the first 8 bytes of SHA-256 over the key, hex-encoded (16 chars).
/// Used as the rotating segment of [`KeyProvider::version`] so an in-place
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

/// Source of the current encryption key for [`EncryptionLayer`](super::EncryptionLayer).
///
/// Implementations must preserve every rule from
/// [`docs/STYLE.md §6 — Secret handling`](../../../../docs/STYLE.md#6-secret-handling):
/// zeroized intermediate plaintext, redacted `Debug`, typed errors without
/// embedded secret material.
pub trait KeyProvider: Send + Sync + 'static {
    /// Return the current encryption key. Called on every encrypt/decrypt
    /// cycle, so implementations cache internally rather than re-reading the
    /// backing source on every call.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] when the backing source is unreachable or the
    /// material fails validation. The layer wraps this as
    /// [`StoreError::Backend`](crate::StoreError::Backend) so callers see a
    /// uniform failure taxonomy.
    fn current_key(&self) -> Result<Arc<EncryptionKey>, ProviderError>;

    /// Stable identifier for the current key. Stored as
    /// [`EncryptedData::key_id`](crate::crypto::EncryptedData) in new
    /// envelopes; used by the rotation path to detect mismatches. Must be
    /// non-empty, and **must change whenever `current_key()` returns
    /// different key bytes** — otherwise the layer will treat pre-rotation
    /// records as "current" and silently mis-decrypt them under the new
    /// key. `EnvKeyProvider` / `FileKeyProvider` derive this from a
    /// SHA-256 fingerprint of the key material; operator-authored
    /// providers that hand-manage the version string must preserve the
    /// same invariant.
    fn version(&self) -> &str;
}

/// Typed errors returned by [`KeyProvider`] implementations.
///
/// `#[non_exhaustive]` so future KMS / Vault backends can add variants without
/// breaking downstream consumers. No variant carries raw key bytes per
/// `docs/STYLE.md §6`.
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
/// ```rust,ignore
/// let provider = EnvKeyProvider::from_env()?;
/// let layer = EncryptionLayer::new(inner_store, Arc::new(provider));
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
        let raw = std::env::var(Self::ENV_VAR).map_err(|_| ProviderError::NotConfigured {
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
    fn current_key(&self) -> Result<Arc<EncryptionKey>, ProviderError> {
        Ok(Arc::clone(&self.key))
    }

    fn version(&self) -> &str {
        &self.version
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
/// ```rust,ignore
/// let provider = FileKeyProvider::from_path("/run/secrets/nebula_cred_key")?;
/// let layer = EncryptionLayer::new(inner_store, Arc::new(provider));
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
    fn current_key(&self) -> Result<Arc<EncryptionKey>, ProviderError> {
        Ok(Arc::clone(&self.key))
    }

    fn version(&self) -> &str {
        &self.version
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
/// Gated behind `#[cfg(any(test, feature = "test-util"))]`. Production release
/// builds never see this type; every non-test composition path uses
/// [`EnvKeyProvider`] or [`FileKeyProvider`].
#[cfg(any(test, feature = "test-util"))]
pub struct StaticKeyProvider {
    key: Arc<EncryptionKey>,
    version: Arc<str>,
}

#[cfg(any(test, feature = "test-util"))]
impl StaticKeyProvider {
    /// Wrap `key` with the default test version `"static:test"`.
    pub fn new(key: Arc<EncryptionKey>) -> Self {
        Self::with_version(key, "static:test")
    }

    /// Wrap `key` with a caller-supplied version string. The version is
    /// stored as the envelope `key_id`, so tests that exercise rotation
    /// across versions pass distinct strings here.
    pub fn with_version(key: Arc<EncryptionKey>, version: impl Into<Arc<str>>) -> Self {
        Self {
            key,
            version: version.into(),
        }
    }
}

#[cfg(any(test, feature = "test-util"))]
impl KeyProvider for StaticKeyProvider {
    fn current_key(&self) -> Result<Arc<EncryptionKey>, ProviderError> {
        Ok(Arc::clone(&self.key))
    }

    fn version(&self) -> &str {
        &self.version
    }
}

#[cfg(any(test, feature = "test-util"))]
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
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use base64::Engine;

    use super::*;

    // ------------------------------------------------------------------------
    // ProviderError shape
    // ------------------------------------------------------------------------

    #[test]
    fn provider_error_debug_does_not_mention_key_bytes() {
        // None of the variants carry raw key material; their Debug output
        // must not include anything that looks like decoded bytes.
        let err = ProviderError::NotConfigured {
            name: "NEBULA_CRED_MASTER_KEY".into(),
        };
        let formatted = format!("{err:?}");
        assert!(formatted.contains("NotConfigured"));
        assert!(!formatted.contains("0x"));

        let err = ProviderError::KeyMaterialRejected {
            reason: "expected 32 bytes".into(),
        };
        let formatted = format!("{err:?}");
        assert!(formatted.contains("KeyMaterialRejected"));
        // 0x42 is the byte pattern used in our test keys — must not leak.
        assert!(!formatted.contains("0x42"));
    }

    // ------------------------------------------------------------------------
    // EnvKeyProvider
    // ------------------------------------------------------------------------

    fn valid_base64_key() -> String {
        base64::engine::general_purpose::STANDARD.encode([0x42u8; 32])
    }

    // Env-var manipulation (`std::env::set_var` / `remove_var`) is marked
    // `unsafe` in Rust 2024 edition and is forbidden by this crate's
    // `#![forbid(unsafe_code)]`. `EnvKeyProvider::from_env` is covered by the
    // integration test at `tests/env_provider.rs`, which lives in a separate
    // test-binary crate without that forbid. Validation-logic coverage for
    // dev-placeholder / wrong-length / decode-failure paths lives here via
    // `from_base64`, which exercises the same validators minus the env lookup
    // itself.

    #[test]
    fn env_provider_dev_placeholder_rejected_via_base64() {
        let err = EnvKeyProvider::from_base64(EnvKeyProvider::DEV_PLACEHOLDER)
            .expect_err("dev placeholder must error");
        assert!(matches!(err, ProviderError::DevPlaceholder));
    }

    #[test]
    fn env_provider_short_value_rejected_via_base64() {
        let short = base64::engine::general_purpose::STANDARD.encode([0x42u8; 16]);
        let err = EnvKeyProvider::from_base64(&short).expect_err("short key must error");
        match err {
            ProviderError::KeyMaterialRejected { reason } => {
                assert!(reason.contains("32"), "reason mentions required length");
                assert!(reason.contains("16"), "reason mentions actual length");
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn env_provider_valid_key_round_trips_via_base64() {
        let provider =
            EnvKeyProvider::from_base64(&valid_base64_key()).expect("valid key must succeed");
        let version = provider.version();
        assert!(
            version.starts_with("env:"),
            "version has env prefix; got {version}"
        );
        assert_eq!(
            version.len(),
            "env:".len() + 16,
            "version ends in 16-char (8-byte) hex fingerprint; got {version}"
        );
        provider.current_key().expect("key available");
    }

    #[test]
    fn env_provider_invalid_base64_rejected() {
        let err = EnvKeyProvider::from_base64("not~valid~base64~~").expect_err("must error");
        assert!(matches!(err, ProviderError::Decode { .. }));
    }

    #[test]
    fn env_provider_debug_redacts_key() {
        let provider = EnvKeyProvider::from_base64(&valid_base64_key()).unwrap();
        let formatted = format!("{provider:?}");
        assert!(formatted.contains("[REDACTED]"));
        assert!(!formatted.contains("0x42"));
    }

    /// Two different keys must produce different `version()`s so an in-place
    /// env-var rotation flips the envelope `key_id` instead of silently
    /// mis-decrypting under the new key. Regression guard for the rotation
    /// safety invariant recorded in ADR-0022 §3.
    #[test]
    fn env_provider_version_changes_with_key() {
        let k1 = base64::engine::general_purpose::STANDARD.encode([0x11u8; 32]);
        let k2 = base64::engine::general_purpose::STANDARD.encode([0x22u8; 32]);
        let v1 = EnvKeyProvider::from_base64(&k1)
            .unwrap()
            .version()
            .to_owned();
        let v2 = EnvKeyProvider::from_base64(&k2)
            .unwrap()
            .version()
            .to_owned();
        assert_ne!(
            v1, v2,
            "different keys must produce different versions (v1={v1}, v2={v2})"
        );
    }

    /// Two providers constructed from the same bytes must report the same
    /// `version()` — so restarting a stable deployment does not churn
    /// envelope `key_id`s.
    #[test]
    fn env_provider_version_stable_for_same_key() {
        let k = valid_base64_key();
        let v1 = EnvKeyProvider::from_base64(&k)
            .unwrap()
            .version()
            .to_owned();
        let v2 = EnvKeyProvider::from_base64(&k)
            .unwrap()
            .version()
            .to_owned();
        assert_eq!(v1, v2);
    }

    // ------------------------------------------------------------------------
    // FileKeyProvider
    // ------------------------------------------------------------------------

    #[test]
    fn file_provider_valid_key_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nebula.key");
        std::fs::write(&path, [0x42u8; 32]).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        let provider = FileKeyProvider::from_path(&path).expect("valid file must succeed");
        let version = provider.version();
        assert!(
            version.starts_with("file:nebula.key:"),
            "version has file prefix + filename; got {version}"
        );
        assert_eq!(
            version.len(),
            "file:nebula.key:".len() + 16,
            "version ends in 16-char fingerprint; got {version}"
        );
        provider.current_key().expect("key available");
    }

    /// In-place file rewrite (same path, new bytes) must produce a different
    /// `version()`. Mirrors `env_provider_version_changes_with_key` — the
    /// rotation-observability guarantee must hold for file-mounted secrets
    /// (Kubernetes secret rewrites, systemd credential refreshes).
    #[test]
    fn file_provider_version_changes_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rotated.key");

        std::fs::write(&path, [0x11u8; 32]).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let v1 = FileKeyProvider::from_path(&path)
            .unwrap()
            .version()
            .to_owned();

        std::fs::write(&path, [0x22u8; 32]).unwrap();
        let v2 = FileKeyProvider::from_path(&path)
            .unwrap()
            .version()
            .to_owned();

        assert_ne!(
            v1, v2,
            "rewriting file with new bytes must rotate version (v1={v1}, v2={v2})"
        );
    }

    #[test]
    fn file_provider_wrong_length_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.key");
        std::fs::write(&path, [0x42u8; 16]).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        let err = FileKeyProvider::from_path(&path).expect_err("short file must error");
        match err {
            ProviderError::KeyMaterialRejected { reason } => {
                assert!(reason.contains("32"));
                assert!(reason.contains("16"));
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_provider_world_readable_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("world.key");
        std::fs::write(&path, [0x42u8; 32]).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o644); // world-readable
        std::fs::set_permissions(&path, perms).unwrap();

        let err = FileKeyProvider::from_path(&path).expect_err("world-readable must error");
        match err {
            ProviderError::InsecurePermissions { path: p } => {
                assert_eq!(p, path);
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn file_provider_missing_file_fails_closed() {
        let err = FileKeyProvider::from_path("/nonexistent/path/nebula.key")
            .expect_err("missing file must error");
        match err {
            ProviderError::FileIo { path, source: _ } => {
                // Path must survive through to the error so operators do not
                // have to correlate the failure with their own log lines.
                assert!(
                    path.ends_with("nebula.key"),
                    "error carries the offending path; got {}",
                    path.display()
                );
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    /// Pointing `FileKeyProvider` at a non-regular-file path (here: a
    /// directory) must refuse before any `read` call — otherwise the
    /// behaviour would range from "reads 0 bytes" to "blocks forever on a
    /// FIFO" to "reads unbounded data from `/dev/urandom`". Regular-file
    /// gate closes the class.
    ///
    /// Accepts either:
    /// - `KeyMaterialRejected` — the common path: `File::open` on a directory succeeds on Unix; the
    ///   `is_file()` check fires after.
    /// - `FileIo` — Windows rejects `File::open` on a directory at the syscall level, so the error
    ///   never gets past the open step.
    ///
    /// `InsecurePermissions` must NOT be a permitted outcome here: the
    /// regular-file gate precedes the permissions gate precisely so a
    /// 0o755 directory does not trip the world-readable check and
    /// emit a misleading "insecure permissions" reason for what is
    /// really a "not a file" problem. Guarding against that reordering
    /// bug is part of this test's job.
    #[test]
    fn file_provider_refuses_non_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = FileKeyProvider::from_path(dir.path())
            .expect_err("directory must be refused before read");
        assert!(
            matches!(
                err,
                ProviderError::KeyMaterialRejected { .. } | ProviderError::FileIo { .. }
            ),
            "unexpected variant (note: InsecurePermissions would indicate \
             is_file() / permissions ordering regressed): {err:?}"
        );
    }

    // ------------------------------------------------------------------------
    // StaticKeyProvider
    // ------------------------------------------------------------------------

    #[test]
    fn static_provider_round_trip() {
        let key = Arc::new(EncryptionKey::from_bytes([0x11; 32]));
        let provider = StaticKeyProvider::new(Arc::clone(&key));
        assert_eq!(provider.version(), "static:test");
        assert!(Arc::ptr_eq(&provider.current_key().unwrap(), &key));
    }

    #[test]
    fn static_provider_with_version_preserves_version() {
        let key = Arc::new(EncryptionKey::from_bytes([0x22; 32]));
        let provider = StaticKeyProvider::with_version(key, "rot-v2");
        assert_eq!(provider.version(), "rot-v2");
    }

    // ------------------------------------------------------------------------
    // Trait-level: rotation triggers re-fetch
    // ------------------------------------------------------------------------

    /// Counts `current_key()` invocations. Used to assert that the layer
    /// re-queries the provider on each read/write rather than caching the
    /// key at construction time.
    struct CountingKeyProvider {
        inner: StaticKeyProvider,
        current_key_calls: AtomicUsize,
        version_calls: AtomicUsize,
    }

    impl CountingKeyProvider {
        fn new(key: Arc<EncryptionKey>) -> Self {
            Self {
                inner: StaticKeyProvider::new(key),
                current_key_calls: AtomicUsize::new(0),
                version_calls: AtomicUsize::new(0),
            }
        }

        fn current_key_calls(&self) -> usize {
            self.current_key_calls.load(Ordering::SeqCst)
        }

        fn version_calls(&self) -> usize {
            self.version_calls.load(Ordering::SeqCst)
        }
    }

    impl KeyProvider for CountingKeyProvider {
        fn current_key(&self) -> Result<Arc<EncryptionKey>, ProviderError> {
            self.current_key_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.current_key()
        }

        fn version(&self) -> &str {
            self.version_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.version()
        }
    }

    #[tokio::test]
    async fn layer_refetches_provider_on_put_and_get() {
        use crate::{
            layer::EncryptionLayer,
            store::{CredentialStore, PutMode, test_helpers::make_credential},
            store_memory::InMemoryStore,
        };

        let key = Arc::new(EncryptionKey::from_bytes([0x33; 32]));
        let provider = Arc::new(CountingKeyProvider::new(Arc::clone(&key)));
        let store = EncryptionLayer::new(InMemoryStore::new(), Arc::clone(&provider) as _);

        let before_put = provider.current_key_calls();
        store
            .put(make_credential("refetch-1", b"secret"), PutMode::CreateOnly)
            .await
            .unwrap();
        assert!(
            provider.current_key_calls() > before_put,
            "put must call current_key at least once"
        );

        let before_get = provider.current_key_calls();
        store.get("refetch-1").await.unwrap();
        assert!(
            provider.current_key_calls() > before_get,
            "get must call current_key at least once"
        );

        assert!(
            provider.version_calls() > 0,
            "provider version must be queried too"
        );
    }

    /// Failure from `current_key()` surfaces through the layer as a
    /// `StoreError::Backend` — the typed taxonomy the rest of the credential
    /// surface expects.
    struct FailingKeyProvider;

    impl KeyProvider for FailingKeyProvider {
        fn current_key(&self) -> Result<Arc<EncryptionKey>, ProviderError> {
            Err(ProviderError::NotConfigured {
                name: "test-injected".into(),
            })
        }

        fn version(&self) -> &str {
            "failing:test"
        }
    }

    #[tokio::test]
    async fn provider_failure_surfaces_as_backend_error() {
        use crate::{
            StoreError,
            layer::EncryptionLayer,
            store::{CredentialStore, PutMode, test_helpers::make_credential},
            store_memory::InMemoryStore,
        };

        let store = EncryptionLayer::new(
            InMemoryStore::new(),
            Arc::new(FailingKeyProvider) as Arc<dyn KeyProvider>,
        );

        let err = store
            .put(make_credential("fail-1", b"x"), PutMode::CreateOnly)
            .await
            .expect_err("provider failure must propagate");
        assert!(
            matches!(err, StoreError::Backend(_)),
            "expected Backend variant, got {err:?}"
        );
    }
}
