//! OAuth/OIDC HTTP infrastructure for Plane-A identity sign-in.
//!
//! Plane-B integration credentials use the universal credential acquisition
//! protocol; no raw provider authorization/callback ceremony is mounted by the
//! API. This module therefore serves only [`crate::domain::auth`].
//!
//! # Sub-modules
//!
//! | Module | Responsibility |
//! |--------|---------------|
//! | [`flow`] | Authorization URI construction and code exchange helpers |
//! | [`http`] | HTTP client for token endpoint requests |

pub mod discovery;
pub mod flow;
pub mod http;
pub mod userinfo;
