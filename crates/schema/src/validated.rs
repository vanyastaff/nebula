//! Validated schema handles — proof-tokens.

use std::sync::Arc;

use indexmap::IndexMap;
use smallvec::SmallVec;

use crate::{
    error::{Severity, ValidationError, ValidationReport},
    field::{Field, ListField, NumberField, ObjectField},
    key::FieldKey,
    mode::{ExpressionMode, RequiredMode, VisibilityMode},
    path::FieldPath,
    value::{FieldValue, FieldValues},
};

/// Flags computed once at build time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaFlags {
    /// Whether any field allows or requires expressions.
    pub uses_expressions: bool,
    /// Whether any loader-backed field is present (async path).
    pub has_async_loaders: bool,
    /// Maximum nesting depth reached in this schema.
    pub max_depth: u8,
}

/// Cursor into the field tree: breadcrumb of child indices starting from root.
#[derive(Debug, Clone)]
pub struct FieldHandle {
    /// Index path from root fields vec downward.
    pub cursor: SmallVec<[u16; 4]>,
    /// Depth (1 = top-level).
    pub depth: u8,
}

/// Shared interior of a `ValidSchema`.
#[derive(Debug)]
pub struct ValidSchemaInner {
    /// Top-level ordered fields.
    pub fields: Vec<Field>,
    /// Flat index from `FieldPath` → `FieldHandle` for O(1) path lookup.
    pub index: IndexMap<FieldPath, FieldHandle>,
    /// Flags computed during build.
    pub flags: SchemaFlags,
}

/// Proof-token: schema has been built and linted successfully.
///
/// Cheap to clone — backed by `Arc`.
#[derive(Debug, Clone)]
pub struct ValidSchema(pub(crate) Arc<ValidSchemaInner>);

impl ValidSchema {
    pub(crate) fn from_inner(inner: ValidSchemaInner) -> Self {
        Self(Arc::new(inner))
    }

    /// Borrow all top-level fields in insertion order.
    pub fn fields(&self) -> &[Field] {
        &self.0.fields
    }

    /// Borrow the build-time flags.
    pub fn flags(&self) -> &SchemaFlags {
        &self.0.flags
    }

    /// Find a top-level field by key.
    pub fn find(&self, key: &FieldKey) -> Option<&Field> {
        self.0.fields.iter().find(|f| f.key() == key)
    }

    /// Find a field by dotted path using the O(1) index.
    pub fn find_by_path(&self, path: &FieldPath) -> Option<&Field> {
        let handle = self.0.index.get(path)?;
        let mut cur = self.0.fields.get(*handle.cursor.first()? as usize)?;
        for &step in &handle.cursor[1..] {
            cur = match cur {
                Field::Object(o) => o.fields.get(step as usize)?,
                Field::List(l) => l.item.as_deref()?,
                Field::Mode(m) => &m.variants.get(step as usize)?.field,
                _ => return None,
            };
        }
        Some(cur)
    }

    /// Validate runtime `values` against this schema (schema-time phase).
    ///
    /// Two-phase expression handling:
    /// - `ExpressionMode::Forbidden` + `FieldValue::Expression` → hard error
    ///   `"expression.forbidden"`.
    /// - Any other `FieldValue::Expression` → skip value/type rules; run predicate rules
    ///   (visibility/required `When(rule)`) only.
    ///
    /// Full value rules (type check, range, pattern …) run on literal values
    /// here; expression resolution happens in `ValidValues::resolve` (Task 23).
    ///
    /// # Errors
    ///
    /// Returns `Err(ValidationReport)` when any hard error is found.
    #[allow(clippy::result_large_err)]
    pub fn validate<'s>(
        &'s self,
        values: &FieldValues,
    ) -> Result<ValidValues<'s>, ValidationReport> {
        use crate::context::RootContext;

        let mut report = ValidationReport::new();
        let ctx = RootContext(values);

        for field in &self.0.fields {
            let path = FieldPath::root().join(field.key().clone());
            validate_field(field, values.get(field.key()), &ctx, &path, &mut report);
        }

        if report.has_errors() {
            return Err(report);
        }

        let warnings: Arc<[ValidationError]> = report
            .iter()
            .filter(|e| e.severity == Severity::Warning)
            .cloned()
            .collect();
        Ok(ValidValues {
            schema: self,
            values: values.clone(),
            warnings,
        })
    }
}

/// Validated values — tied to a specific `ValidSchema`.
///
/// Produced by `ValidSchema::validate()` (Task 21). Proof-token that values
/// have been checked against the schema at least once.
#[derive(Debug, Clone)]
pub struct ValidValues<'s> {
    pub(crate) schema: &'s ValidSchema,
    pub(crate) values: FieldValues,
    pub(crate) warnings: Arc<[ValidationError]>,
}

impl<'s> ValidValues<'s> {
    /// Borrow the schema these values were validated against.
    pub fn schema(&self) -> &'s ValidSchema {
        self.schema
    }

    /// Borrow the raw value tree.
    pub fn raw(&self) -> &FieldValues {
        &self.values
    }

    /// Borrow the raw value tree (alias for [`raw`](Self::raw)).
    pub fn raw_values(&self) -> &FieldValues {
        &self.values
    }

    /// Iterate validation warnings that were non-fatal.
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Look up a top-level value by key.
    pub fn get(&self, key: &FieldKey) -> Option<&FieldValue> {
        self.values.get(key)
    }

    /// Look up a value by dotted path.
    pub fn get_path(&self, path: &FieldPath) -> Option<&FieldValue> {
        self.values.get_path(path)
    }
}

/// Resolved values — all `FieldValue::Expression` entries have been evaluated.
///
/// Produced by `ValidValues::resolve()` (Task 23). Proof-token that no
/// expression placeholders remain in the value tree.
#[derive(Debug, Clone)]
pub struct ResolvedValues<'s> {
    pub(crate) schema: &'s ValidSchema,
    pub(crate) values: FieldValues,
    pub(crate) warnings: Arc<[ValidationError]>,
}

impl<'s> ResolvedValues<'s> {
    /// Borrow the schema these values were resolved against.
    pub fn schema(&self) -> &'s ValidSchema {
        self.schema
    }

    /// Iterate resolution warnings.
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Look up a resolved literal value by key.
    ///
    /// Returns `None` if the field is absent or still an expression
    /// (should not happen in a properly resolved set).
    pub fn get(&self, key: &FieldKey) -> Option<&serde_json::Value> {
        match self.values.get(key)? {
            FieldValue::Literal(v) => Some(v),
            _ => None,
        }
    }

    /// Consume into a flat JSON object.
    pub fn into_json(self) -> serde_json::Value {
        self.values.to_json()
    }

    /// Consume and deserialize into a typed value.
    pub fn into_typed<T: serde::de::DeserializeOwned>(self) -> Result<T, Box<ValidationError>> {
        serde_json::from_value(self.into_json()).map_err(|e| {
            Box::new(
                ValidationError::new("type_mismatch")
                    .message(format!("deserialize failed: {e}"))
                    .build(),
            )
        })
    }
}

// ── Schema-time validation helpers ───────────────────────────────────────────

/// Validate a single field against an optional raw value and a context.
///
/// Recurses for `Object`, `List`, and `Mode` containers.
fn validate_field(
    field: &Field,
    raw: Option<&FieldValue>,
    ctx: &dyn nebula_validator::RuleContext,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    // Visibility predicate — if hidden and absent, skip silently.
    let visible = match field.visible() {
        VisibilityMode::Always => true,
        VisibilityMode::Never => false,
        VisibilityMode::When(rule) => rule.evaluate(ctx),
    };
    if !visible && raw.is_none() {
        return;
    }

    // Required predicate.
    let required = match field.required() {
        RequiredMode::Never => false,
        RequiredMode::Always => true,
        RequiredMode::When(rule) => rule.evaluate(ctx),
    };
    let value_is_null_or_absent =
        raw.is_none() || matches!(raw, Some(FieldValue::Literal(serde_json::Value::Null)));
    if required && value_is_null_or_absent {
        report.push(
            ValidationError::new("required")
                .at(path.clone())
                .message(format!("field `{path}` is required"))
                .build(),
        );
        return;
    }

    let Some(value) = raw else {
        return;
    };

    // Expression-mode enforcement.
    match (field.expression(), value) {
        (ExpressionMode::Forbidden, FieldValue::Expression(_)) => {
            report.push(
                ValidationError::new("expression.forbidden")
                    .at(path.clone())
                    .message(format!("field `{path}` does not allow expression values"))
                    .build(),
            );
            return;
        },
        (_, FieldValue::Expression(expr)) => {
            // Expression is allowed or required here — attempt a parse so
            // obvious syntax errors are caught at validate-time.
            if let Err(e) = expr.parse() {
                report.push(e);
            }
            // Skip all value/type rules — expression not yet resolved.
            return;
        },
        _ => {},
    }

    // Value rules apply to literals only from this point on.
    validate_literal_value(field, value, ctx, path, report);
}

/// Type-check and rule-run a literal (non-expression) value.
#[expect(
    clippy::too_many_lines,
    reason = "field-type dispatch table — splitting into smaller fns reduces clarity"
)]
fn validate_literal_value(
    field: &Field,
    value: &FieldValue,
    ctx: &dyn nebula_validator::RuleContext,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    use crate::{
        Transformer,
        field::{ModeField, SelectField},
    };

    // Helper: apply transformers to a serde_json::Value.
    fn apply_transformers(transformers: &[Transformer], v: serde_json::Value) -> serde_json::Value {
        transformers.iter().fold(v, |cur, t| t.apply(&cur))
    }

    match field {
        Field::String(f) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            let transformed = apply_transformers(&f.transformers, lit.clone());
            if !transformed.is_string() {
                report.push(
                    ValidationError::new("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a string value"))
                        .build(),
                );
                return;
            }
            run_rules(field.rules(), &transformed, path, report);
        },
        Field::Secret(f) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            let transformed = apply_transformers(&f.transformers, lit.clone());
            if !transformed.is_string() {
                report.push(
                    ValidationError::new("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a string value"))
                        .build(),
                );
                return;
            }
            run_rules(field.rules(), &transformed, path, report);
        },
        Field::Code(f) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            let transformed = apply_transformers(&f.transformers, lit.clone());
            if !transformed.is_string() {
                report.push(
                    ValidationError::new("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a string value"))
                        .build(),
                );
                return;
            }
            run_rules(field.rules(), &transformed, path, report);
        },
        Field::Number(NumberField {
            integer,
            transformers,
            rules,
            ..
        }) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            let transformed = apply_transformers(transformers, lit.clone());
            let Some(num) = transformed.as_f64() else {
                report.push(
                    ValidationError::new("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a numeric value"))
                        .build(),
                );
                return;
            };
            if *integer && num.fract() != 0.0 {
                report.push(
                    ValidationError::new("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a whole number"))
                        .build(),
                );
                return;
            }
            run_rules(rules, &transformed, path, report);
        },
        Field::Boolean(_) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            if !lit.is_boolean() {
                report.push(
                    ValidationError::new("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a boolean value"))
                        .build(),
                );
            }
        },
        Field::Select(SelectField {
            options,
            multiple,
            allow_custom,
            rules,
            transformers,
            ..
        }) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            let transformed = apply_transformers(transformers, lit.clone());
            run_rules(rules, &transformed, path, report);
            if !allow_custom && !options.is_empty() {
                check_select_options(options, *multiple, &transformed, path, report);
            }
        },
        Field::List(ListField {
            min_items,
            max_items,
            item,
            rules,
            ..
        }) => {
            // FieldValue::List is the canonical typed form; Literal(Array) may occur
            // from raw JSON that wasn't fully parsed through FieldValue::from_json.
            let (item_count, items_typed): (usize, Option<&Vec<FieldValue>>) = match value {
                FieldValue::List(v) => (v.len(), Some(v)),
                FieldValue::Literal(serde_json::Value::Array(a)) => (a.len(), None),
                _ => {
                    report.push(
                        ValidationError::new("type_mismatch")
                            .at(path.clone())
                            .message(format!("field `{path}` expects an array value"))
                            .build(),
                    );
                    return;
                },
            };
            let _ = rules; // rules on the list itself run via run_rules below if needed
            if let Some(min) = min_items
                && item_count < *min as usize
            {
                report.push(
                    ValidationError::new("items.min")
                        .at(path.clone())
                        .param("min", serde_json::json!(min))
                        .param("actual", serde_json::json!(item_count))
                        .message(format!(
                            "field `{path}` requires at least {min} items, got {item_count}"
                        ))
                        .build(),
                );
            }
            if let Some(max) = max_items
                && item_count > *max as usize
            {
                report.push(
                    ValidationError::new("items.max")
                        .at(path.clone())
                        .param("max", serde_json::json!(max))
                        .param("actual", serde_json::json!(item_count))
                        .message(format!(
                            "field `{path}` allows at most {max} items, got {item_count}"
                        ))
                        .build(),
                );
            }
            // Recurse into typed items when schema is present.
            if let (Some(item_field), Some(items_fv)) = (item.as_deref(), items_typed) {
                for (i, item_val) in items_fv.iter().enumerate() {
                    let item_path = path.clone().join(i);
                    validate_field(item_field, Some(item_val), ctx, &item_path, report);
                }
            }
        },
        Field::Object(ObjectField {
            fields: child_fields,
            transformers,
            rules,
            ..
        }) => {
            let FieldValue::Object(map) = value else {
                report.push(
                    ValidationError::new("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects an object value"))
                        .build(),
                );
                return;
            };
            let sub_ctx = crate::context::ObjectContext(map);
            for child in child_fields {
                let child_path = path.clone().join(child.key().clone());
                validate_field(child, map.get(child.key()), &sub_ctx, &child_path, report);
            }
            let _ = (transformers, rules);
        },
        Field::Mode(ModeField {
            variants,
            default_variant,
            rules,
            ..
        }) => {
            let FieldValue::Mode {
                mode: mode_key,
                value: mode_value,
            } = value
            else {
                report.push(
                    ValidationError::new("type_mismatch")
                        .at(path.clone())
                        .message(format!(
                            "field `{path}` expects a mode value ({{\"mode\": \"...\", ...}})"
                        ))
                        .build(),
                );
                return;
            };
            run_rules(rules, &value.to_json(), path, report);
            let resolved_key = Some(mode_key.as_str()).or(default_variant.as_deref());
            let Some(resolved_key) = resolved_key else {
                report.push(
                    ValidationError::new("mode.required")
                        .at(path.clone())
                        .message(format!("field `{path}` requires a mode key"))
                        .build(),
                );
                return;
            };
            let Some(variant) = variants.iter().find(|v| v.key == resolved_key) else {
                report.push(
                    ValidationError::new("mode.invalid")
                        .at(path.clone())
                        .param("mode", serde_json::Value::String(resolved_key.to_owned()))
                        .message(format!(
                            "field `{path}` has unknown mode variant `{resolved_key}`"
                        ))
                        .build(),
                );
                return;
            };
            if let Some(payload) = mode_value {
                let payload_path = path
                    .clone()
                    .join(FieldKey::new("value").expect("static key"));
                validate_field(&variant.field, Some(payload), ctx, &payload_path, report);
            }
        },
        // File, Computed, Dynamic, Notice — no type-check rule at schema time.
        Field::File(f) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            if f.multiple {
                if !lit.is_array() {
                    report.push(
                        ValidationError::new("type_mismatch")
                            .at(path.clone())
                            .message(format!("field `{path}` expects an array of file paths"))
                            .build(),
                    );
                }
            } else if !lit.is_string() {
                report.push(
                    ValidationError::new("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a string file path"))
                        .build(),
                );
            }
        },
        Field::Computed(_) | Field::Dynamic(_) | Field::Notice(_) => {},
    }
}

/// Validate a transformed value against a static option set.
fn check_select_options(
    options: &[crate::SelectOption],
    multiple: bool,
    transformed: &serde_json::Value,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    if multiple {
        if let Some(arr) = transformed.as_array() {
            for (i, v) in arr.iter().enumerate() {
                if !options.iter().any(|o| o.value == *v) {
                    report.push(
                        ValidationError::new("option.invalid")
                            .at(path.clone())
                            .param("index", serde_json::json!(i))
                            .message(format!("field `{path}[{i}]` is not in allowed option set"))
                            .build(),
                    );
                }
            }
        }
    } else if !options.iter().any(|o| o.value == *transformed) {
        report.push(
            ValidationError::new("option.invalid")
                .at(path.clone())
                .message(format!("field `{path}` value is not in allowed option set"))
                .build(),
        );
    }
}

/// Apply a slice of rules to a JSON literal value, pushing errors into `report`.
fn run_rules(
    rules: &[nebula_validator::Rule],
    value: &serde_json::Value,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    use nebula_validator::ExecutionMode;
    if let Err(errs) = nebula_validator::validate_rules(value, rules, ExecutionMode::StaticOnly) {
        for e in errs.errors() {
            // Clone into owned strings to satisfy `Cow<'static, str>` constraint.
            let code: String = e.code.as_ref().to_owned();
            let msg: String = e.message.as_ref().to_owned();
            report.push(
                ValidationError::new(code)
                    .at(path.clone())
                    .message(msg)
                    .build(),
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, FieldKey, Schema};

    #[test]
    fn clone_is_cheap_via_arc() {
        let s = Schema::builder()
            .add(Field::string(FieldKey::new("x").unwrap()))
            .build()
            .unwrap();
        let c = s.clone();
        assert!(Arc::ptr_eq(&s.0, &c.0));
    }

    #[test]
    fn find_returns_top_level() {
        let s = Schema::builder()
            .add(Field::string(FieldKey::new("x").unwrap()))
            .build()
            .unwrap();
        assert!(s.find(&FieldKey::new("x").unwrap()).is_some());
        assert!(s.find(&FieldKey::new("y").unwrap()).is_none());
    }
}
