//! Custom Extractors
//!
//! Кастомные extractors для извлечения данных из запросов.

#[cfg(feature = "credential-oauth")]
pub mod credential;
pub mod json_extractor;

pub use json_extractor::ValidatedJson;
