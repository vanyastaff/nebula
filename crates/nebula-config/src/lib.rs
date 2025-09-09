//! Nebula Config - configuration management for Nebula
#![deny(unused_must_use)]

// Public modules
pub mod error;
pub mod source;
pub mod loader;
pub mod watcher;
pub mod validator;
pub mod builder;

// Common result type for this crate
pub type ConfigResult<T> = core::result::Result<T, error::ConfigError>;

// Re-exports of main types for ergonomic usage
pub use builder::{Config, ConfigBuilder};
pub use error::ConfigError;
pub use loader::{ConfigLoader, CompositeLoader, EnvLoader, FileLoader};
pub use source::{ConfigFormat, ConfigSource, SourceMetadata};
pub use validator::{CompositeValidator, ConfigValidator, NoOpValidator, SchemaValidator};
pub use watcher::{ConfigWatchEvent, ConfigWatchEventType, ConfigWatcher, FileWatcher, PollingWatcher};
