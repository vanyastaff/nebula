//!
//!
//! ## Modules
//! - `allocator` - Main `StackAllocator` implementation with LIFO semantics
//! - `config` - Configuration variants (production, debug, performance)
//! - `frame` - RAII helper for automatic stack restoration
//! - `marker` - Position markers for scoped deallocation
//! A stack allocator for LIFO (Last In, First Out) memory management.
//! Stack allocator implementation
//! Supports markers for scoped deallocation.
pub mod allocator;
pub mod config;
pub mod frame;
pub mod marker;
pub use allocator::StackAllocator;
pub use config::StackConfig;
pub use frame::StackFrame;
pub use marker::StackMarker;
