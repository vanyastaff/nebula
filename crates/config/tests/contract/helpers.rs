use async_trait::async_trait;
use nebula_config::{
    ConfigError, ConfigFormat, ConfigLoader, ConfigResult, ConfigSource, SourceMetadata,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Default)]
pub struct StaticFixtureLoader {
    payloads: HashMap<ConfigSource, Value>,
}

impl StaticFixtureLoader {
    pub fn with_payload(mut self, source: ConfigSource, payload: Value) -> Self {
        self.payloads.insert(source, payload);
        self
    }
}

#[async_trait]
impl ConfigLoader for StaticFixtureLoader {
    async fn load(&self, source: &ConfigSource) -> ConfigResult<Value> {
        self.payloads.get(source).cloned().ok_or_else(|| {
            ConfigError::source_error(
                format!("no fixture payload for source: {source}"),
                source.name(),
            )
        })
    }

    fn supports(&self, source: &ConfigSource) -> bool {
        self.payloads.contains_key(source)
    }

    async fn metadata(&self, source: &ConfigSource) -> ConfigResult<SourceMetadata> {
        Ok(SourceMetadata::new(source.clone()).with_format(ConfigFormat::Json))
    }
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn write_temp_file(stem: &str, extension: &str, contents: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let counter = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = format!("nebula_config_{stem}_{timestamp}_{counter}.{extension}");
    let path = std::env::temp_dir().join(file_name);
    std::fs::write(&path, contents).expect("should write temporary fixture file");
    path
}

pub fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(relative)
}

pub fn unique_env_prefix() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    format!("NEBULA_CONFIG_CONTRACT_{timestamp}")
}

pub fn assert_validation_failed(err: &ConfigError) {
    assert!(matches!(err, ConfigError::ValidationError { .. }));
    assert_eq!(
        err.contract_category(),
        nebula_config::core::error::ContractErrorCategory::ValidationFailed
    );
}

pub fn assert_contract_category(err: &ConfigError, expected: &'static str) {
    assert_eq!(err.contract_category().as_str(), expected);
}
