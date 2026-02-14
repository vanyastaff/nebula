mod process;
mod stateful;
/// Trigger action types and supporting structures.
pub mod trigger;

pub use process::ProcessAction;
pub use stateful::StatefulAction;
pub use trigger::TriggerAction;
