use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;

/// The severity/style of a notice parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeType {
    Info,
    Warning,
    Error,
    Success,
}

/// A display-only parameter that shows a message to the user.
///
/// Notice parameters have no value and no validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoticeParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// The type of notice (determines visual style).
    pub notice_type: NoticeType,

    /// The message content to display.
    pub content: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,
}

impl NoticeParameter {
    #[must_use]
    pub fn new(
        key: impl Into<String>,
        name: impl Into<String>,
        notice_type: NoticeType,
        content: impl Into<String>,
    ) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            notice_type,
            content: content.into(),
            display: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_notice() {
        let p = NoticeParameter::new(
            "api_warning",
            "API Warning",
            NoticeType::Warning,
            "This API is deprecated.",
        );
        assert_eq!(p.metadata.key, "api_warning");
        assert_eq!(p.notice_type, NoticeType::Warning);
        assert_eq!(p.content, "This API is deprecated.");
    }

    #[test]
    fn serde_round_trip() {
        let p = NoticeParameter::new(
            "info",
            "Information",
            NoticeType::Info,
            "Configure your settings below.",
        );

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: NoticeParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "info");
        assert_eq!(deserialized.notice_type, NoticeType::Info);
        assert_eq!(deserialized.content, "Configure your settings below.");
    }

    #[test]
    fn notice_type_serde() {
        for nt in [
            NoticeType::Info,
            NoticeType::Warning,
            NoticeType::Error,
            NoticeType::Success,
        ] {
            let json = serde_json::to_string(&nt).unwrap();
            let deserialized: NoticeType = serde_json::from_str(&json).unwrap();
            assert_eq!(nt, deserialized);
        }
    }
}
