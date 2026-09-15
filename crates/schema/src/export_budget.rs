//! Pre-copy resource accounting for the JSON Schema projection.

use std::io::{self, Write};

use serde::{Serialize, Serializer, ser::SerializeMap};

use super::JsonSchemaExportError;
use crate::{Field, ValidSchema};

const MAX_SOURCE_DESCRIPTOR_BYTES: usize = 1024 * 1024;
const MAX_SERIALIZED_COPY_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct ExportBudget {
    copies: ByteBudget,
}

impl ExportBudget {
    pub(super) fn for_schema(schema: &ValidSchema) -> Result<Self, JsonSchemaExportError> {
        let mut source = ByteBudget::new(MAX_SOURCE_DESCRIPTOR_BYTES, BudgetKind::Source);
        source.charge(&SourceDescriptor {
            policy_version: schema.policy_version(),
            kind: schema.kind(),
            serde_tagging: schema.serde_tagging(),
            fields: SourceFields(schema.fields()),
            scalar: schema.scalar_schema(),
            root_rules: if schema.scalar_schema().is_some() {
                &[]
            } else {
                schema.root_rules()
            },
        })?;
        Ok(Self {
            copies: ByteBudget::new(MAX_SERIALIZED_COPY_BYTES, BudgetKind::Copies),
        })
    }

    pub(super) fn copy<T: Serialize + Clone>(
        &mut self,
        value: &T,
    ) -> Result<T, JsonSchemaExportError> {
        self.copies.charge(value)?;
        Ok(value.clone())
    }
}

#[derive(Clone, Copy)]
enum BudgetKind {
    Source,
    Copies,
}

struct ByteBudget {
    remaining: usize,
    limit: usize,
    kind: BudgetKind,
}

impl ByteBudget {
    const fn new(limit: usize, kind: BudgetKind) -> Self {
        Self {
            remaining: limit,
            limit,
            kind,
        }
    }

    fn charge<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), JsonSchemaExportError> {
        let mut counter = CountingWriter {
            remaining: self.remaining,
            exceeded: false,
        };
        let result = serde_json::to_writer(&mut counter, value);
        self.remaining = counter.remaining;
        if counter.exceeded {
            let (code, error) = match self.kind {
                BudgetKind::Source => (
                    "schema.export.source_budget",
                    JsonSchemaExportError::SourceBudgetExceeded,
                ),
                BudgetKind::Copies => (
                    "schema.export.copy_budget",
                    JsonSchemaExportError::CopyBudgetExceeded,
                ),
            };
            tracing::debug!(
                code,
                limit_bytes = self.limit,
                "JSON Schema export budget exceeded"
            );
            return Err(error);
        }
        result.map_err(|_| {
            tracing::debug!(
                code = "schema.export.budget_serialization",
                "JSON Schema export budget measurement failed"
            );
            JsonSchemaExportError::BudgetSerialization
        })
    }
}

struct CountingWriter {
    remaining: usize,
    exceeded: bool,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(remaining) = self.remaining.checked_sub(bytes.len()) else {
            self.exceeded = true;
            self.remaining = 0;
            return Err(io::Error::other("JSON Schema export budget exceeded"));
        };
        self.remaining = remaining;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// Field's wire serializer clones its owned mirror, including descendants. This
// private borrowing view instead counts every slot, even omitted defaults, so it
// conservatively bounds source wire size without allocating that mirror or JSON.
#[derive(Serialize)]
struct SourceDescriptor<'a> {
    policy_version: u16,
    kind: crate::SchemaKind,
    serde_tagging: Option<&'a crate::SerdeTagging>,
    fields: SourceFields<'a>,
    scalar: Option<&'a crate::ScalarSchema>,
    root_rules: &'a [nebula_validator::Rule],
}

struct SourceFields<'a>(&'a [Field]);

impl Serialize for SourceFields<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(SourceField))
    }
}

struct SourceField<'a>(&'a Field);

impl Serialize for SourceField<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Exhaustive destructuring forces new source slots to enter accounting.
        macro_rules! serialize_field {
            ($ty:ident, $field:expr, { $($extra:ident => $value:expr),* $(,)? }) => {{
                let crate::field::$ty {
                    key, label, description, placeholder, default, visible, required,
                    expression, group, read_aliases, emit_as, rules, transformers,
                    $($extra,)*
                } = $field;
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("type", self.0.type_name())?;
                map.serialize_entry("key", key)?;
                map.serialize_entry("label", label)?;
                map.serialize_entry("description", description)?;
                map.serialize_entry("placeholder", placeholder)?;
                map.serialize_entry("default", &default.as_ref().map(SourceJson::new))?;
                map.serialize_entry("visible", visible)?;
                map.serialize_entry("required", required)?;
                map.serialize_entry("expression", expression)?;
                map.serialize_entry("group", group)?;
                map.serialize_entry("read_aliases", read_aliases)?;
                map.serialize_entry("emit_as", emit_as)?;
                map.serialize_entry("rules", rules)?;
                map.serialize_entry("transformers", transformers)?;
                $(map.serialize_entry(stringify!($extra), &$value)?;)*
                map.end()
            }};
        }

        match self.0 {
            Field::String(field) => serialize_field!(StringField, field, {
                hint => hint, widget => widget,
            }),
            Field::Secret(field) => serialize_field!(SecretField, field, {
                widget => widget, reveal_last => reveal_last,
            }),
            Field::Number(field) => serialize_field!(NumberField, field, {
                integer => integer, widget => widget, step => step,
            }),
            Field::Boolean(field) => serialize_field!(BooleanField, field, { widget => widget }),
            Field::Select(field) => serialize_field!(SelectField, field, {
                options => SourceOptions(options), dynamic => dynamic, loader => loader,
                depends_on => depends_on, multiple => multiple, allow_custom => allow_custom,
                searchable => searchable, widget => widget,
            }),
            Field::Object(field) => serialize_field!(ObjectField, field, {
                fields => SourceFields(fields), widget => widget,
            }),
            Field::List(field) => serialize_field!(ListField, field, {
                item => item.as_deref().map(SourceField), min_items => min_items,
                max_items => max_items, unique => unique, widget => widget,
            }),
            Field::Mode(field) => serialize_field!(ModeField, field, {
                variants => SourceVariants(variants), default_variant => default_variant,
            }),
            Field::Code(field) => serialize_field!(CodeField, field, {
                language => language, widget => widget,
            }),
            Field::File(field) => serialize_field!(FileField, field, {
                accept => accept, max_size => max_size, multiple => multiple,
            }),
            Field::Computed(field) => {
                serialize_field!(ComputedField, field, { returns => returns })
            },
            Field::Dynamic(field) => serialize_field!(DynamicField, field, {
                depends_on => depends_on, loader => loader,
            }),
            Field::Notice(field) => serialize_field!(NoticeField, field, { severity => severity }),
            Field::Unknown(_) => self.0.serialize(serializer),
        }
    }
}

struct SourceVariants<'a>(&'a [crate::field::ModeVariant]);

impl Serialize for SourceVariants<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct SourceVariant<'a> {
            key: &'a str,
            label: &'a str,
            field: SourceField<'a>,
        }
        serializer.collect_seq(self.0.iter().map(|variant| {
            let crate::field::ModeVariant { key, label, field } = variant;
            SourceVariant {
                key,
                label,
                field: SourceField(field),
            }
        }))
    }
}

struct SourceOptions<'a>(&'a [crate::SelectOption]);

impl Serialize for SourceOptions<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct SourceOption<'a> {
            value: SourceJson<'a>,
            label: &'a str,
            description: &'a Option<String>,
            disabled: bool,
        }
        serializer.collect_seq(self.0.iter().map(|option| {
            let crate::SelectOption {
                value,
                label,
                description,
                disabled,
            } = option;
            SourceOption {
                value: SourceJson::new(value),
                label,
                description,
                disabled: *disabled,
            }
        }))
    }
}

struct SourceJson<'a> {
    value: &'a serde_json::Value,
    depth: u8,
}

impl<'a> SourceJson<'a> {
    const fn new(value: &'a serde_json::Value) -> Self {
        Self { value, depth: 0 }
    }
}

impl Serialize for SourceJson<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error as _;

        if self.depth > crate::MAX_VALUE_DEPTH {
            return Err(S::Error::custom("source JSON exceeds measurement depth"));
        }
        let child_depth = self
            .depth
            .checked_add(1)
            .ok_or_else(|| S::Error::custom("source JSON exceeds measurement depth"))?;
        match self.value {
            serde_json::Value::Array(values) => {
                serializer.collect_seq(values.iter().map(|value| Self {
                    value,
                    depth: child_depth,
                }))
            },
            serde_json::Value::Object(values) => {
                serializer.collect_map(values.iter().map(|(key, value)| {
                    (
                        key,
                        Self {
                            value,
                            depth: child_depth,
                        },
                    )
                }))
            },
            scalar => scalar.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches, cell::Cell};

    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn string_accounting_matches_serde_json(value in any::<String>(), spare in 0usize..1024) {
            let bytes = serde_json::to_vec(&value).unwrap().len();
            let mut budget = ByteBudget::new(bytes + spare, BudgetKind::Source);
            budget.charge(&value).unwrap();
            prop_assert_eq!(budget.remaining, spare);
            let mut short = ByteBudget::new(bytes - 1, BudgetKind::Source);
            prop_assert!(matches!(short.charge(&value), Err(JsonSchemaExportError::SourceBudgetExceeded)));
        }
    }

    #[test]
    fn unelided_field_view_preserves_every_known_field_payload() {
        use crate::field_key;

        let fields: Vec<Field> = vec![
            Field::string(field_key!("string")).description("\0").into(),
            Field::secret(field_key!("secret")).into(),
            Field::number(field_key!("number")).into(),
            Field::boolean(field_key!("boolean")).into(),
            Field::select(field_key!("select"))
                .option("value", "Label")
                .into(),
            Field::object(field_key!("object"))
                .add(Field::string(field_key!("child")))
                .into(),
            Field::list(field_key!("list"))
                .item(Field::boolean(field_key!("item")))
                .into(),
            Field::mode(field_key!("mode"))
                .variant_empty("empty", "Empty")
                .into(),
            Field::code(field_key!("code")).into(),
            Field::file(field_key!("file")).into(),
            Field::computed(field_key!("computed")).into(),
            Field::dynamic(field_key!("dynamic")).into(),
            Field::notice(field_key!("notice")).into(),
        ];
        for field in fields {
            let borrowed = serde_json::to_vec(&SourceField(&field)).unwrap();
            let wire = serde_json::to_vec(&field).unwrap();
            assert!(borrowed.len() >= wire.len(), "{}", field.type_name());
            assert_eq!(serde_json::from_slice::<Field>(&borrowed).unwrap(), field);
        }
    }

    #[test]
    fn counting_matches_compact_json_including_escaping_and_utf8() {
        for value in [
            serde_json::json!(null),
            serde_json::json!({"control\n": ["\0\t\"\\", "\u{00e9}\u{1f600}"]}),
        ] {
            let bytes = serde_json::to_vec(&value).unwrap().len();
            let mut exact = ByteBudget::new(bytes, BudgetKind::Source);
            exact.charge(&value).unwrap();
            assert_eq!(exact.remaining, 0);
            let mut short = ByteBudget::new(bytes - 1, BudgetKind::Source);
            assert_matches!(
                short.charge(&value),
                Err(JsonSchemaExportError::SourceBudgetExceeded)
            );
        }
    }

    #[test]
    fn copy_inputs_are_charged_cumulatively_before_cloning() {
        let value = serde_json::json!({"value": "\n"});
        let bytes = serde_json::to_vec(&value).unwrap().len();
        let mut budget = ExportBudget {
            copies: ByteBudget::new(bytes * 2, BudgetKind::Copies),
        };
        assert_eq!(budget.copy(&value).unwrap(), value);
        assert_eq!(budget.copy(&value).unwrap(), value);
        assert_eq!(budget.copies.remaining, 0);
        assert_matches!(
            budget.copy(&value),
            Err(JsonSchemaExportError::CopyBudgetExceeded)
        );
    }

    #[test]
    fn exhausted_copy_budget_does_not_invoke_clone() {
        #[derive(Serialize)]
        struct CloneProbe<'a> {
            value: &'a str,
            #[serde(skip)]
            clones: &'a Cell<usize>,
        }
        impl Clone for CloneProbe<'_> {
            fn clone(&self) -> Self {
                self.clones.set(self.clones.get() + 1);
                Self {
                    value: self.value,
                    clones: self.clones,
                }
            }
        }
        let clones = Cell::new(0);
        let value = CloneProbe {
            value: "\0",
            clones: &clones,
        };
        let mut budget = ExportBudget {
            copies: ByteBudget::new(1, BudgetKind::Copies),
        };
        assert_matches!(
            budget.copy(&value).err(),
            Some(JsonSchemaExportError::CopyBudgetExceeded)
        );
        assert_eq!(clones.get(), 0);
    }

    #[test]
    fn source_json_measurement_refuses_excess_depth() {
        let mut value = serde_json::Value::Null;
        for _ in 0..crate::MAX_VALUE_DEPTH {
            value = serde_json::Value::Array(vec![value]);
        }
        let mut budget = ByteBudget::new(MAX_SOURCE_DESCRIPTOR_BYTES, BudgetKind::Source);
        budget.charge(&SourceJson::new(&value)).unwrap();
        value = serde_json::Value::Array(vec![value]);
        assert_matches!(
            budget.charge(&SourceJson::new(&value)),
            Err(JsonSchemaExportError::BudgetSerialization)
        );
    }

    #[test]
    fn counter_rejects_underflow_even_after_exhaustion() {
        let mut counter = CountingWriter {
            remaining: 1,
            exceeded: false,
        };
        assert_eq!(counter.write(b"x").unwrap(), 1);
        assert_eq!(counter.remaining, 0);
        assert_eq!(
            counter.write(b"x").unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(counter.remaining, 0);
        assert!(counter.exceeded);
        assert_eq!(
            counter.write(b"xx").unwrap_err().kind(),
            io::ErrorKind::Other
        );
    }
}
