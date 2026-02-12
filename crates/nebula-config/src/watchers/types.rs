//! Watcher-specific types and events

use crate::core::ConfigSource;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration watch event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigWatchEvent {
    /// Event type
    pub event_type: ConfigWatchEventType,

    /// Source that changed
    pub source: ConfigSource,

    /// File path (if applicable)
    pub path: Option<PathBuf>,

    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// Additional metadata
    pub metadata: Option<serde_json::Value>,
}

impl ConfigWatchEvent {
    /// Create a new watch event
    pub fn new(event_type: ConfigWatchEventType, source: ConfigSource) -> Self {
        Self {
            event_type,
            source,
            path: None,
            timestamp: chrono::Utc::now(),
            metadata: None,
        }
    }

    /// Set the path
    #[must_use = "builder methods must be chained or built"]
    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }

    /// Set metadata
    #[must_use = "builder methods must be chained or built"]
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Configuration watch event type
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigWatchEventType {
    /// File or resource created
    Created,

    /// File or resource modified
    Modified,

    /// File or resource deleted
    Deleted,

    /// File or resource renamed
    Renamed {
        /// Old path/name
        from: PathBuf,
        /// New path/name
        to: PathBuf,
    },

    /// Error occurred while watching
    Error(String),

    /// Other event
    Other(String),
}

impl ConfigWatchEventType {
    /// Check if this is an error event
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Check if this is a change event (created, modified, deleted)
    pub fn is_change(&self) -> bool {
        matches!(
            self,
            Self::Created | Self::Modified | Self::Deleted | Self::Renamed { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watch_event_builder() {
        let event = ConfigWatchEvent::new(ConfigWatchEventType::Modified, ConfigSource::Env)
            .with_path(PathBuf::from("/tmp/config.json"))
            .with_metadata(serde_json::json!({"reason": "manual"}));

        assert_eq!(event.event_type, ConfigWatchEventType::Modified);
        assert_eq!(event.source, ConfigSource::Env);
        assert_eq!(event.path, Some(PathBuf::from("/tmp/config.json")));
        assert!(event.metadata.is_some());
    }

    #[test]
    fn test_watch_event_type_checks() {
        assert!(!ConfigWatchEventType::Created.is_error());
        assert!(ConfigWatchEventType::Created.is_change());

        assert!(ConfigWatchEventType::Modified.is_change());
        assert!(ConfigWatchEventType::Deleted.is_change());

        let renamed = ConfigWatchEventType::Renamed {
            from: PathBuf::from("a"),
            to: PathBuf::from("b"),
        };
        assert!(renamed.is_change());
        assert!(!renamed.is_error());

        let error = ConfigWatchEventType::Error("fail".into());
        assert!(error.is_error());
        assert!(!error.is_change());

        let other = ConfigWatchEventType::Other("custom".into());
        assert!(!other.is_error());
        assert!(!other.is_change());
    }
}
