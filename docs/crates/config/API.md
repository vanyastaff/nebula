# API Reference — `nebula-config`

Public surface based on `crates/config/src/*`.

## Quick Start

```rust
use nebula_config::prelude::*;

let cfg = ConfigBuilder::new()
    .with_defaults_json(serde_json::json!({"server": {"port": 3000}}))
    .with_source(ConfigSource::File("config.toml".into()))
    .with_source(ConfigSource::Env)
    .with_hot_reload(true)
    .build()
    .await?;

let port: u16 = cfg.get("server.port").await?;
```

---

## Public Surface

- **Stable APIs:** `ConfigBuilder`, `Config`, `ConfigSource`, `ConfigFormat`, `SourceMetadata`, `ConfigLoader`, `ConfigValidator`, `ConfigWatcher`, `ConfigError`, `ConfigResult`, `ConfigResultExt`, `ConfigResultAggregator`, `try_sources`, watcher types, validator implementations, `builders`, `utils`
- **Experimental:** `ConfigSource::Remote`, `Database`, `KeyValue` source variants (not fully implemented by default loaders — load via `SourceError`)
- **Deprecated:** `Config::get_path` (since 0.2.0 → use `get`)
- **Feature flags:** `json`, `toml`, `yaml`, `env` (all enabled by default)

---

## `ConfigBuilder`

Assembly point for configuration. Build once, share `Config` as a dependency.

```rust
pub struct ConfigBuilder { /* ... */ }
```

All builder methods are `#[must_use]`.

| Method | Signature | Notes |
|--------|-----------|-------|
| `new` | `fn new() -> Self` | Also `Default::default()` |
| `with_source` | `fn with_source(self, source: ConfigSource) -> Self` | Appends one source |
| `with_sources` | `fn with_sources(self, sources: Vec<ConfigSource>) -> Self` | Extends sources |
| `with_defaults` | `fn with_defaults<T: Serialize>(self, defaults: T) -> ConfigResult<Self>` | Typed defaults via serde |
| `with_defaults_json` | `fn with_defaults_json(self, defaults: Value) -> Self` | Raw JSON defaults |
| `with_loader` | `fn with_loader(self, loader: Arc<dyn ConfigLoader>) -> Self` | Override default `CompositeLoader` |
| `with_validator` | `fn with_validator(self, validator: Arc<dyn ConfigValidator>) -> Self` | Gated validation |
| `with_watcher` | `fn with_watcher(self, watcher: Arc<dyn ConfigWatcher>) -> Self` | File/polling watcher |
| `with_hot_reload` | `fn with_hot_reload(self, enabled: bool) -> Self` | Enables watcher on `build()` |
| `with_auto_reload_interval` | `fn with_auto_reload_interval(self, interval: Duration) -> Self` | Spawns Tokio task on `build()` |
| `with_fail_on_missing` | `fn with_fail_on_missing(self, fail: bool) -> Self` | Default: false (optional sources silently skipped) |
| `build` | `async fn build(self) -> ConfigResult<Config>` | Validates, loads, merges, activates |

**`build()` behavior:**
1. Validates: at least one source or defaults must be provided.
2. Sorts sources by priority (lower priority number = higher override power; see `ConfigSource::priority()`).
3. Loads all non-`Default` sources concurrently via `join_all`.
4. Merges in priority order: objects merged recursively, scalars/arrays replaced.
5. Runs validator (if set); rejects candidate on failure.
6. Starts watcher (if `with_hot_reload(true)`).
7. Spawns auto-reload task (if `with_auto_reload_interval` set); task is cancelled on `Config` drop.

---

## `Config`

Thread-safe runtime configuration container. `Clone`-able (all internal state is `Arc`-wrapped).

```rust
#[derive(Clone)]
pub struct Config { /* data: Arc<RwLock<Value>>, ... */ }
```

### Typed Access

| Method | Signature | Notes |
|--------|-----------|-------|
| `get` | `async fn get<T: DeserializeOwned>(&self, path: &str) -> ConfigResult<T>` | Dot-separated path |
| `get_all` | `async fn get_all<T: DeserializeOwned>(&self) -> ConfigResult<T>` | Deserializes entire config |
| `get_or` | `async fn get_or<T: DeserializeOwned>(&self, path: &str, default: T) -> T` | Returns default on error |
| `get_or_else` | `async fn get_or_else<T: DeserializeOwned, F: FnOnce() -> T>(&self, path: &str, f: F) -> T` | Lazy default |
| `get_opt` | `async fn get_opt<T: DeserializeOwned>(&self, path: &str) -> Option<T>` | `None` on any error |
| `has` | `async fn has(&self, path: &str) -> bool` | Path existence check |
| `get_path` | `async fn get_path<T: DeserializeOwned>(&self, path: &str) -> ConfigResult<T>` | **Deprecated** since 0.2.0 → use `get` |

### Raw Access

| Method | Signature | Notes |
|--------|-----------|-------|
| `get_raw` | `async fn get_raw(&self, path: Option<&str>) -> ConfigResult<Value>` | `None` = root |
| `get_value` | `async fn get_value(&self, path: &str) -> ConfigResult<Value>` | Clone of nested value |
| `as_value` | `async fn as_value(&self) -> Value` | Clone of entire tree |
| `keys` | `async fn keys(&self, path: Option<&str>) -> ConfigResult<Vec<String>>` | Object keys at path; error if not object |
| `flatten` | `async fn flatten(&self) -> HashMap<String, Value>` | Dot-notation keys; arrays use `[i]` suffix |

### Mutation

| Method | Signature | Notes |
|--------|-----------|-------|
| `set_value` | `async fn set_value(&self, path: &str, value: Value) -> ConfigResult<()>` | Creates intermediate objects |
| `set_typed` | `async fn set_typed<T: Serialize>(&self, path: &str, value: T) -> ConfigResult<()>` | Serializes then sets |
| `merge` | `async fn merge(&self, value: Value) -> ConfigResult<()>` | Recursive merge into current data |

### Lifecycle

| Method | Signature | Notes |
|--------|-----------|-------|
| `reload` | `async fn reload(&self) -> ConfigResult<()>` | Re-loads all sources concurrently; validator-gated; atomic swap |
| `start_watching` | `async fn start_watching(&self) -> ConfigResult<()>` | No-op if hot_reload disabled |
| `stop_watching` | `async fn stop_watching(&self) -> ConfigResult<()>` | Cancels background tasks + watcher |
| `is_watching` | `fn is_watching(&self) -> bool` | |
| `sources` | `fn sources(&self) -> &[ConfigSource]` | |

### Metadata

| Method | Signature |
|--------|-----------|
| `get_metadata` | `fn get_metadata(&self, source: &ConfigSource) -> Option<SourceMetadata>` |
| `get_all_metadata` | `fn get_all_metadata(&self) -> HashMap<ConfigSource, SourceMetadata>` |

**Path syntax:** Dot-separated keys for nested objects (`"server.port"`). Numeric indices for arrays (`"arr.1.name"`, `"arr.0"`). Empty path returns root.

**Reload semantics:** Reload starts from `defaults` captured at build time, then re-loads all non-`Default` sources concurrently, merges in priority order, validates, then atomically swaps the internal `RwLock<Value>`. If validation fails, the previous state is preserved.

**Drop behavior:** `Config::drop` cancels the `CancellationToken`, stopping all auto-reload tasks.

---

## `ConfigSource`

```rust
#[non_exhaustive]
pub enum ConfigSource { /* 11 variants */ }
```

| Variant | Priority (lower = higher override) | Optional? |
|---------|------|----------|
| `Inline(String)` | 1 | No |
| `Database { url, table, key }` | 5 | No |
| `KeyValue { url, bucket }` | 5 | No |
| `Remote(String)` | 10 | No |
| `CommandLine` | 20 | No |
| `Env` | 30 | Yes |
| `EnvWithPrefix(String)` | 30 | Yes |
| `Directory(PathBuf)` | 40 | No |
| `File(PathBuf)` | 50 | No |
| `FileAuto(PathBuf)` | 50 | No |
| `Default` | 100 | Yes |

Optional sources that fail to load are silently skipped (unless `with_fail_on_missing(true)`).

**Methods:**

| Method | Notes |
|--------|-------|
| `is_file_based()` | `File`, `FileAuto`, `Directory` |
| `is_env_based()` | `Env`, `EnvWithPrefix` |
| `is_remote()` | `Remote` |
| `is_database()` | `Database` |
| `is_key_value()` | `KeyValue` |
| `is_optional()` | `Env`, `EnvWithPrefix`, `Default` |
| `priority() -> u8` | See table above |
| `name() -> &'static str` | Human-readable short name |
| `Display` | Human-readable long description |

---

## `ConfigFormat`

```rust
#[non_exhaustive]
pub enum ConfigFormat { Json, Toml, Yaml, Ini, Hcl, Properties, Env, Unknown(String) }
```

| Method | Notes |
|--------|-------|
| `extension() -> &str` | `"json"`, `"toml"`, `"yml"`, `"ini"`, `"hcl"`, `"properties"`, `"env"` |
| `mime_type() -> &str` | `"application/json"`, `"application/toml"`, `"application/x-yaml"`, etc. |
| `from_extension(ext: &str) -> Self` | Case-insensitive; `".yml"`/`".yaml"` both → `Yaml`; `".cfg"` → `Ini`; `".tf"` → `Hcl`; `".props"` → `Properties`; unknown → `Unknown` |
| `from_path(path: &Path) -> Self` | Delegates to `from_extension` on file extension |

Feature-gated: if the corresponding feature flag is disabled, `from_extension` returns `Unknown` for that format.

---

## `SourceMetadata`

```rust
pub struct SourceMetadata {
    pub source: ConfigSource,
    pub last_modified: Option<DateTime<Utc>>,
    pub version: Option<String>,
    pub checksum: Option<String>,
    pub size: Option<u64>,
    pub format: Option<ConfigFormat>,
    pub encoding: Option<String>,
    pub compression: Option<String>,
    pub encryption: Option<String>,
    pub extra: HashMap<String, Value>,
}
```

Builder: `new(source)`, `with_last_modified`, `with_version`, `with_checksum`, `with_size`, `with_format`, `with_encoding`, `with_compression`, `with_encryption`, `with_extra`. All builder methods `#[must_use]`.

---

## Errors

### `ConfigError`

```rust
#[non_exhaustive]
pub enum ConfigError { /* 15 variants */ }
```

| Variant | Fields | Category | Recoverable? |
|---------|--------|----------|-------------|
| `FileNotFound { path }` | PathBuf | NotFound | Yes |
| `FileReadError { path, message }` | PathBuf, String | Io | No |
| `ParseError { path, message }` | PathBuf, String | Parse | No |
| `ValidationError { message, field }` | String, Option\<String\> | Validation | Yes |
| `SourceError { message, origin }` | String, String | Operation | No |
| `EnvVarNotFound { name }` | String | NotFound | Yes |
| `EnvVarParseError { name, value }` | String, String | Parse | No |
| `ReloadError { message }` | String | Operation | No |
| `WatchError { message }` | String | Io | No |
| `MergeError { message }` | String | Operation | No |
| `TypeError { message, expected, actual }` | String, String, String | Validation | No |
| `PathError { message, path }` | String, String | Operation | No |
| `FormatNotSupported { format }` | String | Parse | No |
| `EncryptionError { message }` | String | Security | No |
| `DecryptionError { message }` | String | Security | No |

### Constructors

| Constructor | Signature |
|-------------|-----------|
| `file_not_found` | `fn file_not_found(path: impl Into<PathBuf>) -> Self` |
| `file_read_error` | `fn file_read_error(path, message) -> Self` |
| `parse_error` | `fn parse_error(path, message) -> Self` |
| `validation_error` | `fn validation_error(message, field: Option<String>) -> Self` |
| `validation` | `fn validation(message) -> Self` (field = None) |
| `validation_with_field` | `fn validation_with_field(message, field) -> Self` |
| `source_error` | `fn source_error(message, origin) -> Self` |
| `env_var_not_found` | `fn env_var_not_found(name) -> Self` |
| `env_var_parse_error` | `fn env_var_parse_error(name, value) -> Self` |
| `reload_error` | `fn reload_error(message) -> Self` |
| `watch_error` | `fn watch_error(message) -> Self` |
| `merge_error` | `fn merge_error(message) -> Self` |
| `type_error` | `fn type_error(message, expected, actual) -> Self` |
| `path_error` | `fn path_error(message, path) -> Self` |
| `format_not_supported` | `fn format_not_supported(format) -> Self` |
| `encryption_error` | `fn encryption_error(message) -> Self` |
| `decryption_error` | `fn decryption_error(message) -> Self` |
| `not_found` | `fn not_found(resource_type, resource_id) -> Self` (dispatches to `file_not_found`, `env_var_not_found`, or `source_error`) |
| `internal` | `fn internal(message) -> Self` (wraps as `SourceError { origin: "internal" }`) |

### Classification Methods

```rust
fn is_recoverable(&self) -> bool    // FileNotFound, EnvVarNotFound, ValidationError
fn is_missing_source(&self) -> bool // FileNotFound, EnvVarNotFound
fn category(&self) -> ErrorCategory
fn contract_category(&self) -> ContractErrorCategory
```

### `ErrorCategory`

```rust
#[non_exhaustive]
pub enum ErrorCategory { NotFound, Io, Parse, Validation, Operation, Security }
```

Mapping: `FileNotFound`/`EnvVarNotFound` → `NotFound`; `FileReadError`/`WatchError` → `Io`; `ParseError`/`EnvVarParseError`/`FormatNotSupported` → `Parse`; `ValidationError`/`TypeError` → `Validation`; `SourceError`/`ReloadError`/`MergeError`/`PathError` → `Operation`; `EncryptionError`/`DecryptionError` → `Security`.

### `ContractErrorCategory`

```rust
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ContractErrorCategory {
    SourceLoadFailed, MergeFailed, ValidationFailed,
    MissingPath, TypeMismatch, InvalidValue, WatcherFailed,
}
```

`as_str()` returns the snake_case string (stable, suitable for fixtures and cross-crate contracts).

### `From` Implementations

| Source | Target |
|--------|--------|
| `std::io::Error` (NotFound) | `FileNotFound { path: "unknown" }` |
| `std::io::Error` (PermissionDenied) | `FileReadError` |
| `std::io::Error` (other) | `FileReadError` |
| `serde_json::Error` | `ParseError { path: "json" }` |
| `toml::de::Error` (feature `toml`) | `ParseError { path: "toml" }` |
| `yaml_rust2::ScanError` (feature `yaml`) | `ParseError { path: "yaml" }` |
| `notify::Error` | `WatchError` |

### `ConfigResult<T>`

```rust
pub type ConfigResult<T> = Result<T, ConfigError>;
```

---

## `ConfigResultExt`

Extension trait on `ConfigResult<T>`:

```rust
pub trait ConfigResultExt<T> {
    fn with_context<F: FnOnce() -> String>(self, f: F) -> ConfigResult<T>;
    fn map_config_error<F: FnOnce(ConfigError) -> ConfigError>(self, f: F) -> ConfigResult<T>;
    fn log_error(self) -> Option<T>;    // logs via nebula_log::error!
    fn handle_error<F: FnOnce(&ConfigError)>(self, f: F) -> Option<T>;
}
```

---

## `ConfigResultAggregator`

Collects multiple results and aggregates errors:

```rust
let mut agg = ConfigResultAggregator::with_context("validation phase");
agg.add(result1);
agg.check(result2);
agg.finish()?;  // Err if any errors; multi-error wraps into ValidationError
```

Methods: `new()`, `with_context(ctx)`, `add(result) -> Option<T>`, `check(result) -> bool`, `has_errors()`, `error_count()`, `errors()`, `finish() -> ConfigResult<()>`.

---

## `try_sources`

```rust
pub async fn try_sources<F, T, Fut>(
    sources: &[ConfigSource],
    f: F,
) -> ConfigResult<T>
where
    F: FnMut(&ConfigSource) -> Fut,
    Fut: Future<Output = ConfigResult<T>>,
```

Tries each source in order; returns the first success or the last error.

---

## Traits

### `ConfigLoader`

```rust
#[async_trait]
pub trait ConfigLoader: Send + Sync {
    async fn load(&self, source: &ConfigSource) -> ConfigResult<Value>;
    fn supports(&self, source: &ConfigSource) -> bool;
    async fn metadata(&self, source: &ConfigSource) -> ConfigResult<SourceMetadata>;
}
```

### `ConfigValidator`

```rust
#[async_trait]
pub trait ConfigValidator: Send + Sync {
    async fn validate(&self, data: &Value) -> ConfigResult<()>;
    fn schema(&self) -> Option<Value> { None }
    fn rules(&self) -> Option<String> { None }
}
```

**Blanket impl:** Any `T: Validate<Value> + Send + Sync` (from `nebula-validator`) automatically implements `ConfigValidator`. The bridge maps `ValidationError` → `ConfigError::ValidationError`.

### `ConfigWatcher`

```rust
#[async_trait]
pub trait ConfigWatcher: Send + Sync {
    async fn start_watching(&self, sources: &[ConfigSource]) -> ConfigResult<()>;
    async fn stop_watching(&self) -> ConfigResult<()>;
    fn is_watching(&self) -> bool;
}
```

### `Validatable`

```rust
pub trait Validatable: Send + Sync {
    fn validate(&self) -> Result<(), ConfigError>;
    fn default_config() -> Self where Self: Sized;
    fn merge(&mut self, other: Self) where Self: Sized;
    fn is_valid(&self) -> bool { self.validate().is_ok() }
}
```

### `Configurable`

```rust
pub trait Configurable: Send + Sync {
    type Config: Validatable;
    fn configure(&mut self, config: Self::Config) -> Result<(), ConfigError>;
    fn configuration(&self) -> &Self::Config;
    fn reset_config(&mut self) -> Result<(), ConfigError>;  // default: configure(default_config())
}
```

### `AsyncConfigurable`

Same as `Configurable` but async (`async fn configure`, `async fn reset_config`).

---

## Implementations

### Loaders

| Type | Source | Notes |
|------|--------|-------|
| `FileLoader` | `File`, `FileAuto`, `Directory` | Parses JSON/TOML/YAML/INI/HCL/Properties/env based on extension |
| `EnvLoader` (feature `env`) | `Env`, `EnvWithPrefix` | Maps `__` → `.` for nesting; prefix stripped |
| `CompositeLoader` | All | Default loader; delegates to `FileLoader` + `EnvLoader` |

`CompositeLoader::default()` — includes all available loaders. `CompositeLoader::default_loaders()` — same.

### Validators

| Type | Description |
|------|-------------|
| `NoOpValidator` | Always passes; useful as placeholder |
| `FunctionValidator` | Wraps a `fn(&Value) -> ConfigResult<()>` closure |
| `SchemaValidator` | Validates against a JSON schema (`serde_json::Value`); `schema()` returns it |
| `CompositeValidator` | Runs multiple validators; fails on first failure |

### Watchers

| Type | Description |
|------|-------------|
| `NoOpWatcher` | No-op; `is_watching()` always false |
| `FileWatcher` | Uses `notify` crate; callback receives `ConfigWatchEvent` |
| `PollingWatcher` | Polls at interval; callback receives `ConfigWatchEvent` |

### `ConfigWatchEvent`

```rust
pub struct ConfigWatchEvent {
    pub event_type: ConfigWatchEventType,
    pub source: ConfigSource,
    pub path: Option<PathBuf>,
    pub timestamp: DateTime<Utc>,
    pub metadata: Option<Value>,
}
```

Builder: `new(event_type, source)`, `with_path(PathBuf)`, `with_metadata(Value)`.

### `ConfigWatchEventType`

```rust
#[non_exhaustive]
pub enum ConfigWatchEventType {
    Created, Modified, Deleted,
    Renamed { from: PathBuf, to: PathBuf },
    Error(String), Other(String),
}
```

Methods: `is_error()`, `is_change()` (true for Created/Modified/Deleted/Renamed).

---

## `builders` Module

Convenience factory functions:

```rust
builders::from_file(path: impl Into<PathBuf>) -> ConfigBuilder
builders::from_env() -> ConfigBuilder                          // feature "env"
builders::from_env_prefix(prefix: impl Into<String>) -> ConfigBuilder  // feature "env"
builders::standard_app_config(config_file) -> ConfigBuilder   // file + env; feature "env"
builders::with_hot_reload(config_file) -> ConfigBuilder        // file + FileWatcher + hot_reload
builders::with_schema_validation(config_file, schema: Value) -> ConfigBuilder
```

---

## `utils` Module

```rust
utils::check_config_file(path: &Path) -> ConfigResult<()>   // async; existence + readable check
utils::merge_json_values(values: Vec<Value>) -> ConfigResult<Value>
utils::parse_config_string(content: &str, format: ConfigFormat) -> ConfigResult<Value>
```

---

## Contract Baseline

### Precedence Order (lowest to highest priority)

```
Default (100) < File/FileAuto (50) < Directory (40) < Env (30) < CommandLine (20) < Remote (10) < Database/KeyValue (5) < Inline (1)
```

Lower priority number overrides higher priority number. Sources with the same priority are merged in insertion order.

### Merge Semantics

- Object keys: merged **recursively** (deep merge).
- Scalar and array leaf values: **replaced** by the higher-priority source.

### Reload Baseline

- Default values remain part of reload baseline (captured at `build()`, reused on each `reload()`).
- Candidate config activates only after successful validation.
- On validator failure during reload, the previously active snapshot is preserved.

### Typed Access Contract

- Missing path → `ConfigError::PathError` (`contract_category: missing_path`)
- Deserialization mismatch → `ConfigError::TypeError` (`contract_category: type_mismatch`)
- Validation rejection → `ConfigError::ValidationError` (`contract_category: validation_failed`)

### Validator Integration

- Any `nebula_validator::foundation::Validate<Value>` impl is a valid `ConfigValidator` via blanket impl.
- Validator failure blocks config activation at both `build()` and `reload()`.

### Error Categories (contract)

| `contract_category` | Meaning |
|---------------------|---------|
| `source_load_failed` | IO, parse, format, env, reload, encryption errors |
| `merge_failed` | Object merge conflict |
| `validation_failed` | Validator rejected candidate |
| `missing_path` | Requested key not found |
| `type_mismatch` | Typed conversion failed |
| `invalid_value` | Value present but semantically invalid |
| `watcher_failed` | Watch lifecycle error |

---

## Minimal Example

```rust
use nebula_config::prelude::*;

let cfg = ConfigBuilder::new()
    .with_source(ConfigSource::File("config.toml".into()))
    .with_source(ConfigSource::Env)
    .build()
    .await?;

let port: u16 = cfg.get("server.port").await?;
```

## Advanced Example

```rust
use nebula_config::prelude::*;
use std::sync::Arc;

let cfg = ConfigBuilder::new()
    .with_defaults_json(serde_json::json!({ "server": { "port": 3000 }}))
    .with_source(ConfigSource::File("config.toml".into()))
    .with_validator(Arc::new(SchemaValidator::new(serde_json::json!({ "type": "object" }))))
    .with_hot_reload(true)
    .build()
    .await?;

cfg.reload().await?;
let port: u16 = cfg.get("server.port").await?;
```

## Validator Crate Integration

```rust
use nebula_config::ConfigBuilder;
use nebula_validator::foundation::{Validate, ValidationError};
use std::sync::Arc;

#[derive(Clone)]
struct RequireEnabled;

impl Validate<serde_json::Value> for RequireEnabled {
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        let enabled = input
            .get("feature")
            .and_then(|f| f.get("enabled"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if enabled { Ok(()) }
        else { Err(ValidationError::new("validation_failed", "feature.enabled must be true")) }
    }
}

// Blanket impl: RequireEnabled is automatically ConfigValidator
let _cfg = ConfigBuilder::new()
    .with_defaults_json(serde_json::json!({"feature":{"enabled": true}}))
    .with_validator(Arc::new(RequireEnabled))
    .build()
    .await?;
```

---

## Error Semantics

- **Recoverable errors:** `FileNotFound`, `EnvVarNotFound`, `ValidationError` — caller can supply fallback or retry.
- **Fatal errors:** parse failures, format mismatch, required key type mismatch, merge conflict.
- **Retryable (source IO):** `SourceError` on transient connectivity — retry at caller discretion.
- **Validation errors** block config activation; previously active snapshot is preserved.

## Compatibility Rules

- **Major bump:** source precedence semantics change; path traversal behavior changes; validation contract changes; `ContractErrorCategory` variant renames.
- **Deprecation policy:** keep aliases/deprecated accessors for at least one minor cycle; `#[deprecated]` with replacement.

## Contract Fixtures and Schema

- Precedence fixture: `crates/config/tests/fixtures/compat/precedence_v1.json`
- Typed path fixture: `crates/config/tests/fixtures/compat/path_contract_v1.json`
- Error envelope schema: `specs/001-config-crate-spec/contracts/config-error-envelope.schema.json`
- Validator compatibility fixture: `crates/config/tests/fixtures/compat/validator_contract_v1.json`
