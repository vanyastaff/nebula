//! Shared by `ordered_migration_observations` (SQLite) and
//! `ordered_migration_observations_postgres`: the retained-observation writer and
//! the migration-ledger constants both backends record against.

use std::io::Write;

pub(crate) use serde_json::{Value, json};

pub(crate) const PREVIOUS_SUPPORTED: i64 = 45;
/// Both catalogs end at 0064. PostgreSQL carries a migration SQLite reserves
/// (0060 rate limits), so the SQLite ledger skips it on the way to the head.
const CURRENT_HEAD: i64 = 64;

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").unwrap();
            output
        },
    )
}

pub(crate) fn retain(
    variable: &str,
    backend: &str,
    database_version: String,
    scenarios: Vec<Value>,
) {
    if let Some(path) = std::env::var_os(variable) {
        let path = std::path::Path::new(&path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let bytes = serde_json::to_vec_pretty(&json!({
            "producer_version": 2,
            "contract": "ordered-migrations",
            "scenario_inventory_version": 1,
            "backend": backend,
            "database_version": database_version,
            "previous_supported_version": PREVIOUS_SUPPORTED,
            "current_head": CURRENT_HEAD,
            "scenarios": scenarios,
        }))
        .unwrap();
        assert!(bytes.len() <= 512 * 1024);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
    }
}
