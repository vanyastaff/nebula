//! Cross-platform system information and utilities for Nebula ecosystem
//!
//! This crate provides a unified interface for:
//! - System information (CPU, memory, OS)
//! - Memory management utilities
//! - Process information
//! - Hardware detection
//! - Performance monitoring
//!
//! # Features
//!
//! - `sysinfo` (default): System information gathering
//! - `memory` (default): Memory management utilities
//! - `process`: Process information and management
//! - `network`: Network interface information
//! - `disk`: Disk and filesystem information
//! - `component`: Hardware component monitoring (temperatures, etc.)
//! - `metrics`: Performance metrics collection
//! - `serde`: Serialization support
//!
//! # Example
//!
//! ```no_run
//! use nebula_system::{SystemInfo, MemoryPressure};
//!
//! fn main() -> nebula_system::Result<()> {
//!     // Initialize the system
//!     nebula_system::init()?;
//!
//!     // Get system information
//!     let info = SystemInfo::get();
//!     println!("CPU: {} cores", info.cpu.cores);
//!     println!("Memory: {} GB", info.memory.total / (1024 * 1024 * 1024));
//!
//!     // Check memory pressure
//!     let pressure = nebula_system::memory::pressure();
//!     if pressure.is_concerning() {
//!         println!("Warning: Memory pressure is high!");
//!     }
//!
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod error;
pub mod info;

#[cfg(feature = "memory")]
#[cfg_attr(docsrs, doc(cfg(feature = "memory")))]
pub mod memory;

#[cfg(feature = "sysinfo")]
#[cfg_attr(docsrs, doc(cfg(feature = "sysinfo")))]
pub mod cpu;

#[cfg(feature = "process")]
#[cfg_attr(docsrs, doc(cfg(feature = "process")))]
pub mod process;

#[cfg(feature = "network")]
#[cfg_attr(docsrs, doc(cfg(feature = "network")))]
pub mod network;

#[cfg(feature = "disk")]
#[cfg_attr(docsrs, doc(cfg(feature = "disk")))]
pub mod disk;

// Re-exports
pub use error::{Result, SystemError};
pub use info::SystemInfo;

#[cfg(feature = "memory")]
pub use memory::{MemoryInfo, MemoryPressure};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize the system information subsystem
///
/// This should be called once at program startup to initialize
/// caches and prepare the system information gathering.
pub fn init() -> Result<()> {
    info::init()
}

/// Get a formatted summary of system information
pub fn summary() -> String {
    info::summary()
}
