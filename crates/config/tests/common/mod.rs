//! Shared test helpers for nebula-config integration tests.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write content to a uniquely named temporary file and return its path.
///
/// The caller is responsible for removing the file after the test completes.
pub fn write_temp_file(stem: &str, extension: &str, contents: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let counter = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name =
        format!("nebula_config_edge_{stem}_{timestamp}_{counter}.{extension}");
    let path = std::env::temp_dir().join(file_name);
    std::fs::write(&path, contents).expect("should write temporary fixture file");
    path
}

/// Initialise a `tracing` subscriber scoped to the current test.
///
/// Safe to call multiple times; subsequent calls are silently ignored.
pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
}
