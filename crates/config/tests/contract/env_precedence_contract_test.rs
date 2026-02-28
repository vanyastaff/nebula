use super::helpers::{unique_env_prefix, write_temp_file};
use nebula_config::{ConfigBuilder, ConfigSource};
use serde_json::json;

#[tokio::test]
async fn env_overrides_file_and_defaults() {
    let prefix = unique_env_prefix();
    let env_key = format!("{prefix}_SERVICE_PORT");
    let file_path = write_temp_file(
        "env_precedence",
        "json",
        r#"{"service":{"port":7000,"host":"file-host"}}"#,
    );

    // SAFETY: contract tests use unique variable keys to avoid collisions.
    unsafe { std::env::set_var(&env_key, "9100") };

    let config = ConfigBuilder::new()
        .with_defaults_json(json!({"service":{"port":3000,"host":"default-host"}}))
        .with_source(ConfigSource::File(file_path))
        .with_source(ConfigSource::EnvWithPrefix(prefix.clone()))
        .build()
        .await
        .expect("config should build from defaults + file + env");

    let port: u16 = config
        .get("service.port")
        .await
        .expect("typed get should work");
    assert_eq!(port, 9100);

    let host: String = config
        .get("service.host")
        .await
        .expect("host should come from file layer");
    assert_eq!(host, "file-host");

    // SAFETY: cleanup of a unique test-only env variable.
    unsafe { std::env::remove_var(env_key) };
}
