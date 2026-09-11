//! Scalar domains shared by root checking, preparation, and contract export.

use nebula_validator::{DiagnosticDisclosure, Rule, ValueRule};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::{ScalarValue, ValidationError, ValuePath, ValueTree};

/// A concrete scalar JSON shape. Integers form a subset of numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarKind {
    /// JSON null, the unit wire value.
    Null,
    /// JSON true or false.
    Boolean,
    /// A Unicode string.
    String,
    /// A losslessly representable i64/u64 integer.
    Integer,
    /// Any finite JSON number within the declared bounds.
    Number,
}

#[derive(Debug, Clone, PartialEq)]
enum ScalarDomain {
    Null,
    Boolean,
    String,
    Integer { minimum: Number, maximum: Number },
    Number { minimum: Number, maximum: Number },
}

/// A checked scalar domain and rules on the scalar itself.
///
/// There is no field key, visibility policy, or expression admission. Numeric
/// bounds are JSON numbers and use the validator's exact mixed-number ordering.
#[derive(Debug, Clone, PartialEq)]
pub struct ScalarSchema {
    domain: ScalarDomain,
    root_rules: Vec<Rule>,
}

impl ScalarSchema {
    /// Version of the scalar descriptor, independent of legacy schema/plan wire.
    pub const WIRE_VERSION: u16 = 1;

    /// The null-only JSON domain used by unit types and unit structs.
    #[must_use]
    pub const fn null() -> Self {
        Self {
            domain: ScalarDomain::Null,
            root_rules: Vec::new(),
        }
    }

    /// The boolean JSON domain.
    #[must_use]
    pub const fn boolean() -> Self {
        Self {
            domain: ScalarDomain::Boolean,
            root_rules: Vec::new(),
        }
    }

    /// The string JSON domain.
    #[must_use]
    pub const fn string() -> Self {
        Self {
            domain: ScalarDomain::String,
            root_rules: Vec::new(),
        }
    }

    /// Construct an inclusive integer domain within JSON's i64/u64 envelope.
    ///
    /// # Errors
    /// Rejects fractional bounds and reversed ranges.
    #[tracing::instrument(name = "schema.scalar.integer", skip_all, err)]
    pub fn integer(
        minimum: impl Into<Number>,
        maximum: impl Into<Number>,
    ) -> Result<Self, ValidationError> {
        let minimum = minimum.into();
        let maximum = maximum.into();
        if !(minimum.is_i64() || minimum.is_u64())
            || !(maximum.is_i64() || maximum.is_u64())
            || !at_least(&maximum, &minimum)
        {
            return Err(invalid_bounds());
        }
        Ok(Self {
            domain: ScalarDomain::Integer { minimum, maximum },
            root_rules: Vec::new(),
        })
    }

    /// Construct an inclusive finite number domain.
    ///
    /// # Errors
    /// Rejects non-finite bounds and reversed ranges.
    #[tracing::instrument(name = "schema.scalar.number", skip_all, err)]
    pub fn number(
        minimum: impl Into<Number>,
        maximum: impl Into<Number>,
    ) -> Result<Self, ValidationError> {
        let minimum = minimum.into();
        let maximum = maximum.into();
        if !at_least(&maximum, &minimum) {
            return Err(invalid_bounds());
        }
        Ok(Self {
            domain: ScalarDomain::Number { minimum, maximum },
            root_rules: Vec::new(),
        })
    }

    /// Append a rule evaluated normally by the staged validation pipeline.
    #[must_use]
    pub fn root_rule(mut self, rule: Rule) -> Self {
        self.root_rules.push(rule);
        self
    }

    /// The scalar's concrete JSON shape.
    #[must_use]
    pub const fn kind(&self) -> ScalarKind {
        match self.domain {
            ScalarDomain::Null => ScalarKind::Null,
            ScalarDomain::Boolean => ScalarKind::Boolean,
            ScalarDomain::String => ScalarKind::String,
            ScalarDomain::Integer { .. } => ScalarKind::Integer,
            ScalarDomain::Number { .. } => ScalarKind::Number,
        }
    }

    /// Inclusive lower numeric bound, if this is a numeric scalar.
    #[must_use]
    pub const fn minimum(&self) -> Option<&Number> {
        match &self.domain {
            ScalarDomain::Integer { minimum, .. } | ScalarDomain::Number { minimum, .. } => {
                Some(minimum)
            },
            _ => None,
        }
    }

    /// Inclusive upper numeric bound, if this is a numeric scalar.
    #[must_use]
    pub const fn maximum(&self) -> Option<&Number> {
        match &self.domain {
            ScalarDomain::Integer { maximum, .. } | ScalarDomain::Number { maximum, .. } => {
                Some(maximum)
            },
            _ => None,
        }
    }

    /// Root rules, separate from the intrinsic domain checks.
    #[must_use]
    pub fn root_rules(&self) -> &[Rule] {
        &self.root_rules
    }

    /// Check the intrinsic shape and exact bounds without executing root rules.
    pub(crate) fn validate_value<E>(
        &self,
        value: &ValueTree<E>,
        path: &ValuePath,
    ) -> Result<(), ValidationError> {
        if matches!(value, ValueTree::Expression(_)) {
            return Err(ValidationError::builder("expression.forbidden")
                .at(path.clone())
                .message("scalar roots do not admit expressions")
                .build());
        }
        let Some(value) = value.as_literal() else {
            return Err(type_error(path));
        };
        let correct = match (&self.domain, value) {
            (ScalarDomain::Null, Value::Null)
            | (ScalarDomain::Boolean, Value::Bool(_))
            | (ScalarDomain::String, Value::String(_)) => true,
            (ScalarDomain::Number { .. }, Value::Number(_)) => true,
            (ScalarDomain::Integer { .. }, Value::Number(number)) => {
                exact_integer(number).is_some()
            },
            _ => false,
        };
        if !correct {
            return Err(type_error(path));
        }
        if let (Some(minimum), Some(maximum)) = (self.minimum(), self.maximum()) {
            for rule in [
                ValueRule::Min(minimum.clone()),
                ValueRule::Max(maximum.clone()),
            ] {
                rule.validate_value(value, DiagnosticDisclosure::IncludeValue)
                    .map_err(|error| {
                        ValidationError::builder(error.code.into_owned())
                            .at(path.clone())
                            .message("scalar is outside its declared numeric domain")
                            .build()
                    })?;
            }
        }
        Ok(())
    }

    /// Retain lossless integral-number normalization; never evaluate a program.
    pub(crate) fn prepare_value<E>(
        &self,
        value: ValueTree<E>,
        path: &ValuePath,
    ) -> Result<ValueTree<E>, ValidationError> {
        self.validate_value(&value, path)?;
        if self.kind() == ScalarKind::Integer
            && let Some(number) = value
                .as_literal()
                .and_then(Value::as_number)
                .and_then(exact_integer)
        {
            return Ok(ValueTree::Literal(ScalarValue::try_from(Value::Number(
                number,
            ))?));
        }
        Ok(value)
    }
}

impl Serialize for ScalarSchema {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let has_bounds = self.minimum().is_some();
        let has_rules = !self.root_rules.is_empty();
        let mut wire = serializer.serialize_struct(
            "ScalarSchema",
            2 + 2 * usize::from(has_bounds) + usize::from(has_rules),
        )?;
        wire.serialize_field("version", &Self::WIRE_VERSION)?;
        wire.serialize_field("type", &self.kind())?;
        if let (Some(minimum), Some(maximum)) = (self.minimum(), self.maximum()) {
            wire.serialize_field("minimum", minimum)?;
            wire.serialize_field("maximum", maximum)?;
        }
        if has_rules {
            wire.serialize_field("root_rules", &self.root_rules)?;
        }
        wire.end()
    }
}

impl<'de> Deserialize<'de> for ScalarSchema {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            version: u16,
            #[serde(rename = "type")]
            kind: ScalarKind,
            #[serde(default, deserialize_with = "super::deserialize_present")]
            minimum: Option<Number>,
            #[serde(default, deserialize_with = "super::deserialize_present")]
            maximum: Option<Number>,
            #[serde(default)]
            root_rules: Vec<Rule>,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.version != Self::WIRE_VERSION {
            return Err(serde::de::Error::custom(
                "unsupported scalar schema version",
            ));
        }
        let mut scalar = match (wire.kind, wire.minimum, wire.maximum) {
            (ScalarKind::Null, None, None) => Self::null(),
            (ScalarKind::Boolean, None, None) => Self::boolean(),
            (ScalarKind::String, None, None) => Self::string(),
            (ScalarKind::Integer, Some(minimum), Some(maximum)) => {
                Self::integer(minimum, maximum).map_err(serde::de::Error::custom)?
            },
            (ScalarKind::Number, Some(minimum), Some(maximum)) => {
                Self::number(minimum, maximum).map_err(serde::de::Error::custom)?
            },
            _ => {
                return Err(serde::de::Error::custom(
                    "scalar kind and numeric bounds disagree",
                ));
            },
        };
        scalar.root_rules = wire.root_rules;
        Ok(scalar)
    }
}

fn at_least(value: &Number, minimum: &Number) -> bool {
    ValueRule::Min(minimum.clone())
        .validate_value(
            &Value::Number(value.clone()),
            DiagnosticDisclosure::IncludeValue,
        )
        .is_ok()
}

fn invalid_bounds() -> ValidationError {
    ValidationError::builder("schema.scalar_bounds")
        .message("numeric bounds must be representable, ordered, and match the scalar kind")
        .build()
}

fn type_error(path: &ValuePath) -> ValidationError {
    ValidationError::builder("type_mismatch")
        .at(path.clone())
        .message("value does not match the declared scalar domain")
        .build()
}

fn exact_integer(number: &Number) -> Option<Number> {
    if number.is_i64() || number.is_u64() {
        return Some(number.clone());
    }
    let number = number.as_f64()?;
    // Both endpoints are exact powers of two. Test BEFORE casting: u64::MAX
    // rounded to f64 is 2^64, which would otherwise saturate and change the value.
    if number.fract() != 0.0
        || !(-9_223_372_036_854_775_808.0..18_446_744_073_709_551_616.0).contains(&number)
    {
        return None;
    }
    Some(if number < 0.0 {
        Number::from(number as i64)
    } else {
        Number::from(number as u64)
    })
}
