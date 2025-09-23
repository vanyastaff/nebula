//! Layer for injecting global fields

// External dependencies
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

// Internal crates
use crate::config::Fields;

/// Layer that adds global fields to spans
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
        // Implementation pending
    }

    fn on_event(&self, _event: &Event<'_>, _ctx: Context<'_, S>) {
        // Implementation pending
    }
}
