//! Generic event distribution layer for Nebula.
//!
//! This crate provides a broadcast-based [`EventBus<E>`] with configurable
//! [`BackPressurePolicy`]. Domain crates (telemetry, resource, etc.) own their
//! event types and construct `EventBus<ExecutionEvent>`, `EventBus<ResourceEvent>`, etc.
//! Eventbus is transport-only: no domain event types are defined here.
//!
//! Backed by [`tokio::sync::broadcast`] per architecture: bounded, Lagged semantics,
//! zero-copy clone, minimal hot-path overhead (no extra allocations on send).
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_eventbus::EventBus;
//!
//! #[derive(Clone)]
//! struct MyEvent {
//!     id: u64,
//! }
//!
//! let bus = EventBus::<MyEvent>::new(64);
//! let mut sub = bus.subscribe();
//!
//! let outcome = bus.emit(MyEvent { id: 1 });
//! assert!(outcome.is_sent());
//! let event = sub.try_recv().expect("event must be available");
//! assert_eq!(event.id, 1);
//! ```
//!
//! ## Core Types
//!
//! - [`EventBus`] - typed event broadcaster.
//! - [`BackPressurePolicy`] - buffer saturation behavior.
//! - [`PublishOutcome`] - explicit send result for control-flow decisions.
//! - [`EventBusStats`] - observability counters for sent/dropped/subscribers.
//! - [`EventBusRegistry`] - multi-bus isolation by key (e.g. per-tenant buses).
//! - [`SubscriptionScope`] and [`ScopedEvent`] - scope metadata for targeted subscriptions.
//! - [`EventFilter`] and [`FilteredSubscriber`] - predicate-based event selection.
//!
//! # Contract
//!
//! - **Non-blocking send by default** — producers never block on subscriber speed.
//! - **Best-effort delivery** — no guarantee of delivery or global ordering.
//! - **EventBusStats** — `sent_count`, `dropped_count`, `subscriber_count` for observability.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bus;
mod filter;
mod filtered_subscriber;
mod outcome;
mod policy;
mod registry;
mod scope;
mod stats;
mod subscriber;

pub use bus::EventBus;
pub use filter::EventFilter;
pub use filtered_subscriber::FilteredSubscriber;
pub use outcome::PublishOutcome;
pub use policy::BackPressurePolicy;
pub use registry::EventBusRegistry;
pub use registry::EventBusRegistryStats;
pub use scope::ScopedEvent;
pub use scope::SubscriptionScope;
pub use stats::EventBusStats;
pub use subscriber::Subscriber;

/// Alias for [`Subscriber`]; matches INTERACTIONS/ARCHITECTURE naming.
pub type EventSubscriber<E> = Subscriber<E>;
