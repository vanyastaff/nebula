//! Edge-case tests for the TOML file loader.
//!
//! Covers deeply-nested keys, arrays of tables, Unicode keys/values, and
//! large files (1 000 top-level keys).

mod common;

use nebula_config::{ConfigBuilder, ConfigSource};

// ── Deeply nested key ────────────────────────────────────────────────────────

/// TOML dotted-key syntax produces a deeply nested JSON tree.
/// `Config::get` must traverse all ten levels correctly.
#[tokio::test]
async fn deeply_nested_key_resolves() {
    common::init_tracing();

    // TOML allows chaining dotted keys arbitrarily deep.
    let content = "a.b.c.d.e.f.g.h.i.j = \"deep\"\n";
    let path = common::write_temp_file("nested", "toml", content);

    tracing::debug!(?path, "loading deeply-nested TOML fixture");

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(path.clone()))
        .build()
        .await
        .expect("config should build from deeply-nested TOML");

    let value: String = config
        .get("a.b.c.d.e.f.g.h.i.j")
        .await
        .expect("should resolve a 10-level nested key");

    assert_eq!(value, "deep", "unexpected value at max nesting depth");

    let _ = std::fs::remove_file(&path);
}

// ── Array of tables ──────────────────────────────────────────────────────────

/// `[[entries]]` syntax produces a JSON array; all 50 entries must survive
/// the load-and-deserialise round-trip.
#[tokio::test]
async fn array_of_tables_loads_all_entries() {
    common::init_tracing();

    let mut content = String::new();
    for i in 0u32..50 {
        content.push_str(&format!("[[entries]]\nid = {i}\nname = \"item{i}\"\n\n"));
    }
    let path = common::write_temp_file("array_tables", "toml", &content);

    tracing::debug!(?path, "loading array-of-tables TOML fixture");

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(path.clone()))
        .build()
        .await
        .expect("config should build from array-of-tables TOML");

    let entries: Vec<serde_json::Value> = config
        .get("entries")
        .await
        .expect("should deserialise entries array");

    assert_eq!(
        entries.len(),
        50,
        "all 50 [[entries]] blocks must be preserved"
    );

    // Spot-check first and last element
    assert_eq!(entries[0]["id"], serde_json::json!(0));
    assert_eq!(entries[49]["id"], serde_json::json!(49));

    let _ = std::fs::remove_file(&path);
}

// ── Unicode keys and values ───────────────────────────────────────────────────

/// TOML allows quoted Unicode keys; both key and value must survive intact.
#[tokio::test]
async fn unicode_key_and_value_roundtrip() {
    common::init_tracing();

    // TOML quoted keys support full Unicode.
    let content = "\"café\" = \"latté\"\n\"crab\" = \"🦀\"\n";
    let path = common::write_temp_file("unicode", "toml", content);

    tracing::debug!(?path, "loading Unicode-key TOML fixture");

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(path.clone()))
        .build()
        .await
        .expect("config should build from Unicode-key TOML");

    let latte: String = config
        .get("café")
        .await
        .expect("should retrieve a Unicode key");
    assert_eq!(latte, "latté");

    let crab: String = config
        .get("crab")
        .await
        .expect("should retrieve emoji value");
    assert_eq!(crab, "🦀");

    let _ = std::fs::remove_file(&path);
}

// ── Large file ────────────────────────────────────────────────────────────────

/// Loading a TOML file with 1 000 top-level keys must succeed and return the
/// correct value for the last key (validates no truncation or off-by-one).
#[tokio::test]
async fn large_toml_file_loads_correctly() {
    common::init_tracing();

    let mut content = String::with_capacity(64 * 1024);
    for i in 0u32..1_000 {
        content.push_str(&format!("key_{i:04} = \"value_{i:04}\"\n"));
    }
    let path = common::write_temp_file("large", "toml", &content);

    tracing::debug!(?path, bytes = content.len(), "loading large TOML fixture");

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(path.clone()))
        .build()
        .await
        .expect("config should build from a 1 000-key TOML file");

    // Spot-check a key in the middle and the very last key.
    let mid: String = config
        .get("key_0499")
        .await
        .expect("should read a middle key");
    assert_eq!(mid, "value_0499");

    let last: String = config
        .get("key_0999")
        .await
        .expect("should read the last key in a large file");
    assert_eq!(last, "value_0999");

    let _ = std::fs::remove_file(&path);
}
