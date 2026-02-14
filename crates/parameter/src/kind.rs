use serde::{Deserialize, Serialize};

/// The kind of a parameter, determining its UI widget and value semantics.
///
/// Phase 1 covers the 14 most common parameter kinds. Composite kinds
/// (Collection, FixedCollection, ResourceLocator, etc.) will be added
/// in a later phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterKind {
    Text,
    Textarea,
    Secret,
    Number,
    Checkbox,
    Select,
    MultiSelect,
    DateTime,
    Date,
    Time,
    Code,
    Color,
    Hidden,
    Notice,
    Object,
    List,
    Mode,
    Group,
    Expirable,
}

/// Capabilities that a parameter kind may support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterCapability {
    HasValue,
    Editable,
    Validatable,
    Displayable,
    Requirable,
    SupportsExpressions,
    Container,
    Interactive,
    Serializable,
}

impl ParameterKind {
    /// Returns the set of capabilities for this parameter kind.
    #[must_use]
    pub fn capabilities(&self) -> &'static [ParameterCapability] {
        use ParameterCapability::*;

        // Common capability sets to avoid repetition.
        const VALUE_EDITABLE: &[ParameterCapability] = &[
            HasValue,
            Editable,
            Validatable,
            Displayable,
            Requirable,
            SupportsExpressions,
            Serializable,
        ];
        const SELECTION: &[ParameterCapability] = &[
            HasValue,
            Editable,
            Validatable,
            Displayable,
            Requirable,
            SupportsExpressions,
            Interactive,
            Serializable,
        ];

        match self {
            Self::Text
            | Self::Textarea
            | Self::Number
            | Self::DateTime
            | Self::Date
            | Self::Time
            | Self::Color => VALUE_EDITABLE,
            Self::Secret => &[
                HasValue,
                Editable,
                Validatable,
                Requirable,
                SupportsExpressions,
                Serializable,
            ],
            Self::Checkbox => &[HasValue, Editable, Displayable, Serializable],
            Self::Select | Self::MultiSelect => SELECTION,
            Self::Code => &[
                HasValue,
                Editable,
                Validatable,
                Displayable,
                Requirable,
                SupportsExpressions,
                Interactive,
                Serializable,
            ],
            Self::Hidden => &[HasValue, Serializable],
            Self::Notice => &[Displayable],
            Self::Object => &[
                HasValue, Container, Displayable, Requirable, Validatable, Serializable,
            ],
            Self::List => &[
                HasValue, Container, Editable, Displayable, Requirable, Validatable, Interactive,
                Serializable,
            ],
            Self::Mode => &[
                HasValue, Container, Editable, Displayable, Requirable, Interactive, Serializable,
            ],
            Self::Group => &[Container, Displayable],
            Self::Expirable => &[
                HasValue, Container, Displayable, Requirable, Validatable, Serializable,
            ],
        }
    }

    /// Check whether this kind has a specific capability.
    #[must_use]
    pub fn has_capability(&self, cap: ParameterCapability) -> bool {
        self.capabilities().contains(&cap)
    }

    /// Whether this kind carries a value (as opposed to display-only).
    #[must_use]
    pub fn has_value(&self) -> bool {
        self.has_capability(ParameterCapability::HasValue)
    }

    /// Whether the user can edit this parameter in the UI.
    #[must_use]
    pub fn is_editable(&self) -> bool {
        self.has_capability(ParameterCapability::Editable)
    }

    /// Whether validation rules can be applied.
    #[must_use]
    pub fn is_validatable(&self) -> bool {
        self.has_capability(ParameterCapability::Validatable)
    }

    /// Whether this parameter is shown in the UI.
    #[must_use]
    pub fn is_displayable(&self) -> bool {
        self.has_capability(ParameterCapability::Displayable)
    }

    /// Whether this parameter can be marked as required.
    #[must_use]
    pub fn is_requirable(&self) -> bool {
        self.has_capability(ParameterCapability::Requirable)
    }

    /// Whether expression evaluation is supported for this kind.
    #[must_use]
    pub fn supports_expressions(&self) -> bool {
        self.has_capability(ParameterCapability::SupportsExpressions)
    }

    /// Whether this kind acts as a container for child parameters.
    #[must_use]
    pub fn is_container(&self) -> bool {
        self.has_capability(ParameterCapability::Container)
    }

    /// Whether this kind is text-based (string value).
    #[must_use]
    pub fn is_text_based(&self) -> bool {
        matches!(
            self,
            Self::Text | Self::Textarea | Self::Secret | Self::Code | Self::Color
        )
    }

    /// Whether this kind uses a selection from options.
    #[must_use]
    pub fn is_selection_based(&self) -> bool {
        matches!(self, Self::Select | Self::MultiSelect)
    }

    /// Whether this kind deals with date/time values.
    #[must_use]
    pub fn is_temporal(&self) -> bool {
        matches!(self, Self::DateTime | Self::Date | Self::Time)
    }

    /// String identifier for serialization/logging.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Textarea => "textarea",
            Self::Secret => "secret",
            Self::Number => "number",
            Self::Checkbox => "checkbox",
            Self::Select => "select",
            Self::MultiSelect => "multi_select",
            Self::DateTime => "date_time",
            Self::Date => "date",
            Self::Time => "time",
            Self::Code => "code",
            Self::Color => "color",
            Self::Hidden => "hidden",
            Self::Notice => "notice",
            Self::Object => "object",
            Self::List => "list",
            Self::Mode => "mode",
            Self::Group => "group",
            Self::Expirable => "expirable",
        }
    }

    /// The JSON value type this parameter expects.
    #[must_use]
    pub fn value_type(&self) -> &'static str {
        match self {
            Self::Text | Self::Textarea | Self::Secret | Self::Code | Self::Color => "string",
            Self::Number => "number",
            Self::Checkbox => "boolean",
            Self::Select => "any",
            Self::MultiSelect => "array",
            Self::DateTime | Self::Date | Self::Time => "string",
            Self::Hidden => "any",
            Self::Notice => "none",
            Self::Object => "object",
            Self::List => "array",
            Self::Mode => "object",
            Self::Group => "none",
            Self::Expirable => "any",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_kinds_have_capabilities() {
        let kinds = [
            ParameterKind::Text,
            ParameterKind::Textarea,
            ParameterKind::Secret,
            ParameterKind::Number,
            ParameterKind::Checkbox,
            ParameterKind::Select,
            ParameterKind::MultiSelect,
            ParameterKind::DateTime,
            ParameterKind::Date,
            ParameterKind::Time,
            ParameterKind::Code,
            ParameterKind::Color,
            ParameterKind::Hidden,
            ParameterKind::Notice,
            ParameterKind::Object,
            ParameterKind::List,
            ParameterKind::Mode,
            ParameterKind::Group,
            ParameterKind::Expirable,
        ];

        for kind in &kinds {
            assert!(
                !kind.capabilities().is_empty(),
                "{:?} should have at least one capability",
                kind
            );
        }
    }

    #[test]
    fn notice_is_display_only() {
        let notice = ParameterKind::Notice;
        assert!(!notice.has_value());
        assert!(!notice.is_editable());
        assert!(notice.is_displayable());
        assert_eq!(notice.value_type(), "none");
    }

    #[test]
    fn hidden_carries_value_but_is_not_displayed() {
        let hidden = ParameterKind::Hidden;
        assert!(hidden.has_value());
        assert!(!hidden.is_displayable());
        assert!(!hidden.is_editable());
    }

    #[test]
    fn secret_is_not_displayable() {
        let secret = ParameterKind::Secret;
        assert!(secret.has_value());
        assert!(secret.is_editable());
        assert!(!secret.is_displayable());
    }

    #[test]
    fn text_based_classification() {
        assert!(ParameterKind::Text.is_text_based());
        assert!(ParameterKind::Textarea.is_text_based());
        assert!(ParameterKind::Secret.is_text_based());
        assert!(ParameterKind::Code.is_text_based());
        assert!(ParameterKind::Color.is_text_based());

        assert!(!ParameterKind::Number.is_text_based());
        assert!(!ParameterKind::Checkbox.is_text_based());
        assert!(!ParameterKind::Select.is_text_based());
    }

    #[test]
    fn selection_based_classification() {
        assert!(ParameterKind::Select.is_selection_based());
        assert!(ParameterKind::MultiSelect.is_selection_based());

        assert!(!ParameterKind::Text.is_selection_based());
        assert!(!ParameterKind::Number.is_selection_based());
    }

    #[test]
    fn temporal_classification() {
        assert!(ParameterKind::DateTime.is_temporal());
        assert!(ParameterKind::Date.is_temporal());
        assert!(ParameterKind::Time.is_temporal());

        assert!(!ParameterKind::Text.is_temporal());
        assert!(!ParameterKind::Number.is_temporal());
    }

    #[test]
    fn as_str_round_trips_through_serde() {
        let kinds = [
            ParameterKind::Text,
            ParameterKind::Textarea,
            ParameterKind::Secret,
            ParameterKind::Number,
            ParameterKind::Checkbox,
            ParameterKind::Select,
            ParameterKind::MultiSelect,
            ParameterKind::DateTime,
            ParameterKind::Date,
            ParameterKind::Time,
            ParameterKind::Code,
            ParameterKind::Color,
            ParameterKind::Hidden,
            ParameterKind::Notice,
            ParameterKind::Object,
            ParameterKind::List,
            ParameterKind::Mode,
            ParameterKind::Group,
            ParameterKind::Expirable,
        ];

        for kind in &kinds {
            let json = serde_json::to_string(kind).unwrap();
            let quoted = format!("\"{}\"", kind.as_str());
            assert_eq!(json, quoted, "as_str mismatch for {:?}", kind);

            let deserialized: ParameterKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*kind, deserialized);
        }
    }

    #[test]
    fn value_types_are_valid() {
        let valid = ["string", "number", "boolean", "any", "array", "none", "object"];
        let kinds = [
            ParameterKind::Text,
            ParameterKind::Textarea,
            ParameterKind::Secret,
            ParameterKind::Number,
            ParameterKind::Checkbox,
            ParameterKind::Select,
            ParameterKind::MultiSelect,
            ParameterKind::DateTime,
            ParameterKind::Date,
            ParameterKind::Time,
            ParameterKind::Code,
            ParameterKind::Color,
            ParameterKind::Hidden,
            ParameterKind::Notice,
            ParameterKind::Object,
            ParameterKind::List,
            ParameterKind::Mode,
            ParameterKind::Group,
            ParameterKind::Expirable,
        ];

        for kind in &kinds {
            assert!(
                valid.contains(&kind.value_type()),
                "{:?} has unexpected value_type: {}",
                kind,
                kind.value_type()
            );
        }
    }

    #[test]
    fn no_phase1_kinds_are_containers() {
        let kinds = [
            ParameterKind::Text,
            ParameterKind::Textarea,
            ParameterKind::Secret,
            ParameterKind::Number,
            ParameterKind::Checkbox,
            ParameterKind::Select,
            ParameterKind::MultiSelect,
            ParameterKind::DateTime,
            ParameterKind::Date,
            ParameterKind::Time,
            ParameterKind::Code,
            ParameterKind::Color,
            ParameterKind::Hidden,
            ParameterKind::Notice,
        ];

        for kind in &kinds {
            assert!(
                !kind.is_container(),
                "{:?} should not be a container in phase 1",
                kind
            );
        }
    }

    #[test]
    fn selection_kinds_are_interactive() {
        assert!(ParameterKind::Select.has_capability(ParameterCapability::Interactive));
        assert!(ParameterKind::MultiSelect.has_capability(ParameterCapability::Interactive));
        assert!(ParameterKind::Code.has_capability(ParameterCapability::Interactive));
    }

    #[test]
    fn all_container_kinds_are_containers() {
        let containers = [
            ParameterKind::Object,
            ParameterKind::List,
            ParameterKind::Mode,
            ParameterKind::Group,
            ParameterKind::Expirable,
        ];

        for kind in &containers {
            assert!(
                kind.is_container(),
                "{:?} should be a container",
                kind
            );
        }
    }

    #[test]
    fn group_has_no_value() {
        let group = ParameterKind::Group;
        assert!(!group.has_value());
        assert!(group.is_container());
        assert!(group.is_displayable());
        assert_eq!(group.value_type(), "none");
    }

    #[test]
    fn container_value_types() {
        assert_eq!(ParameterKind::Object.value_type(), "object");
        assert_eq!(ParameterKind::List.value_type(), "array");
        assert_eq!(ParameterKind::Mode.value_type(), "object");
        assert_eq!(ParameterKind::Group.value_type(), "none");
        assert_eq!(ParameterKind::Expirable.value_type(), "any");
    }
}
