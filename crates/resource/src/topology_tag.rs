//! Topology identifier tag.

use std::fmt;

/// Identifies which topology a resource handle was acquired from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TopologyTag {
    /// Pool — N interchangeable instances.
    Pool,
    /// Resident — one shared instance, clone on acquire.
    Resident,
    /// Service — long-lived runtime, short-lived tokens.
    Service,
    /// Transport — shared connection, multiplexed sessions.
    Transport,
    /// Exclusive — one caller at a time.
    Exclusive,
    /// EventSource — pull-based event subscription.
    EventSource,
    /// Daemon — background run loop.
    Daemon,
}

impl TopologyTag {
    /// Returns the tag as a static string slice.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pool => "pool",
            Self::Resident => "resident",
            Self::Service => "service",
            Self::Transport => "transport",
            Self::Exclusive => "exclusive",
            Self::EventSource => "event_source",
            Self::Daemon => "daemon",
        }
    }
}

impl fmt::Display for TopologyTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
