//! Shared structural declarations for core action input records.

use nebula_schema::{
    DynamicField, Field, FieldKey, ListField, ObjectField, Predicate, Rule, ValidationError,
    ValidationReport, ValuePath, field_key,
};
use serde_json::Value;

pub(super) fn record_data() -> ListField {
    Field::list(field_key!("data"))
        .item(Field::object(field_key!("item")))
        .description("Required array of open JSON objects; an empty array is valid.")
}

pub(super) fn strings(key: FieldKey) -> ListField {
    Field::list(key).item(Field::string(field_key!("item")))
}

/// Form-style List::required rejects [], but these inputs require presence only.
/// The typed List field checks the kind; this root rule checks non-null presence.
pub(super) fn array_present(key: FieldKey) -> Result<Rule, ValidationReport> {
    let path = ValuePath::root().push(key.as_str());
    let populated = admit_rule(Rule::predicate(Predicate::Set(path.clone())))?;
    let empty = admit_rule(Rule::predicate(Predicate::Eq(
        path,
        Value::Array(Vec::new()),
    )))?;
    admit_rule(Rule::any([populated, empty]))
}

pub(super) fn admit_rule<E>(rule: Result<Rule, E>) -> Result<Rule, ValidationReport>
where
    E: std::error::Error + Send + Sync + 'static,
{
    rule.map_err(|error| {
        ValidationError::builder("schema.rule_admission")
            .message("schema rule exceeds validator admission bounds")
            .source(error)
            .build()
            .into()
    })
}

pub(super) fn nullable_object_data() -> DynamicField {
    // Field::object is non-nullable even when optional. Keep null and arbitrary
    // keys intact; the action's existing object-or-null check remains authoritative.
    Field::dynamic(field_key!("data"))
        .description("Optional open JSON object or null; the action checks this nullable shape.")
}

pub(super) fn condition(key: FieldKey) -> ObjectField {
    // Condition uses a recursive key-sniffed union, not a mode envelope. Its
    // serde visitor owns the contents; only the common object kind is declared.
    Field::object(key)
        .description(
            "Recursive condition object; its contents are checked by the Condition decoder.",
        )
        .required()
}
