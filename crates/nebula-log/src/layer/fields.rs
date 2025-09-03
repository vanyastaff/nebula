//! Layer for injecting global fields

use crate::config::Fields;
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    layer::{Context, Layer},
    registry::LookupSpan,
};

/// Layer that adds global fields to spans
pub struct FieldsLayer {
    fields: Fields,
}

impl FieldsLayer {
    pub fn new(fields: Fields) -> Self {
        Self { fields }
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
        // TODO: Add global fields to span
    }

    fn on_event(&self, _event: &Event<'_>, _ctx: Context<'_, S>) {
        // TODO: Add global fields to event
    }
}
