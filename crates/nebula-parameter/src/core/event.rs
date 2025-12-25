//! Parameter events for reactive updates

use nebula_core::ParameterKey;
use nebula_validator::core::ValidationError;
use nebula_value::Value;

use super::ParameterFlags;

/// Events emitted by ParameterContext when state changes.
#[derive(Debug, Clone)]
pub enum ParameterEvent {
    /// A parameter value was changed.
    ValueChanged {
        key: ParameterKey,
        old: Value,
        new: Value,
    },

    /// Parameter state flags changed.
    StateChanged {
        key: ParameterKey,
        old_flags: ParameterFlags,
        new_flags: ParameterFlags,
    },

    /// Validation completed for a parameter.
    Validated {
        key: ParameterKey,
        errors: Vec<ValidationError>,
    },

    /// Parameter visibility changed (display conditions).
    VisibilityChanged { key: ParameterKey, visible: bool },

    /// All values were loaded (initial load or reset).
    Loaded,

    /// All values were cleared.
    Cleared,
}

impl ParameterEvent {
    /// Get the parameter key if this event is about a specific parameter.
    #[must_use]
    pub fn key(&self) -> Option<&ParameterKey> {
        match self {
            Self::ValueChanged { key, .. }
            | Self::StateChanged { key, .. }
            | Self::Validated { key, .. }
            | Self::VisibilityChanged { key, .. } => Some(key),
            Self::Loaded | Self::Cleared => None,
        }
    }

    /// Check if this is a value change event.
    #[must_use]
    pub fn is_value_changed(&self) -> bool {
        matches!(self, Self::ValueChanged { .. })
    }

    /// Check if this is a validation event.
    #[must_use]
    pub fn is_validated(&self) -> bool {
        matches!(self, Self::Validated { .. })
    }

    /// Check if this is a visibility change event.
    #[must_use]
    pub fn is_visibility_changed(&self) -> bool {
        matches!(self, Self::VisibilityChanged { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> ParameterKey {
        ParameterKey::new("test").unwrap()
    }

    #[test]
    fn test_event_key() {
        let event = ParameterEvent::ValueChanged {
            key: test_key(),
            old: Value::Null,
            new: Value::text("hello"),
        };
        assert_eq!(event.key(), Some(&test_key()));

        let event = ParameterEvent::Loaded;
        assert_eq!(event.key(), None);
    }

    #[test]
    fn test_event_type_checks() {
        let event = ParameterEvent::ValueChanged {
            key: test_key(),
            old: Value::Null,
            new: Value::text("hello"),
        };
        assert!(event.is_value_changed());
        assert!(!event.is_validated());

        let event = ParameterEvent::Validated {
            key: test_key(),
            errors: vec![],
        };
        assert!(event.is_validated());
        assert!(!event.is_value_changed());
    }
}
