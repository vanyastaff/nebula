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
    Object(ObjectParameter),
    List(ListParameter),
    Mode(ModeParameter),
    Group(GroupParameter),
    Expirable(ExpirableParameter),
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
            Self::Object(p) => &p.metadata,
            Self::List(p) => &p.metadata,
            Self::Mode(p) => &p.metadata,
            Self::Group(p) => &p.metadata,
            Self::Expirable(p) => &p.metadata,
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
            Self::Object(p) => p.display.as_ref(),
            Self::List(p) => p.display.as_ref(),
            Self::Mode(p) => p.display.as_ref(),
            Self::Group(p) => p.display.as_ref(),
            Self::Expirable(p) => p.display.as_ref(),
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
            Self::Object(_) => ParameterKind::Object,
            Self::List(_) => ParameterKind::List,
            Self::Mode(_) => ParameterKind::Mode,
            Self::Group(_) => ParameterKind::Group,
            Self::Expirable(_) => ParameterKind::Expirable,
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

    /// Returns references to child parameters for container types.
    ///
    /// Returns `None` for scalar types and `Some(vec)` for containers.
    #[must_use]
    pub fn children(&self) -> Option<Vec<&ParameterDef>> {
        match self {
            Self::Object(p) => Some(p.fields.iter().collect()),
            Self::List(p) => Some(vec![p.item_template.as_ref()]),
            Self::Mode(p) => Some(
                p.variants
                    .iter()
                    .flat_map(|v| v.parameters.iter())
                    .collect(),
            ),
            Self::Group(p) => Some(p.parameters.iter().collect()),
            Self::Expirable(p) => Some(vec![p.inner.as_ref()]),
            _ => None,
        }
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

    #[test]
    fn kind_matches_container_variants() {
        let cases: Vec<(ParameterDef, ParameterKind)> = vec![
            (
                ParameterDef::Object(ObjectParameter::new("a", "A")),
                ParameterKind::Object,
            ),
            (
                ParameterDef::List(ListParameter::new(
                    "a",
                    "A",
                    ParameterDef::Text(TextParameter::new("item", "Item")),
                )),
                ParameterKind::List,
            ),
            (
                ParameterDef::Mode(ModeParameter::new("a", "A")),
                ParameterKind::Mode,
            ),
            (
                ParameterDef::Group(GroupParameter::new("a", "A")),
                ParameterKind::Group,
            ),
            (
                ParameterDef::Expirable(ExpirableParameter::new(
                    "a",
                    "A",
                    ParameterDef::Secret(SecretParameter::new("s", "S")),
                )),
                ParameterKind::Expirable,
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
    fn children_returns_none_for_scalars() {
        let def = ParameterDef::Text(TextParameter::new("x", "X"));
        assert!(def.children().is_none());
    }

    #[test]
    fn children_returns_object_fields() {
        let def = ParameterDef::Object(
            ObjectParameter::new("conn", "Conn")
                .with_field(ParameterDef::Text(TextParameter::new("host", "Host")))
                .with_field(ParameterDef::Number(NumberParameter::new("port", "Port"))),
        );

        let children = def.children().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].key(), "host");
        assert_eq!(children[1].key(), "port");
    }

    #[test]
    fn children_returns_list_template() {
        let def = ParameterDef::List(ListParameter::new(
            "items",
            "Items",
            ParameterDef::Text(TextParameter::new("item", "Item")),
        ));

        let children = def.children().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].key(), "item");
    }

    #[test]
    fn children_returns_mode_variant_params() {
        let mut mode = ModeParameter::new("auth", "Auth");
        mode.variants.push(
            ModeVariant::new("key", "Key")
                .with_parameter(ParameterDef::Secret(SecretParameter::new("api_key", "Key"))),
        );
        mode.variants.push(
            ModeVariant::new("oauth", "OAuth")
                .with_parameter(ParameterDef::Text(TextParameter::new("client", "Client")))
                .with_parameter(ParameterDef::Secret(SecretParameter::new(
                    "secret",
                    "Secret",
                ))),
        );
        let def = ParameterDef::Mode(mode);

        let children = def.children().unwrap();
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].key(), "api_key");
        assert_eq!(children[1].key(), "client");
        assert_eq!(children[2].key(), "secret");
    }

    #[test]
    fn children_returns_group_params() {
        let def = ParameterDef::Group(
            GroupParameter::new("adv", "Advanced")
                .with_parameter(ParameterDef::Number(NumberParameter::new(
                    "timeout",
                    "Timeout",
                ))),
        );

        let children = def.children().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].key(), "timeout");
    }

    #[test]
    fn children_returns_expirable_inner() {
        let def = ParameterDef::Expirable(ExpirableParameter::new(
            "token",
            "Token",
            ParameterDef::Secret(SecretParameter::new("val", "Value")),
        ));

        let children = def.children().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].key(), "val");
    }

    #[test]
    fn serde_round_trip_object() {
        let def = ParameterDef::Object(
            ObjectParameter::new("db", "Database")
                .with_field(ParameterDef::Text(TextParameter::new("host", "Host")))
                .with_field(ParameterDef::Number(NumberParameter::new("port", "Port"))),
        );

        let json_str = serde_json::to_string(&def).unwrap();
        assert!(json_str.contains("\"type\":\"object\""));

        let deserialized: ParameterDef = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.key(), "db");
        assert_eq!(deserialized.kind(), ParameterKind::Object);
        assert_eq!(deserialized.children().unwrap().len(), 2);
    }

    #[test]
    fn serde_round_trip_list() {
        let def = ParameterDef::List(ListParameter::new(
            "emails",
            "Emails",
            ParameterDef::Text(TextParameter::new("email", "Email")),
        ));

        let json_str = serde_json::to_string(&def).unwrap();
        assert!(json_str.contains("\"type\":\"list\""));

        let deserialized: ParameterDef = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.key(), "emails");
        assert_eq!(deserialized.kind(), ParameterKind::List);
    }

    #[test]
    fn serde_round_trip_mode() {
        let mut mode = ModeParameter::new("auth", "Auth");
        mode.variants.push(
            ModeVariant::new("api_key", "API Key")
                .with_parameter(ParameterDef::Secret(SecretParameter::new("key", "Key"))),
        );
        let def = ParameterDef::Mode(mode);

        let json_str = serde_json::to_string(&def).unwrap();
        assert!(json_str.contains("\"type\":\"mode\""));

        let deserialized: ParameterDef = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.key(), "auth");
        assert_eq!(deserialized.kind(), ParameterKind::Mode);
    }

    #[test]
    fn recursive_nesting_object_in_list() {
        let inner_obj = ObjectParameter::new("header", "Header")
            .with_field(ParameterDef::Text(TextParameter::new("name", "Name")))
            .with_field(ParameterDef::Text(TextParameter::new("value", "Value")));

        let def = ParameterDef::List(ListParameter::new(
            "headers",
            "Headers",
            ParameterDef::Object(inner_obj),
        ));

        let json_str = serde_json::to_string(&def).unwrap();
        let deserialized: ParameterDef = serde_json::from_str(&json_str).unwrap();

        let template = &deserialized.children().unwrap()[0];
        assert_eq!(template.kind(), ParameterKind::Object);
        assert_eq!(template.children().unwrap().len(), 2);
    }

    #[test]
    fn recursive_nesting_list_in_object() {
        let inner_list = ListParameter::new(
            "tags",
            "Tags",
            ParameterDef::Text(TextParameter::new("tag", "Tag")),
        );

        let def = ParameterDef::Object(
            ObjectParameter::new("item", "Item")
                .with_field(ParameterDef::Text(TextParameter::new("name", "Name")))
                .with_field(ParameterDef::List(inner_list)),
        );

        let json_str = serde_json::to_string(&def).unwrap();
        let deserialized: ParameterDef = serde_json::from_str(&json_str).unwrap();

        let children = deserialized.children().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[1].kind(), ParameterKind::List);
    }

    #[test]
    fn three_level_deep_nesting_serde() {
        // List -> Object -> List -> Text (3 levels deep)
        let inner_list = ListParameter::new(
            "values",
            "Values",
            ParameterDef::Text(TextParameter::new("val", "Value")),
        );
        let obj = ObjectParameter::new("entry", "Entry")
            .with_field(ParameterDef::List(inner_list));
        let def = ParameterDef::List(ListParameter::new(
            "entries",
            "Entries",
            ParameterDef::Object(obj),
        ));

        let json_str = serde_json::to_string(&def).unwrap();
        let deserialized: ParameterDef = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.key(), "entries");

        // Level 1: List template is an Object
        let level1 = &deserialized.children().unwrap()[0];
        assert_eq!(level1.kind(), ParameterKind::Object);

        // Level 2: Object field is a List
        let level2 = &level1.children().unwrap()[0];
        assert_eq!(level2.kind(), ParameterKind::List);

        // Level 3: List template is Text
        let level3 = &level2.children().unwrap()[0];
        assert_eq!(level3.kind(), ParameterKind::Text);
        assert_eq!(level3.key(), "val");
    }

    #[test]
    fn deserialize_object_from_json() {
        let json = json!({
            "type": "object",
            "key": "connection",
            "name": "Connection",
            "fields": [
                {"type": "text", "key": "host", "name": "Host"},
                {"type": "number", "key": "port", "name": "Port", "default": 5432.0}
            ]
        });

        let def: ParameterDef = serde_json::from_value(json).unwrap();
        assert_eq!(def.key(), "connection");
        assert_eq!(def.kind(), ParameterKind::Object);

        let children = def.children().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].key(), "host");
        assert_eq!(children[1].key(), "port");
    }

    #[test]
    fn deserialize_mode_from_json() {
        let json = json!({
            "type": "mode",
            "key": "auth",
            "name": "Authentication",
            "default_variant": "api_key",
            "variants": [
                {
                    "key": "api_key",
                    "name": "API Key",
                    "parameters": [
                        {"type": "secret", "key": "key", "name": "API Key"}
                    ]
                },
                {
                    "key": "oauth",
                    "name": "OAuth",
                    "parameters": [
                        {"type": "text", "key": "client_id", "name": "Client ID"},
                        {"type": "secret", "key": "client_secret", "name": "Client Secret"}
                    ]
                }
            ]
        });

        let def: ParameterDef = serde_json::from_value(json).unwrap();
        assert_eq!(def.key(), "auth");
        assert_eq!(def.kind(), ParameterKind::Mode);

        let children = def.children().unwrap();
        assert_eq!(children.len(), 3);
    }
}
