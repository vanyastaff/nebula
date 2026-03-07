//! Generic Notice parameter for display-only messages.

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::notice::NoticeType;

/// A display-only parameter that shows a message to the user.
///
/// Notice parameters have no value and no validation.
///
/// ## Example
///
/// ```
/// use nebula_parameter::typed::Notice;
/// use nebula_parameter::types::notice::NoticeType;
///
/// let warning = Notice::warning("deprecation", "Deprecation Warning")
///     .content("This API will be removed in v2.0")
///     .build();
///
/// let info = Notice::info("help", "Configuration Tip")
///     .content("Use environment variables for sensitive data")
///     .build();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// The type of notice (determines visual style).
    pub notice_type: NoticeType,

    /// The message content to display.
    pub content: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,
}

impl Notice {
    /// Create a notice builder with info severity.
    #[must_use]
    pub fn info(key: impl Into<String>, name: impl Into<String>) -> NoticeBuilder {
        NoticeBuilder::new(key, name, NoticeType::Info)
    }

    /// Create a notice builder with warning severity.
    #[must_use]
    pub fn warning(key: impl Into<String>, name: impl Into<String>) -> NoticeBuilder {
        NoticeBuilder::new(key, name, NoticeType::Warning)
    }

    /// Create a notice builder with error severity.
    #[must_use]
    pub fn error(key: impl Into<String>, name: impl Into<String>) -> NoticeBuilder {
        NoticeBuilder::new(key, name, NoticeType::Error)
    }

    /// Create a notice builder with success severity.
    #[must_use]
    pub fn success(key: impl Into<String>, name: impl Into<String>) -> NoticeBuilder {
        NoticeBuilder::new(key, name, NoticeType::Success)
    }

    /// Create a notice with explicit type.
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

/// Builder for Notice parameters.
#[derive(Debug)]
pub struct NoticeBuilder {
    metadata: ParameterMetadata,
    notice_type: NoticeType,
    content: String,
    display: Option<ParameterDisplay>,
}

impl NoticeBuilder {
    fn new(key: impl Into<String>, name: impl Into<String>, notice_type: NoticeType) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            notice_type,
            content: String::new(),
            display: None,
        }
    }

    /// Set the notice content message.
    #[must_use]
    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = content.into();
        self
    }

    /// Set the description (optional).
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.metadata.description = Some(desc.into());
        self
    }

    /// Build the Notice parameter.
    #[must_use]
    pub fn build(self) -> Notice {
        Notice {
            metadata: self.metadata,
            notice_type: self.notice_type,
            content: self.content,
            display: self.display,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_creates_warning_notice() {
        let notice = Notice::warning("deprecated", "Deprecation Warning")
            .content("This API is deprecated")
            .build();

        assert_eq!(notice.metadata.key, "deprecated");
        assert_eq!(notice.notice_type, NoticeType::Warning);
        assert_eq!(notice.content, "This API is deprecated");
    }

    #[test]
    fn builder_creates_info_notice() {
        let notice = Notice::info("tip", "Helpful Tip")
            .content("Use X for Y")
            .build();

        assert_eq!(notice.notice_type, NoticeType::Info);
    }
}
