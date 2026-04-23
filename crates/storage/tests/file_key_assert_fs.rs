//! [`assert_fs`] fixtures for paths read by [`nebula_storage::credential::FileKeyProvider`].
//! Complements `tempfile` in unit tests — same behaviour, slightly clearer child-file API.

use assert_fs::{TempDir, prelude::*};
use nebula_storage::credential::{FileKeyProvider, KeyProvider};

#[test]
fn file_key_loads_from_assert_fs_child_path() {
    let temp = TempDir::new().expect("tempdir");
    let keyfile = temp.child("nebula.key");
    keyfile
        .write_binary(&[0x42u8; 32])
        .expect("write 32-byte raw key");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(keyfile.path(), std::fs::Permissions::from_mode(0o600))
            .expect("restrict key file perms");
    }

    let provider = FileKeyProvider::from_path(keyfile.path()).expect("load from assert_fs path");
    assert!(
        provider.version().starts_with("file:nebula.key:"),
        "expected file-scoped version prefix, got {}",
        provider.version()
    );
    provider.current_key().expect("key material");
}
