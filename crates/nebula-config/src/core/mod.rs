//! Core configuration functionality

pub mod config;
pub mod result;
pub mod traits;
pub mod error;
pub mod source;
// Public modules
pub mod builder;

// Re-export core types
pub use config::Config;
pub use builder::ConfigBuilder;
pub use source::{ConfigFormat, ConfigSource, SourceMetadata};
pub use error::ConfigError;
pub use result::{try_sources, ConfigResult, ConfigResultAggregator, ConfigResultExt};

// Re-export core traits
pub use traits::{
    AsyncConfigurable,
    ConfigLoader,
    ConfigValidator,
    ConfigWatcher,
    Configurable,
    Validatable,
};