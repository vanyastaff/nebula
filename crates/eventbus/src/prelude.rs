//! Convenience re-exports for eventbus users.
//!
//! ```
//! use nebula_eventbus::prelude::*;
//!
//! let bus: EventBus<u32> = EventBus::new(16);
//! let _subscriber = bus.subscribe();
//!
//! // With a live subscriber the event is accepted into the buffer.
//! assert_eq!(bus.emit(42), PublishOutcome::Sent);
//!
//! let stats = bus.stats();
//! assert_eq!(stats.sent_count, 1);
//! assert_eq!(stats.subscriber_count, 1);
//! ```

pub use crate::{
    BackPressurePolicy, EventBus, EventBusRegistry, EventBusRegistryStats, EventBusStats,
    EventFilter, FilteredStream, FilteredSubscriber, PublishOutcome, ScopedEvent, Subscriber,
    SubscriberStream, SubscriptionScope,
};
