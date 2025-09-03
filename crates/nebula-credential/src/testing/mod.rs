//! Testing utilities for credential management
//!
//! This module provides mock implementations and helpers for testing
//! credential-based code without real infrastructure.

#[cfg(any(test, feature = "testing"))]
pub mod mocks;

#[cfg(any(test, feature = "testing"))]
pub mod fixtures;

#[cfg(any(test, feature = "testing"))]
pub mod helpers;

#[cfg(any(test, feature = "testing"))]
pub mod assertions;

#[cfg(any(test, feature = "testing"))]
pub use self::mocks::*;

#[cfg(any(test, feature = "testing"))]
pub use self::fixtures::*;

#[cfg(any(test, feature = "testing"))]
pub use self::helpers::*;

#[cfg(any(test, feature = "testing"))]
pub use self::assertions::*;
