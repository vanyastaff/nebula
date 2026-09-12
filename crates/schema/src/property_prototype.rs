// guard-justified: Schema maintainers own this throwaway, test-only authoring
// prototype. It does not establish a production API, wire contract, or migration.
// guard-justified: Only finite data-only records, strings, and booleans are
// evidenced. Context rebasing, general rule composition, performance, and SDK
// integration are not proven; unsupported policies must fail explicitly.

use nebula_validator::{RuleView, ValueRule};

use crate::{
    ExpressionMode, Field, FieldKey, RequiredMode, RootShape, Rule, ScalarKind, ValidSchema,
    ValidationError, ValidationReport, ValuePath, VisibilityMode,
};

// Retains CURRENT RequiredMode semantics: required rejects missing, null, and
// empty strings. Optional permits omission but still type-checks present null.
// This is not evidence of orthogonal key-presence, nullability, or emptiness.
enum Requirement {
    Required,
    Optional,
}

struct Presentation {
    label: Option<String>,
    visibility: VisibilityMode,
}

struct Property {
    key: FieldKey,
    value: ValidSchema,
    requirement: Requirement,
    presentation: Presentation,
}

struct OccurrenceAnnotation {
    path: ValuePath,
    presentation: Presentation,
}

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
            check_visibility(&property.presentation.visibility, &path)?;
            let field = lower_value(
                &property.key,
                &property.value,
                &property.requirement,
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
    value: &ValidSchema,
    requirement: &Requirement,
    path: &ValuePath,
    annotations: &mut Vec<OccurrenceAnnotation>,
) -> Result<Field, ValidationReport> {
    let required = match requirement {
        Requirement::Required => RequiredMode::Always,
        Requirement::Optional => RequiredMode::Never,
    };
    match value.root_shape() {
        RootShape::Scalar(scalar) => match scalar.kind() {
            ScalarKind::String => {
                check_string_rules(scalar.root_rules(), path)?;
                let mut field = Field::string(key.clone()).no_expression();
                field.required = required;
                field.rules = scalar.root_rules().to_vec();
                Ok(field.into())
            },
            ScalarKind::Boolean => {
                if !scalar.root_rules().is_empty() {
                    return Err(unsupported("property.boolean_rules", path));
                }
                let mut field = Field::boolean(key.clone()).no_expression();
                field.required = required;
                Ok(field.into())
            },
            ScalarKind::Null | ScalarKind::Integer | ScalarKind::Number => {
                Err(unsupported("property.unsupported_shape", path))
            },
        },
        RootShape::Record(record) => {
            if !record.root_rules().is_empty() {
                return Err(unsupported("property.record_rules", path));
            }
            let mut field = Field::object(key.clone()).no_expression();
            field.required = required;
            field.fields = lower_fields(record.fields(), path, annotations)?;
            Ok(field.into())
        },
        RootShape::Any | RootShape::Union(_) => {
            Err(unsupported("property.unsupported_shape", path))
        },
    }
}

fn lower_fields(
    fields: &[Field],
    parent: &ValuePath,
    annotations: &mut Vec<OccurrenceAnnotation>,
) -> Result<Vec<Field>, ValidationReport> {
    fields
        .iter()
        .map(|field| {
            let path = parent.push(field.key().as_str());
            lower_field(field, &path, annotations)
        })
        .collect()
}

fn lower_field(
    source: &Field,
    path: &ValuePath,
    annotations: &mut Vec<OccurrenceAnnotation>,
) -> Result<Field, ValidationReport> {
    if !matches!(
        source,
        Field::String(_) | Field::Boolean(_) | Field::Object(_)
    ) {
        return Err(unsupported("property.unsupported_field", path));
    }
    if !source.read_aliases().is_empty() || source.emit_as().is_some() {
        return Err(unsupported("property.aliases", path));
    }
    if !source.transformers().is_empty() {
        return Err(unsupported("property.transforms", path));
    }
    if source.default().is_some() {
        return Err(unsupported("property.defaults", path));
    }
    if source.expression() != &ExpressionMode::Forbidden {
        return Err(unsupported("property.expressions", path));
    }
    if matches!(source.required(), RequiredMode::When(_)) {
        return Err(unsupported("property.presence", path));
    }
    check_visibility(source.visible(), path)?;

    // Compare against the supported builder shape before removing presentation.
    // A nondefault policy or type-specific option cannot disappear unnoticed.
    let (runtime, label) = match source {
        Field::String(source) => {
            check_string_rules(&source.rules, path)?;
            let mut field = Field::string(source.key.clone()).no_expression();
            field.required = source.required.clone();
            field.rules.clone_from(&source.rules);
            field.label.clone_from(&source.label);
            field.visible = source.visible.clone();
            if &field != source {
                return Err(unsupported("property.field_metadata", path));
            }
            let label = field.label.take();
            field.visible = VisibilityMode::Always;
            (field.into(), label)
        },
        Field::Boolean(source) => {
            if !source.rules.is_empty() {
                return Err(unsupported("property.boolean_rules", path));
            }
            let mut field = Field::boolean(source.key.clone()).no_expression();
            field.required = source.required.clone();
            field.label.clone_from(&source.label);
            field.visible = source.visible.clone();
            if &field != source {
                return Err(unsupported("property.field_metadata", path));
            }
            let label = field.label.take();
            field.visible = VisibilityMode::Always;
            (field.into(), label)
        },
        Field::Object(source) => {
            if !source.rules.is_empty() {
                return Err(unsupported("property.record_rules", path));
            }
            let mut field = Field::object(source.key.clone()).no_expression();
            field.required = source.required.clone();
            field.label.clone_from(&source.label);
            field.visible = source.visible.clone();
            field.fields.clone_from(&source.fields);
            if &field != source {
                return Err(unsupported("property.field_metadata", path));
            }
            let label = field.label.take();
            field.visible = VisibilityMode::Always;
            field.fields = lower_fields(&source.fields, path, annotations)?;
            (field.into(), label)
        },
        _ => return Err(unsupported("property.unsupported_field", path)),
    };
    annotations.push(OccurrenceAnnotation {
        path: path.clone(),
        presentation: Presentation {
            label,
            visibility: source.visible().clone(),
        },
    });
    Ok(runtime)
}

fn check_visibility(visibility: &VisibilityMode, path: &ValuePath) -> Result<(), ValidationReport> {
    match visibility {
        VisibilityMode::Always | VisibilityMode::Never => Ok(()),
        VisibilityMode::When(_) => Err(unsupported("property.visibility", path)),
    }
}

fn check_string_rules(rules: &[Rule], path: &ValuePath) -> Result<(), ValidationReport> {
    for rule in rules {
        let supported = match rule.view() {
            RuleView::Value(
                ValueRule::MinLength(_)
                | ValueRule::MaxLength(_)
                | ValueRule::Pattern(_)
                | ValueRule::Email
                | ValueRule::Url,
            ) => true,
            RuleView::Value(ValueRule::OneOf(values)) => {
                values.iter().all(serde_json::Value::is_string)
            },
            _ => false,
        };
        if !supported {
            return Err(unsupported("property.string_rule", path));
        }
    }
    Ok(())
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
