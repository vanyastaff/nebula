//! # Nebula Resilience
//!
//! A comprehensive resilience library for Rust applications, providing patterns
//! and tools for building fault-tolerant distributed systems.
//!
//! ## Features
//!
//! - **Circuit Breaker**: Prevent cascading failures
//! - **Retry Mechanisms**: Configurable retry strategies with backoff
//! - **Rate Limiting**: Multiple algorithms (token bucket, leaky bucket, sliding window)
//! - **Bulkhead Isolation**: Resource isolation patterns
//! - **Timeout Management**: Adaptive and hierarchical timeouts
//! - **Fallback Strategies**: Graceful degradation
//! - **Caching**: Multiple caching patterns
//! - **Load Balancing**: Various load distribution strategies
//! - **Observability**: Built-in metrics and tracing
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_resilience::prelude::*;
//! use nebula_resilience::ResilienceManager;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a resilience manager with default configuration
//!     let manager = ResilienceManager::builder()
//!         .with_circuit_breaker()
//!         .with_retry(RetryConfig::default())
//!         .with_rate_limiter(RateLimiterConfig::default())
//!         .build()?;
//!
//!     // Execute a protected operation
//!     let result = manager
//!         .execute("my-service", async {
//!             // Your potentially failing operation
//!             Ok::<_, Error>("Success")
//!         })
//!         .await?;
//!
//!     Ok(())
//! }
//! ```

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

// Core modules
pub mod circuit_breaker;
pub mod config;
pub mod error;
pub mod manager;
pub mod prelude;
pub mod retry;

// Pattern modules
pub mod backpressure;
pub mod bulkhead;
pub mod cache;
pub mod fallback;
pub mod rate_limiting;
pub mod throttling;
pub mod timeout;

// Advanced modules
pub mod degradation;
pub mod health;
pub mod load_balancing;
pub mod state;

// Infrastructure modules
pub mod observability;

// Optional modules
#[cfg(feature = "chaos")]
pub mod chaos;

#[cfg(feature = "distributed")]
pub mod distributed;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

// Re-exports for convenience
pub use config::Config;
pub use error::{Error, Result};
pub use manager::ResilienceManager;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");