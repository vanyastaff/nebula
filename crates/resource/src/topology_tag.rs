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
    /// Bounded — a runtime concurrency cap over a non-pooled resource
    /// (capped / exclusive / unbounded).
    Bounded,
    /// A custom author-supplied [`Topology`](crate::topology::Topology) that is
    /// neither the built-in pool nor resident — used to label rotation /
    /// diagnostic spans for open topologies.
    Custom,
}

impl TopologyTag {
    /// Returns the tag as a static string slice.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pool => "pool",
            Self::Resident => "resident",
            Self::Bounded => "bounded",
            Self::Custom => "custom",
        }
    }
}

impl fmt::Display for TopologyTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
