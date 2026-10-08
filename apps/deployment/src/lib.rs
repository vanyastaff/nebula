//! Shared first-party deployment assembly.
//!
//! Server and worker applications select the same linked plugin release and
//! runtime wiring here. Executable roots retain environment configuration,
//! database opening, signals and process lifetime. This unpublished application
//! package is not a supported SDK or downstream embedding interface.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod release;
pub mod worker;

pub use release::{CoreRelease, CoreReleaseError};
