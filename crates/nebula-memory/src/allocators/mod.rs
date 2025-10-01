//! Modular allocator implementations
//!
//! This module contains the reorganized allocator implementations with cleaner,
//! more maintainable structure. Each allocator is split into focused submodules
//! for better code organization and maintainability.

/// Bump allocator (arena) implementation with checkpointing support
pub mod bump;

/// Pool allocator for fixed-size object reuse
pub mod pool;

/// Stack allocator for LIFO memory management
pub mod stack;
