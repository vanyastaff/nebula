use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::kind::ParameterKind;
use crate::metadata::ParameterMetadata;
use crate::types::*;

/// A concrete parameter definition, tagged by type.
///
/// Each variant wraps a specific parameter type struct. The `type` field
/// in JSON determines which variant is used during deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParameterDef {
    Text(TextParameter),
    Textarea(TextareaParameter),
    Code(CodeParameter),
    Secret(SecretParameter),
    Number(NumberParameter),
    Checkbox(CheckboxParameter),
    Select(SelectParameter),
    MultiSelect(MultiSelectParameter),
    Color(ColorParameter),
    DateTime(DateTimeParameter),
    Date(DateParameter),
    Time(TimeParameter),
    Hidden(HiddenParameter),
    Notice(NoticeParameter),
}

macro_rules! delegate_metadata {
    ($self:ident => $method:ident -> $ret:ty) => {
        match $self {
            Self::Text(p) => &p.metadata,
            Self::Textarea(p) => &p.metadata,
            Self::Code(p) => &p.metadata,
            Self::Secret(p) => &p.metadata,
            Self::Number(p) => &p.metadata,
            Self::Checkbox(p) => &p.metadata,
            Self::Select(p) => &p.metadata,
            Self::MultiSelect(p) => &p.metadata,
            Self::Color(p) => &p.metadata,
            Self::DateTime(p) => &p.metadata,
            Self::Date(p) => &p.metadata,
            Self::Time(p) => &p.metadata,
            Self::Hidden(p) => &p.metadata,
            Self::Notice(p) => &p.metadata,
        }
    };
}

macro_rules! delegate_display {
    ($self:ident) => {
        match $self {
            Self::Text(p) => p.display.as_ref(),
            Self::Textarea(p) => p.display.as_ref(),
            Self::Code(p) => p.display.as_ref(),
            Self::Secret(p) => p.display.as_ref(),
            Self::Number(p) => p.display.as_ref(),
            Self::Checkbox(p) => p.display.as_ref(),
            Self::Select(p) => p.display.as_ref(),
            Self::MultiSelect(p) => p.display.as_ref(),
            Self::Color(p) => p.display.as_ref(),
            Self::DateTime(p) => p.display.as_ref(),
            Self::Date(p) => p.display.as_ref(),
            Self::Time(p) => p.display.as_ref(),
            Self::Hidden(p) => p.display.as_ref(),
            Self::Notice(p) => p.display.as_ref(),
        }
    };
}

impl ParameterDef {
    /// The unique key identifying this parameter.
    #[must_use]
    pub fn key(&self) -> &str {
        let meta = delegate_metadata!(self => key -> &str);
        &meta.key
    }

    /// The human-readable display name.
    #[must_use]
    pub fn name(&self) -> &str {
        let meta = delegate_metadata!(self => name -> &str);
        &meta.name
    }

    /// The parameter kind (determines UI widget and value semantics).
    #[must_use]
    pub fn kind(&self) -> ParameterKind {
        match self {
            Self::Text(_) => ParameterKind::Text,
            Self::Textarea(_) => ParameterKind::Textarea,
            Self::Code(_) => ParameterKind::Code,
            Self::Secret(_) => ParameterKind::Secret,
            Self::Number(_) => ParameterKind::Number,
            Self::Checkbox(_) => ParameterKind::Checkbox,
            Self::Select(_) => ParameterKind::Select,
            Self::MultiSelect(_) => ParameterKind::MultiSelect,
            Self::Color(_) => ParameterKind::Color,
            Self::DateTime(_) => ParameterKind::DateTime,
            Self::Date(_) => ParameterKind::Date,
            Self::Time(_) => ParameterKind::Time,
            Self::Hidden(_) => ParameterKind::Hidden,
            Self::Notice(_) => ParameterKind::Notice,
        }
    }

    /// Access the full metadata for this parameter.
    #[must_use]
    pub fn metadata(&self) -> &ParameterMetadata {
        delegate_metadata!(self => metadata -> &ParameterMetadata)
    }

    /// Whether this parameter is required.
    #[must_use]
    pub fn is_required(&self) -> bool {
        self.metadata().required
    }

    /// Access the display configuration, if any.
    #[must_use]
    pub fn display(&self) -> Option<&ParameterDisplay> {
        delegate_display!(self)
    }

    /// Whether this parameter's value should be masked in UI and logs.
    #[must_use]
    pub fn is_sensitive(&self) -> bool {
        self.metadata().sensitive
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_and_name_delegation() {
        let def = ParameterDef::Text(TextParameter::new("host", "Hostname"));
        assert_eq!(def.key(), "host");
        assert_eq!(def.name(), "Hostname");
    }

    #[test]
    fn kind_matches_variant() {
        let cases: Vec<(ParameterDef, ParameterKind)> = vec![
            (
                ParameterDef::Text(TextParameter::new("a", "A")),
                ParameterKind::Text,
            ),
            (
                ParameterDef::Textarea(TextareaParameter::new("a", "A")),
                ParameterKind::Textarea,
            ),
            (
                ParameterDef::Code(CodeParameter::new("a", "A")),
                ParameterKind::Code,
            ),
            (
                ParameterDef::Secret(SecretParameter::new("a", "A")),
                ParameterKind::Secret,
            ),
            (
                ParameterDef::Number(NumberParameter::new("a", "A")),
                ParameterKind::Number,
            ),
            (
                ParameterDef::Checkbox(CheckboxParameter::new("a", "A")),
                ParameterKind::Checkbox,
            ),
            (
                ParameterDef::Select(SelectParameter::new("a", "A")),
                ParameterKind::Select,
            ),
            (
                ParameterDef::MultiSelect(MultiSelectParameter::new("a", "A")),
                ParameterKind::MultiSelect,
            ),
            (
                ParameterDef::Color(ColorParameter::new("a", "A")),
                ParameterKind::Color,
            ),
            (
                ParameterDef::DateTime(DateTimeParameter::new("a", "A")),
                ParameterKind::DateTime,
            ),
            (
                ParameterDef::Date(DateParameter::new("a", "A")),
                ParameterKind::Date,
            ),
            (
                ParameterDef::Time(TimeParameter::new("a", "A")),
                ParameterKind::Time,
            ),
            (
                ParameterDef::Hidden(HiddenParameter::new("a", "A")),
                ParameterKind::Hidden,
            ),
            (
                ParameterDef::Notice(NoticeParameter::new("a", "A", NoticeType::Info, "msg")),
                ParameterKind::Notice,
            ),
        ];

        for (def, expected_kind) in &cases {
            assert_eq!(
                def.kind(),
                *expected_kind,
                "kind mismatch for {:?}",
                def.key()
            );
        }
    }

    #[test]
    fn is_required_delegation() {
        let mut text = TextParameter::new("name", "Name");
        text.metadata.required = true;
        let def = ParameterDef::Text(text);
        assert!(def.is_required());

        let def2 = ParameterDef::Text(TextParameter::new("opt", "Optional"));
        assert!(!def2.is_required());
    }

    #[test]
    fn is_sensitive_for_secret() {
        let def = ParameterDef::Secret(SecretParameter::new("key", "API Key"));
        assert!(def.is_sensitive());

        let def2 = ParameterDef::Text(TextParameter::new("name", "Name"));
        assert!(!def2.is_sensitive());
    }

    #[test]
    fn serde_round_trip_text() {
        let def = ParameterDef::Text(TextParameter::new("host", "Hostname"));
        let json_str = serde_json::to_string(&def).unwrap();
        assert!(json_str.contains("\"type\":\"text\""));

        let deserialized: ParameterDef = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.key(), "host");
        assert_eq!(deserialized.kind(), ParameterKind::Text);
    }

    #[test]
    fn serde_round_trip_number() {
        let mut num = NumberParameter::new("port", "Port");
        num.default = Some(8080.0);
        let def = ParameterDef::Number(num);

        let json_str = serde_json::to_string(&def).unwrap();
        assert!(json_str.contains("\"type\":\"number\""));

        let deserialized: ParameterDef = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.kind(), ParameterKind::Number);
    }

    #[test]
    fn serde_round_trip_notice() {
        let def = ParameterDef::Notice(NoticeParameter::new(
            "warn",
            "Warning",
            NoticeType::Warning,
            "Be careful!",
        ));

        let json_str = serde_json::to_string(&def).unwrap();
        assert!(json_str.contains("\"type\":\"notice\""));

        let deserialized: ParameterDef = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.kind(), ParameterKind::Notice);
    }

    #[test]
    fn serde_deserialize_from_json_object() {
        let json = json!({
            "type": "select",
            "key": "region",
            "name": "Region",
            "options": [
                {"key": "us", "name": "US", "value": "us-east-1"},
                {"key": "eu", "name": "EU", "value": "eu-west-1"}
            ]
        });

        let def: ParameterDef = serde_json::from_value(json).unwrap();
        assert_eq!(def.key(), "region");
        assert_eq!(def.kind(), ParameterKind::Select);
    }
}
