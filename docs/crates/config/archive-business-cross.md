# Archived From "docs/archive/business-cross.md"

### nebula-config
**Назначение:** Унифицированная система конфигурации с hot-reload.

**Ключевые возможности:**
- Множественные форматы (TOML/YAML/JSON)
- Environment variables override
- Hot-reload
- Schema validation

```rust
// Определение конфигурации
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct EngineConfig {
    pub max_concurrent_executions: usize,
    pub default_timeout: Duration,
    pub retry_policy: RetryConfig,
}

impl Configurable for EngineConfig {
    fn config_prefix() -> &'static str { "engine" }
    
    fn validate(&self) -> Result<()> {
        ensure!(self.max_concurrent_executions > 0, "Invalid concurrency");
        Ok(())
    }
}

// config.toml
/*
[engine]
max_concurrent_executions = 100
default_timeout = "5m"

[engine.retry_policy]
max_attempts = 3
strategy = "exponential"

[database]
url = "postgres://localhost/nebula"
max_connections = 50
*/

// Использование
let config = ConfigManager::load("config.toml").await?;
config.enable_hot_reload().await?;

let engine_config: EngineConfig = config.get()?;

// Подписка на изменения
config.on_reload(|new_config| async move {
    engine.update_config(new_config).await;
});
```

---

