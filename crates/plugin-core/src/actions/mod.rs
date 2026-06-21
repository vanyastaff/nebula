//! Actions provided by the `core` plugin.

pub mod aggregate;
pub mod datetime;
pub mod dedupe;
pub mod delay;
pub mod filter;
pub mod if_action;
pub mod json_transform;
pub mod map;
pub mod set_fields;
pub mod sort;
pub mod switch_action;

pub use aggregate::Aggregate;
pub use datetime::DateTimeAction;
pub use dedupe::Dedupe;
pub use delay::CoreDelay;
pub use filter::Filter;
pub use if_action::CoreIf;
pub use json_transform::JsonTransform;
pub use map::MapAction;
pub use set_fields::SetFields;
pub use sort::Sort;
pub use switch_action::CoreSwitch;
