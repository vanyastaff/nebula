//! Event filtering system for selective hook activation
//!
//! Allows hooks to filter events by category, pattern, or custom predicates.

use super::hooks::ObservabilityEvent;
use std::collections::HashSet;

/// Event filter for selective processing
///
/// Hooks can use filters to only process events they care about,
/// reducing overhead and improving performance.
#[derive(Debug, Clone)]
pub enum EventFilter {
    /// Allow all events
    All,
    /// Filter by event name prefix (e.g., "workflow." matches "workflow.started", "workflow.completed")
    Prefix(String),
    /// Filter by exact event name
    Exact(String),
    /// Filter by multiple event names
    Set(HashSet<String>),
    /// Custom predicate function (not cloneable, so stored as String for display)
    Custom(fn(&dyn ObservabilityEvent) -> bool),
    /// Combine multiple filters with AND logic
    And(Vec<EventFilter>),
    /// Combine multiple filters with OR logic
    Or(Vec<EventFilter>),
    /// Invert filter (NOT logic)
    Not(Box<EventFilter>),
}

impl EventFilter {
    /// Create a prefix filter
    pub fn prefix(prefix: impl Into<String>) -> Self {
        Self::Prefix(prefix.into())
    }

    /// Create an exact name filter
    pub fn exact(name: impl Into<String>) -> Self {
        Self::Exact(name.into())
    }

    /// Create a set filter from an iterator of names
    pub fn set<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Set(names.into_iter().map(Into::into).collect())
    }

    /// Create a custom filter with a predicate function
    pub fn custom(predicate: fn(&dyn ObservabilityEvent) -> bool) -> Self {
        Self::Custom(predicate)
    }

    /// Combine filters with AND logic
    pub fn and(filters: Vec<EventFilter>) -> Self {
        Self::And(filters)
    }

    /// Combine filters with OR logic
    pub fn or(filters: Vec<EventFilter>) -> Self {
        Self::Or(filters)
    }

    /// Invert this filter (NOT logic)
    ///
    /// # Example
    /// ```
    /// use nebula_log::observability::EventFilter;
    ///
    /// let filter = EventFilter::prefix("workflow.").negate();
    /// // Now matches everything EXCEPT workflow.* events
    /// ```
    pub fn negate(self) -> Self {
        Self::Not(Box::new(self))
    }

    /// Check if an event passes this filter
    pub fn matches(&self, event: &dyn ObservabilityEvent) -> bool {
        match self {
            EventFilter::All => true,
            EventFilter::Prefix(prefix) => event.name().starts_with(prefix),
            EventFilter::Exact(name) => event.name() == name,
            EventFilter::Set(names) => names.contains(event.name()),
            EventFilter::Custom(predicate) => predicate(event),
            EventFilter::And(filters) => filters.iter().all(|f| f.matches(event)),
            EventFilter::Or(filters) => filters.iter().any(|f| f.matches(event)),
            EventFilter::Not(filter) => !filter.matches(event),
        }
    }
}

/// Trait for hooks that support event filtering
///
/// Implements a default filter that allows all events.
pub trait FilteredHook {
    /// Get the event filter for this hook
    ///
    /// Override this method to provide custom filtering logic.
    fn filter(&self) -> &EventFilter {
        &EventFilter::All
    }

    /// Check if this hook should process an event
    fn should_process(&self, event: &dyn ObservabilityEvent) -> bool {
        self.filter().matches(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestEvent {
        name: String,
    }

    impl ObservabilityEvent for TestEvent {
        fn name(&self) -> &str {
            &self.name
        }
    }

    #[test]
    fn test_filter_all() {
        let filter = EventFilter::All;
        let event = TestEvent {
            name: "test".to_string(),
        };
        assert!(filter.matches(&event));
    }

    #[test]
    fn test_filter_prefix() {
        let filter = EventFilter::prefix("workflow.");

        let matching = TestEvent {
            name: "workflow.started".to_string(),
        };
        assert!(filter.matches(&matching));

        let non_matching = TestEvent {
            name: "node.started".to_string(),
        };
        assert!(!filter.matches(&non_matching));
    }

    #[test]
    fn test_filter_exact() {
        let filter = EventFilter::exact("workflow.started");

        let matching = TestEvent {
            name: "workflow.started".to_string(),
        };
        assert!(filter.matches(&matching));

        let non_matching = TestEvent {
            name: "workflow.completed".to_string(),
        };
        assert!(!filter.matches(&non_matching));
    }

    #[test]
    fn test_filter_set() {
        let filter = EventFilter::set(vec!["event1", "event2", "event3"]);

        let matching = TestEvent {
            name: "event2".to_string(),
        };
        assert!(filter.matches(&matching));

        let non_matching = TestEvent {
            name: "event4".to_string(),
        };
        assert!(!filter.matches(&non_matching));
    }

    #[test]
    fn test_filter_custom() {
        let filter = EventFilter::custom(|event| event.name().len() > 5);

        let matching = TestEvent {
            name: "long_event_name".to_string(),
        };
        assert!(filter.matches(&matching));

        let non_matching = TestEvent {
            name: "test".to_string(),
        };
        assert!(!filter.matches(&non_matching));
    }

    #[test]
    fn test_filter_and() {
        let filter = EventFilter::and(vec![
            EventFilter::prefix("workflow."),
            EventFilter::custom(|event| event.name().contains("started")),
        ]);

        let matching = TestEvent {
            name: "workflow.started".to_string(),
        };
        assert!(filter.matches(&matching));

        let non_matching1 = TestEvent {
            name: "workflow.completed".to_string(),
        };
        assert!(!filter.matches(&non_matching1));

        let non_matching2 = TestEvent {
            name: "node.started".to_string(),
        };
        assert!(!filter.matches(&non_matching2));
    }

    #[test]
    fn test_filter_or() {
        let filter = EventFilter::or(vec![
            EventFilter::exact("event1"),
            EventFilter::exact("event2"),
        ]);

        let matching1 = TestEvent {
            name: "event1".to_string(),
        };
        assert!(filter.matches(&matching1));

        let matching2 = TestEvent {
            name: "event2".to_string(),
        };
        assert!(filter.matches(&matching2));

        let non_matching = TestEvent {
            name: "event3".to_string(),
        };
        assert!(!filter.matches(&non_matching));
    }

    #[test]
    fn test_filter_not() {
        let filter = EventFilter::prefix("workflow.").negate();

        let matching = TestEvent {
            name: "node.started".to_string(),
        };
        assert!(filter.matches(&matching));

        let non_matching = TestEvent {
            name: "workflow.started".to_string(),
        };
        assert!(!filter.matches(&non_matching));
    }

    #[test]
    fn test_complex_filter() {
        // Filter for: (workflow.* OR node.*) AND NOT *.internal
        let filter = EventFilter::and(vec![
            EventFilter::or(vec![
                EventFilter::prefix("workflow."),
                EventFilter::prefix("node."),
            ]),
            EventFilter::custom(|event| !event.name().ends_with(".internal")).into(),
        ]);

        assert!(filter.matches(&TestEvent {
            name: "workflow.started".to_string()
        }));
        assert!(filter.matches(&TestEvent {
            name: "node.completed".to_string()
        }));
        assert!(!filter.matches(&TestEvent {
            name: "workflow.internal".to_string()
        }));
        assert!(!filter.matches(&TestEvent {
            name: "action.started".to_string()
        }));
    }
}
