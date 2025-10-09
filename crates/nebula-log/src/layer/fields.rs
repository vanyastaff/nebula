//! Layer for injecting global fields
//!
//! This layer is currently a placeholder. Global fields are instead injected
//! via a root span created in [`LoggerBuilder::build()`].
//!
//! ## Implementation Status
//!
//! **Current approach:** Root span with global fields (see `builder.rs:96-107`)
//! **Future approach:** Proper layer that attaches fields to all events/spans
//!
//! ## TODO
//!
//! This layer exists for future enhancement when we need more sophisticated
//! field injection (e.g., per-event enrichment, conditional fields).
//!
//! For now, the layer is registered but does nothing - global fields work
//! through the root span mechanism.

// External dependencies
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    layer::{Context, Layer},
    registry::LookupSpan,
};

// Internal crates
use crate::config::Fields;

/// Layer that adds global fields to spans and events
///
/// **Status:** Currently a no-op placeholder. Global fields are injected
/// via a root span created during logger initialization.
///
/// See module-level documentation for details.
pub struct FieldsLayer {
    _fields: Fields,
}

impl FieldsLayer {
    pub fn new(fields: Fields) -> Self {
        Self { _fields: fields }
    }
}

impl<S> Layer<S> for FieldsLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        _attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: Context<'_, S>,
    ) {
        // TODO(feature): Implement field injection for spans
        // Currently handled by root span in builder.rs
    }

    fn on_event(&self, _event: &Event<'_>, _ctx: Context<'_, S>) {
        // TODO(feature): Implement field injection for events
        // Currently handled by root span in builder.rs
    }
}
