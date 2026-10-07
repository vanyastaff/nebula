//! Environment configuration for the standalone worker process.

use nebula_core::ArtifactSetDigest;

/// Hex-encode a processor-id byte slice for structured log fields.
fn hex_id(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

// ── Config helpers (used by main) ─────────────────────────────────────────────

/// Worker config read from environment variables.
///
/// Artifact identity comes from the trusted deployment manifest; storage and
/// polling settings have local defaults.
pub struct WorkerConfig {
    /// Digest of the selected worker artifact set from the deployment manifest.
    /// `NEBULA_WORKER_ARTIFACT_SET_DIGEST` is required (64 lowercase hex digits).
    /// Parsing this value does not authenticate the deployed artifacts.
    pub artifact_set_digest: ArtifactSetDigest,
    /// Postgres connection URL.
    ///
    /// Set `NEBULA_WORKER_DATABASE_URL` to a `postgres://` DSN to use the
    /// Postgres backend. When unset the worker uses SQLite (see `db_path`).
    ///
    /// The binary must be compiled with `--features postgres` (i.e. the
    /// `postgres` cargo feature) for the Postgres path to be available. If
    /// `NEBULA_WORKER_DATABASE_URL` is set on a binary built without that
    /// feature, startup fails with an explicit error rather than silently
    /// falling back to SQLite.
    pub database_url: Option<String>,

    /// Path to the SQLite database file.
    ///
    /// Defaults to `nebula-worker.db` in the current working directory.
    /// Set `NEBULA_WORKER_DB_PATH` to override. An in-memory database is not
    /// suitable for production because durability is lost on process restart.
    /// Ignored when `database_url` is `Some` (Postgres path takes precedence).
    pub db_path: String,

    /// 16-byte processor identity, hex-encoded (32 hex chars).
    ///
    /// Read from `NEBULA_WORKER_PROCESSOR_ID`. When absent, a fresh random
    /// UUID v4 is generated (ephemeral per boot). Ephemeral-per-boot is
    /// correct: a restarted worker's in-flight claims are recovered by the
    /// reclaim sweep regardless of processor id. Set `NEBULA_WORKER_PROCESSOR_ID`
    /// to a stable 32-char hex value per process when you want consistent fence
    /// tokens across restarts (e.g. for observability / log correlation).
    ///
    /// **Every process must use a distinct value** — two workers sharing the
    /// same processor id can acknowledge each other's claimed commands (at-least-once
    /// violation).
    pub processor_id: [u8; 16],
}

impl std::fmt::Debug for WorkerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerConfig")
            .field("artifact_set_digest", &self.artifact_set_digest)
            .field(
                "database_url",
                &self.database_url.as_ref().map(|_| "[REDACTED]"),
            )
            .field("db_path", &self.db_path)
            .field("processor_id", &self.processor_id)
            .finish()
    }
}

/// Errors produced while loading [`WorkerConfig`] from the environment.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkerConfigError {
    /// No trusted worker artifact identity was supplied.
    #[error("NEBULA_WORKER_ARTIFACT_SET_DIGEST is required from the deployment manifest")]
    MissingArtifactSetDigest,
    /// The supplied artifact identity is not a canonical transport digest.
    #[error("NEBULA_WORKER_ARTIFACT_SET_DIGEST must contain 64 lowercase hexadecimal characters")]
    InvalidArtifactSetDigest,
    /// `NEBULA_WORKER_PROCESSOR_ID` is set but is not exactly 32 hex characters.
    #[error(
        "NEBULA_WORKER_PROCESSOR_ID must be exactly 32 hex characters (16 bytes); got {len} chars"
    )]
    ProcessorIdLength {
        /// Actual length of the supplied string.
        len: usize,
    },
    /// `NEBULA_WORKER_PROCESSOR_ID` contains non-ASCII characters.
    ///
    /// Non-ASCII input passes the 32-byte length check (multibyte UTF-8 chars
    /// each occupy >1 byte) but the subsequent byte-slice `&str[i*2..i*2+2]`
    /// would land on a non-char boundary and panic. This variant is returned
    /// before the slice loop so the function never panics on malformed input.
    #[error("NEBULA_WORKER_PROCESSOR_ID must be ASCII hex (0-9, a-f, A-F)")]
    ProcessorIdAscii,
    /// `NEBULA_WORKER_PROCESSOR_ID` contains non-hex characters.
    #[error("NEBULA_WORKER_PROCESSOR_ID contains non-hex characters: {source}")]
    ProcessorIdHex {
        /// Underlying hex decode error.
        #[source]
        source: std::num::ParseIntError,
    },
    /// `NEBULA_WORKER_DATABASE_URL` is set but its value is not valid UTF-8.
    ///
    /// `.ok()` would silently swallow this as `None` and fall back to SQLite —
    /// a contradiction of the fail-closed contract. This variant ensures a
    /// set-but-malformed DSN is always a hard startup error.
    #[error("NEBULA_WORKER_DATABASE_URL is set but not valid UTF-8")]
    DatabaseUrlNotUnicode,
}

impl WorkerConfig {
    /// Load configuration from environment variables.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerConfigError`] when a set environment variable is present
    /// but structurally invalid (wrong format, not parseable as the expected
    /// type, or out of the valid range). The artifact-set digest is mandatory;
    /// absent storage variables use defaults.
    pub fn from_env() -> Result<Self, WorkerConfigError> {
        let artifact_set_digest =
            parse_artifact_set_digest(std::env::var("NEBULA_WORKER_ARTIFACT_SET_DIGEST"))?;
        let database_url = parse_database_url(std::env::var("NEBULA_WORKER_DATABASE_URL"))?;

        let db_path = std::env::var("NEBULA_WORKER_DB_PATH")
            .unwrap_or_else(|_| "nebula-worker.db".to_owned());

        let processor_id = match std::env::var("NEBULA_WORKER_PROCESSOR_ID") {
            Ok(raw) => parse_processor_id(&raw)?,
            Err(_) => generate_ephemeral_processor_id(),
        };

        Ok(Self {
            artifact_set_digest,
            database_url,
            db_path,
            processor_id,
        })
    }
}

fn parse_artifact_set_digest(
    raw: Result<String, std::env::VarError>,
) -> Result<ArtifactSetDigest, WorkerConfigError> {
    match raw {
        Ok(value) => value
            .parse()
            .map_err(|_| WorkerConfigError::InvalidArtifactSetDigest),
        Err(std::env::VarError::NotPresent) => Err(WorkerConfigError::MissingArtifactSetDigest),
        Err(std::env::VarError::NotUnicode(_)) => Err(WorkerConfigError::InvalidArtifactSetDigest),
    }
}

/// Parse the raw result of `std::env::var("NEBULA_WORKER_DATABASE_URL")` into
/// an `Option<String>`, distinguishing the three meaningful cases:
///
/// - `Ok(url)` — variable is set and valid UTF-8 → `Ok(Some(url))`
/// - `Err(VarError::NotPresent)` — variable is unset → `Ok(None)` (SQLite default)
/// - `Err(VarError::NotUnicode(_))` — variable is set but not valid UTF-8 →
///   `Err(WorkerConfigError::DatabaseUrlNotUnicode)` (fail-closed)
///
/// Using `.ok()` instead would collapse the `NotUnicode` case into `None`,
/// silently falling back to SQLite when the operator clearly intended Postgres.
fn parse_database_url(
    raw: Result<String, std::env::VarError>,
) -> Result<Option<String>, WorkerConfigError> {
    match raw {
        Ok(url) => Ok(Some(url)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(WorkerConfigError::DatabaseUrlNotUnicode),
    }
}

/// Parse a 32-char hex string into a 16-byte processor id.
fn parse_processor_id(raw: &str) -> Result<[u8; 16], WorkerConfigError> {
    let trimmed = raw.trim();
    if trimmed.len() != 32 {
        return Err(WorkerConfigError::ProcessorIdLength { len: trimmed.len() });
    }
    // Guard against multibyte UTF-8: a 32-BYTE non-ASCII string passes the
    // length check above but byte-slicing `&str[i*2..i*2+2]` would land on a
    // non-char boundary and panic. Requiring ASCII makes every byte offset a
    // valid char boundary, so the slice loop below is panic-free.
    if !trimmed.is_ascii() {
        return Err(WorkerConfigError::ProcessorIdAscii);
    }
    let mut out = [0u8; 16];
    for i in 0..16usize {
        // Slice directly into the &str — no allocation, no from_utf8, no expect.
        // Bounds are safe: trimmed is exactly 32 ASCII chars; each i*2..i*2+2
        // is a valid char boundary (ASCII chars are single bytes) and in [0, 32).
        let hex_pair = &trimmed[i * 2..i * 2 + 2];
        out[i] = u8::from_str_radix(hex_pair, 16)
            .map_err(|source| WorkerConfigError::ProcessorIdHex { source })?;
    }
    Ok(out)
}

/// Generate a fresh random 16-byte processor id from a UUID v4 and warn.
///
/// Called when `NEBULA_WORKER_PROCESSOR_ID` is not set. The ephemeral id is
/// unique per boot; in-flight claims from a prior boot are recovered by the
/// reclaim sweep regardless, so uniqueness-per-boot is the correct invariant.
///
/// A `WARN`-level log line is emitted so operators see that no stable id is
/// configured. Set `NEBULA_WORKER_PROCESSOR_ID` (32 hex chars) per process for
/// a stable fence token across restarts.
fn generate_ephemeral_processor_id() -> [u8; 16] {
    let id = uuid::Uuid::new_v4().into_bytes();
    tracing::warn!(
        processor_id = %hex_id(&id),
        "NEBULA_WORKER_PROCESSOR_ID not set; generated an ephemeral processor id — \
         set it explicitly for a stable fence identity across restarts"
    );
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_activation_requires_canonical_deployment_identity() {
        assert!(matches!(
            parse_artifact_set_digest(Err(std::env::VarError::NotPresent)),
            Err(WorkerConfigError::MissingArtifactSetDigest)
        ));
        for malformed in [
            String::new(),
            "ff".repeat(31),
            "FF".repeat(32),
            "secret-canary".to_owned(),
        ] {
            let error = parse_artifact_set_digest(Ok(malformed)).expect_err("invalid identity");
            assert!(matches!(error, WorkerConfigError::InvalidArtifactSetDigest));
            assert!(!format!("{error:?} {error}").contains("secret-canary"));
        }
        let digest = ArtifactSetDigest::from_bytes([0x71; 32]);
        assert_eq!(
            parse_artifact_set_digest(Ok(digest.to_string())).unwrap(),
            digest
        );
    }

    #[test]
    fn parse_processor_id_accepts_32_hex_chars() {
        let id = parse_processor_id("0102030405060708090a0b0c0d0e0f10").unwrap();
        assert_eq!(
            id,
            [
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
                0x0f, 0x10
            ]
        );
    }

    #[test]
    fn parse_processor_id_rejects_wrong_length() {
        let err = parse_processor_id("deadbeef").unwrap_err();
        assert!(
            matches!(err, WorkerConfigError::ProcessorIdLength { len: 8 }),
            "expected ProcessorIdLength(8), got {err}"
        );
    }

    #[test]
    fn parse_processor_id_rejects_non_hex() {
        let err = parse_processor_id("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").unwrap_err();
        assert!(
            matches!(err, WorkerConfigError::ProcessorIdHex { .. }),
            "expected ProcessorIdHex, got {err}"
        );
    }

    /// `£` is U+00A3 (2 bytes in UTF-8). Sixteen repetitions = 32 bytes,
    /// which passes the `len() != 32` byte-length check. Without the ASCII
    /// guard, the subsequent `&trimmed[i*2..i*2+2]` byte-slice lands on a
    /// non-char boundary and panics. With the guard it returns `ProcessorIdAscii`.
    ///
    /// Red-able: removing (or bypassing) the `is_ascii()` guard causes this test
    /// to panic rather than return `Err`, and `unwrap_err()` converts the panic
    /// into a test failure with a clear message.
    #[test]
    fn parse_processor_id_rejects_non_ascii_multibyte() {
        // 16 × "£" = 16 × 2 bytes = 32 bytes — passes the length check but is
        // not ASCII. This is the exact shape that would panic without the guard.
        let non_ascii_32_bytes = "£".repeat(16);
        assert_eq!(
            non_ascii_32_bytes.len(),
            32,
            "test invariant: input must be 32 bytes so the length check passes"
        );
        let err = parse_processor_id(&non_ascii_32_bytes).unwrap_err();
        assert!(
            matches!(err, WorkerConfigError::ProcessorIdAscii),
            "expected ProcessorIdAscii for non-ASCII input, got: {err}"
        );
    }

    // ── NEBULA_WORKER_DATABASE_URL parsing ───────────────────────────────────
    //
    // Tests drive `parse_database_url` directly — no env-var mutation, no
    // `unsafe`. The helper has real branching across three cases, so each gets
    // its own test.
    //
    // Red-able: replacing the helper with `.ok()` collapses `NotUnicode` into
    // `None` and makes `database_url_not_unicode_is_fail_closed` fail — it
    // would receive `Ok(None)` instead of `Err(DatabaseUrlNotUnicode)`.

    #[test]
    fn database_url_present_passes_through() {
        let result = parse_database_url(Ok("postgres://localhost/nebula".to_owned()));
        assert_eq!(
            result.unwrap(),
            Some("postgres://localhost/nebula".to_owned()),
            "a valid DSN must be forwarded as Some so the Postgres arm is selected"
        );
    }

    #[test]
    fn database_url_not_present_yields_none() {
        let result = parse_database_url(Err(std::env::VarError::NotPresent));
        assert_eq!(
            result.unwrap(),
            None,
            "an absent variable must yield None, keeping the SQLite default"
        );
    }

    /// A set-but-non-UTF-8 variable must fail closed — not silently fall back
    /// to SQLite. Runs on all platforms: `VarError::NotUnicode` can be
    /// constructed with any `OsString`; the helper matches on the variant
    /// discriminant, not the byte content, so a UTF-8-clean `OsString` is
    /// sufficient to exercise the `NotUnicode` branch cross-platform.
    ///
    /// Red-able: with `.ok()` instead of `parse_database_url`, the `NotUnicode`
    /// case collapses to `None` and this test fails — it receives `Ok(None)`
    /// where it expects `Err(DatabaseUrlNotUnicode)`.
    #[test]
    fn database_url_not_unicode_is_fail_closed() {
        // `VarError::NotUnicode(OsString)` can be constructed on any platform;
        // the `OsString` content is irrelevant — the match arm fires on the
        // variant, not the payload.
        let os_str = std::ffi::OsString::from("any-value");
        let result = parse_database_url(Err(std::env::VarError::NotUnicode(os_str)));
        assert!(
            matches!(result, Err(WorkerConfigError::DatabaseUrlNotUnicode)),
            "a set-but-non-UTF-8 variable must yield DatabaseUrlNotUnicode, not Ok(None)"
        );
    }

    /// Additional unix-only test: verifies the same branch with a genuinely
    /// invalid-UTF-8 byte sequence, proving the real-world trigger path also
    /// reaches `DatabaseUrlNotUnicode` and not a silent `Ok(None)`.
    #[cfg(unix)]
    #[test]
    fn database_url_invalid_utf8_bytes_is_fail_closed() {
        use std::os::unix::ffi::OsStringExt as _;
        let not_utf8 = std::ffi::OsString::from_vec(vec![0xFF]);
        let result = parse_database_url(Err(std::env::VarError::NotUnicode(not_utf8)));
        assert!(
            matches!(result, Err(WorkerConfigError::DatabaseUrlNotUnicode)),
            "invalid-UTF-8 bytes must yield DatabaseUrlNotUnicode, not Ok(None)"
        );
    }

    #[test]
    fn generate_ephemeral_processor_id_produces_distinct_ids() {
        // Two calls must yield different 128-bit ids with overwhelming probability.
        // A collision would require two UUIDs to collide, which has a probability
        // of ~1/2^122 — negligibly small for a test assertion.
        let a = generate_ephemeral_processor_id();
        let b = generate_ephemeral_processor_id();
        assert_ne!(
            a, b,
            "two ephemeral processor ids must be distinct (UUID v4)"
        );
    }
}
