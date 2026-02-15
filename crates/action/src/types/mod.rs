/// Interactive (human-in-the-loop) action types and supporting structures.
pub mod interactive;
mod process;
/// Simplified action trait for the common (80%) case.
pub mod simple;
mod stateful;
/// Streaming action types and supporting structures.
pub mod streaming;
/// Transactional action types and supporting structures.
pub mod transactional;
/// Trigger action types and supporting structures.
pub mod trigger;

pub use interactive::InteractiveAction;
pub use process::ProcessAction;
pub use simple::SimpleAction;
pub use stateful::StatefulAction;
pub use streaming::StreamingAction;
pub use transactional::TransactionalAction;
pub use trigger::TriggerAction;
