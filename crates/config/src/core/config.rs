//! Main configuration container

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use dashmap::DashMap;
use serde::de::DeserializeOwned;
use smallvec::SmallVec;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use super::{
    ConfigError, ConfigLoader, ConfigResult, ConfigSource, ConfigValidator, ConfigWatcher,
    SourceMetadata,
};

/// Internal signal passed from a watcher callback into the hot-reload task.
///
/// Carries only enough context for diagnostics — the reload task always calls
/// [`Config::reload`], which re-loads ALL sources rather than relying on the
/// trigger payload. This keeps the reload semantics identical to a manual
/// `reload()` invocation regardless of which source fired the event.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub(crate) struct ReloadTrigger {
    /// Source whose backing storage changed (file, directory, …).
    pub source: ConfigSource,
    /// Concrete path that triggered the event, when available.
    pub path: Option<PathBuf>,
}

/// Coalescing window for the hot-reload pipeline. After the first trigger
/// arrives, the reloader waits up to this duration for additional triggers
/// before issuing a single `reload()` call. This collapses bursts (rapid
/// editor saves, deploy storms) into one reload.
const RELOAD_COALESCE: Duration = Duration::from_millis(250);

/// Inline storage for config sources — most configs have 1-3 sources.
pub(crate) type Sources = SmallVec<[ConfigSource; 4]>;

/// Runtime behavior options for `Config`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConfigRuntimeOptions {
    pub hot_reload: bool,
    pub fail_on_missing: bool,
}

/// Main configuration container
#[derive(Clone)]
pub struct Config {
    /// Configuration data
    data: Arc<RwLock<serde_json::Value>>,

    /// Configuration sources
    sources: Sources,

    /// Default values captured at build time and reused during reload.
    defaults: Option<serde_json::Value>,

    /// Source metadata
    metadata: Arc<DashMap<ConfigSource, SourceMetadata>>,

    /// Configuration loader
    loader: Arc<dyn ConfigLoader>,

    /// Configuration validator
    validator: Option<Arc<dyn ConfigValidator>>,

    /// Configuration watcher
    watcher: Option<Arc<dyn ConfigWatcher>>,

    /// Hot reload enabled
    hot_reload: bool,

    /// Whether reload should fail on optional source errors.
    fail_on_missing: bool,

    /// Cancellation token for background tasks (auto-reload, etc.)
    cancel_token: CancellationToken,

    /// Receiver end of the hot-reload trigger channel.
    ///
    /// The builder installs a `FileWatcher` whose callback writes to the
    /// matching `Sender`. [`Config::start_hot_reload_pipeline`] drains the
    /// receiver inside a debouncing reloader task and calls [`Config::reload`]
    /// on each coalesced batch. Stored in an `Arc<Mutex<Option<…>>>` so
    /// `Config: Clone` semantics are preserved (cheap clones share the same
    /// slot, and the receiver — a non-`Clone` resource — is `take()`n exactly
    /// once when the pipeline starts). `None` means hot reload is either
    /// disabled or already started.
    reload_rx: Arc<Mutex<Option<mpsc::Receiver<ReloadTrigger>>>>,

    /// Count of completed hot-reload cycles since this `Config` was built.
    ///
    /// Exposed via [`Config::hot_reload_count`] for tests and dashboards. A
    /// "cycle" is one debounced batch ending in a `reload()` call (regardless
    /// of whether the reload succeeded — failures still increment, with the
    /// error logged).
    reload_count: Arc<std::sync::atomic::AtomicU64>,
}

impl Config {
    /// Create new config (internal use only, use ConfigBuilder)
    pub(crate) fn new(
        data: serde_json::Value,
        sources: Sources,
        defaults: Option<serde_json::Value>,
        loader: Arc<dyn ConfigLoader>,
        validator: Option<Arc<dyn ConfigValidator>>,
        watcher: Option<Arc<dyn ConfigWatcher>>,
        options: ConfigRuntimeOptions,
    ) -> Self {
        Self {
            data: Arc::new(RwLock::new(data)),
            sources,
            defaults,
            metadata: Arc::new(DashMap::new()),
            loader,
            validator,
            watcher,
            hot_reload: options.hot_reload,
            fail_on_missing: options.fail_on_missing,
            cancel_token: CancellationToken::new(),
            reload_rx: Arc::new(Mutex::new(None)),
            reload_count: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Get the cancellation token for background tasks
    pub(crate) fn cancel_token(&self) -> &CancellationToken {
        &self.cancel_token
    }

    /// Stash the hot-reload receiver supplied by the builder.
    ///
    /// Called from [`ConfigBuilder::build`] when hot reload is enabled, after
    /// the builder has constructed the trigger channel and installed a
    /// `FileWatcher` whose callback writes to the matching sender. The
    /// receiver is consumed by [`Config::start_hot_reload_pipeline`].
    pub(crate) async fn install_reload_rx(&self, rx: mpsc::Receiver<ReloadTrigger>) {
        let mut slot = self.reload_rx.lock().await;
        *slot = Some(rx);
    }

    /// Number of completed hot-reload cycles since this `Config` was built.
    ///
    /// Each cycle is one debounced batch ending in a `reload()` call. Useful
    /// for asserting in tests that a coalesced burst produced fewer reloads
    /// than triggers, and for dashboard alerting on reload churn.
    pub fn hot_reload_count(&self) -> u64 {
        self.reload_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get entire configuration as typed value
    pub async fn get_all<T>(&self) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        let data = self.data.read().await;
        T::deserialize(&*data).map_err(|e| {
            ConfigError::type_error(e.to_string(), std::any::type_name::<T>(), "JSON value")
        })
    }

    /// Get configuration value by path
    pub async fn get<T>(&self, path: &str) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        let data = self.data.read().await;
        let value = self.get_nested_value(&data, path)?;
        T::deserialize(value).map_err(|e| {
            ConfigError::type_error(e.to_string(), std::any::type_name::<T>(), "JSON value")
        })
    }

    /// Get configuration value by path (alias for get)
    #[deprecated(since = "0.2.0", note = "use `get` instead")]
    pub async fn get_path<T>(&self, path: &str) -> ConfigResult<T>
    where
        T: DeserializeOwned,
    {
        self.get(path).await
    }

    /// Get configuration value by path with default
    pub async fn get_or<T>(&self, path: &str, default: T) -> T
    where
        T: DeserializeOwned,
    {
        self.get(path).await.unwrap_or(default)
    }

    /// Get configuration value by path or default
    pub async fn get_or_else<T, F>(&self, path: &str, default_fn: F) -> T
    where
        T: DeserializeOwned,
        F: FnOnce() -> T,
    {
        self.get(path).await.unwrap_or_else(|_| default_fn())
    }

    /// Check if configuration has a path
    pub async fn has(&self, path: &str) -> bool {
        let data = self.data.read().await;
        self.get_nested_value(&data, path).is_ok()
    }

    /// Try to get configuration value by path, returning None on error
    pub async fn get_opt<T>(&self, path: &str) -> Option<T>
    where
        T: DeserializeOwned,
    {
        self.get(path).await.ok()
    }

    /// Get all configuration keys at a path
    pub async fn keys(&self, path: Option<&str>) -> ConfigResult<Vec<String>> {
        let data = self.data.read().await;
        let value = if let Some(path) = path {
            self.get_nested_value(&data, path)?
        } else {
            &*data
        };

        match value {
            serde_json::Value::Object(obj) => Ok(obj.keys().cloned().collect()),
            _ => Err(ConfigError::type_error(
                "Path does not point to an object",
                "Object",
                value.to_string(),
            )),
        }
    }

    /// Get raw JSON value at path
    pub async fn get_raw(&self, path: Option<&str>) -> ConfigResult<serde_json::Value> {
        let data = self.data.read().await;

        if let Some(path) = path {
            Ok(self.get_nested_value(&data, path)?.clone())
        } else {
            Ok(data.clone())
        }
    }

    /// Reload configuration from all sources
    ///
    /// Sources are loaded **concurrently** for maximum throughput,
    /// then merged in priority order (pre-sorted at construction time).
    pub async fn reload(&self) -> ConfigResult<()> {
        nebula_log::info!(
            "Reloading configuration from {} sources",
            self.sources.len()
        );

        // Start from defaults captured during the initial build.
        let mut merged_data = self
            .defaults
            .clone()
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));

        // Skip pseudo default marker source, it has no loader representation.
        let loadable: Vec<_> = self
            .sources
            .iter()
            .filter(|source| !matches!(source, ConfigSource::Default))
            .collect();

        // Load all sources concurrently
        let loader = &self.loader;
        let load_futures = loadable.iter().map(|source| async move {
            let data = loader.load(source).await;
            let metadata = loader.metadata(source).await.ok();
            (source, data, metadata)
        });
        let results = futures::future::join_all(load_futures).await;

        // Merge in priority order (sources are pre-sorted at construction time)
        for (source, result, metadata) in results {
            match result {
                Ok(data) => {
                    nebula_log::debug!("Loaded configuration from source: {}", source);

                    if let Some(metadata) = metadata {
                        self.metadata.insert((*source).clone(), metadata);
                    }

                    merge_json(&mut merged_data, data)?;
                }
                Err(e) => {
                    nebula_log::warn!("Failed to load from source {}: {}", source, e);

                    if self.fail_on_missing || !source.is_optional() {
                        return Err(e);
                    }
                }
            }
        }

        // Validate if validator is present
        if let Some(validator) = &self.validator {
            nebula_log::debug!("Validating configuration");
            validator.validate(&merged_data).await?;
        }

        // Update configuration data
        {
            let mut data = self.data.write().await;
            *data = merged_data;
        }

        nebula_log::info!("Configuration reloaded successfully");
        Ok(())
    }

    /// Get source metadata
    pub fn get_metadata(&self, source: &ConfigSource) -> Option<SourceMetadata> {
        self.metadata.get(source).map(|entry| entry.value().clone())
    }

    /// Get all source metadata
    pub fn get_all_metadata(&self) -> HashMap<ConfigSource, SourceMetadata> {
        self.metadata
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Get configuration sources
    pub fn sources(&self) -> &[ConfigSource] {
        &self.sources
    }

    /// Start watching for configuration changes (if hot reload is enabled).
    ///
    /// **This method only starts the underlying watcher; it does NOT wire
    /// watcher events to [`Config::reload`].** Use
    /// `ConfigBuilder::with_hot_reload` to opt into the full hot-reload
    /// pipeline, which the builder sets up by calling
    /// `Config::start_hot_reload_pipeline` (#313).
    ///
    /// Kept for backwards-compatibility with callers that wire their own
    /// reload logic on top of a raw `ConfigWatcher`.
    #[deprecated(
        since = "0.2.1",
        note = "use ConfigBuilder::with_hot_reload — start_watching alone does not apply file changes to Config data; see #313"
    )]
    pub async fn start_watching(&self) -> ConfigResult<()> {
        if !self.hot_reload {
            nebula_log::debug!("Hot reload is disabled, skipping watch setup");
            return Ok(());
        }

        if let Some(watcher) = &self.watcher {
            nebula_log::info!("Starting configuration watcher");
            // Hand the watcher a child of our cancel token. Cancelling the
            // child via `stop_watching` still works, and parent cancellation
            // (fired by `Config::drop`) cascades to this child, tearing down
            // any spawned watcher tasks without requiring an async drop.
            watcher
                .start_watching(&self.sources, self.cancel_token.child_token())
                .await?;
        } else {
            nebula_log::debug!("No watcher configured");
        }

        Ok(())
    }

    /// Wire watcher events through to [`Config::reload`] via a debounced
    /// internal channel (#313).
    ///
    /// Called from [`ConfigBuilder::build`] when `hot_reload(true)` is set.
    /// The builder is responsible for installing a `FileWatcher` whose
    /// callback forwards [`ReloadTrigger`] values into the channel previously
    /// stashed via [`Config::install_reload_rx`].
    ///
    /// # Pipeline shape
    ///
    /// 1. The watcher callback writes to `reload_tx` via non-blocking `try_send` — see #310. Drops
    ///    on saturation are acceptable: the next file change retriggers, and the reloader always
    ///    reloads from scratch.
    /// 2. This method consumes the matching `reload_rx`, starts the watcher, then spawns a
    ///    debouncing reload task.
    /// 3. The reload task waits for the first trigger, then collects any additional triggers that
    ///    arrive within `RELOAD_COALESCE` (250 ms) before issuing one [`Config::reload`] call. This
    ///    collapses bursts of editor-saves into a single reload.
    /// 4. The reload task exits when `cancel_token` fires — which happens automatically on
    ///    `Config::drop`, so the task is reclaimed without requiring an async destructor.
    ///
    /// # Ownership
    ///
    /// The reload task holds a `Config` clone — `Config: Clone` is cheap
    /// (`Arc`-shared inner state) and lets the task call `reload()` directly
    /// without `Arc<Config>` gymnastics.
    pub(crate) async fn start_hot_reload_pipeline(&self) -> ConfigResult<()> {
        // Take the receiver installed by the builder. Doing this BEFORE we
        // call into the watcher means a misconfigured pipeline (no
        // `install_reload_rx`) fails fast before any background tasks spawn.
        let reload_rx = {
            let mut slot = self.reload_rx.lock().await;
            slot.take().ok_or_else(|| {
                ConfigError::internal(
                    "hot reload pipeline started without a reload channel; \
                     the builder must call `install_reload_rx` first",
                )
            })?
        };

        // Now bring the watcher up. If this fails, the receiver has already
        // been moved out of the slot — another `start_hot_reload_pipeline`
        // call cannot accidentally re-use it.
        if let Some(watcher) = &self.watcher {
            nebula_log::info!("Starting configuration watcher (hot reload)");
            watcher
                .start_watching(&self.sources, self.cancel_token.child_token())
                .await?;
        } else {
            // Without a watcher there is nothing to drain; this is a misuse
            // of the API and we surface it loudly.
            return Err(ConfigError::internal(
                "hot reload pipeline started without a watcher",
            ));
        }

        // Spawn the debouncing reloader. Holds a Config clone (cheap).
        let cfg = self.clone();
        let cancel = self.cancel_token.clone();
        tokio::spawn(async move {
            run_hot_reload_loop(cfg, reload_rx, cancel).await;
        });

        Ok(())
    }

    /// Stop watching for configuration changes
    pub async fn stop_watching(&self) -> ConfigResult<()> {
        // Cancel background tasks (auto-reload, etc.)
        self.cancel_token.cancel();

        if let Some(watcher) = &self.watcher {
            nebula_log::info!("Stopping configuration watcher");
            watcher.stop_watching().await?;
        }

        Ok(())
    }

    /// Check if watching for changes
    pub fn is_watching(&self) -> bool {
        self.watcher.as_ref().is_some_and(|w| w.is_watching())
    }

    /// Get nested value from JSON using dot notation (zero-alloc path traversal)
    fn get_nested_value<'a>(
        &self,
        value: &'a serde_json::Value,
        path: &str,
    ) -> ConfigResult<&'a serde_json::Value> {
        if path.is_empty() {
            return Ok(value);
        }

        let mut current = value;
        let mut remaining = path;

        loop {
            // Byte scan for `.` — avoids SplitInternal struct and per-segment call
            // overhead. All path separators are ASCII; multi-byte UTF-8 sequences
            // always have the high bit set and cannot be confused with b'.'.
            let (part, rest, has_dot) = match remaining.as_bytes().iter().position(|&b| b == b'.') {
                Some(pos) => (&remaining[..pos], &remaining[pos + 1..], true),
                None => (remaining, "", false),
            };

            if part.is_empty() {
                return Err(ConfigError::path_error(
                    "Path segment must not be empty (check for leading, trailing, or consecutive dots)",
                    path.to_string(),
                ));
            }

            match current {
                serde_json::Value::Object(obj) => {
                    current = obj.get(part).ok_or_else(|| {
                        ConfigError::path_error(format!("Key '{part}' not found"), path.to_string())
                    })?;
                }
                serde_json::Value::Array(arr) => {
                    let index: usize = part.parse().map_err(|_| {
                        ConfigError::path_error(
                            format!("Invalid array index '{part}'"),
                            path.to_string(),
                        )
                    })?;
                    current = arr.get(index).ok_or_else(|| {
                        ConfigError::path_error(
                            format!("Array index {index} out of bounds (size: {})", arr.len()),
                            path.to_string(),
                        )
                    })?;
                }
                _ => {
                    return Err(ConfigError::path_error(
                        format!(
                            "Cannot index into {} with '{part}'",
                            json_type_name(current),
                        ),
                        path.to_string(),
                    ));
                }
            }

            if rest.is_empty() {
                // A trailing dot means `has_dot` is true but nothing follows — reject it.
                if has_dot {
                    return Err(ConfigError::path_error(
                        "Path segment must not be empty (check for leading, trailing, or consecutive dots)",
                        path.to_string(),
                    ));
                }
                break;
            }
            remaining = rest;
        }

        Ok(current)
    }

    /// Set nested value in JSON using dot notation
    #[allow(clippy::excessive_nesting)] // Reason: deeply nested JSON path traversal
    fn set_nested_value(
        &self,
        value: &mut serde_json::Value,
        path: &str,
        new_value: serde_json::Value,
    ) -> ConfigResult<()> {
        if path.is_empty() {
            *value = new_value;
            return Ok(());
        }

        // Split into parent path and final key to avoid Vec allocation
        let (parent_path, final_key) = match path.rsplit_once('.') {
            Some((parent, key)) => (Some(parent), key),
            None => (None, path),
        };

        // Navigate to the parent
        let current = if let Some(parent_path) = parent_path {
            let mut current = &mut *value;
            for part in parent_path.split('.') {
                match current {
                    serde_json::Value::Object(obj) => {
                        current = obj
                            .entry(part.to_string())
                            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                    }
                    serde_json::Value::Array(arr) => {
                        let index: usize = part.parse().map_err(|_| {
                            ConfigError::path_error(
                                format!("Invalid array index '{part}'"),
                                path.to_string(),
                            )
                        })?;
                        while arr.len() <= index {
                            arr.push(serde_json::Value::Null);
                        }
                        current = &mut arr[index];
                    }
                    _ => {
                        return Err(ConfigError::path_error(
                            format!("Cannot navigate into {} type", json_type_name(current)),
                            path.to_string(),
                        ));
                    }
                }
            }
            current
        } else {
            value
        };

        // Set the final value
        match current {
            serde_json::Value::Object(obj) => {
                obj.insert(final_key.to_string(), new_value);
            }
            serde_json::Value::Array(arr) => {
                let index: usize = final_key.parse().map_err(|_| {
                    ConfigError::path_error(
                        format!("Invalid array index '{final_key}'"),
                        path.to_string(),
                    )
                })?;
                while arr.len() <= index {
                    arr.push(serde_json::Value::Null);
                }
                arr[index] = new_value;
            }
            _ => {
                return Err(ConfigError::path_error(
                    format!("Cannot set value in {} type", json_type_name(current)),
                    path.to_string(),
                ));
            }
        }

        Ok(())
    }

    // ==================== Dynamic Value Integration ====================

    /// Get entire configuration as dynamic value
    pub async fn as_value(&self) -> serde_json::Value {
        let data = self.data.read().await;
        data.clone()
    }

    /// Get configuration value by path as dynamic value
    pub async fn get_value(&self, path: &str) -> ConfigResult<serde_json::Value> {
        let data = self.data.read().await;
        let json_value = self.get_nested_value(&data, path)?;
        Ok(json_value.clone())
    }

    /// Set configuration value by path
    pub async fn set_value(&self, path: &str, value: serde_json::Value) -> ConfigResult<()> {
        let mut data = self.data.write().await;
        self.set_nested_value(&mut data, path, value)?;
        Ok(())
    }

    /// Set typed configuration with automatic serialization
    pub async fn set_typed<T>(&self, path: &str, value: T) -> ConfigResult<()>
    where
        T: serde::Serialize,
    {
        let json_value = serde_json::to_value(value).map_err(|e| {
            ConfigError::type_error(
                format!("Failed to serialize: {e}"),
                "JSON value",
                std::any::type_name::<T>(),
            )
        })?;
        self.set_value(path, json_value).await
    }

    /// Get all configuration as flat key-value pairs with dot-notation keys.
    ///
    /// Array elements use dot-separated indices (e.g. `tags.0`, `tags.1`)
    /// consistent with the `get`/`set_value` path API.
    pub async fn flatten(&self) -> Vec<(String, serde_json::Value)> {
        let snapshot = {
            let data = self.data.read().await;
            data.clone()
        };
        let capacity = count_leaves(&snapshot);
        let mut pairs = Vec::with_capacity(capacity);
        let mut buf = String::with_capacity(64);
        flatten_into(&mut buf, &snapshot, &mut pairs);
        pairs
    }

    /// Merge configuration from dynamic value
    pub async fn merge(&self, value: serde_json::Value) -> ConfigResult<()> {
        let mut data = self.data.write().await;
        merge_json(&mut data, value)
    }
}

/// Debouncing reload loop spawned by [`Config::start_hot_reload_pipeline`].
///
/// The loop has three exit paths:
///
/// - `cancel.cancelled()` — fired by `Config::drop` or an explicit `stop_watching()`. This is the
///   primary shutdown signal; `biased` ensures it always wins over a pending `recv()` when both are
///   ready.
/// - `reload_rx.recv()` returns `None` — the watcher tore down its sender, typically because the
///   `FileWatcher`'s notify thread exited.
/// - A panic inside `cfg.reload()` — caught nowhere; the spawned task exits and the `JoinError` is
///   observable via the runtime, matching the existing auto-reload task's contract.
///
/// Inside the loop, the first trigger opens a coalescing window of length
/// [`RELOAD_COALESCE`]. Any additional triggers that arrive in the window are
/// drained without issuing reloads; once the window closes (or the channel
/// closes, or cancel fires) a single `cfg.reload()` is performed and
/// `reload_count` is incremented.
async fn run_hot_reload_loop(
    cfg: Config,
    mut reload_rx: mpsc::Receiver<ReloadTrigger>,
    cancel: CancellationToken,
) {
    nebula_log::debug!("hot reload pipeline task started");

    loop {
        // Wait for the first trigger or shutdown.
        let first = tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            maybe = reload_rx.recv() => match maybe {
                Some(trigger) => trigger,
                None => break,
            },
        };

        nebula_log::debug!(
            source = %first.source,
            path = ?first.path,
            "hot reload trigger received; starting coalesce window"
        );

        // Coalesce: drain any additional triggers that arrive within the
        // window. Cancel still wins. A `recv() == None` aborts the loop.
        let deadline = tokio::time::sleep(RELOAD_COALESCE);
        tokio::pin!(deadline);
        let mut closed = false;
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return,
                () = &mut deadline => break,
                maybe = reload_rx.recv() => {
                    if maybe.is_none() {
                        closed = true;
                        break;
                    }
                    // Additional trigger collapsed into the same reload cycle.
                }
            }
        }

        // Perform the reload. We always call `cfg.reload()` regardless of
        // which source fired — the trigger payload is diagnostic only, and
        // re-reading every source preserves cross-source merge semantics.
        match cfg.reload().await {
            Ok(()) => {
                nebula_log::info!("hot reload applied");
            }
            Err(e) => {
                // Do not exit the loop on a transient reload failure: the
                // next file change will retrigger. Surface the error loudly.
                nebula_log::warn!(error = %e, "hot reload failed");
            }
        }
        cfg.reload_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        if closed {
            break;
        }
    }

    nebula_log::debug!("hot reload pipeline task exiting");
}

/// Get human-readable type name for a JSON value (zero-alloc)
pub(crate) fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Merge two JSON values using entry API to avoid double BTreeMap lookups.
pub(crate) fn merge_json(
    target: &mut serde_json::Value,
    source: serde_json::Value,
) -> ConfigResult<()> {
    match (target, source) {
        (serde_json::Value::Object(target_obj), serde_json::Value::Object(source_obj)) => {
            for (key, value) in source_obj {
                match target_obj.entry(key) {
                    serde_json::map::Entry::Occupied(mut entry) => {
                        merge_json(entry.get_mut(), value)?;
                    }
                    serde_json::map::Entry::Vacant(entry) => {
                        entry.insert(value);
                    }
                }
            }
        }
        (target, source) => {
            *target = source;
        }
    }
    Ok(())
}

/// Count leaf nodes for Vec pre-allocation.
#[inline]
fn count_leaves(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(obj) => obj.values().map(count_leaves).sum(),
        serde_json::Value::Array(arr) => arr.iter().map(count_leaves).sum(),
        _ => 1,
    }
}

/// Flatten a JSON value into `(dotted_key, leaf_value)` pairs.
///
/// Uses a single reusable `String` buffer (push/truncate) instead of
/// allocating via `format!()` at every recursion level, and `itoa` for
/// array indices. Array elements use dot notation (`a.0`, `a.1`) matching
/// the `get`/`set_value` path API.
fn flatten_into(
    buf: &mut String,
    value: &serde_json::Value,
    out: &mut Vec<(String, serde_json::Value)>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                let prev_len = buf.len();
                if !buf.is_empty() {
                    buf.push('.');
                }
                buf.push_str(key);
                flatten_into(buf, val, out);
                buf.truncate(prev_len);
            }
        }
        serde_json::Value::Array(arr) => {
            let mut itoa_buf = itoa::Buffer::new();
            for (index, val) in arr.iter().enumerate() {
                let prev_len = buf.len();
                if !buf.is_empty() {
                    buf.push('.');
                }
                buf.push_str(itoa_buf.format(index));
                flatten_into(buf, val, out);
                buf.truncate(prev_len);
            }
        }
        _ => {
            out.push((buf.clone(), value.clone()));
        }
    }
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("sources", &self.sources.len())
            .field("hot_reload", &self.hot_reload)
            .field("watching", &self.is_watching())
            .field("has_validator", &self.validator.is_some())
            .field("has_watcher", &self.watcher.is_some())
            .field("fail_on_missing", &self.fail_on_missing)
            .finish()
    }
}

// Cleanup on drop
impl Drop for Config {
    fn drop(&mut self) {
        // Cancel all background tasks (auto-reload, etc.)
        self.cancel_token.cancel();

        if let Some(watcher) = &self.watcher
            && watcher.is_watching()
        {
            nebula_log::debug!("Config dropped while still watching");
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn test_config(data: serde_json::Value) -> Config {
        Config::new(
            data,
            smallvec::smallvec![ConfigSource::Default],
            None,
            Arc::new(crate::loaders::CompositeLoader::default()),
            None,
            None,
            ConfigRuntimeOptions {
                hot_reload: false,
                fail_on_missing: false,
            },
        )
    }

    #[tokio::test]
    async fn test_get_and_get_all() {
        let cfg = test_config(json!({
            "name": "app",
            "port": 8080,
            "nested": {"key": "value"}
        }));

        let name: String = cfg.get("name").await.unwrap();
        assert_eq!(name, "app");

        let port: u16 = cfg.get("port").await.unwrap();
        assert_eq!(port, 8080);

        let nested_val: String = cfg.get("nested.key").await.unwrap();
        assert_eq!(nested_val, "value");

        // get_all deserializes entire config
        #[derive(serde::Deserialize)]
        struct AppConfig {
            name: String,
            port: u16,
        }
        let all: AppConfig = cfg.get_all().await.unwrap();
        assert_eq!(all.name, "app");
        assert_eq!(all.port, 8080);
    }

    #[tokio::test]
    async fn test_get_or_and_get_or_else() {
        let cfg = test_config(json!({"existing": "hello"}));

        let val: String = cfg.get_or("existing", "default".to_string()).await;
        assert_eq!(val, "hello");

        let val: String = cfg.get_or("missing", "default".to_string()).await;
        assert_eq!(val, "default");

        let val: String = cfg.get_or_else("missing", || "computed".to_string()).await;
        assert_eq!(val, "computed");
    }

    #[tokio::test]
    async fn test_has_and_get_opt() {
        let cfg = test_config(json!({"key": "value", "nested": {"a": 1}}));

        assert!(cfg.has("key").await);
        assert!(cfg.has("nested.a").await);
        assert!(!cfg.has("missing").await);
        assert!(!cfg.has("nested.b").await);

        let some: Option<String> = cfg.get_opt("key").await;
        assert_eq!(some, Some("value".to_string()));

        let none: Option<String> = cfg.get_opt("missing").await;
        assert_eq!(none, None);
    }

    #[tokio::test]
    async fn test_keys() {
        let cfg = test_config(json!({
            "a": 1,
            "b": 2,
            "nested": {"x": 10, "y": 20}
        }));

        let mut root_keys = cfg.keys(None).await.unwrap();
        root_keys.sort();
        assert_eq!(root_keys, vec!["a", "b", "nested"]);

        let mut nested_keys = cfg.keys(Some("nested")).await.unwrap();
        nested_keys.sort();
        assert_eq!(nested_keys, vec!["x", "y"]);

        // Non-object path errors
        assert!(cfg.keys(Some("a")).await.is_err());
    }

    #[tokio::test]
    async fn test_get_raw_and_get_value() {
        let data = json!({"key": "value", "num": 42});
        let cfg = test_config(data.clone());

        let raw_all = cfg.get_raw(None).await.unwrap();
        assert_eq!(raw_all, data);

        let raw_key = cfg.get_raw(Some("key")).await.unwrap();
        assert_eq!(raw_key, json!("value"));

        let val = cfg.get_value("num").await.unwrap();
        assert_eq!(val, json!(42));
    }

    #[tokio::test]
    async fn test_as_value() {
        let data = json!({"hello": "world"});
        let cfg = test_config(data.clone());
        assert_eq!(cfg.as_value().await, data);
    }

    #[tokio::test]
    async fn test_set_value_and_set_typed() {
        let cfg = test_config(json!({"a": 1}));

        cfg.set_value("b", json!("new")).await.unwrap();
        let val: String = cfg.get("b").await.unwrap();
        assert_eq!(val, "new");

        // Set nested path (creates intermediary objects)
        cfg.set_value("nested.deep", json!(true)).await.unwrap();
        let val: bool = cfg.get("nested.deep").await.unwrap();
        assert!(val);

        // set_typed serializes automatically
        cfg.set_typed("count", 42u32).await.unwrap();
        let val: u32 = cfg.get("count").await.unwrap();
        assert_eq!(val, 42);
    }

    #[tokio::test]
    async fn test_flatten() {
        let cfg = test_config(json!({
            "server": {"host": "localhost", "port": 8080},
            "tags": ["a", "b"]
        }));

        let flat = cfg.flatten().await;
        let find = |key: &str| flat.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        assert_eq!(find("server.host"), Some(json!("localhost")));
        assert_eq!(find("server.port"), Some(json!(8080)));
        assert_eq!(find("tags.0"), Some(json!("a")));
        assert_eq!(find("tags.1"), Some(json!("b")));
    }

    #[tokio::test]
    async fn test_merge() {
        let cfg = test_config(json!({
            "a": 1,
            "nested": {"x": 10, "y": 20}
        }));

        cfg.merge(json!({
            "b": 2,
            "nested": {"y": 99, "z": 30}
        }))
        .await
        .unwrap();

        let val: i64 = cfg.get("a").await.unwrap();
        assert_eq!(val, 1); // preserved
        let val: i64 = cfg.get("b").await.unwrap();
        assert_eq!(val, 2); // added
        let val: i64 = cfg.get("nested.x").await.unwrap();
        assert_eq!(val, 10); // preserved
        let val: i64 = cfg.get("nested.y").await.unwrap();
        assert_eq!(val, 99); // overwritten
        let val: i64 = cfg.get("nested.z").await.unwrap();
        assert_eq!(val, 30); // added
    }

    #[test]
    fn test_json_type_name() {
        assert_eq!(json_type_name(&json!(null)), "null");
        assert_eq!(json_type_name(&json!(true)), "boolean");
        assert_eq!(json_type_name(&json!(42)), "number");
        assert_eq!(json_type_name(&json!("hi")), "string");
        assert_eq!(json_type_name(&json!([1, 2])), "array");
        assert_eq!(json_type_name(&json!({"a": 1})), "object");
    }

    #[test]
    fn test_merge_json() {
        let mut target = json!({"a": 1, "nested": {"x": 10}});
        merge_json(&mut target, json!({"b": 2, "nested": {"y": 20}})).unwrap();
        assert_eq!(
            target,
            json!({"a": 1, "b": 2, "nested": {"x": 10, "y": 20}})
        );

        // Scalar overwrite
        let mut target2 = json!({"key": "old"});
        merge_json(&mut target2, json!({"key": "new"})).unwrap();
        assert_eq!(target2["key"], json!("new"));
    }

    #[test]
    fn test_config_debug() {
        let cfg = test_config(json!({}));
        let debug = format!("{:?}", cfg);
        assert!(debug.contains("Config"));
        assert!(debug.contains("sources"));
        assert!(debug.contains("hot_reload"));
        assert!(debug.contains("watching"));
        assert!(debug.contains("has_validator"));
        assert!(debug.contains("has_watcher"));
    }

    #[tokio::test]
    async fn rejects_leading_dot_in_path() {
        let config = test_config(json!({"a": "val"}));
        let err = config.get::<String>(".a").await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn rejects_trailing_dot_in_path() {
        let config = test_config(json!({"a": "val"}));
        let err = config.get::<String>("a.").await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn rejects_consecutive_dots_in_path() {
        let config = test_config(json!({"a": {"b": "val"}}));
        let err = config.get::<String>("a..b").await;
        assert!(err.is_err());
    }

    /// #315: dropping a `Config` must cancel its `cancel_token`, which in
    /// turn reaches any `ConfigWatcher` spawned task via the child token
    /// passed in `start_watching`. Observed via an external clone of the
    /// same token — a child observes its parent's cancellation.
    #[tokio::test]
    async fn dropping_config_cancels_child_watcher_token() {
        let data = json!({"k": "v"});
        let config = Config::new(
            data,
            smallvec::smallvec![ConfigSource::Default],
            None,
            Arc::new(crate::loaders::CompositeLoader::default()),
            None,
            None,
            ConfigRuntimeOptions {
                hot_reload: true,
                fail_on_missing: false,
            },
        );

        // Grab a child from the same parent token the watcher would get.
        let child = config.cancel_token().child_token();
        assert!(!child.is_cancelled());

        drop(config);

        // Cancellation cascades from parent to child synchronously.
        assert!(
            child.is_cancelled(),
            "Config::drop must cancel its cancel_token so watcher child tokens fire"
        );
    }
}
