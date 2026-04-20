//! Parser tests for `plugin.toml` per canon §7.1.

use std::path::PathBuf;

use nebula_sandbox::plugin_toml::{PluginTomlError, parse_plugin_toml};

fn write(contents: &str) -> tempfile::NamedTempFile {
    use std::io::Write;
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(contents.as_bytes()).unwrap();
    f.flush().unwrap();
    f
}

#[test]
fn parse_minimal_plugin_toml() {
    let f = write(
        r#"
        [nebula]
        sdk = "^0.8"
    "#,
    );
    let m = parse_plugin_toml(f.path()).unwrap();
    assert_eq!(m.sdk.to_string(), "^0.8");
    assert!(m.plugin_id.is_none());
}

#[test]
fn parse_plugin_toml_with_optional_id() {
    let f = write(
        r#"
        [nebula]
        sdk = "^0.8"

        [plugin]
        id = "com.author.slack"
    "#,
    );
    let m = parse_plugin_toml(f.path()).unwrap();
    assert_eq!(m.plugin_id.as_deref(), Some("com.author.slack"));
}

#[test]
fn missing_file_errors() {
    let err = parse_plugin_toml(&PathBuf::from("/this/does/not/exist/plugin.toml")).unwrap_err();
    assert!(matches!(err, PluginTomlError::Missing { .. }));
}

#[test]
fn missing_sdk_constraint_errors() {
    let f = write("[nebula]");
    let err = parse_plugin_toml(f.path()).unwrap_err();
    assert!(matches!(err, PluginTomlError::MissingSdkConstraint { .. }));
}

#[test]
fn invalid_toml_errors() {
    let f = write("this is not toml = = ==");
    let err = parse_plugin_toml(f.path()).unwrap_err();
    assert!(matches!(err, PluginTomlError::InvalidToml { .. }));
}

#[test]
fn invalid_sdk_constraint_errors() {
    let f = write(
        r#"
        [nebula]
        sdk = "not-a-semver-req"
    "#,
    );
    let err = parse_plugin_toml(f.path()).unwrap_err();
    assert!(matches!(err, PluginTomlError::InvalidSdkConstraint { .. }));
}
