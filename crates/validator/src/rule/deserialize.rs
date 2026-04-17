//! Manual `Deserialize` for [`Rule`] — dispatches the first JSON key to
//! the matching sub-enum variant, producing friendly errors for unknown
//! keys.

use serde::de::{self, Deserializer, MapAccess, Visitor};

use super::{DeferredRule, Logic, Predicate, Rule, ValueRule};
use crate::foundation::FieldPath;

// ─── Known-variant catalog ────────────────────────────────────────────────
//
// Keep these lists in sync with the `visit_map` match arms below. They are
// only used to produce friendly "Known rules: ..." hints when a payload
// carries an unknown key — adding a new variant without updating these will
// NOT silently accept bad input (the match arm is authoritative), but the
// error message will omit the new variant from the hint list.

const VALUE_RULES: &[&str] = &[
    "min_length",
    "max_length",
    "pattern",
    "min",
    "max",
    "greater_than",
    "less_than",
    "one_of",
    "min_items",
    "max_items",
    "email",
    "url",
];

const PREDICATES: &[&str] = &[
    "eq", "ne", "gt", "gte", "lt", "lte", "is_true", "is_false", "set", "empty", "contains",
    "matches", "in",
];

const LOGIC: &[&str] = &["all", "any", "not"];
const DEFERRED: &[&str] = &["custom", "unique_by"];
const DESCRIBED: &str = "described";

fn all_known() -> String {
    let mut out = String::new();
    for (i, k) in VALUE_RULES
        .iter()
        .chain(PREDICATES)
        .chain(LOGIC)
        .chain(DEFERRED)
        .chain(std::iter::once(&DESCRIBED))
        .enumerate()
    {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(k);
    }
    out
}

impl<'de> serde::Deserialize<'de> for Rule {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(RuleVisitor)
    }
}

struct RuleVisitor;

impl<'de> Visitor<'de> for RuleVisitor {
    type Value = Rule;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "a rule as a bare string (unit variant) or map with a single rule key"
        )
    }

    // Unit variants: "email", "url".
    fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
        match s {
            "email" => Ok(Rule::Value(ValueRule::Email)),
            "url" => Ok(Rule::Value(ValueRule::Url)),
            other => Err(E::custom(format!(
                "unknown unit rule {other:?}; known unit rules: email, url"
            ))),
        }
    }

    fn visit_string<E: de::Error>(self, s: String) -> Result<Self::Value, E> {
        self.visit_str(&s)
    }

    fn visit_map<M: MapAccess<'de>>(self, mut m: M) -> Result<Self::Value, M::Error> {
        let Some(key) = m.next_key::<String>()? else {
            return Err(de::Error::custom("empty rule object"));
        };

        let rule = match key.as_str() {
            // ── Value rules ────────────────────────────────────────────
            "min_length" => Rule::Value(ValueRule::MinLength(m.next_value()?)),
            "max_length" => Rule::Value(ValueRule::MaxLength(m.next_value()?)),
            "pattern" => Rule::Value(ValueRule::Pattern(m.next_value()?)),
            "min" => Rule::Value(ValueRule::Min(m.next_value()?)),
            "max" => Rule::Value(ValueRule::Max(m.next_value()?)),
            "greater_than" => Rule::Value(ValueRule::GreaterThan(m.next_value()?)),
            "less_than" => Rule::Value(ValueRule::LessThan(m.next_value()?)),
            "one_of" => Rule::Value(ValueRule::OneOf(m.next_value()?)),
            "min_items" => Rule::Value(ValueRule::MinItems(m.next_value()?)),
            "max_items" => Rule::Value(ValueRule::MaxItems(m.next_value()?)),
            "email" => Rule::Value(ValueRule::Email),
            "url" => Rule::Value(ValueRule::Url),

            // ── Predicates ─────────────────────────────────────────────
            "eq" => {
                let (p, v): (FieldPath, serde_json::Value) = m.next_value()?;
                Rule::Predicate(Predicate::Eq(p, v))
            },
            "ne" => {
                let (p, v): (FieldPath, serde_json::Value) = m.next_value()?;
                Rule::Predicate(Predicate::Ne(p, v))
            },
            "gt" => {
                let (p, v): (FieldPath, serde_json::Number) = m.next_value()?;
                Rule::Predicate(Predicate::Gt(p, v))
            },
            "gte" => {
                let (p, v): (FieldPath, serde_json::Number) = m.next_value()?;
                Rule::Predicate(Predicate::Gte(p, v))
            },
            "lt" => {
                let (p, v): (FieldPath, serde_json::Number) = m.next_value()?;
                Rule::Predicate(Predicate::Lt(p, v))
            },
            "lte" => {
                let (p, v): (FieldPath, serde_json::Number) = m.next_value()?;
                Rule::Predicate(Predicate::Lte(p, v))
            },
            "is_true" => Rule::Predicate(Predicate::IsTrue(m.next_value()?)),
            "is_false" => Rule::Predicate(Predicate::IsFalse(m.next_value()?)),
            "set" => Rule::Predicate(Predicate::Set(m.next_value()?)),
            "empty" => Rule::Predicate(Predicate::Empty(m.next_value()?)),
            "contains" => {
                let (p, v): (FieldPath, serde_json::Value) = m.next_value()?;
                Rule::Predicate(Predicate::Contains(p, v))
            },
            "matches" => {
                let (p, pat): (FieldPath, String) = m.next_value()?;
                Rule::Predicate(Predicate::Matches(p, pat))
            },
            "in" => {
                let (p, vs): (FieldPath, Vec<serde_json::Value>) = m.next_value()?;
                Rule::Predicate(Predicate::In(p, vs))
            },

            // ── Logic ──────────────────────────────────────────────────
            "all" => Rule::Logic(Box::new(Logic::All(m.next_value()?))),
            "any" => Rule::Logic(Box::new(Logic::Any(m.next_value()?))),
            "not" => Rule::Logic(Box::new(Logic::Not(m.next_value()?))),

            // ── Deferred ───────────────────────────────────────────────
            "custom" => Rule::Deferred(DeferredRule::Custom(m.next_value()?)),
            "unique_by" => Rule::Deferred(DeferredRule::UniqueBy(m.next_value()?)),

            // ── Described ──────────────────────────────────────────────
            "described" => {
                let (inner, msg): (Rule, String) = m.next_value()?;
                Rule::Described(Box::new(inner), msg)
            },

            other => {
                return Err(de::Error::custom(format!(
                    "unknown rule {other:?}. Known rules: {}",
                    all_known()
                )));
            },
        };

        // Defensive: reject trailing keys — rules are single-key objects.
        if let Some(extra) = m.next_key::<String>()? {
            return Err(de::Error::custom(format!(
                "rule object must have exactly one key; found extra key {extra:?}"
            )));
        }

        Ok(rule)
    }
}
