// guard-justified: Schema maintainers own this throwaway, test-only authoring
// prototype. It does not establish a production API, wire contract, or migration.
// guard-justified: The array item key is an adapter detail required by legacy
// Field. Union and default semantics remain explicit promotion blockers.

use nebula_validator::{RuleRef, RuleView, ValueRule};
use serde::Serialize;
use serde_json::{Number, Value};

use crate::{
    Field, FieldKey, RequiredMode, Rule, ScalarSchema, ValidSchema, ValidationError,
    ValidationReport, ValuePath,
};

const ARRAY_ITEM_KEY: &str = "_prototype_item";

// Retains CURRENT RequiredMode semantics: required rejects missing, null,
// empty strings, and empty arrays. Optional permits omission but still checks
// the type of a present null. This is not orthogonal presence/nullability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Requirement {
    Required,
    Optional,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
struct Presentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
struct Property {
    key: FieldKey,
    value: ValueDefinition,
    requirement: Requirement,
    #[serde(skip)]
    presentation: Presentation,
}

// Property equality is semantic equality. Presentation has its own serialized
// occurrence stream and is deliberately excluded here.
impl PartialEq for Property {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.value == other.value && self.requirement == other.requirement
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ValueDefinition {
    String {
        rules: Vec<Rule>,
    },
    Boolean {
        rules: Vec<Rule>,
    },
    Number(NumberDefinition),
    Record {
        properties: Vec<Property>,
        rules: Vec<Rule>,
    },
    Array(ArrayDefinition),
    // Explicitly represented so unsupported union semantics cannot disappear.
    Union,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct NumberDefinition {
    integer: bool,
    minimum: Number,
    maximum: Number,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ArrayDefinition {
    item: Box<ValueDefinition>,
    min_items: Option<u32>,
    max_items: Option<u32>,
    unique: bool,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, Copy)]
enum DefinitionKind {
    String,
    Boolean,
    Number { integer: bool },
    Record,
    Array,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct OccurrenceAnnotation {
    path: ValuePath,
    presentation: Presentation,
}

#[derive(Debug)]
struct LoweredSchema {
    schema: ValidSchema,
    annotations: Vec<OccurrenceAnnotation>,
}

struct Schema;

impl Schema {
    fn builder() -> SchemaBuilder {
        SchemaBuilder {
            properties: Vec::new(),
        }
    }
}

#[must_use]
struct SchemaBuilder {
    properties: Vec<Property>,
}

impl SchemaBuilder {
    fn property(mut self, property: Property) -> Self {
        self.properties.push(property);
        self
    }

    #[tracing::instrument(name = "schema.property_prototype.build", skip_all, err)]
    fn build(self) -> Result<LoweredSchema, ValidationReport> {
        let mut builder = crate::Schema::builder();
        let mut annotations = Vec::new();
        for property in self.properties {
            let path = ValuePath::single(property.key.as_str());
            let field = lower_value(
                &property.key,
                &property.value,
                property.requirement,
                &path,
                &mut annotations,
            )?;
            builder = builder.add(field);
            annotations.push(OccurrenceAnnotation {
                path,
                presentation: property.presentation,
            });
        }
        Ok(LoweredSchema {
            schema: builder.build()?,
            annotations,
        })
    }
}

fn lower_value(
    key: &FieldKey,
    definition: &ValueDefinition,
    requirement: Requirement,
    path: &ValuePath,
    annotations: &mut Vec<OccurrenceAnnotation>,
) -> Result<Field, ValidationReport> {
    match definition {
        ValueDefinition::String { rules } => {
            check_rules(rules, DefinitionKind::String, path)?;
            let mut field = Field::string(key.clone()).no_expression();
            field.required = required_mode(requirement);
            field.rules.clone_from(rules);
            Ok(field.into())
        },
        ValueDefinition::Boolean { rules } => {
            check_rules(rules, DefinitionKind::Boolean, path)?;
            let mut field = Field::boolean(key.clone()).no_expression();
            field.required = required_mode(requirement);
            field.rules.clone_from(rules);
            Ok(field.into())
        },
        ValueDefinition::Number(number) => lower_number(key, number, requirement, path),
        ValueDefinition::Record { properties, rules } => {
            check_rules(rules, DefinitionKind::Record, path)?;
            let mut field = Field::object(key.clone()).no_expression();
            field.required = required_mode(requirement);
            field.rules.clone_from(rules);
            for property in properties {
                let child_path = path.push(property.key.as_str());
                let child = lower_value(
                    &property.key,
                    &property.value,
                    property.requirement,
                    &child_path,
                    annotations,
                )?;
                field = field.add(child);
                annotations.push(OccurrenceAnnotation {
                    path: child_path,
                    presentation: property.presentation.clone(),
                });
            }
            Ok(field.into())
        },
        ValueDefinition::Array(array) => lower_array(key, array, requirement, path, annotations),
        ValueDefinition::Union => Err(unsupported("property.union", path)),
    }
}

fn lower_number(
    key: &FieldKey,
    definition: &NumberDefinition,
    requirement: Requirement,
    path: &ValuePath,
) -> Result<Field, ValidationReport> {
    check_rules(
        &definition.rules,
        DefinitionKind::Number {
            integer: definition.integer,
        },
        path,
    )?;
    let bounds = if definition.integer {
        ScalarSchema::integer(definition.minimum.clone(), definition.maximum.clone())
    } else {
        ScalarSchema::number(definition.minimum.clone(), definition.maximum.clone())
    };
    if bounds.is_err() {
        return Err(unsupported("property.number_bounds", path));
    }

    let mut field = Field::number(key.clone())
        .no_expression()
        .min(definition.minimum.clone())
        .max(definition.maximum.clone());
    if definition.integer {
        field = field.integer();
    }
    field.required = required_mode(requirement);
    field.rules.extend(definition.rules.iter().cloned());
    Ok(field.into())
}

fn lower_array(
    key: &FieldKey,
    definition: &ArrayDefinition,
    requirement: Requirement,
    path: &ValuePath,
    annotations: &mut Vec<OccurrenceAnnotation>,
) -> Result<Field, ValidationReport> {
    check_rules(&definition.rules, DefinitionKind::Array, path)?;
    if definition
        .min_items
        .zip(definition.max_items)
        .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        return Err(unsupported("property.array_bounds", path));
    }
    let item_key =
        FieldKey::new(ARRAY_ITEM_KEY).map_err(|_| unsupported("property.array_adapter", path))?;
    let item = lower_value(
        &item_key,
        &definition.item,
        Requirement::Optional,
        path,
        annotations,
    )?;
    let mut field = Field::list(key.clone()).no_expression().item(item);
    if let Some(minimum) = definition.min_items {
        field = field.min_items(minimum);
    }
    if let Some(maximum) = definition.max_items {
        field = field.max_items(maximum);
    }
    if definition.unique {
        field = field.unique();
    }
    field.required = required_mode(requirement);
    field.rules.clone_from(&definition.rules);
    Ok(field.into())
}

const fn required_mode(requirement: Requirement) -> RequiredMode {
    match requirement {
        Requirement::Required => RequiredMode::Always,
        Requirement::Optional => RequiredMode::Never,
    }
}

fn check_rules(
    rules: &[Rule],
    kind: DefinitionKind,
    path: &ValuePath,
) -> Result<(), ValidationReport> {
    check_context_free_rules(rules, path)?;
    check_compatible_rules(rules, kind, path)
}

fn check_context_free_rules(rules: &[Rule], path: &ValuePath) -> Result<(), ValidationReport> {
    for rule in rules {
        let mut pending = vec![rule.root()];
        while let Some(current) = pending.pop() {
            match current.view() {
                RuleView::Value(_) => {},
                RuleView::All(children) | RuleView::Any(children) => pending.extend(children),
                RuleView::Not(inner) | RuleView::Described { inner, .. } => pending.push(inner),
                RuleView::Predicate(_) => {
                    return Err(unsupported("property.rule.contextual", path));
                },
                RuleView::Deferred(_) => {
                    return Err(unsupported("property.rule.deferred", path));
                },
                _ => return Err(unsupported_rule_kind(current, path)),
            }
        }
    }
    Ok(())
}

fn check_compatible_rules(
    rules: &[Rule],
    kind: DefinitionKind,
    path: &ValuePath,
) -> Result<(), ValidationReport> {
    for rule in rules {
        let mut pending = vec![rule.root()];
        while let Some(current) = pending.pop() {
            match current.view() {
                RuleView::Value(value) if value_rule_is_compatible(value, kind) => {},
                RuleView::Value(_) => {
                    return Err(unsupported("property.rule.incompatible", path));
                },
                RuleView::All(children) | RuleView::Any(children) => pending.extend(children),
                RuleView::Not(inner) | RuleView::Described { inner, .. } => pending.push(inner),
                RuleView::Predicate(_) | RuleView::Deferred(_) => {
                    return Err(unsupported("property.rule.unsupported", path));
                },
                _ => return Err(unsupported_rule_kind(current, path)),
            }
        }
    }
    Ok(())
}

fn value_rule_is_compatible(rule: &ValueRule, kind: DefinitionKind) -> bool {
    match (kind, rule) {
        (
            DefinitionKind::String,
            ValueRule::MinLength(_)
            | ValueRule::MaxLength(_)
            | ValueRule::Pattern(_)
            | ValueRule::Email
            | ValueRule::Url,
        ) => true,
        (DefinitionKind::String, ValueRule::OneOf(values)) => values.iter().all(Value::is_string),
        (DefinitionKind::Boolean, ValueRule::OneOf(values)) => values.iter().all(Value::is_boolean),
        (
            DefinitionKind::Number { .. },
            ValueRule::Min(_)
            | ValueRule::Max(_)
            | ValueRule::GreaterThan(_)
            | ValueRule::LessThan(_),
        ) => true,
        (DefinitionKind::Number { integer }, ValueRule::OneOf(values)) => {
            values.iter().all(|value| {
                value.as_number().is_some_and(|number| {
                    !integer
                        || number.is_i64()
                        || number.is_u64()
                        || number.as_f64().is_some_and(|value| value.fract() == 0.0)
                })
            })
        },
        (DefinitionKind::Record, ValueRule::OneOf(values)) => values.iter().all(Value::is_object),
        (DefinitionKind::Array, ValueRule::MinItems(_) | ValueRule::MaxItems(_)) => true,
        (DefinitionKind::Array, ValueRule::OneOf(values)) => values.iter().all(Value::is_array),
        _ => false,
    }
}

fn unsupported_rule_kind(_rule: RuleRef<'_>, path: &ValuePath) -> ValidationReport {
    unsupported("property.rule.unsupported", path)
}

fn unsupported(code: &'static str, path: &ValuePath) -> ValidationReport {
    ValidationError::builder(code)
        .at(path.clone())
        .message("definition is outside the finite property prototype")
        .build()
        .into()
}

#[path = "property_prototype/tests.rs"]
mod tests;
