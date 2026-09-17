#[cfg(feature = "sqlite")]
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine;
#[cfg(feature = "sqlite")]
use nebula_core::CredentialId;
#[cfg(feature = "sqlite")]
use nebula_storage_port::{
    CredentialOwner, CredentialPersistence, CredentialPersistenceError, CredentialSelector,
};

use super::*;

#[cfg(feature = "sqlite")]
fn selector(id: CredentialId) -> CredentialSelector {
    CredentialSelector::new(CredentialOwner::from_canonical("test-owner"), id)
}

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

// Env-var manipulation is forbidden by this crate's
// `#![forbid(unsafe_code)]` (`set_var` / `remove_var` are `unsafe` in the
// 2024 edition). `EnvKeyProvider::from_env` is covered by the integration
// test at `tests/credential_env_provider.rs`, which uses
// `nebula_env::testing::EnvGuard` to centralize the unsafe boundary.
// Validation-logic coverage for
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
    let snapshot = provider.current().expect("key snapshot available");
    let version = snapshot.key_id();
    assert!(
        version.starts_with("env:"),
        "version has env prefix; got {version}"
    );
    assert_eq!(
        version.len(),
        "env:".len() + 16,
        "version ends in 16-char (8-byte) hex fingerprint; got {version}"
    );
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
/// safety invariant: version must change when key bytes change.
#[test]
fn env_provider_version_changes_with_key() {
    let k1 = base64::engine::general_purpose::STANDARD.encode([0x11u8; 32]);
    let k2 = base64::engine::general_purpose::STANDARD.encode([0x22u8; 32]);
    let v1 = EnvKeyProvider::from_base64(&k1)
        .unwrap()
        .current()
        .unwrap()
        .key_id()
        .to_owned();
    let v2 = EnvKeyProvider::from_base64(&k2)
        .unwrap()
        .current()
        .unwrap()
        .key_id()
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
        .current()
        .unwrap()
        .key_id()
        .to_owned();
    let v2 = EnvKeyProvider::from_base64(&k)
        .unwrap()
        .current()
        .unwrap()
        .key_id()
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
    let snapshot = provider.current().expect("key snapshot available");
    let version = snapshot.key_id();
    assert!(
        version.starts_with("file:nebula.key:"),
        "version has file prefix + filename; got {version}"
    );
    assert_eq!(
        version.len(),
        "file:nebula.key:".len() + 16,
        "version ends in 16-char fingerprint; got {version}"
    );
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
        .current()
        .unwrap()
        .key_id()
        .to_owned();

    std::fs::write(&path, [0x22u8; 32]).unwrap();
    let v2 = FileKeyProvider::from_path(&path)
        .unwrap()
        .current()
        .unwrap()
        .key_id()
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
    let err =
        FileKeyProvider::from_path(dir.path()).expect_err("directory must be refused before read");
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
    let snapshot = provider.current().unwrap();
    assert_eq!(snapshot.key_id(), "static:test");
    assert!(std::ptr::eq(snapshot.key(), key.as_ref()));
}

#[test]
fn static_provider_with_version_preserves_version() {
    let key = Arc::new(EncryptionKey::from_bytes([0x22; 32]));
    let provider = StaticKeyProvider::with_version(key, "rot-v2");
    assert_eq!(provider.current().unwrap().key_id(), "rot-v2");
}

// ------------------------------------------------------------------------
// Trait-level: rotation triggers re-fetch
// ------------------------------------------------------------------------

/// Counts atomic snapshot invocations. Used to assert that the layer
/// re-queries the provider on each read/write rather than caching the
/// key at construction time.
///
/// Only used by the cross-layer tests below (sqlite feature).
#[cfg(feature = "sqlite")]
struct CountingKeyProvider {
    inner: StaticKeyProvider,
    snapshot_calls: AtomicUsize,
}

#[cfg(feature = "sqlite")]
impl CountingKeyProvider {
    fn new(key: Arc<EncryptionKey>) -> Self {
        Self {
            inner: StaticKeyProvider::new(key),
            snapshot_calls: AtomicUsize::new(0),
        }
    }

    fn snapshot_calls(&self) -> usize {
        self.snapshot_calls.load(Ordering::SeqCst)
    }
}

#[cfg(feature = "sqlite")]
impl KeyProvider for CountingKeyProvider {
    fn current(&self) -> Result<KeySnapshot, ProviderError> {
        self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.current()
    }
}

// Cross-layer test — requires the sqlite feature for SqliteCredentialPersistence.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn layer_refetches_provider_on_create_and_get() -> Result<(), CredentialPersistenceError> {
    use crate::credential::test_support::make_credential;

    use super::super::{layer::EncryptionLayer, sqlite::SqliteCredentialPersistence};

    let key = Arc::new(EncryptionKey::from_bytes([0x33; 32]));
    let provider = Arc::new(CountingKeyProvider::new(Arc::clone(&key)));
    let store = EncryptionLayer::new(
        SqliteCredentialPersistence::connect_memory().await?,
        Arc::clone(&provider) as _,
    );

    let before_create = provider.snapshot_calls();
    let selector = selector(CredentialId::new());
    store.create(&selector, make_credential(b"secret")).await?;
    assert!(
        provider.snapshot_calls() > before_create,
        "create must atomically snapshot the provider at least once"
    );

    let before_get = provider.snapshot_calls();
    store.get(&selector).await?;
    assert!(
        provider.snapshot_calls() > before_get,
        "get must atomically snapshot the provider at least once"
    );
    Ok(())
}

/// Failure from `current()` surfaces through the layer as a
/// closed `CredentialPersistenceError::Unavailable` taxonomy.
///
/// Only used by the cross-layer tests below (sqlite feature).
#[cfg(feature = "sqlite")]
struct FailingKeyProvider;

#[cfg(feature = "sqlite")]
impl KeyProvider for FailingKeyProvider {
    fn current(&self) -> Result<KeySnapshot, ProviderError> {
        Err(ProviderError::NotConfigured {
            name: "test-injected".into(),
        })
    }
}

// Cross-layer test — see `layer_refetches_provider_on_put_and_get`.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn provider_failure_surfaces_as_unavailable() -> Result<(), CredentialPersistenceError> {
    use crate::credential::test_support::make_credential;

    use super::super::{layer::EncryptionLayer, sqlite::SqliteCredentialPersistence};

    let store = EncryptionLayer::new(
        SqliteCredentialPersistence::connect_memory().await?,
        Arc::new(FailingKeyProvider) as Arc<dyn KeyProvider>,
    );

    let selector = selector(CredentialId::new());
    let err = store
        .create(&selector, make_credential(b"x"))
        .await
        .expect_err("provider failure must propagate");
    assert_eq!(err, CredentialPersistenceError::Unavailable);
    Ok(())
}
