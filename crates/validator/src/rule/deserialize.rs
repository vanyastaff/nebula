//! Bounded manual deserialization for [`Rule`].

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

use super::{Predicate, Rule, RuleBuildError, RulePattern, ValueRule, limits::RuleBudgetState};
use crate::foundation::FieldPath;

impl<'de> serde::Deserialize<'de> for Rule {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut budget = RuleBudgetState::default();
        RuleSeed {
            budget: &mut budget,
            depth: 1,
        }
        .deserialize(deserializer)
    }
}

struct RuleSeed<'a> {
    budget: &'a mut RuleBudgetState,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for RuleSeed<'_> {
    type Value = Rule;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        self.budget
            .enter_rule(self.depth)
            .map_err(de::Error::custom)?;
        deserializer.deserialize_any(RuleVisitor {
            budget: self.budget,
            depth: self.depth,
        })
    }
}

struct RuleVisitor<'a> {
    budget: &'a mut RuleBudgetState,
    depth: usize,
}

impl<'de> Visitor<'de> for RuleVisitor<'_> {
    type Value = Rule;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a unit rule or a single-key rule object")
    }

    fn visit_str<E: de::Error>(self, name: &str) -> Result<Self::Value, E> {
        match name {
            "email" => Ok(Rule::email()),
            "url" => Ok(Rule::url()),
            _ => Err(E::custom("unknown unit rule; expected email or url")),
        }
    }

    fn visit_string<E: de::Error>(self, name: String) -> Result<Self::Value, E> {
        self.visit_str(&name)
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let RuleVisitor { budget, depth } = self;
        let Some(key) = map.next_key_seed(RuleKeySeed)? else {
            return Err(de::Error::custom("empty rule object"));
        };

        let rule = match key {
            RuleKey::MinLength => Rule::min_length(map.next_value()?),
            RuleKey::MaxLength => Rule::max_length(map.next_value()?),
            RuleKey::Pattern => {
                let pattern = map.next_value_seed(BoundedTextSeed { budget })?;
                let pattern = RulePattern::new(&pattern).map_err(de::Error::custom)?;
                admitted(Rule::value(ValueRule::Pattern(pattern)))?
            },
            RuleKey::Min => admitted(Rule::value(ValueRule::Min(map.next_value()?)))?,
            RuleKey::Max => admitted(Rule::value(ValueRule::Max(map.next_value()?)))?,
            RuleKey::GreaterThan => {
                admitted(Rule::value(ValueRule::GreaterThan(map.next_value()?)))?
            },
            RuleKey::LessThan => admitted(Rule::value(ValueRule::LessThan(map.next_value()?)))?,
            RuleKey::OneOf => {
                let values = map.next_value_seed(JsonOperandsSeed { budget })?;
                admitted(Rule::value(ValueRule::OneOf(values)))?
            },
            RuleKey::MinItems => Rule::min_items(map.next_value()?),
            RuleKey::MaxItems => Rule::max_items(map.next_value()?),
            RuleKey::Email => {
                let _: () = map.next_value()?;
                Rule::email()
            },
            RuleKey::Url => {
                let _: () = map.next_value()?;
                Rule::url()
            },
            RuleKey::Eq => {
                let (path, value) = map.next_value_seed(PathJsonSeed { budget })?;
                admitted(Rule::predicate(Predicate::Eq(path, value)))?
            },
            RuleKey::Ne => {
                let (path, value) = map.next_value_seed(PathJsonSeed { budget })?;
                admitted(Rule::predicate(Predicate::Ne(path, value)))?
            },
            RuleKey::Gt => {
                let (path, value) = map.next_value_seed(PathNumberSeed { budget })?;
                admitted(Rule::predicate(Predicate::Gt(path, value)))?
            },
            RuleKey::Gte => {
                let (path, value) = map.next_value_seed(PathNumberSeed { budget })?;
                admitted(Rule::predicate(Predicate::Gte(path, value)))?
            },
            RuleKey::Lt => {
                let (path, value) = map.next_value_seed(PathNumberSeed { budget })?;
                admitted(Rule::predicate(Predicate::Lt(path, value)))?
            },
            RuleKey::Lte => {
                let (path, value) = map.next_value_seed(PathNumberSeed { budget })?;
                admitted(Rule::predicate(Predicate::Lte(path, value)))?
            },
            RuleKey::IsTrue => admitted(Rule::predicate(Predicate::IsTrue(next_path(
                &mut map, budget,
            )?)))?,
            RuleKey::IsFalse => admitted(Rule::predicate(Predicate::IsFalse(next_path(
                &mut map, budget,
            )?)))?,
            RuleKey::Set => admitted(Rule::predicate(Predicate::Set(next_path(
                &mut map, budget,
            )?)))?,
            RuleKey::Empty => admitted(Rule::predicate(Predicate::Empty(next_path(
                &mut map, budget,
            )?)))?,
            RuleKey::Contains => {
                let (path, value) = map.next_value_seed(PathJsonSeed { budget })?;
                admitted(Rule::predicate(Predicate::Contains(path, value)))?
            },
            RuleKey::Matches => {
                let (path, pattern) = map.next_value_seed(PathPatternSeed { budget })?;
                admitted(Rule::predicate(Predicate::Matches(path, pattern)))?
            },
            RuleKey::In => {
                let (path, values) = map.next_value_seed(PathJsonOperandsSeed { budget })?;
                admitted(Rule::predicate(Predicate::In(path, values)))?
            },
            RuleKey::All => admitted(Rule::all(map.next_value_seed(RuleListSeed {
                budget,
                child_depth: depth.saturating_add(1),
            })?))?,
            RuleKey::Any => admitted(Rule::any(map.next_value_seed(RuleListSeed {
                budget,
                child_depth: depth.saturating_add(1),
            })?))?,
            RuleKey::Not => {
                budget.add_operands(1).map_err(de::Error::custom)?;
                let inner = map.next_value_seed(RuleSeed {
                    budget,
                    depth: depth.saturating_add(1),
                })?;
                admitted(Rule::not(inner))?
            },
            RuleKey::Custom => {
                let expression = map.next_value_seed(BoundedTextSeed { budget })?;
                admitted(Rule::custom(expression))?
            },
            RuleKey::UniqueBy => {
                let path = next_path(&mut map, budget)?;
                admitted(Rule::unique_by(path.as_str()))?
            },
            RuleKey::Described => map.next_value_seed(DescribedSeed {
                budget,
                child_depth: depth.saturating_add(1),
            })?,
            RuleKey::Unknown => return Err(de::Error::custom("unknown rule key")),
        };

        if map.next_key::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom("rule object must have exactly one key"));
        }
        Ok(rule)
    }
}

#[derive(Clone, Copy)]
enum RuleKey {
    MinLength,
    MaxLength,
    Pattern,
    Min,
    Max,
    GreaterThan,
    LessThan,
    OneOf,
    MinItems,
    MaxItems,
    Email,
    Url,
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    IsTrue,
    IsFalse,
    Set,
    Empty,
    Contains,
    Matches,
    In,
    All,
    Any,
    Not,
    Custom,
    UniqueBy,
    Described,
    Unknown,
}

struct RuleKeySeed;

impl<'de> DeserializeSeed<'de> for RuleKeySeed {
    type Value = RuleKey;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_identifier(RuleKeyVisitor)
    }
}

struct RuleKeyVisitor;

impl Visitor<'_> for RuleKeyVisitor {
    type Value = RuleKey;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a supported rule key")
    }

    fn visit_str<E: de::Error>(self, key: &str) -> Result<Self::Value, E> {
        Ok(match key {
            "min_length" => RuleKey::MinLength,
            "max_length" => RuleKey::MaxLength,
            "pattern" => RuleKey::Pattern,
            "min" => RuleKey::Min,
            "max" => RuleKey::Max,
            "greater_than" => RuleKey::GreaterThan,
            "less_than" => RuleKey::LessThan,
            "one_of" => RuleKey::OneOf,
            "min_items" => RuleKey::MinItems,
            "max_items" => RuleKey::MaxItems,
            "email" => RuleKey::Email,
            "url" => RuleKey::Url,
            "eq" => RuleKey::Eq,
            "ne" => RuleKey::Ne,
            "gt" => RuleKey::Gt,
            "gte" => RuleKey::Gte,
            "lt" => RuleKey::Lt,
            "lte" => RuleKey::Lte,
            "is_true" => RuleKey::IsTrue,
            "is_false" => RuleKey::IsFalse,
            "set" => RuleKey::Set,
            "empty" => RuleKey::Empty,
            "contains" => RuleKey::Contains,
            "matches" => RuleKey::Matches,
            "in" => RuleKey::In,
            "all" => RuleKey::All,
            "any" => RuleKey::Any,
            "not" => RuleKey::Not,
            "custom" => RuleKey::Custom,
            "unique_by" => RuleKey::UniqueBy,
            "described" => RuleKey::Described,
            _ => RuleKey::Unknown,
        })
    }

    fn visit_string<E: de::Error>(self, key: String) -> Result<Self::Value, E> {
        self.visit_str(&key)
    }
}

struct RuleListSeed<'a> {
    budget: &'a mut RuleBudgetState,
    child_depth: usize,
}

impl<'de> DeserializeSeed<'de> for RuleListSeed<'_> {
    type Value = Vec<Rule>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(RuleListVisitor {
            budget: self.budget,
            child_depth: self.child_depth,
        })
    }
}

struct RuleListVisitor<'a> {
    budget: &'a mut RuleBudgetState,
    child_depth: usize,
}

impl<'de> Visitor<'de> for RuleListVisitor<'_> {
    type Value = Vec<Rule>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded list of rules")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut rules = Vec::new();
        while let Some(rule) = sequence.next_element_seed(RuleOperandSeed {
            budget: &mut *self.budget,
            depth: self.child_depth,
        })? {
            rules.push(rule);
        }
        Ok(rules)
    }
}

struct RuleOperandSeed<'a> {
    budget: &'a mut RuleBudgetState,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for RuleOperandSeed<'_> {
    type Value = Rule;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        self.budget.add_operands(1).map_err(de::Error::custom)?;
        RuleSeed {
            budget: self.budget,
            depth: self.depth,
        }
        .deserialize(deserializer)
    }
}

struct DescribedSeed<'a> {
    budget: &'a mut RuleBudgetState,
    child_depth: usize,
}

impl<'de> DeserializeSeed<'de> for DescribedSeed<'_> {
    type Value = Rule;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(DescribedVisitor {
            budget: self.budget,
            child_depth: self.child_depth,
        })
    }
}

struct DescribedVisitor<'a> {
    budget: &'a mut RuleBudgetState,
    child_depth: usize,
}

impl<'de> Visitor<'de> for DescribedVisitor<'_> {
    type Value = Rule;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a rule and description pair")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        self.budget.add_operands(1).map_err(de::Error::custom)?;
        let inner = sequence
            .next_element_seed(RuleSeed {
                budget: &mut *self.budget,
                depth: self.child_depth,
            })?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let message = sequence
            .next_element_seed(BoundedTextSeed {
                budget: &mut *self.budget,
            })?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        reject_extra_pair_element(&mut sequence)?;
        admitted(Rule::described(inner, message))
    }
}

struct BoundedTextSeed<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> DeserializeSeed<'de> for BoundedTextSeed<'_> {
    type Value = String;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_string(BoundedTextVisitor {
            budget: self.budget,
        })
    }
}

struct BoundedTextVisitor<'a> {
    budget: &'a mut RuleBudgetState,
}

impl Visitor<'_> for BoundedTextVisitor<'_> {
    type Value = String;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded text")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Self::Value, E> {
        self.budget.add_text(text).map_err(E::custom)?;
        Ok(text.to_owned())
    }

    fn visit_string<E: de::Error>(self, text: String) -> Result<Self::Value, E> {
        self.budget.add_text(&text).map_err(E::custom)?;
        Ok(text)
    }
}

struct PathSeed<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> DeserializeSeed<'de> for PathSeed<'_> {
    type Value = FieldPath;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_string(PathVisitor {
            budget: self.budget,
        })
    }
}

struct PathVisitor<'a> {
    budget: &'a mut RuleBudgetState,
}

impl Visitor<'_> for PathVisitor<'_> {
    type Value = FieldPath;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded RFC 6901 JSON Pointer")
    }

    fn visit_str<E: de::Error>(self, path: &str) -> Result<Self::Value, E> {
        self.budget.add_text(path).map_err(E::custom)?;
        FieldPath::from_pointer(path).map_err(|_| E::custom(RuleBuildError::InvalidFieldPath))
    }

    fn visit_string<E: de::Error>(self, path: String) -> Result<Self::Value, E> {
        self.visit_str(&path)
    }
}

fn next_path<'de, M: MapAccess<'de>>(
    map: &mut M,
    budget: &mut RuleBudgetState,
) -> Result<FieldPath, M::Error> {
    map.next_value_seed(PathSeed { budget })
}

struct BoundedJsonSeed<'a> {
    budget: &'a mut RuleBudgetState,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for BoundedJsonSeed<'_> {
    type Value = serde_json::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        self.budget
            .enter_json(self.depth)
            .map_err(de::Error::custom)?;
        deserializer.deserialize_any(BoundedJsonVisitor {
            budget: self.budget,
            depth: self.depth,
        })
    }
}

struct BoundedJsonVisitor<'a> {
    budget: &'a mut RuleBudgetState,
    depth: usize,
}

impl<'de> Visitor<'de> for BoundedJsonVisitor<'_> {
    type Value = serde_json::Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Number(value.into()))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Number(value.into()))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Self::Value, E> {
        self.budget.add_text(text).map_err(E::custom)?;
        Ok(serde_json::Value::String(text.to_owned()))
    }

    fn visit_string<E: de::Error>(self, text: String) -> Result<Self::Value, E> {
        self.budget.add_text(&text).map_err(E::custom)?;
        Ok(serde_json::Value::String(text))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(BoundedJsonSeed {
            budget: &mut *self.budget,
            depth: self.depth.saturating_add(1),
        })? {
            values.push(value);
        }
        Ok(serde_json::Value::Array(values))
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut values = serde_json::Map::new();
        while let Some(key) = map.next_key_seed(BoundedTextSeed {
            budget: &mut *self.budget,
        })? {
            let value = map.next_value_seed(BoundedJsonSeed {
                budget: &mut *self.budget,
                depth: self.depth.saturating_add(1),
            })?;
            values.insert(key, value);
        }
        Ok(serde_json::Value::Object(values))
    }
}

struct JsonOperandsSeed<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> DeserializeSeed<'de> for JsonOperandsSeed<'_> {
    type Value = Vec<serde_json::Value>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(JsonOperandsVisitor {
            budget: self.budget,
        })
    }
}

struct JsonOperandsVisitor<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> Visitor<'de> for JsonOperandsVisitor<'_> {
    type Value = Vec<serde_json::Value>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded list of JSON operands")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(JsonOperandSeed {
            budget: &mut *self.budget,
        })? {
            values.push(value);
        }
        Ok(values)
    }
}

struct JsonOperandSeed<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> DeserializeSeed<'de> for JsonOperandSeed<'_> {
    type Value = serde_json::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        self.budget.add_operands(1).map_err(de::Error::custom)?;
        BoundedJsonSeed {
            budget: self.budget,
            depth: 1,
        }
        .deserialize(deserializer)
    }
}

struct PathJsonSeed<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> DeserializeSeed<'de> for PathJsonSeed<'_> {
    type Value = (FieldPath, serde_json::Value);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(PathJsonVisitor {
            budget: self.budget,
        })
    }
}

struct PathJsonVisitor<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> Visitor<'de> for PathJsonVisitor<'_> {
    type Value = (FieldPath, serde_json::Value);

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a field path and bounded JSON operand pair")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let path = sequence
            .next_element_seed(PathSeed {
                budget: &mut *self.budget,
            })?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let value = sequence
            .next_element_seed(BoundedJsonSeed {
                budget: &mut *self.budget,
                depth: 1,
            })?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        reject_extra_pair_element(&mut sequence)?;
        Ok((path, value))
    }
}

struct PathJsonOperandsSeed<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> DeserializeSeed<'de> for PathJsonOperandsSeed<'_> {
    type Value = (FieldPath, Vec<serde_json::Value>);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(PathJsonOperandsVisitor {
            budget: self.budget,
        })
    }
}

struct PathJsonOperandsVisitor<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> Visitor<'de> for PathJsonOperandsVisitor<'_> {
    type Value = (FieldPath, Vec<serde_json::Value>);

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a field path and bounded JSON operand list pair")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let path = sequence
            .next_element_seed(PathSeed {
                budget: &mut *self.budget,
            })?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let values = sequence
            .next_element_seed(JsonOperandsSeed {
                budget: &mut *self.budget,
            })?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        reject_extra_pair_element(&mut sequence)?;
        Ok((path, values))
    }
}

struct PathNumberSeed<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> DeserializeSeed<'de> for PathNumberSeed<'_> {
    type Value = (FieldPath, serde_json::Number);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(PathNumberVisitor {
            budget: self.budget,
        })
    }
}

struct PathNumberVisitor<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> Visitor<'de> for PathNumberVisitor<'_> {
    type Value = (FieldPath, serde_json::Number);

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a field path and JSON number pair")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let path = sequence
            .next_element_seed(PathSeed {
                budget: &mut *self.budget,
            })?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let value = sequence
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        reject_extra_pair_element(&mut sequence)?;
        Ok((path, value))
    }
}

struct PathPatternSeed<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> DeserializeSeed<'de> for PathPatternSeed<'_> {
    type Value = (FieldPath, RulePattern);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(PathPatternVisitor {
            budget: self.budget,
        })
    }
}

struct PathPatternVisitor<'a> {
    budget: &'a mut RuleBudgetState,
}

impl<'de> Visitor<'de> for PathPatternVisitor<'_> {
    type Value = (FieldPath, RulePattern);

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a field path and bounded pattern pair")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let path = sequence
            .next_element_seed(PathSeed {
                budget: &mut *self.budget,
            })?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let pattern = sequence
            .next_element_seed(BoundedTextSeed {
                budget: &mut *self.budget,
            })?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        reject_extra_pair_element(&mut sequence)?;
        let pattern = RulePattern::new(&pattern).map_err(de::Error::custom)?;
        Ok((path, pattern))
    }
}

fn reject_extra_pair_element<'de, A: SeqAccess<'de>>(sequence: &mut A) -> Result<(), A::Error> {
    let no_extra_element = sequence.next_element_seed(RejectExtraElement)?;
    debug_assert!(no_extra_element.is_none());
    Ok(())
}

struct RejectExtraElement;

impl<'de> DeserializeSeed<'de> for RejectExtraElement {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, _deserializer: D) -> Result<Self::Value, D::Error> {
        Err(de::Error::custom(
            "rule tuple must contain exactly two elements",
        ))
    }
}

fn admitted<E: de::Error>(rule: Result<Rule, RuleBuildError>) -> Result<Rule, E> {
    rule.map_err(E::custom)
}
