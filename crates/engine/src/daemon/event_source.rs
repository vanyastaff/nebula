//! EventSource topology + `EventSourceAdapter<E>: TriggerAction`.
//!
//! Filled in by Task 5.

#![allow(missing_docs, reason = "filled by Task 5")]

use nebula_resource::Resource;

#[allow(dead_code, reason = "filled by Task 5")]
pub trait EventSource: Resource {}

#[allow(dead_code, reason = "filled by Task 5")]
pub struct EventSourceConfig {
    pub buffer_size: usize,
}

#[allow(dead_code, reason = "filled by Task 5")]
pub struct EventSourceRuntime<E: EventSource> {
    _placeholder: std::marker::PhantomData<E>,
}

#[allow(dead_code, reason = "filled by Task 5")]
pub struct EventSourceAdapter<E: EventSource> {
    _placeholder: std::marker::PhantomData<E>,
}
