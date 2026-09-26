//! HTTP resource adapter (`resource-http` feature): an auth-neutral
//! [`HttpTransport`] configured by [`HttpConfig`], for a resource whose
//! instance is (or holds) the transport.
//!
//! The transport follows no redirect, retries nothing, uses no proxy,
//! sends no `Referer`, keeps no cookies and holds no credential: it is
//! built once in `Provider::create` and reused across credential rotations.
//! TLS verifies against the platform's roots plus the configured extra
//! roots, with no switch to skip verification.
//!
//! Nothing here prints or logs a URL, a header value or a transport error:
//! configuration errors are permanent with static text.

mod config;

pub use config::{HttpConfig, HttpTransport};
