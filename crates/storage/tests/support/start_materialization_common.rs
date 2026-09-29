//! Shared by `start_materialization` (in-memory + SQLite) and
//! `start_materialization_postgres`: the retained start-authority observation
//! writer.

use std::{fs::OpenOptions, io::BufWriter, path::Path};

pub(crate) fn write_observations(backend: &str, env: &str, observations: serde_json::Value) {
    let Ok(path) = std::env::var(env) else {
        return;
    };
    let path = Path::new(&path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let report = serde_json::json!({
        "producer_version": 1,
        "contract": "start-authority",
        "scenario_inventory_version": 1,
        "backend": backend,
        "observations": observations
    });
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();
    serde_json::to_writer_pretty(BufWriter::new(file), &report).unwrap();
}
