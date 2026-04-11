//! Convenience re-exports for eventbus users.
//!
//! ```rust,ignore
//! use nebula_eventbus::prelude::*;
//! ```

pub use crate::{
    BackPressurePolicy, EventBus, EventBusRegistry, EventBusRegistryStats, EventBusStats,
    EventFilter, FilteredStream, FilteredSubscriber, PublishOutcome, ScopedEvent, Subscriber,
    SubscriberStream, SubscriptionScope,
};
