//! Generic error wrapper.
//!
//! Placeholder — full implementation in a later task.

/// The main error wrapper that enriches any [`Classify`](crate::Classify)
/// error with details, context chain, and metadata.
///
/// Placeholder — full implementation in a later task.
#[derive(Debug)]
pub struct NebulaError<E> {
    _inner: E,
}
