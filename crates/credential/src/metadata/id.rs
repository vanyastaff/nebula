//! Credential instance identifier.
//!
//! `CredentialId` is the system-generated ULID that identifies a specific
//! credential instance in storage (Stripe-style prefix: `cred_01J9ABCDEF...`).
//! Convention: `FooId` = system-generated ULID, `FooKey` = author-defined string.
//!
//! Defined in [`nebula_core::CredentialId`] alongside all other ULID identifiers.
//! Re-exported here for discoverability.

pub use nebula_core::CredentialId;
