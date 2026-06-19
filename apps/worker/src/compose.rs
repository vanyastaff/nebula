//! Composition root for the core-flavor worker binary.
//!
//! All testable assembly logic lives here; `main.rs` is a thin driver that
//! reads config, builds the SQLite pool, and calls into this module.

use std::sync::Arc;

use nebula_action::result::ActionResult;
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, EngineError, ExecutionStores,
    InProcessRunner, Plugin, PluginKey, PluginWiringError, ResolvedPlugin, WorkflowEngine,
    WorkflowStores,
};
use nebula_metrics::MetricsRegistry;
use nebula_plugin::{ManifestError, PluginError};
use nebula_plugin_core::CorePlugin;
use nebula_storage_port::store::JobDispatchQueue;
use nebula_worker::{WorkerBuildError, WorkerRuntimeBuilder};

/// Typed errors emitted by the composition root.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ComposeError {
    /// The core plugin manifest or key is structurally invalid.
    ///
    /// In practice this should never fire — the `core` key is a compile-time
    /// constant — but the fallible path exists in `CorePlugin::try_new` per the
    /// `PluginManifest::builder().build()` validation contract.
    #[error("core plugin manifest invalid: {0}")]
    Plugin(#[from] ManifestError),

    /// `ResolvedPlugin::from` rejected the plugin (namespace mismatch or
    /// duplicate component key within the `core` namespace).
    #[error("core plugin resolution failed: {0}")]
    Resolve(#[from] PluginError),

    /// `WorkflowEngine::with_plugin` rejected the plugin (duplicate plugin key
    /// or duplicate action key against an already-registered action).
    #[error("plugin wiring into engine failed: {0}")]
    Wiring(#[from] PluginWiringError),

    /// Engine or action-runtime construction failed (metrics registry rejected
    /// counter/histogram registration, or the engine failed to initialize its
    /// shared state).
    #[error("engine / runtime construction failed: {0}")]
    Engine(#[from] EngineError),

    /// `WorkerRuntimeBuilder::build` rejected the assembled configuration.
    ///
    /// `WorkerRuntimeBuilder` construction failed. The only current cause is
    /// `NoPlugins`, which cannot occur here because `build_core_flavor_runtime`
    /// always supplies at least `[core_key]`. The variant is retained so callers
    /// can distinguish a logic regression from the other failure paths.
    #[error("worker runtime builder construction failed: {0}")]
    Worker(#[from] WorkerBuildError),
}

/// Assemble a core-flavor `WorkerRuntime` from the supplied stores and queue.
///
/// Steps performed:
/// 1. Construct and resolve [`CorePlugin`] into a [`ResolvedPlugin`].
/// 2. Build a [`WorkflowEngine`] with an [`ActionRuntime`] and attach the
///    execution + workflow stores.
/// 3. Wire the resolved plugin via [`WorkflowEngine::with_plugin`] so the
///    engine can dispatch `core.*` actions.
/// 4. Build a `WorkerRuntime` via [`WorkerRuntimeBuilder`] with the derived
///    `available_plugins` set.
///
/// Returns a ready-to-configure [`WorkerRuntimeBuilder`], the shared
/// [`MetricsRegistry`] (pass it to `builder.with_metrics` so orchestrator
/// counters land in the same registry as engine counters), and the [`PluginKey`]
/// advertised by this flavor (`"core"`). The engine is captured inside the
/// builder's `Arc<WorkflowEngine>`; callers apply optional tuning via the
/// builder's `with_*` methods and then call `.build()` to materialise the
/// `WorkerRuntime`.
///
/// Returning the builder (not the already-built runtime) lets the caller — most
/// importantly `main` — apply env-driven overrides (`batch_size`,
/// `poll_interval`) before `.build()` without needing to thread extra parameters
/// through this function.
///
/// # Errors
///
/// Returns [`ComposeError`] if any boot step fails. All failures are
/// fail-closed: the process must not start with a mis-wired engine.
pub fn build_core_flavor_runtime(
    execution_stores: ExecutionStores,
    workflow_stores: WorkflowStores,
    queue: Arc<dyn JobDispatchQueue>,
    processor_id: [u8; 16],
) -> Result<(WorkerRuntimeBuilder, MetricsRegistry, PluginKey), ComposeError> {
    // Step 1 — boot and resolve the CorePlugin.
    let core_plugin = CorePlugin::try_new()?;
    let plugin_key = core_plugin.key().clone();
    let resolved = Arc::new(ResolvedPlugin::from(core_plugin)?);

    // Step 2 — build the ActionRuntime and WorkflowEngine.
    //
    // `InProcessRunner` + a no-op executor are the structural boilerplate required
    // by `ActionRuntime::try_new`. The factory-dispatch path (reached via
    // `with_plugin`) does not use the legacy executor; it calls the factory's
    // `create` method directly and drives the produced action through the engine's
    // own dispatch machinery. The no-op executor is present only to satisfy the
    // `ActionRuntime` constructor, which requires it even when all actions arrive
    // via `register_*_factory` / `with_plugin`.
    let metrics = MetricsRegistry::new();
    let registry = Arc::new(ActionRegistry::new());
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runner = Arc::new(InProcessRunner::new(executor));
    // `try_new` returns `Result<_, MetricsError>`; `MetricsError: Into<EngineError>`
    // via `EngineError::Telemetry(#[from] MetricsError)`, so `.map_err` bridges
    // the two error types through the shared `EngineError` wrapper.
    let action_runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .map_err(EngineError::from)?,
    );

    let engine = WorkflowEngine::new(action_runtime, metrics.clone())?
        .with_execution_stores(execution_stores.clone())
        .with_workflow_stores(workflow_stores);

    // Step 3 — wire the CorePlugin into the engine.
    let engine = Arc::new(engine.with_plugin(Arc::clone(&resolved))?);

    // Step 4 — construct the WorkerRuntimeBuilder.
    //
    // The builder is returned to the caller rather than materialised here so
    // `main` can apply env-driven overrides (`batch_size`, `poll_interval`)
    // before calling `.build()`. Integration tests call `.build()` directly
    // on the returned builder, optionally adding a fast poll interval first.
    let builder = WorkerRuntimeBuilder::from_wired_engine(
        Arc::clone(&engine),
        execution_stores,
        queue,
        vec![plugin_key.clone()],
        processor_id,
    );

    tracing::info!(
        plugin = %plugin_key,
        processor = %hex_id(&processor_id),
        "core-flavor: plugin wired, worker runtime builder ready (ADR-0095 D1)"
    );

    Ok((builder, metrics, plugin_key))
}

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
/// All fields are optional with sensible defaults for local/dev runs.
#[derive(Debug)]
pub struct WorkerConfig {
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
    /// same processor id can ack each other's claimed jobs (at-least-once
    /// violation).
    pub processor_id: [u8; 16],

    /// Claim batch size (number of jobs claimed per poll).
    ///
    /// Read from `NEBULA_WORKER_BATCH_SIZE`. Defaults to the orchestrator's
    /// built-in default (32).
    pub batch_size: Option<u32>,

    /// Poll interval in milliseconds.
    ///
    /// Read from `NEBULA_WORKER_POLL_INTERVAL_MS`. Defaults to the
    /// orchestrator's built-in default (100 ms).
    pub poll_interval_ms: Option<u64>,
}

/// Errors produced while loading [`WorkerConfig`] from the environment.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkerConfigError {
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
    /// `NEBULA_WORKER_BATCH_SIZE` is set but cannot be parsed as an integer.
    #[error("NEBULA_WORKER_BATCH_SIZE must be a positive integer; got {raw:?}: {source}")]
    BatchSize {
        /// Raw environment value that failed to parse.
        raw: String,
        /// Underlying parse error.
        #[source]
        source: std::num::ParseIntError,
    },
    /// `NEBULA_WORKER_BATCH_SIZE` parsed successfully but the value is zero.
    ///
    /// A batch size of zero makes the orchestrator issue `LIMIT 0` claims, so
    /// the worker appears healthy but never claims any job.
    #[error(
        "NEBULA_WORKER_BATCH_SIZE must be ≥ 1; got 0 — a zero batch size \
         causes the orchestrator to claim no jobs (silent stall)"
    )]
    BatchSizeNotPositive,
    /// `NEBULA_WORKER_POLL_INTERVAL_MS` is set but cannot be parsed as an integer.
    #[error(
        "NEBULA_WORKER_POLL_INTERVAL_MS must be a positive integer (milliseconds); got {raw:?}: {source}"
    )]
    PollInterval {
        /// Raw environment value that failed to parse.
        raw: String,
        /// Underlying parse error.
        #[source]
        source: std::num::ParseIntError,
    },
    /// `NEBULA_WORKER_POLL_INTERVAL_MS` parsed successfully but the value is zero.
    ///
    /// A poll interval of zero produces a tight busy-loop that hammers the
    /// SQLite store on every empty-queue tick.
    #[error(
        "NEBULA_WORKER_POLL_INTERVAL_MS must be ≥ 1 ms; got 0 — a zero interval \
         causes a tight busy-loop hammering the job-dispatch store"
    )]
    PollIntervalNotPositive,

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
    /// type, or out of the valid range). Absent variables fall back to defaults
    /// and do not error. Zero is rejected for `NEBULA_WORKER_BATCH_SIZE` and
    /// `NEBULA_WORKER_POLL_INTERVAL_MS` at parse time — both would silently
    /// break the claim-loop at runtime.
    pub fn from_env() -> Result<Self, WorkerConfigError> {
        let database_url = parse_database_url(std::env::var("NEBULA_WORKER_DATABASE_URL"))?;

        let db_path = std::env::var("NEBULA_WORKER_DB_PATH")
            .unwrap_or_else(|_| "nebula-worker.db".to_owned());

        let processor_id = match std::env::var("NEBULA_WORKER_PROCESSOR_ID") {
            Ok(raw) => parse_processor_id(&raw)?,
            Err(_) => generate_ephemeral_processor_id(),
        };

        let batch_size = parse_optional_u32("NEBULA_WORKER_BATCH_SIZE", |raw, source| {
            WorkerConfigError::BatchSize {
                raw: raw.to_owned(),
                source,
            }
        })?;
        let batch_size = reject_zero_u32(batch_size, WorkerConfigError::BatchSizeNotPositive)?;

        let poll_interval_ms =
            parse_optional_u64("NEBULA_WORKER_POLL_INTERVAL_MS", |raw, source| {
                WorkerConfigError::PollInterval {
                    raw: raw.to_owned(),
                    source,
                }
            })?;
        let poll_interval_ms =
            reject_zero_u64(poll_interval_ms, WorkerConfigError::PollIntervalNotPositive)?;

        Ok(Self {
            database_url,
            db_path,
            processor_id,
            batch_size,
            poll_interval_ms,
        })
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

/// Parse an optional `u32` from the named env var, mapping errors with `make_err`.
fn parse_optional_u32(
    var: &str,
    make_err: impl Fn(&str, std::num::ParseIntError) -> WorkerConfigError,
) -> Result<Option<u32>, WorkerConfigError> {
    match std::env::var(var) {
        Ok(raw) => raw.parse::<u32>().map(Some).map_err(|e| make_err(&raw, e)),
        Err(_) => Ok(None),
    }
}

/// Parse an optional `u64` from the named env var, mapping errors with `make_err`.
fn parse_optional_u64(
    var: &str,
    make_err: impl Fn(&str, std::num::ParseIntError) -> WorkerConfigError,
) -> Result<Option<u64>, WorkerConfigError> {
    match std::env::var(var) {
        Ok(raw) => raw.parse::<u64>().map(Some).map_err(|e| make_err(&raw, e)),
        Err(_) => Ok(None),
    }
}

/// Reject `Some(0)` from a parsed `Option<u32>`, mapping it to `err`.
///
/// Used to fail-closed at startup when a config value is structurally valid
/// but semantically invalid (zero batch size causes silent stall; zero poll
/// interval causes a tight busy-loop).
fn reject_zero_u32(
    value: Option<u32>,
    err: WorkerConfigError,
) -> Result<Option<u32>, WorkerConfigError> {
    if value == Some(0) {
        Err(err)
    } else {
        Ok(value)
    }
}

/// Reject `Some(0)` from a parsed `Option<u64>`, mapping it to `err`.
fn reject_zero_u64(
    value: Option<u64>,
    err: WorkerConfigError,
) -> Result<Option<u64>, WorkerConfigError> {
    if value == Some(0) {
        Err(err)
    } else {
        Ok(value)
    }
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

    // ── NEBULA_WORKER_BATCH_SIZE zero-rejection ───────────────────────────────
    //
    // These tests call `reject_zero_u32` / `reject_zero_u64` directly — no
    // env-var mutation, no `unsafe`. The helpers are the sole zero-enforcement
    // site in `from_env`; testing them directly gives the same coverage without
    // environment side-effects.
    //
    // Red-able: without the zero-check helper, `from_env` would return
    // `Ok(Some(0))` for a zero value. These tests call the helper that
    // `from_env` delegates to; removing the helper (or the call) makes them
    // panic on an `Ok` where they expect an `Err`.

    #[test]
    fn batch_size_zero_is_rejected() {
        let err = reject_zero_u32(Some(0), WorkerConfigError::BatchSizeNotPositive).unwrap_err();
        assert!(
            matches!(err, WorkerConfigError::BatchSizeNotPositive),
            "expected BatchSizeNotPositive, got: {err}"
        );
    }

    #[test]
    fn batch_size_positive_is_accepted() {
        let result = reject_zero_u32(Some(16), WorkerConfigError::BatchSizeNotPositive);
        assert_eq!(result.unwrap(), Some(16));
    }

    #[test]
    fn batch_size_none_is_accepted() {
        let result = reject_zero_u32(None, WorkerConfigError::BatchSizeNotPositive);
        assert_eq!(
            result.unwrap(),
            None,
            "absent batch_size must pass through as None"
        );
    }

    // ── NEBULA_WORKER_POLL_INTERVAL_MS zero-rejection ─────────────────────────

    #[test]
    fn poll_interval_zero_is_rejected() {
        let err = reject_zero_u64(Some(0), WorkerConfigError::PollIntervalNotPositive).unwrap_err();
        assert!(
            matches!(err, WorkerConfigError::PollIntervalNotPositive),
            "expected PollIntervalNotPositive, got: {err}"
        );
    }

    #[test]
    fn poll_interval_positive_is_accepted() {
        let result = reject_zero_u64(Some(100), WorkerConfigError::PollIntervalNotPositive);
        assert_eq!(result.unwrap(), Some(100));
    }

    #[test]
    fn poll_interval_none_is_accepted() {
        let result = reject_zero_u64(None, WorkerConfigError::PollIntervalNotPositive);
        assert_eq!(
            result.unwrap(),
            None,
            "absent poll_interval_ms must pass through as None"
        );
    }
}
