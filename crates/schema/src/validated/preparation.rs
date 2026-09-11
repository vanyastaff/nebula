//! Consuming admission and preparation. No predicate observes raw input.

use indexmap::IndexMap;
use serde_json::Value;

use super::ValidSchema;
use crate::{
    AuthoredValue, CompiledValue, ExpressionMode, Field, ResolvedValue, ScalarValue, SecretValue,
    ValidationError, ValidationReport, ValuePath, ValueTree,
};

/// The declaration governing this exact data node. Inherited expression
/// prohibitions are tracked separately from a node's permission to execute.
#[derive(Clone, Copy)]
pub(super) enum Scope<'a> {
    Root(&'a [Field]),
    Field(&'a Field),
    Opaque,
}

#[derive(Clone, Copy)]
pub(super) enum Properties<'a> {
    Fields(&'a [Field]),
    Mode(Option<&'a Field>),
    Opaque,
}

impl<'a> Scope<'a> {
    pub(super) fn field(self) -> Option<&'a Field> {
        match self {
            Self::Field(field) => Some(field),
            _ => None,
        }
    }

    pub(super) fn properties<E>(
        self,
        values: &mut IndexMap<String, ValueTree<E>>,
    ) -> Properties<'a> {
        let fields = match self {
            Self::Root(fields) => Some(fields),
            Self::Field(Field::Object(object)) => Some(object.fields.as_slice()),
            _ => None,
        };
        if let Some(fields) = fields {
            fold_aliases(fields, values);
            return Properties::Fields(fields);
        }
        if let Self::Field(Field::Mode(mode)) = self {
            let selected = values.get("mode").and_then(ValueTree::as_str).or_else(|| {
                (!values.contains_key("mode"))
                    .then_some(mode.default_variant.as_deref())
                    .flatten()
            });
            return Properties::Mode(selected.and_then(|key| {
                mode.variants
                    .iter()
                    .find(|variant| variant.key == key)
                    .map(|variant| variant.field.as_ref())
            }));
        }
        Properties::Opaque
    }

    pub(super) fn item(self) -> Self {
        match self {
            Self::Field(Field::List(list)) => {
                list.item.as_deref().map_or(Self::Opaque, Self::Field)
            },
            _ => Self::Opaque,
        }
    }
}

impl<'a> Properties<'a> {
    pub(super) fn child(self, key: &str) -> Scope<'a> {
        match self {
            Self::Fields(fields) => fields
                .iter()
                .find(|field| field.key().as_str() == key)
                .map_or(Scope::Opaque, Scope::Field),
            Self::Mode(Some(field)) if key == "value" => Scope::Field(field),
            _ => Scope::Opaque,
        }
    }
}

/// Canonical key wins; otherwise the first declared alias wins. All aliases
/// are consumed, including losing aliases that might contain secret data.
pub(super) fn fold_aliases<V>(fields: &[Field], values: &mut IndexMap<String, V>) {
    for field in fields {
        let canonical = field.key().as_str();
        for alias in field.read_aliases() {
            if let Some(value) = values.shift_remove(alias.as_str()) {
                values.entry(canonical.to_owned()).or_insert(value);
            }
        }
    }
}

pub(super) struct PreparedValues {
    pub(super) values: CompiledValue,
    pub(super) expression_paths: Vec<ValuePath>,
}

#[tracing::instrument(level = "debug", skip_all, fields(field_count = schema.fields().len()))]
pub(super) fn prepare_input(
    values: AuthoredValue,
    schema: &ValidSchema,
) -> Result<PreparedValues, ValidationReport> {
    values.check_budget(|expression| expression.source())?;
    let values = match schema.scalar_schema() {
        Some(scalar) => scalar.prepare_value(values, &ValuePath::root())?,
        None => values,
    };
    let mut expression_paths = Vec::new();
    let values = compile_node(
        values,
        Scope::Root(schema.fields()),
        &ValuePath::root(),
        &mut expression_paths,
        false,
    )?;
    values.check_budget(|program| program.source())?;
    Ok(PreparedValues {
        values,
        expression_paths,
    })
}

fn compile_node(
    value: AuthoredValue,
    scope: Scope<'_>,
    path: &ValuePath,
    expressions: &mut Vec<ValuePath>,
    ancestor_forbids_expressions: bool,
) -> Result<CompiledValue, ValidationError> {
    let expressions_forbidden = ancestor_forbids_expressions
        || scope
            .field()
            .is_some_and(|field| matches!(field.expression(), ExpressionMode::Forbidden));
    if !matches!(&value, ValueTree::Expression(_))
        && scope
            .field()
            .is_some_and(|field| matches!(field.expression(), ExpressionMode::Required))
    {
        return Err(ValidationError::builder("expression.required")
            .at(path.clone())
            .message("field requires an authored expression")
            .build());
    }
    match value {
        ValueTree::Literal(value) => prepare_scalar(value, scope, path),
        ValueTree::Secret(secret) => prepare_secret(secret, scope, path),
        ValueTree::Object(mut values) => {
            let properties = scope.properties(&mut values);
            values
                .into_iter()
                .map(|(key, value)| {
                    let child = compile_node(
                        value,
                        properties.child(&key),
                        &path.push(&key),
                        expressions,
                        expressions_forbidden,
                    )?;
                    Ok((key, child))
                })
                .collect::<Result<_, _>>()
                .map(ValueTree::Object)
        },
        ValueTree::List(values) => values
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                compile_node(
                    value,
                    scope.item(),
                    &path.push(index.to_string()),
                    expressions,
                    expressions_forbidden,
                )
            })
            .collect::<Result<_, _>>()
            .map(ValueTree::List),
        ValueTree::Expression(expression) => {
            if expressions_forbidden
                || !scope.field().is_some_and(|field| {
                    matches!(
                        field.expression(),
                        ExpressionMode::Allowed | ExpressionMode::Required
                    )
                })
            {
                return Err(ValidationError::builder("expression.forbidden")
                    .at(path.clone())
                    .message("expression has no permitting field declaration")
                    .build());
            }
            let program = expression.parse_at(path)?.clone();
            expressions.push(path.clone());
            Ok(ValueTree::Expression(program))
        },
    }
}

/// Results are decoded as data, never as authoring syntax, and prepared once.
pub(super) fn prepare_result(
    scope: Scope<'_>,
    value: Value,
    path: &ValuePath,
) -> Result<ResolvedValue, ValidationError> {
    let depth = u8::try_from(path.depth()).map_err(|_| {
        ValidationError::builder("recursion_limit")
            .at(path.clone())
            .message("expression result exceeds the value depth limit")
            .build()
    })?;
    let value = ResolvedValue::from_data_at(value, path, depth)?;
    prepare_data(value, scope, path)
}

fn prepare_data(
    value: ResolvedValue,
    scope: Scope<'_>,
    path: &ValuePath,
) -> Result<ResolvedValue, ValidationError> {
    match value {
        ValueTree::Literal(value) => prepare_scalar(value, scope, path),
        ValueTree::Secret(secret) => prepare_secret(secret, scope, path),
        ValueTree::Object(mut values) => {
            let properties = scope.properties(&mut values);
            values
                .into_iter()
                .map(|(key, value)| {
                    let child = prepare_data(value, properties.child(&key), &path.push(&key))?;
                    Ok((key, child))
                })
                .collect::<Result<_, _>>()
                .map(ValueTree::Object)
        },
        ValueTree::List(values) => values
            .into_iter()
            .enumerate()
            .map(|(index, value)| prepare_data(value, scope.item(), &path.push(index.to_string())))
            .collect::<Result<_, _>>()
            .map(ValueTree::List),
        ValueTree::Expression(impossible) => match impossible {},
    }
}

fn prepare_scalar<E>(
    value: ScalarValue,
    scope: Scope<'_>,
    path: &ValuePath,
) -> Result<ValueTree<E>, ValidationError> {
    let mut value = value.into_json();
    if let Some(field) = scope.field() {
        for transformer in field.transformers() {
            value = transformer.apply(&value);
        }
        if matches!(field, Field::Secret(_))
            && let Value::String(text) = value
        {
            return Ok(ValueTree::Secret(SecretValue::string(text)));
        }
    }
    ScalarValue::try_from(value)
        .map(ValueTree::Literal)
        .map_err(|error| error.at(path.clone()))
}

fn prepare_secret<E>(
    secret: SecretValue,
    scope: Scope<'_>,
    path: &ValuePath,
) -> Result<ValueTree<E>, ValidationError> {
    if let Some(field) = scope.field()
        && !field.transformers().is_empty()
        && let SecretValue::String(text) = &secret
    {
        let mut value = Value::String(text.expose().to_owned());
        for transformer in field.transformers() {
            value = transformer.apply(&value);
        }
        let Value::String(text) = value else {
            return Err(ValidationError::builder("type_mismatch")
                .at(path.clone())
                .message("a secret transformer must preserve its string type")
                .build());
        };
        return Ok(ValueTree::Secret(SecretValue::string(text)));
    }
    Ok(ValueTree::Secret(secret))
}
