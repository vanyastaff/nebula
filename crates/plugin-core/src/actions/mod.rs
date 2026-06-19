//! Actions provided by the `core` plugin.

pub mod datetime;
pub mod filter;
pub mod if_action;
pub mod json_transform;
pub mod set_fields;
pub mod switch_action;

pub use datetime::DateTimeAction;
pub use filter::Filter;
pub use if_action::CoreIf;
pub use json_transform::JsonTransform;
pub use set_fields::SetFields;
pub use switch_action::CoreSwitch;
