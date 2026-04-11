#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![allow(unsafe_code)] // CPU affinity on Linux requires unsafe
//! # Nebula System
//!
//! Cross-platform system information and utilities for Nebula ecosystem.
//!
//! Provides a unified interface for:
//! - System information (CPU, memory, OS, hardware)
//! - Memory and CPU pressure detection
//! - Process information and monitoring
//! - Network interface statistics
//! - Disk usage and pressure
//!
//! ## Features
//!
//! - `sysinfo` (default): System information gathering via sysinfo crate
//! - `process`: Process information and monitoring
//! - `network`: Network interface statistics
//! - `disk`: Disk and filesystem information
//! - `serde`: Serialization support for all data types
//!
//! ## Platform Support Matrix
//!
//! | Module    | Linux | macOS | Windows | Notes                                          |
//! |-----------|-------|-------|---------|-------------------------------------------------|
//! | `memory`  | ✓     | ✓     | ✓       | Via `sysinfo`                                   |
//! | `cpu`     | ✓     | ✓     | ✓       | SSE/AVX feature detection x86 only              |
//! | `disk`    | ✓     | ✓     | ✓       | I/O counters Linux-only (`io_stats()`)          |
//! | `network` | ✓     | ✓     | ✓       | `ip_addresses` always empty                     |
//! | `process` | ✓     | ✓     | ✓       | `thread_count` hardcoded, `uid`/`gid` always None |
//!
//! ## Example
//!
//! ```no_run
//! use nebula_system::SystemInfo;
//!
//! fn main() -> nebula_system::SystemResult<()> {
//!     nebula_system::init()?;
//!
//!     let info = SystemInfo::get();
//!     println!("CPU: {} cores", info.cpu.cores);
//!     println!("Memory: {} GB", info.memory.total / (1024 * 1024 * 1024));
//!
//!     let pressure = nebula_system::memory::pressure();
//!     if pressure.is_concerning() {
//!         println!("Warning: Memory pressure is high!");
//!     }
//!
//!     Ok(())
//! }
//! ```
pub mod core;
pub mod info;
pub mod prelude;
pub mod utils;

#[cfg(feature = "sysinfo")]
#[cfg_attr(docsrs, doc(cfg(feature = "sysinfo")))]
pub mod memory;

#[cfg(feature = "sysinfo")]
#[cfg_attr(docsrs, doc(cfg(feature = "sysinfo")))]
pub mod cpu;

#[cfg(feature = "sysinfo")]
#[cfg_attr(docsrs, doc(cfg(feature = "sysinfo")))]
pub mod load;

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
pub use core::{SystemError, SystemResult, SystemResultExt};

pub use info::SystemInfo;
#[cfg(feature = "sysinfo")]
pub use memory::{MemoryInfo, MemoryPressure};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize the system information subsystem
///
/// This should be called once at program startup to initialize
/// caches and prepare the system information gathering.
pub fn init() -> SystemResult<()> {
    info::init()
}

/// Get a formatted summary of system information
#[must_use]
pub fn summary() -> String {
    info::summary()
}
