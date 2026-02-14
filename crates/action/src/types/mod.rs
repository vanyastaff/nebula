mod process;
mod stateful;
/// Trigger action types and supporting structures.
pub mod trigger;
/// Streaming action types and supporting structures.
pub mod streaming;
/// Transactional action types and supporting structures.
pub mod transactional;
/// Interactive (human-in-the-loop) action types and supporting structures.
pub mod interactive;

pub use process::ProcessAction;
pub use stateful::StatefulAction;
pub use trigger::TriggerAction;
pub use streaming::StreamingAction;
pub use transactional::TransactionalAction;
pub use interactive::InteractiveAction;
