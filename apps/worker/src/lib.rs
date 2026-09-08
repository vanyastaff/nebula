//! # nebula-worker-bin — core-flavor worker binary
//!
//! This crate is the runnable process that statically links the first-party
//! [`CorePlugin`] and runs durable control, recovery, and timer processing via
//! [`nebula_worker`].
//!
//! The `compose` module is public so integration tests can drive the
//! composition root directly with in-memory adapters, proving the full
//! boot → plugin-wire → claim → drive → complete path without SQLite I/O.
//!
//! [`CorePlugin`]: nebula_plugin_core::CorePlugin

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod compose;
