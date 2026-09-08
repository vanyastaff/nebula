use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};

use super::VerificationError;

pub(super) const MAX_FILE_BYTES: usize = 1024 * 1024;

/// Lowercase hex, without a formatter — nothing here can fail, so no caller
/// needs a `Result` or an `expect` to encode a digest it already holds.
pub(super) fn hex(bytes: impl IntoIterator<Item = u8>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 15)]));
    }
    encoded
}

pub(super) fn digest(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes))
}

/// The caller supplies an immutable artifact tree; links are never accepted.
pub(super) fn root(path: &Path) -> Result<PathBuf, VerificationError> {
    if !fs::symlink_metadata(path)
        .map_err(|_| VerificationError::ArtifactRead)?
        .is_dir()
    {
        return Err(VerificationError::ArtifactPath);
    }
    path.canonicalize()
        .map_err(|_| VerificationError::ArtifactPath)
}

pub(super) fn artifact(
    root: &Path,
    relative: &str,
    expected_digest: &str,
) -> Result<Vec<u8>, VerificationError> {
    if relative.is_empty()
        || relative.len() > 512
        || relative.contains('\\')
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(VerificationError::ArtifactPath);
    }
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(component) = component else {
            return Err(VerificationError::ArtifactPath);
        };
        path.push(component);
        if fs::symlink_metadata(&path)
            .map_err(|_| VerificationError::ArtifactRead)?
            .is_symlink()
        {
            return Err(VerificationError::ArtifactPath);
        }
    }
    if !path
        .canonicalize()
        .map_err(|_| VerificationError::ArtifactPath)?
        .starts_with(root)
    {
        return Err(VerificationError::ArtifactPath);
    }
    let bytes = bounded_file(&path)?;
    if !is_digest(expected_digest, 64) || digest(&bytes) != expected_digest {
        return Err(VerificationError::ArtifactDigest);
    }
    Ok(bytes)
}

pub(super) fn bounded_file(path: &Path) -> Result<Vec<u8>, VerificationError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| VerificationError::ArtifactRead)?;
    if !metadata.is_file() {
        return Err(VerificationError::ArtifactPath);
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err(VerificationError::ArtifactSize);
    }
    let file = File::open(path).map_err(|_| VerificationError::ArtifactRead)?;
    if !file
        .metadata()
        .map_err(|_| VerificationError::ArtifactRead)?
        .is_file()
    {
        return Err(VerificationError::ArtifactPath);
    }
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| VerificationError::ArtifactRead)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(VerificationError::ArtifactSize);
    }
    Ok(bytes)
}

pub(super) fn is_digest(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
