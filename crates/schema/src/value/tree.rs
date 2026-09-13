//! Canonical value representation, parameterized by its expression capability.

use std::convert::Infallible;

use indexmap::IndexMap;
use nebula_expression::CompiledProgram;
use nebula_validator::foundation::FieldPath as ValuePath;
use serde_json::Value;

use crate::{Expression, SecretValue, ValidationError};

use super::{MAX_VALUE_DEPTH, MAX_VALUE_NODES, budget::ValueBudget};

/// A JSON scalar. Objects and arrays are represented only by tree containers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarValue(Value);

impl ScalarValue {
    /// Borrow the scalar without allocating a validator view.
    #[must_use]
    pub const fn as_json(&self) -> &Value {
        &self.0
    }

    /// Consume the scalar into its JSON representation.
    #[must_use]
    pub fn into_json(self) -> Value {
        self.0
    }
}

impl TryFrom<Value> for ScalarValue {
    type Error = ValidationError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if value.is_array() || value.is_object() {
            return Err(ValidationError::builder("type_mismatch")
                .message("a scalar cannot contain an object or array")
                .build());
        }
        Ok(Self(value))
    }
}

/// A canonical tree with one representation per data shape.
///
/// The expression parameter controls what a tree can contain; private schema
/// proof tokens additionally certify admission, preparation, and validation.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueTree<E> {
    /// Null, boolean, number, or string data.
    Literal(ScalarValue),
    /// Object data. Property names are data, not schema identifiers.
    Object(IndexMap<String, Self>),
    /// Ordered data.
    List(Vec<Self>),
    /// Code admitted according to the tree's expression capability.
    Expression(E),
    /// Secret material, redacted by diagnostic and ordinary JSON views.
    Secret(SecretValue),
}

/// Authoring input containing lazy expression sources.
pub type AuthoredValue = ValueTree<Expression>;
/// Prepared input containing retained, immutable programs.
pub type CompiledValue = ValueTree<CompiledProgram>;
/// Runtime data whose expression variant is uninhabited.
pub type ResolvedValue = ValueTree<Infallible>;

impl<E> ValueTree<E> {
    /// Create an empty object.
    #[must_use]
    pub fn object() -> Self {
        Self::Object(IndexMap::new())
    }

    /// Convert JSON data without interpreting any content as an expression.
    ///
    /// # Errors
    /// Returns `recursion_limit` for data deeper than [`MAX_VALUE_DEPTH`] or
    /// `value.limit_exceeded` when a structural custody budget is exceeded.
    pub fn from_data(value: Value) -> Result<Self, ValidationError> {
        Self::from_data_at(value, &ValuePath::root(), 0)
    }

    pub(crate) fn from_data_at(
        value: Value,
        path: &ValuePath,
        depth: u8,
    ) -> Result<Self, ValidationError> {
        Self::from_data_with_budget(value, path, depth, &ValueBudget::default())
    }

    fn from_data_with_budget(
        value: Value,
        path: &ValuePath,
        depth: u8,
        budget: &ValueBudget,
    ) -> Result<Self, ValidationError> {
        check_depth(path, depth)?;
        budget.charge_data_node(path)?;
        match value {
            Value::Object(values) => {
                let mut tree = IndexMap::with_capacity(values.len().min(MAX_VALUE_NODES));
                for (key, value) in values {
                    let child_path = path.push(&key);
                    budget.charge_data_text(key.len(), &child_path)?;
                    tree.insert(
                        key,
                        Self::from_data_with_budget(value, &child_path, depth + 1, budget)?,
                    );
                }
                Ok(Self::Object(tree))
            },
            Value::Array(values) => {
                let mut tree = Vec::with_capacity(values.len().min(MAX_VALUE_NODES));
                for (index, value) in values.into_iter().enumerate() {
                    tree.push(Self::from_data_with_budget(
                        value,
                        &path.push(index.to_string()),
                        depth + 1,
                        budget,
                    )?);
                }
                Ok(Self::List(tree))
            },
            scalar => {
                if let Value::String(value) = &scalar {
                    budget.charge_data_text(value.len(), path)?;
                }
                Ok(Self::Literal(ScalarValue(scalar)))
            },
        }
    }

    /// Borrow a scalar. Secrets are deliberately excluded.
    #[must_use]
    pub fn as_literal(&self) -> Option<&Value> {
        match self {
            Self::Literal(value) => Some(value.as_json()),
            _ => None,
        }
    }

    /// Borrow a non-secret string scalar.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        self.as_literal().and_then(Value::as_str)
    }

    /// Borrow the properties of an object.
    #[must_use]
    pub const fn as_object(&self) -> Option<&IndexMap<String, Self>> {
        match self {
            Self::Object(values) => Some(values),
            _ => None,
        }
    }

    /// Borrow a property without interpreting its name as a path.
    #[must_use]
    pub fn get(&self, key: impl AsRef<str>) -> Option<&Self> {
        self.as_object()?.get(key.as_ref())
    }

    /// Look up an RFC6901 pointer. Numeric object keys remain object keys.
    #[must_use]
    pub fn get_path(&self, path: &ValuePath) -> Option<&Self> {
        let mut current = self;
        for segment in path.segments() {
            current = match current {
                Self::Object(values) => values.get(segment.as_ref())?,
                Self::List(values) => {
                    if (segment.starts_with('0') && segment.len() > 1)
                        || segment.is_empty()
                        || !segment.bytes().all(|byte| byte.is_ascii_digit())
                    {
                        return None;
                    }
                    values.get(segment.parse::<usize>().ok()?)?
                },
                _ => return None,
            };
        }
        Some(current)
    }

    /// Insert an object property. This does not confer schema validation.
    ///
    /// # Errors
    /// Returns `type_mismatch` when called on a non-object.
    pub fn insert(
        &mut self,
        key: impl Into<String>,
        value: Self,
    ) -> Result<Option<Self>, ValidationError> {
        let Self::Object(values) = self else {
            return Err(ValidationError::builder("type_mismatch")
                .message("properties can only be inserted into an object")
                .build());
        };
        Ok(values.insert(key.into(), value))
    }

    /// Insert literal data into an object, without expression interpretation.
    ///
    /// # Errors
    /// Returns `type_mismatch` for a non-object, or `recursion_limit` for deep data.
    pub fn insert_data(
        &mut self,
        key: impl Into<String>,
        value: Value,
    ) -> Result<Option<Self>, ValidationError> {
        self.insert(key, Self::from_data(value)?)
    }

    /// Locate protected material without exposing its contents.
    #[must_use]
    pub fn first_secret_path(&self) -> Option<ValuePath> {
        let mut pending = vec![(self, ValuePath::root())];
        while let Some((value, path)) = pending.pop() {
            match value {
                Self::Secret(_) => return Some(path),
                Self::Object(values) => pending.extend(
                    values
                        .iter()
                        .rev()
                        .map(|(key, value)| (value, path.push(key))),
                ),
                Self::List(values) => pending.extend(
                    values
                        .iter()
                        .enumerate()
                        .rev()
                        .map(|(index, value)| (value, path.push(index.to_string()))),
                ),
                _ => {},
            }
        }
        None
    }

    pub(crate) fn check_depth(&self, path: &ValuePath, depth: u8) -> Result<(), ValidationError> {
        check_depth(path, depth)?;
        match self {
            Self::Object(values) => {
                for (key, value) in values {
                    value.check_depth(&path.push(key), depth + 1)?;
                }
            },
            Self::List(values) => {
                for (index, value) in values.iter().enumerate() {
                    value.check_depth(&path.push(index.to_string()), depth + 1)?;
                }
            },
            _ => {},
        }
        Ok(())
    }

    pub(crate) fn check_budget<'a>(
        &'a self,
        expression_source: impl Fn(&'a E) -> &'a str + Copy,
    ) -> Result<(), ValidationError> {
        self.check_budget_at(
            &ValuePath::root(),
            0,
            &ValueBudget::default(),
            expression_source,
        )
    }

    fn check_budget_at<'a>(
        &'a self,
        path: &ValuePath,
        depth: u8,
        budget: &ValueBudget,
        expression_source: impl Fn(&'a E) -> &'a str + Copy,
    ) -> Result<(), ValidationError> {
        check_depth(path, depth)?;
        budget.charge_data_node(path)?;
        match self {
            Self::Literal(value) => {
                if let Some(text) = value.as_json().as_str() {
                    budget.charge_data_text(text.len(), path)?;
                }
            },
            Self::Object(values) => {
                for (key, value) in values {
                    let child_path = path.push(key);
                    budget.charge_data_text(key.len(), &child_path)?;
                    value.check_budget_at(&child_path, depth + 1, budget, expression_source)?;
                }
            },
            Self::List(values) => {
                for (index, value) in values.iter().enumerate() {
                    value.check_budget_at(
                        &path.push(index.to_string()),
                        depth + 1,
                        budget,
                        expression_source,
                    )?;
                }
            },
            Self::Expression(expression) => {
                budget.charge_expression(path, expression_source(expression))?;
            },
            Self::Secret(secret) => budget.charge_data_text(secret.len_bytes(), path)?,
        }
        Ok(())
    }

    pub(crate) fn json_with(&self, expression: &impl Fn(&E) -> Value) -> Value {
        match self {
            Self::Literal(value) => value.as_json().clone(),
            Self::Object(values) => Value::Object(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), value.json_with(expression)))
                    .collect(),
            ),
            Self::List(values) => Value::Array(
                values
                    .iter()
                    .map(|value| value.json_with(expression))
                    .collect(),
            ),
            Self::Expression(value) => expression(value),
            Self::Secret(_) => Value::String("<redacted>".into()),
        }
    }
}

impl AuthoredValue {
    /// Decode authoring shorthand: expression strings and explicit `$expr` envelopes.
    ///
    /// Use [`Self::from_data`] for external data or literal template-like strings.
    /// Serde uses an unambiguous tagged representation instead of this shorthand.
    ///
    /// # Errors
    /// Returns `recursion_limit` for input deeper than [`MAX_VALUE_DEPTH`] or
    /// `value.limit_exceeded` when a structural custody budget is exceeded.
    pub fn from_template_json(value: Value) -> Result<Self, ValidationError> {
        Self::template_at(value, &ValuePath::root(), 0, &ValueBudget::default())
    }

    fn template_at(
        value: Value,
        path: &ValuePath,
        depth: u8,
        budget: &ValueBudget,
    ) -> Result<Self, ValidationError> {
        check_depth(path, depth)?;
        budget.charge_data_node(path)?;
        match value {
            Value::String(value) if nebula_expression::has_expression_marker(&value) => {
                budget.charge_expression(path, &value)?;
                Ok(Self::Expression(Expression::new(value)))
            },
            Value::Object(values)
                if values.len() == 1 && values.get("$expr").is_some_and(Value::is_string) =>
            {
                let Some(Value::String(source)) = values.into_values().next() else {
                    return Err(ValidationError::builder("expression.parse")
                        .message("expression envelope must contain a string")
                        .build());
                };
                budget.charge_expression(path, &source)?;
                Ok(Self::Expression(Expression::new(source)))
            },
            Value::Object(values) => {
                let mut tree = IndexMap::with_capacity(values.len().min(MAX_VALUE_NODES));
                for (key, value) in values {
                    let child_path = path.push(&key);
                    budget.charge_data_text(key.len(), &child_path)?;
                    tree.insert(
                        key,
                        Self::template_at(value, &child_path, depth + 1, budget)?,
                    );
                }
                Ok(Self::Object(tree))
            },
            Value::Array(values) => {
                let mut tree = Vec::with_capacity(values.len().min(MAX_VALUE_NODES));
                for (index, value) in values.into_iter().enumerate() {
                    tree.push(Self::template_at(
                        value,
                        &path.push(index.to_string()),
                        depth + 1,
                        budget,
                    )?);
                }
                Ok(Self::List(tree))
            },
            scalar => {
                if let Value::String(value) = &scalar {
                    budget.charge_data_text(value.len(), path)?;
                }
                Ok(Self::Literal(ScalarValue(scalar)))
            },
        }
    }

    /// Render a redacted JSON view; use serde to persist authoring identity.
    #[must_use]
    pub fn to_json(&self) -> Value {
        self.json_with(&|expression| serde_json::json!({"$expr": expression.source()}))
    }
}

impl CompiledValue {
    /// Render a redacted JSON view of prepared values.
    #[must_use]
    pub fn to_json(&self) -> Value {
        self.json_with(&|program| serde_json::json!({"$expr": program.source()}))
    }
}

impl ResolvedValue {
    /// Render runtime data, redacting any protected secret leaves.
    #[must_use]
    pub fn to_json(&self) -> Value {
        self.json_with(&|&never| match never {})
    }
}

pub(super) fn check_depth(path: &ValuePath, depth: u8) -> Result<(), ValidationError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(ValidationError::builder("recursion_limit")
            .at(path.clone())
            .param("limit", MAX_VALUE_DEPTH)
            .message("value tree exceeds the recursion limit")
            .build());
    }
    Ok(())
}
