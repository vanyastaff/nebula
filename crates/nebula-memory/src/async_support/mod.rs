//! Async/await support for nebula-memory
//!
//! This module provides async-friendly versions of allocators and pools
//! that work seamlessly with Tokio and other async runtimes.
//!
//! # Features
//!
//! - `AsyncArena` - Async-friendly arena allocator with RwLock
//! - `AsyncPool` - Async object pool with semaphore-based backpressure
//! - Proper async/await integration
//! - Backpressure control with semaphores
//! - Concurrent access with async locks
//!
//! # Examples
//!
//! ```ignore
//! use nebula_memory::async_support::{AsyncArena, AsyncPool};
//!
//! #[tokio::main]
//! async fn main() {
//!     // Async arena
//!     let arena = AsyncArena::new();
//!     let value = arena.alloc(42).await.unwrap();
//!
//!     // Async pool
//!     let pool = AsyncPool::new(10, || String::new());
//!     let obj = pool.acquire().await.unwrap();
//! }
//! ```

#![cfg(feature = "async")]

pub mod arena;
pub mod pool;

pub use arena::{ArenaHandle, AsyncArena, AsyncArenaScope};
pub use pool::{AsyncPool, AsyncPooledValue};
