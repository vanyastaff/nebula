//! Core configuration functionality

pub mod config;
pub mod error;
pub mod result;
pub mod source;
pub mod traits;
// Public modules
pub mod builder;

// Re-export core types
pub use builder::ConfigBuilder;
pub use config::Config;
pub use error::ConfigError;
pub use result::{ConfigResult, ConfigResultAggregator, ConfigResultExt, try_sources};
pub use source::{ConfigFormat, ConfigSource, SourceMetadata};

// Re-export core traits
pub use traits::{
    AsyncConfigurable, ConfigLoader, ConfigValidator, ConfigWatcher, Configurable, Validatable,
};
