//! # nebula-worker-bin — core-flavor worker binary
//!
//! This crate is the runnable process that statically links the first-party
//! core plugin and runs durable control, recovery, resource fanout, and
//! timer processing via [`nebula_worker`].
//!
//! Runtime assembly lives in the shared `nebula-deployment` application package.
//! This package retains standalone configuration and credential projection.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod credential_projection;
