//! Configuration watcher implementations

mod file;
mod polling;
mod noop;
mod types;

pub use file::FileWatcher;
pub use polling::PollingWatcher;
pub use noop::NoOpWatcher;
pub use types::{ConfigWatchEvent, ConfigWatchEventType};

// Re-export trait from core for convenience
pub use crate::core::ConfigWatcher;