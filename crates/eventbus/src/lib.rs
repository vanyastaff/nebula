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
//! ```no_run
//! use nebula_eventbus::EventBus;
//!
//! #[derive(Clone)]
//! struct MyEvent {
//!     id: u64,
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let bus = EventBus::<MyEvent>::new(64);
//!     let mut sub = bus.subscribe();
//!
//!     let outcome = bus.emit(MyEvent { id: 1 });
//!     assert!(outcome.is_sent());
//!     let event = sub.try_recv().expect("event must be available");
//!     assert_eq!(event.id, 1);
//! }
//! ```
//!
//! ## Subscriber Lifecycle
//!
//! ### Slow Subscribers and Lag
//!
//! When a subscriber is slow relative to the emit rate, it may fall behind **buffer_size** events.
//! Upon calling [`recv()`](crate::Subscriber::recv) or [`try_recv()`](crate::Subscriber::try_recv),
//! the subscriber automatically skips lagged events and re-positions to the latest, allowing
//! the producer to continue unblocked.
//!
//! **Monitoring lag:** Use [`lagged_count()`](crate::Subscriber::lagged_count) to track how many
//! events were skipped:
//!
//! ```no_run
//! use nebula_eventbus::EventBus;
//! # #[derive(Clone)]
//! # struct Event(u64);
//! # #[tokio::main]
//! # async fn main() {
//! # let bus = EventBus::<Event>::new(10);
//! let mut sub = bus.subscribe();
//! // ... emit 20 events with a slow subscriber ...
//! if let Some(_) = sub.recv().await {
//!     if sub.lagged_count() > 0 {
//!         println!("Fell behind by {} events", sub.lagged_count());
//!     }
//! }
//! # }
//! ```
//!
//! ### Buffer Overflow Recovery
//!
//! Subscribers do not reconnect or restart when they lag. Instead, they automatically
//! re-position to the most recent event and continue receiving. This is transparent
//! to the subscriber logic.
//!
//! ### Closure and Drop
//!
//! When a [`Subscriber`](crate::Subscriber) is dropped, it automatically decrements the
//! subscriber count. No explicit close call is required. Use [`is_closed()`](crate::Subscriber::is_closed)
//! to check if the underlying bus has been dropped.
//!
//! ## Architecture Note: Persistence
//!
//! EventBus is **in-memory only** in Phase 2. This means:
//! - Events are not persisted to storage.
//! - Subscribers will lose events if they run out of buffer space or disconnect.
//! - Persistence and durability are planned for Phase 3.
//!
//! For reliable event delivery in production, consider implementing persistence at the
//! application layer or waiting for Phase 3 enhancements.
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
