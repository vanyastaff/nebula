//! Row-to-domain type conversion utilities.
//!
//! Helpers for encoding/decoding IDs, timestamps, and JSON payloads
//! when moving data between Rust domain types and database rows.

mod ids;
mod json;
mod timestamps;

pub use ids::{bytes_to_id, id_to_bytes};
pub use json::{from_json, from_json_opt, to_json};
pub use timestamps::{from_iso8601, now, to_iso8601};
