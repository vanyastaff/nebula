//! Configuration loader implementations

mod composite;
#[cfg(feature = "env")]
mod env;
pub(crate) mod file;

pub use composite::CompositeLoader;
#[cfg(feature = "env")]
pub use env::EnvLoader;
pub use file::FileLoader;

// Re-export trait from core for convenience
pub use crate::core::ConfigLoader;
