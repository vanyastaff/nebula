//! # nebula-credential-runtime
//!
//! **Role:** Credential management runtime — the single owner of the
//! credential *management bounded context*. Sole public entry is
//! `CredentialService` (lands in a later increment); all
//! invariant-bearing composition is crate-private so the secure
//! construction path is the only path.
//!
//! Exec tier. Narrowly supersedes the facade-ownership slice of
//! ADR-0030 (engine retains the low-level resolver / RefreshCoordinator
//! / lease mechanism); see `docs/adr/0052-credential-runtime-crate.md`.
//!
//! This increment ships only the crate scaffold and the
//! [`CredentialServiceError`](error::CredentialServiceError) taxonomy.
#![forbid(unsafe_code)]

pub mod error;
pub mod observer;
pub mod scope;

pub use error::CredentialServiceError;
pub use observer::{CredentialObserver, EventMetricObserver, NoopObserver};
pub use scope::{FixedScopeResolver, TenantScope};
