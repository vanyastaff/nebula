//! High-performance memory management for Nebula workflow automation
//!
//! This crate provides efficient memory management primitives optimized
//! for workflow automation scenarios, including:
//!
//! - Cross-platform system information
//! - Memory pools and arenas
//! - Cache-aware data structures
//! - Memory pressure monitoring
//! - Memory budgeting and limits
//!
//! # Features
//!
//! - `std` (default): Enables standard library support
//! - `sysinfo` (default): Enables detailed system information gathering
//! - `pool`: Object pooling support
//! - `arena`: Arena allocation support
//! - `cache`: Caching support
//! - `stats`: Statistics collection
//! - `budget`: Memory budgeting
//!
//! # Example
//!
//! ```no_run
//! use nebula_memory::system;
//!
//! fn main() -> nebula_memory::error::Result<()> {
//!     // Initialize the memory subsystem
//!     system::init()?;
//!
//!     // Get system information
//!     let info = system::SystemInfo::get();
//!     println!("Total memory: {}", system::SystemInfo::format_bytes(info.total_memory));
//!     println!("CPU cores: {}", info.cpu_count);
//!
//!     // Check memory pressure
//!     let pressure = system::get_memory_pressure();
//!     println!("Memory pressure: {:?}", pressure);
//!
//!     Ok(())
//! }
//! ```

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

// Core modules
pub mod error;
pub mod utils;

// Re-export nebula-system as `system` for system information utilities
#[cfg(feature = "std")]
pub use nebula_system as system;

// Re-export common types for convenience
pub use error::{MemoryError, Result};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize the memory subsystem
///
/// This should be called once at program startup.
/// It initializes system information caches and prepares
/// the memory management subsystem.
///
/// # Example
///
/// ```no_run
/// fn main() -> nebula_memory::Result<()> {
///     nebula_memory::init()?;
///     // Your application code here
///     Ok(())
/// }
/// ```
#[cfg(feature = "std")]
pub fn init() -> Result<()> {
    system::init()?;
    Ok(())
}

/// Get a formatted summary of system information
///
/// Returns a human-readable string with system details including
/// OS, CPU, memory, and current memory pressure.
///
/// # Example
///
/// ```no_run
/// # nebula_memory::init().unwrap();
/// println!("{}", nebula_memory::info());
/// ```
#[cfg(feature = "std")]
pub fn info() -> String {
    system::summary()
}
