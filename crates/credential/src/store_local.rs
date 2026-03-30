//! Filesystem-based credential store for desktop/single-node use.
//!
//! Each credential is stored as a JSON file: `{base_dir}/{credential_id}.json`.
//! Writes use atomic rename (temp file then rename) for crash safety.
//! Directory is created automatically on first write.
//!
//! # Path safety
//!
//! Credential IDs are validated to reject path separators and `..` components,
//! preventing directory traversal attacks.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::credential_store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// JSON-serializable representation of a [`StoredCredential`] on disk.
#[derive(Serialize, Deserialize)]
struct StoredFile {
    id: String,
    #[serde(with = "base64_bytes")]
    data: Vec<u8>,
    state_kind: String,
    state_version: u32,
    version: u64,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    metadata: serde_json::Map<String, serde_json::Value>,
}

/// Base64 encoding for the `data` field so binary bytes survive JSON round-trips.
mod base64_bytes {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(d)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}

impl From<StoredCredential> for StoredFile {
    fn from(c: StoredCredential) -> Self {
        Self {
            id: c.id,
            data: c.data,
            state_kind: c.state_kind,
            state_version: c.state_version,
            version: c.version,
            created_at: c.created_at,
            updated_at: c.updated_at,
            expires_at: c.expires_at,
            metadata: c.metadata,
        }
    }
}

impl From<StoredFile> for StoredCredential {
    fn from(f: StoredFile) -> Self {
        Self {
            id: f.id,
            data: f.data,
            state_kind: f.state_kind,
            state_version: f.state_version,
            version: f.version,
            created_at: f.created_at,
            updated_at: f.updated_at,
            expires_at: f.expires_at,
            metadata: f.metadata,
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Validate that a credential ID is safe for use as a filename.
///
/// Rejects IDs containing path separators (`/`, `\`), the parent-directory
/// marker (`..`), and empty strings.
fn validate_id(id: &str) -> Result<(), StoreError> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") || id == "." {
        return Err(StoreError::Backend(
            format!("invalid credential id (path traversal rejected): {id}").into(),
        ));
    }
    Ok(())
}

/// Atomically write `data` to `path` via a temp file in the same directory.
async fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let temp_path = path.with_file_name(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("credential"),
        Uuid::new_v4(),
    ));

    tokio::fs::write(&temp_path, data).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        tokio::fs::set_permissions(&temp_path, perms).await?;
    }

    // Atomic rename — same filesystem guarantees atomicity.
    if let Err(e) = tokio::fs::rename(&temp_path, path).await {
        // Best-effort cleanup of temp file on failure.
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(e);
    }

    Ok(())
}

// ── LocalFileStore ─────────────────────────────────────────────────────────

/// Filesystem credential store.
///
/// Stores each credential as a JSON file under a configurable base directory.
/// Suitable for desktop and single-node deployments where a full database is
/// not warranted.
///
/// Feature-gated behind `storage-local`.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::store_local::LocalFileStore;
///
/// let store = LocalFileStore::new("/var/lib/nebula/credentials");
/// let cred = store.get("my-api-key").await?;
/// ```
pub struct LocalFileStore {
    base_dir: PathBuf,
}

impl LocalFileStore {
    /// Create a new filesystem store rooted at `base_dir`.
    ///
    /// The directory is created lazily on the first write operation.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Build the on-disk path for a credential.
    fn path_for(&self, id: &str) -> PathBuf {
        self.base_dir.join(format!("{id}.json"))
    }

    /// Ensure `base_dir` exists, creating it (and parents) if necessary.
    async fn ensure_dir(&self) -> Result<(), StoreError> {
        tokio::fs::create_dir_all(&self.base_dir)
            .await
            .map_err(|e| StoreError::Backend(Box::new(e)))
    }

    /// Read and deserialize a credential file, returning `None` if missing.
    async fn read_file(&self, path: &Path) -> Result<Option<StoredCredential>, StoreError> {
        match tokio::fs::read(path).await {
            Ok(bytes) => {
                let file: StoredFile =
                    serde_json::from_slice(&bytes).map_err(|e| StoreError::Backend(Box::new(e)))?;
                Ok(Some(file.into()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StoreError::Backend(Box::new(e))),
        }
    }

    /// Serialize and atomically write a credential to disk.
    async fn write_file(&self, credential: &StoredCredential) -> Result<(), StoreError> {
        let file: StoredFile = credential.clone().into();
        let json =
            serde_json::to_vec_pretty(&file).map_err(|e| StoreError::Backend(Box::new(e)))?;
        let path = self.path_for(&credential.id);
        atomic_write(&path, &json)
            .await
            .map_err(|e| StoreError::Backend(Box::new(e)))
    }
}

impl CredentialStore for LocalFileStore {
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        validate_id(id)?;
        let path = self.path_for(id);
        self.read_file(&path)
            .await?
            .ok_or_else(|| StoreError::NotFound { id: id.to_string() })
    }

    async fn put(
        &self,
        mut credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        validate_id(&credential.id)?;
        self.ensure_dir().await?;

        let path = self.path_for(&credential.id);
        let existing = self.read_file(&path).await?;

        match mode {
            PutMode::CreateOnly => {
                if existing.is_some() {
                    return Err(StoreError::AlreadyExists {
                        id: credential.id.clone(),
                    });
                }
                credential.version = 1;
                credential.created_at = chrono::Utc::now();
                credential.updated_at = credential.created_at;
            }
            PutMode::Overwrite => {
                let version = existing.as_ref().map_or(1, |e| e.version + 1);
                credential.version = version;
                credential.updated_at = chrono::Utc::now();
                if version == 1 {
                    credential.created_at = credential.updated_at;
                }
            }
            PutMode::CompareAndSwap { expected_version } => {
                if let Some(ref ex) = existing
                    && ex.version != expected_version
                {
                    return Err(StoreError::VersionConflict {
                        id: credential.id.clone(),
                        expected: expected_version,
                        actual: ex.version,
                    });
                }
                credential.version = expected_version + 1;
                credential.updated_at = chrono::Utc::now();
            }
        }

        self.write_file(&credential).await?;
        Ok(credential)
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        validate_id(id)?;
        let path = self.path_for(id);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(StoreError::NotFound { id: id.to_string() })
            }
            Err(e) => Err(StoreError::Backend(Box::new(e))),
        }
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        // If directory doesn't exist yet, there are no credentials.
        let mut entries = match tokio::fs::read_dir(&self.base_dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(StoreError::Backend(Box::new(e))),
        };

        let mut ids = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| StoreError::Backend(Box::new(e)))?
        {
            let path = entry.path();

            // Only consider .json files.
            let is_json = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == "json");
            if !is_json {
                continue;
            }

            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };

            // Skip temp files produced by atomic_write.
            if stem.starts_with('.') {
                continue;
            }

            if let Some(kind) = state_kind {
                // Need to read the file to check state_kind.
                if let Ok(Some(cred)) = self.read_file(&path).await
                    && cred.state_kind == kind
                {
                    ids.push(cred.id);
                }
            } else {
                ids.push(stem.to_string());
            }
        }

        Ok(ids)
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        validate_id(id)?;
        let path = self.path_for(id);
        match tokio::fs::metadata(&path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(StoreError::Backend(Box::new(e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store() -> (LocalFileStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = LocalFileStore::new(dir.path());
        (store, dir)
    }

    fn make_credential(id: &str, data: &[u8]) -> StoredCredential {
        StoredCredential {
            id: id.into(),
            data: data.to_vec(),
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn crud_operations() {
        let (store, _dir) = make_store();
        let cred = make_credential("test-1", b"secret-data");

        // Create
        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
        assert_eq!(stored.version, 1);

        // Read
        let fetched = store.get("test-1").await.unwrap();
        assert_eq!(fetched.data, b"secret-data");
        assert_eq!(fetched.version, 1);

        // Exists
        assert!(store.exists("test-1").await.unwrap());
        assert!(!store.exists("nonexistent").await.unwrap());

        // Update via overwrite
        let update = make_credential("test-1", b"updated-data");
        let updated = store.put(update, PutMode::Overwrite).await.unwrap();
        assert_eq!(updated.version, 2);
        assert_eq!(updated.data, b"updated-data");

        // List
        let ids = store.list(None).await.unwrap();
        assert_eq!(ids, vec!["test-1"]);

        // Delete
        store.delete("test-1").await.unwrap();
        assert!(!store.exists("test-1").await.unwrap());
    }

    #[tokio::test]
    async fn create_only_rejects_duplicate() {
        let (store, _dir) = make_store();
        let cred = make_credential("dup", b"");
        store.put(cred.clone(), PutMode::CreateOnly).await.unwrap();

        let err = store
            .put(make_credential("dup", b""), PutMode::CreateOnly)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::AlreadyExists { .. }));
    }

    #[tokio::test]
    async fn compare_and_swap_works() {
        let (store, _dir) = make_store();
        let cred = make_credential("cas", b"v1");
        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
        assert_eq!(stored.version, 1);

        // Successful CAS
        let mut update = stored.clone();
        update.data = b"v2".to_vec();
        let updated = store
            .put(
                update,
                PutMode::CompareAndSwap {
                    expected_version: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.version, 2);

        // Stale CAS should fail
        let mut stale = stored;
        stale.data = b"v3".to_vec();
        let err = store
            .put(
                stale,
                PutMode::CompareAndSwap {
                    expected_version: 1,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::VersionConflict { .. }));
    }

    #[tokio::test]
    async fn get_not_found() {
        let (store, _dir) = make_store();
        let err = store.get("missing").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn delete_not_found() {
        let (store, _dir) = make_store();
        let err = store.delete("missing").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn list_filters_by_state_kind() {
        let (store, _dir) = make_store();

        let mut bearer = make_credential("c1", b"");
        bearer.state_kind = "bearer".into();
        store.put(bearer, PutMode::CreateOnly).await.unwrap();

        let mut api_key = make_credential("c2", b"");
        api_key.state_kind = "api_key".into();
        store.put(api_key, PutMode::CreateOnly).await.unwrap();

        let all = store.list(None).await.unwrap();
        assert_eq!(all.len(), 2);

        let bearers = store.list(Some("bearer")).await.unwrap();
        assert_eq!(bearers, vec!["c1"]);

        let api_keys = store.list(Some("api_key")).await.unwrap();
        assert_eq!(api_keys, vec!["c2"]);

        let empty = store.list(Some("nonexistent")).await.unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn directory_created_on_first_write() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("nested").join("deep");
        let store = LocalFileStore::new(&nested);

        assert!(!nested.exists());

        let cred = make_credential("auto-dir", b"data");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        assert!(nested.exists());
        assert!(store.exists("auto-dir").await.unwrap());
    }

    #[tokio::test]
    async fn rejects_path_traversal_id() {
        let (store, _dir) = make_store();

        for bad_id in ["../etc/passwd", "foo/bar", "a\\b", "..", ""] {
            assert!(
                store.get(bad_id).await.is_err(),
                "should reject id: {bad_id:?}"
            );
            assert!(
                store.exists(bad_id).await.is_err(),
                "should reject id: {bad_id:?}"
            );
            assert!(
                store.delete(bad_id).await.is_err(),
                "should reject id: {bad_id:?}"
            );
            assert!(
                store
                    .put(make_credential(bad_id, b"x"), PutMode::CreateOnly)
                    .await
                    .is_err(),
                "should reject id: {bad_id:?}"
            );
        }
    }

    #[tokio::test]
    async fn list_on_nonexistent_dir_returns_empty() {
        let dir = TempDir::new().unwrap();
        let store = LocalFileStore::new(dir.path().join("does-not-exist"));
        let ids = store.list(None).await.unwrap();
        assert!(ids.is_empty());
    }
}
