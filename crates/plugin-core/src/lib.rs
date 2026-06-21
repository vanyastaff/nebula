//! # nebula-plugin-core
//!
//! First-party **core** plugin for Nebula.
//!
//! Provides foundational utility actions available in every deployment without
//! any external dependencies or credentials.
//!
//! ## Actions
//!
//! | Key | Description |
//! |-----|-------------|
//! | `core.aggregate`       | Reduce an array of objects to grouped/scalar summaries (sum/count/avg/min/max/collect/join) |
//! | `core.array`           | Shape a JSON array with chunk/flatten/take/skip operations applied left-to-right |
//! | `core.set_fields`      | Merge a list of named field assignments onto a JSON object      |
//! | `core.json_transform`  | Apply pick/omit/rename operations to a JSON object             |
//! | `core.datetime`        | Offset-aware RFC3339 timestamp formatting, parsing, arithmetic, and diff |
//! | `core.dedupe`          | Remove duplicate array elements by one or more key fields (first occurrence wins) |
//! | `core.filter`          | Filter array elements by a condition                           |
//! | `core.if`              | Binary branch: route to `"true"` or `"false"` port on a field condition |
//! | `core.map`             | Reshape each element of an array (per-element pick/omit/rename) |
//! | `core.sort`            | Sort an array of objects by one or more fields (asc/desc)           |
//! | `core.switch`          | N-way branch: route to the first matching case port, or `"default"` |
//!
//! ## Usage
//!
//! Wire the plugin into the engine via `WorkflowEngine::with_plugin`:
//!
//! ```rust,ignore
//! use nebula_engine::WorkflowEngine;
//! use nebula_plugin::ResolvedPlugin;
//! use nebula_plugin_core::CorePlugin;
//!
//! let plugin = ResolvedPlugin::from(CorePlugin::try_new()?)
//!     .expect("core plugin must resolve");
//! let engine = engine.with_plugin(std::sync::Arc::new(plugin))?;
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod actions;
pub mod condition;
mod plugin;
mod util;

pub use plugin::CorePlugin;
