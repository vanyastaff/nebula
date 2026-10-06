//! Custom Extractors
//!
//! Кастомные extractors для извлечения данных из запросов.

pub mod api_json;
pub mod api_query;
pub mod credential;

pub use api_json::ApiJson;
pub use api_query::ApiQuery;
