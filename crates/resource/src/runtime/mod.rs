//! Topology runtime implementations.
//!
//! Each access pattern has a framework topology struct that owns instance
//! storage and drives the provider hooks:
//!
//! - [`pool::Pooled<R>`] — N interchangeable instances over an
//!   [`InstanceStore`](crate::topology::store::InstanceStore) idle queue.
//! - [`resident::Resident<R>`] — one shared instance in a lock-free cell.
//! - [`bounded::Bounded<R>`] — a runtime concurrency cap over a non-pooled
//!   resource (capped / exclusive / unbounded).
//!
//! All implement the open [`Topology`](crate::topology::Topology) contract, so
//! they are reached monomorphically through a resource's
//! [`Provider::Topology`](crate::resource::Provider::Topology) associated type.
//! There is no dispatch enum: the single blanket
//! `impl ManagedHandle for ManagedResource<R>` in [`crate::registry`] calls
//! `self.topology.{try_reserve, acquire, phase, load, tag}` directly.

pub mod bounded;
pub mod managed;
pub mod pool;
pub mod resident;
