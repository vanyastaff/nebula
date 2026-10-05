//! Custom Extractors
//!
//! Кастомные extractors для извлечения данных из запросов.

pub mod api_json;
pub mod credential;
pub mod json_extractor;

pub use api_json::ApiJson;
pub use json_extractor::ValidatedJson;
