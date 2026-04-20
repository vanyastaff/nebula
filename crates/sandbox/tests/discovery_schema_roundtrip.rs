//! End-to-end: schema round-trips from plugin-side declaration into
//! host-side ActionMetadata via discover_directory.
//!
//! Verifies that the two-field schema declared by `nebula-plugin-schema-fixture`
//! (`name`: required string, `age`: optional number) survives the full broker
//! envelope round-trip and appears in the host-side `ActionMetadata.base.schema`
//! after `discover_directory`.
//!
//! # Running
//!
//! Build the fixture binary first, then run with `--run-ignored all`:
//!
//! ```bash
//! cargo build -p nebula-plugin-sdk --bin nebula-plugin-schema-fixture
//! cargo nextest run -p nebula-sandbox --test discovery_schema_roundtrip --run-ignored all
//! ```
//!
//! CI must do the same: build the fixture, then invoke nextest with
//! `--run-ignored all`.

use std::{path::PathBuf, time::Duration};

use nebula_plugin::PluginRegistry;
use nebula_sandbox::{capabilities::PluginCapabilities, discovery};
use nebula_schema::field_key;

fn fixture_binary_path() -> PathBuf {
    let bin_name = if cfg!(windows) {
        "nebula-plugin-schema-fixture.exe"
    } else {
        "nebula-plugin-schema-fixture"
    };
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap() // crates/
        .parent()
        .unwrap() // workspace root
        .join("target")
        .join(profile)
        .join(bin_name)
}

#[tokio::test]
#[ignore = "requires pre-built fixture binary; run `cargo build -p nebula-plugin-sdk --bin nebula-plugin-schema-fixture` first, then `cargo nextest run -p nebula-sandbox --test discovery_schema_roundtrip --run-ignored all`"]
async fn discovery_roundtrips_action_schema() {
    let src_binary = fixture_binary_path();
    assert!(
        src_binary.exists(),
        "fixture binary not built: {}",
        src_binary.display(),
    );

    let scan_dir = tempfile::tempdir().unwrap();

    let dest_binary = scan_dir.path().join(src_binary.file_name().unwrap());
    std::fs::copy(&src_binary, &dest_binary).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest_binary).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest_binary, perms).unwrap();
    }

    // sdk = "*" matches any host version.
    std::fs::write(
        scan_dir.path().join("plugin.toml"),
        "[nebula]\nsdk = \"*\"\n",
    )
    .unwrap();

    let mut registry = PluginRegistry::new();
    discovery::discover_directory(
        scan_dir.path(),
        &mut registry,
        Duration::from_secs(5),
        PluginCapabilities::none(),
    )
    .await;

    let plugin_key = "com.author.schema".parse().unwrap();
    let plugin = registry
        .get(&plugin_key)
        .expect("plugin should be registered after discovery");

    let action_key = nebula_core::ActionKey::new("com.author.schema.describe").unwrap();
    let action = plugin
        .action(&action_key)
        .expect("describe action should be present");

    let schema = &action.metadata().base.schema;

    // Both fields must round-trip through the broker wire envelope.
    let name_key = field_key!("name");
    let age_key = field_key!("age");

    assert!(
        schema.find(&name_key).is_some(),
        "schema must contain 'name' field after discovery round-trip"
    );
    assert!(
        schema.find(&age_key).is_some(),
        "schema must contain 'age' field after discovery round-trip"
    );
    assert_eq!(
        schema.fields().len(),
        2,
        "schema must have exactly 2 fields"
    );
}
