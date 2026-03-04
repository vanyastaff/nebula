//! Cross-cutting HTTP middleware setup.

mod trace;

pub(crate) use trace::http_trace_layer;
