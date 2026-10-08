//! Reader for the parameter schema an action publishes, in the `nebula-schema` wire format
//! (`{"fields": [{"type": "string", "key": ...}, ...]}`). The client reads the wire format instead of
//! linking `nebula-schema`, which does not build for the browser and would carry its validator into the
//! bundle. The reader is tolerant: an unknown field type, widget or rule keeps the field editable as raw
//! JSON instead of failing the form, matching the schema crate's own forward-compatibility contract.
//!
//! Nothing here touches egui, so parsing, visibility and the inline checks are unit-tested.

use serde_json::{Map, Value};

/// The parameters of one action.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Form {
    pub(crate) fields: Vec<Field>,
}

/// One declared input.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Field {
    pub(crate) key: String,
    pub(crate) label: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) placeholder: Option<String>,
    pub(crate) default: Option<Value>,
    pub(crate) visible: Condition,
    pub(crate) required: Condition,
    pub(crate) expression: ExpressionMode,
    pub(crate) group: Option<String>,
    pub(crate) bounds: Bounds,
    pub(crate) kind: Kind,
}

/// When a field is shown or required.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Condition {
    Always,
    Never,
    /// A `nebula-validator` rule over sibling values, kept as its wire JSON.
    When(Value),
}

/// Whether a field may hold an expression instead of a fixed value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExpressionMode {
    Forbidden,
    #[default]
    Allowed,
    /// The field only takes an expression, as a computed field does.
    Required,
}

/// Limits taken from the field's value rules, used for controls and inline checks.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Bounds {
    pub(crate) min: Option<f64>,
    pub(crate) max: Option<f64>,
    pub(crate) min_length: Option<u64>,
    pub(crate) max_length: Option<u64>,
    pub(crate) email: bool,
    pub(crate) url: bool,
}

/// Semantic kind of a string input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Hint {
    #[default]
    Text,
    Email,
    Url,
    Password,
    Phone,
    Ip,
    Regex,
    Markdown,
    Cron,
    Date,
    DateTime,
    Time,
    Color,
    Duration,
    Uuid,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum NumberWidget {
    #[default]
    Plain,
    Slider,
    Stepper,
    Percent,
    Currency,
    Duration,
    Bytes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BooleanWidget {
    #[default]
    Toggle,
    Checkbox,
    Radio,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SelectWidget {
    #[default]
    Dropdown,
    Radio,
    Checkboxes,
    Combobox,
    Tags,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ObjectWidget {
    #[default]
    Inline,
    Collapsed,
    PickFields,
    Sections,
    Tabs,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ListWidget {
    #[default]
    Plain,
    Sortable,
    Tags,
    KeyValue,
    Accordion,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Severity {
    #[default]
    Info,
    Warning,
    Danger,
    Success,
}

/// One option of a select field.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Choice {
    pub(crate) value: Value,
    pub(crate) label: String,
    pub(crate) description: Option<String>,
    pub(crate) disabled: bool,
}

/// One variant of a mode field: its key, label and payload field.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Variant {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) field: Field,
}

/// The type-specific part of a field.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Text {
        hint: Hint,
        multiline: bool,
    },
    Secret {
        multiline: bool,
    },
    Number {
        integer: bool,
        widget: NumberWidget,
        step: Option<f64>,
    },
    Boolean {
        widget: BooleanWidget,
    },
    Select {
        options: Vec<Choice>,
        multiple: bool,
        allow_custom: bool,
        searchable: bool,
        widget: SelectWidget,
        /// Options come from a server loader the client cannot call yet.
        loader: Option<String>,
    },
    Object {
        fields: Vec<Field>,
        widget: ObjectWidget,
    },
    List {
        item: Option<Box<Field>>,
        min_items: Option<u64>,
        max_items: Option<u64>,
        unique: bool,
        widget: ListWidget,
    },
    Mode {
        variants: Vec<Variant>,
        default_variant: Option<String>,
    },
    Code {
        language: String,
        simple: bool,
    },
    File {
        accept: Option<String>,
        max_size: Option<u64>,
        multiple: bool,
    },
    Computed {
        returns: String,
    },
    Dynamic {
        loader: Option<String>,
    },
    Notice {
        severity: Severity,
    },
    /// A field type newer than this client. It is edited as raw JSON.
    Unknown {
        type_name: String,
    },
}

impl Form {
    /// Reads `{"fields": [...]}`. Entries that are not objects with a string `key` are skipped, since
    /// they cannot be bound to a parameter.
    pub(crate) fn parse(schema: &Value) -> Self {
        Self {
            fields: fields_of(&schema["fields"]),
        }
    }
}

fn fields_of(value: &Value) -> Vec<Field> {
    value
        .as_array()
        .map(|fields| fields.iter().filter_map(Field::parse).collect())
        .unwrap_or_default()
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

fn word<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

impl Field {
    fn parse(value: &Value) -> Option<Self> {
        let key = text(&value["key"])?;
        let type_name = word(value, "type").to_owned();
        let kind = Kind::parse(&type_name, value);
        // A computed field only ever holds an expression; the others follow the declared mode.
        let expression = match word(value, "expression") {
            "forbidden" => ExpressionMode::Forbidden,
            "required" => ExpressionMode::Required,
            _ if matches!(kind, Kind::Computed { .. }) => ExpressionMode::Required,
            _ if matches!(
                kind,
                Kind::Boolean { .. } | Kind::Select { .. } | Kind::Notice { .. }
            ) =>
            {
                ExpressionMode::Forbidden
            },
            _ => ExpressionMode::Allowed,
        };
        Some(Self {
            key,
            label: text(&value["label"]),
            description: text(&value["description"]),
            placeholder: text(&value["placeholder"]),
            default: value
                .get("default")
                .filter(|default| !default.is_null())
                .cloned(),
            visible: Condition::parse(&value["visible"], Condition::Always),
            required: Condition::parse(&value["required"], Condition::Never),
            expression,
            group: text(&value["group"]),
            bounds: Bounds::parse(&value["rules"]),
            kind,
        })
    }

    /// The label a person reads: the declared label, or the key when there is none.
    pub(crate) fn title(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.key)
    }

    /// The value the field starts from when the node does not set it.
    pub(crate) fn initial(&self) -> Value {
        if let Some(default) = &self.default {
            return default.clone();
        }
        match &self.kind {
            Kind::Number { .. } => Value::from(0),
            Kind::Boolean { .. } => Value::Bool(false),
            Kind::Select { multiple: true, .. } | Kind::List { .. } => Value::Array(Vec::new()),
            Kind::Select { options, .. } => options
                .first()
                .map_or(Value::Null, |choice| choice.value.clone()),
            Kind::Object { fields, .. } => Value::Object(
                fields
                    .iter()
                    .filter(|field| !matches!(field.kind, Kind::Notice { .. }))
                    .map(|field| (field.key.clone(), field.initial()))
                    .collect(),
            ),
            Kind::Mode {
                variants,
                default_variant,
            } => {
                let chosen = default_variant
                    .as_deref()
                    .and_then(|key| variants.iter().find(|variant| variant.key == key))
                    .or_else(|| variants.first());
                chosen.map_or(Value::Null, |variant| {
                    serde_json::json!({"mode": variant.key, "value": variant.field.initial()})
                })
            },
            Kind::Text { .. } | Kind::Secret { .. } | Kind::Code { .. } => Value::from(""),
            _ => Value::Null,
        }
    }

    /// Whether the field takes part in the form given its siblings' values. An unreadable condition
    /// shows the field, so a newer rule never hides an input a person needs.
    pub(crate) fn is_visible(&self, siblings: &Map<String, Value>) -> bool {
        match &self.visible {
            Condition::Always => true,
            Condition::Never => false,
            Condition::When(rule) => holds(rule, siblings).unwrap_or(true),
        }
    }

    pub(crate) fn is_required(&self, siblings: &Map<String, Value>) -> bool {
        match &self.required {
            Condition::Always => true,
            Condition::Never => false,
            Condition::When(rule) => holds(rule, siblings).unwrap_or(false),
        }
    }

    /// What is wrong with a fixed value, in words, for the line under the input. The server validates
    /// again on publish; this only catches what a person can fix while typing.
    pub(crate) fn problem(&self, value: &Value, required: bool) -> Option<String> {
        if required && is_empty(value) {
            return Some("Required.".to_owned());
        }
        if is_empty(value) {
            return None;
        }
        let bounds = &self.bounds;
        if let Some(number) = value.as_f64() {
            if let Kind::Number { integer: true, .. } = self.kind
                && number.fract() != 0.0
            {
                return Some("Use a whole number.".to_owned());
            }
            if let Some(min) = bounds.min
                && number < min
            {
                return Some(format!("At least {}.", trim_number(min)));
            }
            if let Some(max) = bounds.max
                && number > max
            {
                return Some(format!("At most {}.", trim_number(max)));
            }
        }
        if let Some(text) = value.as_str() {
            let length = text.chars().count() as u64;
            if let Some(min) = bounds.min_length
                && length < min
            {
                return Some(format!("At least {min} characters."));
            }
            if let Some(max) = bounds.max_length
                && length > max
            {
                return Some(format!("At most {max} characters."));
            }
            let hint = match self.kind {
                Kind::Text { hint, .. } => hint,
                _ => Hint::Text,
            };
            if (bounds.email || hint == Hint::Email) && !looks_like_email(text) {
                return Some("Enter an email address.".to_owned());
            }
            if (bounds.url || hint == Hint::Url) && !looks_like_url(text) {
                return Some("Enter a URL starting with http:// or https://.".to_owned());
            }
            if let Some(format) = date_format(hint)
                && !matches_digits(text, format)
            {
                return Some(format!("Use the format {format}."));
            }
        }
        if let (
            Some(items),
            Kind::List {
                min_items,
                max_items,
                ..
            },
        ) = (value.as_array(), &self.kind)
        {
            let count = items.len() as u64;
            if let Some(min) = min_items
                && count < *min
            {
                return Some(format!("Add at least {min} items."));
            }
            if let Some(max) = max_items
                && count > *max
            {
                return Some(format!("Keep at most {max} items."));
            }
        }
        None
    }
}

/// A placeholder that shows the expected shape, for hints with a fixed format.
pub(crate) const fn hint_placeholder(hint: Hint) -> Option<&'static str> {
    match hint {
        Hint::Email => Some("name@example.com"),
        Hint::Url => Some("https://"),
        Hint::Phone => Some("+1 555 0100"),
        Hint::Ip => Some("192.0.2.1"),
        Hint::Cron => Some("*/5 * * * *"),
        Hint::Date => Some("YYYY-MM-DD"),
        Hint::DateTime => Some("YYYY-MM-DDTHH:MM:SS"),
        Hint::Time => Some("HH:MM:SS"),
        Hint::Duration => Some("PT1H30M"),
        Hint::Uuid => Some("00000000-0000-0000-0000-000000000000"),
        Hint::Color => Some("#6366F1"),
        Hint::Text | Hint::Password | Hint::Regex | Hint::Markdown => None,
    }
}

const fn date_format(hint: Hint) -> Option<&'static str> {
    match hint {
        Hint::Date => Some("YYYY-MM-DD"),
        Hint::Time => Some("HH:MM:SS"),
        _ => None,
    }
}

/// `2026-10-08` against `YYYY-MM-DD`: letters stand for digits, everything else must match.
fn matches_digits(text: &str, format: &str) -> bool {
    text.len() == format.len()
        && text.chars().zip(format.chars()).all(|(actual, expected)| {
            if expected.is_ascii_alphabetic() {
                actual.is_ascii_digit()
            } else {
                actual == expected
            }
        })
}

fn looks_like_email(text: &str) -> bool {
    text.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
    })
}

fn looks_like_url(text: &str) -> bool {
    ["http://", "https://"].iter().any(|scheme| {
        text.strip_prefix(scheme)
            .is_some_and(|rest| !rest.is_empty())
    })
}

fn trim_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{number:.0}")
    } else {
        number.to_string()
    }
}

/// Null, an empty string and an empty list count as missing, as the schema's `required` does.
pub(crate) fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Bool(_) | Value::Number(_) | Value::Object(_) => false,
    }
}

impl Condition {
    fn parse(value: &Value, absent: Self) -> Self {
        match word(value, "kind") {
            "always" => Self::Always,
            "never" => Self::Never,
            "when" => {
                // The rule is flattened next to `kind`, so everything else is the rule.
                let mut rule = value.as_object().cloned().unwrap_or_default();
                rule.remove("kind");
                Self::When(Value::Object(rule))
            },
            _ => absent,
        }
    }
}

impl Bounds {
    fn parse(rules: &Value) -> Self {
        let mut bounds = Self::default();
        for rule in rules.as_array().into_iter().flatten() {
            let Some((name, argument)) = rule.as_object().and_then(|rule| rule.iter().next())
            else {
                continue;
            };
            match name.as_str() {
                "min" => bounds.min = argument.as_f64(),
                "max" => bounds.max = argument.as_f64(),
                "min_length" => bounds.min_length = argument.as_u64(),
                "max_length" => bounds.max_length = argument.as_u64(),
                "email" => bounds.email = true,
                "url" => bounds.url = true,
                _ => {},
            }
        }
        bounds
    }
}

impl Kind {
    fn parse(type_name: &str, value: &Value) -> Self {
        match type_name {
            "string" => Self::Text {
                hint: hint(word(value, "hint")),
                multiline: word(value, "widget") == "multiline",
            },
            "secret" => Self::Secret {
                multiline: word(value, "widget") == "multiline",
            },
            "number" => Self::Number {
                integer: value["integer"].as_bool().unwrap_or(false),
                widget: match word(value, "widget") {
                    "slider" => NumberWidget::Slider,
                    "stepper" => NumberWidget::Stepper,
                    "percent" => NumberWidget::Percent,
                    "currency" => NumberWidget::Currency,
                    "duration" => NumberWidget::Duration,
                    "bytes" => NumberWidget::Bytes,
                    _ => NumberWidget::Plain,
                },
                step: value["step"].as_f64(),
            },
            "boolean" => Self::Boolean {
                widget: match word(value, "widget") {
                    "checkbox" => BooleanWidget::Checkbox,
                    "radio" => BooleanWidget::Radio,
                    _ => BooleanWidget::Toggle,
                },
            },
            "select" => Self::Select {
                options: value["options"]
                    .as_array()
                    .map(|options| options.iter().filter_map(Choice::parse).collect())
                    .unwrap_or_default(),
                multiple: value["multiple"].as_bool().unwrap_or(false),
                allow_custom: value["allow_custom"].as_bool().unwrap_or(false),
                searchable: value["searchable"].as_bool().unwrap_or(false),
                widget: match word(value, "widget") {
                    "radio" => SelectWidget::Radio,
                    "checkboxes" => SelectWidget::Checkboxes,
                    "combobox" => SelectWidget::Combobox,
                    "tags" => SelectWidget::Tags,
                    _ => SelectWidget::Dropdown,
                },
                loader: text(&value["loader"]),
            },
            "object" => Self::Object {
                fields: fields_of(&value["fields"]),
                widget: match word(value, "widget") {
                    "collapsed" => ObjectWidget::Collapsed,
                    "pick_fields" => ObjectWidget::PickFields,
                    "sections" => ObjectWidget::Sections,
                    "tabs" => ObjectWidget::Tabs,
                    _ => ObjectWidget::Inline,
                },
            },
            "list" => Self::List {
                item: Field::parse(&value["item"]).map(Box::new),
                min_items: value["min_items"].as_u64(),
                max_items: value["max_items"].as_u64(),
                unique: value["unique"].as_bool().unwrap_or(false),
                widget: match word(value, "widget") {
                    "sortable" => ListWidget::Sortable,
                    "tags" => ListWidget::Tags,
                    "key_value" => ListWidget::KeyValue,
                    "accordion" => ListWidget::Accordion,
                    _ => ListWidget::Plain,
                },
            },
            "mode" => Self::Mode {
                variants: value["variants"]
                    .as_array()
                    .map(|variants| variants.iter().filter_map(Variant::parse).collect())
                    .unwrap_or_default(),
                default_variant: text(&value["default_variant"]),
            },
            "code" => Self::Code {
                language: text(&value["language"]).unwrap_or_else(|| "plaintext".to_owned()),
                simple: word(value, "widget") == "simple",
            },
            "file" => Self::File {
                accept: text(&value["accept"]),
                max_size: value["max_size"].as_u64(),
                multiple: value["multiple"].as_bool().unwrap_or(false),
            },
            "computed" => Self::Computed {
                returns: text(&value["returns"]).unwrap_or_else(|| "string".to_owned()),
            },
            "dynamic" => Self::Dynamic {
                loader: text(&value["loader"]),
            },
            "notice" => Self::Notice {
                severity: match word(value, "severity") {
                    "warning" => Severity::Warning,
                    "danger" => Severity::Danger,
                    "success" => Severity::Success,
                    _ => Severity::Info,
                },
            },
            other => Self::Unknown {
                type_name: other.to_owned(),
            },
        }
    }
}

fn hint(name: &str) -> Hint {
    match name {
        "email" => Hint::Email,
        "url" => Hint::Url,
        "password" => Hint::Password,
        "phone" => Hint::Phone,
        "ip" => Hint::Ip,
        "regex" => Hint::Regex,
        "markdown" => Hint::Markdown,
        "cron" => Hint::Cron,
        "date" => Hint::Date,
        "date_time" => Hint::DateTime,
        "time" => Hint::Time,
        "color" => Hint::Color,
        "duration" => Hint::Duration,
        "uuid" => Hint::Uuid,
        _ => Hint::Text,
    }
}

impl Choice {
    fn parse(value: &Value) -> Option<Self> {
        let choice = value.get("value")?.clone();
        Some(Self {
            label: text(&value["label"]).unwrap_or_else(|| display(&choice)),
            value: choice,
            description: text(&value["description"]),
            disabled: value["disabled"].as_bool().unwrap_or(false),
        })
    }
}

impl Variant {
    fn parse(value: &Value) -> Option<Self> {
        let key = text(&value["key"])?;
        Some(Self {
            label: text(&value["label"]).unwrap_or_else(|| key.clone()),
            field: Field::parse(&value["field"])?,
            key,
        })
    }
}

/// A value as a person reads it: strings without quotes, everything else as JSON.
pub(crate) fn display(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

/// Evaluates a `nebula-validator` rule against sibling values. `None` means the rule uses something
/// this client cannot judge, such as a regular expression, or a sibling that holds an expression.
pub(crate) fn holds(rule: &Value, siblings: &Map<String, Value>) -> Option<bool> {
    let (name, argument) = rule.as_object()?.iter().next()?;
    let at = |path: &Value| path.as_str().and_then(|path| lookup(siblings, path));
    let pair = || {
        let pair = argument.as_array()?;
        Some((pair.first()?, pair.get(1)?))
    };
    let compare = |order: fn(f64, f64) -> bool| -> Option<bool> {
        let (path, bound) = pair()?;
        Some(
            at(path)?
                .as_f64()
                .is_some_and(|value| order(value, bound.as_f64().unwrap_or(f64::NAN))),
        )
    };
    match name.as_str() {
        "eq" => {
            let (path, expected) = pair()?;
            Some(at(path).unwrap_or(&Value::Null) == expected)
        },
        "ne" => {
            let (path, expected) = pair()?;
            Some(at(path).unwrap_or(&Value::Null) != expected)
        },
        "gt" => compare(|value, bound| value > bound),
        "gte" => compare(|value, bound| value >= bound),
        "lt" => compare(|value, bound| value < bound),
        "lte" => compare(|value, bound| value <= bound),
        "is_true" => Some(at(argument) == Some(&Value::Bool(true))),
        "is_false" => Some(at(argument) == Some(&Value::Bool(false))),
        "set" => Some(at(argument).is_some_and(|value| !is_empty(value))),
        "empty" => Some(at(argument).is_none_or(is_empty)),
        "contains" => {
            let (path, needle) = pair()?;
            Some(match at(path) {
                Some(Value::Array(items)) => items.contains(needle),
                Some(Value::String(text)) => {
                    needle.as_str().is_some_and(|needle| text.contains(needle))
                },
                _ => false,
            })
        },
        "in" => {
            let (path, allowed) = pair()?;
            let value = at(path).unwrap_or(&Value::Null);
            Some(
                allowed
                    .as_array()
                    .is_some_and(|allowed| allowed.contains(value)),
            )
        },
        "all" => {
            let mut all = true;
            for child in argument.as_array()? {
                all &= holds(child, siblings)?;
            }
            Some(all)
        },
        "any" => {
            let mut any = false;
            for child in argument.as_array()? {
                any |= holds(child, siblings)?;
            }
            Some(any)
        },
        "not" => holds(argument, siblings).map(|inner| !inner),
        "described" => holds(argument.as_array()?.first()?, siblings),
        _ => None,
    }
}

/// Paths travel as JSON Pointers (`/auth/kind`), relative to the sibling values.
fn lookup<'a>(siblings: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut parts = path
        .strip_prefix('/')?
        .split('/')
        .map(|part| part.replace("~1", "/").replace("~0", "~"));
    let mut current = siblings.get(&parts.next()?)?;
    for part in parts {
        current = match current {
            Value::Array(items) => items.get(part.parse::<usize>().ok()?)?,
            other => other.get(&part)?,
        };
    }
    Some(current)
}

#[cfg(test)]
mod tests;
