//! Configuration watcher implementations

mod file;
mod noop;
mod polling;
mod types;

pub use file::FileWatcher;
pub use noop::NoOpWatcher;
pub use polling::PollingWatcher;
pub use types::{ConfigWatchEvent, ConfigWatchEventType};

// Re-export trait from core for convenience
pub use crate::core::ConfigWatcher;
