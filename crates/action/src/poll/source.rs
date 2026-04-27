//! [`PollSource`] — `TriggerSource` for periodic poll triggers.
//!
//! Poll triggers are **self-driving**: the adapter runs its own
//! sleep→poll→emit loop inside `start()` and never receives externally
//! pushed events (`accepts_events() = false`). The `Event` type is `()`
//! because `TriggerAction::handle()` is never called on a poll trigger.
//!
//! If a future poll variant becomes event-driven (e.g., a push-based
//! notification that kicks an immediate poll cycle), supersede this file
//! with a concrete event envelope and update `PollSource::Event`.

use crate::trigger::TriggerSource;

/// Trigger event source for poll-based triggers.
///
/// Poll triggers are self-driving: `PollTriggerAdapter::start` runs
/// the entire sleep→poll→emit loop internally and does not accept
/// externally pushed events. `type Event = ()` reflects that
/// [`crate::TriggerAction::handle`] is never called for this source.
#[derive(Debug, Clone, Copy)]
pub struct PollSource;

impl TriggerSource for PollSource {
    /// Poll triggers are self-driving; no external event envelope needed.
    type Event = ();
}
