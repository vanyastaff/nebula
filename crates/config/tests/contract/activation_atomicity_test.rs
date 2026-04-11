use std::sync::Arc;

use nebula_config::{ConfigBuilder, ConfigError, ConfigSource, core::ConfigValidator};

use super::helpers::write_temp_file;

struct ClosureValidator<F>(F);
#[async_trait::async_trait]
impl<F: Fn(&serde_json::Value) -> nebula_config::ConfigResult<()> + Send + Sync> ConfigValidator
    for ClosureValidator<F>
{
    async fn validate(&self, data: &serde_json::Value) -> nebula_config::ConfigResult<()> {
        (self.0)(data)
    }
}

#[tokio::test]
async fn activation_is_atomic_on_reload() {
    let path = write_temp_file(
        "atomicity",
        "toml",
        "[service]\nhost = \"127.0.0.1\"\nport = 8080\n",
    );

    let validator = ClosureValidator(|value: &serde_json::Value| {
        let host_ok = value
            .get("service")
            .and_then(|service| service.get("host"))
            .and_then(serde_json::Value::as_str)
            .is_some();
        let port_ok = value
            .get("service")
            .and_then(|service| service.get("port"))
            .and_then(serde_json::Value::as_u64)
            .is_some();
        if host_ok && port_ok {
            Ok(())
        } else {
            Err(ConfigError::validation(
                "service.host and service.port are both required",
            ))
        }
    });

    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File(path.clone()))
        .with_validator(Arc::new(validator))
        .build()
        .await
        .expect("initial valid config should build");

    std::fs::write(&path, "[service]\nhost = \"10.0.0.1\"\n")
        .expect("should write invalid candidate missing port");

    assert!(config.reload().await.is_err());

    let host: String = config
        .get("service.host")
        .await
        .expect("host should remain");
    let port: u16 = config
        .get("service.port")
        .await
        .expect("port should remain");
    assert_eq!(host, "127.0.0.1");
    assert_eq!(port, 8080);
}
