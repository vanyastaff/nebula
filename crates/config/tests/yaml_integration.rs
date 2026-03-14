//! Integration tests for YAML configuration loading
#![cfg(feature = "yaml")]

use nebula_config::{ConfigBuilder, ConfigFormat, ConfigSource};
use std::io::Write;
use tempfile::NamedTempFile;

fn write_yaml_tempfile(content: &str) -> NamedTempFile {
    let mut f = tempfile::Builder::new()
        .suffix(".yaml")
        .tempfile()
        .expect("create temp YAML file");
    f.write_all(content.as_bytes())
        .expect("write temp YAML file");
    f
}

#[tokio::test]
async fn yaml_file_loads_via_config_builder() {
    let yaml = "server:\n  host: localhost\n  port: 8080\n";
    let tmp = write_yaml_tempfile(yaml);

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(tmp.path().to_path_buf()))
        .build()
        .await
        .expect("build config from YAML");

    assert_eq!(
        config.get::<String>("server.host").await.unwrap(),
        "localhost"
    );
    assert_eq!(config.get::<i64>("server.port").await.unwrap(), 8080);
}

#[tokio::test]
async fn yaml_nested_structures() {
    let yaml = r#"
database:
  primary:
    host: db.example.com
    port: 5432
  replicas:
    - host: replica1.example.com
      port: 5432
    - host: replica2.example.com
      port: 5433
"#;
    let tmp = write_yaml_tempfile(yaml);

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(tmp.path().to_path_buf()))
        .build()
        .await
        .expect("build config from nested YAML");

    assert_eq!(
        config.get::<String>("database.primary.host").await.unwrap(),
        "db.example.com"
    );
    assert_eq!(
        config.get::<i64>("database.primary.port").await.unwrap(),
        5432
    );
}

#[tokio::test]
async fn yaml_unicode_keys() {
    let yaml = "日本語キー: value\nemoji_🚀: launch\n";
    let tmp = write_yaml_tempfile(yaml);

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(tmp.path().to_path_buf()))
        .build()
        .await
        .expect("build config from YAML with unicode keys");

    let data = config.get_raw(None).await.unwrap();
    assert_eq!(data["日本語キー"], "value");
    assert_eq!(data["emoji_🚀"], "launch");
}

#[tokio::test]
async fn yaml_overrides_toml_in_composite() {
    // Base TOML
    let mut toml_file = tempfile::Builder::new()
        .suffix(".toml")
        .tempfile()
        .expect("create temp TOML file");
    toml_file
        .write_all(b"[server]\nhost = \"default\"\nport = 3000\n")
        .unwrap();

    // Override YAML
    let yaml = "server:\n  host: override-host\n";
    let yaml_file = write_yaml_tempfile(yaml);

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(toml_file.path().to_path_buf()))
        .with_source(ConfigSource::File(yaml_file.path().to_path_buf()))
        .build()
        .await
        .expect("build composite config");

    assert_eq!(
        config.get::<String>("server.host").await.unwrap(),
        "override-host"
    );
}

#[tokio::test]
async fn yaml_format_detection_yml_extension() {
    let mut f = tempfile::Builder::new()
        .suffix(".yml")
        .tempfile()
        .expect("create temp .yml file");
    f.write_all(b"key: value\n").unwrap();

    let format = ConfigFormat::from_path(f.path());
    assert_eq!(format, ConfigFormat::Yaml);

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(f.path().to_path_buf()))
        .build()
        .await
        .expect("build config from .yml");

    assert_eq!(config.get::<String>("key").await.unwrap(), "value");
}
