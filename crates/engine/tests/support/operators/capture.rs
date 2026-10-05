//! Structured, execution-scoped tracing evidence independent of journal reads.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};

use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};
use tracing_subscriber::{Layer, layer::Context, prelude::*, registry::LookupSpan};

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct TraceObservation {
    pub target: String,
    pub name: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct Fields(BTreeMap<String, String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
}

#[derive(Clone, Default)]
pub(super) struct TraceCapture(Arc<Mutex<Vec<TraceObservation>>>);

impl<S> Layer<S> for TraceCapture
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: Context<'_, S>) {
        let mut fields = Fields::default();
        attributes.record(&mut fields);
        context
            .span(id)
            .expect("new span exists")
            .extensions_mut()
            .insert(fields);
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, context: Context<'_, S>) {
        let span = context.span(id).expect("recorded span exists");
        let mut extensions = span.extensions_mut();
        values.record(
            extensions
                .get_mut::<Fields>()
                .expect("capture owns span fields"),
        );
    }

    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        let mut fields = Fields::default();
        if let Some(scope) = context.event_scope(event) {
            for span in scope.from_root() {
                if let Some(inherited) = span.extensions().get::<Fields>() {
                    fields.0.extend(inherited.0.clone());
                }
            }
        }
        event.record(&mut fields);
        self.0
            .lock()
            .expect("trace capture lock")
            .push(TraceObservation {
                target: event.metadata().target().to_owned(),
                name: event.metadata().name().to_owned(),
                fields: fields.0,
            });
    }

    fn on_close(&self, id: Id, context: Context<'_, S>) {
        let span = context.span(&id).expect("closing span exists");
        let mut fields = BTreeMap::new();
        for parent in span.scope().from_root() {
            if let Some(inherited) = parent.extensions().get::<Fields>() {
                fields.extend(inherited.0.clone());
            }
        }
        if fields.contains_key("outcome") {
            self.0
                .lock()
                .expect("trace capture lock")
                .push(TraceObservation {
                    target: span.metadata().target().to_owned(),
                    name: span.metadata().name().to_owned(),
                    fields,
                });
        }
    }
}

impl TraceCapture {
    pub(super) fn global() -> &'static Self {
        static CAPTURE: OnceLock<TraceCapture> = OnceLock::new();
        CAPTURE.get_or_init(|| {
            let capture = Self::default();
            tracing_subscriber::registry()
                .with(capture.clone())
                .try_init()
                .expect("operator evidence installs its dedicated global subscriber");
            capture
        })
    }

    /// Filter the process-wide stream by the scenario's real execution identity.
    pub(super) fn for_execution(&self, execution_id: &str) -> Vec<TraceObservation> {
        self.0
            .lock()
            .expect("trace capture lock")
            .iter()
            .filter(|event| {
                event
                    .fields
                    .get("execution_id")
                    .is_some_and(|value| value == execution_id)
            })
            .cloned()
            .collect()
    }
}
