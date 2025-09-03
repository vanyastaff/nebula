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
pub mod bulkhead;
pub mod circuit_breaker;
pub mod error;
pub mod policy;
pub mod retry;
pub mod timeout;

// Re-exports for convenience
pub use crate::timeout::timeout;
pub use error::{ResilienceError, ResilienceResult};
pub use policy::policies;
pub use policy::{ResilienceBuilder, ResiliencePolicy};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
